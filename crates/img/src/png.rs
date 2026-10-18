//! PNG decode (all color types, filters, Adam7) + PNG encode.

use crate::Image;

pub fn decode(data: &[u8]) -> Result<Image, String> {
    crate::input(data)?;
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("not png".into());
    }
    let mut pos = 8usize;
    let (mut w, mut h, mut bit_depth, mut color_type, mut interlace) = (0, 0, 0, 0, 0);
    let mut idat = Vec::new();
    let mut plte: Vec<[u8; 3]> = Vec::new();
    let mut trns = Vec::new();
    let (mut header, mut ended, mut seen_data) = (false, false, false);
    while pos < data.len() {
        let chunk_header = data.get(pos..pos + 8).ok_or("truncated PNG chunk header")?;
        let len = u32::from_be_bytes(chunk_header[..4].try_into().unwrap()) as usize;
        let end = pos
            .checked_add(8)
            .and_then(|n| n.checked_add(len))
            .ok_or("PNG chunk overflow")?;
        let body = data.get(pos + 8..end).ok_or("truncated PNG chunk")?;
        let checksum = data.get(end..end + 4).ok_or("missing PNG checksum")?;
        if sol_net::inflate::crc32(&data[pos + 4..end])
            != u32::from_be_bytes(checksum.try_into().unwrap())
        {
            return Err("PNG checksum mismatch".into());
        }
        let ty = &chunk_header[4..8];
        if !header && ty != b"IHDR" {
            return Err("PNG must start with IHDR".into());
        }
        match ty {
            b"IHDR" => {
                if header || body.len() != 13 {
                    return Err("invalid IHDR".into());
                }
                w = u32::from_be_bytes(body[..4].try_into().unwrap());
                h = u32::from_be_bytes(body[4..8].try_into().unwrap());
                crate::dimensions(w, h)?;
                bit_depth = body[8];
                color_type = body[9];
                interlace = body[12];
                let valid_depth = match color_type {
                    0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
                    2 | 4 | 6 => matches!(bit_depth, 8 | 16),
                    3 => matches!(bit_depth, 1 | 2 | 4 | 8),
                    _ => false,
                };
                if !valid_depth || body[10] != 0 || body[11] != 0 || interlace > 1 {
                    return Err("unsupported PNG header".into());
                }
                header = true;
            }
            b"PLTE" => {
                if seen_data
                    || !plte.is_empty()
                    || body.is_empty()
                    || body.len() > 768
                    || body.len() % 3 != 0
                {
                    return Err("invalid PNG palette".into());
                }
                plte = body.as_chunks::<3>().0.to_vec();
            }
            b"tRNS" => {
                let valid = match color_type {
                    0 => body.len() == 2,
                    2 => body.len() == 6,
                    3 => !plte.is_empty() && body.len() <= plte.len(),
                    _ => false,
                };
                if !valid || seen_data {
                    return Err("invalid PNG transparency".into());
                }
                trns = body.to_vec();
            }
            b"IDAT" => {
                idat.try_reserve_exact(body.len())
                    .map_err(|e| format!("PNG allocation: {e}"))?;
                idat.extend_from_slice(body);
                seen_data = true;
            }
            b"IEND" => {
                if !body.is_empty() || !seen_data || end + 4 != data.len() {
                    return Err("invalid PNG end".into());
                }
                ended = true;
                break;
            }
            _ if ty[0].is_ascii_uppercase() => return Err("unsupported critical PNG chunk".into()),
            _ => {}
        }
        pos = end + 4;
    }
    if !ended || (color_type == 3 && (plte.is_empty() || plte.len() > (1 << bit_depth))) {
        return Err("incomplete PNG".into());
    }
    let channels = match color_type {
        0 | 3 => 1usize,
        2 => 3,
        4 => 2,
        _ => 4,
    };
    let passes: &[(u32, u32, u32, u32)] = if interlace == 1 {
        &[
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ]
    } else {
        &[(0, 0, 1, 1)]
    };
    let mut expected = 0usize;
    for &(x, y, dx, dy) in passes {
        let pw = w.saturating_sub(x).div_ceil(dx) as usize;
        let ph = h.saturating_sub(y).div_ceil(dy) as usize;
        if pw != 0 && ph != 0 {
            expected += (1 + (pw * channels * bit_depth as usize).div_ceil(8)) * ph;
        }
    }
    let raw =
        sol_net::inflate::zlib_decode_limited(&idat, expected).map_err(|e| format!("idat: {e}"))?;
    if raw.len() != expected {
        return Err("incorrect PNG scanline size".into());
    }
    let mut img = Image::try_new(w, h)?;
    let mut off = 0usize;
    for &(x0, y0, dx, dy) in passes {
        let pw = w.saturating_sub(x0).div_ceil(dx);
        let ph = h.saturating_sub(y0).div_ceil(dy);
        if pw == 0 || ph == 0 {
            continue;
        }
        let row_bytes = (pw as usize * channels * bit_depth as usize).div_ceil(8);
        let bpp = (channels * bit_depth as usize).div_ceil(8);
        let mut prev = crate::zeroed(row_bytes)?;
        let mut line = crate::zeroed(row_bytes)?;
        for py in 0..ph {
            let ft = raw[off];
            if ft > 4 {
                return Err("invalid PNG filter".into());
            }
            unfilter(
                ft,
                &raw[off + 1..off + 1 + row_bytes],
                &prev,
                &mut line,
                bpp,
            );
            off += 1 + row_bytes;
            for px in 0..pw {
                if color_type == 3 && bits(&line, px as usize, bit_depth) as usize >= plte.len() {
                    return Err("PNG palette index out of range".into());
                }
                let rgba = sample(
                    &line,
                    px as usize,
                    channels,
                    bit_depth,
                    &plte,
                    &trns,
                    color_type,
                );
                img.set(x0 + px * dx, y0 + py * dy, rgba);
            }
            std::mem::swap(&mut prev, &mut line);
        }
    }
    Ok(img)
}

