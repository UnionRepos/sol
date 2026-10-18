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

#[test]
fn modern_css_values_and_relational_selectors_reach_geometry_and_pixels() {
    let mut page = page_with(
        r#"<style>
        :root { --width: 20px; --rgb: 255 0 0; }
        body { margin:0 }
        @future preserved { x:y }
        @supports (display:flex) {
            @media (100px <= width < 200px) {
                #box:has(> .child) { position:absolute; left:3px; top:4px;
                    width:calc(var(--width) * 2 + 5px); height:20px;
                    background:rgb(var(--rgb)); }
            }
        }
        :where(#box) { width:1px }
        </style><div id=box><span class=child></span></div>"#,
    );
    let width = page
        .js
        .eval("document.getElementById('box').getBoundingClientRect().width")
        .unwrap();
    assert_eq!(width.to_num(&page.js.rt), 45.0);
    assert_eq!(pixel(&page, 10, 10), [255, 0, 0, 255]);
    assert_eq!(pixel(&page, 50, 10), [255, 255, 255, 255]);
}

#[test]
fn javascript_selector_lists_and_scope_use_the_shared_css_parser() {
    let mut page =
        page_with("<main id=root><div id=a class=x><b class=x></b></div><div id=b></div></main>");
    let value=page.js.eval("var root=document.getElementById('root'); root.querySelectorAll(':scope > .x, :scope > #b').length").unwrap();
    assert_eq!(value.to_num(&page.js.rt), 2.0);
    let value = page
        .js
        .eval("document.getElementById('a').matches(':scope:is(.x, .missing), #other')")
        .unwrap();
    assert_eq!(value.to_str(&page.js.rt), "true");
}

#[test]
fn opacity_composites_descendants_as_one_group_including_positioned_children() {
    let page=page_with("<body style='margin:0'><div style='position:relative;opacity:.5;width:40px;height:40px;background:red'><div style='position:absolute;left:0;top:0;width:20px;height:20px;background:blue'></div></div>");
    assert_eq!(pixel(&page, 10, 10), [128, 128, 255, 255]);
    assert_eq!(pixel(&page, 30, 30), [255, 128, 128, 255]);
}

#[test]
fn positioned_descendants_escape_non_stacking_ancestors_and_hits_follow_z_index() {
    let page=page_with("<body style='margin:0'><div><div id=front style='position:absolute;left:0;top:0;width:40px;height:40px;background:blue;z-index:10'></div></div><div id=back style='position:absolute;left:0;top:0;width:40px;height:40px;background:red;z-index:1'></div>");
    assert_eq!(pixel(&page, 10, 10), [0, 0, 255, 255]);
    assert_eq!(
        page.hit(10.0, 10.0),
        page.doc.borrow().get_element_by_id("front")
    );
}

#[test]
fn generated_content_changes_layout_without_inserting_dom_nodes() {
    let mut page=page_with("<style>body{margin:0} #host::before{content:attr(data-label);display:block;height:12px;background:red} #host::after{content:'';display:block;height:8px;background:blue}</style><div id=host data-label=hello></div>");
    let value = page
        .js
        .eval("document.getElementById('host').getBoundingClientRect().height")
        .unwrap();
    assert_eq!(value.to_num(&page.js.rt), 20.0);
    assert_eq!(pixel(&page, 100, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&page, 100, 15), [0, 0, 255, 255]);
    assert_eq!(
        page.doc
            .borrow()
            .children(page.doc.borrow().get_element_by_id("host").unwrap())
            .len(),
        0
    );
}

#[test]
fn stylesheet_mutations_rebuild_the_cascade_without_reloading_the_document() {
    let mut page = page_with(
        "<style id=css>#box{width:10px;height:10px;background:red}</style><div id=box></div>",
    );
    page.js.eval("document.getElementById('css').textContent='#box{width:30px;height:10px;background:blue}'").unwrap();
    page.settle();
    let value = page
        .js
        .eval("document.getElementById('box').getBoundingClientRect().width")
        .unwrap();
    assert_eq!(value.to_num(&page.js.rt), 30.0);
    assert_eq!(pixel(&page, 15, 10), [0, 0, 255, 255]);
}

