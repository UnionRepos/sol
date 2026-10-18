//! Generated content stays outside the DOM so rendering cannot change selector
//! matching, script traversal, or node identity. Counter state therefore follows
//! layout traversal with explicit scopes instead of attaching synthetic DOM nodes.
use crate::{
    tree::{make_face, Ctx},
    Kind, LBox, TextRun,
};
use sol_css::{Display, Style, Value};
use sol_dom::NodeId;
use std::rc::Rc;

#[derive(Default)]
pub(crate) struct Counters {
    values: std::collections::HashMap<String, Vec<(NodeId, i64)>>,
    quotes: usize,
    scopes: std::collections::HashMap<NodeId, std::collections::HashSet<String>>,
}
pub(crate) fn enter(ctx: &mut Ctx, node: NodeId, style: &Style, pseudo: bool) {
    let scope = if pseudo {
        node
    } else {
        ctx.doc.parent(node).unwrap_or(node)
    };
    for (property, default) in [
        ("counter-reset", 0),
        ("counter-set", 0),
        ("counter-increment", 1),
    ] {
        let values = style.get(property);
        let mut index = 0;
        while let Some(value) = values.and_then(|values| values.get(index)) {
            let Value::Ident(name) = value else {
                index += 1;
                continue;
            };
            if name == "none" {
                break;
            }
            index += 1;
            let value =
                if let Some(Value::Number(value)) = values.and_then(|values| values.get(index)) {
                    index += 1;
                    *value as i64
                } else {
                    default
                };
            let stack = ctx.counters.values.entry(name.clone()).or_default();
            if property == "counter-reset" {
                if stack.last().is_some_and(|(owner, _)| *owner == scope) {
                    stack.last_mut().unwrap().1 = value;
                } else {
                    stack.push((scope, value));
                    ctx.counters
                        .scopes
                        .entry(scope)
                        .or_default()
                        .insert(name.clone());
                }
            } else {
                if stack.is_empty() {
                    stack.push((scope, 0));
                    ctx.counters
                        .scopes
                        .entry(scope)
                        .or_default()
                        .insert(name.clone());
                }
                let current = &mut stack.last_mut().unwrap().1;
                if property == "counter-set" {
                    *current = value
                } else {
                    *current = current.saturating_add(value);
                }
            }
        }
    }
    if style.display() == Display::ListItem && style.get("counter-increment").is_none() {
        let stack = ctx.counters.values.entry("list-item".into()).or_default();
        if stack.is_empty() {
            stack.push((scope, 0));
            ctx.counters
                .scopes
                .entry(scope)
                .or_default()
                .insert("list-item".into());
        }
        let current = &mut stack.last_mut().unwrap().1;
        *current = current.saturating_add(1);
    }
}
pub(crate) fn leave(ctx: &mut Ctx, node: NodeId) {
    if let Some(names) = ctx.counters.scopes.remove(&node) {
        for name in names {
            if let Some(stack) = ctx.counters.values.get_mut(&name) {
                while stack.last().is_some_and(|(scope, _)| *scope == node) {
                    stack.pop();
                }
                if stack.is_empty() {
                    ctx.counters.values.remove(&name);
                }
            }
        }
    }
}

