//! Property interpretation follows balanced component parsing so source strings
//! cannot terminate a declaration, selector, or at-rule prematurely.
use crate::syntax::{self, ComponentValue as C, TokenKind as T};
use crate::values::Value;

#[derive(Debug, Clone)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}
#[derive(Debug, Clone)]
pub enum Rule {
    Style(StyleRule),
    Media(MediaRule),
    Supports(SupportsRule),
    Layer(LayerRule),
    LayerOrder(Vec<String>),
    Keyframes(KeyframesRule),
    Container(ContainerRule),
    Property(crate::properties::Registration),
    FontFace(Vec<Decl>),
    Import(String),
    Other(String),
}
#[derive(Debug, Clone)]
pub struct MediaRule {
    pub queries: Vec<MediaQuery>,
    pub rules: Vec<Rule>,
}
#[derive(Debug, Clone)]
pub struct MediaQuery {
    pub medium: Option<String>,
    pub features: Vec<(String, String)>,
    pub condition: crate::conditions::Condition,
    pub negated: bool,
}
#[derive(Debug, Clone)]
pub struct SupportsRule {
    pub condition: crate::conditions::Condition,
    pub rules: Vec<Rule>,
}
#[derive(Debug, Clone)]
pub struct LayerRule {
    pub name: Option<String>,
    pub rules: Vec<Rule>,
}
#[derive(Debug, Clone)]
pub struct Keyframe {
    pub offset: f64,
    pub decls: std::rc::Rc<Vec<Decl>>,
}
#[derive(Debug, Clone)]
pub struct KeyframesRule {
    pub name: String,
    pub frames: Vec<Keyframe>,
}
#[derive(Debug, Clone)]
pub struct ContainerRule {
    pub name: Option<String>,
    pub condition: crate::conditions::Condition,
    pub rules: Vec<Rule>,
}
#[derive(Debug, Clone)]
pub struct StyleRule {
    pub selectors: Vec<crate::selector::Selector>,
    pub decls: Vec<Decl>,
}
#[derive(Debug, Clone)]
pub struct Decl {
    pub name: String,
    pub value: Vec<Value>,
    pub important: bool,
}

pub fn parse(css: &str) -> Stylesheet {
    let stylesheet = syntax::parse_stylesheet(css);
    let namespaces = crate::namespaces::declarations(&stylesheet.rules);
    let mut budget = syntax::MAX_TOKENS;
    let mut rules = convert_rules(stylesheet.rules, &mut budget);
    crate::namespaces::apply(&mut rules, &namespaces);
    Stylesheet { rules }
}