#[test]
fn imported_stylesheets_resolve_background_urls_against_their_own_directory() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = std::env::temp_dir().join(format!(
        "sol-css-resources-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(directory.join("css/nested")).unwrap();
    std::fs::write(
        directory.join("css/main.css"),
        "@import 'nested/paint.css';",
    )
    .unwrap();
    std::fs::write(
        directory.join("css/nested/paint.css"),
        "#box{width:30px;height:20px;background:url(tile.png) center/10px 10px no-repeat red}",
    )
    .unwrap();
    let mut image = sol_img::Image::new(2, 2);
    for y in 0..2 {
        for x in 0..2 {
            image.set(x, y, [0, 0, 255, 255]);
        }
    }
    std::fs::write(
        directory.join("css/nested/tile.png"),
        sol_img::encode_png(&image),
    )
    .unwrap();
    let mut page = Page::new(64, 48);
    let url = Url::parse(&format!("file://{}/index.html", directory.display())).unwrap();
    page.load_html(
        "<link rel=stylesheet href='css/main.css'><body style='margin:0'><div id=box></div>",
        &url,
    )
    .unwrap();
    assert_eq!(pixel(&page, 15, 10), [0, 0, 255, 255]);
    assert_eq!(pixel(&page, 2, 2), [255, 0, 0, 255]);
    assert!(
        page.console_lines().is_empty(),
        "{:?}",
        page.console_lines()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn keyframes_sample_geometry_color_and_important_priority_at_explicit_times() {
    let mut page=page_with("<style>body{margin:0} @keyframes grow{from{width:10px;background-color:red} to{width:50px;background-color:blue}} #box{height:20px;animation:grow 2s linear forwards}</style><div id=box></div>");
    page.sample_animations(1.0);
    let width = page
        .js
        .eval("document.getElementById('box').getBoundingClientRect().width")
        .unwrap();
    assert_eq!(width.to_num(&page.js.rt), 30.0);
    assert_eq!(pixel(&page, 4, 4), [128, 0, 128, 255]);
    page.sample_animations(3.0);
    assert_eq!(pixel(&page, 40, 4), [0, 0, 255, 255]);
    page.js
        .eval("document.getElementById('box').style.setProperty('width','20px','important')")
        .unwrap();
    page.settle();
    let width = page
        .js
        .eval("document.getElementById('box').getBoundingClientRect().width")
        .unwrap();
    assert_eq!(width.to_num(&page.js.rt), 20.0);
}

#[test]
fn named_container_queries_use_container_size_and_update_after_resizing() {
    let mut page=page_with("<style>body{margin:0} #container{container:panel / inline-size;width:80px} #box{height:20px;background:red} @container panel (width>=70px){#box{background:blue}}</style><div id=container><div id=box></div></div>");
    assert_eq!(pixel(&page, 2, 2), [0, 0, 255, 255]);
    page.js
        .eval("document.getElementById('container').style.width='40px'")
        .unwrap();
    page.settle();
    assert_eq!(pixel(&page, 2, 2), [255, 0, 0, 255]);
}

#[test]
fn transitions_sample_mutations_and_keep_the_original_target_between_frames() {
    let mut page=page_with("<style>body{margin:0} #box{width:10px;height:20px;background-color:red;transition:width 2s linear,background-color 2s linear}</style><div id=box></div>");
    page.sample_animations(0.0);
    page.js.eval("var box=document.getElementById('box');box.style.width='50px';box.style.backgroundColor='blue'").unwrap();
    page.settle();
    page.sample_animations(1.0);
    let value = page.js.eval("box.getBoundingClientRect().width").unwrap();
    assert_eq!(value.to_num(&page.js.rt), 30.0);
    assert_eq!(pixel(&page, 2, 2), [128, 0, 128, 255]);
    page.sample_animations(3.0);
    assert_eq!(pixel(&page, 40, 2), [0, 0, 255, 255]);
}

#[test]
fn generated_counters_preserve_case_and_nested_quotes_use_declared_pairs() {
    let page=page_with("<style>main{counter-reset:Section} p{counter-increment:Section} p::before{content:counter(Section) ': '} q{quotes:'[' ']' '{' '}'}</style><main><p>x</p><p>y</p><q>a<q>b</q></q></main>");
    let mut boxes = Vec::new();
    page.layout.as_ref().unwrap().root.walk(&mut boxes);
    let text: Vec<_> = boxes
        .into_iter()
        .filter_map(|box_| box_.text.as_ref().map(|text| text.text.as_str()))
        .collect();
    assert!(text.contains(&"1:"), "{text:?}");
    assert!(text.contains(&"2:"), "{text:?}");
    assert!(
        text.iter().any(|text| text.contains('[')) && text.iter().any(|text| text.contains('{')),
        "{text:?}"
    );
}

#[test]
fn two_pages_keep_independent_viewport_and_root_font_background_dimensions() {
    let mut first = Page::new(100, 50);
    first.load_html("<style>html{font-size:10px}body{margin:0}div{width:50px;height:20px;background:linear-gradient(red,red) left/1rem 100% no-repeat blue}</style><div></div>",&Url::parse("file:///tmp/first.html").unwrap()).unwrap();
    let mut second = Page::new(200, 50);
    second
        .load_html(
            "<style>html{font-size:30px}</style>",
            &Url::parse("file:///tmp/second.html").unwrap(),
        )
        .unwrap();
    assert_eq!(pixel(&first, 5, 5), [255, 0, 0, 255]);
    assert_eq!(pixel(&first, 15, 5), [0, 0, 255, 255]);
}
