//! sol_paint: display list construction + software rasterization.
//!
//! Pipeline: layout tree → DisplayList (ordered primitives) → Canvas
//! (RGBA8 framebuffer). Painting follows CSS paint order per stacking
//! context (background → block descendants → floats → inline →
//! positioned), approximated by ordered DFS since absolute boxes are
//! appended last by layout.

mod background;
mod effects;
mod graphics;
mod shadows;
pub use shadows::Shadow;
mod stacking;
pub use graphics::{
    contains_rounded, contains_rounded_rect, css_transform, transformed_rect, Radii,
};
use sol_css::{Style, Value};
use sol_dom::{Document, NodeId};
use sol_img::Image;
use sol_layout::{FaceSet, Kind, LBox, Rect};
use std::collections::HashMap;
use std::rc::Rc;
pub use tiny_skia::{Point, Transform};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

#[derive(Clone, Debug)]
pub enum Prim {
    Shadow {
        rect: Rect,
        radii: Radii,
        shadow: shadows::Shadow,
    },
    TextShadow {
        x: f32,
        baseline: f32,
        face_idx: usize,
        text: String,
        shadow: shadows::Shadow,
    },
    Background {
        border: Rect,
        padding: Rect,
        content: Rect,
        radii: Radii,
        style: Rc<Style>,
    },
    PushOpacity(f32),
    PopOpacity,
    Hit {
        node: NodeId,
        rect: Rect,
        radii: Radii,
        enabled: bool,
    },
    RoundedFill {
        rect: Rect,
        radii: Radii,
        color: Rgba,
    },
    RoundedBorder {
        rect: Rect,
        radii: Radii,
        w: sol_layout::Edges,
        colors: [Rgba; 4],
        styles: [u8; 4],
    },
    RoundedClip {
        rect: Rect,
        radii: Radii,
    },
    PushTransform(tiny_skia::Transform),
    PopTransform,
    Fill {
        rect: Rect,
        color: Rgba,
    },
    Text {
        x: f32,
        baseline: f32,
        face_idx: usize,
        text: String,
        color: Rgba,
    },
    Img {
        rect: Rect,
        node: NodeId,
    },
    FittedImg {
        rect: Rect,
        node: NodeId,
        fit: String,
        position: Vec<Value>,
    },
    Border {
        rect: Rect,
        w: sol_layout::Edges,
        colors: [Rgba; 4],
        styles: [u8; 4],
    },
    Clip {
        rect: Rect,
    },
    PopClip,
    /// underline/overline/line-through line
    Deco {
        x0: f32,
        x1: f32,
        y: f32,
        thick: f32,
        color: Rgba,
    },
}

#[derive(Default)]
pub struct DisplayList {
    pub prims: Vec<Prim>,
}

impl DisplayList {
    pub fn hit(&self, x: f32, y: f32) -> Option<NodeId> {
        let mut transform = Transform::identity();
        let mut transforms = Vec::new();
        let mut clips = Vec::new();
        let mut hit = None;
        let point = |matrix: Transform| {
            let mut p = Point::from_xy(x, y);
            matrix.invert()?.map_point(&mut p);
            Some(p)
        };
        for primitive in &self.prims {
            match primitive {
                Prim::PushTransform(matrix) => {
                    transforms.push(transform);
                    transform = transform.pre_concat(*matrix);
                }
                Prim::PopTransform => {
                    if let Some(matrix) = transforms.pop() {
                        transform = matrix;
                    }
                }
                Prim::RoundedClip { rect, radii } => clips.push((transform, *rect, *radii)),
                Prim::Clip { rect } => clips.push((transform, *rect, [(0.0, 0.0); 4])),
                Prim::PopClip => {
                    clips.pop();
                }
                Prim::Hit {
                    node,
                    rect,
                    radii,
                    enabled: true,
                } if point(transform)
                    .is_some_and(|p| contains_rounded_rect(*radii, *rect, p.x, p.y))
                    && clips.iter().all(|(matrix, rect, radii)| {
                        point(*matrix)
                            .is_some_and(|p| contains_rounded_rect(*radii, *rect, p.x, p.y))
                    }) =>
                {
                    hit = Some(*node);
                }
                _ => {}
            }
        }
        hit
    }
}

