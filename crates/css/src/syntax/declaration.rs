use super::{trim, ComponentValue as C, ErrorKind, ParseError, Rule, TokenKind as T};

#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub name: String,
    pub value: Vec<C>,
    pub important: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub enum BlockItem {
    Declaration(Declaration),
    Rule(Rule),
}
#[derive(Clone, Debug, PartialEq)]
pub struct BlockContents {
    pub items: Vec<BlockItem>,
    pub errors: Vec<ParseError>,
}

pub fn parse_declaration(css: &str) -> Result<Declaration, Vec<ParseError>> {
    let parsed = super::parse_component_values(css);
    declaration(&parsed.values).ok_or_else(|| {
        if parsed.errors.is_empty() {
            vec![ParseError {
                offset: 0,
                kind: ErrorKind::InvalidDeclaration,
            }]
        } else {
            parsed.errors
        }
    })
}
pub fn declaration(values: &[C]) -> Option<Declaration> {
    let values = trim(values);
    let T::Ident(name) = values.first()?.kind()? else {
        return None;
    };
    if name == "--" {
        return None;
    }
    let tail = trim(&values[1..]);
    if tail.first()?.kind() != Some(&T::Colon) {
        return None;
    }
    let mut value = trim(&tail[1..]);
    let mut important = false;
    if matches!(value.last().and_then(C::kind),Some(T::Ident(name)) if name.eq_ignore_ascii_case("important"))
    {
        let prefix = trim(&value[..value.len() - 1]);
        if prefix.last().and_then(C::kind) == Some(&T::Delim('!')) {
            important = true;
            value = trim(&prefix[..prefix.len() - 1]);
        }
    }
    let custom = name.starts_with("--");
    if !custom
        && value
            .iter()
            .any(|v| matches!(v, C::Block { opening: '{', .. }))
        && !matches!(value, [C::Block { opening: '{', .. }])
    {
        return None;
    }
    if value.iter().any(|v| v.kind() == Some(&T::Delim('!'))) {
        return None;
    }
    if invalid(value) || value.iter().any(|v| v.kind() == Some(&T::Semicolon)) {
        return None;
    }
    Some(Declaration {
        name: if custom {
            name.clone()
        } else {
            name.to_ascii_lowercase()
        },
        value: if name.eq_ignore_ascii_case("unicode-range") {
            unicode_ranges(value)
        } else {
            value.to_vec()
        },
        important,
    })
}
fn unicode_ranges(values: &[C]) -> Vec<C> {
    let mut input = String::new();
    let mut end = None;
    let mut spans = Vec::new();
    for value in values {
        let C::Token(token) = value else {
            return values.to_vec();
        };
        // Retokenizing this legacy descriptor is required by CSS Syntax. Source
        // adjacency matters: comments must not turn separated U and + into U+.
        if end.is_some_and(|end| end != token.start) {
            input.push_str("/**/");
        }
        let start = input.len();
        input.push_str(&token.kind.to_css());
        spans.push((start, input.len(), token.start, token.end));
        end = Some(token.end);
    }
    super::tokenize_with_unicode_ranges(&input, true)
        .tokens
        .into_iter()
        .map(|mut token| {
            if let Some(span) = spans.get(spans.partition_point(|span| span.1 <= token.start)) {
                token.start = span.2;
            }
            if let Some(span) = spans.get(spans.partition_point(|span| span.1 < token.end)) {
                token.end = span.3;
            }
            C::Token(token)
        })
        .collect()
}

fn invalid(values: &[C]) -> bool {
    values.iter().any(|v| match v {
        C::Token(t) => matches!(t.kind, T::BadString | T::BadUrl | T::Close(_)),
        C::Function { value, .. } | C::Block { value, .. } => invalid(value),
    })
}
pub fn parse_block_contents(css: &str) -> BlockContents {
    let parsed = super::parse_component_values(css);
    let mut errors = parsed.errors;
    let items = block_contents(&parsed.values, &mut errors);
    BlockContents { items, errors }
}
pub fn block_contents(values: &[C], errors: &mut Vec<ParseError>) -> Vec<BlockItem> {
    let mut items = Vec::new();
    let mut i = 0;
    while i < values.len() {
        if values[i].is_whitespace() || values[i].kind() == Some(&T::Semicolon) {
            i += 1;
            continue;
        }
        let start = i;
        let at = matches!(values[i].kind(), Some(T::AtKeyword(_)));
        let custom = matches!(values[i].kind(),Some(T::Ident(s)) if s.starts_with("--"));
        let mut block = None;
        while i < values.len() && values[i].kind() != Some(&T::Semicolon) {
            if matches!(values[i], C::Block { opening: '{', .. }) && (at || !custom) {
                block = Some(i);
                break;
            }
            i += 1;
        }
        if let Some(end) = block {
            if !at {
                let mut after = end + 1;
                while values.get(after).is_some_and(C::is_whitespace) {
                    after += 1;
                }
                if values.get(after).and_then(C::kind) == Some(&T::Delim('!')) {
                    after += 1;
                    while values.get(after).is_some_and(C::is_whitespace) {
                        after += 1;
                    }
                    if matches!(values.get(after).and_then(C::kind), Some(T::Ident(name)) if name.eq_ignore_ascii_case("important"))
                    {
                        after += 1;
                        while values.get(after).is_some_and(C::is_whitespace) {
                            after += 1;
                        }
                    }
                }
                if after == values.len() || values[after].kind() == Some(&T::Semicolon) {
                    if let Some(declaration) = declaration(&values[start..after]) {
                        items.push(BlockItem::Declaration(declaration));
                        i = (after + 1).min(values.len());
                        continue;
                    }
                }
            }
            let mut rules = super::parse_rule_list(&values[start..=end], false);
            if let Some(rule) = rules.pop() {
                items.push(BlockItem::Rule(rule));
            }
            i = end + 1;
            continue;
        }
        if at {
            if let Some(rule) = super::parse_rule_list(&values[start..i], false).pop() {
                items.push(BlockItem::Rule(rule));
            }
        } else if let Some(decl) = declaration(&values[start..i]) {
            items.push(BlockItem::Declaration(decl));
        } else {
            errors.push(ParseError {
                offset: values[start].kind().map_or(0, |_| {
                    if let C::Token(t) = &values[start] {
                        t.start
                    } else {
                        0
                    }
                }),
                kind: ErrorKind::InvalidDeclaration,
            });
        }
        i = (i + 1).min(values.len());
    }
    items
}
