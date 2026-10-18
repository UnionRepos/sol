//! Flexbox layout: row/column, wrap, justify-content, align-items,
//! grow/shrink. Called from block layout when display is flex.

use crate::block::{lay_block, shift_pub, Floats, LC};
use crate::{Kind, LBox, Rect};
use sol_css::{Style, Value};
use std::rc::Rc;

fn fl(st: &Rc<Style>, name: &str, def: f32) -> f32 {
    st.get(name)
        .and_then(|v| v.first())
        .and_then(|v| match v {
            Value::Number(n) => Some(*n as f32),
            _ => None,
        })
        .unwrap_or(def)
}

fn lenx(st: &Rc<Style>, name: &str, pct: f32) -> Option<f32> {
    sol_css::style::value_px(
        st.get(name)?.first()?,
        st.length_px("font-size", 16.0, 800.0).unwrap_or(16.0),
        pct as f64,
    )
    .map(|x| x as f32)
}

fn ident(st: &Rc<Style>, name: &str) -> String {
    st.ident(name).unwrap_or_default()
}

/// Lay out the children of a flex container `b` inside content box
/// (x, y, w). Returns the content bottom y.
pub fn layout_flex(lc: &mut LC, b: &mut LBox, x: f32, y: f32, w: f32) -> f32 {
    let st = b.style.clone().unwrap();
    let dir = ident(&st, "flex-direction");
    let column = dir.starts_with("column");
    let reverse = dir.ends_with("-reverse");
    let wrap = ident(&st, "flex-wrap") == "wrap";
    let justify = ident(&st, "justify-content");
    let align_items = ident(&st, "align-items");
    let gap = lenx(&st, "column-gap", w)
        .or_else(|| lenx(&st, "row-gap", w))
        .unwrap_or(0.0);
    let row_gap = lenx(&st, "row-gap", w).unwrap_or(gap);
    let col_gap = lenx(&st, "column-gap", w).unwrap_or(gap);

    let kids = std::mem::take(&mut b.kids);
    struct Item {
        b: LBox,
        basis: f32,
        grow: f32,
        shrink: f32,
        main: f32,
        align_self: String,
    }
    let mut items: Vec<Item> = Vec::new();
    for k in kids {
        let kst = k.style.clone();
        let (basis, grow, shrink, aself) = if let Some(s) = &kst {
            (
                lenx(s, "flex-basis", w)
                    .or_else(|| lenx(s, "width", w))
                    .unwrap_or(-1.0),
                fl(s, "flex-grow", 0.0),
                fl(s, "flex-shrink", 1.0),
                ident(s, "align-self"),
            )
        } else {
            (-1.0, 0.0, 1.0, String::new())
        };
        items.push(Item {
            b: k,
            basis,
            grow,
            shrink,
            main: 0.0,
            align_self: aself,
        });
    }

    // measure natural main size for items with auto basis: lay out once
    // at container width on a CLONE — laying mutates the box (text turns
    // into Line boxes), and the item is laid for real during placement, so
    // the probe must not touch it.b. (full shrink-to-fit is costly)
    for it in items.iter_mut() {
        if it.basis < 0.0 {
            let mut probe = it.b.clone();
            let mut floats = Floats::new();
            let mut pm = None;
            let cb = Rect { x, y, w, h: 0.0 };
            lay_block(lc, &mut probe, (x, y), w, &mut floats, &mut pm, cb);
            it.basis = if column { probe.rect.h } else { probe.rect.w };
        }
    }

    // group into lines
    let mut lines: Vec<Vec<usize>> = vec![vec![]];
    let mut line_len = 0.0f32;
    for (i, it) in items.iter().enumerate() {
        let needed = it.basis
            + if lines.last().unwrap().is_empty() {
                0.0
            } else {
                col_gap
            };
        if wrap && line_len + needed > w + 0.01 && !lines.last().unwrap().is_empty() {
            lines.push(vec![]);
            line_len = 0.0;
        }
        line_len += needed;
        lines.last_mut().unwrap().push(i);
    }

    // resolve main sizes per line
    for line in &lines {
        let total_basis: f32 = line.iter().map(|&i| items[i].basis).sum();
        let gaps = (line.len().saturating_sub(1)) as f32 * col_gap;
        let free = w - total_basis - gaps;
        let total_grow: f32 = line.iter().map(|&i| items[i].grow).sum();
        let total_shrink_w: f32 = line.iter().map(|&i| items[i].shrink * items[i].basis).sum();
        for &i in line {
            let it = &mut items[i];
            it.main = if free > 0.0 && total_grow > 0.0 {
                it.basis + free * it.grow / total_grow
            } else if free < 0.0 && total_shrink_w > 0.0 {
                (it.basis + free * (it.shrink * it.basis) / total_shrink_w).max(0.0)
            } else {
                it.basis
            };
        }
    }

    // place + lay out each line
    let mut cy = y;
    let mut final_kids = Vec::new();
    for line in &lines {
        let total_main: f32 = line.iter().map(|&i| items[i].main).sum::<f32>()
            + (line.len().saturating_sub(1)) as f32 * col_gap;
        let remain = (w - total_main).max(0.0);
        let (mut pen, extra_gap) = match justify.as_str() {
            "flex-end" | "end" | "right" => (x + remain, 0.0),
            "center" => (x + remain / 2.0, 0.0),
            "space-between" => (
                x,
                if line.len() > 1 {
                    remain / (line.len() - 1) as f32
                } else {
                    0.0
                },
            ),
            "space-around" => (
                x + remain / (line.len() * 2) as f32,
                remain / line.len().max(1) as f32,
            ),
            "space-evenly" => (
                x + remain / (line.len() + 1) as f32,
                remain / (line.len() + 1) as f32,
            ),
            _ => (x, 0.0),
        };
        let order: Vec<usize> = if reverse {
            line.iter().rev().copied().collect()
        } else {
            line.clone()
        };
        // lay out items to learn cross size
        let mut line_boxes: Vec<(String, LBox)> = Vec::new();
        let mut max_cross = 0.0f32;
        for &i in &order {
            let it = &mut items[i];
            let mut kb = std::mem::replace(&mut it.b, LBox::new(Kind::Block));
            let aself = if it.align_self.is_empty() || it.align_self == "auto" {
                align_items.clone()
            } else {
                it.align_self.clone()
            };
            let mut floats = Floats::new();
            let mut pm = None;
            if column {
                let cb = Rect {
                    x,
                    y: cy + pen - x,
                    w,
                    h: 0.0,
                };
                // for column direction, "main" is height; x positions are
                // cross-axis — line (column) index selects x offset
                lay_block(
                    lc,
                    &mut kb,
                    (x, cy + (pen - x)),
                    w,
                    &mut floats,
                    &mut pm,
                    cb,
                );
            } else {
                let cb = Rect {
                    x,
                    y: cy,
                    w: it.main,
                    h: 0.0,
                };
                lay_block(lc, &mut kb, (pen, cy), it.main, &mut floats, &mut pm, cb);
                let (kx, ky) = (kb.rect.x, kb.rect.y);
                shift_pub(&mut kb, pen - kx, cy - ky);
            }
            let cross = if column { kb.rect.w } else { kb.rect.h };
            max_cross = max_cross.max(cross);
            line_boxes.push((aself, kb));
            pen += it.main + col_gap + extra_gap;
        }
        for (aself, mut kb) in line_boxes {
            let cross = if column { kb.rect.w } else { kb.rect.h };
            let off = match aself.as_str() {
                "flex-end" | "end" => max_cross - cross,
                "center" => (max_cross - cross) / 2.0,
                _ => 0.0,
            };
            if column {
                shift_pub(&mut kb, off, 0.0);
            } else {
                shift_pub(&mut kb, 0.0, off);
            }
            final_kids.push(kb);
        }
        cy += if column { 0.0 } else { max_cross } + row_gap;
    }
    if column {
        cy = final_kids.iter().map(|k| k.rect.bottom()).fold(y, f32::max);
    }
    b.kids = final_kids;
    cy
}