fn color_of(st: &Style, name: &str) -> Option<Rgba> {
    let v = st.get(name)?.first()?;
    match v {
        Value::Color(r, g, b, a) => Some(Rgba(*r, *g, *b, *a)),
        Value::Ident(s) | Value::Keyword(s) => match s.as_str() {
            "currentcolor" if name != "color" => st
                .color("color")
                .map(|c| Rgba(c[0], c[1], c[2], c[3]))
                .or(Some(Rgba(0, 0, 0, 255))),
            "transparent" => None,
            _ => None,
        },
        _ => None,
    }
}

fn is_hidden(st: Option<&Rc<Style>>) -> bool {
    st.as_ref()
        .and_then(|s| s.ident("visibility"))
        .map(|v| v == "hidden" || v == "collapse")
        .unwrap_or(false)
}

fn overflow_clip(st: Option<&Rc<Style>>) -> bool {
    st.as_ref()
        .and_then(|s| {
            s.ident("overflow-x")
                .filter(|value| value != "visible")
                .or_else(|| s.ident("overflow-y").filter(|value| value != "visible"))
                .or_else(|| s.ident("overflow"))
        })
        .map(|v| matches!(v.as_str(), "hidden" | "scroll" | "auto"))
        .unwrap_or(false)
}

/// Build the display list from a laid-out tree.
pub fn build_dl(doc: &Document, root: &LBox, dl: &mut DisplayList) {
    fn find<'a>(doc: &Document, box_: &'a LBox, tag: &str) -> Option<&'a LBox> {
        if box_.node.is_some_and(|node| doc.is_tag(node, tag)) {
            return Some(box_);
        }
        box_.kids.iter().find_map(|child| find(doc, child, tag))
    }
    let active = |box_: &&LBox| {
        box_.style.as_ref().is_some_and(|style| {
            style.color("background-color").is_some_and(|c| c[3] > 0)
                || style.get("background-image").is_some_and(|values| {
                    values
                        .iter()
                        .any(|v| matches!(v, Value::Url(_) | Value::Func(..)))
                })
        })
    };
    let source = find(doc, root, "html")
        .filter(active)
        .or_else(|| find(doc, root, "body").filter(active));
    if let Some(source) = source {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            w: root.rect.w,
            h: root
                .rect
                .h
                .max(sol_css::style::viewport::HEIGHT.get() as f32),
        };
        dl.prims.push(Prim::Background {
            border: rect,
            padding: rect,
            content: rect,
            radii: [(0.0, 0.0); 4],
            style: source.style.as_ref().unwrap().clone(),
        });
    }
    stacking::paint(doc, root, dl, source.and_then(|box_| box_.node));
}

