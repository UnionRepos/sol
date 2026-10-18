//! Keeping nesting in the token tree avoids replacing ampersands inside strings
//! or losing the parent's selector-list specificity when a child is expanded.
use crate::syntax::{self, ComponentValue as C, Token, TokenKind as T};
fn token(kind: T) -> C {
    C::Token(Token {
        kind,
        start: 0,
        end: 0,
    })
}
fn parent_selector(parent: &[C]) -> Vec<C> {
    vec![
        token(T::Colon),
        C::Function {
            name: "is".into(),
            value: parent.to_vec(),
        },
    ]
}
fn replace(values: &[C], parent: &[C], found: &mut bool) -> Vec<C> {
    values
        .iter()
        .flat_map(|value| match value {
            C::Token(t) if t.kind == T::Delim('&') => {
                *found = true;
                parent_selector(parent)
            }
            C::Function { name, value } => vec![C::Function {
                name: name.clone(),
                value: replace(value, parent, found),
            }],
            C::Block { opening, value } => vec![C::Block {
                opening: *opening,
                value: replace(value, parent, found),
            }],
            value => vec![value.clone()],
        })
        .collect()
}
fn count(values: &[C]) -> usize {
    values
        .iter()
        .map(|v| {
            1 + match v {
                C::Function { value, .. } | C::Block { value, .. } => count(value),
                _ => 0,
            }
        })
        .sum()
}
fn references(values: &[C]) -> usize {
    values
        .iter()
        .map(|v| match v {
            C::Token(token) if token.kind == T::Delim('&') => 1,
            C::Function { value, .. } | C::Block { value, .. } => references(value),
            _ => 0,
        })
        .sum()
}
pub(crate) fn selector(values: &[C], parent: &[C], budget: &mut usize) -> Vec<C> {
    let references = syntax::split(values, &T::Comma)
        .iter()
        .map(|group| references(group).max(1))
        .sum::<usize>();
    let size = count(parent)
        .saturating_mul(references)
        .saturating_add(count(values))
        .saturating_add(4 * references);
    if size > 65_536 || size > *budget {
        return Vec::new();
    }
    *budget -= size;
    let mut out = Vec::new();
    for group in syntax::split(values, &T::Comma) {
        if !out.is_empty() {
            out.push(token(T::Comma));
        }
        let mut found = false;
        let expanded = replace(group, parent, &mut found);
        if !found {
            out.extend(parent_selector(parent));
            out.push(token(T::Whitespace));
        }
        out.extend(expanded);
    }
    out
}
