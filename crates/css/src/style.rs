//! Cascade, computed style, selector matching against the DOM.

pub use crate::matching::{matches, matches_scoped};
use crate::parser::{Rule, Stylesheet};
use crate::values::Value;
use sol_dom::{Document, NodeId};
use std::collections::HashMap;
use std::rc::Rc;

/// Properties that inherit by default.
const INHERITED: &[&str] = &[
    "color",
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
    "font-variant",
    "letter-spacing",
    "line-height",
    "text-align",
    "text-indent",
    "text-transform",
    "visibility",
    "white-space",
    "word-spacing",
    "border-collapse",
    "border-spacing",
    "cursor",
    "pointer-events",
    "direction",
    "quotes",
    "list-style",
    "list-style-type",
    "list-style-position",
    "list-style-image",
    "orphans",
    "widows",
    "font",
    "-sol-font",
];

/// Interactive element state for pseudo-class matching, provided by kernel.
// Selector matching must remain usable without a window or event loop. This
// boundary lets static rendering use NoState while the kernel supplies live
// interaction, avoiding a dependency from the style engine back to Page.
pub trait PseudoState {
    fn is_hovered(&self, _n: NodeId) -> bool {
        false
    }
    fn is_active(&self, _n: NodeId) -> bool {
        false
    }
    fn is_focused(&self, _n: NodeId) -> bool {
        false
    }
    fn is_visited(&self, _href: &str) -> bool {
        false
    }
    fn is_target(&self, _n: NodeId) -> bool {
        false
    }
}

pub struct NoState;
impl PseudoState for NoState {}

#[derive(Clone, Debug, Default)]
pub struct Style {
    pub map: HashMap<String, Vec<Value>>,
    pub pseudos: HashMap<String, Box<Style>>,
}

