use sol_text::{Font, Fonts};

fn dejavu() -> Vec<u8> {
    include_bytes!("../assets/DejaVuSans.ttf").to_vec()
}

#[test]
fn load_and_map() {
    let f = Font::load(dejavu()).unwrap();
    assert_eq!(f.units_per_em, 2048);
    let a = f.glyph_index('A');
    let b = f.glyph_index('B');
    assert_ne!(a, 0);
    assert_ne!(a, b);
    assert!(f.advance(a) > 0);
    assert!(f.ascent > 0 && f.descent < 0);
}

#[test]
fn glyph_outline() {
    let f = Font::load(dejavu()).unwrap();
    let g = f.glyph(f.glyph_index('O'));
    assert!(g.contours.len() >= 2); // O has outer+inner contour
    let h = f.glyph(f.glyph_index('i'));
    assert!(!h.contours.is_empty());
}

#[test]
fn rasterize_glyph() {
    let f = Font::load(dejavu()).unwrap();
    let face = sol_text::Face::new(std::rc::Rc::new(f), 32.0);
    let gid = face.glyph_id('H');
    let bm = face.rasterize(gid);
    assert!(bm.w > 5 && bm.h > 5);
    let total: u32 = bm.buf.iter().map(|&b| b as u32).sum();
    assert!(total > 500, "coverage too low: {total}");
    // 'i' should be mostly empty at top (dot) vs stem — just check nonempty
    let bm2 = face.rasterize(face.glyph_id('i'));
    assert!(bm2.buf.iter().any(|&b| b > 0));
}

#[test]
fn measure() {
    let f = Font::load(dejavu()).unwrap();
    let face = sol_text::Face::new(std::rc::Rc::new(f), 20.0);
    let w1 = face.measure("i");
    let w2 = face.measure("W");
    assert!(w2 > w1 * 2.0);
    let w3 = face.measure("Hello");
    assert!(w3 > 30.0 && w3 < 100.0);
}

#[test]
fn discovery() {
    let fonts = Fonts::discover();
    assert!(!fonts.is_empty());
    let face = fonts.face("sans-serif", 16.0, false, false);
    assert!(face.measure("test") > 10.0);
    let mono = fonts.face("monospace", 16.0, false, false);
    assert!(mono.measure("iii") > 0.0);
    let bold = fonts.face("DejaVu Sans", 16.0, true, false);
    // bold face should differ from regular or at least resolve
    let _ = bold.measure("x");
}

#[test]
fn bundled_fallback_works_without_system_fonts() {
    let fonts = Fonts::from_dirs(Vec::new());
    assert!(!fonts.is_empty());
    let face = fonts.face("missing-family, sans-serif", 18.0, false, false);
    assert!(face.measure("Portable text") > 50.0);
    assert!(face
        .rasterize(face.glyph_id('A'))
        .buf
        .iter()
        .any(|&v| v > 0));
    let (width, _) = face.layout("AVATAR");
    assert_eq!(width, face.measure("AVATAR"));
}

