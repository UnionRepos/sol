//! GIF87a/89a decoder — LZW, interlace, transparency, first frame output.

use crate::Image;

pub fn decode(data: &[u8]) -> Result<Image, String> {
    crate::input(data)?;
    if data.len() < 13 || (&data[..6] != b"GIF87a" && &data[..6] != b"GIF89a") {
        return Err("not gif".into());
    }
    let sw = u16::from_le_bytes([data[6], data[7]]);
    let sh = u16::from_le_bytes([data[8], data[9]]);
    crate::dimensions(sw as u32, sh as u32)?;
    let packed = data[10];
    let gct_flag = packed & 0x80 != 0;
    let gct_size = if gct_flag {
        3 * (1usize << ((packed & 7) + 1))
    } else {
        0
    };
    let mut pos = 13;
    if pos + gct_size > data.len() {
        return Err("gct short".into());
    }
    let gct: Vec<[u8; 3]> = data[pos..pos + gct_size]
        .chunks(3)
        .map(|c| [c[0], c[1], c[2]])
        .collect();
    pos += gct_size;

    let mut transparent: Option<u8> = None;
    loop {
        if pos >= data.len() {
            return Err("no image".into());
        }
        match data[pos] {
            0x21 => {
                // extension
                pos += 1;
                if pos >= data.len() {
                    break;
                }
                let label = data[pos];
                pos += 1;
                if label == 0xF9 && pos < data.len() {
                    // graphic control
                    let blen = data[pos] as usize;
                    if blen != 4 || pos + 1 + blen + 1 > data.len() || data[pos + 1 + blen] != 0 {
                        return Err("invalid GIF control extension".into());
                    }
                    if blen == 4 {
                        let flags = data[pos + 1];
                        if flags & 1 != 0 {
                            transparent = Some(data[pos + 4]);
                        }
                    }
                    pos += 1 + blen + 1;
                } else {
                    pos = skip_blocks(data, pos)?;
                }
            }
            0x2C => {
                // image descriptor
                if pos + 10 > data.len() {
                    return Err("desc short".into());
                }
                let ix = u16::from_le_bytes([data[pos + 1], data[pos + 2]]);
                let iy = u16::from_le_bytes([data[pos + 3], data[pos + 4]]);
                let iw = u16::from_le_bytes([data[pos + 5], data[pos + 6]]);
                let ih = u16::from_le_bytes([data[pos + 7], data[pos + 8]]);
                crate::dimensions(iw as u32, ih as u32)?;
                if ix as u32 + iw as u32 > sw as u32 || iy as u32 + ih as u32 > sh as u32 {
                    return Err("GIF frame exceeds canvas".into());
                }
                let ip = data[pos + 9];
                pos += 10;
                let lct = if ip & 0x80 != 0 {
                    let sz = 3 * (1usize << ((ip & 7) + 1));
                    if pos + sz > data.len() {
                        return Err("lct short".into());
                    }
                    let t: Vec<[u8; 3]> = data[pos..pos + sz]
                        .chunks(3)
                        .map(|c| [c[0], c[1], c[2]])
                        .collect();
                    pos += sz;
                    t
                } else {
                    gct.clone()
                };
                let interlaced = ip & 0x40 != 0;
                if pos >= data.len() {
                    return Err("lzw short".into());
                }
                let min_code = data[pos];
                pos += 1;
                let start = pos;
                let end = skip_blocks(data, pos)?;
                let mut stream = Vec::new();
                stream
                    .try_reserve_exact(end - start)
                    .map_err(|e| format!("GIF allocation: {e}"))?;
                while pos < end {
                    let n = data[pos] as usize;
                    pos += 1;
                    if n == 0 {
                        break;
                    }
                    stream.extend_from_slice(&data[pos..pos + n]);
                    pos += n;
                }
                let idx = lzw_decode(&stream, min_code, iw as usize * ih as usize)?;
                // de-interlace
                let order: Vec<usize> = if interlaced {
                    let mut o: Vec<usize> = (0..ih as usize).step_by(8).collect();
                    o.extend((4..ih as usize).step_by(8));
                    o.extend((2..ih as usize).step_by(4));
                    o.extend((1..ih as usize).step_by(2));
                    o
                } else {
                    (0..ih as usize).collect()
                };
                if lct.is_empty() {
                    return Err("missing GIF palette".into());
                }
                let mut img = Image::try_new(sw as u32, sh as u32)?;
                // make canvas transparent-ish; caller composites over bg
                for (row_i, &ry) in order.iter().enumerate() {
                    for cx in 0..iw as usize {
                        let src = row_i * iw as usize + cx;
                        let ci = idx[src];
                        if Some(ci) == transparent {
                            continue;
                        }
                        let c = lct
                            .get(ci as usize)
                            .copied()
                            .ok_or("invalid GIF palette index")?;
                        img.set(
                            ix as u32 + cx as u32,
                            iy as u32 + ry as u32,
                            [c[0], c[1], c[2], 255],
                        );
                    }
                }
                return Ok(img);
            }
            0x3B => return Err("no image".into()),
            _ => pos += 1,
        }
    }
    Err("truncated".into())
}

