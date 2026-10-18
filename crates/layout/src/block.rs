//! Block + inline layout, floats, positioning, line boxes.

use crate::tree::Ctx;
use crate::{Edges, Kind, LBox, Rect, TextRun};
use sol_css::{Clear, Float, Position, Style, Value};
use sol_dom::NodeId;
use std::rc::Rc;

use crate::{FaceSet, LayoutInput};
use sol_dom::Document;

pub struct LC<'a> {
    pub doc: &'a Document,
    pub input: &'a LayoutInput<'a>,
    pub faces: &'a mut FaceSet,
    pub vw: f32,
    pub vh: f32,
    /// absolutely-positioned boxes appended at root level after flow
    pub deferred: Vec<LBox>,
}

fn mk_face(lc: &LC, style: Option<&sol_css::Style>) -> sol_text::Face {
    crate::tree::make_face(lc.input, style)
}

pub struct Floats {
    lefts: Vec<(f32, f32, f32)>,  // (x_right, y_top, y_bottom)
    rights: Vec<(f32, f32, f32)>, // (x_left, y_top, y_bottom)
}

impl Default for Floats {
    fn default() -> Self {
        Self::new()
    }
}

impl Floats {
    pub fn new() -> Floats {
        Floats {
            lefts: vec![],
            rights: vec![],
        }
    }
    fn add(&mut self, f: Float, x: f32, y: f32, w: f32, h: f32) {
        match f {
            Float::Left => self.lefts.push((x + w, y, y + h)),
            Float::Right => self.rights.push((x, y, y + h)),
            Float::None => {}
        }
    }
    fn left_at(&self, y: f32, h: f32, min: f32) -> f32 {
        let mut x = min;
        for &(xr, t, b) in &self.lefts {
            if y < b && y + h > t {
                x = x.max(xr);
            }
        }
        x
    }
    fn right_at(&self, y: f32, h: f32, max: f32) -> f32 {
        let mut x = max;
        for &(xl, t, b) in &self.rights {
            if y < b && y + h > t {
                x = x.min(xl);
            }
        }
        x
    }
    fn bottom(&self) -> f32 {
        self.lefts
            .iter()
            .chain(&self.rights)
            .map(|f| f.2)
            .fold(0.0, f32::max)
    }
}

// ---------- value resolution ----------

fn fs(style: Option<&Rc<Style>>) -> f64 {
    style
        .and_then(|s| s.length_px("font-size", 16.0, 800.0))
        .unwrap_or(16.0)
}

fn idlike(v: &Value) -> Option<&str> {
    match v {
        Value::Ident(s) | Value::Keyword(s) => Some(s),
        _ => None,
    }
}

pub fn len(style: Option<&Rc<Style>>, name: &str, pct: f32) -> Option<f32> {
    let s = style?;
    let v = s.get(name)?.first()?;
    if let Some(id) = idlike(v) {
        match id {
            "auto" | "none" | "normal" => return None,
            "thin" if name.starts_with("border") => return Some(1.0),
            "medium" if name.starts_with("border") => return Some(3.0),
            "thick" if name.starts_with("border") => return Some(5.0),
            _ => {}
        }
    }
    sol_css::style::value_px(v, fs(style), pct as f64).map(|x| x as f32)
}

fn is_auto(style: Option<&Rc<Style>>, name: &str) -> bool {
    style
        .and_then(|s| s.get(name))
        .and_then(|v| v.first())
        .and_then(idlike)
        .map(|i| i == "auto")
        .unwrap_or(false)
}

pub fn edges(style: Option<&Rc<Style>>, prefix: &str, cw: f32) -> Edges {
    let g = |n: &str| len(style, &format!("{prefix}-{n}"), cw).unwrap_or(0.0);
    Edges {
        top: g("top"),
        right: g("right"),
        bottom: g("bottom"),
        left: g("left"),
    }
}

fn border_width(style: Option<&Rc<Style>>, side: &str, cw: f32) -> f32 {
    let sty = style
        .and_then(|s| s.ident(&format!("border-{side}-style")))
        .unwrap_or_else(|| "none".into());
    if sty == "none" || sty == "hidden" {
        return 0.0;
    }
    len(style, &format!("border-{side}-width"), cw).unwrap_or(match sty.as_str() {
        "thin" => 1.0,
        "thick" => 5.0,
        _ => 3.0,
    })
}

pub fn border_edges(style: Option<&Rc<Style>>, cw: f32) -> Edges {
    Edges {
        top: border_width(style, "top", cw),
        right: border_width(style, "right", cw),
        bottom: border_width(style, "bottom", cw),
        left: border_width(style, "left", cw),
    }
}

// ---------- entry ----------

pub fn layout_document(ctx: &mut Ctx, root: &mut LBox, vw: f32, vh: f32) {
    let mut lc = LC {
        doc: ctx.doc,
        input: ctx.input,
        faces: &mut *ctx.faces,
        vw,
        vh,
        deferred: Vec::new(),
    };
    let mut floats = Floats::new();
    root.rect = Rect {
        x: 0.0,
        y: 0.0,
        w: vw,
        h: 0.0,
    };
    let kids = std::mem::take(&mut root.kids);
    let mut y = 0.0f32;
    let mut prev_margin: Option<f32> = None;
    let mut new_kids = Vec::new();
    let cb = Rect {
        x: 0.0,
        y: 0.0,
        w: vw,
        h: vh,
    };
    for mut k in kids {
        y = lay_block(
            &mut lc,
            &mut k,
            (0.0, y),
            vw,
            &mut floats,
            &mut prev_margin,
            cb,
        );
        new_kids.push(k);
    }
    root.kids = new_kids;
    root.rect.h = y.max(floats.bottom());
    let deferred = std::mem::take(&mut lc.deferred);
    root.kids.extend(deferred);
}

