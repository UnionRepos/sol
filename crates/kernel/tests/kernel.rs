//! End-to-end kernel tests: parse -> style -> layout -> paint -> rasterize.
use sol_kernel::Page;
use sol_url::Url;

fn page_with(html: &str) -> Page {
    let mut p = Page::new(800, 600);
    let u = Url::parse("file:///tmp/test.html").unwrap();
    p.load_html(html, &u).unwrap();
    p
}

#[test]
fn full_pipeline_renders() {
    let p = page_with(
        r#"<!doctype html><html><head><title>T</title>
        <style>body{background:#ff0000} h1{color:#00ff00}</style>
        </head><body><h1>Hello</h1><p>world</p></body></html>"#,
    );
    assert_eq!(p.title, "T");
    let png = p.render_png();
    assert!(png.starts_with(b"\x89PNG"));
    let img = sol_img::decode(&png).unwrap();
    assert_eq!(img.w, 800);
    assert_eq!(img.h, 600);
    // body bg is red — top-left pixel should be red
    assert_eq!(&img.px[0..3], &[255, 0, 0]);
}

#[test]
fn dom_script_mutations_reflow() {
    let mut p = page_with(
        r#"<!doctype html><html><body><div id="a">x</div>
        <script>
          document.getElementById("a").textContent = "changed";
          document.body.setAttribute("data-ok", "1");
        </script></body></html>"#,
    );
    p.settle();
    let html = p.outer_html();
    assert!(html.contains("changed"), "script mutation missing: {html}");
    assert!(html.contains("data-ok=\"1\""));
}

#[test]
fn hit_testing_finds_elements() {
    let mut p = page_with(
        r#"<!doctype html><html><body>
        <div id="box" style="position:absolute;left:50px;top:50px;width:100px;height:100px"></div>
        </body></html>"#,
    );
    p.settle();
    let hit = p.hit(60.0, 60.0);
    assert!(hit.is_some(), "hit at 60,60 should find #box");
    let d = p.doc.borrow();
    let el = d.node(hit.unwrap()).element().unwrap();
    assert_eq!(el.attr("id"), Some("box"));
    assert!(p.hit(500.0, 500.0).is_none());
}

#[test]
fn data_url_page() {
    let mut p = Page::new(400, 300);
    let u = Url::parse("data:text/html,<h1>hi</h1>").unwrap();
    p.open_url(&u).unwrap();
    assert!(!p.outer_html().is_empty());
}

#[test]
fn transformed_hit_testing_and_dom_rects_match_painting() {
    let p=page_with("<body style='margin:0'><div id='target' style='width:20px;height:20px;transform-origin:0 0;transform:translate(60px,10px) scale(2);background:red'></div></body>");
    let node = p.doc.borrow().get_element_by_id("target").unwrap();
    assert_eq!(p.hit(70.0, 20.0), Some(node));
    assert_ne!(p.hit(5.0, 5.0), Some(node));
    let mut p = p;
    let value =
        p.js.eval("document.getElementById('target').getBoundingClientRect().left")
            .unwrap();
    assert_eq!(value.to_num(&p.js.rt), 60.0);
}

#[test]
fn parallel_resources_preserve_css_script_order_and_reuse_cache() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let received = requests.clone();
    let server = std::thread::spawn(move || {
        let mut handles = Vec::new();
        for _ in 0..4 {
            let deadline = Instant::now() + Duration::from_secs(10);
            let stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            let received = received.clone();
            handles.push(std::thread::spawn(move || {
                stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut reader=BufReader::new(stream);let mut line=String::new();reader.read_line(&mut line).unwrap();
                let path=line.split_whitespace().nth(1).unwrap().to_string();
                loop {let mut header=String::new();reader.read_line(&mut header).unwrap();if header.trim().is_empty(){break;}}
                received.lock().unwrap().push(path.clone());
                let (mime,body):(&str,&[u8])=match path.as_str() {
                    "/a.css"=>("text/css",b"#box { background:red }") ,
                    "/a.js"=>("text/javascript",b"var sequence='A';"),
                    "/b.js"=>("text/javascript",b"document.body.setAttribute('data-sequence',sequence+'B');"),
                    "/pic.bmp"=>("image/bmp",include_bytes!("../../img/tests/t2x2.bmp")),
                    _=>panic!("unexpected path {path}"),
                };
                write!(reader.get_mut(),"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {mime}\r\nCache-Control: max-age=60\r\nConnection: close\r\n\r\n",body.len()).unwrap();
                reader.get_mut().write_all(body).unwrap();
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
    });
    let html="<body style='margin:0'><link rel='stylesheet' href='a.css'><style>#box{background:blue}</style><div id='box' style='width:20px;height:20px'></div><img src='pic.bmp'><img src='pic.bmp'><script src='a.js'></script><script>sequence+='I';</script><script src='b.js'></script></body>";
    let mut page = Page::new(100, 100);
    page.network_client().set_proxy(None);
    page.resource_parallelism = 2;
    page.load_html(html, &base).unwrap();
    assert!(page.outer_html().contains("data-sequence=\"AIB\""));
    let canvas = page.render();
    let offset = (10 * canvas.w + 10) as usize * 4;
    assert_eq!(&canvas.px[offset..offset + 4], &[0, 0, 255, 255]);
    server.join().unwrap();
    let paths = requests.lock().unwrap();
    assert_eq!(paths.iter().filter(|path| *path == "/pic.bmp").count(), 1);
    drop(paths);
    page.load_html(html, &base).unwrap();
    assert!(page.outer_html().contains("data-sequence=\"AIB\""));
    assert_eq!(page.network_client().cache_stats().hits, 4);
}
