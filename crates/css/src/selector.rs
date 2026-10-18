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
}

#[derive(Debug, Clone)]
pub struct AttrSel {
    pub name: String,
    pub op: AttrOp,
    pub value: String,
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

pub fn parse_selector_list(s: &str) -> Vec<Selector> {
    // split on commas not inside parens/brackets
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                if let Some(sel) = parse_selector(cur.trim()) {
                    out.push(sel);
                }
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if let Some(sel) = parse_selector(cur.trim()) {
        out.push(sel);
    }
    out
}

pub fn parse_selector(s: &str) -> Option<Selector> {
    // tokenize left-to-right then reverse
    let mut parts: Vec<Compound> = Vec::new();
    let mut combs: Vec<Combinator> = Vec::new();
    let mut i = 0;
    let b: Vec<char> = s.chars().collect();
    let mut pending_comb: Option<Combinator> = None;
    let mut saw_space = false;
    while i < b.len() {
        match b[i] {
            c if c.is_whitespace() => {
                saw_space = true;
                i += 1;
            }
            '>' | '+' | '~' => {
                pending_comb = Some(match b[i] {
                    '>' => Combinator::Child,
                    '+' => Combinator::NextSibling,
                    _ => Combinator::LaterSibling,
                });
                saw_space = false;
                i += 1;
            }
            _ => {
                let (comp, ni) = parse_compound(&b, i);
                i = ni;
                if !parts.is_empty() {
                    combs.push(pending_comb.take().unwrap_or_else(|| {
                        if saw_space {
                            Combinator::Descendant
                        } else {
                            Combinator::Descendant
                        }
                    }));
                }
                pending_comb = None;
                saw_space = false;
                parts.push(comp);
            }
        }
    }
    if parts.is_empty() {
        return None;
    }
    // compute specificity of subject chain
    let mut spec = (0u32, 0u32, 0u32);
    for c in &parts {
        let (a, b2, c2) = c.specificity();
        spec.0 += a;
        spec.1 += b2;
        spec.2 += c2;
    }
    // reverse to subject-first order
    parts.reverse();
    combs.reverse();
    Some(Selector {
        parts,
        combinators: combs,
        specificity: spec,
    })
}

fn parse_compound(b: &[char], mut i: usize) -> (Compound, usize) {
    let mut c = Compound {
        tag: None,
        id: None,
        classes: Vec::new(),
        attrs: Vec::new(),
        pseudos: Vec::new(),
    };
    // tag or *
    if i < b.len()
        && (b[i] == '*'
            || b[i].is_alphanumeric()
            || b[i] == '_'
            || b[i] == '-'
            || b[i] >= '\u{80}'
            || b[i] == '|')
    {
        if b[i] == '*' {
            i += 1;
            // no tag constraint
        } else {
            let start = i;
            while i < b.len()
                && (b[i].is_alphanumeric() || matches!(b[i], '-' | '_' | '|') || b[i] >= '\u{80}')
            {
                i += 1;
            }
            let mut t: String = b[start..i].iter().collect();
            if let Some(idx) = t.find('|') {
                t = t[idx + 1..].to_string();
            }
            c.tag = Some(t.to_ascii_lowercase());
        }
    }
    while i < b.len() {
        match b[i] {
            '#' => {
                i += 1;
                let (name, ni) = read_name(b, i);
                i = ni;
                c.id = Some(name);
            }
            '.' => {
                i += 1;
                let (name, ni) = read_name(b, i);
                i = ni;
                c.classes.push(name);
            }
            '[' => {
                i += 1;
                // attr selector
                let mut name = String::new();
                let mut value = String::new();
                let mut op = AttrOp::Exists;
                let mut in_name = true;
                while i < b.len() && b[i] != ']' {
                    if in_name {
                        match b[i] {
                            ' ' | '\t' => {}
                            '=' => {
                                op = AttrOp::Eq;
                                in_name = false;
                            }
                            '~' => {
                                op = AttrOp::Includes;
                                i += 1;
                                in_name = false;
                            }
                            '|' => {
                                op = AttrOp::DashMatch;
                                i += 1;
                                in_name = false;
                            }
                            '^' => {
                                op = AttrOp::Prefix;
                                i += 1;
                                in_name = false;
                            }
                            '$' => {
                                op = AttrOp::Suffix;
                                i += 1;
                                in_name = false;
                            }
                            '*' => {
                                op = AttrOp::Substr;
                                i += 1;
                                in_name = false;
                            }
                            ch => name.push(ch.to_ascii_lowercase()),
                        }
                    } else if !matches!(b[i], '"' | '\'') {
                        if !b[i].is_whitespace() {
                            value.push(b[i]);
                        }
                    }
                    i += 1;
                }
                if i < b.len() {
                    i += 1; // ']'
                }
                let value = value.trim_matches('"').trim_matches('\'').to_string();
                c.attrs.push(AttrSel {
                    name: name.trim().to_string(),
                    op,
                    value,
                });
            }
            ':' => {
                i += 1;
                if i < b.len() && b[i] == ':' {
                    i += 1; // ::pseudo-element — treat as unknown
                }
                let (name, ni) = read_name(b, i);
                i = ni;
                let lname = name.to_ascii_lowercase();
                let mut arg = String::new();
                if i < b.len() && b[i] == '(' {
                    i += 1;
                    let mut depth = 1;
                    let start = i;
                    while i < b.len() && depth > 0 {
                        if b[i] == '(' {
                            depth += 1;
                        } else if b[i] == ')' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        i += 1;
                    }
                    arg = b[start..i].iter().collect();
                    if i < b.len() {
                        i += 1;
                    }
                }
                let p = match lname.as_str() {
                    "first-child" => Pseudo::FirstChild,
                    "last-child" => Pseudo::LastChild,
                    "only-child" => Pseudo::OnlyChild,
                    "first-of-type" => Pseudo::FirstOfType,
                    "last-of-type" => Pseudo::LastOfType,
                    "only-of-type" => Pseudo::OnlyOfType,
                    "nth-child" => Pseudo::NthChild(parse_nth(&arg)),
                    "nth-of-type" => Pseudo::NthOfType(parse_nth(&arg)),
                    "nth-last-child" => Pseudo::NthLastChild(parse_nth(&arg)),
                    "empty" => Pseudo::Empty,
                    "root" => Pseudo::Root,
                    "not" => {
                        Pseudo::Not(Box::new(parse_selector(&arg).unwrap_or_else(|| Selector {
                            parts: vec![Compound {
                                tag: Some("*".into()),
                                id: None,
                                classes: vec![],
                                attrs: vec![],
                                pseudos: vec![],
                            }],
                            combinators: vec![],
                            specificity: (0, 0, 0),
                        })))
                    }
                    "is" | "where" | "matches" => {
                        Pseudo::Is(Box::new(parse_selector(&arg).unwrap_or_else(|| Selector {
                            parts: vec![Compound {
                                tag: Some("*".into()),
                                id: None,
                                classes: vec![],
                                attrs: vec![],
                                pseudos: vec![],
                            }],
                            combinators: vec![],
                            specificity: (0, 0, 0),
                        })))
                    }
                    "has" => {
                        Pseudo::Has(Box::new(parse_selector(&arg).unwrap_or_else(|| Selector {
                            parts: vec![Compound {
                                tag: Some("*".into()),
                                id: None,
                                classes: vec![],
                                attrs: vec![],
                                pseudos: vec![],
                            }],
                            combinators: vec![],
                            specificity: (0, 0, 0),
                        })))
                    }
                    "hover" => Pseudo::Hover,
                    "active" => Pseudo::Active,
                    "focus" => Pseudo::Focus,
                    "visited" => Pseudo::Visited,
                    "link" => Pseudo::Link,
                    "checked" => Pseudo::Checked,
                    "disabled" => Pseudo::Disabled,
                    "enabled" => Pseudo::Enabled,
                    "target" => Pseudo::Target,
                    other => Pseudo::Unknown(other.to_string()),
                };
                c.pseudos.push(p);
            }
            _ => break,
        }
    }
    (c, i)
}

fn read_name(b: &[char], mut i: usize) -> (String, usize) {
    let start = i;
    while i < b.len()
        && (b[i].is_alphanumeric() || matches!(b[i], '-' | '_') || b[i] >= '\u{80}' || b[i] == '\\')
    {
        if b[i] == '\\' {
            i += 1;
        }
        i += 1;
    }
    (b[start..i].iter().collect(), i)
}

fn parse_nth(arg: &str) -> NthArg {
    let a = arg.trim().to_ascii_lowercase();
    if a == "odd" {
        return NthArg::Odd;
    }
    if a == "even" {
        return NthArg::Even;
    }
    if let Ok(n) = a.parse::<i32>() {
        return NthArg::N(n);
    }
    // an+b
    let a = a.replace(' ', "");
    if let Some(npos) = a.find('n') {
        let (astr, bstr) = a.split_at(npos);
        let a_coef = match astr {
            "" | "+" => 1,
            "-" => -1,
            x => x.parse().unwrap_or(0),
        };
        let b_val: i32 = bstr[1..].parse().unwrap_or(0);
        NthArg::AnB(a_coef, b_val)
    } else {
        NthArg::N(0)
    }
}

impl Compound {
    pub fn specificity(&self) -> (u32, u32, u32) {
        let mut a = 0;
        let mut b = 0;
        let mut c = 0;
        if self.id.is_some() {
            a += 1;
        }
        b += self.classes.len() as u32;
        b += self.attrs.len() as u32;
        for p in &self.pseudos {
            match p {
                Pseudo::Not(s) | Pseudo::Is(s) => {
                    let (sa, sb, sc) = s.specificity;
                    a += sa;
                    b += sb;
                    c += sc;
                }
                Pseudo::Has(s) => {
                    let (sa, sb, sc) = s.specificity;
                    a += sa;
                    b += sb;
                    c += sc;
                }
                _ => b += 1,
            }
        }
        if self.tag.is_some() {
            c += 1;
        }
        (a, b, c)
    }
}