fn is_bfc_root(b: &LBox) -> bool {
    if b.float != Float::None {
        return true;
    }
    match b.kind {
        Kind::InlineBlock | Kind::TableCell | Kind::Table | Kind::Replaced => true,
        Kind::Block => b
            .style
            .as_ref()
            .map(|s| {
                matches!(
                    s.ident("overflow-x").as_deref(),
                    Some("hidden") | Some("auto") | Some("scroll")
                ) || s.position() != Position::Static
            })
            .unwrap_or(false),
        _ => false,
    }
}

fn is_positioned(b: &LBox) -> bool {
    b.style
        .as_ref()
        .map(|s| s.position() != Position::Static)
        .unwrap_or(false)
}

/// Lay out one block-level box. Returns y for the next sibling
/// (past margin-bottom, collapsed).
pub fn lay_block(
    lc: &mut LC,
    b: &mut LBox,
    origin: (f32, f32),
    avail_w: f32,
    floats: &mut Floats,
    prev_margin: &mut Option<f32>,
    cb: Rect,
) -> f32 {
    let (cx, mut y) = origin;
    let st = b.style.clone();
    b.margin = Edges {
        top: len(st.as_ref(), "margin-top", avail_w).unwrap_or(0.0),
        right: len(st.as_ref(), "margin-right", avail_w).unwrap_or(0.0),
        bottom: len(st.as_ref(), "margin-bottom", avail_w).unwrap_or(0.0),
        left: len(st.as_ref(), "margin-left", avail_w).unwrap_or(0.0),
    };
    b.padding = edges(st.as_ref(), "padding", avail_w);
    b.border = border_edges(st.as_ref(), avail_w);
    let pos = st
        .as_ref()
        .map(|s| s.position())
        .unwrap_or(Position::Static);

    if pos == Position::Absolute || pos == Position::Fixed {
        let pcb = if pos == Position::Fixed {
            Rect {
                x: 0.0,
                y: 0.0,
                w: lc.vw,
                h: lc.vh,
            }
        } else {
            cb
        };
        position_absolute(lc, b, pcb);
        return y;
    }

    // margin collapsing
    match prev_margin {
        None => y += b.margin.top,
        Some(pm) => y += (*pm).max(b.margin.top) - *pm,
    }
    *prev_margin = Some(b.margin.bottom);

    if b.float != Float::None {
        lay_float(lc, b, cx, y, avail_w, floats);
        return y;
    }

    // clear
    if b.clear != Clear::None {
        let need = match b.clear {
            Clear::Left => floats.lefts.iter().map(|f| f.2).fold(y, f32::max),
            Clear::Right => floats.rights.iter().map(|f| f.2).fold(y, f32::max),
            _ => floats
                .lefts
                .iter()
                .chain(&floats.rights)
                .map(|f| f.2)
                .fold(y, f32::max),
        };
        y = y.max(need);
    }

    // horizontal solve
    let mut spec_w = len(st.as_ref(), "width", avail_w);
    let mut rep_h = 0.0f32;
    if b.kind == Kind::Replaced {
        let (rw, rh) = replaced_size(lc, b, avail_w);
        if spec_w.is_none() {
            spec_w = Some(rw);
        }
        rep_h = rh;
    }
    let ml_auto = is_auto(st.as_ref(), "margin-left");
    let mr_auto = is_auto(st.as_ref(), "margin-right");
    let extras = b.padding.horiz() + b.border.horiz();
    let border_box = st
        .as_ref()
        .and_then(|s| s.ident("box-sizing"))
        .map(|s| s == "border-box")
        .unwrap_or(false);

    let base_l = if is_bfc_root(b) {
        floats.left_at(y, 10.0, cx)
    } else {
        cx
    };
    let base_r = if is_bfc_root(b) {
        floats.right_at(y, 10.0, cx + avail_w)
    } else {
        cx + avail_w
    };

    let content_w = match spec_w {
        Some(sw) if border_box => (sw - extras).max(0.0),
        Some(sw) => sw,
        None => (base_r - base_l - b.margin.horiz() - extras).max(0.0),
    };
    let mut border_w = content_w + extras;

    let mut x = base_l + b.margin.left;
    if ml_auto && mr_auto && spec_w.is_some() {
        x = base_l + ((base_r - base_l) - border_w).max(0.0) / 2.0;
        b.margin.left = x - base_l;
        b.margin.right = base_r - x - border_w;
    } else if mr_auto {
        b.margin.right = base_r - x - border_w - b.margin.left;
    } else if ml_auto {
        x = base_r - border_w - b.margin.right;
        b.margin.left = x - base_l;
    }
    if let Some(mw) = len(st.as_ref(), "min-width", avail_w) {
        border_w = border_w.max(mw + if border_box { 0.0 } else { extras });
    }
    if let Some(mw) = len(st.as_ref(), "max-width", avail_w) {
        border_w = border_w.min(mw + if border_box { 0.0 } else { extras });
    }
    b.rect.x = x;
    b.rect.y = y;
    b.rect.w = border_w;

    let inner_x = x + b.border.left + b.padding.left;
    let inner_y = y + b.border.top + b.padding.top;
    let inner_w = (border_w - extras).max(0.0);

    match b.kind {
        Kind::Table => {
            crate::table::layout_table(lc, b, inner_x, inner_y, inner_w);
        }
        Kind::Replaced => {
            let mut h = rep_h;
            if len(st.as_ref(), "height", lc.vh).is_none() && rep_h == 0.0 {
                h = rep_h;
            }
            if let Some(eh) = len(st.as_ref(), "height", lc.vh) {
                h = eh;
            }
            b.rect.h = h + b.padding.vert() + b.border.vert();
        }
        _ => {
            let new_bfc = is_bfc_root(b);
            let mut local_floats = Floats::new();
            let fref = if new_bfc { &mut local_floats } else { floats };
            let ncb = if is_positioned(b) {
                Rect {
                    x: b.rect.x + b.border.left,
                    y: b.rect.y + b.border.top,
                    w: inner_w + b.padding.horiz(),
                    h: len(st.as_ref(), "height", lc.vh)
                        .map(|height| {
                            height
                                + if border_box {
                                    -b.border.vert()
                                } else {
                                    b.padding.vert()
                                }
                        })
                        .unwrap_or(lc.vh),
                }
            } else {
                cb
            };
            let grid = st.as_ref().is_some_and(|style| {
                matches!(
                    style.display(),
                    sol_css::Display::Grid | sol_css::Display::InlineGrid
                )
            });
            let content_bottom = if grid {
                crate::grid::layout_grid(lc, b, inner_x, inner_y, inner_w)
            } else if st
                .as_ref()
                .map(|s| {
                    matches!(
                        s.display(),
                        sol_css::Display::Flex | sol_css::Display::InlineFlex
                    )
                })
                .unwrap_or(false)
            {
                crate::flex::layout_flex(lc, b, inner_x, inner_y, inner_w)
            } else {
                lay_children(lc, b, inner_x, inner_y, inner_w, fref, ncb)
            };
            let mut h = content_bottom - inner_y;
            if st
                .as_ref()
                .is_some_and(|style| style.ident("container-type").as_deref() == Some("size"))
            {
                h = 0.0;
            }
            if new_bfc {
                h = h.max(local_floats.bottom() - inner_y);
            }
            if let Some(eh) = len(st.as_ref(), "height", lc.vh) {
                h = eh
                    - if border_box {
                        b.padding.vert() + b.border.vert()
                    } else {
                        0.0
                    };
            }
            if let Some(mh) = len(st.as_ref(), "min-height", lc.vh) {
                h = h.max(mh);
            }
            if let Some(mh) = len(st.as_ref(), "max-height", lc.vh) {
                h = h.min(mh);
            }
            b.rect.h = h.max(0.0) + b.padding.vert() + b.border.vert();
        }
    }

    if pos == Position::Relative {
        let dy = len(st.as_ref(), "top", avail_w)
            .or_else(|| len(st.as_ref(), "bottom", avail_w).map(|v| -v))
            .unwrap_or(0.0);
        let dx = len(st.as_ref(), "left", avail_w)
            .or_else(|| len(st.as_ref(), "right", avail_w).map(|v| -v))
            .unwrap_or(0.0);
        shift(b, dx, dy);
    }
    b.rect.bottom() + b.margin.bottom
}

