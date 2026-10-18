//! Tracks and item placement stay separate so intrinsic measurement cannot
//! change the placement of later items or leak positioned probe boxes.
use crate::block::{border_edges, edges, lay_block, len, shift_pub, Floats, LC};
use crate::{LBox, Rect};
use sol_css::{Position, Style, Value};
use std::collections::HashSet;
use std::rc::Rc;

const MAX_TRACKS: usize = 256;

#[derive(Clone, Copy, Default)]
struct Track {
    base: f32,
    flex: f32,
    auto: bool,
    limit: Option<f32>,
}
fn number(value: &Value, percent: f32) -> Option<f32> {
    sol_css::style::value_px(value, 16.0, percent as f64).map(|v| (v as f32).max(0.0))
}
fn track(value: &Value, percent: f32) -> Track {
    match value {
        Value::Length(n, sol_css::values::Unit::Fr) => Track {
            flex: (*n as f32).max(0.0),
            ..Track::default()
        },
        Value::Func(name, args) if name == "minmax" => {
            let mut parts = args.split(|v| matches!(v, Value::Ident(s) if s == ","));
            let min = parts.next().and_then(|v| v.first());
            let max = parts.next().and_then(|v| v.first());
            let mut result = max.map(|v| track(v, percent)).unwrap_or_default();
            result.base = min.and_then(|v| number(v, percent)).unwrap_or(0.0);
            if result.flex == 0.0 && !result.auto {
                result.limit = max
                    .and_then(|v| number(v, percent))
                    .map(|v| v.max(result.base));
            }
            result
        }
        Value::Func(name, args) if name == "fit-content" => Track {
            auto: true,
            limit: args.first().and_then(|v| number(v, percent)),
            ..Track::default()
        },
        Value::Ident(name) if matches!(name.as_str(), "auto" | "min-content" | "max-content") => {
            Track {
                auto: true,
                ..Track::default()
            }
        }
        _ => Track {
            base: number(value, percent).unwrap_or(0.0),
            ..Track::default()
        },
    }
}
fn tracks(values: Option<&Vec<Value>>, percent: f32, gap: f32, depth: usize) -> Vec<Track> {
    if depth > 16 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for value in values.into_iter().flatten() {
        match value {
            Value::Block('[', _) | Value::Ident(_)
                if matches!(value, Value::Block('[', _))
                    || matches!(value,Value::Ident(s) if s=="none") => {}
            Value::Func(name, args) if name == "repeat" => {
                let Some(comma) = args
                    .iter()
                    .position(|v| matches!(v,Value::Ident(s) if s==","))
                else {
                    continue;
                };
                let children = tracks(Some(&args[comma + 1..].to_vec()), percent, gap, depth + 1);
                if children.is_empty() {
                    continue;
                }
                let count = match args.first() {
                    Some(Value::Number(n)) if *n >= 1.0 && n.fract() == 0.0 => {
                        (*n as usize).min(MAX_TRACKS)
                    }
                    Some(Value::Ident(name))
                        if matches!(name.as_str(), "auto-fit" | "auto-fill") =>
                    {
                        let minimum = children.iter().map(|t| t.base.max(1.0)).sum::<f32>()
                            + gap * children.len().saturating_sub(1) as f32;
                        (((percent + gap) / (minimum + gap)).floor() as usize).clamp(1, MAX_TRACKS)
                    }
                    _ => continue,
                };
                for _ in 0..count {
                    for child in &children {
                        if out.len() < MAX_TRACKS {
                            out.push(*child);
                        }
                    }
                }
            }
            _ if out.len() < MAX_TRACKS => out.push(track(value, percent)),
            _ => {}
        }
    }
    out
}
fn axis(style: Option<&Rc<Style>>, name: &str, count: usize) -> (Option<usize>, usize) {
    let start = style.and_then(|s| s.get(&format!("grid-{name}-start")));
    let end = style.and_then(|s| s.get(&format!("grid-{name}-end")));
    let line = |values: Option<&Vec<Value>>| match values?.as_slice() {
        [Value::Number(n)] if *n != 0.0 && n.fract() == 0.0 && n.abs() <= MAX_TRACKS as f64 => {
            if *n > 0.0 {
                Some(*n as usize - 1)
            } else {
                (count as isize + 1 + *n as isize).try_into().ok()
            }
        }
        _ => None,
    };
    let span = |values: Option<&Vec<Value>>| match values?.as_slice() {
        [Value::Ident(s), Value::Number(n)] if s == "span" && *n >= 1.0 && n.fract() == 0.0 => {
            Some((*n as usize).min(MAX_TRACKS))
        }
        _ => None,
    };
    let a = line(start);
    let b = line(end);
    let span = span(end)
        .or_else(|| span(start))
        .unwrap_or_else(|| match (a, b) {
            (Some(a), Some(b)) if b > a => b - a,
            _ => 1,
        });
    (a.or_else(|| b.map(|b| b.saturating_sub(span))), span.max(1))
}
fn size_tracks(tracks: &mut [Track], available: Option<f32>, gap: f32) {
    let Some(available) = available else { return };
    let fixed =
        tracks.iter().map(|t| t.base).sum::<f32>() + gap * tracks.len().saturating_sub(1) as f32;
    let mut free = (available - fixed).max(0.0);
    let mut active: Vec<usize> = tracks
        .iter()
        .enumerate()
        .filter_map(|(i, t)| (t.flex > 0.0).then_some(i))
        .collect();
    while !active.is_empty() {
        let total = active.iter().map(|&i| tracks[i].flex).sum::<f32>().max(1.0);
        let unit = (available
            - gap * tracks.len().saturating_sub(1) as f32
            - tracks
                .iter()
                .enumerate()
                .filter(|(i, _)| !active.contains(i))
                .map(|(_, t)| t.base)
                .sum::<f32>())
        .max(0.0)
            / total;
        let frozen: Vec<_> = active
            .iter()
            .copied()
            .filter(|&i| tracks[i].base > tracks[i].flex * unit)
            .collect();
        if frozen.is_empty() {
            for i in active {
                tracks[i].base = (tracks[i].flex * unit).max(tracks[i].base);
            }
            free = 0.0;
            break;
        }
        active.retain(|i| !frozen.contains(i));
    }
    let auto = tracks.iter().filter(|t| t.auto).count();
    if auto > 0 && free > 0.0 {
        for t in tracks {
            if t.auto {
                t.base = (t.base + free / auto as f32).min(t.limit.unwrap_or(f32::INFINITY));
            }
        }
    }
}
fn offsets(tracks: &[Track], gap: f32) -> Vec<f32> {
    let mut out = vec![0.0];
    for track in tracks {
        out.push(out.last().copied().unwrap() + track.base + gap);
    }
    out
}
fn span_size(tracks: &[Track], start: usize, span: usize, gap: f32) -> f32 {
    tracks[start..start + span]
        .iter()
        .map(|t| t.base)
        .sum::<f32>()
        + gap * span.saturating_sub(1) as f32
}
fn intrinsic(box_: &LBox, lc: &LC) -> f32 {
    let style = box_.style.as_ref();
    if let Some(width) = len(style, "width", lc.vw) {
        return width + edges(style, "padding", lc.vw).horiz() + border_edges(style, lc.vw).horiz();
    }
    if let Some(text) = &box_.text {
        return text
            .text
            .split_ascii_whitespace()
            .map(|word| lc.faces.get(text.face_idx).measure(word))
            .fold(0.0, f32::max);
    }
    box_.kids
        .iter()
        .map(|b| intrinsic(b, lc))
        .fold(0.0, f32::max)
}

