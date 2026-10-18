//! Automatic table layout: columns sized by content (min-content probe
//! is approximated by laying out cells at table width), rows stacked.

use crate::block::{border_edges, edges, lay_children, Floats, LC};
use crate::{Kind, LBox, Rect};
use sol_css::{Style, Value};
use std::rc::Rc;

fn len(st: Option<&Rc<Style>>, name: &str, pct: f32) -> Option<f32> {
    let s = st?;
    let v = s.get(name)?.first()?;
    if let Value::Ident(id) = v {
        if id == "auto" {
            return None;
        }
    }
    sol_css::style::value_px(
        v,
        s.length_px("font-size", 16.0, 800.0).unwrap_or(16.0),
        pct as f64,
    )
    .map(|x| x as f32)
}

fn cell_spacing(st: Option<&Rc<Style>>) -> (f32, f32, bool) {
    // (h_spacing, v_spacing, collapsed)
    let collapse = st
        .and_then(|s| s.ident("border-collapse"))
        .map(|v| v == "collapse")
        .unwrap_or(false);
    let sp = st
        .and_then(|s| s.get("border-spacing"))
        .and_then(|v| v.first())
        .and_then(|v| sol_css::style::value_px(v, 16.0, 0.0))
        .unwrap_or(2.0) as f32;
    (sp, sp, collapse)
}

