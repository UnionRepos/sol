//! Minimal DER (ASN.1) reader for X.509 and TLS structures.

#[derive(Debug, Clone)]
pub struct Tlv<'a> {
    /// Full tag byte including class+constructed bits.
    pub tag: u8,
    pub value: &'a [u8],
    /// Raw encoded TLV (header + value).
    pub raw: &'a [u8],
}

pub fn read_tlv<'a>(d: &'a [u8]) -> Result<(Tlv<'a>, &'a [u8]), String> {
    if d.len() < 2 {
        return Err("der: short".into());
    }
    let tag = d[0];
    let lb = d[1];
    let (len, hdr) = if lb & 0x80 == 0 {
        (lb as usize, 2)
    } else {
        let n = (lb & 0x7f) as usize;
        if n == 0 || n > 4 || d.len() < 2 + n {
            return Err("der: bad length".into());
        }
        let mut l = 0usize;
        for i in 0..n {
            l = (l << 8) | d[2 + i] as usize;
        }
        (l, 2 + n)
    };
    if d.len() < hdr + len {
        return Err("der: truncated".into());
    }
    Ok((
        Tlv {
            tag,
            value: &d[hdr..hdr + len],
            raw: &d[..hdr + len],
        },
        &d[hdr + len..],
    ))
}

/// Iterate the children of a constructed value.
pub struct Iter<'a> {
    d: &'a [u8],
}

impl<'a> Iterator for Iter<'a> {
    type Item = Tlv<'a>;
    fn next(&mut self) -> Option<Tlv<'a>> {
        if self.d.is_empty() {
            return None;
        }
        match read_tlv(self.d) {
            Ok((t, rest)) => {
                self.d = rest;
                Some(t)
            }
            Err(_) => {
                self.d = &[];
                None
            }
        }
    }
}

pub fn children(d: &[u8]) -> Iter<'_> {
    Iter { d }
}

/// Parse OID content bytes to dotted form, e.g. "1.2.840.113549.1.1.11".
pub fn oid_str(v: &[u8]) -> String {
    if v.is_empty() {
        return String::new();
    }
    let mut parts = vec![(v[0] / 40) as u32, (v[0] % 40) as u32];
    let mut cur = 0u32;
    for &b in &v[1..] {
        cur = (cur << 7) | (b & 0x7f) as u32;
        if b & 0x80 == 0 {
            parts.push(cur);
            cur = 0;
        }
    }
    parts
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(".")
}

/// Decode INTEGER to bytes (strip leading 0x00 sign pad).
pub fn int_bytes(v: &[u8]) -> &[u8] {
    if v.len() > 1 && v[0] == 0 {
        &v[1..]
    } else {
        v
    }
}

/// Bit string content → raw bytes (asserts 0 unused bits).
pub fn bit_string(v: &[u8]) -> Result<&[u8], String> {
    if v.is_empty() || v[0] != 0 {
        return Err("der: bad bit string".into());
    }
    Ok(&v[1..])
}

/// UTF8String / PrintableString / IA5String / T61 → String (lossy).
pub fn to_string(v: &[u8]) -> String {
    String::from_utf8_lossy(v).into_owned()
}
