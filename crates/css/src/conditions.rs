use crate::syntax::{self, ComponentValue as C, TokenKind as T};
use crate::values::{Unit, Value};

#[derive(Debug, Clone)]
pub enum Condition {
    True,
    False,
    Invalid,
    Not(Box<Condition>),
    And(Vec<Condition>),
    Or(Vec<Condition>),
    Feature(Vec<C>),
    Selector(Vec<C>),
}

pub fn parse(values: &[C]) -> Condition {
    let values = syntax::trim(values);
    if values.is_empty() {
        return Condition::Invalid;
    }
    if matches!(values.first().and_then(C::kind),Some(T::Ident(s)) if s.eq_ignore_ascii_case("not"))
    {
        let tail = syntax::trim(&values[1..]);
        if tail.len() != 1 {
            return Condition::Invalid;
        }
        return Condition::Not(Box::new(parse(tail)));
    }
    let mut op = None;
    let mut groups = Vec::new();
    let mut start = 0;
    for (i, value) in values.iter().enumerate() {
        if let Some(T::Ident(name)) = value.kind() {
            if name.eq_ignore_ascii_case("and") || name.eq_ignore_ascii_case("or") {
                let next = name.to_ascii_lowercase();
                if op.as_ref().is_some_and(|previous| previous != &next) {
                    return Condition::Invalid;
                }
                op = Some(next);
                let group = syntax::trim(&values[start..i]);
                if group.len() != 1 {
                    return Condition::Invalid;
                }
                groups.push(parse(group));
                start = i + 1;
            }
        }
    }
    if let Some(op) = op {
        let group = syntax::trim(&values[start..]);
        if group.len() != 1 {
            return Condition::Invalid;
        }
        groups.push(parse(group));
        return if op == "and" {
            Condition::And(groups)
        } else {
            Condition::Or(groups)
        };
    }
    match values {
        [C::Block {
            opening: '(',
            value,
        }] => {
            let value = syntax::trim(value);
            if value.is_empty() {
                return Condition::Invalid;
            }
            if !value.iter().any(|v| v.kind() == Some(&T::Colon))
                && (matches!(
                    value.first(),
                    Some(C::Block { opening: '(', .. } | C::Function { .. })
                ) || matches!(value.first().and_then(C::kind),Some(T::Ident(s)) if s.eq_ignore_ascii_case("not")))
            {
                parse(value)
            } else {
                Condition::Feature(value.to_vec())
            }
        }
        [C::Function { name, value }] if name.eq_ignore_ascii_case("selector") => {
            Condition::Selector(value.clone())
        }
        [C::Function { .. }] => Condition::False,
        _ => Condition::Invalid,
    }
}

pub fn parse_media(values: &[C]) -> Vec<crate::parser::MediaQuery> {
    syntax::split(values, &T::Comma)
        .into_iter()
        .map(parse_media_query)
        .collect()
}

fn parse_media_query(group: &[C]) -> crate::parser::MediaQuery {
    let mut values = syntax::trim(group);
    let mut medium = None;
    let mut negated = false;
    let prefix = match values.first().and_then(C::kind) {
        Some(T::Ident(name))
            if name.eq_ignore_ascii_case("not") || name.eq_ignore_ascii_case("only") =>
        {
            Some(name.to_ascii_lowercase())
        }
        _ => None,
    };
    if prefix.is_some()
        && matches!(
            syntax::trim(&values[1..]).first().and_then(C::kind),
            Some(T::Ident(_))
        )
    {
        negated = prefix.as_deref() == Some("not");
        values = syntax::trim(&values[1..]);
    }
    if let Some(T::Ident(name)) = values.first().and_then(C::kind) {
        if !name.eq_ignore_ascii_case("not") {
            medium = Some(name.to_ascii_lowercase());
            values = syntax::trim(&values[1..]);
        }
    }
    if medium
        .as_deref()
        .is_some_and(|name| matches!(name, "and" | "or" | "only" | "not"))
    {
        return crate::parser::MediaQuery {
            medium: None,
            features: Vec::new(),
            condition: Condition::Invalid,
            negated: false,
        };
    }
    let mut condition = if medium.is_some() {
        if values.is_empty() {
            Condition::True
        } else if matches!(values.first().and_then(C::kind), Some(T::Ident(name)) if name.eq_ignore_ascii_case("and"))
        {
            parse(&values[1..])
        } else {
            Condition::Invalid
        }
    } else {
        parse(values)
    };
    if medium.is_some() && matches!(condition, Condition::Or(_)) {
        condition = Condition::Invalid;
    }
    let features = group
        .iter()
        .filter_map(|value| {
            let C::Block {
                opening: '(',
                value,
            } = value
            else {
                return None;
            };
            let parts = syntax::split(value, &T::Colon);
            (parts.len() == 2).then(|| {
                (
                    syntax::serialize(parts[0])
                        .replace("/**/", "")
                        .trim()
                        .to_ascii_lowercase(),
                    syntax::serialize(parts[1]),
                )
            })
        })
        .collect();
    crate::parser::MediaQuery {
        medium,
        features,
        condition,
        negated,
    }
}

