use sol_net::{Client, CookieJar, Request};
use sol_url::Url;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct Received {
    line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}
impl Received {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

fn accept(listener: &TcpListener) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                return stream;
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(1))
            }
            Err(error) => panic!("accept: {error}"),
        }
    }
}

fn receive(reader: &mut impl BufRead) -> Received {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        if header.trim().is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').unwrap();
        headers.push((name.to_ascii_lowercase(), value.trim().into()));
    }
    let len = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value): &(String, String)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; len];
    reader.read_exact(&mut body).unwrap();
    Received {
        line: line.trim().into(),
        headers,
        body,
    }
}

fn serve(responses: Vec<Vec<u8>>) -> (Url, JoinHandle<Vec<Received>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let server = thread::spawn(move || {
        responses
            .into_iter()
            .map(|response| {
                let mut reader = BufReader::new(accept(&listener));
                let request = receive(&mut reader);
                let _ = reader.get_mut().write_all(&response);
                request
            })
            .collect()
    });
    (url, server)
}

fn client() -> Client {
    let mut client = Client::new();
    client.set_proxy(None);
    client.timeout = Duration::from_secs(2);
    client
}

fn response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

#[test]
fn default_client_verifies_tls() {
    assert!(Client::default().verify_tls);
    assert!(Client::new().verify_tls);
}

#[test]
fn status_errors_are_returned_as_responses() {
    let (url, server) = serve(vec![response("404 Not Found", "", b"missing")]);
    let resp = client().get(&url).unwrap();
    assert_eq!(resp.status, 404);
    assert_eq!(resp.body, b"missing");
    server.join().unwrap();
}

#[test]
fn post_302_redirect_becomes_get_and_preserves_cookies() {
    let (url, server) = serve(vec![
        response(
            "302 Found",
            "Location: /final\r\nSet-Cookie: session=ok; Path=/\r\n",
            b"",
        ),
        response("200 OK", "", b"done"),
    ]);
    let resp = client()
        .request(Request {
            method: "POST".into(),
            url,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: b"ping".to_vec(),
        })
        .unwrap();
    assert_eq!(resp.url.path, "/final");
    let received = server.join().unwrap();
    assert_eq!(received[0].line, "POST / HTTP/1.1");
    assert_eq!(received[0].body, b"ping");
    assert_eq!(received[1].line, "GET /final HTTP/1.1");
    assert!(received[1].body.is_empty());
    assert_eq!(received[1].header("content-type"), None);
    assert_eq!(received[1].header("cookie"), Some("session=ok"));
}

#[test]
fn redirect_307_preserves_method_and_body() {
    let (url, server) = serve(vec![
        response("307 Temporary Redirect", "Location: /final\r\n", b""),
        response("200 OK", "", b"ok"),
    ]);
    client()
        .request(Request {
            method: "POST".into(),
            url,
            headers: vec![],
            body: b"ping".to_vec(),
        })
        .unwrap();
    let received = server.join().unwrap();
    assert_eq!(received[1].line, "POST /final HTTP/1.1");
    assert_eq!(received[1].body, b"ping");
}

#[test]
fn cross_origin_redirect_strips_sensitive_headers() {
    let (target, second) = serve(vec![response("200 OK", "", b"ok")]);
    let (url, first) = serve(vec![response(
        "302 Found",
        &format!("Location: {}\r\n", target.serialize()),
        b"",
    )]);
    let mut c = client();
    c.extra_headers = vec![
        ("Authorization".into(), "Bearer secret".into()),
        ("Cookie".into(), "secret=yes".into()),
    ];
    c.get(&url).unwrap();
    let sent = first.join().unwrap();
    assert_eq!(sent[0].header("authorization"), Some("Bearer secret"));
    let sent = second.join().unwrap();
    assert_eq!(sent[0].header("authorization"), None);
    assert_eq!(sent[0].header("cookie"), None);
}

#[test]
fn http_redirects_cannot_read_local_files() {
    let (url, server) = serve(vec![response(
        "302 Found",
        "Location: file:///etc/passwd\r\n",
        b"",
    )]);
    assert!(client()
        .get(&url)
        .unwrap_err()
        .contains("redirects must target"));
    server.join().unwrap();
}

