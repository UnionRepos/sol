//! Layout integration tests: parse HTML + CSS, compute styles, lay out.

use sol_dom::{Document, NodeId};
use sol_layout::{Kind, LBox, LayoutInput};
use std::collections::HashMap;
use std::rc::Rc;

fn lay(
    html: &str,
    css: &str,
    w: f32,
    h: f32,
) -> (
    Document,
    sol_layout::Out,
    HashMap<NodeId, Rc<sol_css::Style>>,
) {
    let fonts = sol_text::Fonts::discover();
    let mut doc = Document::new();
    sol_html::parse(html, &mut doc);
    let ua = sol_css::parse_stylesheet(sol_css::UA_CSS);
    let author = sol_css::parse_stylesheet(css);
    let styles = sol_css::style_document(&doc, &[&ua, &author], &sol_css::NoState);
    let image_sizes = HashMap::new();
    let input = LayoutInput {
        styles: &styles,
        fonts: &fonts,
        viewport_w: w,
        viewport_h: h,
        image_sizes: &image_sizes,
    };
    let out = sol_layout::layout(&doc, &input);
    (doc, out, styles)
}

fn lay_imgs(
    html: &str,
    css: &str,
    w: f32,
    h: f32,
    imgs: HashMap<NodeId, (u32, u32)>,
) -> (Document, sol_layout::Out) {
    let fonts = sol_text::Fonts::discover();
    let mut doc = Document::new();
    sol_html::parse(html, &mut doc);
    let ua = sol_css::parse_stylesheet(sol_css::UA_CSS);
    let author = sol_css::parse_stylesheet(css);
    let styles = sol_css::style_document(&doc, &[&ua, &author], &sol_css::NoState);
    let input = LayoutInput {
        styles: &styles,
        fonts: &fonts,
        viewport_w: w,
        viewport_h: h,
        image_sizes: &imgs,
    };
    let out = sol_layout::layout(&doc, &input);
    (doc, out)
}

fn find<'a>(b: &'a LBox, kind: Kind, out: &mut Vec<&'a LBox>) {
    if b.kind == kind {
        out.push(b);
    }
    for k in &b.kids {
        find(k, kind, out);
    }
}

fn find_tag<'a>(doc: &Document, b: &'a LBox, tag: &str, out: &mut Vec<&'a LBox>) {
    if let Some(n) = b.node {
        if doc.is_tag(n, tag) {
            out.push(b);
        }
    }
    for k in &b.kids {
        find_tag(doc, k, tag, out);
    }
}

fn texts<'a>(b: &'a LBox, out: &mut Vec<&'a LBox>) {
    find(b, Kind::Text, out);
}

#[test]
fn block_stacking() {
    let (_doc, out, _s) = lay(
        "<body><div id=a style='height:50px'></div><div id=b style='height:30px'></div></body>",
        "",
        800.0,
        600.0,
    );
    let mut v = Vec::new();
    find_tag(&_doc, &out.root, "div", &mut v);
    assert_eq!(v.len(), 2);
    assert!(
        v[1].rect.y >= v[0].rect.y + 50.0,
        "b below a: {:?}",
        (v[0].rect, v[1].rect)
    );
    assert_eq!(v[0].rect.w, 800.0 - 16.0 - 8.0 * 0.0); // body margin 8*2
}

#[test]
fn margin_collapse_and_center() {
    let (_doc, out, _s) = lay(
        "<body style='margin:0'><div id=a style='margin:20px 0'><p>hi</p></div><div id=b style='width:100px;margin:10px auto;height:10px'></div></body>",
        "",
        800.0,
        600.0,
    );
    let mut v = Vec::new();
    find_tag(&_doc, &out.root, "div", &mut v);
    let b = v[1];
    // auto margins center the 100px box in 800px
    assert!((b.rect.x - 350.0).abs() < 1.0, "centered x: {:?}", b.rect);
}

#[test]
fn text_wraps_into_lines() {
    let (_doc, out, _s) = lay(
        "<body><p style='width:80px;margin:0'>hello world hello world hello world</p></body>",
        "",
        800.0,
        600.0,
    );
    let mut lines = Vec::new();
    find(&out.root, Kind::Line, &mut lines);
    assert!(
        lines.len() >= 3,
        "expected wrapped lines, got {}",
        lines.len()
    );
    // first line starts at left edge of content
    assert!(lines[0].kids.iter().any(|k| k.kind == Kind::Text));
}

