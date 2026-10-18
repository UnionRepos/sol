//! Paint tests: display list content + actual rasterization to PNG.

use sol_dom::Document;
use sol_layout::LayoutInput;
use std::collections::HashMap;
use std::rc::Rc;

fn shot(html: &str, css: &str, w: u32, h: u32) -> (sol_paint::Canvas, sol_paint::DisplayList) {
    let fonts = sol_text::Fonts::discover();
    let mut doc = Document::new();
    sol_html::parse(html, &mut doc);
    let ua = sol_css::parse_stylesheet(sol_css::UA_CSS);
    let author = sol_css::parse_stylesheet(css);
    let styles = sol_css::style_document(&doc, &[&ua, &author], &sol_css::NoState);
    let imgs = HashMap::new();
    let input = LayoutInput {
        styles: &styles,
        fonts: &fonts,
        viewport_w: w as f32,
        viewport_h: h as f32,
        image_sizes: &imgs,
    };
    let out = sol_layout::layout(&doc, &input);
    let mut dl = sol_paint::DisplayList::default();
    sol_paint::build_dl(&doc, &out.root, &mut dl);
    let mut canvas = sol_paint::Canvas::new(w, h);
    canvas.clear(sol_paint::Rgba(255, 255, 255, 255));
    let fallback = fonts.face("sans-serif", 16.0, false, false);
    let images: HashMap<sol_dom::NodeId, Rc<sol_img::Image>> = HashMap::new();
    sol_paint::render(&dl, &out.faces, &images, &fallback, &mut canvas);
    (canvas, dl)
}

fn px(c: &sol_paint::Canvas, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * c.w + x) * 4) as usize;
    [c.px[i], c.px[i + 1], c.px[i + 2], c.px[i + 3]]
}

#[test]
fn fills_background_and_border() {
    let (c, _dl) = shot(
        "<body style='margin:0'><div style='background:#f00;width:100px;height:100px;border:10px solid #00f'></div></body>",
        "",
        400,
        300,
    );
    // inside the red box
    assert_eq!(px(&c, 50, 50), [255, 0, 0, 255]);
    // on the top border
    assert_eq!(px(&c, 50, 3), [0, 0, 255, 255]);
    // outside
    assert_eq!(px(&c, 200, 200), [255, 255, 255, 255]);
}

#[test]
fn draws_text_glyphs() {
    let (c, _dl) = shot(
        "<body><p style='color:#000;font-size:20px'>Hello</p></body>",
        "",
        400,
        300,
    );
    // count dark pixels — real glyphs produce coverage
    let dark =
        c.px.as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] < 100 && p[1] < 100 && p[2] < 100)
            .count();
    assert!(dark > 50, "expected glyph pixels, got {dark}");
}

#[test]
fn image_blit() {
    let fonts = sol_text::Fonts::discover();
    let mut doc = Document::new();
    sol_html::parse("<body style='margin:0'><img src=x.png></body>", &mut doc);
    let img_node = doc
        .descendants(doc.root)
        .find(|&n| doc.is_tag(n, "img"))
        .unwrap();
    // 8x8 red image
    let mut px8 = vec![0u8; 8 * 8 * 4];
    for p in px8.as_chunks_mut::<4>().0 {
        p[0] = 255;
        p[3] = 255;
    }
    let img = Rc::new(sol_img::Image {
        w: 8,
        h: 8,
        px: px8,
    });
    let ua = sol_css::parse_stylesheet(sol_css::UA_CSS);
    let styles = sol_css::style_document(&doc, &[&ua], &sol_css::NoState);
    let mut imgs = HashMap::new();
    imgs.insert(img_node, (8u32, 8u32));
    let input = LayoutInput {
        styles: &styles,
        fonts: &fonts,
        viewport_w: 400.0,
        viewport_h: 300.0,
        image_sizes: &imgs,
    };
    let out = sol_layout::layout(&doc, &input);
    let mut dl = sol_paint::DisplayList::default();
    sol_paint::build_dl(&doc, &out.root, &mut dl);
    let mut canvas = sol_paint::Canvas::new(400, 300);
    canvas.clear(sol_paint::Rgba(255, 255, 255, 255));
    let fallback = fonts.face("sans-serif", 16.0, false, false);
    let mut images: HashMap<sol_dom::NodeId, Rc<sol_img::Image>> = HashMap::new();
    images.insert(img_node, img);
    sol_paint::render(&dl, &out.faces, &images, &fallback, &mut canvas);
    assert_eq!(px(&canvas, 4, 4), [255, 0, 0, 255]);
}

