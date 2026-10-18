use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub struct Numeric {
    pub value: f64,
    pub integer: bool,
    pub representation: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    Ident(String),
    Function(String),
    AtKeyword(String),
    Hash { value: String, id: bool },
    String(String),
    BadString,
    Url(String),
    BadUrl,
    Number(Numeric),
    Percentage(Numeric),
    Dimension(Numeric, String),
    UnicodeRange(u32, u32),
    Whitespace,
    Colon,
    Semicolon,
    Comma,
    Open(char),
    Close(char),
    Delim(char),
    Cdo,
    Cdc,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    UnterminatedComment,
    UnterminatedString,
    BadString,
    BadUrl,
    InvalidEscape,
    UnexpectedClose,
    UnclosedBlock,
    LimitExceeded,
    InvalidRule,
    InvalidDeclaration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub offset: usize,
    pub kind: ErrorKind,
}

pub fn escape_ident(value: &str) -> String {
    let chars: Vec<_> = value.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == '\0' {
            out.push('\u{fffd}');
        } else if c.is_control() || (c.is_ascii_digit() && (i == 0 || (i == 1 && chars[0] == '-')))
        {
            write!(out, "\\{:x} ", c as u32).unwrap();
        } else if c == '-' && chars.len() == 1 {
            out.push_str("\\-");
        } else if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') || c >= '\u{80}' {
            out.push(c);
        } else {
            out.push('\\');
            out.push(c);
        }
    }
    out
}

fn string(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => {
                write!(out, "\\{:x} ", c as u32).unwrap();
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl TokenKind {
    pub fn to_css(&self) -> String {
        match self {
            Self::Ident(s) => escape_ident(s),
            Self::Function(s) => format!("{}(", escape_ident(s)),
            Self::AtKeyword(s) => format!("@{}", escape_ident(s)),
            Self::Hash { value, id } => {
                let value = if *id {
                    escape_ident(value)
                } else {
                    let mut out = String::new();
                    for c in value.chars() {
                        if c.is_control() {
                            write!(out, "\\{:x} ", c as u32).unwrap();
                        } else if c.is_ascii_alphanumeric()
                            || matches!(c, '-' | '_')
                            || c >= '\u{80}'
                        {
                            out.push(c);
                        } else {
                            out.push('\\');
                            out.push(c);
                        }
                    }
                    out
                };
                format!("#{value}")
            }
            Self::String(s) => string(s),
            Self::BadString => "\"\n".into(),
            Self::Url(s) => {
                let mut value = String::new();
                for c in s.chars() {
                    if c.is_control() || matches!(c, ' ' | '"' | '\'' | '(' | ')' | '\\') {
                        write!(value, "\\{:x} ", c as u32).unwrap();
                    } else {
                        value.push(c);
                    }
                }
                format!("url({value})")
            }
            Self::BadUrl => "url(\"\n)".into(),
            Self::Number(n) => n.representation.clone(),
            Self::Percentage(n) => format!("{}%", n.representation),
            Self::Dimension(n, unit) => {
                let mut unit = escape_ident(unit);
                // An exponent-looking unit would otherwise be absorbed into the
                // numeric token when the serialized stylesheet is parsed again.
                if matches!(unit.as_bytes().first(), Some(b'e' | b'E'))
                    && (unit.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
                        || matches!(unit.as_bytes().get(1), Some(b'+' | b'-'))
                            && unit.as_bytes().get(2).is_some_and(u8::is_ascii_digit))
                {
                    unit = format!("\\{:x} {}", unit.as_bytes()[0], &unit[1..]);
                }
                format!("{}{unit}", n.representation)
            }
            Self::UnicodeRange(start, end) => format!("U+{start:X}-{end:X}"),
            Self::Whitespace => " ".into(),
            Self::Colon => ":".into(),
            Self::Semicolon => ";".into(),
            Self::Comma => ",".into(),
            Self::Open(c) | Self::Close(c) | Self::Delim(c) => c.to_string(),
            Self::Cdo => "<!--".into(),
            Self::Cdc => "-->".into(),
        }
    }
}
