//! Keeping token boundaries separate from property interpretation lets unknown
//! CSS survive parsing without being mistaken for syntax the renderer supports.
//!
//! ```
//! use sol_css::syntax::{parse_stylesheet, parse_declaration, Rule};
//! let sheet = parse_stylesheet("@future x(y) { --Theme: { accent: blue }; }");
//! assert!(sheet.errors.is_empty());
//! let Rule::At(rule) = &sheet.rules[0] else { panic!("expected at-rule") };
//! assert_eq!(rule.name, "future");
//! let declaration = parse_declaration("--Theme: 'a;}';");
//! assert!(declaration.is_err());
//! let declaration = parse_declaration("--Theme: 'a;}' ! /**/ important").unwrap();
//! assert!(declaration.important);
//! assert_eq!(declaration.name, "--Theme");
//! ```
pub(crate) mod declaration;
mod lexer;
mod token;

pub use declaration::{
    parse_block_contents, parse_declaration, BlockContents, BlockItem, Declaration,
};
pub use lexer::{tokenize, tokenize_with_unicode_ranges, Tokenized, MAX_TOKENS};
pub use token::{escape_ident, ErrorKind, Numeric, ParseError, Token, TokenKind};

pub const MAX_NESTING: usize = 128;

#[derive(Clone, Debug, PartialEq)]
pub enum ComponentValue {
    Token(Token),
    Function {
        name: String,
        value: Vec<ComponentValue>,
    },
    Block {
        opening: char,
        value: Vec<ComponentValue>,
    },
}

impl ComponentValue {
    pub fn kind(&self) -> Option<&TokenKind> {
        if let Self::Token(token) = self {
            Some(&token.kind)
        } else {
            None
        }
    }
    pub fn is_whitespace(&self) -> bool {
        matches!(self.kind(), Some(TokenKind::Whitespace))
    }
    pub fn to_css(&self) -> String {
        match self {
            Self::Token(t) => t.kind.to_css(),
            Self::Function { name, value } => {
                format!("{}({})", escape_ident(name), serialize(value))
            }
            Self::Block { opening, value } => {
                format!("{}{}{}", opening, serialize(value), closing(*opening))
            }
        }
    }
}

pub fn serialize(values: &[ComponentValue]) -> String {
    // Removing comments must not merge adjacent tokens into a new identifier,
    // number, or function. A comment separator preserves boundaries without
    // inventing whitespace that could introduce a descendant combinator.
    values
        .iter()
        .map(ComponentValue::to_css)
        .collect::<Vec<_>>()
        .join("/**/")
}

pub fn trim(values: &[ComponentValue]) -> &[ComponentValue] {
    let start = values
        .iter()
        .position(|v| !v.is_whitespace())
        .unwrap_or(values.len());
    let end = values
        .iter()
        .rposition(|v| !v.is_whitespace())
        .map_or(start, |i| i + 1);
    &values[start..end]
}

pub fn split<'a>(values: &'a [ComponentValue], token: &TokenKind) -> Vec<&'a [ComponentValue]> {
    values.split(|v| v.kind() == Some(token)).collect()
}