pub(crate) fn paint_single(
    doc: &Document,
    b: &LBox,
    dl: &mut DisplayList,
    canvas_source: Option<NodeId>,
) {
    if is_hidden(b.style.as_ref()) {
        return;
    }
    let st = b.style.as_ref();
    if let Some(node) = b.node {
        dl.prims.push(Prim::Hit {
            node,
            rect: b.rect,
            radii: st.map(|s| graphics::radii(s, b.rect)).unwrap_or_default(),
            enabled: st.is_none_or(|s| s.ident("pointer-events").as_deref() != Some("none")),
        });
    }
    let radii = st
        .map(|s| graphics::radii(s, b.rect))
        .unwrap_or([(0.0, 0.0); 4]);
    // background + borders
    match b.kind {
        Kind::Text | Kind::Line | Kind::AnonymousBlock => {}
        _ => {
            if let Some(s) = st {
                shadows::boxes(s, b.rect, radii, false, dl);
                if b.node != canvas_source || b.node.is_none() {
                    dl.prims.push(Prim::Background {
                        border: b.rect,
                        padding: b.padding_rect(),
                        content: b.content_rect(),
                        radii,
                        style: s.clone(),
                    });
                }
                shadows::boxes(
                    s,
                    b.padding_rect(),
                    graphics::inset_radii(radii, b.border),
                    true,
                    dl,
                );
                push_border(b, s, dl);
            }
        }
    }
    let image_clip = b.kind == Kind::Replaced && radii.iter().any(|&(x, y)| x > 0.0 || y > 0.0);
    if image_clip {
        dl.prims.push(Prim::RoundedClip {
            rect: b.rect,
            radii,
        });
    }
    // own content
    match b.kind {
        Kind::Text | Kind::Marker => {
            if let Some(tr) = &b.text {
                let color = st
                    .and_then(|s| color_of(s, "color"))
                    .unwrap_or(Rgba(0, 0, 0, 255));
                let baseline = b.rect.y + b.baseline;
                if let Some(style) = st {
                    shadows::text(style, b.rect.x, baseline, tr.face_idx, &tr.text, dl);
                }
                dl.prims.push(Prim::Text {
                    x: b.rect.x,
                    baseline,
                    face_idx: tr.face_idx,
                    text: tr.text.clone(),
                    color,
                });
                if let Some(s) = st {
                    let deco = s
                        .ident("text-decoration-line")
                        .or_else(|| s.ident("text-decoration"))
                        .unwrap_or_default();
                    let thick = (b.rect.h / 12.0).clamp(1.0, 3.0);
                    if deco.contains("underline") {
                        dl.prims.push(Prim::Deco {
                            x0: b.rect.x,
                            x1: b.rect.right(),
                            y: baseline + thick,
                            thick,
                            color,
                        });
                    }
                    if deco.contains("overline") {
                        dl.prims.push(Prim::Deco {
                            x0: b.rect.x,
                            x1: b.rect.right(),
                            y: b.rect.y,
                            thick,
                            color,
                        });
                    }
                    if deco.contains("line-through") {
                        dl.prims.push(Prim::Deco {
                            x0: b.rect.x,
                            x1: b.rect.right(),
                            y: baseline - b.rect.h * 0.3,
                            thick,
                            color,
                        });
                    }
                }
            }
        }
        Kind::Replaced => {
            if let Some(n) = b.node {
                let tag = doc.tag(n).to_string();
                if tag == "img" {
                    if let Some(style) = st.filter(|style| {
                        style.get("object-fit").is_some() || style.get("object-position").is_some()
                    }) {
                        dl.prims.push(Prim::FittedImg {
                            rect: b.content_rect(),
                            node: n,
                            fit: style.ident("object-fit").unwrap_or_else(|| "fill".into()),
                            position: style.get("object-position").cloned().unwrap_or_else(|| {
                                vec![Value::Percentage(50.0), Value::Percentage(50.0)]
                            }),
                        });
                    } else {
                        dl.prims.push(Prim::Img {
                            rect: b.content_rect(),
                            node: n,
                        });
                    }
                } else {
                    paint_form(doc, n, b, &tag, dl);
                }
            }
        }
        _ => {}
    }
    if image_clip {
        dl.prims.push(Prim::PopClip);
    }
}

pub(crate) fn paint_outline(box_: &LBox, dl: &mut DisplayList) {
    if is_hidden(box_.style.as_ref())
        || matches!(
            box_.kind,
            Kind::Text | Kind::Line | Kind::AnonymousBlock | Kind::Marker
        )
    {
        return;
    }
    let Some(style) = &box_.style else { return };
    let kind = style
        .ident("outline-style")
        .unwrap_or_else(|| "none".into());
    let kind = match kind.as_str() {
        "solid" | "auto" => 0,
        "dashed" => 1,
        "dotted" => 2,
        "double" => 3,
        _ => return,
    };
    let width = style
        .length_px("outline-width", 16.0, box_.rect.w as f64)
        .unwrap_or(3.0)
        .max(0.0) as f32;
    let offset = style.length_px("outline-offset", 16.0, 0.0).unwrap_or(0.0) as f32;
    let distance = width + offset;
    let rect = Rect {
        x: box_.rect.x - distance,
        y: box_.rect.y - distance,
        w: box_.rect.w + 2.0 * distance,
        h: box_.rect.h + 2.0 * distance,
    };
    let radii = graphics::radii(style, box_.rect)
        .map(|(x, y)| ((x + distance).max(0.0), (y + distance).max(0.0)));
    let color = color_of(style, "outline-color")
        .or_else(|| color_of(style, "color"))
        .unwrap_or(Rgba(0, 0, 0, 255));
    dl.prims.push(Prim::RoundedBorder {
        rect,
        radii,
        w: sol_layout::Edges {
            top: width,
            right: width,
            bottom: width,
            left: width,
        },
        colors: [color; 4],
        styles: [kind; 4],
    });
}