fn position_absolute(lc: &mut LC, b: &mut LBox, cb: Rect) {
    let st = b.style.clone();
    b.padding = edges(st.as_ref(), "padding", cb.w);
    b.border = border_edges(st.as_ref(), cb.w);
    b.margin = edges(st.as_ref(), "margin", cb.w);
    let spec_w = len(st.as_ref(), "width", cb.w);
    let left = len(st.as_ref(), "left", cb.w);
    let right = len(st.as_ref(), "right", cb.w);
    let top = len(st.as_ref(), "top", cb.h);
    let bottom = len(st.as_ref(), "bottom", cb.h);
    let extras_h = b.border.horiz() + b.padding.horiz();
    let w = match (spec_w, left, right) {
        (None, Some(l), Some(r)) => (cb.w - l - r - b.margin.horiz() - extras_h).max(0.0),
        (Some(w), _, _) => w,
        _ => (cb.w - left.unwrap_or(0.0) - right.unwrap_or(0.0) - extras_h).max(0.0),
    };
    let border_w = w + extras_h;
    let x = cb.x
        + match (left, right, spec_w) {
            (Some(l), _, _) => l + b.margin.left,
            (None, Some(r), _) => cb.w - r - border_w - b.margin.right,
            _ => 0.0,
        };
    b.rect.x = x;
    b.rect.w = border_w;
    let inner_x = x + b.border.left + b.padding.left;
    let inner_w = (border_w - extras_h).max(0.0);
    let mut floats = Floats::new();
    let y0 = cb.y + top.unwrap_or(0.0) + b.margin.top;
    let inner_y = y0 + b.border.top + b.padding.top;
    b.rect.y = y0;
    let self_cb = Rect {
        x: inner_x - b.padding.left,
        y: inner_y - b.padding.top,
        w: inner_w + b.padding.horiz(),
        h: lc.vh,
    };
    let content_bottom = match st.as_ref().map(|s| s.display()) {
        Some(sol_css::Display::Grid | sol_css::Display::InlineGrid) => {
            crate::grid::layout_grid(lc, b, inner_x, inner_y, inner_w)
        }
        Some(sol_css::Display::Flex | sol_css::Display::InlineFlex) => {
            crate::flex::layout_flex(lc, b, inner_x, inner_y, inner_w)
        }
        _ => lay_children(lc, b, inner_x, inner_y, inner_w, &mut floats, self_cb),
    };
    let mut h = content_bottom - inner_y;
    h = h.max(floats.bottom() - inner_y);
    if let Some(eh) = len(st.as_ref(), "height", cb.h) {
        h = eh;
    }
    b.rect.h = h + b.padding.vert() + b.border.vert();
    if top.is_none() {
        if let Some(bm) = bottom {
            let ny = cb.y + cb.h - bm - b.rect.h - b.margin.bottom;
            shift(b, 0.0, ny - b.rect.y);
        }
    }
}