impl Condition {
    // Unknown features are false and may be negated, but malformed boolean
    // grammar must invalidate its whole query instead of becoming true via not.
    fn valid(&self) -> bool {
        match self {
            Self::Invalid => false,
            Self::Not(value) => value.valid(),
            Self::And(values) | Self::Or(values) => values.iter().all(Self::valid),
            _ => true,
        }
    }
    fn evaluate(&self, feature: &impl Fn(&[C]) -> bool, selector: &impl Fn(&[C]) -> bool) -> bool {
        match self {
            Self::True => true,
            Self::False | Self::Invalid => false,
            Self::Not(c) => !c.evaluate(feature, selector),
            Self::And(v) => v.iter().all(|c| c.evaluate(feature, selector)),
            Self::Or(v) => v.iter().any(|c| c.evaluate(feature, selector)),
            Self::Feature(v) => feature(v),
            Self::Selector(v) => selector(v),
        }
    }
    pub fn matches_size(&self, width: f64, height: f64) -> bool {
        self.valid()
            && self.evaluate(
                &|values| media_feature_at(values, Some((width, height))),
                &|_| false,
            )
    }
    pub fn needs_block_size(&self) -> bool {
        match self{Self::Feature(values)=>values.iter().any(|value|matches!(value.kind(),Some(T::Ident(name)) if name.to_ascii_lowercase().contains("height")||name.to_ascii_lowercase().contains("aspect-ratio")||name.eq_ignore_ascii_case("orientation"))),Self::Not(value)=>value.needs_block_size(),Self::And(values)|Self::Or(values)=>values.iter().any(Self::needs_block_size),_=>false}
    }
    pub fn supports(&self) -> bool {
        self.valid()
            && self.evaluate(
                &|values| {
                    crate::parser::declaration(values).is_some_and(|d| supports_declaration(&d))
                },
                &|values| {
                    !crate::selector::parse_selector_list(&syntax::serialize(values)).is_empty()
                },
            )
    }
}

pub(crate) fn valid_computed_value(name: &str, value: &[Value]) -> bool {
    !value.is_empty()
        && supports_declaration(&crate::parser::Decl {
            name: name.to_string(),
            value: value.to_vec(),
            important: false,
        })
}

