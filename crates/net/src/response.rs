//! HTTP response metadata and text interpretation.
use crate::decode_text;
use sol_url::Url;

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Final URL after redirects.
    pub url: Url,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    pub fn headers_all(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect()
    }
    pub fn text(&self) -> String {
        decode_text(&self.body, self.content_type_charset())
    }
    pub fn content_type_charset(&self) -> Option<String> {
        let ct = self.header("content-type")?;
        for part in ct.split(';').skip(1) {
            let p = part.trim();
            if let Some(v) = p.strip_prefix("charset=") {
                return Some(v.trim_matches('"').to_ascii_lowercase());
            }
            if p.to_ascii_lowercase().starts_with("charset=") {
                return Some(p[8..].trim_matches('"').to_ascii_lowercase());
            }
        }
        None
    }
    pub fn mime(&self) -> String {
        self.header("content-type")
            .map(|c| {
                c.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            })
            .unwrap_or_else(|| "application/octet-stream".into())
    }
}