#[test]
fn malformed_fonts_and_indices_do_not_panic() {
    let valid = dejavu();
    for length in [0, 1, 11, 12, 28, 128] {
        assert!(Font::load(valid[..length].to_vec()).is_err());
    }
    let mut broken = valid.clone();
    let count = u16::from_be_bytes([broken[4], broken[5]]) as usize;
    for table in 0..count {
        let offset = 12 + table * 16 + 8;
        broken[offset..offset + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    }
    assert!(Font::load(broken).is_err());
    assert!(Font::load_index(valid.clone(), 1).is_err());
    let font = Font::load(valid).unwrap();
    assert_eq!(font.advance(u16::MAX), 0);
    assert!(font.glyph(u16::MAX).path.is_none());
}

#[test]
fn invalid_sizes_and_empty_glyphs_are_bounded() {
    let fonts = Fonts::from_dirs(Vec::new());
    for size in [f32::NAN, f32::INFINITY, -10.0, 0.0, f32::MAX] {
        let face = fonts.face("sans-serif", size, false, false);
        assert!(face.px.is_finite() && (0.0..=512.0).contains(&face.px));
        let bitmap = face.rasterize(face.glyph_id('A'));
        assert!(bitmap.w <= 2048 && bitmap.h <= 2048);
        assert!(face.measure("Text").is_finite());
    }
    let face = fonts.face("sans-serif", 16.0, false, false);
    assert!(face.rasterize(face.glyph_id(' ')).buf.is_empty());
}

#[test]
fn system_font_discovery_reuses_parsed_fonts() {
    let first = Fonts::discover().face("sans-serif", 16.0, false, false);
    let second = Fonts::discover().face("sans-serif", 16.0, false, false);
    assert!(std::rc::Rc::ptr_eq(&first.font, &second.font));
}

#[test]
fn opentype_cubic_outlines_are_parsed_and_rasterized() {
    // A self-contained synthetic CFF font: .notdef plus a single cubic arch.
    let mut head = vec![0; 54];
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut hhea = vec![0; 36];
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[34..36].copy_from_slice(&2u16.to_be_bytes());
    let mut cmap = vec![0, 0, 0, 1, 0, 3, 0, 1, 0, 0, 0, 12, 0, 0, 1, 6, 0, 0];
    cmap.extend([0; 256]);
    cmap[18 + b'A' as usize] = 1;
    let cff = vec![
        1, 0, 4, 4, // CFF header
        0, 1, 1, 1, 5, b'T', b'e', b's', b't', // Name INDEX
        0, 1, 1, 1, 3, 163, 17, // Top DICT: CharStrings at byte 24
        0, 0, 0, 0, // empty String and Global Subr INDEX
        0, 2, 1, 1, 2, 19, // CharStrings INDEX
        14, // .notdef
        139, 139, 21, // rmoveto 0,0
        139, 28, 2, 188, 28, 2, 88, 139, 139, 28, 253, 68, 8, 14,
    ];
    let mut tables = vec![
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"maxp", vec![0, 0, 80, 0, 0, 2]),
        (*b"hmtx", vec![2, 88, 0, 0, 2, 88, 0, 0]),
        (*b"cmap", cmap),
        (*b"CFF ", cff),
    ];
    tables.sort_by_key(|table| table.0);
    let mut data = b"OTTO".to_vec();
    data.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    data.extend([0; 6]);
    data.resize(12 + tables.len() * 16, 0);
    for (index, (tag, table)) in tables.into_iter().enumerate() {
        let offset = 12 + index * 16;
        let start = data.len() as u32;
        data[offset..offset + 4].copy_from_slice(&tag);
        data[offset + 8..offset + 12].copy_from_slice(&start.to_be_bytes());
        data[offset + 12..offset + 16].copy_from_slice(&(table.len() as u32).to_be_bytes());
        data.extend_from_slice(&table);
        while !data.len().is_multiple_of(4) {
            data.push(0);
        }
    }
    let font = Font::load(data).unwrap();
    assert_eq!(font.glyph_index('A'), 1);
    let face = sol_text::Face::new(std::rc::Rc::new(font), 32.0);
    let bitmap = face.rasterize(1);
    assert!(bitmap.w > 10 && bitmap.h > 10);
    assert!(bitmap.buf.iter().any(|&v| v > 0 && v < 255));
}

#[test]
fn truetype_collections_load_by_index() {
    let mut data = dejavu();
    let count = u16::from_be_bytes([data[4], data[5]]) as usize;
    // TTC table offsets are relative to the collection, not the member face.
    for table in 0..count {
        let start = 12 + table * 16 + 8;
        let offset = u32::from_be_bytes(data[start..start + 4].try_into().unwrap()) + 16;
        data[start..start + 4].copy_from_slice(&offset.to_be_bytes());
    }
    let mut collection = b"ttcf\x00\x01\x00\x00\x00\x00\x00\x01\x00\x00\x00\x10".to_vec();
    collection.extend(data);
    let font = Font::load_index(collection.clone(), 0).unwrap();
    assert_ne!(font.glyph_index('A'), 0);
    assert!(Font::load_index(collection, 1).is_err());
}

