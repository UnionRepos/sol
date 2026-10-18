use crate::selector::{Pseudo, Selector};
use crate::{
    parser::Rule,
    syntax::{self, ComponentValue as C, Rule as Raw, TokenKind as T},
};
use std::collections::HashMap;

pub(crate) fn declarations(rules: &[Raw]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for rule in rules {
        match rule {
            Raw::At(rule)
                if rule.name.eq_ignore_ascii_case("namespace") && rule.block.is_none() =>
            {
                let values = syntax::trim(&rule.prelude);
                let (prefix, values) = if let Some(C::Token(token)) = values.first() {
                    if let T::Ident(prefix) = &token.kind {
                        (prefix.as_str(), syntax::trim(&values[1..]))
                    } else {
                        ("", values)
                    }
                } else {
                    ("", values)
                };
                if values.len() == 1 {
                    if let Some(uri) = crate::parser::import_url(values.first()) {
                        out.insert(prefix.into(), uri);
                    }
                }
            }
            Raw::At(rule)
                if matches!(
                    rule.name.to_ascii_lowercase().as_str(),
                    "charset" | "import"
                ) || rule.name.eq_ignore_ascii_case("layer") && rule.block.is_none() => {}
            _ => break,
        }
    }
    out
}
fn resolve(selector: &mut Selector, namespaces: &HashMap<String, String>, default: bool) -> bool {
    for compound in &mut selector.parts {
        if let Some(prefix) = &compound.namespace {
            if !matches!(prefix.as_str(), "" | "*") {
                let Some(uri) = namespaces.get(prefix) else {
                    return false;
                };
                compound.namespace = Some(uri.clone());
            }
        } else if default || compound.tag.is_some() {
            compound.namespace = namespaces.get("").cloned();
        }
        for attribute in &mut compound.attrs {
            if let Some(prefix) = &attribute.namespace {
                if !matches!(prefix.as_str(), "" | "*") {
                    let Some(uri) = namespaces.get(prefix) else {
                        return false;
                    };
                    attribute.namespace = Some(uri.clone());
                }
            }
        }
        for pseudo in &mut compound.pseudos {
            let valid = match pseudo {
                Pseudo::Not(s) | Pseudo::Is(s) | Pseudo::Has(s) => resolve(s, namespaces, false),
                Pseudo::IsList(list) | Pseudo::Where(list) => {
                    list.retain_mut(|selector| resolve(selector, namespaces, false));
                    true
                }
                Pseudo::NotList(list)
                | Pseudo::NthChildOf(_, list)
                | Pseudo::NthLastChildOf(_, list) => {
                    list.iter_mut().all(|s| resolve(s, namespaces, false))
                }
                Pseudo::HasList(list) => list
                    .iter_mut()
                    .all(|s| resolve(&mut s.selector, namespaces, false)),
                _ => true,
            };
            if !valid {
                return false;
            }
        }
    }
    true
}
pub(crate) fn apply(rules: &mut Vec<Rule>, namespaces: &HashMap<String, String>) {
    rules.retain_mut(|rule| match rule {
        Rule::Style(rule) => rule
            .selectors
            .iter_mut()
            .all(|selector| resolve(selector, namespaces, true)),
        Rule::Container(rule) => {
            apply(&mut rule.rules, namespaces);
            true
        }
        Rule::Media(rule) => {
            apply(&mut rule.rules, namespaces);
            true
        }
        Rule::Supports(rule) => {
            apply(&mut rule.rules, namespaces);
            true
        }
        Rule::Layer(rule) => {
            apply(&mut rule.rules, namespaces);
            true
        }
        _ => true,
    });
}