#[test]
fn connection_reuse_handles_head_and_no_content() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let server = thread::spawn(move || {
        let mut reader = BufReader::new(accept(&listener));
        let first = receive(&mut reader);
        assert_eq!(first.line, "HEAD / HTTP/1.1");
        reader
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 999\r\n\r\n")
            .unwrap();
        receive(&mut reader);
        reader
            .get_mut()
            .write_all(b"HTTP/1.1 204 No Content\r\n\r\n")
            .unwrap();
        receive(&mut reader);
        reader
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .unwrap();
    });
    let mut c = client();
    assert!(c
        .request(Request {
            method: "HEAD".into(),
            url: url.clone(),
            headers: vec![],
            body: vec![]
        })
        .unwrap()
        .body
        .is_empty());
    assert_eq!(c.get(&url).unwrap().status, 204);
    assert_eq!(c.get(&url).unwrap().body, b"ok");
    server.join().unwrap();
}

#[test]
fn chunked_and_informational_responses_are_supported() {
    let (url, server) = serve(vec![b"HTTP/1.1 103 Early Hints\r\nLink: </a.css>\r\n\r\nHTTP/1.1 200 OK\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n2;name=value\r\nhe\r\n3\r\nllo\r\n0\r\nX-Trailer: yes\r\n\r\n".to_vec()]);
    assert_eq!(client().get(&url).unwrap().body, b"hello");
    server.join().unwrap();
}

#[test]
fn gzip_and_deflate_are_decoded_with_output_limits() {
    use flate2::{
        write::{DeflateEncoder, GzEncoder, ZlibEncoder},
        Compression,
    };
    let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
    gzip.write_all(b"hello").unwrap();
    let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
    zlib.write_all(b"hello").unwrap();
    let mut raw = DeflateEncoder::new(Vec::new(), Compression::default());
    raw.write_all(b"hello").unwrap();
    for (encoding, bytes) in [
        ("gzip", gzip.finish().unwrap()),
        ("deflate", zlib.finish().unwrap()),
        ("deflate", raw.finish().unwrap()),
    ] {
        let (url, server) = serve(vec![response(
            "200 OK",
            &format!("Content-Encoding: {encoding}\r\n"),
            &bytes,
        )]);
        let mut c = client();
        c.max_body_bytes = 5;
        assert_eq!(c.get(&url).unwrap().body, b"hello");
        server.join().unwrap();
        let (url, server) = serve(vec![response(
            "200 OK",
            &format!("Content-Encoding: {encoding}\r\n"),
            &bytes,
        )]);
        c.max_body_bytes = 4;
        assert!(c.get(&url).unwrap_err().contains("body too large"));
        server.join().unwrap();
    }
}

#[test]
fn body_limits_and_truncation_are_errors() {
    for bytes in [
        response("200 OK", "", b"hello"),
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhi".to_vec(),
        b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nhello".to_vec(),
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n".to_vec(),
    ] {
        let (url, server) = serve(vec![bytes]);
        let mut c = client();
        c.max_body_bytes = 4;
        assert!(c.get(&url).is_err());
        server.join().unwrap();
    }
}

#[test]
fn invalid_headers_are_rejected() {
    let url = Url::parse("http://127.0.0.1:1/").unwrap();
    let error = client()
        .request(Request {
            method: "GET".into(),
            url,
            headers: vec![("x-test".into(), "ok\r\nInjected: yes".into())],
            body: vec![],
        })
        .unwrap_err();
    assert!(error.contains("header"));
}

#[test]
fn data_urls_preserve_binary_bytes_and_charset() {
    let response = client()
        .get(&Url::parse("data:application/octet-stream,%00%FF%80%2B").unwrap())
        .unwrap();
    assert_eq!(response.body, [0, 255, 128, b'+']);
    let response = client()
        .get(&Url::parse("data:text/plain;charset=windows-1252,%80").unwrap())
        .unwrap();
    assert_eq!(response.text(), "€");
    assert_eq!(
        client()
            .get(&Url::parse("data:;base64,aGVsbG8=").unwrap())
            .unwrap()
            .body,
        b"hello"
    );
}

#[test]
fn cookie_scope_checks_domain_path_and_expiry() {
    let url = Url::parse("https://example.com/account/page").unwrap();
    let mut jar = CookieJar::default();
    jar.set("host=yes; Path=/account", &url);
    assert_eq!(jar.header_for(&url).as_deref(), Some("host=yes"));
    assert_eq!(
        jar.header_for(&Url::parse("https://sub.example.com/account/").unwrap()),
        None
    );
    assert_eq!(
        jar.header_for(&Url::parse("https://example.com/accountant").unwrap()),
        None
    );
    jar.set("bad=yes; Domain=ample.com; Path=/", &url);
    jar.set("bad=yes; Domain=; Path=/", &url);
    jar.set("domain=yes; Domain=example.com; Path=/; Secure", &url);
    assert_eq!(
        jar.header_for(&Url::parse("https://sub.example.com/").unwrap())
            .as_deref(),
        Some("domain=yes")
    );
    assert_eq!(
        jar.header_for(&Url::parse("http://sub.example.com/").unwrap()),
        None
    );
    jar.set("domain=gone; Domain=example.com; Path=/; Max-Age=0", &url);
    assert_eq!(
        jar.header_for(&Url::parse("https://sub.example.com/").unwrap()),
        None
    );
    jar.set("long=yes; Max-Age=9223372036854775807; Path=/", &url);
    assert!(jar.header_for(&url).unwrap().contains("long=yes"));
}

