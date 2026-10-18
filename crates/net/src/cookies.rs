//! Cookie parsing, storage, expiry, and request scope matching.
use crate::Response;
use sol_url::Url;

#[derive(Debug, Clone)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub expires_ms: Option<u64>,
}

#[derive(Default, Clone)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

impl CookieJar {
    pub fn store_from(&mut self, resp: &Response, url: &Url) {
        for h in resp.headers_all("set-cookie") {
            self.set(h, url);
        }
    }

    pub fn set(&mut self, header: &str, url: &Url) {
        let mut parts = header.split(';');
        let Some(nv) = parts.next() else { return };
        let Some((name, value)) = nv.split_once('=') else {
            return;
        };
        let mut c = Cookie {
            name: name.trim().to_string(),
            value: value.trim().to_string(),
            domain: url.host.clone(),
            host_only: true,
            path: cookie_path(&url.path),
            secure: false,
            http_only: false,
            expires_ms: None,
        };
        for p in parts {
            let p = p.trim();
            let (k, v) = p
                .split_once('=')
                .map(|(a, b)| (a.trim(), b.trim()))
                .unwrap_or((p, ""));
            match k.to_ascii_lowercase().as_str() {
                "domain" => {
                    let d = v.trim_start_matches('.').to_ascii_lowercase();
                    if d.is_empty() || !domain_matches(&url.host, &d) {
                        return;
                    }
                    c.domain = d;
                    c.host_only = false;
                }
                "path" => {
                    c.path = if v.starts_with('/') {
                        v.to_string()
                    } else {
                        c.path.clone()
                    }
                }
                "secure" => c.secure = true,
                "httponly" => c.http_only = true,
                "max-age" => {
                    if let Ok(s) = v.parse::<i64>() {
                        c.expires_ms = Some(if s <= 0 {
                            0
                        } else {
                            sol_util::now_ms().saturating_add((s as u64).saturating_mul(1000))
                        });
                    }
                }
                _ => {}
            }
        }
        self.cookies
            .retain(|o| !(o.name == c.name && o.domain == c.domain && o.path == c.path));
        if c.expires_ms.map(|e| e > sol_util::now_ms()).unwrap_or(true) {
            self.cookies.push(c);
        }
    }

    pub fn header_for(&self, url: &Url) -> Option<String> {
        let now = sol_util::now_ms();
        let mut out = Vec::new();
        for c in &self.cookies {
            if c.expires_ms.map(|e| e <= now).unwrap_or(false) {
                continue;
            }
            if c.secure && url.scheme != "https" {
                continue;
            }
            let dom_ok = if c.host_only {
                url.host == c.domain
            } else {
                domain_matches(&url.host, &c.domain)
            };
            // A plain prefix would leak a cookie for /account to /accounting.
            // Directory boundaries distinguish related paths from shared text.
            let path_ok = url.path == c.path
                || (url.path.starts_with(&c.path)
                    && (c.path.ends_with('/')
                        || url.path.as_bytes().get(c.path.len()) == Some(&b'/')));
            if dom_ok && path_ok {
                out.push(format!("{}={}", c.name, c.value));
            }
        }
        if out.is_empty() {
            None
        } else {
            Some(out.join("; "))
        }
    }
}

fn cookie_path(url_path: &str) -> String {
    match url_path.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => url_path[..i].to_string(),
    }
}

fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain
        || (host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
            && host.parse::<std::net::IpAddr>().is_err())
}