fn skip_blocks(data: &[u8], mut pos: usize) -> Result<usize, String> {
    loop {
        let n = *data.get(pos).ok_or("truncated GIF sub-block")? as usize;
        pos += 1;
        if n == 0 {
            return Ok(pos);
        }
        if n > data.len() - pos {
            return Err("truncated GIF sub-block".into());
        }
        pos += n;
    }
}

fn lzw_decode(data: &[u8], min_code: u8, expected: usize) -> Result<Vec<u8>, String> {
    if !(2..=8).contains(&min_code) {
        return Err("invalid GIF LZW code size".into());
    }
    let clear = 1usize << min_code;
    let eoi = clear + 1;
    let mut dict: Vec<Vec<u8>> = (0..clear).map(|i| vec![i as u8]).collect();
    dict.push(vec![]); // clear
    dict.push(vec![]); // eoi
    let mut code_size = min_code as usize + 1;
    let mut out = crate::zeroed(expected)?;
    out.clear();
    let mut cleared = false;
    let mut prev: Option<Vec<u8>> = None;
    let mut bitpos = 0usize;

    let read = |data: &[u8], bitpos: usize, n: usize| -> Option<u16> {
        let mut v = 0u16;
        for i in 0..n {
            let byte = data.get((bitpos + i) / 8)?;
            let bit = (byte >> ((bitpos + i) % 8)) & 1;
            v |= (bit as u16) << i;
        }
        Some(v)
    };

    while let Some(code) = read(data, bitpos, code_size) {
        bitpos += code_size;
        let code = code as usize;
        if code == clear {
            dict.truncate(clear + 2);
            code_size = min_code as usize + 1;
            prev = None;
            cleared = true;
            continue;
        }
        if !cleared {
            return Err("GIF LZW must start with clear code".into());
        }
        if code == eoi {
            return if out.len() == expected {
                Ok(out)
            } else {
                Err("truncated GIF pixels".into())
            };
        }
        let entry = if code < dict.len() {
            dict[code].clone()
        } else if code == dict.len() {
            match &prev {
                Some(p) => {
                    let mut e = p.clone();
                    e.push(p[0]);
                    e
                }
                None => return Err("lzw code".into()),
            }
        } else {
            return Err("lzw code".into());
        };
        if entry.is_empty() || entry.len() > expected.saturating_sub(out.len()) {
            return Err("GIF LZW output exceeds frame".into());
        }
        out.extend_from_slice(&entry);
        if let Some(p) = &prev {
            if dict.len() < 4096 {
                let mut ne = p.clone();
                ne.push(entry[0]);
                dict.push(ne);
                if dict.len() == (1 << code_size) && code_size < 12 {
                    code_size += 1;
                }
            }
        }
        prev = Some(entry);
    }
    Err("missing GIF LZW end code".into())
}
