//! Build the layout box tree from DOM + computed styles.
//!
//! - element → box(es) per display
//! - display:none prunes; display:contents passes children through
//! - text nodes → Text boxes under inline/block parents
//! - table structure repair (missing tbody/row/cell wrappers)
//! - anonymous block boxing for mixed inline+block content

use crate::{FaceSet, Kind, LBox, LayoutInput, TextRun};
use sol_css::{Display, Value};
use sol_dom::{Document, NodeData, NodeId};
use std::rc::Rc;

pub struct Ctx<'a> {
    pub doc: &'a Document,
    pub input: &'a LayoutInput<'a>,
    pub faces: &'a mut FaceSet,
    pub(crate) counters: crate::generated::Counters,
}

pub fn build(ctx: &mut Ctx) -> LBox {
    let mut root = LBox::new(Kind::Root);
    let mut pending_inline: Vec<LBox> = Vec::new();
    for &c in ctx.doc.children(ctx.doc.root) {
        build_node(ctx, c, &mut pending_inline, &mut root);
    }
    flush_inline(&mut pending_inline, &mut root);
    root
}

fn flush_inline(pending: &mut Vec<LBox>, parent: &mut LBox) {
    if pending.is_empty() {
        return;
    }
    // wrap runs of inline boxes in an anonymous block
    let mut wrap = LBox::new(Kind::AnonymousBlock);
    wrap.kids = std::mem::take(pending);
    parent.kids.push(wrap);
}

fn is_inlineish(b: &LBox) -> bool {
    matches!(
        b.kind,
        Kind::Inline | Kind::InlineBlock | Kind::Text | Kind::Replaced
    ) && b.float == sol_css::Float::None
}