fn supports_declaration(decl: &crate::parser::Decl) -> bool {
    if decl.important {
        return false;
    }
    if decl.name.starts_with("--") {
        return true;
    }
    if !crate::style::is_known_property(&decl.name) {
        return false;
    }
    if crate::variables::deferred(&decl.value) {
        return true;
    }
    if decl.value.len() == 1
        && matches!(&decl.value[0],Value::Ident(s) if matches!(s.as_str(),"initial"|"inherit"|"unset"|"revert"|"revert-layer"))
    {
        return true;
    }
    let values = decl.value.as_slice();
    match decl.name.as_str() {
        "display" => matches!(values, [Value::Ident(name)] if matches!(name.as_str(),
            "none" | "block" | "inline" | "inline-block" | "flex" | "inline-flex" | "grid" | "inline-grid" |
            "table" | "inline-table" | "table-row" | "table-cell" | "contents" | "list-item")),
        "position" => {
            matches!(values, [Value::Ident(name)] if matches!(name.as_str(), "static" | "relative" | "absolute" | "fixed"))
        }
        name if name.ends_with("color") => {
            matches!(values, [Value::Color(..)])
                || matches!(values, [Value::Ident(name)] if name == "currentcolor")
        }
        name if length_property(name) => {
            let shorthand = name == "margin" || name == "padding";
            !values.is_empty()
                && values.len() <= if shorthand { 4 } else { 1 }
                && values.iter().all(|value| match value {
                    Value::Percentage(_) | Value::Number(0.0) => true,
                    Value::Length(..) | Value::Math(..) => {
                        crate::style::value_px(value, 16.0, 800.0).is_some()
                    }
                    Value::Ident(keyword) => match keyword.as_str() {
                        "auto" => !name.starts_with("padding"),
                        "none" => name.starts_with("max-"),
                        "min-content" | "max-content" | "fit-content" => {
                            name.ends_with("width") || name.ends_with("height")
                        }
                        _ => false,
                    },
                    _ => false,
                })
        }
        _ => !values.is_empty() && !values.iter().any(|value| matches!(value, Value::Token(_))),
    }
}

fn length_property(name: &str) -> bool {
    [
        "width",
        "height",
        "top",
        "right",
        "bottom",
        "left",
        "min-width",
        "min-height",
        "max-width",
        "max-height",
        "margin",
        "padding",
    ]
    .contains(&name)
        || name.starts_with("margin-")
        || name.starts_with("padding-")
}

pub fn media_matches(queries: &[crate::parser::MediaQuery]) -> bool {
    queries.iter().any(|q| {
        if !q.condition.valid() {
            return false;
        }
        let medium = q.medium.as_ref().is_none_or(|m| {
            m == "all"
                || crate::style::viewport::MEDIUM
                    .with(|current| current.borrow().eq_ignore_ascii_case(m))
        });
        let matched = medium && q.condition.evaluate(&media_feature, &|_| false);
        if q.negated {
            !matched
        } else {
            matched
        }
    })
}