#[test]
fn float_and_clear() {
    let (_doc, out, _s) = lay(
        "<body style='margin:0'>\
         <div style='float:left;width:100px;height:100px'></div>\
         <p id=t style='margin:0'>text beside float text beside float text beside float text beside float text beside float</p>\
         <div id=c style='clear:left;height:10px'></div></body>",
        "",
        800.0,
        600.0,
    );
    let mut v = Vec::new();
    find_tag(&_doc, &out.root, "p", &mut v);
    let p = v[0];
    // text should begin to the right of the 100px float
    let mut txt = Vec::new();
    texts(p, &mut txt);
    assert!(!txt.is_empty());
    assert!(
        txt[0].rect.x >= 100.0,
        "text shifted right of float: {:?}",
        txt[0].rect
    );
    let mut v2 = Vec::new();
    find_tag(&_doc, &out.root, "div", &mut v2);
    let cleared = v2.iter().find(|b| (b.rect.h - 10.0).abs() < 0.5).unwrap();
    assert!(
        cleared.rect.y >= 100.0,
        "cleared box below float: {:?}",
        cleared.rect
    );
}

#[test]
fn flex_row() {
    let (_doc, out, _s) = lay(
        "<body style='margin:0'><div style='display:flex'>\
         <div id=a style='width:100px;height:20px'></div>\
         <div id=b style='width:50px;height:20px'></div></div></body>",
        "",
        800.0,
        600.0,
    );
    let mut v = Vec::new();
    find_tag(&_doc, &out.root, "div", &mut v);
    assert!(v.len() >= 3);
    // b sits to the right of a on same row
    assert!(
        v[2].rect.x >= v[1].rect.x + 100.0 - 1.0,
        "flex row: {:?}",
        (v[1].rect, v[2].rect)
    );
    assert!((v[2].rect.y - v[1].rect.y).abs() < 1.0);
}

#[test]
fn table_cells() {
    let (_doc, out, _s) = lay(
        "<body style='margin:0'><table><tr><td style='width:200px'>a</td><td>b</td></tr></table></body>",
        "",
        800.0,
        600.0,
    );
    let mut cells = Vec::new();
    find(&out.root, Kind::TableCell, &mut cells);
    assert_eq!(cells.len(), 2);
    assert!(cells[1].rect.x > cells[0].rect.x, "cells side by side");
    assert!(
        (cells[0].rect.w - 200.0).abs() < 30.0,
        "first col ~200px: {:?}",
        cells[0].rect
    );
}

#[test]
fn image_natural_size() {
    let mut doc = Document::new();
    sol_html::parse("<body style='margin:0'><img src=x.png></body>", &mut doc);
    let img_node = doc
        .descendants(doc.root)
        .find(|&n| doc.is_tag(n, "img"))
        .unwrap();
    let mut imgs = HashMap::new();
    imgs.insert(img_node, (64u32, 32u32));
    let (_d, out) = lay_imgs(
        "<body style='margin:0'><img src=x.png></body>",
        "",
        800.0,
        600.0,
        imgs,
    );
    let mut v = Vec::new();
    find(&out.root, Kind::Replaced, &mut v);
    let img = v
        .iter()
        .find(|b| (b.rect.h - 32.0).abs() < 1.0)
        .expect("img box");
    assert!(
        (img.rect.w - 64.0).abs() < 1.0,
        "img natural w: {:?}",
        img.rect
    );
}

#[test]
fn absolute_positioning() {
    let (_doc, out, _s) = lay(
        "<body style='margin:0;position:relative'>\
         <div id=a style='position:absolute;left:50px;top:40px;width:30px;height:30px'></div>\
         <p>flow text</p></body>",
        "",
        800.0,
        600.0,
    );
    let mut v = Vec::new();
    find_tag(&_doc, &out.root, "div", &mut v);
    let a = v[0];
    assert!((a.rect.x - 50.0).abs() < 2.0, "abs x: {:?}", a.rect);
    assert!((a.rect.y - 40.0).abs() < 2.0, "abs y: {:?}", a.rect);
}

#[test]
fn relative_offset() {
    let (_doc, out, _s) = lay(
        "<body style='margin:0'><div id=a style='position:relative;left:10px;top:5px;height:20px'></div></body>",
        "",
        800.0,
        600.0,
    );
    let mut v = Vec::new();
    find_tag(&_doc, &out.root, "div", &mut v);
    assert!((v[0].rect.x - 10.0).abs() < 1.0);
    assert!((v[0].rect.y - 5.0).abs() < 1.0);
}

#[test]
fn list_markers() {
    let (_doc, out, _s) = lay(
        "<body><ul><li>one</li><li>two</li></ul></body>",
        "",
        800.0,
        600.0,
    );
    let mut v = Vec::new();
    find(&out.root, Kind::Text, &mut v);
    let has_bullet = v.iter().any(|b| {
        b.text
            .as_ref()
            .map(|t| t.text.contains('•'))
            .unwrap_or(false)
    });
    assert!(has_bullet, "expected a bullet marker text box");
}

#[test]
fn nested_block_and_inline_mix() {
    let (_doc, out, _s) = lay(
        "<body><div>before <b>bold</b> after<div>block inside</div>trailing</div></body>",
        "",
        800.0,
        600.0,
    );
    // anonymous block should separate inline runs around the inner block
    let mut anon = Vec::new();
    find(&out.root, Kind::AnonymousBlock, &mut anon);
    assert!(!anon.is_empty(), "expected anonymous block boxes");
    let mut txt = Vec::new();
    texts(&out.root, &mut txt);
    let all: String = txt
        .iter()
        .filter_map(|t| t.text.as_ref().map(|r| r.text.clone()))
        .collect();
    assert!(
        all.contains("before") && all.contains("trailing"),
        "got: {all}"
    );
}