pub fn shift_pub(b: &mut LBox, dx: f32, dy: f32) {
    shift(b, dx, dy);
}

fn shift(b: &mut LBox, dx: f32, dy: f32) {
    b.rect.x += dx;
    b.rect.y += dy;
    for k in &mut b.kids {
        shift(k, dx, dy);
    }
}

fn lay_float(lc: &mut LC, b: &mut LBox, cx: f32, y: f32, avail_w: f32, floats: &mut Floats) {
    let st = b.style.clone();
    let spec_w = len(st.as_ref(), "width", avail_w);
    let extras = b.border.horiz() + b.padding.horiz();
    let rem = (floats.right_at(y, 10.0, cx + avail_w) - floats.left_at(y, 10.0, cx)).max(0.0);
    let cw = spec_w.unwrap_or(rem).min(rem) - b.margin.horiz();
    let cw = cw.max(0.0);
    let x = match b.float {
        Float::Left => floats.left_at(y, 10.0, cx) + b.margin.left,
        Float::Right => floats.right_at(y, 10.0, cx + avail_w) - (cw + extras) - b.margin.right,
        Float::None => cx + b.margin.left,
    };
    b.rect = Rect {
        x,
        y: y + b.margin.top,
        w: cw + extras,
        h: 0.0,
    };
    let inner_x = x + b.border.left + b.padding.left;
    let inner_y = b.rect.y + b.border.top + b.padding.top;
    let inner_w = (b.rect.w - extras).max(0.0);
    let mut local = Floats::new();
    let ncb = if is_positioned(b) {
        b.padding_rect()
    } else {
        Rect {
            x: 0.0,
            y: 0.0,
            w: lc.vw,
            h: lc.vh,
        }
    };
    let content_bottom = lay_children(lc, b, inner_x, inner_y, inner_w, &mut local, ncb);
    let mut h = content_bottom - inner_y;
    h = h.max(local.bottom() - inner_y);
    if let Some(eh) = len(st.as_ref(), "height", lc.vh) {
        h = eh;
    }
    b.rect.h = h + b.padding.vert() + b.border.vert();
    floats.add(
        b.float,
        b.rect.x - b.margin.left,
        b.rect.y - b.margin.top,
        b.rect.w + b.margin.horiz(),
        b.rect.h + b.margin.vert(),
    );
}

/// Lay out children of `b` inside content box (x,y,w).
pub fn lay_children(
    lc: &mut LC,
    b: &mut LBox,
    x: f32,
    y: f32,
    w: f32,
    floats: &mut Floats,
    cb: Rect,
) -> f32 {
    let mut cur_y = y;
    let mut prev_margin: Option<f32> = None;
    let kids = std::mem::take(&mut b.kids);
    let mut new_kids = Vec::new();
    let mut inline_run: Vec<LBox> = Vec::new();
    let parent_st = b.style.clone();

    macro_rules! flush_ifc {
        () => {
            if !inline_run.is_empty() {
                let mut ifc = LBox::new(Kind::AnonymousBlock);
                ifc.rect = Rect {
                    x,
                    y: cur_y,
                    w,
                    h: 0.0,
                };
                let items = std::mem::take(&mut inline_run);
                cur_y = layout_ifc(
                    lc,
                    &mut ifc,
                    items,
                    (x, cur_y),
                    w,
                    floats,
                    parent_st.as_ref(),
                );
                prev_margin = Some(0.0);
                new_kids.push(ifc);
            }
        };
    }

    for mut k in kids {
        if is_inline_kind(&k) {
            inline_run.push(k);
            continue;
        }
        flush_ifc!();
        match k.kind {
            Kind::AnonymousBlock => {
                k.rect = Rect {
                    x,
                    y: cur_y,
                    w,
                    h: 0.0,
                };
                let items = std::mem::take(&mut k.kids);
                cur_y = layout_ifc(lc, &mut k, items, (x, cur_y), w, floats, parent_st.as_ref());
                prev_margin = Some(0.0);
                new_kids.push(k);
            }
            _ => {
                cur_y = lay_block(lc, &mut k, (x, cur_y), w, floats, &mut prev_margin, cb);
                new_kids.push(k);
            }
        }
    }
    flush_ifc!();
    let _ = prev_margin;
    b.kids = new_kids;
    cur_y
}