#[test]
fn dl_order_is_deterministic() {
    let (_c, dl) = shot(
        "<body><div style='background:#000'></div><p>hi</p></body>",
        "",
        200,
        200,
    );
    let n1 = dl.prims.len();
    let (_c2, dl2) = shot(
        "<body><div style='background:#000'></div><p>hi</p></body>",
        "",
        200,
        200,
    );
    assert_eq!(n1, dl2.prims.len());
}

#[test]
fn transparent_compositing_preserves_straight_colors() {
    use sol_paint::{Canvas, Rgba};
    let mut c = Canvas::new(1, 1);
    let rect = sol_layout::Rect {
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 1.0,
    };
    c.fill_rect(rect, Rgba(255, 0, 0, 128));
    assert_eq!(px(&c, 0, 0), [255, 0, 0, 128]);
    c.fill_rect(rect, Rgba(0, 0, 255, 128));
    assert_eq!(px(&c, 0, 0), [85, 0, 170, 192]);
    let image = sol_img::Image {
        w: 1,
        h: 1,
        px: c.px.clone(),
    };
    assert_eq!(
        sol_img::decode(&sol_img::encode_png(&image)).unwrap().px,
        c.px
    );
}

#[test]
fn fractional_rectangles_have_antialiased_edges() {
    let mut c = sol_paint::Canvas::new(2, 1);
    c.fill_rect(
        sol_layout::Rect {
            x: 0.5,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        },
        sol_paint::Rgba(255, 0, 0, 255),
    );
    assert_eq!(px(&c, 0, 0), [255, 0, 0, 128]);
    assert_eq!(px(&c, 1, 0), [255, 0, 0, 128]);
}

#[test]
fn bilinear_scaling_avoids_transparent_color_halos() {
    let image = sol_img::Image {
        w: 2,
        h: 1,
        px: vec![255, 0, 0, 255, 0, 0, 255, 0],
    };
    let mut c = sol_paint::Canvas::new(3, 1);
    c.blit(
        &image,
        sol_layout::Rect {
            x: 0.0,
            y: 0.0,
            w: 3.0,
            h: 1.0,
        },
    );
    assert_eq!(px(&c, 1, 0), [255, 0, 0, 128]);
    assert_eq!(px(&c, 2, 0), [0, 0, 0, 0]);
}

#[test]
fn invalid_geometry_and_short_images_are_ignored() {
    let mut c = sol_paint::Canvas::new(2, 2);
    for value in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
        c.fill_rect(
            sol_layout::Rect {
                x: 0.0,
                y: 0.0,
                w: value,
                h: 1.0,
            },
            sol_paint::Rgba(255, 0, 0, 255),
        );
    }
    c.blit(
        &sol_img::Image {
            w: 1,
            h: 1,
            px: vec![],
        },
        sol_layout::Rect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        },
    );
    assert!(c.px.iter().all(|&v| v == 0));
}

#[test]
fn canvas_rejects_overflow_and_excessive_allocations() {
    for (w, h) in [(0, 1), (1, 0), (u32::MAX, u32::MAX), (16384, 16384)] {
        assert!(sol_paint::Canvas::try_new(w, h).is_err());
    }
    assert!(sol_paint::Canvas::try_new(2, 2).is_ok());
}

#[test]
fn extreme_text_positions_and_border_lengths_do_not_overflow_or_hang() {
    use sol_paint::{Canvas, DisplayList, Prim, Rgba};
    let fonts = sol_text::Fonts::from_dirs(Vec::new());
    let face = fonts.face("sans-serif", 16.0, false, false);
    let mut canvas = Canvas::new(4, 4);
    canvas.draw_text(&face, f32::MAX, 16.0, "hidden", Rgba(0, 0, 0, 255));
    canvas.draw_text(&face, 0.0, f32::MAX, "hidden", Rgba(0, 0, 0, 255));
    assert!(canvas.px.iter().all(|&byte| byte == 0));
    let dl = DisplayList {
        prims: vec![Prim::Border {
            rect: sol_layout::Rect {
                x: 0.0,
                y: 0.0,
                w: 1e30,
                h: 1.0,
            },
            w: sol_layout::Edges {
                top: 1.0,
                ..Default::default()
            },
            colors: [Rgba(255, 0, 0, 255); 4],
            styles: [1; 4],
        }],
    };
    sol_paint::render(
        &dl,
        &sol_layout::FaceSet { faces: Vec::new() },
        &HashMap::new(),
        &face,
        &mut canvas,
    );
    assert_eq!(px(&canvas, 0, 0), [255, 0, 0, 255]);
}

