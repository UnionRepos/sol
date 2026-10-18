//! Flattening only within a stacking context lets positioned descendants escape
//! non-stacking ancestors while opacity and transforms keep their subtrees atomic.
use crate::{css_transform, graphics, overflow_clip, paint_single, DisplayList, Prim};
use sol_dom::Document;
use sol_layout::{Kind, LBox};

struct Entry<'a> {
    box_: &'a LBox,
    clips: Vec<&'a LBox>,
    phase: u8,
    z: i32,
    context: bool,
}
fn principal(b: &LBox) -> bool {
    !matches!(
        b.kind,
        Kind::Text | Kind::Marker | Kind::Line | Kind::AnonymousBlock
    )
}
fn z_index(b: &LBox) -> Option<i32> {
    b.style.as_ref()?.get("z-index")?.first().and_then(|value| {
        if let sol_css::Value::Number(n) = value {
            Some(*n as i32)
        } else {
            None
        }
    })
}
fn positioned(b: &LBox) -> bool {
    principal(b)
        && b.style
            .as_ref()
            .is_some_and(|s| s.position() != sol_css::Position::Static)
}
fn opacity(b: &LBox) -> f32 {
    if !principal(b) {
        return 1.0;
    }
    b.style
        .as_ref()
        .and_then(|s| s.get("opacity"))
        .and_then(|v| v.first())
        .map(|v| match v {
            sol_css::Value::Number(n) => *n as f32,
            sol_css::Value::Percentage(n) => *n as f32 / 100.0,
            _ => 1.0,
        })
        .unwrap_or(1.0)
        .clamp(0.0, 1.0)
}
fn context(b: &LBox, item: bool) -> bool {
    principal(b)
        && ((positioned(b) || item) && z_index(b).is_some()
            || opacity(b) < 1.0
            || b.style.as_ref().is_some_and(|s| {
                s.ident("isolation").as_deref() == Some("isolate")
                    || s.get("transform").is_some_and(
                        |v| !matches!(v.as_slice(),[sol_css::Value::Ident(s)] if s=="none"),
                    )
            }))
}
fn clip(b: &LBox) -> bool {
    principal(b) && overflow_clip(b.style.as_ref())
}
fn push_clip(b: &LBox, dl: &mut DisplayList) {
    let radii = b
        .style
        .as_ref()
        .map(|s| graphics::radii(s, b.rect))
        .unwrap_or_default();
    dl.prims.push(Prim::RoundedClip {
        rect: b.padding_rect(),
        radii: graphics::inset_radii(radii, b.border),
    });
}
fn collect<'a>(
    b: &'a LBox,
    clips: &[&'a LBox],
    inherited: u8,
    item: bool,
    out: &mut Vec<Entry<'a>>,
) {
    let context = context(b, item);
    let z = if context { z_index(b).unwrap_or(0) } else { 0 };
    let phase = if context && z < 0 {
        0
    } else if context && z > 0 {
        5
    } else if positioned(b) || context {
        4
    } else if inherited >= 4 {
        inherited
    } else if b.float != sol_css::Float::None {
        2
    } else if matches!(
        b.kind,
        Kind::Inline | Kind::InlineBlock | Kind::Text | Kind::Line | Kind::Marker
    ) {
        3
    } else {
        inherited.max(1)
    };
    out.push(Entry {
        box_: b,
        clips: clips.to_vec(),
        phase,
        z,
        context,
    });
    if !context {
        let mut clips = clips.to_vec();
        if clip(b) {
            clips.push(b);
        }
        let item = b.style.as_ref().is_some_and(|s| {
            matches!(
                s.display(),
                sol_css::Display::Flex
                    | sol_css::Display::InlineFlex
                    | sol_css::Display::Grid
                    | sol_css::Display::InlineGrid
            )
        });
        for child in &b.kids {
            collect(child, &clips, if phase >= 4 { phase } else { 1 }, item, out);
        }
    }
}
pub(super) fn paint(
    doc: &Document,
    root: &LBox,
    dl: &mut DisplayList,
    canvas_source: Option<sol_dom::NodeId>,
) {
    paint_context(doc, root, dl, canvas_source);
}
fn paint_context(
    doc: &Document,
    b: &LBox,
    dl: &mut DisplayList,
    canvas_source: Option<sol_dom::NodeId>,
) {
    let transform = principal(b)
        && b.style
            .as_ref()
            .and_then(|s| css_transform(s, b.rect))
            .map(|t| {
                dl.prims.push(Prim::PushTransform(t));
                true
            })
            .unwrap_or(false);
    let opacity = opacity(b);
    if opacity < 1.0 {
        dl.prims.push(Prim::PushOpacity(opacity));
    }
    paint_single(doc, b, dl, canvas_source);
    let clips = if clip(b) { vec![b] } else { Vec::new() };
    let item = b.style.as_ref().is_some_and(|s| {
        matches!(
            s.display(),
            sol_css::Display::Flex
                | sol_css::Display::InlineFlex
                | sol_css::Display::Grid
                | sol_css::Display::InlineGrid
        )
    });
    let mut entries = Vec::new();
    for child in &b.kids {
        collect(child, &clips, 1, item, &mut entries);
    }
    entries.sort_by_key(|entry| (entry.phase, entry.z));
    for entry in entries {
        for clip in &entry.clips {
            push_clip(clip, dl);
        }
        if entry.context {
            paint_context(doc, entry.box_, dl, canvas_source);
        } else {
            paint_single(doc, entry.box_, dl, canvas_source);
            crate::paint_outline(entry.box_, dl);
        }
        for _ in &entry.clips {
            dl.prims.push(Prim::PopClip);
        }
    }
    crate::paint_outline(b, dl);
    if opacity < 1.0 {
        dl.prims.push(Prim::PopOpacity);
    }
    if transform {
        dl.prims.push(Prim::PopTransform);
    }
}