fn push_border(b: &LBox, s: &Rc<Style>, dl: &mut DisplayList) {
    let styles = [
        s.ident("border-top-style").unwrap_or_default(),
        s.ident("border-right-style").unwrap_or_default(),
        s.ident("border-bottom-style").unwrap_or_default(),
        s.ident("border-left-style").unwrap_or_default(),
    ];
    let smap = |v: &str| match v {
        "solid" => 0u8,
        "dashed" => 1,
        "dotted" => 2,
        "double" => 3,
        _ => 255,
    };
    let st8 = [
        smap(&styles[0]),
        smap(&styles[1]),
        smap(&styles[2]),
        smap(&styles[3]),
    ];
    if st8.iter().all(|&x| x == 255) {
        return;
    }
    let def = Rgba(0, 0, 0, 255);
    let colors = [
        color_of(s, "border-top-color").unwrap_or(def),
        color_of(s, "border-right-color").unwrap_or(def),
        color_of(s, "border-bottom-color").unwrap_or(def),
        color_of(s, "border-left-color").unwrap_or(def),
    ];
    let radii = graphics::radii(s, b.rect);
    if radii.iter().any(|&(x, y)| x > 0.0 || y > 0.0) {
        dl.prims.push(Prim::RoundedBorder {
            rect: b.rect,
            radii,
            w: b.border,
            colors,
            styles: st8,
        });
    } else {
        dl.prims.push(Prim::Border {
            rect: b.rect,
            w: b.border,
            colors,
            styles: st8,
        });
    }
}

fn paint_form(doc: &Document, n: NodeId, b: &LBox, tag: &str, dl: &mut DisplayList) {
    let r = b.content_rect();
    match tag {
        "hr" => {
            dl.prims.push(Prim::Fill {
                rect: Rect {
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h.max(1.0),
                },
                color: Rgba(128, 128, 128, 255),
            });
        }
        "input" => {
            let ty = doc.attr(n, "type").unwrap_or("text").to_ascii_lowercase();
            match ty.as_str() {
                "checkbox" | "radio" => {
                    dl.prims.push(Prim::Fill {
                        rect: r,
                        color: Rgba(255, 255, 255, 255),
                    });
                    dl.prims.push(Prim::Border {
                        rect: r,
                        w: sol_layout::Edges {
                            top: 1.0,
                            right: 1.0,
                            bottom: 1.0,
                            left: 1.0,
                        },
                        colors: [Rgba(118, 118, 118, 255); 4],
                        styles: [0; 4],
                    });
                    if doc.attr(n, "checked").is_some() {
                        dl.prims.push(Prim::Fill {
                            rect: Rect {
                                x: r.x + 3.0,
                                y: r.y + 3.0,
                                w: r.w - 6.0,
                                h: r.h - 6.0,
                            },
                            color: Rgba(30, 30, 30, 255),
                        });
                    }
                }
                "hidden" => {}
                _ => {
                    let is_btn = matches!(ty.as_str(), "submit" | "button" | "reset");
                    dl.prims.push(Prim::Fill {
                        rect: r,
                        color: if is_btn {
                            Rgba(239, 239, 239, 255)
                        } else {
                            Rgba(255, 255, 255, 255)
                        },
                    });
                    dl.prims.push(Prim::Border {
                        rect: r,
                        w: sol_layout::Edges {
                            top: 1.0,
                            right: 1.0,
                            bottom: 1.0,
                            left: 1.0,
                        },
                        colors: [Rgba(118, 118, 118, 255); 4],
                        styles: [0; 4],
                    });
                    let label = doc
                        .attr(n, "value")
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| if is_btn { ty.clone() } else { String::new() });
                    if !label.is_empty() {
                        dl.prims.push(Prim::Text {
                            x: r.x
                                + if is_btn {
                                    (r.w - label.len() as f32 * 7.0).max(4.0) / 2.0
                                } else {
                                    4.0
                                },
                            baseline: r.y + r.h * 0.7,
                            face_idx: usize::MAX,
                            text: label,
                            color: Rgba(0, 0, 0, 255),
                        });
                    }
                }
            }
        }
        "button" => {
            dl.prims.push(Prim::Fill {
                rect: r,
                color: Rgba(239, 239, 239, 255),
            });
            dl.prims.push(Prim::Border {
                rect: r,
                w: sol_layout::Edges {
                    top: 1.0,
                    right: 1.0,
                    bottom: 1.0,
                    left: 1.0,
                },
                colors: [Rgba(118, 118, 118, 255); 4],
                styles: [0; 4],
            });
            let txt = doc.text_content(n);
            dl.prims.push(Prim::Text {
                x: r.x + (r.w - txt.len() as f32 * 7.0).max(4.0) / 2.0,
                baseline: r.y + r.h * 0.7,
                face_idx: usize::MAX,
                text: txt,
                color: Rgba(0, 0, 0, 255),
            });
        }
        "select" | "textarea" => {
            dl.prims.push(Prim::Fill {
                rect: r,
                color: Rgba(255, 255, 255, 255),
            });
            dl.prims.push(Prim::Border {
                rect: r,
                w: sol_layout::Edges {
                    top: 1.0,
                    right: 1.0,
                    bottom: 1.0,
                    left: 1.0,
                },
                colors: [Rgba(118, 118, 118, 255); 4],
                styles: [0; 4],
            });
        }
        _ => {}
    }
}