#[test]
fn rounded_borders_preserve_backgrounds_and_clip_descendants() {
    let (canvas,_) = shot("<body style='margin:0'><div style='width:40px;height:40px;border:4px solid blue;border-radius:16px;overflow:hidden;background:red'><div style='width:100px;height:100px;background:lime'></div></div></body>","",80,80);
    assert_eq!(px(&canvas, 0, 0), [255, 255, 255, 255]);
    assert_eq!(px(&canvas, 24, 1), [0, 0, 255, 255]);
    assert_eq!(px(&canvas, 24, 24), [0, 255, 0, 255]);
    assert_eq!(px(&canvas, 55, 24), [255, 255, 255, 255]);
}

#[test]
fn css_transform_order_origin_and_nested_clips_are_applied() {
    let (canvas,_) = shot("<body style='margin:0'><div style='width:10px;height:10px;background:red;transform-origin:0 0;transform:translate(20px,10px) scale(2)'><div style='width:2px;height:2px;background:blue;transform:translateX(2px)'></div></div></body>","",60,50);
    assert_eq!(px(&canvas, 0, 0), [255, 255, 255, 255]);
    assert_eq!(px(&canvas, 21, 12), [255, 0, 0, 255]);
    assert_eq!(px(&canvas, 25, 12), [0, 0, 255, 255]);
    assert_eq!(px(&canvas, 38, 28), [255, 0, 0, 255]);
    assert_eq!(px(&canvas, 41, 30), [255, 255, 255, 255]);
}

#[test]
fn transformed_images_text_and_clip_stack_restore() {
    use sol_layout::Rect;
    use sol_paint::{Canvas, Rgba, Transform};
    let mut canvas = Canvas::new(80, 40);
    canvas.push_transform(Transform::from_translate(20.0, 0.0));
    canvas.push_rounded_clip(
        Rect {
            x: 0.0,
            y: 0.0,
            w: 20.0,
            h: 20.0,
        },
        [(8.0, 8.0); 4],
    );
    canvas.blit(
        &sol_img::Image {
            w: 1,
            h: 1,
            px: vec![255, 0, 0, 255],
        },
        Rect {
            x: 0.0,
            y: 0.0,
            w: 30.0,
            h: 30.0,
        },
    );
    assert_eq!(px(&canvas, 20, 0), [0; 4]);
    assert_eq!(px(&canvas, 30, 10), [255, 0, 0, 255]);
    canvas.pop_clip();
    canvas.pop_transform();
    canvas.fill_rect(
        Rect {
            x: 0.0,
            y: 0.0,
            w: 2.0,
            h: 2.0,
        },
        Rgba(0, 0, 255, 255),
    );
    assert_eq!(px(&canvas, 0, 0), [0, 0, 255, 255]);
    let fonts = sol_text::Fonts::from_dirs(Vec::new());
    let face = fonts.face("sans-serif", 16.0, false, false);
    canvas.push_transform(Transform::from_translate(40.0, 15.0));
    canvas.draw_text(&face, 0.0, 16.0, "ffi", Rgba(0, 255, 0, 255));
    assert!(canvas
        .px
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[1] > 100 && pixel[3] > 0));
}

#[test]
fn rounded_transform_screenshot_matches_fixture() {
    let html = include_str!("fixtures/rounded-transform.html");
    let (canvas, _) = shot(html, "", 96, 80);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rounded-transform.png");
    if std::env::var_os("SOL_UPDATE_SCREENSHOTS").is_some() {
        let image = sol_img::Image {
            w: canvas.w,
            h: canvas.h,
            px: canvas.px.clone(),
        };
        std::fs::write(&path, sol_img::encode_png(&image)).unwrap();
    }
    let reference = sol_img::decode(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!((canvas.w, canvas.h), (reference.w, reference.h));
    let differences = canvas
        .px
        .iter()
        .zip(&reference.px)
        .filter(|(a, b)| a.abs_diff(**b) > 2)
        .count();
    assert_eq!(
        differences, 0,
        "screenshot differs in {differences} channels"
    );
}
