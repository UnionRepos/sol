//! CSS selector parsing + matching.

#[derive(Debug, Clone)]
pub struct Selector {
    /// Starting with the subject lets matching reject an unrelated element
    /// before walking its ancestors or siblings, where combinator checks can
    /// be more expensive.
    pub parts: Vec<Compound>,
    pub combinators: Vec<Combinator>, // combinators[i] connects parts[i+1] to parts[i]
    pub specificity: (u32, u32, u32),
}

#[derive(Debug, Clone)]
pub struct Compound {
    pub tag: Option<String>, // None = '*'
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<AttrSel>,
    pub pseudos: Vec<Pseudo>,
    pub additional_ids: Vec<String>,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AttrSel {
    pub name: String,
    pub op: AttrOp,
    pub value: String,
    pub case: Option<bool>,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttrOp {
    Exists,
    Eq,        // =
    Includes,  // ~=
    DashMatch, // |=
    Prefix,    // ^=
    Suffix,    // $=
    Substr,    // *=
}

#[derive(Debug, Clone)]
pub enum Pseudo {
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    OnlyOfType,
    NthChild(NthArg),
    NthOfType(NthArg),
    NthLastChild(NthArg),
    Empty,
    Root,
    Not(Box<Selector>),
    Is(Box<Selector>),
    Has(Box<Selector>), // limited: descendant selector support
    Hover,
    Active,
    Focus,
    Visited,
    Link,
    Checked,
    Disabled,
    Enabled,
    Target,
    NotList(Vec<Selector>),
    IsList(Vec<Selector>),
    Where(Vec<Selector>),
    HasList(Vec<RelativeSelector>),
    NthLastOfType(NthArg),
    NthChildOf(NthArg, Vec<Selector>),
    NthLastChildOf(NthArg, Vec<Selector>),
    Scope,
    Lang(Vec<String>),
    Dir(String),
    AnyLink,
    Required,
    Optional,
    ReadOnly,
    ReadWrite,
    PseudoElement(String),
    Unknown(String),
}

#[derive(Debug, Clone)]
pub enum NthArg {
    AnB(i32, i32), // a*n + b
    Odd,
    Even,
    N(i32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    LaterSibling,
}

#[derive(Debug, Clone)]
pub struct RelativeSelector {
    pub leading: Combinator,
    pub selector: Selector,
}

use crate::syntax::{self, ComponentValue as C, TokenKind as T};

pub fn parse_selector_list(css: &str) -> Vec<Selector> {
    let values = syntax::parse_component_values(css);
    if values.errors.iter().any(|e| {
        matches!(
            e.kind,
            syntax::ErrorKind::LimitExceeded
                | syntax::ErrorKind::BadString
                | syntax::ErrorKind::UnexpectedClose
                | syntax::ErrorKind::UnclosedBlock
        )
    }) {
        return Vec::new();
    }
    list(&values.values, 0, false).unwrap_or_default()
}
pub fn parse_selector(css: &str) -> Option<Selector> {
    let values = syntax::parse_component_values(css);
    if values
        .errors
        .iter()
        .any(|e| !matches!(e.kind, syntax::ErrorKind::UnterminatedComment))
    {
        return None;
    }
    parse_components(&values.values, 0)
}
fn list(values: &[C], depth: usize, forgiving: bool) -> Option<Vec<Selector>> {
    let mut out = Vec::new();
    for group in syntax::split(values, &T::Comma) {
        let parsed = parse_components(group, depth + 1).filter(|selector| {
            depth == 0
                || !selector.parts.iter().any(|part| {
                    part.pseudos
                        .iter()
                        .any(|pseudo| matches!(pseudo, Pseudo::PseudoElement(_)))
                })
        });
        match parsed {
            Some(selector) => out.push(selector),
            None if !forgiving => return None,
            None => {}
        }
    }
    if out.is_empty() && !forgiving {
        None
    } else {
        Some(out)
    }
}
fn empty_compound() -> Compound {
    Compound {
        tag: None,
        id: None,
        classes: Vec::new(),
        attrs: Vec::new(),
        pseudos: Vec::new(),
        additional_ids: Vec::new(),
        namespace: None,
    }
}
fn skip_ws(values: &[C], i: &mut usize) -> bool {
    let start = *i;
    while values.get(*i).is_some_and(C::is_whitespace) {
        *i += 1;
    }
    *i != start
}
fn ident(value: Option<&C>) -> Option<&str> {
    match value?.kind()? {
        T::Ident(s) => Some(s),
        _ => None,
    }
}
fn delim(value: Option<&C>, c: char) -> bool {
    value.and_then(C::kind) == Some(&T::Delim(c))
}
fn combinator(value: Option<&C>) -> Option<Combinator> {
    match value?.kind()? {
        T::Delim('>') => Some(Combinator::Child),
        T::Delim('+') => Some(Combinator::NextSibling),
        T::Delim('~') => Some(Combinator::LaterSibling),
        _ => None,
    }
}
fn parse_components(values: &[C], depth: usize) -> Option<Selector> {
    if depth > 64 {
        return None;
    }
    let values = syntax::trim(values);
    if values.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    let mut combinators = Vec::new();
    let mut i = 0;
    loop {
        parts.push(compound(values, &mut i, depth)?);
        let spaced = skip_ws(values, &mut i);
        if i == values.len() {
            break;
        }
        if parts
            .last()?
            .pseudos
            .iter()
            .any(|p| matches!(p, Pseudo::PseudoElement(_)))
        {
            return None;
        }
        let relation = if let Some(c) = combinator(values.get(i)) {
            i += 1;
            skip_ws(values, &mut i);
            c
        } else if spaced {
            Combinator::Descendant
        } else {
            return None;
        };
        if i == values.len() {
            return None;
        }
        combinators.push(relation);
    }
    let mut specificity = (0, 0, 0);
    for part in &parts {
        let (a, b, c) = part.specificity();
        specificity.0 += a;
        specificity.1 += b;
        specificity.2 += c;
    }
    parts.reverse();
    combinators.reverse();
    Some(Selector {
        parts,
        combinators,
        specificity,
    })
}
fn type_name(values: &[C], i: &mut usize) -> Option<(Option<String>, Option<String>)> {
    let first = match values.get(*i).and_then(C::kind) {
        Some(T::Ident(s)) => Some(s.clone()),
        Some(T::Delim('*')) => Some("*".into()),
        Some(T::Delim('|')) => None,
        _ => return None,
    };
    if first.is_some() {
        *i += 1;
    }
    if delim(values.get(*i), '|') {
        *i += 1;
        let tag = match values.get(*i).and_then(C::kind) {
            Some(T::Ident(s)) => Some(s.to_ascii_lowercase()),
            Some(T::Delim('*')) => None,
            _ => return None,
        };
        *i += 1;
        Some((Some(first.unwrap_or_default()), tag))
    } else {
        Some((
            None,
            first.filter(|s| s != "*").map(|s| s.to_ascii_lowercase()),
        ))
    }
}
fn compound(values: &[C], i: &mut usize, depth: usize) -> Option<Compound> {
    let start = *i;
    let mut out = empty_compound();
    if matches!(
        values.get(*i).and_then(C::kind),
        Some(T::Ident(_) | T::Delim('*' | '|'))
    ) {
        let (ns, tag) = type_name(values, i)?;
        out.namespace = ns;
        out.tag = tag;
    }
    while *i < values.len() {
        if out
            .pseudos
            .iter()
            .any(|p| matches!(p, Pseudo::PseudoElement(_)))
            && !matches!(values[*i].kind(), Some(T::Colon))
        {
            break;
        }
        match &values[*i] {
            C::Token(crate::syntax::Token {
                kind: T::Hash { value, id: true },
                ..
            }) => {
                if out.id.is_none() {
                    out.id = Some(value.clone());
                } else {
                    out.additional_ids.push(value.clone());
                }
                *i += 1;
            }
            C::Token(crate::syntax::Token {
                kind: T::Delim('.'),
                ..
            }) => {
                *i += 1;
                let name = ident(values.get(*i))?.to_string();
                out.classes.push(name);
                *i += 1;
            }
            C::Block {
                opening: '[',
                value,
            } => {
                out.attrs.push(attribute(value)?);
                *i += 1;
            }
            C::Token(crate::syntax::Token { kind: T::Colon, .. }) => {
                *i += 1;
                let element = values.get(*i).and_then(C::kind) == Some(&T::Colon);
                if element {
                    *i += 1;
                }
                let pseudo = match values.get(*i)? {
                    C::Token(crate::syntax::Token {
                        kind: T::Ident(name),
                        ..
                    }) => simple_pseudo(name, element)?,
                    C::Function { name, value } if !element => {
                        functional_pseudo(name, value, depth + 1)?
                    }
                    C::Function { name, .. }
                        if element
                            && matches!(
                                name.to_ascii_lowercase().as_str(),
                                "part" | "slotted" | "highlight"
                            ) =>
                    {
                        Pseudo::PseudoElement(name.to_ascii_lowercase())
                    }
                    _ => return None,
                };
                if out
                    .pseudos
                    .iter()
                    .any(|p| matches!(p, Pseudo::PseudoElement(_)))
                    && !matches!(pseudo, Pseudo::Hover | Pseudo::Active | Pseudo::Focus)
                {
                    return None;
                }
                out.pseudos.push(pseudo);
                *i += 1;
            }
            _ => break,
        }
    }
    if *i == start {
        None
    } else {
        Some(out)
    }
}
fn attribute(values: &[C]) -> Option<AttrSel> {
    let mut i = 0;
    skip_ws(values, &mut i);
    let first = match values.get(i).and_then(C::kind) {
        Some(T::Ident(s)) => Some(s.to_ascii_lowercase()),
        Some(T::Delim('*')) => Some("*".into()),
        Some(T::Delim('|')) => None,
        _ => return None,
    };
    if first.is_some() {
        i += 1;
    }
    let (namespace, name) = if delim(values.get(i), '|') && !delim(values.get(i + 1), '=') {
        i += 1;
        let name = ident(values.get(i))?.to_ascii_lowercase();
        i += 1;
        (Some(first.unwrap_or_default()), name)
    } else {
        (None, first?)
    };
    if name == "*" {
        return None;
    }
    skip_ws(values, &mut i);
    if i == values.len() {
        return Some(AttrSel {
            name,
            op: AttrOp::Exists,
            value: String::new(),
            case: None,
            namespace,
        });
    }
    let op = match values.get(i)?.kind()? {
        T::Delim('=') => {
            i += 1;
            AttrOp::Eq
        }
        T::Delim(c @ ('~' | '|' | '^' | '$' | '*')) => {
            if !delim(values.get(i + 1), '=') {
                return None;
            }
            i += 2;
            match c {
                '~' => AttrOp::Includes,
                '|' => AttrOp::DashMatch,
                '^' => AttrOp::Prefix,
                '$' => AttrOp::Suffix,
                _ => AttrOp::Substr,
            }
        }
        _ => return None,
    };
    skip_ws(values, &mut i);
    let value = match values.get(i)?.kind()? {
        T::Ident(s) | T::String(s) => s.clone(),
        _ => return None,
    };
    i += 1;
    skip_ws(values, &mut i);
    let case = if i < values.len() {
        let flag = ident(values.get(i))?;
        i += 1;
        match flag.to_ascii_lowercase().as_str() {
            "i" => Some(false),
            "s" => Some(true),
            _ => return None,
        }
    } else {
        None
    };
    skip_ws(values, &mut i);
    if i != values.len() {
        return None;
    }
    Some(AttrSel {
        name,
        op,
        value,
        case,
        namespace,
    })
}
fn simple_pseudo(name: &str, element: bool) -> Option<Pseudo> {
    let name = name.to_ascii_lowercase();
    if element
        || matches!(
            name.as_str(),
            "before" | "after" | "first-line" | "first-letter"
        )
    {
        return matches!(
            name.as_str(),
            "before"
                | "after"
                | "first-line"
                | "first-letter"
                | "selection"
                | "marker"
                | "placeholder"
                | "backdrop"
                | "file-selector-button"
        )
        .then_some(Pseudo::PseudoElement(name));
    }
    Some(match name.as_str() {
        "first-child" => Pseudo::FirstChild,
        "last-child" => Pseudo::LastChild,
        "only-child" => Pseudo::OnlyChild,
        "first-of-type" => Pseudo::FirstOfType,
        "last-of-type" => Pseudo::LastOfType,
        "only-of-type" => Pseudo::OnlyOfType,
        "empty" => Pseudo::Empty,
        "root" => Pseudo::Root,
        "scope" => Pseudo::Scope,
        "hover" => Pseudo::Hover,
        "active" => Pseudo::Active,
        "focus" => Pseudo::Focus,
        "visited" => Pseudo::Visited,
        "link" => Pseudo::Link,
        "any-link" => Pseudo::AnyLink,
        "checked" => Pseudo::Checked,
        "disabled" => Pseudo::Disabled,
        "enabled" => Pseudo::Enabled,
        "target" => Pseudo::Target,
        "required" => Pseudo::Required,
        "optional" => Pseudo::Optional,
        "read-only" => Pseudo::ReadOnly,
        "read-write" => Pseudo::ReadWrite,
        _ => return None,
    })
}
fn functional_pseudo(name: &str, values: &[C], depth: usize) -> Option<Pseudo> {
    if depth > 64 {
        return None;
    }
    let name = name.to_ascii_lowercase();
    Some(match name.as_str() {
        "not" => Pseudo::NotList(list(values, depth, false)?),
        "is" | "matches" => Pseudo::IsList(list(values, depth, true)?),
        "where" => Pseudo::Where(list(values, depth, true)?),
        "has" => {
            let mut selectors = Vec::new();
            for group in syntax::split(values, &T::Comma) {
                let group = syntax::trim(group);
                let (leading, group) = if let Some(c) = combinator(group.first()) {
                    (c, syntax::trim(&group[1..]))
                } else {
                    (Combinator::Descendant, group)
                };
                let selector = parse_components(group, depth + 1)?;
                if forbidden_in_has(&selector) {
                    return None;
                }
                selectors.push(RelativeSelector { leading, selector });
            }
            Pseudo::HasList(selectors)
        }
        "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type" => {
            let of = values
                .iter()
                .position(|v| matches!(v.kind(),Some(T::Ident(s)) if s.eq_ignore_ascii_case("of")));
            let formula = &values[..of.unwrap_or(values.len())];
            let nth = parse_nth(formula)?;
            let selectors = if let Some(of) = of {
                if name.ends_with("of-type") {
                    return None;
                }
                Some(list(&values[of + 1..], depth, false)?)
            } else {
                None
            };
            match (name.as_str(), selectors) {
                ("nth-child", Some(s)) => Pseudo::NthChildOf(nth, s),
                ("nth-last-child", Some(s)) => Pseudo::NthLastChildOf(nth, s),
                ("nth-child", None) => Pseudo::NthChild(nth),
                ("nth-last-child", None) => Pseudo::NthLastChild(nth),
                ("nth-of-type", None) => Pseudo::NthOfType(nth),
                _ => Pseudo::NthLastOfType(nth),
            }
        }
        "lang" => {
            let mut languages = Vec::new();
            for group in syntax::split(values, &T::Comma) {
                let [C::Token(token)] = syntax::trim(group) else {
                    return None;
                };
                let (T::Ident(value) | T::String(value)) = &token.kind else {
                    return None;
                };
                if value.is_empty() {
                    return None;
                }
                languages.push(value.to_ascii_lowercase());
            }
            Pseudo::Lang(languages)
        }
        "dir" => {
            let [C::Token(token)] = syntax::trim(values) else {
                return None;
            };
            let T::Ident(value) = &token.kind else {
                return None;
            };
            let value = value.to_ascii_lowercase();
            if !matches!(value.as_str(), "ltr" | "rtl") {
                return None;
            }
            Pseudo::Dir(value)
        }
        _ => return None,
    })
}
fn forbidden_in_has(selector: &Selector) -> bool {
    selector.parts.iter().any(|part| {
        part.pseudos.iter().any(|pseudo| match pseudo {
            Pseudo::Has(_) | Pseudo::HasList(_) | Pseudo::PseudoElement(_) => true,
            Pseudo::Not(s) | Pseudo::Is(s) => forbidden_in_has(s),
            Pseudo::NotList(s)
            | Pseudo::IsList(s)
            | Pseudo::Where(s)
            | Pseudo::NthChildOf(_, s)
            | Pseudo::NthLastChildOf(_, s) => s.iter().any(forbidden_in_has),
            _ => false,
        })
    })
}

fn parse_nth(values: &[C]) -> Option<NthArg> {
    let values = syntax::trim(values);
    let mut index = 0;
    let plus = delim(values.first(), '+');
    if plus {
        index += 1;
        if !ident(values.get(index)).is_some_and(|name| name.starts_with(['n', 'N'])) {
            return None;
        }
    }
    let first = values.get(index)?.kind()?;
    index += 1;
    let (coefficient, suffix) = match first {
        T::Number(number)
            if number.integer && !plus && syntax::trim(&values[index..]).is_empty() =>
        {
            return number.representation.parse().ok().map(NthArg::N)
        }
        T::Dimension(number, unit) if number.integer && !plus => (
            number.representation.parse::<i32>().ok()?,
            unit.to_ascii_lowercase(),
        ),
        T::Ident(name) => {
            let name = name.to_ascii_lowercase();
            if !plus && syntax::trim(&values[index..]).is_empty() {
                if name == "odd" {
                    return Some(NthArg::Odd);
                }
                if name == "even" {
                    return Some(NthArg::Even);
                }
            }
            if let Some(suffix) = name.strip_prefix('-') {
                (-1, suffix.to_string())
            } else {
                (1, name)
            }
        }
        _ => return None,
    };
    let tail = syntax::trim(&values[index..]);
    let offset = if suffix == "n" {
        if tail.is_empty() {
            0
        } else if let [C::Token(token)] = tail {
            let T::Number(number) = &token.kind else {
                return None;
            };
            if !number.integer || !number.representation.starts_with(['+', '-']) {
                return None;
            }
            number.representation.parse::<i32>().ok()?
        } else {
            let sign = match tail.first()?.kind()? {
                T::Delim(sign @ ('+' | '-')) => *sign,
                _ => return None,
            };
            unsigned_offset(syntax::trim(&tail[1..]), sign)?
        }
    } else if suffix == "n-" {
        unsigned_offset(tail, '-')?
    } else {
        let digits = suffix.strip_prefix("n-")?;
        if !tail.is_empty() || digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        format!("-{digits}").parse::<i32>().ok()?
    };
    Some(NthArg::AnB(coefficient, offset))
}

fn unsigned_offset(values: &[C], sign: char) -> Option<i32> {
    let [C::Token(token)] = values else {
        return None;
    };
    let T::Number(number) = &token.kind else {
        return None;
    };
    if !number.integer || !number.representation.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    format!("{sign}{}", number.representation)
        .parse::<i32>()
        .ok()
}

fn max_specificity(selectors: &[Selector]) -> (u32, u32, u32) {
    selectors
        .iter()
        .map(|s| s.specificity)
        .max()
        .unwrap_or_default()
}
impl Compound {
    pub fn specificity(&self) -> (u32, u32, u32) {
        let mut result = (
            u32::from(self.id.is_some()) + self.additional_ids.len() as u32,
            (self.classes.len() + self.attrs.len()) as u32,
            u32::from(self.tag.is_some()),
        );
        for pseudo in &self.pseudos {
            let (a, b, c) = match pseudo {
                Pseudo::Not(s) | Pseudo::Is(s) | Pseudo::Has(s) => s.specificity,
                Pseudo::NotList(s) | Pseudo::IsList(s) => max_specificity(s),
                Pseudo::Where(_) => (0, 0, 0),
                Pseudo::HasList(s) => s
                    .iter()
                    .map(|s| s.selector.specificity)
                    .max()
                    .unwrap_or_default(),
                Pseudo::NthChildOf(_, s) | Pseudo::NthLastChildOf(_, s) => {
                    let (a, b, c) = max_specificity(s);
                    (a, b + 1, c)
                }
                Pseudo::PseudoElement(_) => (0, 0, 1),
                _ => (0, 1, 0),
            };
            result.0 += a;
            result.1 += b;
            result.2 += c;
        }
        result
    }
}