// ---------- rasterization ----------

pub struct Canvas {
    pub w: u32,
    pub h: u32,
    /// Straight (unpremultiplied) RGBA8, row-major.
    pub px: Vec<u8>,
    clip: Rect,
    clip_stack: Vec<(Rect, usize)>,
    masks: Vec<graphics::DeviceMask>,
    transform: tiny_skia::Transform,
    transform_stack: Vec<tiny_skia::Transform>,
    layers: Vec<effects::Layer>,
    suppress: bool,
}

impl Canvas {
    /// Validate dimensions before allocating a framebuffer (at most 64 MiB).
    pub fn validate_dimensions(w: u32, h: u32) -> Result<(), String> {
        if w == 0 || h == 0 || w > 16384 || h > 16384 || w as u64 * h as u64 > 16 * 1024 * 1024 {
            return Err(
                "viewport must be 1..16384 pixels per side and at most 16 megapixels".into(),
            );
        }
        Ok(())
    }

    pub fn new(w: u32, h: u32) -> Canvas {
        Self::try_new(w, h).expect("valid canvas dimensions and available memory")
    }

    pub fn try_new(w: u32, h: u32) -> Result<Canvas, String> {
        Self::validate_dimensions(w, h)?;
        let len = w as usize * h as usize * 4;
        let mut px = Vec::new();
        px.try_reserve_exact(len)
            .map_err(|e| format!("cannot allocate canvas: {e}"))?;
        px.resize(len, 0);
        Ok(Canvas {
            w,
            h,
            px,
            clip: Rect {
                x: 0.0,
                y: 0.0,
                w: w as f32,
                h: h as f32,
            },
            clip_stack: Vec::new(),
            masks: Vec::new(),
            transform: tiny_skia::Transform::identity(),
            transform_stack: Vec::new(),
            layers: Vec::new(),
            suppress: false,
        })
    }

    pub fn clear(&mut self, c: Rgba) {
        for p in self.px.as_chunks_mut::<4>().0 {
            p[0] = c.0;
            p[1] = c.1;
            p[2] = c.2;
            p[3] = c.3;
        }
    }

    fn push_clip(&mut self, r: Rect) {
        self.push_rounded_clip(r, [(0.0, 0.0); 4]);
    }
    pub fn pop_clip(&mut self) {
        if let Some((rect, masks)) = self.clip_stack.pop() {
            self.clip = rect;
            self.masks.truncate(masks);
        }
    }

    #[inline]
    fn blend(&mut self, x: i32, y: i32, c: Rgba, cov: u8) {
        if self.suppress {
            return;
        }
        if (x as f32 + 1.0) <= self.clip.x
            || (y as f32 + 1.0) <= self.clip.y
            || x as f32 >= self.clip.right()
            || y as f32 >= self.clip.bottom()
            || x < 0
            || y < 0
            || x >= self.w as i32
            || y >= self.h as i32
        {
            return;
        }
        let mut cov = cov as u32;
        for mask in &self.masks {
            cov = (cov * mask.coverage(x, y) as u32 + 127) / 255;
        }
        let a = (c.3 as u32 * cov + 127) / 255;
        if a == 0 {
            return;
        }
        let i = ((y as u32 * self.w + x as u32) * 4) as usize;
        let da = self.px[i + 3] as u32;
        let inverse = 255 - a;
        let alpha = a * 255 + da * inverse;
        for (channel, source) in [c.0, c.1, c.2].into_iter().enumerate() {
            self.px[i + channel] =
                ((source as u32 * a * 255 + self.px[i + channel] as u32 * da * inverse + alpha / 2)
                    / alpha) as u8;
        }
        self.px[i + 3] = ((alpha + 127) / 255) as u8;
    }

    pub fn fill_rect(&mut self, r: Rect, c: Rgba) {
        if !self.transform.is_identity() {
            self.fill_rounded_rect(r, [(0.0, 0.0); 4], c);
            return;
        }
        let Some(r) = self.visible_rect(r) else {
            return;
        };
        for y in r.y.floor() as i32..r.bottom().ceil() as i32 {
            for x in r.x.floor() as i32..r.right().ceil() as i32 {
                self.blend(x, y, c, coverage(r, x, y));
            }
        }
    }