fn is_inline_kind(b: &LBox) -> bool {
    matches!(
        b.kind,
        Kind::Inline | Kind::InlineBlock | Kind::Text | Kind::Replaced | Kind::Marker
    ) && b.float == Float::None
        && b.style
            .as_ref()
            .is_none_or(|s| !matches!(s.position(), Position::Absolute | Position::Fixed))
}

// ---------- inline formatting ----------

/// One measurable unit on a line. `space_w` = leading whitespace width.
struct Item {
    w: f32,
    h: f32,
    baseline: f32,
    space_w: f32,
    is_br: bool,
    valign: f32,
    make: Make,
}

enum Make {
    Word {
        text: String,
        face_idx: usize,
        style: Option<Rc<Style>>,
        node: Option<NodeId>,
    },
    Existing(usize),
    /// inline-box span marker: (box_idx, is_end)
    Marker(usize, bool),
}

fn line_height_of(st: Option<&Rc<Style>>, face: &sol_text::Face, cw: f32) -> f32 {
    let lh = st
        .and_then(|s| s.get("line-height"))
        .and_then(|v| v.first());
    match lh {
        Some(Value::Number(n)) => (*n * face.px as f64) as f32,
        Some(v @ (Value::Length(_, _) | Value::Percentage(_))) => {
            sol_css::style::value_px(v, face.px as f64, cw as f64)
                .unwrap_or(face.line_height() as f64) as f32
        }
        _ => face.line_height(),
    }
}

fn valign_of(st: Option<&Rc<Style>>, face: &sol_text::Face) -> f32 {
    let v = st
        .and_then(|s| s.get("vertical-align"))
        .and_then(|v| v.first());
    match v {
        Some(v @ (Value::Length(_, _) | Value::Percentage(_))) => {
            sol_css::style::value_px(v, face.px as f64, 0.0).unwrap_or(0.0) as f32
        }
        Some(v2) => match idlike(v2) {
            Some("sub") => -face.px * 0.2,
            Some("super") => face.px * 0.3,
            _ => 0.0,
        },
        _ => 0.0,
    }
}

/// Split text into (word, has_leading_space, is_newline_break) pieces.
fn split_words(text: &str, pre: bool) -> Vec<(String, bool, bool)> {
    let mut out = Vec::new();
    if pre {
        let mut cur = String::new();
        let mut leading = true;
        for c in text.chars() {
            if c == '\n' {
                out.push((std::mem::take(&mut cur), leading, true));
                leading = false;
            } else {
                cur.push(c);
            }
        }
        if !cur.is_empty() {
            out.push((cur, leading, false));
        }
        return out;
    }
    let mut cur = String::new();
    let mut pending_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !cur.is_empty() {
                out.push((std::mem::take(&mut cur), pending_space, false));
            }
            pending_space = true;
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push((cur, pending_space, false));
    }
    out
}

/// Emit items for a text run. Returns whether the run ended in
/// whitespace (so the next item gets a leading space).
fn emit_text(
    items: &mut Vec<Item>,
    source: &LBox,
    face: &sol_text::Face,
    w: f32,
    pre: bool,
    leading_ws: bool,
) {
    let run = source.text.as_ref().unwrap();
    let text = &run.text;
    let face_idx = run.face_idx;
    let style = &source.style;
    let node = source.node;
    let lh = line_height_of(style.as_ref(), face, w);
    let va = valign_of(style.as_ref(), face);
    let space_px = face.measure(" ");
    let mut first = true;
    for (word, had_space, is_nl) in split_words(text, pre) {
        if is_nl {
            if !word.is_empty() {
                let m = face.measure(&word);
                items.push(Item {
                    w: m,
                    h: lh,
                    baseline: face.ascent() + (lh - face.line_height()) / 2.0,
                    space_w: 0.0,
                    is_br: false,
                    valign: va,
                    make: Make::Word {
                        text: word,
                        face_idx,
                        style: style.clone(),
                        node,
                    },
                });
            }
            items.push(Item {
                w: 0.0,
                h: 0.0,
                baseline: 0.0,
                space_w: 0.0,
                is_br: true,
                valign: 0.0,
                make: Make::Word {
                    text: String::new(),
                    face_idx,
                    style: style.clone(),
                    node,
                },
            });
            first = false;
            continue;
        }
        if word.is_empty() {
            continue;
        }
        let m = face.measure(&word);
        let sw = if first { leading_ws } else { had_space };
        items.push(Item {
            w: m,
            h: lh,
            baseline: face.ascent() + (lh - face.line_height()) / 2.0,
            space_w: if sw { space_px } else { 0.0 },
            is_br: false,
            valign: va,
            make: Make::Word {
                text: word,
                face_idx,
                style: style.clone(),
                node,
            },
        });
        first = false;
    }
}

