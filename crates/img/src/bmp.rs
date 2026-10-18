//! BMP decoder — indexed, 16/24/32bpp, BI_RGB and BI_BITFIELDS.
use crate::Image;

pub fn decode(data: &[u8]) -> Result<Image, String> {
    crate::input(data)?;
    if data.len() < 26 || &data[..2] != b"BM" {
        return Err("not bmp".into());
    }
    let u16_at = |off| -> Result<u16, String> {
        Ok(u16::from_le_bytes(
            data.get(off..off + 2)
                .ok_or("truncated BMP header")?
                .try_into()
                .unwrap(),
        ))
    };
    let u32_at = |off| -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            data.get(off..off + 4)
                .ok_or("truncated BMP header")?
                .try_into()
                .unwrap(),
        ))
    };
    let data_off = u32_at(10)? as usize;
    let dib = u32_at(14)? as usize;
    if ![12, 40, 52, 56, 108, 124].contains(&dib) || data.len() < 14 + dib {
        return Err("unsupported or truncated BMP header".into());
    }
    let (w, h, bpp, compression) = if dib == 12 {
        if u16_at(22)? != 1 {
            return Err("invalid BMP planes".into());
        }
        (
            u16_at(18)? as i32,
            u16_at(20)? as i32,
            u16_at(24)? as usize,
            0,
        )
    } else {
        if u16_at(26)? != 1 {
            return Err("invalid BMP planes".into());
        }
        (
            u32_at(18)? as i32,
            u32_at(22)? as i32,
            u16_at(28)? as usize,
            u32_at(30)?,
        )
    };
    if w <= 0 || h == 0 || ![1, 4, 8, 16, 24, 32].contains(&bpp) {
        return Err("invalid BMP dimensions or depth".into());
    }
    crate::dimensions(w as u32, h.unsigned_abs())?;
    if compression != 0 && !(compression == 3 && matches!(bpp, 16 | 32)) {
        return Err("unsupported BMP compression".into());
    }
    let top_down = h < 0;
    let (w, h) = (w as usize, h.unsigned_abs() as usize);
    let mut palette_start = 14 + dib;
    let masks = if compression == 3 {
        let masks = [
            u32_at(54)?,
            u32_at(58)?,
            u32_at(62)?,
            if dib >= 56 { u32_at(66)? } else { 0 },
        ];
        if dib == 40 {
            palette_start += 12;
        }
        for (i, &mask) in masks.iter().enumerate() {
            if i < 3 && mask == 0 {
                return Err("empty BMP color mask".into());
            }
            if mask == 0 {
                continue;
            }
            let shifted = mask >> mask.trailing_zeros();
            if shifted & shifted.wrapping_add(1) != 0
                || (bpp == 16 && mask > 65535)
                || masks[..i].iter().any(|previous| previous & mask != 0)
            {
                return Err("invalid BMP color masks".into());
            }
        }
        masks
    } else {
        [0; 4]
    };
    let mut palette = Vec::new();
    if bpp <= 8 {
        let count = if dib >= 40 { u32_at(46)? as usize } else { 0 };
        let count = if count == 0 { 1 << bpp } else { count };
        if count > 1 << bpp {
            return Err("invalid BMP palette length".into());
        }
        let entry_size = if dib == 12 { 3 } else { 4 };
        let end = palette_start + count * entry_size;
        let bytes = data
            .get(palette_start..end)
            .ok_or("truncated BMP palette")?;
        if end > data_off {
            return Err("BMP palette overlaps pixels".into());
        }
        palette = bytes
            .chunks_exact(entry_size)
            .map(|c| [c[2], c[1], c[0], 255])
            .collect();
        palette_start = end;
    }
    let stride = (w * bpp).div_ceil(32) * 4;
    if data_off < palette_start || stride * h > data.len().saturating_sub(data_off) {
        return Err("truncated BMP pixels or invalid offset".into());
    }
    let mut img = Image::try_new(w as u32, h as u32)?;
    for row in 0..h {
        let src = if top_down { row } else { h - 1 - row };
        let start = data_off + src * stride;
        let line = &data[start..start + stride];
        for x in 0..w {
            let color = match bpp {
                1 | 4 | 8 => {
                    let index = match bpp {
                        1 => (line[x / 8] >> (7 - x % 8)) & 1,
                        4 => (line[x / 2] >> (4 - x % 2 * 4)) & 15,
                        _ => line[x],
                    } as usize;
                    *palette.get(index).ok_or("invalid BMP palette index")?
                }
                24 => [line[x * 3 + 2], line[x * 3 + 1], line[x * 3], 255],
                _ => {
                    let value = if bpp == 16 {
                        u16::from_le_bytes(line[x * 2..x * 2 + 2].try_into().unwrap()) as u32
                    } else {
                        u32::from_le_bytes(line[x * 4..x * 4 + 4].try_into().unwrap())
                    };
                    if compression == 3 {
                        [
                            unmask(value, masks[0]),
                            unmask(value, masks[1]),
                            unmask(value, masks[2]),
                            if masks[3] == 0 {
                                255
                            } else {
                                unmask(value, masks[3])
                            },
                        ]
                    } else if bpp == 16 {
                        [
                            unmask(value, 0x7c00),
                            unmask(value, 0x3e0),
                            unmask(value, 0x1f),
                            255,
                        ]
                    } else {
                        [line[x * 4 + 2], line[x * 4 + 1], line[x * 4], 255]
                    }
                }
            };
            img.set(x as u32, row as u32, color);
        }
    }
    Ok(img)
}

fn unmask(value: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let max = (mask >> shift) as u64;
    ((((value & mask) >> shift) as u64 * 255 + max / 2) / max) as u8
}