    fn visible_rect(&self, r: Rect) -> Option<Rect> {
        if ![r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite()) || r.w <= 0.0 || r.h <= 0.0 {
            return None;
        }
        let x = r.x.max(self.clip.x).max(0.0);
        let y = r.y.max(self.clip.y).max(0.0);
        let right = r.right().min(self.clip.right()).min(self.w as f32);
        let bottom = r.bottom().min(self.clip.bottom()).min(self.h as f32);
        (right > x && bottom > y).then_some(Rect {
            x,
            y,
            w: right - x,
            h: bottom - y,
        })
    }

    pub fn draw_text(&mut self, face: &sol_text::Face, x: f32, baseline: f32, text: &str, c: Rgba) {
        if !x.is_finite() || !baseline.is_finite() {
            return;
        }
        let glyphs = face.shape(text).glyphs;
        let pen = x;
        for glyph in glyphs {
            if !self.transform.is_identity() {
                if let Some(path) = glyph.font.glyph(glyph.id).path {
                    let scale = face.scale() * face.font.units_per_em as f32
                        / glyph.font.units_per_em as f32;
                    if let Some(path) = path.transform(tiny_skia::Transform::from_row(
                        scale,
                        0.0,
                        0.0,
                        -scale,
                        pen + glyph.x,
                        baseline - glyph.y,
                    )) {
                        self.fill_path(&path, tiny_skia::FillRule::Winding, c);
                    }
                }
                continue;
            }
            let scale =
                face.scale() * face.font.units_per_em as f32 / glyph.font.units_per_em as f32;
            let bm = sol_text::raster::rasterize(&glyph.font.glyph(glyph.id), scale);
            let gx = pen + glyph.x + bm.left as f32;
            let gy = baseline - glyph.y - bm.top as f32;
            if !gx.is_finite()
                || !gy.is_finite()
                || gx >= self.clip.right()
                || gx + bm.w as f32 <= self.clip.x
                || gy >= self.clip.bottom()
                || gy + bm.h as f32 <= self.clip.y
            {
                continue;
            }
            let gx = gx as i32;
            let gy = gy as i32;
            for row in 0..bm.h {
                for col in 0..bm.w {
                    let cov = bm.buf[(row * bm.w + col) as usize];
                    if cov > 0 {
                        self.blend(gx + col as i32, gy + row as i32, c, cov);
                    }
                }
            }
        }
    }

    /// Transparent pixels can contain arbitrary RGB values. Weighting those
    /// colors without alpha would create colored fringes around scaled edges,
    /// even though the source pixels are meant to contribute no visible color.
    pub fn blit(&mut self, img: &Image, dst: Rect) {
        if img.w == 0 || img.h == 0 || img.px.len() as u64 != img.w as u64 * img.h as u64 * 4 {
            return;
        }
        if !self.transform.is_identity() {
            self.blit_transformed(img, dst);
            return;
        }
        let Some(visible) = self.visible_rect(dst) else {
            return;
        };
        for y in visible.y.floor() as i32..visible.bottom().ceil() as i32 {
            let sy = ((y as f32 + 0.5 - dst.y) / dst.h * img.h as f32 - 0.5)
                .clamp(0.0, (img.h - 1) as f32);
            for x in visible.x.floor() as i32..visible.right().ceil() as i32 {
                let sx = ((x as f32 + 0.5 - dst.x) / dst.w * img.w as f32 - 0.5)
                    .clamp(0.0, (img.w - 1) as f32);
                let x0 = sx.floor() as u32;
                let y0 = sy.floor() as u32;
                let fx = sx.fract();
                let fy = sy.fract();
                let mut rgba = [0.0; 4];
                for (ix, iy, weight) in [
                    (x0, y0, (1.0 - fx) * (1.0 - fy)),
                    ((x0 + 1).min(img.w - 1), y0, fx * (1.0 - fy)),
                    (x0, (y0 + 1).min(img.h - 1), (1.0 - fx) * fy),
                    ((x0 + 1).min(img.w - 1), (y0 + 1).min(img.h - 1), fx * fy),
                ] {
                    let pixel = img.get(ix, iy);
                    let alpha = pixel[3] as f32 * weight;
                    for channel in 0..3 {
                        rgba[channel] += pixel[channel] as f32 * alpha;
                    }
                    rgba[3] += alpha;
                }
                if rgba[3] > 0.0 {
                    let color = Rgba(
                        (rgba[0] / rgba[3]).round() as u8,
                        (rgba[1] / rgba[3]).round() as u8,
                        (rgba[2] / rgba[3]).round() as u8,
                        rgba[3].round() as u8,
                    );
                    self.blend(x, y, color, coverage(visible, x, y));
                }
            }
        }
    }