/// Lay out a table box: `x,y` = content origin, `w` = content width.
/// Sets b.rect.h; kids are TableCaption / TableRow boxes.
pub fn layout_table(lc: &mut LC, b: &mut LBox, x: f32, y: f32, w: f32) {
    let st = b.style.clone();
    let (hsp, vsp, collapse) = cell_spacing(st.as_ref());
    let spacing = if collapse { 0.0 } else { hsp };

    // collect rows (kids may include anonymous wrappers already folded
    // by tree builder into TableRow)
    let kids = std::mem::take(&mut b.kids);
    let mut rows: Vec<LBox> = Vec::new();
    let mut captions: Vec<LBox> = Vec::new();
    for k in kids {
        match k.kind {
            Kind::TableRow => rows.push(k),
            Kind::TableCaption => captions.push(k),
            Kind::AnonymousBlock | Kind::Block => {
                // stray content: treat each row-kind descendant? skip
            }
            _ => {}
        }
    }

    // first pass: determine column count
    let mut ncols = 0usize;
    for r in &rows {
        let mut c = 0usize;
        for cell in &r.kids {
            let span = cell
                .node
                .and_then(|n| lc.doc.attr(n, "colspan"))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);
            c += span;
        }
        ncols = ncols.max(c);
    }
    if ncols == 0 {
        b.rect.h = 0.0;
        b.kids = rows;
        return;
    }

    // column width pass: measure each cell's natural width by laying
    // out at a probe width, then distribute
    let mut col_w = vec![0.0f32; ncols];
    let mut col_fixed = vec![false; ncols];
    let mut probe_rows: Vec<Vec<(LBox, usize)>> = Vec::new(); // (cell, span)
    for r in &rows {
        let mut row_cells = Vec::new();
        let mut ci = 0usize;
        for cell in &r.kids {
            let span = cell
                .node
                .and_then(|n| lc.doc.attr(n, "colspan"))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);
            let cst = cell.style.clone();
            let pad = edges(cst.as_ref(), "padding", w);
            let brd = border_edges(cst.as_ref(), w);
            let spec = len(cst.as_ref(), "width", w);
            let extras = pad.horiz() + brd.horiz();
            if let Some(sw) = spec {
                let share = (sw + extras) / span as f32;
                for c in ci..(ci + span).min(ncols) {
                    col_w[c] = col_w[c].max(share);
                    col_fixed[c] = true;
                }
            } else {
                // measure content: lay out cell contents at wide width
                // to get a natural width estimate — expensive but simple
                let mut probe = LBox::new(Kind::Block);
                probe.kids = cell.kids.iter().map(|k| k.clone()).collect();
                let mut f = Floats::new();
                let cb = Rect {
                    x: 0.0,
                    y: 0.0,
                    w: w * 2.0,
                    h: 0.0,
                };
                let bottom = lay_children(lc, &mut probe, 0.0, 0.0, w * 2.0, &mut f, cb);
                // estimate min width from widest word (probe kids' max w)
                let mut maxw = 0.0f32;
                let mut flat: Vec<&LBox> = Vec::new();
                probe.walk(&mut flat);
                for bx in flat {
                    if bx.kind == Kind::Text && bx.rect.w > maxw {
                        maxw = bx.rect.w;
                    }
                }
                let _ = bottom;
                // approximate natural width: widest text run + extras
                let share = (maxw + extras) / span as f32;
                for c in ci..(ci + span).min(ncols) {
                    if !col_fixed[c] {
                        col_w[c] = col_w[c].max(share);
                    }
                }
            }
            row_cells.push((cell.clone(), span));
            ci += span;
        }
        probe_rows.push(row_cells);
    }

    // distribute remaining width evenly among unfixed columns
    let inner_w = w - spacing * (ncols + 1) as f32;
    let total: f32 = col_w.iter().sum();
    if total < inner_w {
        let unfixed: Vec<usize> = (0..ncols).filter(|&c| !col_fixed[c]).collect();
        if !unfixed.is_empty() {
            let extra = (inner_w - total) / unfixed.len() as f32;
            for &c in &unfixed {
                col_w[c] += extra;
            }
        } else {
            // scale all
            let extra = (inner_w - total) / ncols as f32;
            for c in &mut col_w {
                *c += extra;
            }
        }
    } else if total > inner_w && total > 0.0 {
        let scale = inner_w / total;
        for c in &mut col_w {
            *c *= scale;
        }
    }

    // second pass: lay out rows and cells
    let mut cy = y + spacing;
    // caption top
    for cap in &mut captions {
        cap.rect = Rect {
            x,
            y: cy,
            w,
            h: 0.0,
        };
        let mut f = Floats::new();
        let cb = Rect {
            x,
            y: cy,
            w,
            h: 0.0,
        };
        let bottom = lay_children(lc, cap, x, cy, w, &mut f, cb);
        cap.rect.h = bottom - cy;
        cy = bottom + spacing;
    }

    let mut new_rows = Vec::new();
    for (r, cells) in rows.iter().zip(&probe_rows) {
        let mut row = r.clone();
        row.kids.clear();
        row.rect = Rect {
            x,
            y: cy,
            w,
            h: 0.0,
        };
        let mut cx = x + spacing;
        let mut row_h = 0.0f32;
        let mut ci = 0usize;
        let mut laid: Vec<LBox> = Vec::new();
        for (cell, span) in cells {
            let mut cw = 0.0f32;
            for c in ci..(ci + span).min(ncols) {
                cw += col_w[c];
            }
            cw += spacing * span.saturating_sub(1) as f32;
            let mut c2 = cell.clone();
            let cst = c2.style.clone();
            c2.padding = edges(cst.as_ref(), "padding", cw);
            c2.border = border_edges(cst.as_ref(), cw);
            let content_x = cx + c2.border.left + c2.padding.left;
            let content_w = (cw - c2.border.horiz() - c2.padding.horiz()).max(0.0);
            c2.rect = Rect {
                x: cx,
                y: cy,
                w: cw,
                h: 0.0,
            };
            let mut f = Floats::new();
            let cb = Rect {
                x: content_x,
                y: cy,
                w: content_w,
                h: 0.0,
            };
            let spec_h = len(cst.as_ref(), "height", lc.vh);
            let content_y = cy + c2.border.top + c2.padding.top;
            let bottom = lay_children(lc, &mut c2, content_x, content_y, content_w, &mut f, cb);
            let mut ch = bottom - content_y + c2.padding.vert() + c2.border.vert();
            if let Some(eh) = spec_h {
                ch = ch.max(eh);
            }
            c2.rect.h = ch;
            row_h = row_h.max(ch);
            ci += span;
            cx += cw + spacing;
            laid.push(c2);
        }
        // equalize cell heights in the row
        for c2 in &mut laid {
            if c2.rect.h < row_h {
                c2.rect.h = row_h;
            }
        }
        row.kids = laid;
        row.rect.h = row_h;
        cy += row_h + vsp;
        new_rows.push(row);
    }
    b.rect.h = cy - y - spacing + if collapse { 0.0 } else { spacing };
    let mut kids2 = captions;
    kids2.extend(new_rows);
    b.kids = kids2;
}