#[test]
fn response_deadlines_cover_stalled_bodies() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let server = thread::spawn(move || {
        let mut reader = BufReader::new(accept(&listener));
        receive(&mut reader);
        reader
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n")
            .unwrap();
        thread::sleep(Duration::from_millis(300));
    });
    let mut c = client();
    c.timeout = Duration::from_millis(50);
    assert!(c.get(&url).is_err());
    server.join().unwrap();
}

#[test]
fn https_rejects_untrusted_certificates_and_rebuilds_configuration() {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use std::io::Read;
    use std::sync::Arc;
    let cert = CertificateDer::from(include_bytes!("fixtures/localhost.der").to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        include_bytes!("fixtures/test-key.der").to_vec(),
    ));
    let config = Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .unwrap(),
    );
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = Url::parse(&format!("https://{}/", listener.local_addr().unwrap())).unwrap();
    let server = thread::spawn(move || {
        for verified in [true, false] {
            let connection = rustls::ServerConnection::new(config.clone()).unwrap();
            let mut stream = rustls::StreamOwned::new(connection, accept(&listener));
            if verified {
                assert!(stream.read(&mut [0; 1]).is_err());
            } else {
                receive(&mut BufReader::new(&mut stream));
                stream
                    .write_all(&response("200 OK", "", b"encrypted"))
                    .unwrap();
                stream.flush().unwrap();
            }
        }
    });
    let mut c = client();
    assert!(c.get(&url).is_err());
    // Changing the existing public switch must discard the previous agent.
    c.verify_tls = false;
    assert_eq!(c.get(&url).unwrap().body, b"encrypted");
    server.join().unwrap();
}

#[test]
fn fresh_gets_are_cached_but_no_store_and_credentials_are_not() {
    let (url, server) = serve(vec![response(
        "200 OK",
        "Cache-Control: max-age=60\r\n",
        b"cached",
    )]);
    let mut c = client();
    assert_eq!(c.get(&url).unwrap().body, b"cached");
    server.join().unwrap();
    assert_eq!(c.get(&url).unwrap().body, b"cached");
    assert_eq!(c.cache_stats().hits, 1);
    assert_eq!(c.cache_stats().bytes, 6);
    c.max_body_bytes = 2;
    assert!(c.get(&url).is_err());
    c.clear_cache();
    assert_eq!(c.cache_stats().bytes, 0);
    for header in [
        "Cache-Control: no-store, max-age=60\r\n",
        "Cache-Control: max-age=0\r\n",
        "Cache-Control: max-age=60\r\nVary: *\r\n",
        "Cache-Control: max-age=60\r\nAge: 120\r\n",
    ] {
        let (url, server) = serve(vec![
            response("200 OK", header, b"one"),
            response("200 OK", header, b"two"),
        ]);
        let mut c = client();
        assert_eq!(c.get(&url).unwrap().body, b"one");
        assert_eq!(c.get(&url).unwrap().body, b"two");
        assert_eq!(c.cache_stats().hits, 0);
        assert_eq!(server.join().unwrap().len(), 2);
    }
    let (url, server) = serve(vec![
        response("200 OK", "Cache-Control: max-age=60\r\n", b"one"),
        response("200 OK", "Cache-Control: max-age=60\r\n", b"two"),
    ]);
    let mut c = client();
    c.extra_headers
        .push(("Authorization".into(), "Bearer private".into()));
    assert_eq!(c.get(&url).unwrap().body, b"one");
    assert_eq!(c.get(&url).unwrap().body, b"two");
    assert_eq!(c.cache_stats().entries, 0);
    server.join().unwrap();
}