fn actual_at(name: &str, size: Option<(f64, f64)>) -> Option<f64> {
    let (w, h) = size.unwrap_or_else(|| {
        (
            crate::style::viewport::WIDTH.get(),
            crate::style::viewport::HEIGHT.get(),
        )
    });
    match name {
        "width" | "device-width" => Some(w),
        "height" | "device-height" => Some(h),
        "aspect-ratio" | "device-aspect-ratio" => Some(w / h),
        "resolution" => Some(96.0),
        "color" => Some(8.0),
        "color-index" | "monochrome" => Some(0.0),
        _ => None,
    }
}
fn number(values: &[C], feature: &str) -> Option<f64> {
    let vals = crate::values::from_components(values);
    if feature.ends_with("aspect-ratio") {
        if let [Value::Number(a), Value::Ident(s), Value::Number(b)] = vals.as_slice() {
            if s == "/" && *a >= 0.0 && *b > 0.0 {
                return Some(a / b);
            }
        }
    }
    match vals.as_slice() {
        [Value::Number(n)]
            if feature.ends_with("aspect-ratio")
                || ["color", "color-index", "monochrome"].contains(&feature)
                || (*n == 0.0 && feature != "resolution") =>
        {
            Some(*n)
        }
        [Value::Length(n, unit)] if feature == "resolution" => match unit {
            Unit::Dpi => Some(*n),
            Unit::Dpcm => Some(n * 2.54),
            Unit::Dppx => Some(n * 96.0),
            _ => None,
        },
        [value @ Value::Length(..)]
            if matches!(
                feature,
                "width" | "height" | "device-width" | "device-height"
            ) =>
        {
            crate::style::value_px(value, 16.0, crate::style::viewport::WIDTH.get())
        }
        _ => None,
    }
}
fn media_feature(values: &[C]) -> bool {
    media_feature_at(values, None)
}
fn media_feature_at(values: &[C], size: Option<(f64, f64)>) -> bool {
    let actual = |name: &str| actual_at(name, size);
    let (width, height) = size.unwrap_or_else(|| {
        (
            crate::style::viewport::WIDTH.get(),
            crate::style::viewport::HEIGHT.get(),
        )
    });
    let values = syntax::trim(values);
    let colon = syntax::split(values, &T::Colon);
    if colon.len() == 2 {
        let [C::Token(token)] = syntax::trim(colon[0]) else {
            return false;
        };
        let T::Ident(name) = &token.kind else {
            return false;
        };
        let name = name.to_ascii_lowercase();
        let value = syntax::trim(colon[1]);
        if let [C::Token(token)] = value {
            if let T::Ident(v) = &token.kind {
                let v = v.to_ascii_lowercase();
                return match name.as_str() {
                    "orientation" => match v.as_str() {
                        "landscape" => width > height,
                        "portrait" => width <= height,
                        _ => false,
                    },
                    "prefers-color-scheme" => v == "light",
                    "prefers-reduced-motion" => v == "no-preference",
                    "hover" | "any-hover" => v == "hover",
                    "pointer" | "any-pointer" => v == "fine",
                    _ => false,
                };
            }
        }
        let (operator, name) = if let Some(name) = name.strip_prefix("min-") {
            ('>', name)
        } else if let Some(name) = name.strip_prefix("max-") {
            ('<', name)
        } else {
            ('=', name.as_str())
        };
        let Some(actual) = actual(name) else {
            return false;
        };
        let Some(value) = number(value, name) else {
            return false;
        };
        return match operator {
            '>' => actual >= value,
            '<' => actual <= value,
            _ => (actual - value).abs() < 1e-8,
        };
    }
    if let [C::Token(token)] = values {
        return if let T::Ident(name) = &token.kind {
            actual(&name.to_ascii_lowercase()).is_some_and(|n| n > 0.0)
        } else {
            false
        };
    }
    let mut pieces: Vec<&[C]> = Vec::new();
    let mut ops = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < values.len() {
        if let Some(T::Delim(c @ ('<' | '>' | '='))) = values[i].kind() {
            pieces.push(syntax::trim(&values[start..i]));
            let mut op = c.to_string();
            i += 1;
            if values.get(i).and_then(C::kind) == Some(&T::Delim('=')) {
                op.push('=');
                i += 1;
            }
            ops.push(op);
            start = i;
        } else {
            i += 1;
        }
    }
    pieces.push(syntax::trim(&values[start..]));
    if pieces.len() < 2 || pieces.len() > 3 {
        return false;
    }
    let feature = pieces.iter().find_map(|p| match *p {
        [C::Token(token)] => {
            if let T::Ident(name) = &token.kind {
                actual(&name.to_ascii_lowercase()).map(|_| name.to_ascii_lowercase())
            } else {
                None
            }
        }
        _ => None,
    });
    let Some(feature) = feature else { return false };
    let is_feature = |piece: &&[C]| matches!(*piece, [C::Token(token)] if matches!(&token.kind, T::Ident(s) if s.eq_ignore_ascii_case(&feature)));
    if pieces.iter().filter(|piece| is_feature(piece)).count() != 1 {
        return false;
    }
    if pieces.len() == 3
        && (!is_feature(&pieces[1])
            || ops[0].starts_with('=')
            || ops[1].starts_with('=')
            || ops[0].chars().next() != ops[1].chars().next())
    {
        return false;
    }

    let operands: Option<Vec<f64>> = pieces
        .iter()
        .map(|p| {
            if let [C::Token(token)] = *p {
                if matches!(&token.kind,T::Ident(s) if s.eq_ignore_ascii_case(&feature)) {
                    return actual(&feature);
                }
                number(p, &feature)
            } else {
                number(p, &feature)
            }
        })
        .collect();
    let Some(operands) = operands else {
        return false;
    };
    ops.iter().enumerate().all(|(i, op)| match op.as_str() {
        "<" => operands[i] < operands[i + 1],
        "<=" => operands[i] <= operands[i + 1],
        ">" => operands[i] > operands[i + 1],
        ">=" => operands[i] >= operands[i + 1],
        "=" => operands[i] == operands[i + 1],
        _ => false,
    })
}