#[test]
fn rtl_words_use_visual_order_and_shaped_widths() {
    let (_, out, _) = lay(
        "<body><p style='direction:rtl;font-size:24px'>אחד שתיים שלוש</p></body>",
        "",
        400.0,
        200.0,
    );
    let mut runs = Vec::new();
    texts(&out.root, &mut runs);
    let first = runs
        .iter()
        .find(|b| b.text.as_ref().is_some_and(|t| t.text == "אחד"))
        .unwrap();
    let last = runs
        .iter()
        .find(|b| b.text.as_ref().is_some_and(|t| t.text == "שלוש"))
        .unwrap();
    assert!(first.rect.x > last.rect.x);
    for run in runs {
        let text = run.text.as_ref().unwrap();
        let face = out.faces.get(text.face_idx);
        assert!((run.rect.w - face.shape(&text.text).width).abs() < 0.01);
    }
}

#[test]
fn grid_tracks_spans_gaps_and_stretch_produce_two_dimensional_layout() {
    let (doc,out,_)=lay("<main id=g><div id=a></div><div id=b></div><div id=c></div></main>","body{margin:0} main{display:grid;width:310px;grid-template-columns:100px 1fr;grid-template-rows:30px 40px;gap:10px} #c{grid-column:1 / span 2}",800.0,600.0);
    let rect = |id: &str| {
        let mut boxes = Vec::new();
        out.root.walk(&mut boxes);
        boxes
            .iter()
            .find(|b| b.node == doc.get_element_by_id(id))
            .unwrap()
            .rect
    };
    let a = rect("a");
    let b = rect("b");
    let c = rect("c");
    assert_eq!((a.x, a.y, a.w, a.h), (0.0, 0.0, 100.0, 30.0));
    assert_eq!((b.x, b.y, b.w, b.h), (110.0, 0.0, 200.0, 30.0));
    assert_eq!((c.x, c.y, c.w, c.h), (0.0, 40.0, 310.0, 40.0));
}

#[test]
fn grid_repeat_minmax_and_explicit_cells_reserve_space_before_auto_placement() {
    let (doc,out,_)=lay("<main><div id=a></div><div id=b></div><div id=c></div></main>","body{margin:0} main{display:grid;width:210px;grid-template-columns:repeat(2,minmax(20px,1fr));grid-auto-rows:25px;gap:10px} #c{grid-column:1;grid-row:1}",800.0,600.0);
    let mut boxes = Vec::new();
    out.root.walk(&mut boxes);
    let rect = |id: &str| {
        boxes
            .iter()
            .find(|b| b.node == doc.get_element_by_id(id))
            .unwrap()
            .rect
    };
    assert_eq!((rect("a").x, rect("a").w), (110.0, 100.0));
    assert_eq!((rect("b").x, rect("b").y), (0.0, 35.0));
    assert_eq!((rect("c").x, rect("c").y), (0.0, 0.0));
}

#[test]
fn named_grid_areas_span_cells_and_preserve_item_source_order() {
    let (doc,out,_)=lay("<main><div id=header></div><div id=left></div><div id=right></div></main>","body{margin:0} main{display:grid;width:200px;grid-template-columns:80px 120px;grid-template-rows:20px 40px;grid-template-areas:'head head' 'left right'} #header{grid-area:head} #left{grid-area:left} #right{grid-area:right}",800.0,600.0);
    let mut boxes = Vec::new();
    out.root.walk(&mut boxes);
    let rect = |id: &str| {
        boxes
            .iter()
            .find(|b| b.node == doc.get_element_by_id(id))
            .unwrap()
            .rect
    };
    assert_eq!(
        (rect("header").x, rect("header").w, rect("header").h),
        (0.0, 200.0, 20.0)
    );
    assert_eq!((rect("right").x, rect("right").y), (80.0, 20.0));
}

#[test]
fn inline_block_placement_translates_its_text_descendants_with_the_box() {
    let (doc, out, _) = lay(
        "<div><span id=inline>label</span></div>",
        "body{margin:0} div{padding:40px} span{display:inline-block;width:100px;padding:10px}",
        800.0,
        600.0,
    );
    let mut boxes = Vec::new();
    out.root.walk(&mut boxes);
    let inline = boxes
        .iter()
        .find(|b| b.node == doc.get_element_by_id("inline") && b.kind == Kind::InlineBlock)
        .unwrap();
    let mut children = Vec::new();
    inline.walk(&mut children);
    let text = children
        .into_iter()
        .find(|box_| box_.text.is_some())
        .unwrap();
    assert!(text.rect.x >= inline.rect.x + 10.0);
    assert!(text.rect.y >= inline.rect.y + 10.0);
}