fn convert_rules(rules: Vec<syntax::Rule>, budget: &mut usize) -> Vec<Rule> {
    rules
        .into_iter()
        .flat_map(|rule| convert_rule(rule, None, budget))
        .collect()
}
fn style_block(prelude: Vec<C>, block: Vec<C>, budget: &mut usize) -> Vec<Rule> {
    let selectors = crate::selector::parse_selector_list(&syntax::serialize(&prelude));
    if selectors.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut pending = Vec::new();
    for item in syntax::declaration::block_contents(&block, &mut Vec::new()) {
        match item {
            syntax::BlockItem::Declaration(decl) => {
                if !decl.value.is_empty() || decl.name.starts_with("--") {
                    pending.push(Decl {
                        value: crate::variables::declaration_value(&decl.name, &decl.value),
                        name: decl.name,
                        important: decl.important,
                    });
                }
            }
            syntax::BlockItem::Rule(rule) => {
                if !pending.is_empty() {
                    out.push(Rule::Style(StyleRule {
                        selectors: selectors.clone(),
                        decls: std::mem::take(&mut pending),
                    }));
                }
                out.extend(convert_rule(rule, Some(&prelude), budget));
            }
        }
    }
    if !pending.is_empty() {
        out.push(Rule::Style(StyleRule {
            selectors,
            decls: pending,
        }));
    }
    out
}
fn convert_rule(rule: syntax::Rule, parent: Option<&[C]>, budget: &mut usize) -> Vec<Rule> {
    let preserved = rule.to_css();
    match rule {
        syntax::Rule::Qualified(rule) => {
            let prelude = parent
                .map(|parent| crate::nesting::selector(&rule.prelude, parent, budget))
                .unwrap_or(rule.prelude);
            style_block(prelude, rule.block, budget)
        }
        syntax::Rule::At(rule) => {
            let name = rule.name.to_ascii_lowercase();
            let mut children = |block| {
                if let Some(parent) = parent {
                    style_block(parent.to_vec(), block, budget)
                } else {
                    convert_rules(syntax::into_rule_list(block, false), budget)
                }
            };
            let converted = match (name.as_str(), rule.block) {
                ("media", Some(block)) => Rule::Media(MediaRule {
                    queries: crate::conditions::parse_media(&rule.prelude),
                    rules: children(block),
                }),
                ("supports", Some(block)) => Rule::Supports(SupportsRule {
                    condition: crate::conditions::parse(&rule.prelude),
                    rules: children(block),
                }),
                ("layer", Some(block)) => Rule::Layer(LayerRule {
                    name: layer_name(&rule.prelude),
                    rules: children(block),
                }),
                ("layer", None) => Rule::LayerOrder(
                    syntax::split(&rule.prelude, &T::Comma)
                        .into_iter()
                        .filter_map(layer_name)
                        .collect(),
                ),
                ("property", Some(block)) => crate::properties::parse(&rule.prelude, &block)
                    .map(Rule::Property)
                    .unwrap_or(Rule::Other(preserved)),
                ("container", Some(block)) => {
                    let values = syntax::trim(&rule.prelude);
                    let (name, values) = match values.first().and_then(C::kind) {
                        Some(T::Ident(name)) if !name.eq_ignore_ascii_case("not") => {
                            (Some(name.clone()), syntax::trim(&values[1..]))
                        }
                        _ => (None, values),
                    };
                    Rule::Container(ContainerRule {
                        name,
                        condition: crate::conditions::parse(values),
                        rules: children(block),
                    })
                }
                ("keyframes" | "-webkit-keyframes", Some(block)) => {
                    let Some(C::Token(token)) = syntax::trim(&rule.prelude).first() else {
                        return Vec::new();
                    };
                    let name = match &token.kind {
                        T::Ident(name) | T::String(name) => name.clone(),
                        _ => return Vec::new(),
                    };
                    let mut frames = Vec::new();
                    for rule in syntax::into_rule_list(block, false) {
                        if let syntax::Rule::Qualified(rule) = rule {
                            let decls = std::rc::Rc::new(
                                declarations(&rule.block)
                                    .into_iter()
                                    .filter(|d| !d.important)
                                    .collect::<Vec<_>>(),
                            );
                            for prelude in syntax::split(&rule.prelude, &T::Comma) {
                                let offset = match syntax::trim(prelude) {
                                    [C::Token(token)] => match &token.kind {
                                        T::Percentage(n) => Some(n.value / 100.0),
                                        T::Ident(s) if s.eq_ignore_ascii_case("from") => Some(0.0),
                                        T::Ident(s) if s.eq_ignore_ascii_case("to") => Some(1.0),
                                        _ => None,
                                    },
                                    _ => None,
                                };
                                if let Some(offset) = offset.filter(|n| (0.0..=1.0).contains(n)) {
                                    if frames.len() < 1024 {
                                        frames.push(Keyframe {
                                            offset,
                                            decls: decls.clone(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    frames.sort_by(|a, b| a.offset.total_cmp(&b.offset));
                    Rule::Keyframes(KeyframesRule { name, frames })
                }
                ("font-face", Some(block)) if syntax::trim(&rule.prelude).is_empty() => {
                    Rule::FontFace(declarations(&block))
                }
                ("import", None) => import_url(syntax::trim(&rule.prelude).first())
                    .map(Rule::Import)
                    .unwrap_or(Rule::Other(preserved)),
                _ => Rule::Other(preserved),
            };
            vec![converted]
        }
    }
}
fn layer_name(values: &[C]) -> Option<String> {
    let values = syntax::trim(values);
    if values.is_empty() {
        return None;
    }
    let mut name = String::new();
    let mut expect_ident = true;
    for value in values {
        match value.kind()? {
            T::Ident(part) if expect_ident => {
                name.push_str(part);
                expect_ident = false;
            }
            T::Delim('.') if !expect_ident => {
                name.push('.');
                expect_ident = true;
            }
            _ => return None,
        }
    }
    (!expect_ident).then_some(name)
}
pub fn import_url(value: Option<&C>) -> Option<String> {
    match value? {
        C::Token(token) => match &token.kind {
            T::Url(url) | T::String(url) => Some(url.clone()),
            _ => None,
        },
        C::Function { name, value } if name.eq_ignore_ascii_case("url") => {
            match syntax::trim(value) {
                [C::Token(token)] => {
                    if let T::String(url) = &token.kind {
                        Some(url.clone())
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

pub fn parse_decls(css: &str) -> Vec<Decl> {
    declarations(&syntax::parse_component_values(css).values)
}

pub(crate) fn declaration(values: &[C]) -> Option<Decl> {
    let decl = syntax::declaration::declaration(values)?;
    if decl.value.is_empty() && !decl.name.starts_with("--") {
        return None;
    }
    Some(Decl {
        value: crate::variables::declaration_value(&decl.name, &decl.value),
        name: decl.name,
        important: decl.important,
    })
}

pub(crate) fn declarations(values: &[C]) -> Vec<Decl> {
    syntax::declaration::block_contents(values, &mut Vec::new())
        .into_iter()
        .filter_map(|item| match item {
            syntax::BlockItem::Declaration(decl)
                if !decl.value.is_empty() || decl.name.starts_with("--") =>
            {
                Some(Decl {
                    value: crate::variables::declaration_value(&decl.name, &decl.value),
                    name: decl.name,
                    important: decl.important,
                })
            }
            _ => None,
        })
        .collect()
}