#[cfg(unix)]
#[test]
fn discovery_skips_corrupt_fonts_and_symlink_cycles() {
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("sol-fonts-{}-{id}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("good.ttf"), dejavu()).unwrap();
    std::fs::write(dir.join("bad.otf"), b"broken").unwrap();
    std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
    let fonts = Fonts::from_dirs([dir.clone()]);
    std::fs::remove_dir_all(dir).unwrap();
    assert!(
        fonts
            .face("DejaVu Sans", 16.0, false, false)
            .measure("safe")
            > 0.0
    );
}

#[test]
fn shaping_handles_ligatures_arabic_and_combining_marks() {
    let face = sol_text::Face::new(std::rc::Rc::new(Font::load(dejavu()).unwrap()), 32.0);
    let ligature = face.shape("ffi");
    assert!(ligature.glyphs.len() < 3);
    let arabic = face.shape("سلام");
    assert!(arabic.glyphs.iter().all(|glyph| glyph.id != 0));
    assert!(arabic.glyphs.len() < 4);
    assert!(arabic.glyphs.first().unwrap().cluster > arabic.glyphs.last().unwrap().cluster);
    let accent = face.shape("x\u{301}");
    assert!((accent.width - face.measure("x")).abs() < 0.01);
    assert_eq!(accent.width, face.measure("x\u{301}"));
}

#[test]
fn mixed_direction_runs_keep_numbers_and_latin_in_logical_order() {
    let face = sol_text::Face::new(std::rc::Rc::new(Font::load(dejavu()).unwrap()), 20.0);
    let shaped = face.shape("abc אבג 123");
    let clusters: Vec<_> = shaped.glyphs.iter().map(|glyph| glyph.cluster).collect();
    assert!(clusters.windows(3).any(|c| c == [0, 1, 2]));
    assert!(clusters.windows(3).any(|c| c == [11, 12, 13]));
    assert!(clusters.windows(3).any(|c| c == [8, 6, 4]));
    assert_eq!(
        sol_text::visual_order(&["אחד", "שתיים", "שלוש"], sol_text::Direction::Rtl),
        [2, 1, 0]
    );
}

#[test]
fn fallback_keeps_grapheme_clusters_and_uses_fallback_metrics() {
    // Remove Hebrew coverage from the primary font while retaining a valid face.
    let primary = std::rc::Rc::new(Font::load(dejavu()).unwrap());
    // Use a bundled fixture with limited coverage if available; a font's cmap
    // tables may have multiple subtables, so test missing glyph choice via a
    // primary font that deliberately maps no Unicode characters.
    let mut bytes = dejavu();
    let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    for index in 0..count {
        let off = 12 + index * 16;
        if &bytes[off..off + 4] == b"cmap" {
            bytes[off..off + 4].copy_from_slice(b"XXXX");
        }
    }
    let missing = std::rc::Rc::new(Font::load(bytes).unwrap());
    let face = sol_text::Face::new(missing, 24.0).with_fallbacks(vec![primary.clone()]);
    let text = "A\u{301} سلام";
    let shaped = face.shape(text);
    assert!(shaped
        .glyphs
        .iter()
        .all(|glyph| std::rc::Rc::ptr_eq(&glyph.font, &primary)));
    assert!(shaped.glyphs.iter().all(|glyph| glyph.id != 0));
    assert_eq!(
        shaped.width,
        sol_text::Face::new(primary, 24.0).measure(text)
    );
}

#[test]
fn page_font_aliases_do_not_replace_generic_or_system_font_choices() {
    let mut fonts = sol_text::Fonts::from_dirs(Vec::new());
    let system = fonts.face("sans-serif", 16.0, false, false);
    fonts
        .register(
            "Page Alias",
            include_bytes!("../assets/DejaVuSans.ttf").to_vec(),
            false,
            false,
        )
        .unwrap();
    let alias = fonts.face("Page Alias", 16.0, false, false);
    let generic = fonts.face("sans-serif", 16.0, false, false);
    assert!(!std::rc::Rc::ptr_eq(&system.font, &alias.font));
    assert!(std::rc::Rc::ptr_eq(&system.font, &generic.font));
}