fn format_number(number: i64, kind: &str) -> String {
    if number <= 0 {
        return number.to_string();
    }
    match kind {
        "lower-alpha" | "lower-latin" | "upper-alpha" | "upper-latin" => {
            let mut number = number as u64;
            let mut out = String::new();
            while number > 0 {
                number -= 1;
                out.insert(
                    0,
                    ((if kind.starts_with("upper") {
                        b'A'
                    } else {
                        b'a'
                    }) + (number % 26) as u8) as char,
                );
                number /= 26;
            }
            out
        }
        "lower-roman" | "upper-roman" if number <= 3999 => {
            let mut number = number;
            let mut out = String::new();
            for (value, text) in [
                (1000, "M"),
                (900, "CM"),
                (500, "D"),
                (400, "CD"),
                (100, "C"),
                (90, "XC"),
                (50, "L"),
                (40, "XL"),
                (10, "X"),
                (9, "IX"),
                (5, "V"),
                (4, "IV"),
                (1, "I"),
            ] {
                while number >= value {
                    number -= value;
                    out.push_str(text);
                }
            }
            if kind.starts_with("lower") {
                out.make_ascii_lowercase();
            }
            out
        }
        "decimal-leading-zero" => format!("{number:02}"),
        _ => number.to_string(),
    }
}
fn text(ctx: &mut Ctx, node: NodeId, style: &Style) -> String {
    let mut out = String::new();
    for value in style.get("content").into_iter().flatten() {
        match value {
            Value::Str(text) => out.push_str(text),
            Value::Func(name, args) if name == "attr" => {
                if let Some(Value::Ident(name)) = args.first() {
                    out.push_str(ctx.doc.attr(node, name).unwrap_or(""));
                }
            }
            Value::Func(name, args) if name == "counter" || name == "counters" => {
                let Some(Value::Ident(counter)) = args.first() else {
                    continue;
                };
                let stack = ctx.counters.values.get(counter);
                let kind = args
                    .iter()
                    .rev()
                    .find_map(|v| {
                        if let Value::Ident(name) = v {
                            (name != counter && name != ",").then_some(name.to_ascii_lowercase())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "decimal".into());
                if name == "counters" {
                    let separator = args
                        .iter()
                        .find_map(|v| {
                            if let Value::Str(s) = v {
                                Some(s.as_str())
                            } else {
                                None
                            }
                        })
                        .unwrap_or("");
                    out.push_str(
                        &stack
                            .map(|stack| {
                                stack
                                    .iter()
                                    .map(|(_, value)| format_number(*value, &kind))
                                    .collect::<Vec<_>>()
                                    .join(separator)
                            })
                            .unwrap_or_else(|| "0".into()),
                    );
                } else {
                    out.push_str(&format_number(
                        stack
                            .and_then(|stack| stack.last())
                            .map_or(0, |(_, value)| *value),
                        &kind,
                    ));
                }
            }
            Value::Ident(name)
                if matches!(
                    name.as_str(),
                    "open-quote" | "close-quote" | "no-open-quote" | "no-close-quote"
                ) =>
            {
                let open = name.ends_with("open-quote");
                if !open {
                    ctx.counters.quotes = ctx.counters.quotes.saturating_sub(1);
                }
                let quotes: Vec<_> = style
                    .get("quotes")
                    .into_iter()
                    .flatten()
                    .filter_map(|v| {
                        if let Value::Str(s) = v {
                            Some(s.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();
                if !name.starts_with("no-") && style.ident("quotes").as_deref() != Some("none") {
                    if quotes.len() >= 2 {
                        out.push_str(
                            quotes[(ctx.counters.quotes.min(quotes.len() / 2 - 1)) * 2
                                + usize::from(!open)],
                        );
                    } else {
                        out.push(if ctx.counters.quotes.is_multiple_of(2) {
                            if open {
                                '“'
                            } else {
                                '”'
                            }
                        } else if open {
                            '‘'
                        } else {
                            '’'
                        });
                    }
                }
                if open {
                    ctx.counters.quotes = ctx.counters.quotes.saturating_add(1);
                }
            }
            _ => {}
        }
    }
    out
}
pub(super) fn box_for(ctx: &mut Ctx, node: NodeId, style: &Style, name: &str) -> Option<LBox> {
    let style = Rc::new(style.pseudos.get(name)?.as_ref().clone());
    if style.display() == Display::None {
        return None;
    }
    enter(ctx, node, &style, true);
    let face = ctx.faces.push(make_face(ctx.input, Some(&style)));
    let mut content = LBox::new(Kind::Text);
    content.style = Some(style.clone());
    content.text = Some(TextRun {
        text: text(ctx, node, &style),
        face_idx: face,
    });
    let mut box_ = LBox::new(match style.display() {
        Display::Inline => Kind::Inline,
        Display::InlineBlock => Kind::InlineBlock,
        _ => Kind::Block,
    });
    box_.style = Some(style);
    box_.kids.push(content);
    Some(box_)
}
