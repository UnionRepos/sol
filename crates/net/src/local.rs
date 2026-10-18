//! Bounded file and data URL responses.
use crate::body::{check_body, read_limited};
use crate::{base64_decode, Client, Response, MAX_BODY};
use sol_url::Url;

impl Client {
    pub(super) fn file_response(&self, url: &Url) -> Result<Response, String> {
        let path = sol_url::pct_decode(&url.path);
        let file = std::fs::File::open(&path).map_err(|e| format!("file {}: {}", path, e))?;
        let body =
            read_limited(file, self.max_body_bytes).map_err(|e| format!("file {}: {}", path, e))?;
        let mime = match path.rsplit('.').next().unwrap_or("") {
            "html" | "htm" => "text/html",
            "css" => "text/css",
            "js" | "mjs" => "text/javascript",
            "png" => "image/png",
            "gif" => "image/gif",
            "bmp" => "image/bmp",
            "jpg" | "jpeg" => "image/jpeg",
            "svg" => "image/svg+xml",
            "txt" => "text/plain",
            "json" => "application/json",
            _ => "application/octet-stream",
        };
        Ok(Response {
            status: 200,
            reason: "OK".into(),
            headers: vec![("content-type".into(), mime.into())],
            body,
            url: url.clone(),
        })
    }
}

pub fn decode_data_url(url: &Url) -> Result<(String, Vec<u8>), String> {
    let (meta, data) = url.path.split_once(',').ok_or("bad data url")?;
    let base64 = meta
        .split(';')
        .any(|part| part.eq_ignore_ascii_case("base64"));
    let mut parts = meta
        .split(';')
        .filter(|part| !part.eq_ignore_ascii_case("base64"));
    let mime = match parts.next().unwrap_or("") {
        "" => "text/plain".to_string(),
        mime => mime.to_string(),
    };
    let parameters: Vec<_> = parts.collect();
    let mime = if parameters.is_empty() {
        mime
    } else {
        format!("{};{}", mime, parameters.join(";"))
    };
    // Image data can contain byte sequences that are not UTF-8. Routing this
    // through the URL module's text decoder would replace those bytes and
    // corrupt the payload before the image decoder sees it.
    let bytes = percent_decode_bytes(data);
    let decoded = if base64 {
        base64_decode(std::str::from_utf8(&bytes).map_err(|_| "non-ASCII base64")?)?
    } else {
        bytes
    };
    Ok((mime, check_body(decoded, MAX_BODY)?))
}

fn percent_decode_bytes(data: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut bytes = data.as_bytes().iter().copied().peekable();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let mut lookahead = bytes.clone();
            if let (Some(high), Some(low)) = (
                lookahead.next().and_then(|b| (b as char).to_digit(16)),
                lookahead.next().and_then(|b| (b as char).to_digit(16)),
            ) {
                out.push((high * 16 + low) as u8);
                bytes = lookahead;
                continue;
            }
        }
        out.push(byte);
    }
    out
}
