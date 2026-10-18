use sol_img::{decode, Image};

fn chunk(kind: &[u8; 4], body: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    let crc = sol_net::inflate::crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}
fn png(w: u32, h: u32, depth: u8, color: u8, pixels: &[u8]) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = w.to_be_bytes().to_vec();
    header.extend(h.to_be_bytes());
    header.extend([depth, color, 0, 0, 0]);
    chunk(b"IHDR", &header, &mut out);
    chunk(
        b"IDAT",
        &sol_net::inflate::zlib_encode_stored(pixels),
        &mut out,
    );
    chunk(b"IEND", &[], &mut out);
    out
}

#[test]
fn dimensions_are_checked_before_allocation() {
    for (w, h) in [(0, 1), (1, 0), (u32::MAX, 1), (16384, 16384)] {
        assert!(Image::try_new(w, h).is_err());
        assert!(decode(&png(w, h, 8, 6, &[])).is_err());
    }
    assert_eq!(Image::try_new(2, 2).unwrap().px.len(), 16);
}

#[test]
fn truncated_images_and_mutated_headers_do_not_panic() {
    for data in [
        include_bytes!("t2x2.png").as_slice(),
        include_bytes!("t2x2.bmp"),
        include_bytes!("t2x1.gif"),
    ] {
        for length in 0..data.len() {
            let _ = decode(&data[..length]);
        }
        for offset in 0..data.len() {
            for value in [0, 1, 127, 255] {
                let mut bytes = data.to_vec();
                bytes[offset] = value;
                let _ = decode(&bytes);
            }
        }
    }
}

#[test]
fn png_rejects_short_chunks_invalid_depths_filters_and_checksums() {
    for (depth, color) in [(0, 6), (1, 6), (16, 3), (8, 9), (32, 0)] {
        assert!(decode(&png(1, 1, depth, color, &[0; 9])).is_err());
    }
    assert!(decode(&png(1, 1, 8, 6, &[5, 1, 2, 3, 4])).is_err());
    assert!(decode(&png(1, 1, 8, 6, &[0, 1, 2, 3])).is_err());
    let mut short = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(b"IHDR", &[0; 3], &mut short);
    assert!(decode(&short).is_err());
    let mut valid = png(1, 1, 8, 6, &[0, 1, 2, 3, 255]);
    assert_eq!(decode(&valid).unwrap().get(0, 0), [1, 2, 3, 255]);
    valid[29] ^= 1;
    assert!(decode(&valid).is_err());
}

#[test]
fn png_decompression_cannot_exceed_expected_scanlines() {
    assert!(decode(&png(1, 1, 8, 6, &vec![0; 1024 * 1024]))
        .unwrap_err()
        .contains("limit"));
    for len in 0..6 {
        assert!(sol_net::inflate::zlib_decode(&[0; 6][..len]).is_err());
    }
    let bytes = sol_net::inflate::zlib_encode_stored(b"hello");
    assert!(sol_net::inflate::zlib_decode_limited(&bytes, 4).is_err());
    assert_eq!(
        sol_net::inflate::zlib_decode_limited(&bytes, 5).unwrap(),
        b"hello"
    );
    let mut corrupt = bytes;
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(sol_net::inflate::zlib_decode(&corrupt).is_err());
}

#[test]
fn gif_rejects_extreme_code_sizes_frames_and_truncated_blocks() {
    let original = include_bytes!("t2x1.gif");
    let mut bytes = original.to_vec();
    let descriptor = 13
        + if bytes[10] & 128 != 0 {
            3 * (1usize << ((bytes[10] & 7) + 1))
        } else {
            0
        };
    assert_eq!(bytes[descriptor], 0x2c);
    for code in [0, 1, 9, 32, 255] {
        bytes = original.to_vec();
        bytes[descriptor + 10] = code;
        assert!(decode(&bytes).is_err());
    }
    bytes = original.to_vec();
    bytes[descriptor + 1..descriptor + 3].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(decode(&bytes).is_err());
    assert!(decode(&original[..original.len() - 3]).is_err());
}

#[test]
fn bmp_rejects_bad_offsets_depths_and_signed_minimum_height() {
    let original = include_bytes!("t2x2.bmp");
    for (offset, value) in [(10, u32::MAX), (14, u32::MAX), (22, i32::MIN as u32)] {
        let mut bytes = original.to_vec();
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(decode(&bytes).is_err());
    }
    for depth in [0u16, 2, 15, 64, 255] {
        let mut bytes = original.to_vec();
        bytes[28..30].copy_from_slice(&depth.to_le_bytes());
        assert!(decode(&bytes).is_err());
    }
    assert!(decode(&original[..original.len() - 1]).is_err());
}

#[test]
fn bmp_16_bit_white_uses_wide_channel_arithmetic() {
    let mut bytes = vec![0; 58];
    bytes[..2].copy_from_slice(b"BM");
    for (offset, value) in [(2, 58u32), (10, 54), (14, 40), (18, 1), (22, 1)] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
    bytes[28..30].copy_from_slice(&16u16.to_le_bytes());
    bytes[54..56].copy_from_slice(&0x7fffu16.to_le_bytes());
    assert_eq!(decode(&bytes).unwrap().get(0, 0), [255, 255, 255, 255]);
}

#[test]
fn malformed_deflate_never_panics_or_exceeds_budget() {
    let mut seed = 0x12345678u32;
    for length in 0..64 {
        for _ in 0..64 {
            let mut bytes = Vec::new();
            for _ in 0..length {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                bytes.push(seed as u8);
            }
            if let Ok(output) = sol_net::inflate::inflate_limited(&bytes, 1024) {
                assert!(output.len() <= 1024);
            }
        }
    }
}
