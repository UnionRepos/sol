//! Registered values need validation before variable consumers interpret them:
//! accepting an invalid token stream there would bypass the registration's initial
//! value and make the result depend on which ordinary property used the variable.
use crate::{
    parser::{Rule, Stylesheet},
    syntax::{self, ComponentValue as C, TokenKind as T},
    Value,
};
use std::collections::HashMap;
#[derive(Clone, Debug)]
pub struct Registration {
    pub name: String,
    pub syntax: String,
    pub inherits: bool,
    pub initial: Option<Vec<C>>,
}
fn supported(syntax: &str) -> bool {
    syntax == "*"
        || syntax.split('|').all(|part| {
            let part = part.trim().trim_end_matches(['+', '#']);
            matches!(
                part,
                "<length>"
                    | "<number>"
                    | "<integer>"
                    | "<percentage>"
                    | "<length-percentage>"
                    | "<angle>"
                    | "<time>"
                    | "<resolution>"
                    | "<color>"
                    | "<custom-ident>"
                    | "<string>"
                    | "<url>"
            ) || (!part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_'))
        })
}
pub(crate) fn parse(prelude: &[C], block: &[C]) -> Option<Registration> {
    let [C::Token(token)] = syntax::trim(prelude) else {
        return None;
    };
    let T::Ident(name) = &token.kind else {
        return None;
    };
    if !name.starts_with("--") || name == "--" {
        return None;
    }
    let mut syntax_value = None;
    let mut inherits = None;
    let mut initial = None;
    for item in syntax::declaration::block_contents(block, &mut Vec::new()) {
        if let syntax::BlockItem::Declaration(decl) = item {
            if decl.important {
                continue;
            }
            match (decl.name.as_str(), syntax::trim(&decl.value)) {
                ("syntax", [C::Token(token)]) => {
                    if let T::String(value) = &token.kind {
                        syntax_value = Some(value.clone());
                    }
                }
                ("inherits", [C::Token(token)]) => {
                    if let T::Ident(value) = &token.kind {
                        match value.to_ascii_lowercase().as_str() {
                            "true" => inherits = Some(true),
                            "false" => inherits = Some(false),
                            _ => {}
                        }
                    }
                }
                ("initial-value", values) => initial = Some(values.to_vec()),
                _ => {}
            }
        }
    }
    let registration = Registration {
        name: name.clone(),
        syntax: syntax_value?,
        inherits: inherits?,
        initial,
    };
    if !supported(&registration.syntax)
        || registration.syntax != "*" && registration.initial.is_none()
    {
        return None;
    }
    if let Some(initial) = &registration.initial {
        if crate::variables::contains_var(initial) || !registration.accepts(initial) {
            return None;
        }
    }
    Some(registration)
}
impl Registration {
    pub(crate) fn accepts(&self, values: &[C]) -> bool {
        if self.syntax == "*" {
            return true;
        }
        self.syntax.split('|').any(|syntax| {
            let syntax = syntax.trim();
            let list = syntax.ends_with('#');
            let multiple = syntax.ends_with('+');
            let syntax = syntax.trim_end_matches(['+', '#']);
            if list {
                return syntax::split(values, &T::Comma)
                    .iter()
                    .all(|group| single(syntax, syntax::trim(group)));
            }
            if multiple {
                return !values.is_empty()
                    && crate::values::from_components(values)
                        .iter()
                        .all(|value| typed(syntax, value));
            }
            single(syntax, syntax::trim(values))
        })
    }
}
fn single(syntax: &str, values: &[C]) -> bool {
    if syntax == "<custom-ident>" {
        return matches!(values,[C::Token(token)] if matches!(&token.kind,T::Ident(name) if !matches!(name.as_str(),"initial"|"inherit"|"unset"|"revert"|"revert-layer"|"default")));
    }
    if !syntax.starts_with('<') {
        return matches!(values,[C::Token(token)] if matches!(&token.kind,T::Ident(name) if name.eq_ignore_ascii_case(syntax)));
    }
    let values = crate::values::from_components(values);
    matches!(values.as_slice(),[value] if typed(syntax,value))
}
fn typed(syntax: &str, value: &Value) -> bool {
    use crate::values::Unit;
    match syntax {
        "<length>" => {
            matches!(
                value,
                Value::Length(..) | Value::Math(..) | Value::Number(0.0)
            ) && crate::style::value_px(value, 16.0, 0.0).is_some()
        }
        "<length-percentage>" => matches!(value, Value::Percentage(_)) || typed("<length>", value),
        "<number>" => matches!(value, Value::Number(_)),
        "<integer>" => matches!(value,Value::Number(n) if n.fract()==0.0),
        "<percentage>" => matches!(value, Value::Percentage(_)),
        "<angle>" => matches!(
            value,
            Value::Length(_, Unit::Deg | Unit::Rad | Unit::Grad | Unit::Turn)
        ),
        "<time>" => matches!(value, Value::Length(_, Unit::S | Unit::Ms)),
        "<resolution>" => matches!(value, Value::Length(_, Unit::Dpi | Unit::Dppx | Unit::Dpcm)),
        "<color>" => {
            matches!(value, Value::Color(..))
                || matches!(value,Value::Ident(s) if s=="currentcolor")
        }
        "<string>" => matches!(value, Value::Str(_)),
        "<url>" => matches!(value, Value::Url(_)),
        "<custom-ident>" => matches!(value, Value::Ident(_)),
        _ => false,
    }
}
pub(crate) fn registry(sheets: &[&Stylesheet]) -> HashMap<String, Registration> {
    fn collect(rules: &[Rule], out: &mut HashMap<String, Registration>) {
        for rule in rules {
            match rule {
                Rule::Property(rule) => {
                    out.insert(rule.name.clone(), rule.clone());
                }
                Rule::Layer(rule) => collect(&rule.rules, out),
                Rule::Media(rule) if crate::conditions::media_matches(&rule.queries) => {
                    collect(&rule.rules, out)
                }
                Rule::Supports(rule) if rule.condition.supports() => collect(&rule.rules, out),
                _ => {}
            }
        }
    }
    let mut out = HashMap::new();
    for sheet in sheets {
        collect(&sheet.rules, &mut out);
    }
    out
}