#[test]
fn cache_varies_by_headers_and_invalidates_after_post() {
    let (url, server) = serve(vec![
        response(
            "200 OK",
            "Cache-Control: max-age=60\r\nVary: Accept-Language\r\n",
            b"en",
        ),
        response(
            "200 OK",
            "Cache-Control: max-age=60\r\nVary: Accept-Language\r\n",
            b"fr",
        ),
        response("200 OK", "", b"posted"),
        response("200 OK", "", b"updated"),
    ]);
    let mut c = client();
    c.extra_headers = vec![("Accept-Language".into(), "en".into())];
    assert_eq!(c.get(&url).unwrap().body, b"en");
    c.extra_headers = vec![("Accept-Language".into(), "fr".into())];
    assert_eq!(c.get(&url).unwrap().body, b"fr");
    c.extra_headers = vec![("Accept-Language".into(), "en".into())];
    assert_eq!(c.get(&url).unwrap().body, b"en");
    c.request(Request {
        method: "POST".into(),
        url: url.clone(),
        headers: vec![],
        body: vec![],
    })
    .unwrap();
    assert_eq!(c.get(&url).unwrap().body, b"updated");
    assert_eq!(server.join().unwrap().len(), 4);
}

#[test]
fn parallel_batches_are_bounded_deduplicated_and_return_input_order() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier,
    };
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let maximum = Arc::new(AtomicUsize::new(0));
    let peak = maximum.clone();
    let server = thread::spawn(move || {
        let active = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..6 {
            let stream = accept(&listener);
            let active = active.clone();
            let barrier = barrier.clone();
            let peak = peak.clone();
            handles.push(thread::spawn(move || {
                let mut reader = BufReader::new(stream);
                let request = receive(&mut reader);
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                barrier.wait();
                thread::sleep(Duration::from_millis(5));
                let path = request.line.split_whitespace().nth(1).unwrap();
                active.fetch_sub(1, Ordering::SeqCst);
                reader
                    .get_mut()
                    .write_all(&response("200 OK", "", path.as_bytes()))
                    .unwrap();
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
    });
    let paths = ["0", "1", "0", "2", "3", "4", "5", "1"];
    let urls: Vec<_> = paths.iter().map(|path| base.join(path).unwrap()).collect();
    let responses = client().get_many(&urls, 2);
    for (response, path) in responses.into_iter().zip(paths) {
        assert_eq!(response.unwrap().body, format!("/{path}").as_bytes());
    }
    server.join().unwrap();
    assert_eq!(maximum.load(Ordering::SeqCst), 2);
}

#[test]
fn parallel_cookie_effects_are_committed_in_input_order() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let mut reader = BufReader::new(accept(&listener));
            let request = receive(&mut reader);
            requests.push((reader, request));
        }
        requests.sort_by(|a, b| b.1.line.cmp(&a.1.line));
        for (mut reader, request) in requests {
            let value = if request.line.contains("/first ") {
                "first"
            } else {
                "second"
            };
            reader
                .get_mut()
                .write_all(&response(
                    "200 OK",
                    &format!("Set-Cookie: order={value}; Path=/\r\n"),
                    b"ok",
                ))
                .unwrap();
        }
    });
    let mut c = client();
    let results = c.get_many(
        &[base.join("first").unwrap(), base.join("second").unwrap()],
        2,
    );
    assert!(results.iter().all(Result::is_ok));
    assert_eq!(c.cookies.header_for(&base).as_deref(), Some("order=second"));
    server.join().unwrap();
}

#[test]
fn cache_uses_http_dates_and_explicit_expiration() {
    for headers in [
        "Cache-Control: max-age=60\r\nDate: Sun, 06 Nov 1994 08:49:37 GMT\r\n",
        "Expires: Sun, 06 Nov 1994 08:49:37 GMT\r\n",
    ] {
        let (url, server) = serve(vec![
            response("200 OK", headers, b"one"),
            response("200 OK", headers, b"two"),
        ]);
        let mut c = client();
        assert_eq!(c.get(&url).unwrap().body, b"one");
        assert_eq!(c.get(&url).unwrap().body, b"two");
        server.join().unwrap();
    }
    let date = std::time::SystemTime::now();
    let headers = format!(
        "Date: {}\r\nExpires: {}\r\n",
        httpdate::fmt_http_date(date),
        httpdate::fmt_http_date(date + Duration::from_secs(60))
    );
    let (url, server) = serve(vec![response("200 OK", &headers, b"fresh")]);
    let mut c = client();
    assert_eq!(c.get(&url).unwrap().body, b"fresh");
    server.join().unwrap();
    assert_eq!(c.get(&url).unwrap().body, b"fresh");
    assert_eq!(c.cache_stats().hits, 1);
}
