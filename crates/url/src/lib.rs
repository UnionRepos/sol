//! sol_url: URL parsing, normalization, relative resolution (WHATWG subset).
//!
//! Supports http(s), file, data, about schemes plus generic absolute URLs.
//! Handles dot-segment removal, percent-decoding where required, default
//! port elision, and origin computation.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub scheme: String,
    pub username: String,
    pub password: String,
    pub host: String,
    pub port: Option<u16>,
    /// Path always begins with '/' for hierarchical schemes; may be empty
    /// for opaque schemes (data:, about:blank stores payload in `path`).
    pub path: String,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

impl Url {
    pub fn parse(input: &str) -> Result<Url, String> {
        let input = input.trim();
        let (scheme, rest) = match input.find(':') {
            Some(i) if i > 0 && is_scheme(&input[..i]) => {
                (input[..i].to_ascii_lowercase(), &input[i + 1..])
            }
            _ => return Err("missing or invalid scheme".into()),
        };
        match scheme.as_str() {
            "data" => parse_data(rest),
            "about" => Ok(Url {
                scheme,
                username: String::new(),
                password: String::new(),
                host: String::new(),
                port: None,
                path: rest.split('#').next().unwrap_or("").to_string(),
                query: None,
                fragment: rest.split_once('#').map(|x| x.1.to_string()),
            }),
            _ => {
                if let Some(rest) = rest.strip_prefix("//") {
                    parse_hier(scheme, rest)
                } else if scheme == "file" {
                    // file:/path form: empty authority
                    let (path, frag) = split_frag(rest);
                    let owned;
                    let path = if path.starts_with('/') {
                        path
                    } else {
                        owned = format!("/{}", path);
                        &owned
                    };
                    Ok(Url {
                        scheme,
                        username: String::new(),
                        password: String::new(),
                        host: String::new(),
                        port: None,
                        path: normalize_path(path),
                        query: None,
                        fragment: frag.map(|s| s.to_string()),
                    })
                } else {
                    // e.g. mailto:foo@bar — opaque path
                    let (path, frag) = split_frag(rest);
                    Ok(Url {
                        scheme,
                        username: String::new(),
                        password: String::new(),
                        host: String::new(),
                        port: None,
                        path: path.to_string(),
                        query: None,
                        fragment: frag.map(|s| s.to_string()),
                    })
                }
            }
        }
    }

    /// Resolve `relative` against this URL per RFC 3986 §5.2.
    pub fn join(&self, relative: &str) -> Result<Url, String> {
        let r = relative.trim();
        if r.is_empty() {
            let mut u = self.clone();
            u.fragment = None;
            return Ok(u);
        }
        // Absolute URL?
        if let Some(i) = r.find(':') {
            if is_scheme(&r[..i]) && !r[..i].is_empty() {
                return Url::parse(r);
            }
        }
        if r.starts_with("//") {
            return Url::parse(&format!("{}:{}", self.scheme, r));
        }
        if self.scheme == "data" || self.scheme == "about" {
            return Err("cannot resolve relative URL against opaque base".into());
        }
        let mut u = self.clone();
        u.fragment = None;
        let (pathq, frag) = split_frag(r);
        u.fragment = frag.map(|s| s.to_string());
        let (p, q) = match pathq.split_once('?') {
            Some((a, b)) => (a, Some(b.to_string())),
            None => (pathq, None),
        };
        if p.is_empty() {
            if q.is_some() {
                u.query = q;
            }
            return Ok(u);
        }
        u.query = q;
        if p.starts_with('/') {
            u.path = normalize_path(p);
        } else {
            let base_dir = match self.path.rfind('/') {
                Some(i) => &self.path[..i + 1],
                None => "/",
            };
            u.path = normalize_path(&format!("{}{}", base_dir, p));
        }
        Ok(u)
    }

    pub fn origin(&self) -> String {
        match self.default_port() == self.port {
            true => format!("{}://{}", self.scheme, self.host),
            false => format!("{}://{}:{}", self.scheme, self.host, self.port.unwrap_or(0)),
        }
    }

    pub fn default_port(&self) -> Option<u16> {
        match self.scheme.as_str() {
            "http" => Some(80),
            "https" => Some(443),
            "ftp" => Some(21),
            _ => None,
        }
    }

    pub fn effective_port(&self) -> u16 {
        self.port.or_else(|| self.default_port()).unwrap_or(0)
    }

    pub fn host_and_port(&self) -> String {
        match self.port.filter(|&port| Some(port) != self.default_port()) {
            Some(port) => format!("{}:{}", self.host, port),
            None => self.host.clone(),
        }
    }

    /// Same-origin test per HTML spec (scheme/host/port triple).
    pub fn same_origin(&self, o: &Url) -> bool {
        self.scheme == o.scheme
            && self.host == o.host
            && self.effective_port() == o.effective_port()
    }

    /// Request target for HTTP: path + query, or "/" if empty.
    pub fn request_target(&self) -> String {
        let p = if self.path.is_empty() {
            "/"
        } else {
            self.path.as_str()
        };
        match &self.query {
            Some(q) => format!("{}?{}", p, q),
            None => p.to_string(),
        }
    }

    /// Serialize for display (fragment included).
    pub fn serialize(&self) -> String {
        self.to_string()
    }

    /// Percent-encode a string for use in a query component.
    pub fn encode_component(s: &str) -> String {
        let mut out = String::new();
        for &b in s.as_bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char)
                }
                b' ' => out.push_str("%20"),
                _ => out.push_str(&format!("%{:02X}", b)),
            }
        }
        out
    }
}

