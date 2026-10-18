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
    Stylesheet {
        rules: convert_rules(stylesheet.rules),
    }
}

fn convert_rules(rules: Vec<syntax::Rule>) -> Vec<Rule> {
    rules
        .into_iter()
        .filter_map(|rule| {
            let preserved = rule.to_css();
            match rule {
                syntax::Rule::Qualified(rule) => {
                    let selectors =
                        crate::selector::parse_selector_list(&syntax::serialize(&rule.prelude));
                    if selectors.is_empty() {
                        return None;
                    }
                    Some(Rule::Style(StyleRule {
                        selectors,
                        decls: declarations(&rule.block),
                    }))
                }
                syntax::Rule::At(rule) => {
                    let name = rule.name.to_ascii_lowercase();
                    match (name.as_str(), rule.block) {
                        ("media", Some(block)) => Some(Rule::Media(MediaRule {
                            queries: crate::conditions::parse_media(&rule.prelude),
                            rules: convert_rules(syntax::into_rule_list(block, false)),
                        })),
                        ("supports", Some(block)) => Some(Rule::Supports(SupportsRule {
                            condition: crate::conditions::parse(&rule.prelude),
                            rules: convert_rules(syntax::into_rule_list(block, false)),
                        })),
                        ("font-face", Some(block)) if syntax::trim(&rule.prelude).is_empty() => {
                            Some(Rule::FontFace(declarations(&block)))
                        }
                        ("import", None) => {
                            let values = syntax::trim(&rule.prelude);
                            let url = match values.first() {
                                Some(C::Token(token)) => match &token.kind {
                                    T::Url(url) | T::String(url) => Some(url.clone()),
                                    _ => None,
                                },
                                Some(C::Function { name, value })
                                    if name.eq_ignore_ascii_case("url") =>
                                {
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
                            };
                            url.map(Rule::Import).or(Some(Rule::Other(preserved)))
                        }
                        _ => Some(Rule::Other(preserved)),
                    }
                }
            }
        })
        .collect()
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