fn build_node(ctx: &mut Ctx, n: NodeId, pending: &mut Vec<LBox>, parent: &mut LBox) {
    if let Some(style) = ctx
        .input
        .styles
        .get(&n)
        .cloned()
        .filter(|style| style.display() != Display::None)
    {
        crate::generated::enter(ctx, n, &style, false);
    }
    build_node_inner(ctx, n, pending, parent);
    crate::generated::leave(ctx, n);
}
fn build_node_inner(ctx: &mut Ctx, n: NodeId, pending_inline: &mut Vec<LBox>, parent: &mut LBox) {
    let node = ctx.doc.node(n);
    match &node.data {
        NodeData::Text(t) => {
            let t2 = t.clone();
            let st = nearest_style(ctx, n);
            let face = make_face(ctx.input, st.as_deref());
            let face_idx = ctx.faces.push(face);
            let mut b = LBox::new(Kind::Text);
            b.node = Some(n);
            b.style = st;
            b.text = Some(TextRun { text: t2, face_idx });
            pending_inline.push(b);
        }
        NodeData::Element(e) => {
            let tag = e.tag.clone();
            let Some(style) = crate::style_of(ctx.input, n).cloned() else {
                return;
            };
            let disp = style.display();
            match disp {
                Display::None => return,
                Display::Contents => {
                    let mut inner_pend: Vec<LBox> = Vec::new();
                    for &c in ctx.doc.children(n) {
                        build_node(ctx, c, &mut inner_pend, parent);
                    }
                    pending_inline.extend(inner_pend);
                    return;
                }
                _ => {}
            }
            // replaced elements
            if is_replaced(&tag) {
                let mut b = LBox::new(Kind::Replaced);
                b.node = Some(n);
                b.style = Some(style.clone());
                b.float = style.float();
                b.clear = style.clear();
                if is_inlineish(&b) {
                    pending_inline.push(b);
                } else {
                    flush_inline(pending_inline, parent);
                    parent.kids.push(b);
                }
                return;
            }
            match disp {
                Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid => {
                    let inline = matches!(disp, Display::InlineFlex | Display::InlineGrid);
                    if !inline {
                        flush_inline(pending_inline, parent);
                    }
                    let mut b = LBox::new(if inline {
                        Kind::InlineBlock
                    } else {
                        Kind::Block
                    });
                    b.node = Some(n);
                    b.style = Some(style.clone());
                    if let Some(mut before) = crate::generated::box_for(ctx, n, &style, "before") {
                        before.kind = Kind::Block;
                        b.kids.push(before);
                    }
                    // Flex items (CSS-FLEXBOX §4): each element child is its
                    // own item — inline-level boxes are blockified, never
                    // co-wrapped; each contiguous text run gets one anonymous
                    // item; whitespace-only runs are dropped entirely.
                    fn flush_run(b: &mut LBox, run: &mut Vec<LBox>) {
                        if !run.is_empty() {
                            let mut wrap = LBox::new(Kind::AnonymousBlock);
                            wrap.kids = std::mem::take(run);
                            b.kids.push(wrap);
                        }
                    }
                    let mut run: Vec<LBox> = Vec::new();
                    for &c in ctx.doc.children(n) {
                        let mut pend: Vec<LBox> = Vec::new();
                        let before = b.kids.len();
                        build_node(ctx, c, &mut pend, &mut b);
                        if b.kids.len() > before {
                            // a block-level child pushed straight to b.kids —
                            // any text before it forms an anonymous item first
                            let added = b.kids.split_off(before);
                            flush_run(&mut b, &mut run);
                            b.kids.extend(added);
                        }
                        for mut ib in pend.drain(..) {
                            match ib.kind {
                                Kind::Text => {
                                    let blank = ib
                                        .text
                                        .as_ref()
                                        .map(|t| t.text.trim().is_empty())
                                        .unwrap_or(true);
                                    if !blank {
                                        run.push(ib);
                                    }
                                }
                                _ => {
                                    flush_run(&mut b, &mut run);
                                    ib.kind = Kind::Block; // blockify
                                    b.kids.push(ib);
                                }
                            }
                        }
                    }
                    flush_run(&mut b, &mut run);
                    if let Some(mut after) = crate::generated::box_for(ctx, n, &style, "after") {
                        after.kind = Kind::Block;
                        b.kids.push(after);
                    }
                    if inline {
                        pending_inline.push(b);
                    } else {
                        parent.kids.push(b);
                    }
                }
                Display::Inline | Display::InlineBlock => {
                    let kind = if disp == Display::Inline {
                        Kind::Inline
                    } else {
                        Kind::InlineBlock
                    };
                    let mut b = LBox::new(kind);
                    b.node = Some(n);
                    b.style = Some(style.clone());
                    b.float = style.float();
                    // list-item markers handled in block layout
                    let mut inner: Vec<LBox> = Vec::new();
                    if let Some(before) = crate::generated::box_for(ctx, n, &style, "before") {
                        append_generated(before, &mut inner, &mut b);
                    }
                    for &c in ctx.doc.children(n) {
                        build_node(ctx, c, &mut inner, &mut b);
                    }
                    if let Some(after) = crate::generated::box_for(ctx, n, &style, "after") {
                        append_generated(after, &mut inner, &mut b);
                    }
                    flush_inline(&mut inner, &mut b);
                    pending_inline.push(b);
                }
                Display::Table | Display::InlineTable => {
                    flush_inline(pending_inline, parent);
                    let mut b = LBox::new(Kind::Table);
                    b.node = Some(n);
                    b.style = Some(style);
                    build_table_children(ctx, n, &mut b);
                    parent.kids.push(b);
                }
                _ => {
                    // block-level
                    let mut b = LBox::new(Kind::Block);
                    b.node = Some(n);
                    b.style = Some(style.clone());
                    b.float = style.float();
                    b.clear = style.clear();
                    // list-item marker box (generated content)
                    if disp == Display::ListItem {
                        if let Some(mb) = marker_box(ctx, n, &style) {
                            b.kids.push(mb);
                        }
                    }
                    let mut inner: Vec<LBox> = Vec::new();
                    if let Some(before) = crate::generated::box_for(ctx, n, &style, "before") {
                        append_generated(before, &mut inner, &mut b);
                    }
                    for &c in ctx.doc.children(n) {
                        build_node(ctx, c, &mut inner, &mut b);
                    }
                    if let Some(after) = crate::generated::box_for(ctx, n, &style, "after") {
                        append_generated(after, &mut inner, &mut b);
                    }
                    flush_inline(&mut inner, &mut b);
                    flush_inline(pending_inline, parent);
                    parent.kids.push(b);
                }
            }
        }
        _ => {}
    }
}

fn append_generated(box_: LBox, pending: &mut Vec<LBox>, parent: &mut LBox) {
    if is_inlineish(&box_) {
        pending.push(box_);
    } else {
        flush_inline(pending, parent);
        parent.kids.push(box_);
    }
}

fn is_replaced(tag: &str) -> bool {
    matches!(
        tag,
        "img"
            | "input"
            | "select"
            | "textarea"
            | "iframe"
            | "canvas"
            | "video"
            | "audio"
            | "hr"
            | "embed"
            | "object"
            | "button"
            | "br"
    )
}