fn is_scheme(s: &str) -> bool {
    !s.is_empty()
        && s.as_bytes()[0].is_ascii_alphabetic()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
}

fn split_frag(s: &str) -> (&str, Option<&str>) {
    match s.split_once('#') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    }
}

fn parse_hier(scheme: String, rest: &str) -> Result<Url, String> {
    let (rest, frag) = split_frag(rest);
    let (rest, query) = match rest.split_once('?') {
        Some((a, b)) => (a, Some(b.to_string())),
        None => (rest, None),
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
        None => (None, authority),
    };
    let (username, password) = match userinfo {
        Some(ui) => match ui.split_once(':') {
            Some((u, p)) => (pct_decode(u), pct_decode(p)),
            None => (pct_decode(ui), String::new()),
        },
        None => (String::new(), String::new()),
    };
    // host:port, allow [v6addr]
    let (host, port) = if hostport.starts_with('[') {
        match hostport.find(']') {
            Some(i) => {
                let h = &hostport[..i + 1];
                let p = hostport[i + 1..]
                    .strip_prefix(':')
                    .and_then(|s| s.parse().ok());
                (h.to_string(), p)
            }
            None => (hostport.to_string(), None),
        }
    } else {
        match hostport.rfind(':') {
            Some(i) if hostport[..i].find(':').is_none() => {
                let p = hostport[i + 1..].parse::<u16>().ok();
                (hostport[..i].to_string(), p)
            }
            _ => (hostport.to_string(), None),
        }
    };
    let mut u = Url {
        username,
        password,
        host: host.to_ascii_lowercase(),
        port,
        path: normalize_path(if path.is_empty() { "/" } else { path }),
        query,
        fragment: frag.map(|s| s.to_string()),
        scheme,
    };
    if u.port == u.default_port() {
        u.port = None;
    }
    Ok(u)
}

fn parse_data(rest: &str) -> Result<Url, String> {
    let (path, frag) = split_frag(rest);
    Ok(Url {
        scheme: "data".into(),
        username: String::new(),
        password: String::new(),
        host: String::new(),
        port: None,
        path: path.to_string(),
        query: None,
        fragment: frag.map(|s| s.to_string()),
    })
}

/// RFC 3986 §5.2.4 dot-segment removal.
pub fn normalize_path(p: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    let joined = out.join("/");
    // Preserve trailing slash semantics for '..' and '.' final segments.
    if (p.ends_with("/.") || p.ends_with("/..")) && !joined.ends_with('/') {
        return joined + "/";
    }
    joined
}

pub fn pct_decode(s: &str) -> String {
    pct_decode_plus(s, false)
}

/// application/x-www-form-urlencoded decode (+ means space).
pub fn form_decode(s: &str) -> String {
    pct_decode_plus(s, true)
}

fn pct_decode_plus(s: &str, plus: bool) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = hex(b[i + 1]);
            let l = hex(b[i + 2]);
            if let (Some(h), Some(l)) = (h, l) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(if plus && b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:", self.scheme)?;
        if !self.host.is_empty() || self.scheme == "file" {
            write!(f, "//")?;
            if !self.username.is_empty() {
                write!(f, "{}", self.username)?;
                if !self.password.is_empty() {
                    write!(f, ":{}", self.password)?;
                }
                write!(f, "@")?;
            }
            write!(f, "{}", self.host)?;
            if let Some(p) = self.port {
                write!(f, ":{}", p)?;
            }
        }
        write!(f, "{}", self.path)?;
        if let Some(q) = &self.query {
            write!(f, "?{}", q)?;
        }
        if let Some(fr) = &self.fragment {
            write!(f, "#{}", fr)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic() {
        let u = Url::parse("https://user@Example.COM:8080/a/b/../c?q=1#f").unwrap();
        assert_eq!(u.scheme, "https");
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, Some(8080));
        assert_eq!(u.path, "/a/c");
        assert_eq!(u.query.as_deref(), Some("q=1"));
        assert_eq!(u.fragment.as_deref(), Some("f"));
    }

    #[test]
    fn default_port_elided() {
        let u = Url::parse("https://example.com:443/").unwrap();
        assert_eq!(u.port, None);
        assert_eq!(u.effective_port(), 443);
    }

    #[test]
    fn join_relative() {
        let b = Url::parse("https://a.com/dir/page.html?x=1").unwrap();
        assert_eq!(b.join("other.html").unwrap().path, "/dir/other.html");
        assert_eq!(b.join("/root").unwrap().path, "/root");
        assert_eq!(b.join("//cdn.com/x.css").unwrap().host, "cdn.com");
        assert_eq!(b.join("https://z.com/").unwrap().host, "z.com");
        assert_eq!(b.join("#frag").unwrap().fragment.as_deref(), Some("frag"));
        assert_eq!(b.join("?q=2").unwrap().query.as_deref(), Some("q=2"));
        let u = b.join("../up/./x").unwrap();
        assert_eq!(u.path, "/up/x");
    }

    #[test]
    fn origin() {
        let a = Url::parse("https://x.com/1").unwrap();
        let b = Url::parse("https://x.com:443/2").unwrap();
        assert!(a.same_origin(&b));
        let c = Url::parse("http://x.com/").unwrap();
        assert!(!a.same_origin(&c));
    }
}
