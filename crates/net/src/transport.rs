//! Pooled HTTP/TLS transport and bounded response decompression.
use crate::body::read_limited;
use crate::{Client, Request, Response, MAX_BODY};

impl Client {
    pub(super) fn agent(&mut self) -> &ureq::Agent {
        // These settings are public and can change between requests. A pooled
        // agent captures them at creation, so reuse is valid only while its
        // trust policy and deadline still match the caller's configuration.
        if self.agent.as_ref().is_none_or(|(verify, timeout, _)| {
            *verify != self.verify_tls || *timeout != self.timeout
        }) {
            let tls = ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .disable_verification(!self.verify_tls)
                .build();
            // Redirects belong to the client policy layer so each hop passes
            // the same origin, credential, scheme, and method checks. Automatic
            // transport redirects would bypass those checks.
            let agent = ureq::Agent::config_builder()
                .proxy(self.proxy.clone())
                .tls_config(tls)
                .http_status_as_error(false)
                .max_redirects(0)
                .timeout_global(Some(self.timeout))
                .max_response_header_size(65536)
                .build()
                .new_agent();
            self.agent = Some((self.verify_tls, self.timeout, agent));
        }
        &self.agent.as_ref().unwrap().2
    }

    pub(super) fn single(&mut self, req: &Request) -> Result<Response, String> {
        let uri = format!(
            "{}://{}{}",
            req.url.scheme,
            req.url.host_and_port(),
            req.url.request_target()
        );
        let mut request = ureq::http::Request::builder()
            .method(req.method.as_str())
            .uri(uri)
            .body(req.body.as_slice())
            .map_err(|e| e.to_string())?;
        let headers = request.headers_mut();
        for (key, value) in &req.headers {
            let key =
                ureq::http::HeaderName::from_bytes(key.as_bytes()).map_err(|e| e.to_string())?;
            let value = ureq::http::HeaderValue::from_str(value).map_err(|e| e.to_string())?;
            headers.append(key, value);
        }
        for (name, value) in [
            ("user-agent", self.user_agent.as_str()),
            (
                "accept",
                "text/html,application/xhtml+xml,image/png,image/*,*/*;q=0.8",
            ),
            ("accept-encoding", "gzip, br, deflate"),
            ("accept-language", "en-US,en;q=0.9"),
        ] {
            if !headers.contains_key(name) {
                headers.insert(
                    ureq::http::HeaderName::from_static(name),
                    value
                        .parse()
                        .map_err(|e: ureq::http::header::InvalidHeaderValue| e.to_string())?,
                );
            }
        }
        if !headers.contains_key("cookie") {
            if let Some(cookie) = self.cookies.header_for(&req.url) {
                headers.insert(
                    "cookie",
                    cookie
                        .parse()
                        .map_err(|e: ureq::http::header::InvalidHeaderValue| e.to_string())?,
                );
            }
        }
        let mut response = self.agent().run(request).map_err(|e| e.to_string())?;
        let status = response.status();
        let headers = response
            .headers()
            .iter()
            .map(|(key, value)| {
                (
                    key.as_str().to_string(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let mut resp = Response {
            status: status.as_u16(),
            reason: status.canonical_reason().unwrap_or("").into(),
            headers,
            body: Vec::new(),
            url: req.url.clone(),
        };
        // Content-Length cannot bound decompressed allocations: a small payload
        // can expand dramatically. Independent encoded and decoded limits also
        // cover missing or dishonest length headers before bodies are retained.
        let reader = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY as u64 + 1)
            .reader();
        let deflate = req.method != "HEAD"
            && !matches!(resp.status, 204 | 304)
            && resp
                .header("content-encoding")
                .is_some_and(|value| value.eq_ignore_ascii_case("deflate"));
        let body = read_limited(
            reader,
            if deflate {
                MAX_BODY
            } else {
                self.max_body_bytes
            },
        )
        .map_err(|e| e.to_string())?;
        resp.body = if deflate {
            // Servers use both zlib-wrapped and raw DEFLATE for this encoding.
            // Assuming a wrapper would reject raw streams, while assuming raw
            // bits would misinterpret a valid wrapper as compressed data.
            let zlib = body.len() >= 2
                && body[0] & 0x0f == 8
                && u16::from_be_bytes([body[0], body[1]]) % 31 == 0;
            if zlib {
                read_limited(
                    flate2::read::ZlibDecoder::new(body.as_slice()),
                    self.max_body_bytes,
                )
            } else {
                read_limited(
                    flate2::read::DeflateDecoder::new(body.as_slice()),
                    self.max_body_bytes,
                )
            }
            .map_err(|e| e.to_string())?
        } else {
            body
        };
        self.cookies.store_from(&resp, &req.url);
        if let Some(events) = &mut self.cookie_events {
            for value in resp.headers_all("set-cookie") {
                events.push((req.url.clone(), value.into()));
            }
        }
        Ok(resp)
    }
}
