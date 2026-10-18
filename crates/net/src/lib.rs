//! HTTP/HTTPS with ureq and rustls, bounded decompression, cookies, and local URLs.
//! The in-tree inflater remains available for image codecs.

mod body;
mod cookies;
mod encoding;
pub mod inflate;
mod local;
mod resources;
mod response;
mod transport;

pub use cookies::{Cookie, CookieJar};
pub use encoding::{base64_decode, base64_encode, decode_text};
pub use local::decode_data_url;
pub use resources::CacheStats;
pub use response::Response;

use body::check_body;
use sol_url::Url;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(25);
const MAX_REDIRECTS: u32 = 8;
const MAX_BODY: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// HTTP/HTTPS client with pooled connections, system trust, and bounded bodies.
#[derive(Clone)]
pub struct Client {
    cache: Arc<Mutex<resources::ResourceCache>>,
    cookie_events: Option<Vec<(Url, String)>>,
    agent: Option<(bool, Duration, ureq::Agent)>,
    proxy: Option<ureq::Proxy>,
    pub cookies: CookieJar,
    pub user_agent: String,
    pub extra_headers: Vec<(String, String)>,
    /// Verify HTTPS certificates and hostnames against system roots (default true).
    pub verify_tls: bool,
    /// Deadline for each HTTP exchange, including its body.
    pub timeout: Duration,
    /// Maximum decoded response size. Wire bodies are also capped at 64 MiB.
    pub max_body_bytes: usize,
}

impl Default for Client {
    fn default() -> Self {
        Self {
            cache: Arc::new(Mutex::new(resources::ResourceCache::default())),
            cookie_events: None,
            agent: None,
            proxy: ureq::Proxy::try_from_env(),
            cookies: CookieJar::default(),
            user_agent: "sol/0.1 (sol-kernel)".into(),
            extra_headers: Vec::new(),
            verify_tls: true,
            timeout: TIMEOUT,
            max_body_bytes: MAX_BODY,
        }
    }
}

impl Client {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the environment proxy for this client and reset its connection pool.
    pub fn set_proxy(&mut self, proxy: Option<ureq::Proxy>) {
        self.proxy = proxy;
        // A pooled connection retains its original route. Keeping it after a
        // proxy change could bypass the caller's newly selected network path;
        // cached bodies likewise must not stand in for a request on that path.
        self.agent = None;
        self.clear_cache();
    }

    /// Top-level GET following redirects.
    pub fn get(&mut self, url: &Url) -> Result<Response, String> {
        self.request(Request {
            method: "GET".into(),
            url: url.clone(),
            headers: vec![],
            body: vec![],
        })
    }

    pub fn request(&mut self, req: Request) -> Result<Response, String> {
        let key = self.cache_key(&req);
        if let Some(key) = &key {
            if let Some(response) = self.cached(key) {
                return Ok(response);
            }
        }
        let original = req.url.clone();
        let method = req.method.clone();
        let started = std::time::Instant::now();
        let response = self.request_uncached(req)?;
        if let Some(key) = key {
            // Redirect chains have their own freshness and origin semantics.
            // Without caching that chain explicitly, storing the final body
            // under the first URL would silently skip future redirect policy.
            if response.url == original {
                self.remember(key, &response, started.elapsed());
            }
        }
        // A successful write can make a previously fresh GET stale immediately.
        // Freshness alone cannot establish validity after the resource changes.
        if method != "GET" && method != "HEAD" && (200..400).contains(&response.status) {
            self.invalidate(&original);
        }
        Ok(response)
    }

    fn request_uncached(&mut self, mut req: Request) -> Result<Response, String> {
        for (name, value) in &self.extra_headers {
            if !req
                .headers
                .iter()
                .any(|(key, _)| key.eq_ignore_ascii_case(name))
            {
                req.headers.push((name.clone(), value.clone()));
            }
        }
        for _ in 0..=MAX_REDIRECTS {
            match req.url.scheme.as_str() {
                "data" => {
                    return decode_data_url(&req.url).and_then(|(mime, body)| {
                        Ok(Response {
                            status: 200,
                            reason: "OK".into(),
                            headers: vec![("content-type".into(), mime)],
                            body: check_body(body, self.max_body_bytes)?,
                            url: req.url.clone(),
                        })
                    })
                }
                "file" => return self.file_response(&req.url),
                "http" | "https" => {}
                s => return Err(format!("unsupported scheme: {}", s)),
            }
            let resp = self.single(&req)?;
            match resp.status {
                301 | 302 | 303 | 307 | 308 => {
                    if let Some(loc) = resp.header("location") {
                        let next = resp.url.join(loc).map_err(|e| e.to_string())?;
                        // A remote response must not turn an HTTP request into
                        // a local file read or move protected traffic onto an
                        // unencrypted connection through a Location header.
                        if !matches!(next.scheme.as_str(), "http" | "https") {
                            return Err("HTTP redirects must target http or https".into());
                        }
                        if req.url.scheme == "https" && next.scheme != "https" {
                            return Err("refusing HTTPS downgrade redirect".into());
                        }
                        // Credentials authorize the original origin, not an
                        // arbitrary redirect recipient. Host must also follow
                        // the new target rather than the caller's old header.
                        if !req.url.same_origin(&next) {
                            req.headers.retain(|(key, _)| {
                                !["authorization", "proxy-authorization", "cookie", "host"]
                                    .iter()
                                    .any(|name| key.eq_ignore_ascii_case(name))
                            });
                        }
                        if (resp.status == 303 && req.method != "HEAD")
                            || (matches!(resp.status, 301 | 302) && req.method == "POST")
                        {
                            req.method = "GET".into();
                            req.body.clear();
                            // Body metadata left on the rewritten GET could
                            // describe bytes that no longer exist and confuse
                            // request framing at the next server.
                            req.headers.retain(|(key, _)| {
                                !["content-length", "content-type", "transfer-encoding"]
                                    .iter()
                                    .any(|name| key.eq_ignore_ascii_case(name))
                            });
                        }
                        req.url = next;
                        continue;
                    }
                    return Ok(resp);
                }
                _ => return Ok(resp),
            }
        }
        Err("too many redirects".into())
    }
}