fn areas(style: &Style) -> std::collections::HashMap<String, (usize, usize, usize, usize)> {
    let rows: Vec<Vec<&str>> = style
        .get("grid-template-areas")
        .into_iter()
        .flatten()
        .filter_map(|value| {
            if let Value::Str(row) = value {
                Some(row.split_ascii_whitespace().collect())
            } else {
                None
            }
        })
        .collect();
    let Some(width) = rows.first().map(Vec::len).filter(|width| *width > 0) else {
        return Default::default();
    };
    if width > MAX_TRACKS || rows.len() > MAX_TRACKS || rows.iter().any(|row| row.len() != width) {
        return Default::default();
    }
    let mut result = std::collections::HashMap::new();
    for (r, row) in rows.iter().enumerate() {
        for (c, name) in row.iter().enumerate() {
            if name.chars().all(|c| c == '.') {
                continue;
            }
            let entry = result
                .entry(name.to_string())
                .or_insert((c, r, c + 1, r + 1));
            entry.0 = entry.0.min(c);
            entry.1 = entry.1.min(r);
            entry.2 = entry.2.max(c + 1);
            entry.3 = entry.3.max(r + 1);
        }
    }
    for (name, &(x, y, right, bottom)) in &result {
        if rows[y..bottom]
            .iter()
            .any(|row| row[x..right].iter().any(|cell| *cell != name))
        {
            return Default::default();
        }
    }
    result
}
pub fn layout_grid(lc: &mut LC, b: &mut LBox, x: f32, y: f32, w: f32) -> f32 {
    let style = b.style.clone().unwrap_or_default();
    let height = len(Some(&style), "height", lc.vh);
    let column_gap = len(Some(&style), "column-gap", w).unwrap_or(0.0).max(0.0);
    let row_gap = len(Some(&style), "row-gap", height.unwrap_or(0.0))
        .unwrap_or(0.0)
        .max(0.0);
    let mut columns = tracks(style.get("grid-template-columns"), w, column_gap, 0);
    let mut rows = tracks(
        style.get("grid-template-rows"),
        height.unwrap_or(0.0),
        row_gap,
        0,
    );
    let areas = areas(&style);
    let area_columns = areas.values().map(|area| area.2).max().unwrap_or(0);
    let area_rows = areas.values().map(|area| area.3).max().unwrap_or(0);
    while columns.len() < area_columns {
        columns.push(Track {
            auto: true,
            ..Track::default()
        });
    }
    while rows.len() < area_rows {
        rows.push(Track {
            auto: true,
            ..Track::default()
        });
    }
    let explicit_columns = columns.len();
    let explicit_rows = rows.len();
    if columns.is_empty() {
        columns.push(Track {
            auto: true,
            ..Track::default()
        });
    }
    if rows.is_empty() {
        rows.push(Track {
            auto: true,
            ..Track::default()
        });
    }
    let auto_columns = tracks(style.get("grid-auto-columns"), w, column_gap, 0);
    let auto_rows = tracks(
        style.get("grid-auto-rows"),
        height.unwrap_or(0.0),
        row_gap,
        0,
    );
    if explicit_columns == 0 {
        if let Some(track) = auto_columns.first() {
            columns[0] = *track;
        }
    }
    if explicit_rows == 0 {
        if let Some(track) = auto_rows.first() {
            rows[0] = *track;
        }
    }
    let mut kids = std::mem::take(&mut b.kids);
    kids.sort_by_key(|item| {
        item.style
            .as_ref()
            .and_then(|s| s.get("order"))
            .and_then(|v| v.first())
            .and_then(|v| {
                if let Value::Number(n) = v {
                    Some(*n as i32)
                } else {
                    None
                }
            })
            .unwrap_or(0)
    });
    let column_flow = style.get("grid-auto-flow").is_some_and(|values| {
        values
            .iter()
            .any(|v| matches!(v,Value::Ident(s) if s=="column"))
    });
    let dense = style.get("grid-auto-flow").is_some_and(|values| {
        values
            .iter()
            .any(|v| matches!(v,Value::Ident(s) if s=="dense"))
    });
    struct Item {
        box_: LBox,
        column: usize,
        row: usize,
        columns: usize,
        rows: usize,
    }
    let mut placed = Vec::new();
    let mut absolute = Vec::new();
    let mut occupied = HashSet::new();
    let mut cursor = (0usize, 0usize);
    // Definite placements reserve their cells before the auto-placement cursor
    // advances, including when an explicitly placed item appears later in DOM order.
    let mut pending = Vec::new();
    for item in kids {
        if item
            .style
            .as_ref()
            .is_some_and(|s| matches!(s.position(), Position::Absolute | Position::Fixed))
        {
            absolute.push(item);
            continue;
        }
        let (mut column, mut cs) = axis(item.style.as_ref(), "column", explicit_columns);
        let (mut row, mut rs) = axis(item.style.as_ref(), "row", explicit_rows);
        if let Some(name) = item
            .style
            .as_ref()
            .and_then(|style| style.ident("grid-area"))
        {
            if let Some(&(x, y, right, bottom)) = areas.get(&name) {
                let overridden = |axis: &str| {
                    item.style.as_ref().and_then(|style|style.get(&format!("grid-{axis}-start"))).is_some_and(|values|!matches!(values.as_slice(),[Value::Ident(value)] if value==&name))
                };
                if column.is_none() && !overridden("column") {
                    column = Some(x);
                    cs = right - x;
                }
                if row.is_none() && !overridden("row") {
                    row = Some(y);
                    rs = bottom - y;
                }
            }
        }
        if let (Some(column), Some(row)) = (column, row) {
            for r in row..(row + rs).min(MAX_TRACKS) {
                for c in column..(column + cs).min(MAX_TRACKS) {
                    occupied.insert((c, r));
                }
            }
        }
        pending.push((item, column, row, cs, rs));
    }
    let auto_column_count = columns
        .len()
        .max(
            pending
                .iter()
                .map(|(_, c, _, span, _)| c.map_or(*span, |c| c + span))
                .max()
                .unwrap_or(1),
        )
        .min(MAX_TRACKS);
    let auto_row_count = rows
        .len()
        .max(
            pending
                .iter()
                .map(|(_, _, r, _, span)| r.map_or(*span, |r| r + span))
                .max()
                .unwrap_or(1),
        )
        .min(MAX_TRACKS);
    for (item, column, row, cs, rs) in pending {
        let mut c = column.unwrap_or(if dense { 0 } else { cursor.0 });
        let mut r = row.unwrap_or(if dense { 0 } else { cursor.1 });
        if !(column.is_some() && row.is_some()) {
            if let Some(column) = column {
                c = column;
                if row.is_none() && c < cursor.0 && !dense {
                    r += 1;
                }
            }
            if let Some(row) = row {
                r = row;
                if column.is_none() && r < cursor.1 && column_flow && !dense {
                    c += 1;
                }
            }
            for _ in 0..MAX_TRACKS * MAX_TRACKS {
                if column.is_none() && !column_flow && c + cs > auto_column_count {
                    c = 0;
                    r += 1;
                }
                if row.is_none() && column_flow && r + rs > auto_row_count {
                    r = 0;
                    c += 1;
                }
                if c + cs > MAX_TRACKS || r + rs > MAX_TRACKS {
                    c = 0;
                    r = MAX_TRACKS - rs;
                    break;
                }
                if (r..r + rs).all(|r| (c..c + cs).all(|c| !occupied.contains(&(c, r)))) {
                    break;
                }
                if column.is_some() {
                    r += 1;
                } else if row.is_some() || !column_flow {
                    c += 1;
                } else {
                    r += 1;
                }
            }
        }
        c = c.min(MAX_TRACKS - cs);
        r = r.min(MAX_TRACKS - rs);
        for rr in r..r + rs {
            for cc in c..c + cs {
                occupied.insert((cc, rr));
            }
        }
        cursor = (c, r);
        placed.push(Item {
            box_: item,
            column: c,
            row: r,
            columns: cs,
            rows: rs,
        });
    }
    let column_count = placed
        .iter()
        .map(|i| i.column + i.columns)
        .max()
        .unwrap_or(columns.len());
    let row_count = placed
        .iter()
        .map(|i| i.row + i.rows)
        .max()
        .unwrap_or(rows.len());
    while columns.len() < column_count {
        let i = columns.len().saturating_sub(explicit_columns);
        columns.push(
            auto_columns
                .get(i % auto_columns.len().max(1))
                .copied()
                .unwrap_or(Track {
                    auto: true,
                    ..Track::default()
                }),
        );
    }
    while rows.len() < row_count {
        let i = rows.len().saturating_sub(explicit_rows);
        rows.push(
            auto_rows
                .get(i % auto_rows.len().max(1))
                .copied()
                .unwrap_or(Track {
                    auto: true,
                    ..Track::default()
                }),
        );
    }
    for item in &placed {
        if item.columns == 1 && columns[item.column].auto {
            columns[item.column].base = columns[item.column]
                .base
                .max(intrinsic(&item.box_, lc))
                .min(columns[item.column].limit.unwrap_or(f32::INFINITY));
        }
    }
    size_tracks(&mut columns, Some(w), column_gap);
    let xs = offsets(&columns, column_gap);
    for item in &placed {
        let width = span_size(&columns, item.column, item.columns, column_gap);
        let mut probe = item.box_.clone();
        let deferred = lc.deferred.len();
        lay_block(
            lc,
            &mut probe,
            (0.0, 0.0),
            width,
            &mut Floats::new(),
            &mut None,
            Rect {
                x: 0.0,
                y: 0.0,
                w: width,
                h: height.unwrap_or(lc.vh),
            },
        );
        lc.deferred.truncate(deferred);
        let natural = probe.rect.h + probe.margin.vert();
        let current = span_size(&rows, item.row, item.rows, row_gap);
        let grow: Vec<_> = (item.row..item.row + item.rows)
            .filter(|&r| rows[r].auto || rows[r].flex > 0.0)
            .collect();
        for r in &grow {
            rows[*r].base += (natural - current).max(0.0) / grow.len() as f32;
        }
    }
    size_tracks(&mut rows, height, row_gap);
    let ys = offsets(&rows, row_gap);
    for mut item in placed {
        let width = span_size(&columns, item.column, item.columns, column_gap);
        let height = span_size(&rows, item.row, item.rows, row_gap);
        let cell = Rect {
            x: x + xs[item.column],
            y: y + ys[item.row],
            w: width,
            h: height,
        };
        let mut item_style = item.box_.style.as_deref().cloned().unwrap_or_default();
        let justify = item_style
            .ident("justify-self")
            .filter(|s| s != "auto")
            .or_else(|| style.ident("justify-items"))
            .unwrap_or_else(|| "stretch".into());
        let align = item_style
            .ident("align-self")
            .filter(|s| s != "auto")
            .or_else(|| style.ident("align-items"))
            .unwrap_or_else(|| "stretch".into());
        let original = item.box_.style.clone();
        let margin = edges(original.as_ref(), "margin", width);
        if align == "stretch" && len(original.as_ref(), "height", height).is_none() {
            let extra = edges(original.as_ref(), "padding", width).vert()
                + border_edges(original.as_ref(), width).vert();
            item_style.set(
                "height",
                vec![Value::Length(
                    (height
                        - margin.vert()
                        - if item_style.ident("box-sizing").as_deref() == Some("border-box") {
                            0.0
                        } else {
                            extra
                        })
                    .max(0.0) as f64,
                    sol_css::values::Unit::Px,
                )],
            );
        }
        if justify != "stretch" && len(original.as_ref(), "width", width).is_none() {
            item_style.set(
                "width",
                vec![Value::Length(
                    intrinsic(&item.box_, lc).min(width) as f64,
                    sol_css::values::Unit::Px,
                )],
            );
        }
        item.box_.style = Some(Rc::new(item_style));
        lay_block(
            lc,
            &mut item.box_,
            (cell.x, cell.y),
            width,
            &mut Floats::new(),
            &mut None,
            cell,
        );
        item.box_.style = original;
        let horizontal = (width - item.box_.rect.w - margin.horiz()).max(0.0);
        let vertical = (height - item.box_.rect.h - margin.vert()).max(0.0);
        let offset = |alignment: &str, space: f32| match alignment {
            "center" => space / 2.0,
            "end" | "flex-end" | "right" => space,
            _ => 0.0,
        };
        shift_pub(
            &mut item.box_,
            offset(&justify, horizontal),
            offset(&align, vertical),
        );
        b.kids.push(item.box_);
    }
    for mut item in absolute {
        lay_block(
            lc,
            &mut item,
            (x, y),
            w,
            &mut Floats::new(),
            &mut None,
            Rect {
                x,
                y,
                w,
                h: height.unwrap_or(lc.vh),
            },
        );
        b.kids.push(item);
    }
    y + rows.iter().map(|t| t.base).sum::<f32>() + row_gap * rows.len().saturating_sub(1) as f32
}