/// Build the ::marker substitute box for display:list-item elements.
/// Counter = index of this li among list-item element siblings.
fn marker_box(ctx: &mut Ctx, n: NodeId, style: &Rc<sol_css::Style>) -> Option<LBox> {
    if let Some(mut marker) = crate::generated::box_for(ctx, n, style, "marker") {
        let text = marker.kids.pop()?.text?;
        marker.kind = Kind::Marker;
        marker.text = Some(text);
        return Some(marker);
    }
    let lst = style
        .ident("list-style-type")
        .unwrap_or_else(|| "disc".into());
    if lst == "none" {
        return None;
    }
    let ord = ctx
        .doc
        .parent(n)
        .map(|p| {
            ctx.doc
                .children(p)
                .iter()
                .filter(|&&s| {
                    ctx.input
                        .styles
                        .get(&s)
                        .map(|st| st.display() == Display::ListItem)
                        .unwrap_or(false)
                })
                .position(|&s| s == n)
                .map(|i| i + 1)
                .unwrap_or(1)
        })
        .unwrap_or(1);
    let text = match lst.as_str() {
        "decimal" => format!("{ord}."),
        "decimal-leading-zero" => format!("{ord:02}."),
        "lower-alpha" | "lower-latin" => format!("{}.", (b'a' + ((ord - 1) % 26) as u8) as char),
        "upper-alpha" | "upper-latin" => format!("{}.", (b'A' + ((ord - 1) % 26) as u8) as char),
        "circle" => "◦".into(),
        "square" => "▪".into(),
        _ => "•".into(),
    };
    let face = make_face(ctx.input, Some(style));
    let face_idx = ctx.faces.push(face);
    let mut b = LBox::new(Kind::Marker);
    b.node = Some(n);
    b.style = Some(style.clone());
    b.text = Some(TextRun { text, face_idx });
    Some(b)
}

fn nearest_style(ctx: &Ctx, n: NodeId) -> Option<Rc<sol_css::Style>> {
    let mut cur = ctx.doc.parent(n);
    while let Some(p) = cur {
        if let Some(s) = ctx.input.styles.get(&p) {
            return Some(s.clone());
        }
        cur = ctx.doc.parent(p);
    }
    None
}