    fn border_side(&mut self, r: Rect, w: sol_layout::Edges, colors: [Rgba; 4], styles: [u8; 4]) {
        // top, right, bottom, left — trapezoid approximated by rects
        if styles[0] != 255 && w.top > 0.0 {
            let seg = Rect {
                x: r.x,
                y: r.y,
                w: r.w,
                h: w.top,
            };
            self.border_strip(seg, w.top, colors[0], styles[0], true);
        }
        if styles[2] != 255 && w.bottom > 0.0 {
            let seg = Rect {
                x: r.x,
                y: r.bottom() - w.bottom,
                w: r.w,
                h: w.bottom,
            };
            self.border_strip(seg, w.bottom, colors[2], styles[2], true);
        }
        if styles[3] != 255 && w.left > 0.0 {
            let seg = Rect {
                x: r.x,
                y: r.y + w.top,
                w: w.left,
                h: r.h - w.top - w.bottom,
            };
            self.border_strip(seg, w.left, colors[3], styles[3], false);
        }
        if styles[1] != 255 && w.right > 0.0 {
            let seg = Rect {
                x: r.right() - w.right,
                y: r.y + w.top,
                w: w.right,
                h: r.h - w.top - w.bottom,
            };
            self.border_strip(seg, w.right, colors[1], styles[1], false);
        }
    }

    fn border_strip(&mut self, r: Rect, thick: f32, c: Rgba, style: u8, horiz: bool) {
        if !thick.is_finite() || thick <= 0.0 {
            return;
        }
        let Some(visible) = self.visible_rect(r) else {
            return;
        };
        let start = if horiz {
            visible.x - r.x
        } else {
            visible.y - r.y
        };
        let end = if horiz {
            visible.right() - r.x
        } else {
            visible.bottom() - r.y
        };
        match style {
            1 => {
                // dashed: 3x on, 2x off along the strip
                let step = (thick * 5.0).max(1.0);
                if !step.is_finite() {
                    return;
                }
                let mut t = (start / step).floor() * step;
                let len = end;
                while t < len {
                    let seg = if horiz {
                        Rect {
                            x: r.x + t,
                            y: r.y,
                            w: (thick * 3.0).min(len - t),
                            h: r.h,
                        }
                    } else {
                        Rect {
                            x: r.x,
                            y: r.y + t,
                            w: r.w,
                            h: (thick * 3.0).min(len - t),
                        }
                    };
                    self.fill_rect(seg, c);
                    let next = t + step;
                    if next <= t {
                        break;
                    }
                    t = next;
                }
            }
            2 => {
                // dotted
                let step = (thick * 2.0).max(1.0);
                if !step.is_finite() {
                    return;
                }
                let mut t = (start / step).floor() * step;
                let len = end;
                while t < len {
                    let seg = if horiz {
                        Rect {
                            x: r.x + t,
                            y: r.y,
                            w: thick.min(len - t),
                            h: r.h,
                        }
                    } else {
                        Rect {
                            x: r.x,
                            y: r.y + t,
                            w: r.w,
                            h: thick.min(len - t),
                        }
                    };
                    self.fill_rect(seg, c);
                    let next = t + step;
                    if next <= t {
                        break;
                    }
                    t = next;
                }
            }
            3 => {
                // double: outer + inner lines
                let t2 = (thick / 3.0).max(1.0);
                if horiz {
                    self.fill_rect(
                        Rect {
                            x: r.x,
                            y: r.y,
                            w: r.w,
                            h: t2,
                        },
                        c,
                    );
                    self.fill_rect(
                        Rect {
                            x: r.x,
                            y: r.bottom() - t2,
                            w: r.w,
                            h: t2,
                        },
                        c,
                    );
                } else {
                    self.fill_rect(
                        Rect {
                            x: r.x,
                            y: r.y,
                            w: t2,
                            h: r.h,
                        },
                        c,
                    );
                    self.fill_rect(
                        Rect {
                            x: r.right() - t2,
                            y: r.y,
                            w: t2,
                            h: r.h,
                        },
                        c,
                    );
                }
            }
            _ => self.fill_rect(r, c),
        }
    }
}