fn closing(c: char) -> char {
    match c {
        '(' => ')',
        '[' => ']',
        _ => '}',
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComponentList {
    pub values: Vec<ComponentValue>,
    pub errors: Vec<ParseError>,
}

pub fn parse_component_values(css: &str) -> ComponentList {
    let Tokenized {
        tokens, mut errors, ..
    } = tokenize(css);
    enum Open {
        Root,
        Function(String),
        Block(char),
    }
    struct Frame {
        open: Open,
        start: usize,
        values: Vec<ComponentValue>,
    }
    // Explicit frames avoid using the Rust call stack for untrusted nesting;
    // the cap also keeps later recursive interpretation and destruction bounded.
    let mut stack = vec![Frame {
        open: Open::Root,
        start: 0,
        values: Vec::new(),
    }];
    let mut overflow = 0usize;
    for token in tokens {
        let opening = match &token.kind {
            TokenKind::Function(name) => Some(Open::Function(name.clone())),
            TokenKind::Open(c) => Some(Open::Block(*c)),
            _ => None,
        };
        if overflow > 0 {
            match token.kind {
                TokenKind::Function(_) | TokenKind::Open(_) => overflow += 1,
                TokenKind::Close(_) => overflow -= 1,
                _ => {}
            }
            continue;
        }
        if let Some(open) = opening {
            if stack.len() > MAX_NESTING {
                errors.push(ParseError {
                    offset: token.start,
                    kind: ErrorKind::LimitExceeded,
                });
                overflow = 1;
            } else {
                stack.push(Frame {
                    open,
                    start: token.start,
                    values: Vec::new(),
                });
            }
            continue;
        }
        if let TokenKind::Close(c) = token.kind {
            let matches = match stack.last().unwrap().open {
                Open::Function(_) => c == ')',
                Open::Block(open) => c == closing(open),
                Open::Root => false,
            };
            if matches {
                let frame = stack.pop().unwrap();
                let component = match frame.open {
                    Open::Function(name) => ComponentValue::Function {
                        name,
                        value: frame.values,
                    },
                    Open::Block(opening) => ComponentValue::Block {
                        opening,
                        value: frame.values,
                    },
                    Open::Root => unreachable!(),
                };
                stack.last_mut().unwrap().values.push(component);
                continue;
            }
            errors.push(ParseError {
                offset: token.start,
                kind: ErrorKind::UnexpectedClose,
            });
        }
        stack
            .last_mut()
            .unwrap()
            .values
            .push(ComponentValue::Token(token));
    }
    while stack.len() > 1 {
        let frame = stack.pop().unwrap();
        errors.push(ParseError {
            offset: frame.start,
            kind: ErrorKind::UnclosedBlock,
        });
        let component = match frame.open {
            Open::Function(name) => ComponentValue::Function {
                name,
                value: frame.values,
            },
            Open::Block(opening) => ComponentValue::Block {
                opening,
                value: frame.values,
            },
            Open::Root => unreachable!(),
        };
        stack.last_mut().unwrap().values.push(component);
    }
    ComponentList {
        values: stack.pop().unwrap().values,
        errors,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AtRule {
    pub name: String,
    pub prelude: Vec<ComponentValue>,
    pub block: Option<Vec<ComponentValue>>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct QualifiedRule {
    pub prelude: Vec<ComponentValue>,
    pub block: Vec<ComponentValue>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    At(AtRule),
    Qualified(QualifiedRule),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    pub errors: Vec<ParseError>,
}

pub fn parse_stylesheet(css: &str) -> Stylesheet {
    let components = parse_component_values(css);
    Stylesheet {
        rules: into_rule_list(components.values, true),
        errors: components.errors,
    }
}

pub fn parse_rule_list(values: &[ComponentValue], top_level: bool) -> Vec<Rule> {
    into_rule_list(values.to_vec(), top_level)
}

pub fn into_rule_list(values: Vec<ComponentValue>, top_level: bool) -> Vec<Rule> {
    let mut input = values.into_iter();
    let mut rules = Vec::new();
    while let Some(first) = input.next() {
        if first.is_whitespace()
            || (top_level && matches!(first.kind(), Some(TokenKind::Cdo | TokenKind::Cdc)))
        {
            continue;
        }
        if let ComponentValue::Block {
            opening: '{',
            value,
        } = first
        {
            rules.push(Rule::Qualified(QualifiedRule {
                prelude: Vec::new(),
                block: value,
            }));
            continue;
        }
        let mut prelude = Vec::new();
        let at = if let ComponentValue::Token(Token {
            kind: TokenKind::AtKeyword(name),
            ..
        }) = first
        {
            Some(name)
        } else {
            prelude.push(first);
            None
        };
        loop {
            match input.next() {
                Some(ComponentValue::Block {
                    opening: '{',
                    value,
                }) => {
                    let prelude = trim(&prelude).to_vec();
                    if let Some(name) = at {
                        rules.push(Rule::At(AtRule {
                            name,
                            prelude,
                            block: Some(value),
                        }));
                    } else {
                        rules.push(Rule::Qualified(QualifiedRule {
                            prelude,
                            block: value,
                        }));
                    }
                    break;
                }
                Some(value) if at.is_some() && value.kind() == Some(&TokenKind::Semicolon) => {
                    rules.push(Rule::At(AtRule {
                        name: at.unwrap(),
                        prelude: trim(&prelude).to_vec(),
                        block: None,
                    }));
                    break;
                }
                Some(value) => prelude.push(value),
                None => {
                    if let Some(name) = at {
                        rules.push(Rule::At(AtRule {
                            name,
                            prelude: trim(&prelude).to_vec(),
                            block: None,
                        }));
                    }
                    break;
                }
            }
        }
    }
    rules
}

impl Rule {
    pub fn to_css(&self) -> String {
        match self {
            Self::At(rule) => {
                let prelude = serialize(&rule.prelude);
                let head = format!("@{} {}", escape_ident(&rule.name), prelude);
                match &rule.block {
                    Some(block) => format!("{head}{{{}}}", serialize(block)),
                    None => format!("{head};"),
                }
            }
            Self::Qualified(rule) => {
                format!("{}{{{}}}", serialize(&rule.prelude), serialize(&rule.block))
            }
        }
    }
}

pub fn parse_component_value(css: &str) -> Result<ComponentValue, Vec<ParseError>> {
    let parsed = parse_component_values(css);
    let values = trim(&parsed.values);
    if values.len() == 1
        && !parsed.errors.iter().any(|e| {
            matches!(
                e.kind,
                ErrorKind::BadString
                    | ErrorKind::BadUrl
                    | ErrorKind::UnexpectedClose
                    | ErrorKind::LimitExceeded
            )
        })
    {
        Ok(values[0].clone())
    } else {
        Err(if parsed.errors.is_empty() {
            vec![ParseError {
                offset: 0,
                kind: ErrorKind::InvalidRule,
            }]
        } else {
            parsed.errors
        })
    }
}
pub fn parse_rule(css: &str) -> Result<Rule, Vec<ParseError>> {
    let parsed = parse_component_values(css);
    let values = trim(&parsed.values);
    let at = matches!(
        values.first().and_then(ComponentValue::kind),
        Some(TokenKind::AtKeyword(_))
    );
    let ending = values.iter().position(|v| {
        matches!(v, ComponentValue::Block { opening: '{', .. })
            || at && v.kind() == Some(&TokenKind::Semicolon)
    });
    if ending.is_some_and(|i| !trim(&values[i + 1..]).is_empty()) {
        return Err(vec![ParseError {
            offset: 0,
            kind: ErrorKind::InvalidRule,
        }]);
    }
    let mut rules = parse_rule_list(values, false);
    if rules.len() == 1 {
        Ok(rules.pop().unwrap())
    } else {
        Err(vec![ParseError {
            offset: 0,
            kind: ErrorKind::InvalidRule,
        }])
    }
}
