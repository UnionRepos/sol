//! Route tests cross the request parser, router, views, and response writer.
use super::handle;
use sol_url::Url;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

fn request(target: &str) -> Vec<u8> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let (server, _) = listener.accept().unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    server
        .set_write_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let worker = std::thread::spawn(move || handle(server));
    write!(client, "GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).unwrap();
    worker.join().unwrap();
    response
}

#[test]
fn launcher_and_viewer_keep_links_and_escape_user_input() {
    let launcher = String::from_utf8(request("/")).unwrap();
    assert!(launcher.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(launcher.contains("Content-Type: text/html; charset=utf-8\r\n"));
    assert!(launcher.contains("<form action=/view>"));

    let url = "https://example.test/?q=\"<tag>&";
    let encoded = Url::encode_component(url);
    let viewer = String::from_utf8(request(&format!("/view?u={encoded}"))).unwrap();
    assert!(viewer.contains("https://example.test/?q=&quot;&lt;tag&gt;&amp;"));
    assert!(viewer.contains(&format!("/go?u={encoded}")));
    assert!(viewer.contains(&format!("/click?u={encoded}")));
    assert!(!viewer.contains(url));
}

#[test]
fn image_routes_return_png_with_correct_content_length() {
    let url = Url::encode_component("data:text/html,<body style='background:red'>hello</body>");
    for route in ["go", "shot"] {
        let response = request(&format!("/{route}?u={url}"));
        let boundary = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let headers = std::str::from_utf8(&response[..boundary]).unwrap();
        let body = &response[boundary + 4..];
        assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(headers.contains("Content-Type: image/png\r\n"));
        assert!(headers.contains(&format!("Content-Length: {}\r\n", body.len())));
        assert!(body.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}

#[test]
fn click_routes_accept_image_map_and_explicit_coordinates() {
    let url = "data:text/html,<body style='margin:0'><a href='#target' style='display:block;width:100px;height:100px'>jump</a><div id='target'>target</div></body>";
    let encoded = Url::encode_component(url);
    for suffix in ["?10,10", "&x=10&y=10"] {
        let response = String::from_utf8(request(&format!("/click?u={encoded}{suffix}"))).unwrap();
        assert!(response.starts_with("HTTP/1.1 302 Found\r\n"));
        assert!(response.contains(&format!("Location: /view?u={encoded}\r\n")));
    }
}

#[test]
fn load_errors_are_escaped_and_unknown_routes_return_404() {
    let error = String::from_utf8(request("/go?u=about%3A%3Cbroken%3E")).unwrap();
    assert!(error.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(error.contains("could not load about:&lt;broken&gt;"));
    assert!(error.contains("unsupported scheme: about"));
    assert_eq!(
        request("/missing"),
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"
    );
}
