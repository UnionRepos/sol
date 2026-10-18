//! Shared response-body allocation limits.
use std::io::{self, Read};

pub(super) fn check_body(body: Vec<u8>, limit: usize) -> Result<Vec<u8>, String> {
    if body.len() > limit {
        Err("body too large".into())
    } else {
        Ok(body)
    }
}

pub(super) fn read_limited(reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    // Reading one extra byte distinguishes an exactly-full body from a larger
    // one. Reading exactly `limit` bytes would silently accept truncated data,
    // while an unrestricted read would allocate before discovering the excess.
    reader
        .take((limit as u64).saturating_add(1))
        .read_to_end(&mut body)?;
    check_body(body, limit).map_err(io::Error::other)
}