fn unfilter(ft: u8, cur: &[u8], prev: &[u8], out: &mut [u8], bpp: usize) {
    match ft {
        0 => out.copy_from_slice(cur),
        1 => {
            for i in 0..cur.len() {
                let a = if i >= bpp { out[i - bpp] } else { 0 };
                out[i] = cur[i].wrapping_add(a);
            }
        }
        2 => {
            for i in 0..cur.len() {
                out[i] = cur[i].wrapping_add(prev[i]);
            }
        }
        3 => {
            for i in 0..cur.len() {
                let a = if i >= bpp { out[i - bpp] } else { 0 };
                let b = prev[i];
                out[i] = cur[i].wrapping_add(((a as u16 + b as u16) / 2) as u8);
            }
        }
        4 => {
            for i in 0..cur.len() {
                let a = if i >= bpp { out[i - bpp] } else { 0 };
                let b = prev[i];
                let c = if i >= bpp { prev[i - bpp] } else { 0 };
                out[i] = cur[i].wrapping_add(paeth(a, b, c));
            }
        }
        _ => {}
    }
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (a, b, c) = (a as i32, b as i32, c as i32);
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

fn bits(line: &[u8], idx: usize, depth: u8) -> u16 {
    match depth {
        1 => (line[idx / 8] >> (7 - idx % 8)) as u16 & 1,
        2 => (line[idx / 4] >> (6 - (idx % 4) * 2)) as u16 & 3,
        4 => (line[idx / 2] >> (4 - (idx % 2) * 4)) as u16 & 15,
        8 => line[idx] as u16,
        16 => u16::from_be_bytes([line[idx * 2], line[idx * 2 + 1]]),
        _ => 0,
    }
}

fn to8(v: u16, depth: u8) -> u8 {
    match depth {
        16 => (v >> 8) as u8,
        8 => v as u8,
        4 => (v * 17) as u8,
        2 => (v * 85) as u8,
        1 => (v * 255) as u8,
        _ => 0,
    }
}

fn sample(
    line: &[u8],
    px: usize,
    ch: usize,
    depth: u8,
    plte: &[[u8; 3]],
    trns: &[u8],
    ct: u8,
) -> [u8; 4] {
    match ct {
        0 => {
            let g = to8(bits(line, px, depth), depth);
            let raw = bits(line, px, depth);
            let a = if trns.len() >= 2 && u16::from_be_bytes([trns[0], trns[1]]) == raw {
                0
            } else {
                255
            };
            [g, g, g, a]
        }
        2 => {
            let i = px * ch;
            let (r16, g16, b16) = (
                bits(line, i, depth),
                bits(line, i + 1, depth),
                bits(line, i + 2, depth),
            );
            let a = if trns.len() >= 6 {
                let tr = u16::from_be_bytes([trns[0], trns[1]]);
                let tg = u16::from_be_bytes([trns[2], trns[3]]);
                let tb = u16::from_be_bytes([trns[4], trns[5]]);
                if r16 == tr && g16 == tg && b16 == tb {
                    0
                } else {
                    255
                }
            } else {
                255
            };
            [to8(r16, depth), to8(g16, depth), to8(b16, depth), a]
        }
        3 => {
            let idx = bits(line, px, depth) as usize;
            let c = plte.get(idx).copied().unwrap_or([0, 0, 0]);
            let a = trns.get(idx).copied().unwrap_or(255);
            [c[0], c[1], c[2], a]
        }
        4 => {
            let i = px * 2;
            [
                to8(bits(line, i, depth), depth),
                to8(bits(line, i, depth), depth),
                to8(bits(line, i, depth), depth),
                to8(bits(line, i + 1, depth), depth),
            ]
        }
        _ => {
            let i = px * 4;
            [
                to8(bits(line, i, depth), depth),
                to8(bits(line, i + 1, depth), depth),
                to8(bits(line, i + 2, depth), depth),
                to8(bits(line, i + 3, depth), depth),
            ]
        }
    }
}

// ---------------- encoder ----------------

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffffffffu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xedb88320
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn chunk(ty: &[u8; 4], body: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(ty);
    out.extend_from_slice(body);
    let mut cdata = ty.to_vec();
    cdata.extend_from_slice(body);
    out.extend_from_slice(&crc32(&cdata).to_be_bytes());
}

/// Encode RGBA8 image as PNG (zlib stream with stored deflate blocks).
pub fn encode(img: &Image) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&img.w.to_be_bytes());
    ihdr.extend_from_slice(&img.h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr, &mut out);

    // filtered scanlines, filter 0
    let stride = img.w as usize * 4;
    let mut raw = Vec::with_capacity((stride + 1) * img.h as usize);
    for y in 0..img.h as usize {
        raw.push(0);
        raw.extend_from_slice(&img.px[y * stride..(y + 1) * stride]);
    }
    // zlib: header 0x78 0x01 + stored blocks + adler32
    let mut z = vec![0x78, 0x01];
    let mut i = 0;
    while i < raw.len() {
        let n = (raw.len() - i).min(65535);
        let last = i + n == raw.len();
        z.push(if last { 1 } else { 0 });
        z.extend_from_slice(&(n as u16).to_le_bytes());
        z.extend_from_slice(&(!(n as u16)).to_le_bytes());
        z.extend_from_slice(&raw[i..i + n]);
        i += n;
    }
    let mut s1 = 1u32;
    let mut s2 = 0u32;
    for &b in &raw {
        s1 = (s1 + b as u32) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    z.extend_from_slice(&((s2 << 16) | s1).to_be_bytes());
    chunk(b"IDAT", &z, &mut out);
    chunk(b"IEND", &[], &mut out);
    out
}
