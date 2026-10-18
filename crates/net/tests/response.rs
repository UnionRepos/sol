use sol_net::{base64_decode, base64_encode, Client, Response};
use sol_url::Url;

fn response(headers: &[(&str, &str)], body: &[u8]) -> Response {
    Response {
        status: 200,
        reason: "OK".into(),
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: body.to_vec(),
        url: Url::parse("https://example.test/").unwrap(),
    }
}

#[test]
fn response_headers_and_charset_are_case_insensitive_without_collapsing_repeats() {
    let response = response(
        &[
            ("Content-Type", "Text/Plain; CHARSET=\"WINDOWS-1252\""),
            ("Set-Cookie", "a=1"),
            ("set-cookie", "b=2"),
        ],
        &[0x80, b' ', 0x93, b'x', 0x94],
    );
    assert_eq!(response.mime(), "text/plain");
    assert_eq!(
        response.content_type_charset().as_deref(),
        Some("windows-1252")
    );
    assert_eq!(response.text(), "€ “x”");
    assert_eq!(response.headers_all("SET-COOKIE"), ["a=1", "b=2"]);
    assert_eq!(response.header("missing"), None);
}

#[test]
fn text_decoding_handles_bom_invalid_utf8_and_missing_metadata() {
    let response = response(&[], b"\xef\xbb\xbfhello\xff");
    assert_eq!(response.mime(), "application/octet-stream");
    assert_eq!(response.content_type_charset(), None);
    assert_eq!(response.text(), "hello\u{fffd}");
}

#[test]
fn base64_round_trips_binary_payloads_across_padding_boundaries() {
    let bytes: Vec<u8> = (0..=255).collect();
    for len in 0..=bytes.len() {
        let encoded = base64_encode(&bytes[..len]);
        assert_eq!(
            base64_decode(&encoded).unwrap(),
            bytes[..len],
            "length {len}"
        );
    }
    assert_eq!(base64_decode(" aG\nVs\tbG8= \r").unwrap(), b"hello");
    assert!(base64_decode("!!!!").is_err());
}

#[test]
fn data_body_budget_accepts_exact_limit_and_rejects_one_extra_byte() {
    let mut client = Client::new();
    client.max_body_bytes = 3;
    let exact = Url::parse("data:application/octet-stream,%00%FF%80").unwrap();
    assert_eq!(client.get(&exact).unwrap().body, [0, 255, 128]);
    let oversized = Url::parse("data:application/octet-stream,%00%FF%80%01").unwrap();
    assert_eq!(client.get(&oversized).unwrap_err(), "body too large");
    client.max_body_bytes = 0;
    assert!(client
        .get(&Url::parse("data:,").unwrap())
        .unwrap()
        .body
        .is_empty());
    assert!(client.get(&Url::parse("data:,x").unwrap()).is_err());
}