pub fn make_face(input: &crate::LayoutInput, style: Option<&sol_css::Style>) -> sol_text::Face {
    let default = sol_css::Style::default();
    let st = style.unwrap_or(&default);
    let fam_val = st.get("font-family");
    let fam = fam_val
        .map(|v| {
            v.split(|value| matches!(value,Value::Ident(s) if s==","))
                .map(|group| {
                    group
                        .iter()
                        .filter_map(|value| match value {
                            Value::Ident(s) | Value::Str(s) => Some(s.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "sans-serif".into());
    let px = st
        .length_px("font-size", 16.0, input.viewport_w as f64)
        .unwrap_or(16.0) as f32;
    let bold = matches!(
        st.ident("font-weight").as_deref(),
        Some("bold") | Some("bolder")
    ) || st
        .get("font-weight")
        .and_then(|v| v.first())
        .and_then(|v| match v {
            Value::Number(n) => Some(*n >= 600.0),
            _ => None,
        })
        .unwrap_or(false);
    let italic = matches!(
        st.ident("font-style").as_deref(),
        Some("italic") | Some("oblique")
    );
    input
        .fonts
        .face(&fam, px.max(0.0), bold, italic)
        .with_direction(match st.ident("direction").as_deref() {
            Some("rtl") => sol_text::Direction::Rtl,
            Some("ltr") => sol_text::Direction::Ltr,
            _ => sol_text::Direction::Auto,
        })
}

/// Repair table structure: wrap bare <td> in <tr>, bare <tr> in tbody,
/// hoist caption. Operates on the DOM-derived child list.
fn build_table_children(ctx: &mut Ctx, table: NodeId, out: &mut LBox) {
    let mut pending_rows: Vec<LBox> = Vec::new();
    let mut pending_cells: Vec<LBox> = Vec::new();
    let finish_row = |rows: &mut Vec<LBox>, cells: &mut Vec<LBox>| {
        if !cells.is_empty() {
            let mut row = LBox::new(Kind::TableRow);
            row.kids = std::mem::take(cells);
            rows.push(row);
        }
    };
    for &c in ctx.doc.children(table) {
        let Some(style) = crate::style_of(ctx.input, c).cloned() else {
            continue;
        };
        let disp = style.display();
        let tag = ctx.doc.tag(c).to_string();
        match disp {
            Display::None => {}
            Display::TableCaption => {
                finish_row(&mut pending_rows, &mut pending_cells);
                let mut b = LBox::new(Kind::TableCaption);
                b.node = Some(c);
                b.style = Some(style);
                let mut inner = Vec::new();
                for &cc in ctx.doc.children(c) {
                    build_node(ctx, cc, &mut inner, &mut b);
                }
                flush_inline(&mut inner, &mut b);
                out.kids.push(b);
            }
            Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup => {
                finish_row(&mut pending_rows, &mut pending_cells);
                build_row_group(ctx, c, out);
            }
            Display::TableRow => {
                finish_row(&mut pending_rows, &mut pending_cells);
                let mut row = LBox::new(Kind::TableRow);
                row.node = Some(c);
                row.style = Some(style);
                for &cc in ctx.doc.children(c) {
                    if let Some(cs) = crate::style_of(ctx.input, cc) {
                        if cs.display() == Display::TableCell {
                            let mut cell = LBox::new(Kind::TableCell);
                            cell.node = Some(cc);
                            cell.style = Some(cs.clone());
                            let mut inner = Vec::new();
                            for &k in ctx.doc.children(cc) {
                                build_node(ctx, k, &mut inner, &mut cell);
                            }
                            flush_inline(&mut inner, &mut cell);
                            row.kids.push(cell);
                        }
                    }
                }
                pending_rows.push(row);
            }
            Display::TableCell => {
                let mut cell = LBox::new(Kind::TableCell);
                cell.node = Some(c);
                cell.style = Some(style);
                let mut inner = Vec::new();
                for &cc in ctx.doc.children(c) {
                    build_node(ctx, cc, &mut inner, &mut cell);
                }
                flush_inline(&mut inner, &mut cell);
                pending_cells.push(cell);
            }
            Display::TableCol | Display::TableColGroup => {
                // columns handled in table layout pass
            }
            _ => {
                // non-table content inside table gets wrapped in a cell
                finish_row(&mut pending_rows, &mut pending_cells);
                if tag == "template" {
                    continue;
                }
                let mut cell = LBox::new(Kind::TableCell);
                cell.node = None;
                cell.style = None;
                let mut inner = Vec::new();
                build_node(ctx, c, &mut inner, &mut cell);
                flush_inline(&mut inner, &mut cell);
                pending_cells.push(cell);
            }
        }
    }
    finish_row(&mut pending_rows, &mut pending_cells);
    if !pending_rows.is_empty() {
        // rows get wrapped in an implicit row-group box — represent as
        // TableRowGroup kind via a generic Block? Use TableRow children
        // directly for simplicity: mark them already TableRow.
        out.kids.extend(pending_rows);
    }
}

fn build_row_group(ctx: &mut Ctx, group: NodeId, out: &mut LBox) {
    let mut pending_cells: Vec<LBox> = Vec::new();
    for &r in ctx.doc.children(group) {
        let Some(style) = crate::style_of(ctx.input, r).cloned() else {
            continue;
        };
        match style.display() {
            Display::None => {}
            Display::TableRow => {
                if !pending_cells.is_empty() {
                    let mut row = LBox::new(Kind::TableRow);
                    row.kids = std::mem::take(&mut pending_cells);
                    out.kids.push(row);
                }
                let mut row = LBox::new(Kind::TableRow);
                row.node = Some(r);
                row.style = Some(style);
                for &cc in ctx.doc.children(r) {
                    if let Some(cs) = crate::style_of(ctx.input, cc) {
                        if cs.display() == Display::TableCell {
                            let mut cell = LBox::new(Kind::TableCell);
                            cell.node = Some(cc);
                            cell.style = Some(cs.clone());
                            let mut inner = Vec::new();
                            for &k in ctx.doc.children(cc) {
                                build_node(ctx, k, &mut inner, &mut cell);
                            }
                            flush_inline(&mut inner, &mut cell);
                            row.kids.push(cell);
                        }
                    }
                }
                out.kids.push(row);
            }
            Display::TableCell => {
                let mut cell = LBox::new(Kind::TableCell);
                cell.node = Some(r);
                cell.style = Some(style);
                let mut inner = Vec::new();
                for &cc in ctx.doc.children(r) {
                    build_node(ctx, cc, &mut inner, &mut cell);
                }
                flush_inline(&mut inner, &mut cell);
                pending_cells.push(cell);
            }
            _ => {}
        }
    }
    if !pending_cells.is_empty() {
        let mut row = LBox::new(Kind::TableRow);
        row.kids = pending_cells;
        out.kids.push(row);
    }
}
