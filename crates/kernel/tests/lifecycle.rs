use sol_kernel::Page;
use sol_url::Url;

fn page_with(html: &str) -> Page {
    let mut page = Page::new(128, 96);
    page.load_html(html, &Url::parse("file:///tmp/lifecycle.html").unwrap())
        .unwrap();
    page
}

fn pixel(page: &Page, x: u32, y: u32) -> [u8; 4] {
    let canvas = page.render();
    let offset = ((y * canvas.w + x) * 4) as usize;
    canvas.px[offset..offset + 4].try_into().unwrap()
}

#[test]
fn input_handlers_observe_new_value_and_their_mutations_reflow() {
    let mut page = page_with(
        "<body><input id='field'><div id='output' style='width:10px;height:10px'></div>\
         <script>var seen=[];var field=document.getElementById('field');\
         field.addEventListener('input',()=>{seen.push('input:'+field.value);\
           document.getElementById('output').style.width='40px';});\
         field.addEventListener('change',()=>seen.push('change:'+field.value));</script>",
    );
    let field = page.doc.borrow().get_element_by_id("field").unwrap();
    page.set_input(field, "edited");
    assert_eq!(page.doc.borrow().attr(field, "value"), Some("edited"));
    let events = page.js.eval("seen.join('|')").unwrap();
    assert_eq!(events.to_str(&page.js.rt), "input:edited|change:edited");
    let width = page
        .js
        .eval("document.getElementById('output').getBoundingClientRect().width")
        .unwrap();
    assert_eq!(width.to_num(&page.js.rt), 40.0);
}

#[test]
fn click_dispatch_order_and_focus_style_agree_with_painted_pixels() {
    let mut page = page_with(
        "<style>#button{position:absolute;left:0;top:0;width:40px;height:40px;background:blue}\
         #button:focus{background:lime}</style><div id='button'></div>\
         <script>var events=[];var button=document.getElementById('button');\
         button.addEventListener('mousedown',()=>events.push('down'));\
         button.addEventListener('mouseup',()=>events.push('up'));\
         button.addEventListener('click',()=>events.push('click'));</script>",
    );
    let button = page.doc.borrow().get_element_by_id("button").unwrap();
    page.click(10.0, 10.0);
    assert_eq!(page.interact.focus, Some(button));
    assert_eq!(page.interact.active, Some(button));
    let events = page.js.eval("events.join('|')").unwrap();
    assert_eq!(events.to_str(&page.js.rt), "down|up|click");
    assert_eq!(pixel(&page, 10, 10), [0, 255, 0, 255]);
}

#[test]
fn fragment_click_preserves_live_document_and_repaints_target_state() {
    let mut page = page_with(
        "<style>a{position:absolute;left:0;top:0;width:40px;height:40px;display:block}\
         #target{position:absolute;left:50px;top:0;width:40px;height:40px;background:blue}\
         #target:target{background:red}</style>\
         <a href='#target'><span>jump</span></a><div id='target'></div>",
    );
    page.js
        .eval("document.getElementById('target').setAttribute('data-live','yes')")
        .unwrap();
    page.settle();
    assert!(page.click(20.0, 20.0));
    let doc = page.doc.borrow();
    let target = doc.get_element_by_id("target").unwrap();
    assert_eq!(page.interact.target, Some(target));
    assert_eq!(doc.attr(target, "data-live"), Some("yes"));
    drop(doc);
    assert_eq!(pixel(&page, 60, 10), [255, 0, 0, 255]);
}

#[test]
fn promise_mutations_precede_due_timer_mutations_and_refresh_geometry() {
    let mut page = page_with(
        "<div id='box' style='width:10px;height:10px'></div><script>var order=[];\
         Promise.resolve().then(()=>order.push('microtask'));\
         setTimeout(()=>{order.push('timer');document.getElementById('box').style.width='55px';},0);\
         </script>",
    );
    let order = page.js.eval("order.join('|')").unwrap();
    assert_eq!(order.to_str(&page.js.rt), "microtask|timer");
    let width = page
        .js
        .eval("document.getElementById('box').getBoundingClientRect().width")
        .unwrap();
    assert_eq!(width.to_num(&page.js.rt), 55.0);
}

#[test]
fn reloading_html_reuses_document_handle_without_leaking_author_styles() {
    let mut page = page_with(
        "<style>#box{background:red}</style><div id='old'></div>\
         <div id='box' style='position:absolute;left:0;top:0;width:40px;height:40px'></div>",
    );
    assert_eq!(pixel(&page, 10, 10), [255, 0, 0, 255]);
    let document_handle = page.doc.clone();
    page.load_html(
        "<body style='background:blue'><div id='box' style='position:absolute;left:0;top:0;width:40px;height:40px'></div></body>",
        &Url::parse("file:///tmp/reloaded.html").unwrap(),
    )
    .unwrap();
    assert!(std::rc::Rc::ptr_eq(&document_handle, &page.doc));
    assert!(document_handle.borrow().get_element_by_id("old").is_none());
    page.js
        .eval("document.getElementById('box').setAttribute('data-current','yes')")
        .unwrap();
    assert!(page.outer_html().contains("data-current=\"yes\""));
    assert_eq!(pixel(&page, 10, 10), [0, 0, 255, 255]);
}