impl Style {
    pub fn get(&self, name: &str) -> Option<&Vec<Value>> {
        self.map.get(name)
    }
    pub fn set(&mut self, name: &str, v: Vec<Value>) {
        self.map.insert(name.to_string(), v);
    }
    pub fn ident(&self, name: &str) -> Option<String> {
        match self.map.get(name)?.first()? {
            Value::Ident(s) | Value::Keyword(s) => Some(s.clone()),
            _ => None,
        }
    }
    pub fn color(&self, name: &str) -> Option<[u8; 4]> {
        match self.map.get(name)?.first()? {
            Value::Color(r, g, b, a) => Some([*r, *g, *b, *a]),
            _ => None,
        }
    }
    pub fn display(&self) -> Display {
        match self.ident("display").as_deref() {
            Some("none") => Display::None,
            Some("inline") => Display::Inline,
            Some("inline-block") => Display::InlineBlock,
            Some("block") | None => Display::Block,
            Some("inline-flex") => Display::InlineFlex,
            Some("flex") => Display::Flex,
            Some("table") => Display::Table,
            Some("inline-table") => Display::InlineTable,
            Some("table-row") => Display::TableRow,
            Some("table-cell") => Display::TableCell,
            Some("table-row-group") => Display::TableRowGroup,
            Some("table-header-group") => Display::TableHeaderGroup,
            Some("table-footer-group") => Display::TableFooterGroup,
            Some("table-column-group") => Display::TableColGroup,
            Some("table-column") => Display::TableCol,
            Some("table-caption") => Display::TableCaption,
            Some("list-item") => Display::ListItem,
            Some("grid") => Display::Grid,
            Some("inline-grid") => Display::InlineGrid,
            Some("contents") => Display::Contents,
            Some(_) => Display::Inline,
        }
    }
    pub fn position(&self) -> Position {
        match self.ident("position").as_deref() {
            Some("absolute") => Position::Absolute,
            Some("fixed") => Position::Fixed,
            Some("relative") => Position::Relative,
            _ => Position::Static,
        }
    }
    pub fn float(&self) -> Float {
        match self.ident("float").as_deref() {
            Some("left") => Float::Left,
            Some("right") => Float::Right,
            _ => Float::None,
        }
    }
    pub fn clear(&self) -> Clear {
        match self.ident("clear").as_deref() {
            Some("left") => Clear::Left,
            Some("right") => Clear::Right,
            Some("both") => Clear::Both,
            _ => Clear::None,
        }
    }
    pub fn visibility_hidden(&self) -> bool {
        self.ident("visibility").as_deref() == Some("hidden")
            || self.ident("visibility").as_deref() == Some("collapse")
    }
    /// Resolve a length value to px, given font size + pct base.
    pub fn length_px(&self, name: &str, font_size: f64, pct_base: f64) -> Option<f64> {
        let v = self.map.get(name)?.first()?;
        value_px(v, font_size, pct_base)
    }
    pub fn length_px_or(&self, name: &str, font_size: f64, pct_base: f64, def: f64) -> f64 {
        self.length_px(name, font_size, pct_base).unwrap_or(def)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    None,
    Inline,
    InlineBlock,
    Block,
    ListItem,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
    Table,
    InlineTable,
    TableRowGroup,
    TableHeaderGroup,
    TableFooterGroup,
    TableRow,
    TableCell,
    TableColGroup,
    TableCol,
    TableCaption,
    Contents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Float {
    None,
    Left,
    Right,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clear {
    None,
    Left,
    Right,
    Both,
}

/// Resolve a Value to px. pct_base for percentages, font_size for em/rem.
pub fn value_px(v: &Value, font_size: f64, pct_base: f64) -> Option<f64> {
    match v {
        Value::Math(name, values) => crate::math::length(name, values, font_size, pct_base),
        Value::Number(n) => Some(*n),
        Value::Percentage(p) => Some(p / 100.0 * pct_base),
        Value::Length(n, u) => Some(match u {
            crate::values::Unit::Px => *n,
            crate::values::Unit::Em => *n * font_size,
            crate::values::Unit::Rem => *n * viewport::ROOT_FONT_SIZE.get(),
            crate::values::Unit::Ex => *n * font_size * 0.5,
            crate::values::Unit::Ch => *n * font_size * 0.5,
            crate::values::Unit::Vw => *n * viewport::WIDTH.get() / 100.0,
            crate::values::Unit::Vh => *n * viewport::HEIGHT.get() / 100.0,
            crate::values::Unit::Vmin => {
                *n * viewport::WIDTH.get().min(viewport::HEIGHT.get()) / 100.0
            }
            crate::values::Unit::Vmax => {
                *n * viewport::WIDTH.get().max(viewport::HEIGHT.get()) / 100.0
            }
            crate::values::Unit::Cm => *n * 37.7953,
            crate::values::Unit::Mm => *n * 3.77953,
            crate::values::Unit::Q => *n * 0.944883,
            crate::values::Unit::In => *n * 96.0,
            crate::values::Unit::Pt => *n * 96.0 / 72.0,
            crate::values::Unit::Pc => *n * 16.0,
            crate::values::Unit::Pct => *n * pct_base / 100.0,
            _ => return None,
        }),
        _ => None,
    }
}

/// Collect all applicable declarations for an element, expanded,
/// cascade-sorted (ascending: later overrides), with `!important` flag
/// preserved for the final sort.
pub struct Origin {
    pub ua: u8, // 0=ua, 1=author, 2=author!important, 3=inline!important handled implicitly
}

#[derive(Clone)]
pub struct MatchedDecl {
    pub name: String,
    pub value: Vec<Value>,
    pub important: bool,
    pub specificity: (u32, u32, u32),
    pub order: usize,
    pub origin: u8,
    pub layer: usize,
}

pub fn match_element(
    doc: &Document,
    el: NodeId,
    sheets: &[&Stylesheet],
    state: &dyn PseudoState,
) -> Vec<MatchedDecl> {
    let layers = crate::layers::order(sheets);
    match_element_in_layers(doc, el, sheets, state, &layers)
}

fn match_element_in_layers(
    doc: &Document,
    el: NodeId,
    sheets: &[&Stylesheet],
    state: &dyn PseudoState,
    layers: &[String],
) -> Vec<MatchedDecl> {
    let mut collector = Collector::new(doc, el, state, layers, None);
    for (index, sheet) in sheets.iter().enumerate() {
        collector.origin = u8::from(index != 0);
        collector.collect(&sheet.rules, "");
    }
    let mut out = collector.out;
    let mut order = collector.order;
    // inline style: highest specificity-ish (author origin, specificity 1,0,0,0 wins over classes)
    if let Some(e) = doc.node(el).element() {
        if let Some(style_attr) = e.attr("style") {
            for d in crate::parser::parse_decls(style_attr) {
                for (name, value) in expand_decl(&d.name, d.value.clone()) {
                    out.push(MatchedDecl {
                        name,
                        value,
                        important: d.important,
                        specificity: (1, 0, 0), // between id-selector and head
                        order,
                        layer: usize::MAX,
                        origin: 4, // inline
                    });
                    order += 1;
                }
            }
        }
        // presentational attributes (html3 era) as lowest priority
        for (name, value) in presentational(doc, el) {
            out.push(MatchedDecl {
                name,
                value,
                important: false,
                specificity: (0, 0, 0),
                order,
                origin: 1,
                layer: usize::MAX,
            });
            order += 1;
        }
    }
    out
}

struct Collector<'a> {
    doc: &'a Document,
    el: NodeId,
    state: &'a dyn PseudoState,
    origin: u8,
    order: usize,
    out: Vec<MatchedDecl>,
    pseudo: Option<&'a str>,
    layers: &'a [String],
}
impl<'a> Collector<'a> {
    fn new(
        doc: &'a Document,
        el: NodeId,
        state: &'a dyn PseudoState,
        layers: &'a [String],
        pseudo: Option<&'a str>,
    ) -> Self {
        Self {
            doc,
            el,
            state,
            layers,
            pseudo,
            origin: 0,
            order: 0,
            out: Vec::new(),
        }
    }
    fn collect(&mut self, rules: &[Rule], layer_path: &str) {
        if layer_path.split('.').count() > 128 {
            return;
        }
        for r in rules {
            match r {
                Rule::Style(sr) => {
                    if let Some(specificity) = sr
                        .selectors
                        .iter()
                        .filter(|selector| {
                            matches_pseudo(self.doc, self.el, selector, self.state, self.pseudo)
                        })
                        .map(|selector| selector.specificity)
                        .max()
                    {
                        for declaration in &sr.decls {
                            for (name, value) in
                                expand_decl(&declaration.name, declaration.value.clone())
                            {
                                self.out.push(MatchedDecl {
                                    name,
                                    value,
                                    important: declaration.important,
                                    specificity,
                                    order: self.order,
                                    origin: self.origin,
                                    layer: self
                                        .layers
                                        .iter()
                                        .position(|path| path == layer_path)
                                        .unwrap_or(usize::MAX),
                                });
                                self.order += 1;
                            }
                        }
                    }
                }
                Rule::Media(m) => {
                    if crate::conditions::media_matches(&m.queries) {
                        self.collect(&m.rules, layer_path);
                    }
                }
                Rule::Supports(s) => {
                    if s.condition.supports() {
                        self.collect(&s.rules, layer_path);
                    }
                }
                Rule::Container(rule) => {
                    if crate::containers::matches(self.doc, self.el, rule) {
                        self.collect(&rule.rules, layer_path);
                    }
                }
                Rule::Layer(layer) => {
                    self.collect(&layer.rules, &crate::layers::path(layer_path, layer))
                }
                Rule::Property(_)
                | Rule::Keyframes(_)
                | Rule::LayerOrder(_)
                | Rule::Import(_)
                | Rule::FontFace(_)
                | Rule::Other(_) => {}
            }
        }
    }
}

fn matches_pseudo(
    doc: &Document,
    el: NodeId,
    selector: &crate::Selector,
    state: &dyn PseudoState,
    pseudo: Option<&str>,
) -> bool {
    let Some(pseudo) = pseudo else {
        return matches(doc, el, selector, state);
    };
    let mut selector = selector.clone();
    let Some(subject) = selector.parts.first_mut() else {
        return false;
    };
    let Some(index) = subject
        .pseudos
        .iter()
        .position(|p| matches!(p,crate::selector::Pseudo::PseudoElement(name) if name==pseudo))
    else {
        return false;
    };
    subject.pseudos.remove(index);
    matches(doc, el, &selector, state)
}

/// Evaluate media queries against the current viewport. The kernel sets
/// these once per render.
pub mod viewport {
    use std::cell::Cell;
    thread_local! {
        pub static ROOT_FONT_SIZE:Cell<f64>=const{Cell::new(16.0)};
        pub static WIDTH: Cell<f64> = const { Cell::new(800.0) };
        pub static HEIGHT: Cell<f64> = const { Cell::new(600.0) };
        pub static MEDIUM: std::cell::RefCell<String> = std::cell::RefCell::new("screen".to_string());
    }
}

fn presentational(doc: &Document, el: NodeId) -> Vec<(String, Vec<Value>)> {
    let mut out = Vec::new();
    let e = match doc.node(el).element() {
        Some(e) => e,
        None => return out,
    };
    let a = |n: &str| e.attr(n);
    match &*e.tag {
        "center" => out.push(("text-align".into(), vec![Value::Ident("center".into())])),
        "b" | "strong" => out.push(("font-weight".into(), vec![Value::Ident("bold".into())])),
        "i" | "em" => out.push(("font-style".into(), vec![Value::Ident("italic".into())])),
        "u" | "ins" => out.push((
            "text-decoration-line".into(),
            vec![Value::Ident("underline".into())],
        )),
        "s" | "strike" | "del" => out.push((
            "text-decoration-line".into(),
            vec![Value::Ident("line-through".into())],
        )),
        "tt" | "code" | "kbd" | "samp" => {
            out.push(("font-family".into(), vec![Value::Ident("monospace".into())]))
        }
        "font" => {
            if let Some(c) = a("color") {
                if let Some(v) = crate::values::value1(c) {
                    out.push(("color".into(), vec![v]));
                } else if let Some(c) = crate::values::named_color(&c.to_ascii_lowercase()) {
                    out.push(("color".into(), vec![Value::Color(c.0, c.1, c.2, c.3)]));
                }
            }
            if let Some(f) = a("face") {
                out.push(("font-family".into(), vec![Value::Str(f.to_string())]));
            }
            if let Some(sz) = a("size") {
                // size=3 == 16px, scale roughly
                let n: f64 = sz.parse().unwrap_or(3.0);
                let px = [10.0, 13.0, 16.0, 18.0, 24.0, 32.0, 48.0]
                    .get((n.clamp(1.0, 7.0) as usize) - 1)
                    .copied()
                    .unwrap_or(16.0);
                out.push((
                    "font-size".into(),
                    vec![Value::Length(px, crate::values::Unit::Px)],
                ));
            }
        }
        "img" => {
            if let Some(w) = a("width") {
                if let Ok(n) = w.parse::<f64>() {
                    out.push((
                        "width".into(),
                        vec![Value::Length(n, crate::values::Unit::Px)],
                    ));
                }
            }
            if let Some(h) = a("height") {
                if let Ok(n) = h.parse::<f64>() {
                    out.push((
                        "height".into(),
                        vec![Value::Length(n, crate::values::Unit::Px)],
                    ));
                }
            }
        }
        "table" => {
            if let Some(w) = a("width") {
                if let Ok(n) = w.parse::<f64>() {
                    out.push((
                        "width".into(),
                        vec![Value::Length(n, crate::values::Unit::Px)],
                    ));
                } else if w.ends_with('%') {
                    if let Ok(n) = w.trim_end_matches('%').parse::<f64>() {
                        out.push(("width".into(), vec![Value::Percentage(n)]));
                    }
                }
            }
            if let Some(c) = a("cellpadding") {
                if let Ok(n) = c.parse::<f64>() {
                    out.push((
                        "-sol-cellpadding".into(),
                        vec![Value::Length(n, crate::values::Unit::Px)],
                    ));
                }
            }
            if let Some(c) = a("cellspacing") {
                if let Ok(n) = c.parse::<f64>() {
                    out.push((
                        "border-spacing".into(),
                        vec![Value::Length(n, crate::values::Unit::Px)],
                    ));
                }
            }
            if let Some(b) = a("bgcolor") {
                if let Some(c) = crate::values::named_color(&b.to_ascii_lowercase()).or_else(|| {
                    crate::values::value1(&format!("#{b}")).and_then(|v| match v {
                        Value::Color(r, g, bl, al) => Some((r, g, bl, al)),
                        _ => None,
                    })
                }) {
                    out.push((
                        "background-color".into(),
                        vec![Value::Color(c.0, c.1, c.2, c.3)],
                    ));
                }
            }
        }
        "td" | "th" => {
            if let Some(w) = a("width") {
                if let Ok(n) = w.parse::<f64>() {
                    out.push((
                        "width".into(),
                        vec![Value::Length(n, crate::values::Unit::Px)],
                    ));
                }
            }
            if let Some(b) = a("bgcolor") {
                if let Some(c) = crate::values::named_color(&b.to_ascii_lowercase()).or_else(|| {
                    crate::values::value1(&format!("#{b}")).and_then(|v| match v {
                        Value::Color(r, g, bl, al) => Some((r, g, bl, al)),
                        _ => None,
                    })
                }) {
                    out.push((
                        "background-color".into(),
                        vec![Value::Color(c.0, c.1, c.2, c.3)],
                    ));
                }
            }
            if let Some(v) = a("valign") {
                out.push((
                    "vertical-align".into(),
                    vec![Value::Ident(v.to_ascii_lowercase())],
                ));
            }
            if let Some(al) = a("align") {
                out.push((
                    "text-align".into(),
                    vec![Value::Ident(al.to_ascii_lowercase())],
                ));
            }
        }
        "hr" => {
            if let Some(w) = a("width") {
                if let Ok(n) = w.parse::<f64>() {
                    out.push((
                        "width".into(),
                        vec![Value::Length(n, crate::values::Unit::Px)],
                    ));
                }
            }
            if let Some(c) = a("color") {
                if let Some(c) = crate::values::named_color(&c.to_ascii_lowercase()) {
                    out.push((
                        "background-color".into(),
                        vec![Value::Color(c.0, c.1, c.2, c.3)],
                    ));
                }
            }
        }
        "body" => {
            if let Some(b) = a("bgcolor") {
                let parsed = crate::values::value1(&b.to_ascii_lowercase())
                    .or_else(|| crate::values::value1(&format!("#{b}")));
                if let Some(Value::Color(r, g, bl, al)) = parsed {
                    out.push(("background-color".into(), vec![Value::Color(r, g, bl, al)]));
                }
            }
            if let Some(t) = a("text") {
                let parsed = crate::values::value1(&t.to_ascii_lowercase())
                    .or_else(|| crate::values::value1(&format!("#{t}")));
                if let Some(Value::Color(r, g, bl, al)) = parsed {
                    out.push(("color".into(), vec![Value::Color(r, g, bl, al)]));
                }
            }
        }
        "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "tr" => {
            if let Some(al) = a("align") {
                let a = al.to_ascii_lowercase();
                out.push(("text-align".into(), vec![Value::Ident(a)]));
            }
        }
        _ => {}
    }
    out
}

/// Expand shorthand properties into longhands.
pub fn expand_decl(name: &str, vals: Vec<Value>) -> Vec<(String, Vec<Value>)> {
    if crate::variables::deferred(&vals) {
        return vec![(name.to_string(), vals)];
    }
    if vals.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let n = name.to_ascii_lowercase();
    if matches!(vals.as_slice(),[Value::Ident(s)] if matches!(s.as_str(),"inherit"|"initial"|"unset"|"revert"|"revert-layer"))
    {
        let longhands: Vec<String> = match n.as_str() {
            "all" => KNOWN
                .iter()
                .filter(|name| !matches!(**name, "all" | "direction" | "unicode-bidi"))
                .map(|name| name.to_string())
                .collect(),
            "background" => [
                "color",
                "image",
                "position",
                "size",
                "repeat",
                "attachment",
                "origin",
                "clip",
            ]
            .map(|name| format!("background-{name}"))
            .to_vec(),
            "border" => ["top", "right", "bottom", "left"]
                .into_iter()
                .flat_map(|side| {
                    ["width", "style", "color"].map(|name| format!("border-{side}-{name}"))
                })
                .collect(),
            "border-top" | "border-right" | "border-bottom" | "border-left" | "outline" => {
                ["width", "style", "color"]
                    .map(|name| format!("{n}-{name}"))
                    .to_vec()
            }
            "font" => ["style", "variant", "weight", "stretch", "size", "family"]
                .map(|name| format!("font-{name}"))
                .into_iter()
                .chain(["line-height".into()])
                .collect(),
            "animation" => [
                "name",
                "duration",
                "timing-function",
                "delay",
                "iteration-count",
                "direction",
                "fill-mode",
                "play-state",
            ]
            .map(|name| format!("animation-{name}"))
            .to_vec(),
            "transition" => ["property", "duration", "delay", "timing-function"]
                .map(|name| format!("transition-{name}"))
                .to_vec(),
            "grid-area" => [
                "grid-row-start",
                "grid-column-start",
                "grid-row-end",
                "grid-column-end",
                "grid-area",
            ]
            .map(str::to_string)
            .to_vec(),
            "grid-column" | "grid-row" => [format!("{n}-start"), format!("{n}-end")].to_vec(),
            "container" => ["container-name".into(), "container-type".into()].to_vec(),
            _ => Vec::new(),
        };
        if !longhands.is_empty() {
            return longhands
                .into_iter()
                .map(|name| (name, vals.clone()))
                .collect();
        }
    }

    let quad = |i: usize| -> Option<Value> {
        if vals.len() > 4 {
            return None;
        }
        let index = match (i, vals.len()) {
            (2, 1 | 2) => 0,
            (3, 2 | 3) => 1,
            _ => i.min(vals.len() - 1),
        };
        vals.get(index).cloned()
    };
    match n.as_str() {
        "margin" | "padding" => {
            let sides = ["top", "right", "bottom", "left"];
            for (i, side) in sides.iter().enumerate() {
                let Some(value) = quad(i) else {
                    return Vec::new();
                };
                out.push((format!("{n}-{side}"), vec![value]));
            }
        }
        "border-width" | "border-style" | "border-color" | "border-radius" => {
            if n == "border-radius" {
                let slash = vals
                    .iter()
                    .position(|v| matches!(v, Value::Ident(s) if s == "/"));
                let horizontal = &vals[..slash.unwrap_or(vals.len())];
                let vertical = slash.map(|index| &vals[index + 1..]).unwrap_or(horizontal);
                let expand = |values: &[Value]| -> Option<[Value; 4]> {
                    Some(match values {
                        [a] => [a.clone(), a.clone(), a.clone(), a.clone()],
                        [a, b] => [a.clone(), b.clone(), a.clone(), b.clone()],
                        [a, b, c] => [a.clone(), b.clone(), c.clone(), b.clone()],
                        [a, b, c, d] => [a.clone(), b.clone(), c.clone(), d.clone()],
                        _ => return None,
                    })
                };
                if let (Some(horizontal), Some(vertical)) = (expand(horizontal), expand(vertical)) {
                    for (i, corner) in ["top-left", "top-right", "bottom-right", "bottom-left"]
                        .iter()
                        .enumerate()
                    {
                        out.push((
                            format!("border-{corner}-radius"),
                            vec![horizontal[i].clone(), vertical[i].clone()],
                        ));
                    }
                }
            } else {
                let sides = ["top", "right", "bottom", "left"];
                for (i, side) in sides.iter().enumerate() {
                    let Some(value) = quad(i) else {
                        return Vec::new();
                    };
                    let suffix = n.strip_prefix("border-").unwrap();
                    out.push((format!("border-{side}-{suffix}"), vec![value]));
                }
            }
        }
        "border" => {
            // width | style | color — any order
            let mut w = Some(Value::Ident("medium".into()));
            let mut s = Some(Value::Ident("none".into()));
            let mut c = Some(Value::Ident("currentcolor".into()));
            for v in &vals {
                match v {
                    Value::Length(_, _) | Value::Number(_) => w = Some(v.clone()),
                    Value::Color(_, _, _, _) => c = Some(v.clone()),
                    Value::Ident(id) | Value::Keyword(id) => match id.as_str() {
                        "none" | "solid" | "dotted" | "dashed" | "double" | "groove" | "ridge"
                        | "inset" | "outset" | "hidden" => s = Some(v.clone()),
                        "thin" | "medium" | "thick" => w = Some(v.clone()),
                        "currentcolor" => c = Some(v.clone()),
                        _ => {
                            if crate::values::named_color(id).is_some() {
                                c = Some(v.clone())
                            } else {
                                s = Some(v.clone())
                            }
                        }
                    },
                    _ => {}
                }
            }
            for side in ["top", "right", "bottom", "left"] {
                if let Some(v) = &w {
                    out.push((format!("border-{side}-width"), vec![v.clone()]));
                }
                if let Some(v) = &s {
                    out.push((format!("border-{side}-style"), vec![v.clone()]));
                }
                if let Some(v) = &c {
                    out.push((format!("border-{side}-color"), vec![v.clone()]));
                }
            }
        }
        s if s.starts_with("border-")
            && !s.ends_with("-width")
            && !s.ends_with("-style")
            && !s.ends_with("-color")
            && !s.ends_with("-radius") =>
        {
            // border-left: 1px solid red
            if let Some(side) = s.strip_prefix("border-") {
                if matches!(side, "top" | "right" | "bottom" | "left") {
                    for (k, v) in expand_decl("border", vals) {
                        if k.starts_with(&format!("border-{side}-")) {
                            out.push((k, v));
                        }
                    }
                } else {
                    out.push((n, vals));
                }
            } else {
                out.push((n, vals));
            }
        }
        "container" => {
            let slash = vals
                .iter()
                .position(|v| matches!(v,Value::Ident(s) if s=="/"));
            out.push((
                "container-name".into(),
                vals[..slash.unwrap_or(vals.len())].to_vec(),
            ));
            out.push((
                "container-type".into(),
                slash
                    .map(|index| {
                        vals[index + 1..]
                            .iter()
                            .map(|v| {
                                if let Value::Ident(s) = v {
                                    Value::Ident(s.to_ascii_lowercase())
                                } else {
                                    v.clone()
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_else(|| vec![Value::Ident("normal".into())]),
            ));
        }
        "animation" => out.extend(crate::animations::expand(&vals)),
        "transition" => out.extend(crate::transitions::expand(&vals)),
        "grid-area" => {
            let groups: Vec<_> = vals
                .split(|v| matches!(v,Value::Ident(s) if s=="/"))
                .collect();
            if groups.len() == 1 && matches!(groups[0], [Value::Ident(_)]) {
                out.push(("grid-area".into(), vals.clone()));
            }
            for (index, name) in [
                "grid-row-start",
                "grid-column-start",
                "grid-row-end",
                "grid-column-end",
            ]
            .into_iter()
            .enumerate()
            {
                let value = groups
                    .get(index)
                    .copied()
                    .or_else(|| {
                        if matches!(groups[0],[Value::Ident(name)] if name!="auto"&&name!="span") {
                            Some(groups[0])
                        } else {
                            None
                        }
                    })
                    .map(<[Value]>::to_vec)
                    .unwrap_or_else(|| vec![Value::Ident("auto".into())]);
                out.push((name.into(), value));
            }
        }
        "grid-column" | "grid-row" => {
            let split = vals
                .iter()
                .position(|value| matches!(value, Value::Ident(s) if s == "/"));
            let start = vals[..split.unwrap_or(vals.len())].to_vec();
            let end = split
                .map(|index| vals[index + 1..].to_vec())
                .unwrap_or_else(|| vec![Value::Ident("auto".into())]);
            out.push((format!("{n}-start"), start));
            out.push((format!("{n}-end"), end));
        }
        "place-items" | "place-self" | "place-content" => {
            let suffix = n.strip_prefix("place-").unwrap();
            out.push((format!("align-{suffix}"), vec![vals[0].clone()]));
            out.push((
                format!("justify-{suffix}"),
                vec![vals.get(1).unwrap_or(&vals[0]).clone()],
            ));
        }
        "outline" => {
            for (name, value) in expand_decl("border", vals) {
                if let Some(suffix) = name.strip_prefix("border-top-") {
                    out.push((format!("outline-{suffix}"), value));
                }
            }
        }
        "background" => out.extend(crate::background::expand(&vals)),
        "font" => {
            // [style? variant? weight?] size[/line-height] family
            let mut saw_size = false;
            for v in &vals {
                match v {
                    Value::Ident(id) | Value::Keyword(id) => match id.as_str() {
                        "italic" | "oblique" | "normal" if !saw_size => {
                            if id == "italic" || id == "oblique" {
                                out.push(("font-style".into(), vec![v.clone()]));
                            }
                        }
                        "bold" | "bolder" | "lighter" => {
                            out.push(("font-weight".into(), vec![v.clone()]))
                        }
                        "small-caps" => out.push(("font-variant".into(), vec![v.clone()])),
                        "/" => saw_size = true,
                        _ => out.push(("font-family".into(), vec![Value::Ident(id.clone())])),
                    },
                    Value::Number(nv) => {
                        if !saw_size {
                            out.push(("font-weight".into(), vec![Value::Number(*nv)]));
                        } else {
                            out.push(("line-height".into(), vec![Value::Number(*nv)]));
                        }
                    }
                    Value::Length(_, _) | Value::Percentage(_) => {
                        if saw_size {
                            out.push(("line-height".into(), vec![v.clone()]));
                        } else {
                            out.push(("font-size".into(), vec![v.clone()]));
                            saw_size = true;
                        }
                    }
                    Value::Str(s2) => {
                        out.push(("font-family".into(), vec![Value::Str(s2.clone())]))
                    }
                    _ => {}
                }
            }
        }
        "flex" => {
            // flex: [grow] [shrink] [basis]
            let mut nums = Vec::new();
            let mut basis = None;
            for v in &vals {
                match v {
                    Value::Number(x) => nums.push(*x),
                    Value::Ident(id) if id == "none" || id == "auto" => basis = Some(id.clone()),
                    _ => basis = Some(format!("{v:?}")),
                }
            }
            out.push((
                "flex-grow".into(),
                vec![Value::Number(
                    nums.first().copied().unwrap_or(
                        if vals
                            .iter()
                            .any(|v| matches!(v, Value::Ident(i) if i=="none"))
                        {
                            0.0
                        } else {
                            1.0
                        },
                    ),
                )],
            ));
            out.push((
                "flex-shrink".into(),
                vec![Value::Number(nums.get(1).copied().unwrap_or(1.0))],
            ));
            if let Some(b) = basis {
                if let Ok(x) = b.parse::<f64>() {
                    out.push((
                        "flex-basis".into(),
                        vec![Value::Length(x, crate::values::Unit::Px)],
                    ));
                } else {
                    out.push(("flex-basis".into(), vec![Value::Ident(b)]));
                }
            } else if let Some(x) = nums.get(2) {
                out.push(("flex-basis".into(), vec![Value::Number(*x)]));
            }
        }
        "gap" => {
            out.push((
                "row-gap".into(),
                vec![vals.first().cloned().unwrap_or(Value::Number(0.0))],
            ));
            out.push((
                "column-gap".into(),
                vec![vals
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| vals.first().cloned().unwrap_or(Value::Number(0.0)))],
            ));
        }
        "overflow" => {
            out.push((
                "overflow-x".into(),
                vec![vals
                    .first()
                    .cloned()
                    .unwrap_or(Value::Ident("visible".into()))],
            ));
            out.push((
                "overflow-y".into(),
                vec![vals.get(1).cloned().unwrap_or_else(|| {
                    vals.first()
                        .cloned()
                        .unwrap_or(Value::Ident("visible".into()))
                })],
            ));
        }
        "inset" => {
            let sides = ["top", "right", "bottom", "left"];
            for (i, side) in sides.iter().enumerate() {
                let v = vals
                    .get(i.min(vals.len() - 1))
                    .cloned()
                    .unwrap_or(Value::Number(0.0));
                out.push((side.to_string(), vec![v]));
            }
        }
        "text-decoration" => {
            out.push(("text-decoration-line".into(), vals));
        }
        "list-style" => {
            for v in &vals {
                match v {
                    Value::Ident(id) => match id.as_str() {
                        "inside" | "outside" => {
                            out.push(("list-style-position".into(), vec![v.clone()]))
                        }
                        "none" => {
                            out.push(("list-style-type".into(), vec![v.clone()]));
                            out.push(("list-style-image".into(), vec![v.clone()]));
                        }
                        _ => out.push(("list-style-type".into(), vec![v.clone()])),
                    },
                    Value::Url(u) => {
                        out.push(("list-style-image".into(), vec![Value::Url(u.clone())]))
                    }
                    _ => {}
                }
            }
        }
        _ => out.push((n, vals)),
    }
    out
}

/// Final computed style for an element.
pub fn compute(
    _doc: &Document,
    _el: NodeId,
    matched: Vec<MatchedDecl>,
    parent: Option<&Style>,
) -> Style {
    compute_registered(_doc, _el, matched, parent, &HashMap::new())
}
fn compute_registered(
    _doc: &Document,
    _el: NodeId,
    matched: Vec<MatchedDecl>,
    parent: Option<&Style>,
    properties: &HashMap<String, crate::properties::Registration>,
) -> Style {
    let mut sorted = matched;
    // stable sort: (important?, origin, specificity, order) ascending;
    // last wins — important beats normal
    sorted.sort_by_key(|d| {
        (
            d.important as u8,
            if d.important {
                match d.origin {
                    0 => 3,
                    4 => 2,
                    _ => 1,
                }
            } else {
                d.origin
            },
            if d.important {
                usize::MAX - d.layer
            } else {
                d.layer
            },
            d.specificity.0,
            d.specificity.1,
            d.specificity.2,
            d.order,
        )
    });
    // Rollback keywords remove the entire current layer/origin from the
    // candidate list; treating them as initial would lose earlier layer values.
    let origin = |origin: u8| if origin == 4 { 1 } else { origin };
    let mut excluded = std::collections::HashSet::new();
    let mut origins = std::collections::HashSet::new();
    let mut retained = Vec::new();
    for declaration in sorted.into_iter().rev() {
        if excluded.contains(&(
            declaration.name.clone(),
            declaration.origin,
            declaration.layer,
        )) || origins.contains(&(declaration.name.clone(), origin(declaration.origin)))
        {
            continue;
        }
        let keyword = match declaration.value.as_slice() {
            [Value::Ident(s)] => Some(s.as_str()),
            [Value::Components(values)] => match crate::syntax::trim(values) {
                [crate::syntax::ComponentValue::Token(token)] => {
                    if let crate::syntax::TokenKind::Ident(name) = &token.kind {
                        Some(name.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        };
        match keyword {
            Some("revert-layer") => {
                excluded.insert((declaration.name, declaration.origin, declaration.layer));
            }
            Some("revert") => {
                origins.insert((declaration.name, origin(declaration.origin)));
            }
            _ => retained.push(declaration),
        }
    }
    retained.reverse();
    let sorted = retained;
    let mut source = HashMap::new();
    for (name, property) in properties {
        if let Some(initial) = &property.initial {
            source.insert(name.clone(), initial.clone());
        }
    }
    if let Some(parent) = parent {
        for (name, value) in &parent.map {
            if name.starts_with("--")
                && properties
                    .get(name)
                    .is_none_or(|property| property.inherits)
            {
                if let [Value::Components(values)] = value.as_slice() {
                    source.insert(name.clone(), values.clone());
                }
            }
        }
    }
    let fallbacks = source
        .iter()
        .filter(|(name, _)| properties.contains_key(*name))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    for decl in &sorted {
        if !decl.name.starts_with("--") {
            continue;
        }
        if let [Value::Components(value)] = decl.value.as_slice() {
            let keyword = match crate::syntax::trim(value) {
                [crate::syntax::ComponentValue::Token(token)] => {
                    if let crate::syntax::TokenKind::Ident(name) = &token.kind {
                        Some(name.to_ascii_lowercase())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            match keyword.as_deref() {
                Some("initial") => {
                    source.remove(&decl.name);
                    if let Some(initial) = properties
                        .get(&decl.name)
                        .and_then(|property| property.initial.as_ref())
                    {
                        source.insert(decl.name.clone(), initial.clone());
                    }
                }
                Some("inherit" | "unset" | "revert" | "revert-layer") => {
                    source.remove(&decl.name);
                    if keyword.as_deref() == Some("unset")
                        && properties
                            .get(&decl.name)
                            .is_some_and(|property| !property.inherits)
                    {
                        if let Some(initial) = properties[&decl.name].initial.as_ref() {
                            source.insert(decl.name.clone(), initial.clone());
                        }
                        continue;
                    }
                    if let Some([Value::Components(value)]) = parent
                        .and_then(|p| p.map.get(&decl.name))
                        .map(Vec::as_slice)
                    {
                        source.insert(decl.name.clone(), value.clone());
                    }
                }
                _ => {
                    source.insert(decl.name.clone(), value.clone());
                }
            }
        }
    }
    let mut variables = crate::variables::Variables::new(source, properties, fallbacks);
    let mut map = variables.computed();
    for decl in sorted {
        if decl.name.starts_with("--") {
            continue;
        }
        let deferred = crate::variables::deferred(&decl.value);
        let value = if let [Value::Components(values)] = decl.value.as_slice() {
            variables
                .substitute(values, 0)
                .map(|values| crate::values::from_components(&values))
                .filter(|value| crate::conditions::valid_computed_value(&decl.name, value))
                .unwrap_or_else(|| vec![Value::Ident("unset".into())])
        } else {
            decl.value
        };
        let expanded = if deferred {
            expand_decl(&decl.name, value)
        } else {
            vec![(decl.name, value)]
        };
        for (name, value) in expanded {
            map.insert(name, value);
        }
    }
    // resolve inherit/initial/unset + inherit-by-default
    let mut st = Style {
        map,
        pseudos: HashMap::new(),
    };
    let keys: Vec<String> = st.map.keys().cloned().collect();
    for k in keys {
        let vals = st.map.get(&k).unwrap().clone();
        let first = vals.first();
        let needs = match first {
            Some(Value::Ident(id)) => {
                matches!(id.as_str(), "inherit" | "initial" | "unset" | "revert")
            }
            _ => false,
        };
        if needs {
            let id = match first {
                Some(Value::Ident(id)) => id.as_str(),
                _ => unreachable!(),
            };
            match id {
                "inherit" => {
                    st.map.remove(&k);
                    inherit_prop(&mut st, &k, parent);
                }
                "unset" => {
                    st.map.remove(&k);
                    if INHERITED.contains(&k.as_str()) {
                        inherit_prop(&mut st, &k, parent);
                    }
                }
                _ => {
                    st.map.remove(&k);
                }
            }
        }
    }
    // inherit-by-default for inherited props not explicitly set
    for &k in INHERITED {
        if !st.map.contains_key(k) {
            inherit_prop(&mut st, k, parent);
        }
    }
    if let Some(value) = st
        .get("font-size")
        .and_then(|values| values.first())
        .cloned()
    {
        let parent_size = parent
            .and_then(|parent| parent.length_px("font-size", 16.0, 16.0))
            .unwrap_or(16.0);
        let size = match &value {
            Value::Ident(name) => match name.as_str() {
                "xx-small" => Some(9.0),
                "x-small" => Some(10.0),
                "small" => Some(13.0),
                "medium" => Some(16.0),
                "large" => Some(18.0),
                "x-large" => Some(24.0),
                "xx-large" => Some(32.0),
                "larger" => Some(parent_size * 1.2),
                "smaller" => Some(parent_size / 1.2),
                _ => None,
            },
            _ => value_px(&value, parent_size, parent_size),
        };
        if let Some(size) = size.filter(|size| size.is_finite()) {
            st.set(
                "font-size",
                vec![Value::Length(size.max(0.0), crate::values::Unit::Px)],
            );
        }
    }
    st
}

/// Compute styles for every element in the document.
/// `sheets[0]` should be the UA sheet (origin 0); later sheets are author.
/// Returns a map NodeId -> computed Style in tree order.
pub fn style_document(
    doc: &Document,
    sheets: &[&Stylesheet],
    state: &dyn PseudoState,
) -> HashMap<NodeId, Rc<Style>> {
    style_document_with_timeline(
        doc,
        sheets,
        state,
        &mut crate::animations::Timeline::default(),
    )
}

pub fn style_document_with_timeline(
    doc: &Document,
    sheets: &[&Stylesheet],
    state: &dyn PseudoState,
    timeline: &mut crate::animations::Timeline,
) -> HashMap<NodeId, Rc<Style>> {
    viewport::ROOT_FONT_SIZE.set(16.0);
    let layers = crate::layers::order(sheets);
    let frames = crate::animations::registry(sheets);
    let properties = crate::properties::registry(sheets);
    let mut context = StyleContext {
        doc,
        sheets,
        state,
        layers,
        frames,
        properties,
        timeline,
    };
    context.timeline.active = false;
    let mut out = HashMap::new();
    context.visit(doc.root, None, &mut out);
    context.timeline.retain(&out);
    out
}
struct StyleContext<'a, 'b> {
    doc: &'a Document,
    sheets: &'a [&'a Stylesheet],
    state: &'a dyn PseudoState,
    layers: Vec<String>,
    frames: HashMap<String, &'a crate::parser::KeyframesRule>,
    timeline: &'b mut crate::animations::Timeline,
    properties: HashMap<String, crate::properties::Registration>,
}
impl StyleContext<'_, '_> {
    fn pseudo_style(&self, el: NodeId, parent: &Style, name: &str) -> Option<Style> {
        let (doc, sheets, state, layers) =
            (self.doc, self.sheets, self.state, self.layers.as_slice());
        let mut collector = Collector::new(doc, el, state, layers, Some(name));
        for (index, sheet) in sheets.iter().enumerate() {
            collector.origin = u8::from(index != 0);
            collector.collect(&sheet.rules, "");
        }
        let matched = collector.out;
        if matched.is_empty() {
            return None;
        }
        let mut style = compute_registered(doc, el, matched, Some(parent), &self.properties);
        if !style.map.contains_key("display") {
            style.set("display", vec![Value::Ident("inline".into())]);
        }
        if style.get("content").is_none_or(
            |values| matches!(values.as_slice(),[Value::Ident(s)] if s=="none"||s=="normal"),
        ) {
            return None;
        }
        Some(style)
    }

    fn visit(
        &mut self,
        node: NodeId,
        parent: Option<Rc<Style>>,
        out: &mut HashMap<NodeId, Rc<Style>>,
    ) {
        for &child in self.doc.children(node) {
            if self.doc.node(child).is_element() {
                let matched =
                    match_element_in_layers(self.doc, child, self.sheets, self.state, &self.layers);
                let important = matched
                    .iter()
                    .filter(|d| d.important)
                    .map(|d| d.name.clone())
                    .collect();
                let mut style = compute_registered(
                    self.doc,
                    child,
                    matched,
                    parent.as_deref(),
                    &self.properties,
                );
                if Some(child) == self.doc.document_element() {
                    viewport::ROOT_FONT_SIZE
                        .set(style.length_px("font-size", 16.0, 16.0).unwrap_or(16.0));
                }
                let base = style.clone();
                self.timeline
                    .apply(self.doc, child, &mut style, &self.frames, &important);
                self.timeline.active |=
                    self.timeline
                        .transitions
                        .apply(child, &base, &mut style, self.timeline.time);
                for name in ["before", "after", "marker"] {
                    if let Some(pseudo) = self.pseudo_style(child, &style, name) {
                        style.pseudos.insert(name.into(), Box::new(pseudo));
                    }
                }
                let style = Rc::new(style);
                out.insert(child, style.clone());
                self.visit(child, Some(style), out);
            } else {
                self.visit(child, parent.clone(), out);
            }
        }
    }
}

fn inherit_prop(st: &mut Style, k: &str, parent: Option<&Style>) {
    if let Some(p) = parent {
        if let Some(v) = p.map.get(k) {
            st.map.insert(k.to_string(), v.clone());
        }
    }
}

/// Is `prop` a property sol understands? Used for @supports.
pub fn is_known_property(prop: &str) -> bool {
    KNOWN.binary_search(&prop).is_ok()
}

const KNOWN: &[&str] = &[
    "align-content",
    "align-items",
    "align-self",
    "all",
    "animation",
    "animation-delay",
    "animation-direction",
    "animation-duration",
    "animation-fill-mode",
    "animation-iteration-count",
    "animation-name",
    "animation-play-state",
    "animation-timing-function",
    "aspect-ratio",
    "background",
    "background-attachment",
    "background-clip",
    "background-color",
    "background-image",
    "background-origin",
    "background-position",
    "background-repeat",
    "background-size",
    "border",
    "border-bottom",
    "border-bottom-color",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    "border-bottom-style",
    "border-bottom-width",
    "border-collapse",
    "border-color",
    "border-left",
    "border-left-color",
    "border-left-style",
    "border-left-width",
    "border-radius",
    "border-right",
    "border-right-color",
    "border-right-style",
    "border-right-width",
    "border-spacing",
    "border-style",
    "border-top",
    "border-top-color",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-top-style",
    "border-top-width",
    "border-width",
    "bottom",
    "box-shadow",
    "box-sizing",
    "caption-side",
    "clear",
    "color",
    "column-gap",
    "container",
    "container-name",
    "container-type",
    "content",
    "counter-increment",
    "counter-reset",
    "counter-set",
    "cursor",
    "direction",
    "display",
    "empty-cells",
    "flex",
    "flex-basis",
    "flex-direction",
    "flex-flow",
    "flex-grow",
    "flex-shrink",
    "flex-wrap",
    "float",
    "font",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-weight",
    "gap",
    "grid",
    "grid-area",
    "grid-auto-columns",
    "grid-auto-flow",
    "grid-auto-rows",
    "grid-column",
    "grid-column-end",
    "grid-column-start",
    "grid-row",
    "grid-row-end",
    "grid-row-start",
    "grid-template",
    "grid-template-areas",
    "grid-template-columns",
    "grid-template-rows",
    "height",
    "isolation",
    "justify-content",
    "justify-items",
    "justify-self",
    "left",
    "letter-spacing",
    "line-height",
    "list-style",
    "list-style-image",
    "list-style-position",
    "list-style-type",
    "margin",
    "margin-bottom",
    "margin-left",
    "margin-right",
    "margin-top",
    "max-height",
    "max-width",
    "min-height",
    "min-width",
    "object-fit",
    "object-position",
    "opacity",
    "order",
    "outline",
    "outline-color",
    "outline-offset",
    "outline-style",
    "outline-width",
    "overflow",
    "overflow-x",
    "overflow-y",
    "padding",
    "padding-bottom",
    "padding-left",
    "padding-right",
    "padding-top",
    "place-content",
    "place-items",
    "place-self",
    "pointer-events",
    "position",
    "right",
    "row-gap",
    "table-layout",
    "text-align",
    "text-decoration",
    "text-decoration-line",
    "text-indent",
    "text-overflow",
    "text-shadow",
    "text-transform",
    "top",
    "transform",
    "transform-origin",
    "transition",
    "transition-delay",
    "transition-duration",
    "transition-property",
    "transition-timing-function",
    "vertical-align",
    "visibility",
    "white-space",
    "width",
    "word-break",
    "word-spacing",
    "z-index",
];
