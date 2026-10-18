//! Text and base64 codecs shared by responses and data URLs.

/// Very small latin1-aware text decode: utf-8 (lossy) or windows-1252 for
/// declared latin1/iso-8859-1; windows-1252 high chars map to unicode.
pub fn decode_text(bytes: &[u8], charset: Option<String>) -> String {
    let cs = charset.unwrap_or_else(|| "utf-8".into());
    match cs.as_str() {
        "utf-8" | "utf8" | "us-ascii" | "ascii" => {
            String::from_utf8_lossy(strip_bom(bytes)).into_owned()
        }
        "iso-8859-1" | "latin1" | "latin-1" | "windows-1252" | "cp1252" => {
            bytes.iter().map(|&b| win1252(b)).collect()
        }
        _ => String::from_utf8_lossy(strip_bom(bytes)).into_owned(),
    }
}

fn strip_bom(b: &[u8]) -> &[u8] {
    if b.starts_with(&[0xef, 0xbb, 0xbf]) {
        &b[3..]
    } else {
        b
    }
}

fn win1252(b: u8) -> char {
    match b {
        0x80 => '€',
        0x82 => '‚',
        0x83 => 'ƒ',
        0x84 => '„',
        0x85 => '…',
        0x86 => '†',
        0x87 => '‡',
        0x88 => 'ˆ',
        0x89 => '‰',
        0x8a => 'Š',
        0x8b => '‹',
        0x8c => 'Œ',
        0x8e => 'Ž',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201c}',
        0x94 => '\u{201d}',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x98 => '˜',
        0x99 => '™',
        0x9a => 'š',
        0x9b => '›',
        0x9c => 'œ',
        0x9e => 'ž',
        0x9f => 'Ÿ',
        _ => b as char,
    }
}

pub fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    fn v(c: u8) -> Result<u32, String> {
        match c {
            b'A'..=b'Z' => Ok((c - b'A') as u32),
            b'a'..=b'z' => Ok((c - b'a' + 26) as u32),
            b'0'..=b'9' => Ok((c - b'0' + 52) as u32),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err("bad base64".into()),
        }
    }
    let b: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(b.len() * 3 / 4);
    for chunk in b.chunks(4) {
        let n = chunk.len();
        if n < 2 {
            break;
        }
        let vals: Vec<u32> = chunk
            .iter()
            .map(|&c| if c == b'=' { Ok(0) } else { v(c) })
            .collect::<Result<_, _>>()?;
        let x = (vals[0] << 18)
            | (vals[1] << 12)
            | (vals.get(2).copied().unwrap_or(0) << 6)
            | vals.get(3).copied().unwrap_or(0);
        out.push((x >> 16) as u8);
        if n > 2 && chunk[2] != b'=' {
            out.push((x >> 8) as u8);
        }
        if n > 3 && chunk[3] != b'=' {
            out.push(x as u8);
        }
    }
    Ok(out)
}

pub fn base64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0] as u32,
            *chunk.get(1).unwrap_or(&0) as u32,
            *chunk.get(2).unwrap_or(&0) as u32,
        ];
        let x = (b[0] << 16) | (b[1] << 8) | b[2];
        out.push(T[(x >> 18) as usize & 63] as char);
        out.push(T[(x >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(x >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[x as usize & 63] as char
        } else {
            '='
        });
    }
    out
}