fn layout_ifc(
    lc: &mut LC,
    ifc: &mut LBox,
    items_src: Vec<LBox>,
    origin: (f32, f32),
    w: f32,
    floats: &mut Floats,
    parent_st: Option<&Rc<Style>>,
) -> f32 {
    let (x, y) = origin;
    let mut items: Vec<Item> = Vec::new();
    let mut boxes: Vec<LBox> = Vec::new();
    let pre = parent_st
        .and_then(|s| s.ident("white-space"))
        .map(|ws| matches!(ws.as_str(), "pre" | "pre-wrap" | "break-spaces"))
        .unwrap_or(false);
    let nowrap = parent_st
        .and_then(|s| s.ident("white-space"))
        .map(|ws| ws == "nowrap")
        .unwrap_or(false);

    // pending_space = previous source ended with whitespace
    let mut pending_space = false;
    for mut b in items_src {
        match b.kind {
            Kind::Text | Kind::Marker => {
                let tr = b.text.clone().unwrap();
                let face = lc.faces.get(tr.face_idx).clone();
                let starts_ws = tr
                    .text
                    .chars()
                    .next()
                    .map(|c| c.is_whitespace())
                    .unwrap_or(false);
                let before = items.len();
                emit_text(&mut items, &b, &face, w, pre, pending_space || starts_ws);
                // mark first new item's leading space correctly
                if items.len() > before && (pending_space || starts_ws) {
                    items[before].space_w = items[before].space_w.max(face.measure(" "));
                }
                pending_space = tr
                    .text
                    .chars()
                    .last()
                    .map(|c| c.is_whitespace())
                    .unwrap_or(false)
                    || items.len() == before && starts_ws;
            }
            Kind::Inline => {
                let bi = boxes.len();
                items.push(Item {
                    w: 0.0,
                    h: 0.0,
                    baseline: 0.0,
                    space_w: if pending_space { 8.0 } else { 0.0 },
                    is_br: false,
                    valign: 0.0,
                    make: Make::Marker(bi, false),
                });
                let mut inner_pending = pending_space;
                let kids = std::mem::take(&mut b.kids);
                for k in kids {
                    inner_pending = emit_flat(lc, k, &mut items, &mut boxes, w, inner_pending);
                }
                pending_space = inner_pending;
                items.push(Item {
                    w: 0.0,
                    h: 0.0,
                    baseline: 0.0,
                    space_w: 0.0,
                    is_br: false,
                    valign: 0.0,
                    make: Make::Marker(bi, true),
                });
                boxes.push(b);
            }
            Kind::InlineBlock | Kind::Replaced => {
                let st2 = b.style.clone();
                b.padding = edges(st2.as_ref(), "padding", w);
                b.border = border_edges(st2.as_ref(), w);
                b.margin = edges(st2.as_ref(), "margin", w);
                let is_br = b.node.map(|n| lc.doc.is_tag(n, "br")).unwrap_or(false);
                let bi = boxes.len();
                if b.kind == Kind::InlineBlock {
                    let spec_w = len(st2.as_ref(), "width", w);
                    let extras = b.border.horiz() + b.padding.horiz();
                    let cw = spec_w.unwrap_or(w - extras).max(0.0);
                    let mut local = Floats::new();
                    let inner_w = (cw - extras).max(0.0);
                    b.rect = Rect {
                        x: 0.0,
                        y: 0.0,
                        w: cw,
                        h: 0.0,
                    };
                    let cb = Rect {
                        x: 0.0,
                        y: 0.0,
                        w: inner_w,
                        h: lc.vh,
                    };
                    let inner_y = b.border.top + b.padding.top;
                    let inner_x = b.border.left + b.padding.left;
                    let content_bottom = match st2.as_ref().map(|s| s.display()) {
                        Some(sol_css::Display::Grid | sol_css::Display::InlineGrid) => {
                            crate::grid::layout_grid(lc, &mut b, inner_x, inner_y, inner_w)
                        }
                        Some(sol_css::Display::Flex | sol_css::Display::InlineFlex) => {
                            crate::flex::layout_flex(lc, &mut b, inner_x, inner_y, inner_w)
                        }
                        _ => lay_children(lc, &mut b, inner_x, inner_y, inner_w, &mut local, cb),
                    };
                    let mut h = content_bottom - inner_y;
                    h = h.max(local.bottom() - inner_y);
                    if let Some(eh) = len(st2.as_ref(), "height", lc.vh) {
                        h = eh;
                    }
                    b.rect.h = h + b.padding.vert() + b.border.vert();
                } else {
                    let (rw, rh) = replaced_size(lc, &b, w);
                    b.rect.w = rw + b.border.horiz() + b.padding.horiz();
                    b.rect.h = rh + b.border.vert() + b.padding.vert();
                }
                if is_br {
                    items.push(Item {
                        w: 0.0,
                        h: 0.0,
                        baseline: 0.0,
                        space_w: 0.0,
                        is_br: true,
                        valign: 0.0,
                        make: Make::Existing(bi),
                    });
                } else {
                    let va = valign_of(st2.as_ref(), &mk_face(lc, st2.as_deref()));
                    items.push(Item {
                        w: b.rect.w + b.margin.horiz(),
                        h: b.rect.h.max(1.0),
                        baseline: b.rect.h,
                        space_w: if pending_space { 8.0 } else { 0.0 },
                        is_br: false,
                        valign: va,
                        make: Make::Existing(bi),
                    });
                }
                boxes.push(b);
                pending_space = false;
            }
            _ => {}
        }
    }

    if items.is_empty() {
        ifc.rect.h = 0.0;
        return y;
    }

    // line breaking
    let mut cy = y;
    let align = parent_st
        .and_then(|s| s.ident("text-align"))
        .unwrap_or_default();
    let mut i = 0usize;
    let n = items.len();
    let mut open_markers: Vec<(usize, f32)> = Vec::new();
    loop {
        let mut lx = floats.left_at(cy, 1.0, x);
        let mut rx = floats.right_at(cy, 1.0, x + w);
        if rx - lx < 1.0 && floats.bottom() > cy {
            cy = floats.bottom();
            lx = floats.left_at(cy, 1.0, x);
            rx = floats.right_at(cy, 1.0, x + w);
        }
        let mut line_items: Vec<usize> = Vec::new();
        let mut line_w = 0.0f32;
        while i < n {
            let it = &items[i];
            if it.is_br {
                i += 1;
                break;
            }
            let sep = if line_items.is_empty() {
                0.0
            } else {
                it.space_w
            };
            if line_w + sep + it.w > rx - lx + 0.01 && !line_items.is_empty() && !nowrap {
                break;
            }
            line_items.push(i);
            line_w += sep + it.w;
            i += 1;
        }
        let mut line_h = 0.0f32;
        let mut max_base = 0.0f32;
        for &li in &line_items {
            let it = &items[li];
            line_h = line_h.max(it.h);
            max_base = max_base.max(it.baseline);
        }
        if line_h == 0.0 {
            let face = mk_face(lc, parent_st.map(|r| r.as_ref()));
            line_h = line_height_of(parent_st, &face, w);
            max_base = face.ascent();
        }
        let total: f32 = line_items
            .iter()
            .enumerate()
            .map(|(k, &li)| items[li].w + if k == 0 { 0.0 } else { items[li].space_w })
            .sum();
        let mut pen = match align.as_str() {
            "right" => rx - total,
            "end" if parent_st.and_then(|s| s.ident("direction")).as_deref() == Some("rtl") => lx,
            "end" => rx - total,
            "" | "start"
                if parent_st.and_then(|s| s.ident("direction")).as_deref() == Some("rtl") =>
            {
                rx - total
            }
            "center" => lx + ((rx - lx - total) / 2.0).max(0.0),
            _ => lx,
        };
        let mut line = LBox::new(Kind::Line);
        line.rect = Rect {
            x,
            y: cy,
            w,
            h: line_h,
        };
        let mut spans: Vec<LBox> = Vec::new();
        let mut first = true;
        for &li in &line_items {
            let it = &items[li];
            if !first {
                pen += it.space_w;
            }
            first = false;
            let iy = cy + max_base - it.baseline - it.valign;
            match &it.make {
                Make::Word {
                    text,
                    face_idx,
                    style,
                    node,
                } => {
                    let mut tb = LBox::new(Kind::Text);
                    tb.node = *node;
                    tb.style = style.clone();
                    tb.rect = Rect {
                        x: pen,
                        y: iy,
                        w: it.w,
                        h: it.h,
                    };
                    tb.baseline = it.baseline;
                    tb.text = Some(TextRun {
                        text: text.clone(),
                        face_idx: *face_idx,
                    });
                    line.kids.push(tb);
                }
                Make::Existing(bi) => {
                    let mut sb = std::mem::replace(&mut boxes[*bi], LBox::new(Kind::Block));
                    let dx = pen + sb.margin.left - sb.rect.x;
                    let dy = iy - sb.rect.y;
                    shift(&mut sb, dx, dy);
                    sb.baseline = it.baseline;
                    line.kids.push(sb);
                }
                Make::Marker(bi, is_end) => {
                    if !*is_end {
                        open_markers.push((*bi, pen));
                    } else if let Some((bi2, sx)) = open_markers.pop() {
                        let b2 = &boxes[bi2];
                        let mut span = LBox::new(Kind::Inline);
                        span.node = b2.node;
                        span.style = b2.style.clone();
                        span.rect = Rect {
                            x: sx,
                            y: cy,
                            w: pen - sx,
                            h: line_h,
                        };
                        spans.push(span);
                    }
                }
            }
            pen += it.w;
        }
        // still-open markers get a span to the line end, stay open
        for &(bi2, sx) in &open_markers {
            let b2 = &boxes[bi2];
            let mut span = LBox::new(Kind::Inline);
            span.node = b2.node;
            span.style = b2.style.clone();
            span.rect = Rect {
                x: sx,
                y: cy,
                w: pen - sx,
                h: line_h,
            };
            spans.push(span);
        }
        // re-open markers that continue onto the next line
        if i < n {
            open_markers = open_markers.iter().map(|&(bi2, _)| (bi2, lx)).collect();
        } else {
            open_markers.clear();
        }
        // Reorder measured words and whitespace as visual atoms. Keep each
        // word logical so shaping can resolve joining, ligatures, and marks.
        let direction = match parent_st.and_then(|s| s.ident("direction")).as_deref() {
            Some("rtl") => sol_text::Direction::Rtl,
            Some("ltr") => sol_text::Direction::Ltr,
            _ => sol_text::Direction::Auto,
        };
        let mut atoms: Vec<(Option<usize>, String, f32)> = Vec::new();
        let mut previous_right: Option<f32> = None;
        for (index, kid) in line.kids.iter().enumerate() {
            if let Some(right) = previous_right {
                atoms.push((None, " ".into(), (kid.rect.x - right).max(0.0)));
            }
            atoms.push((
                Some(index),
                kid.text
                    .as_ref()
                    .map(|t| t.text.clone())
                    .unwrap_or_else(|| "\u{fffc}".into()),
                kid.rect.w,
            ));
            previous_right = Some(kid.rect.right());
        }
        let labels: Vec<_> = atoms.iter().map(|(_, text, _)| text.as_str()).collect();
        let order = sol_text::visual_order(&labels, direction);
        let mut visual_pen = line.kids.first().map(|kid| kid.rect.x).unwrap_or(lx);
        for index in order {
            let (child, _, width) = &atoms[index];
            if let Some(child) = child {
                let kid = &mut line.kids[*child];
                let dx = visual_pen - kid.rect.x;
                shift(kid, dx, 0.0);
            }
            visual_pen += width;
        }
        for span in &mut spans {
            if let Some(node) = span.node {
                let descendants: Vec<_> = line
                    .kids
                    .iter()
                    .filter(|kid| {
                        let mut current = kid.node;
                        while let Some(n) = current {
                            if n == node {
                                return true;
                            }
                            current = lc.doc.parent(n);
                        }
                        false
                    })
                    .collect();
                if !descendants.is_empty() {
                    let left = descendants
                        .iter()
                        .map(|kid| kid.rect.x)
                        .fold(f32::INFINITY, f32::min);
                    let right = descendants
                        .iter()
                        .map(|kid| kid.rect.right())
                        .fold(f32::NEG_INFINITY, f32::max);
                    span.rect.x = left;
                    span.rect.w = right - left;
                }
            }
        }
        for s in spans {
            line.kids.insert(0, s);
        }
        ifc.kids.push(line);
        cy += line_h;
        if i >= n {
            break;
        }
    }
    ifc.rect.h = cy - y;
    cy
}