/// Render a display list into a canvas.
pub fn render(
    dl: &DisplayList,
    faces: &FaceSet,
    images: &HashMap<NodeId, Rc<Image>>,
    fallback_face: &sol_text::Face,
    canvas: &mut Canvas,
) {
    render_with_resources(dl, faces, images, &HashMap::new(), fallback_face, canvas);
}

pub fn render_with_resources(
    dl: &DisplayList,
    faces: &FaceSet,
    images: &HashMap<NodeId, Rc<Image>>,
    backgrounds: &HashMap<String, Rc<Image>>,
    fallback_face: &sol_text::Face,
    canvas: &mut Canvas,
) {
    for p in &dl.prims {
        match p {
            Prim::Shadow {
                rect,
                radii,
                shadow,
            } => canvas.shadow(*rect, *radii, shadow),
            Prim::TextShadow {
                x,
                baseline,
                face_idx,
                text,
                shadow,
            } => canvas.text_shadow(
                if *face_idx == usize::MAX {
                    fallback_face
                } else {
                    faces.get(*face_idx)
                },
                *x,
                *baseline,
                text,
                shadow,
            ),
            Prim::Background {
                border,
                padding,
                content,
                radii,
                style,
            } => background::paint(
                canvas,
                *border,
                *padding,
                *content,
                *radii,
                style,
                backgrounds,
            ),
            Prim::PushOpacity(opacity) => canvas.push_opacity(*opacity),
            Prim::PopOpacity => canvas.pop_opacity(),
            Prim::Hit { .. } => {}
            Prim::RoundedFill { rect, radii, color } => {
                canvas.fill_rounded_rect(*rect, *radii, *color)
            }
            Prim::RoundedBorder {
                rect,
                radii,
                w,
                colors,
                styles,
            } => canvas.rounded_border(*rect, *radii, *w, *colors, *styles),
            Prim::RoundedClip { rect, radii } => canvas.push_rounded_clip(*rect, *radii),
            Prim::PushTransform(matrix) => canvas.push_transform(*matrix),
            Prim::PopTransform => canvas.pop_transform(),
            Prim::Fill { rect, color } => canvas.fill_rect(*rect, *color),
            Prim::Clip { rect } => canvas.push_clip(*rect),
            Prim::PopClip => canvas.pop_clip(),
            Prim::Text {
                x,
                baseline,
                face_idx,
                text,
                color,
            } => {
                let face = if *face_idx == usize::MAX {
                    fallback_face
                } else {
                    faces.get(*face_idx)
                };
                canvas.draw_text(face, *x, *baseline, text, *color);
            }
            Prim::FittedImg {
                rect,
                node,
                fit,
                position,
            } => {
                if let Some(image) = images.get(node) {
                    let nw = image.w as f32;
                    let nh = image.h as f32;
                    let factor = match fit.as_str() {
                        "contain" => (rect.w / nw).min(rect.h / nh),
                        "cover" => (rect.w / nw).max(rect.h / nh),
                        "scale-down" => (rect.w / nw).min(rect.h / nh).min(1.0),
                        _ => 1.0,
                    };
                    let (w, h) = if fit == "fill" {
                        (rect.w, rect.h)
                    } else {
                        (nw * factor, nh * factor)
                    };
                    let (x, y) = background::position(position, rect.w - w, rect.h - h);
                    canvas.push_clip(*rect);
                    canvas.blit(
                        image,
                        Rect {
                            x: rect.x + x,
                            y: rect.y + y,
                            w,
                            h,
                        },
                    );
                    canvas.pop_clip();
                }
            }
            Prim::Img { rect, node } => {
                if let Some(img) = images.get(node) {
                    canvas.blit(img, *rect);
                } else {
                    // broken image placeholder
                    canvas.fill_rect(*rect, Rgba(230, 230, 230, 255));
                }
            }
            Prim::Border {
                rect,
                w,
                colors,
                styles,
            } => {
                canvas.border_side(*rect, *w, *colors, *styles);
            }
            Prim::Deco {
                x0,
                x1,
                y,
                thick,
                color,
            } => {
                canvas.fill_rect(
                    Rect {
                        x: *x0,
                        y: *y,
                        w: x1 - x0,
                        h: *thick,
                    },
                    *color,
                );
            }
        }
    }
}

fn coverage(rect: Rect, x: i32, y: i32) -> u8 {
    let width = (rect.right().min(x as f32 + 1.0) - rect.x.max(x as f32)).clamp(0.0, 1.0);
    let height = (rect.bottom().min(y as f32 + 1.0) - rect.y.max(y as f32)).clamp(0.0, 1.0);
    (width * height * 255.0).round() as u8
}
