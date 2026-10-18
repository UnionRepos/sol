//! Minimal GET request framing and HTTP responses for the remote window.
use std::io::{BufRead, Write};

pub(super) fn read_target(r: &mut impl BufRead) -> Option<String> {
    let mut line = String::new();
    if r.read_line(&mut line).is_err() {
        return None;
    }
    // Every route expresses its input in the target URL. Accepting other
    // methods without defining their body semantics would imply support the
    // viewer's protocol does not provide.
    let mut parts = line.split_whitespace();
    if parts.next() != Some("GET") {
        return None;
    }
    let target = parts.next()?;
    // Finish the request header section before writing a response so ordinary
    // clients can complete their request rather than lose the connection while
    // they are still sending headers.
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).is_err() || h.trim().is_empty() {
            break;
        }
    }
    Some(target.to_string())
}

pub(super) fn respond(mut s: std::net::TcpStream, mime: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
    let _ = s.flush();
}

pub(super) fn respond_html(s: std::net::TcpStream, body: &str) {
    respond(s, "text/html; charset=utf-8", body.as_bytes());
}

pub(super) fn respond_redirect(mut s: std::net::TcpStream, loc: &str) {
    let head = format!(
        "HTTP/1.1 302 Found\r\nLocation: {loc}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.flush();
}

pub(super) fn respond_not_found(mut s: std::net::TcpStream) {
    let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
}