/// Flatten nested inline content into items; returns trailing-ws flag.
fn emit_flat(
    lc: &mut LC,
    mut b: LBox,
    items: &mut Vec<Item>,
    boxes: &mut Vec<LBox>,
    w: f32,
    mut pending_space: bool,
) -> bool {
    match b.kind {
        Kind::Text | Kind::Marker => {
            let tr = b.text.clone().unwrap();
            let face = lc.faces.get(tr.face_idx).clone();
            let starts_ws = tr
                .text
                .chars()
                .next()
                .map(|c| c.is_whitespace())
                .unwrap_or(false);
            emit_text(items, &b, &face, w, false, pending_space || starts_ws);
            pending_space = tr
                .text
                .chars()
                .last()
                .map(|c| c.is_whitespace())
                .unwrap_or(false);
        }
        Kind::Inline => {
            let kids = std::mem::take(&mut b.kids);
            for k in kids {
                pending_space = emit_flat(lc, k, items, boxes, w, pending_space);
            }
        }
        _ => {
            pending_space = false;
        }
    }
    let _ = boxes;
    pending_space
}

// ---------- replaced elements ----------

pub fn replaced_size(lc: &mut LC, b: &LBox, avail_w: f32) -> (f32, f32) {
    let st = b.style.clone();
    let (nw, nh) = b
        .node
        .and_then(|n| lc.input.image_sizes.get(&n).copied())
        .unwrap_or((0, 0));
    let tag = b
        .node
        .map(|n| lc.doc.tag(n).to_string())
        .unwrap_or_default();
    let (mut w, mut h) = match tag.as_str() {
        "img" => (nw.max(1) as f32, nh.max(1) as f32),
        "hr" => (avail_w, 1.0),
        "input" => {
            let ty = b
                .node
                .and_then(|n| lc.doc.attr(n, "type"))
                .unwrap_or("text")
                .to_ascii_lowercase();
            match ty.as_str() {
                "checkbox" | "radio" => (13.0, 13.0),
                "submit" | "button" | "reset" => {
                    let label = b
                        .node
                        .and_then(|n| lc.doc.attr(n, "value"))
                        .unwrap_or("Submit")
                        .to_string();
                    let face = mk_face(lc, st.as_deref());
                    (face.measure(&label) + 16.0, 22.0)
                }
                "hidden" => (0.0, 0.0),
                _ => (170.0, 22.0),
            }
        }
        "button" => {
            let face = mk_face(lc, st.as_deref());
            let txt = b.node.map(|n| lc.doc.text_content(n)).unwrap_or_default();
            (face.measure(&txt) + 20.0, 24.0)
        }
        "select" => (140.0, 24.0),
        "textarea" => (170.0, 60.0),
        "iframe" => (300.0, 150.0),
        "canvas" | "video" => (300.0, 150.0),
        "audio" => (300.0, 30.0),
        "embed" | "object" => (300.0, 200.0),
        _ => (0.0, 0.0),
    };
    let cw = len(st.as_ref(), "width", avail_w);
    let ch = len(st.as_ref(), "height", lc.vh);
    match (cw, ch) {
        (Some(w2), Some(h2)) => {
            w = w2;
            h = h2;
        }
        (Some(w2), None) => {
            if nw > 0 && w > 0.0 {
                h = w2 * (h / w);
            }
            w = w2;
        }
        (None, Some(h2)) => {
            if nw > 0 && h > 0.0 {
                w = h2 * (w / h);
            }
            h = h2;
        }
        _ => {}
    }
    (w, h)
}
