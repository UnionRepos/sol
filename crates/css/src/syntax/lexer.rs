use super::token::{ErrorKind, Numeric, ParseError, Token, TokenKind};

pub const MAX_TOKENS: usize = 1_000_000;

#[derive(Clone, Debug)]
pub struct Tokenized {
    pub source: String,
    pub tokens: Vec<Token>,
    pub errors: Vec<ParseError>,
}

pub fn tokenize(input: &str) -> Tokenized {
    tokenize_with_unicode_ranges(input, false)
}

pub fn tokenize_with_unicode_ranges(input: &str, unicode_ranges: bool) -> Tokenized {
    let source = input
        .replace("\r\n", "\n")
        .replace(['\r', '\u{c}'], "\n")
        .replace('\0', "\u{fffd}");
    let mut lexer = Lexer {
        source: &source,
        pos: 0,
        errors: Vec::new(),
    };
    let mut tokens = Vec::new();
    while lexer.peek().is_some() {
        if tokens.len() >= MAX_TOKENS {
            lexer.error(ErrorKind::LimitExceeded);
            break;
        }
        if lexer.rest().starts_with("/*") {
            lexer.pos += 2;
            if let Some(end) = lexer.rest().find("*/") {
                lexer.pos += end + 2;
            } else {
                lexer.error(ErrorKind::UnterminatedComment);
                lexer.pos = source.len();
            }
            continue;
        }
        let start = lexer.pos;
        let kind = lexer.token(unicode_ranges);
        tokens.push(Token {
            kind,
            start,
            end: lexer.pos,
        });
    }
    let errors = std::mem::take(&mut lexer.errors);
    Tokenized {
        source,
        tokens,
        errors,
    }
}

pub(super) fn whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n')
}
fn name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c >= '\u{80}'
}
fn name_char(c: char) -> bool {
    name_start(c) || c.is_ascii_digit() || c == '-'
}

struct Lexer<'a> {
    source: &'a str,
    pos: usize,
    errors: Vec<ParseError>,
}
impl Lexer<'_> {
    fn rest(&self) -> &str {
        &self.source[self.pos..]
    }
    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }
    fn nth(&self, n: usize) -> Option<char> {
        self.rest().chars().nth(n)
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }
    fn error(&mut self, kind: ErrorKind) {
        self.errors.push(ParseError {
            offset: self.pos,
            kind,
        });
    }
    fn escape_at(&self, n: usize) -> bool {
        self.nth(n) == Some('\\') && self.nth(n + 1) != Some('\n')
    }
    fn ident_at(&self, n: usize) -> bool {
        match self.nth(n) {
            Some('-') => {
                self.nth(n + 1).is_some_and(|c| name_start(c) || c == '-') || self.escape_at(n + 1)
            }
            Some(c) if name_start(c) => true,
            Some('\\') => self.escape_at(n),
            _ => false,
        }
    }
    fn number_at(&self) -> bool {
        match self.peek() {
            Some('+' | '-') => {
                self.nth(1).is_some_and(|c| c.is_ascii_digit())
                    || (self.nth(1) == Some('.') && self.nth(2).is_some_and(|c| c.is_ascii_digit()))
            }
            Some('.') => self.nth(1).is_some_and(|c| c.is_ascii_digit()),
            Some(c) => c.is_ascii_digit(),
            None => false,
        }
    }
    fn escape(&mut self) -> char {
        self.bump();
        if self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
            let mut number = 0u32;
            for _ in 0..6 {
                let Some(c) = self.peek().filter(|c| c.is_ascii_hexdigit()) else {
                    break;
                };
                number = number * 16 + c.to_digit(16).unwrap();
                self.bump();
            }
            if self.peek().is_some_and(whitespace) {
                self.bump();
            }
            char::from_u32(number)
                .filter(|&c| c != '\0')
                .unwrap_or('\u{fffd}')
        } else {
            self.bump().unwrap_or('\u{fffd}')
        }
    }
    fn name(&mut self) -> String {
        let mut out = String::new();
        loop {
            match self.peek() {
                Some(c) if name_char(c) => {
                    out.push(c);
                    self.bump();
                }
                Some('\\') if self.escape_at(0) => out.push(self.escape()),
                _ => break,
            }
        }
        out
    }
    fn quoted(&mut self, quote: char) -> TokenKind {
        self.bump();
        let mut out = String::new();
        loop {
            match self.peek() {
                Some(c) if c == quote => {
                    self.bump();
                    return TokenKind::String(out);
                }
                Some('\n') => {
                    self.error(ErrorKind::BadString);
                    return TokenKind::BadString;
                }
                Some('\\') if self.nth(1) == Some('\n') => {
                    self.bump();
                    self.bump();
                }
                Some('\\') if self.nth(1).is_none() => {
                    self.bump();
                }
                Some('\\') => out.push(self.escape()),
                Some(c) => {
                    out.push(c);
                    self.bump();
                }
                None => {
                    self.error(ErrorKind::UnterminatedString);
                    return TokenKind::String(out);
                }
            }
        }
    }
    fn numeric(&mut self) -> TokenKind {
        let start = self.pos;
        let mut integer = true;
        if matches!(self.peek(), Some('+' | '-')) {
            self.bump();
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
        }
        if self.peek() == Some('.') && self.nth(1).is_some_and(|c| c.is_ascii_digit()) {
            integer = false;
            self.bump();
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
        }
        let exponent = matches!(self.peek(), Some('e' | 'E'))
            && (self.nth(1).is_some_and(|c| c.is_ascii_digit())
                || (matches!(self.nth(1), Some('+' | '-'))
                    && self.nth(2).is_some_and(|c| c.is_ascii_digit())));
        if exponent {
            integer = false;
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
        }
        let representation = self.source[start..self.pos].to_string();
        let value = representation
            .parse::<f64>()
            .unwrap_or(0.0)
            .clamp(-f64::MAX, f64::MAX);
        let number = Numeric {
            value,
            integer,
            representation,
        };
        if self.ident_at(0) {
            TokenKind::Dimension(number, self.name())
        } else if self.peek() == Some('%') {
            self.bump();
            TokenKind::Percentage(number)
        } else {
            TokenKind::Number(number)
        }
    }
    fn bad_url(&mut self) -> TokenKind {
        self.error(ErrorKind::BadUrl);
        while let Some(c) = self.peek() {
            if c == ')' {
                self.bump();
                break;
            }
            if self.escape_at(0) {
                self.escape();
            } else {
                self.bump();
            }
        }
        TokenKind::BadUrl
    }
    fn url(&mut self) -> TokenKind {
        while self.peek().is_some_and(whitespace) {
            self.bump();
        }
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return TokenKind::Url(out),
                Some(')') => {
                    self.bump();
                    return TokenKind::Url(out);
                }
                Some(c) if whitespace(c) => {
                    while self.peek().is_some_and(whitespace) {
                        self.bump();
                    }
                    return match self.peek() {
                        None => TokenKind::Url(out),
                        Some(')') => {
                            self.bump();
                            TokenKind::Url(out)
                        }
                        _ => self.bad_url(),
                    };
                }
                Some(
                    '"' | '\'' | '(' | '\u{0}'..='\u{8}' | '\u{b}' | '\u{e}'..='\u{1f}' | '\u{7f}',
                ) => return self.bad_url(),
                Some('\\') if self.escape_at(0) => out.push(self.escape()),
                Some('\\') => return self.bad_url(),
                Some(c) => {
                    out.push(c);
                    self.bump();
                }
            }
        }
    }
    fn unicode_range(&mut self) -> TokenKind {
        self.bump();
        self.bump();
        let start = self.pos;
        while self.pos - start < 6 && self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
            self.bump();
        }
        let digits = &self.source[start..self.pos];
        let mut low = u32::from_str_radix(digits, 16).unwrap_or(0);
        let mut count = self.pos - start;
        let mut wild = false;
        while count < 6 && self.peek() == Some('?') {
            wild = true;
            low *= 16;
            self.bump();
            count += 1;
        }
        if wild {
            let bits = 4 * (count - digits.len());
            return TokenKind::UnicodeRange(low, low + ((1u32 << bits) - 1));
        }
        if self.peek() == Some('-') && self.nth(1).is_some_and(|c| c.is_ascii_hexdigit()) {
            self.bump();
            let start = self.pos;
            while self.pos - start < 6 && self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                self.bump();
            }
            return TokenKind::UnicodeRange(
                low,
                u32::from_str_radix(&self.source[start..self.pos], 16).unwrap(),
            );
        }
        TokenKind::UnicodeRange(low, low)
    }
    fn token(&mut self, unicode_ranges: bool) -> TokenKind {
        let c = self.peek().unwrap();
        if whitespace(c) {
            while self.peek().is_some_and(whitespace) {
                self.bump();
            }
            return TokenKind::Whitespace;
        }
        if self.rest().starts_with("<!--") {
            self.pos += 4;
            return TokenKind::Cdo;
        }
        if self.rest().starts_with("-->") {
            self.pos += 3;
            return TokenKind::Cdc;
        }
        if unicode_ranges
            && matches!(c, 'u' | 'U')
            && self.nth(1) == Some('+')
            && self
                .nth(2)
                .is_some_and(|c| c.is_ascii_hexdigit() || c == '?')
        {
            return self.unicode_range();
        }
        if self.number_at() {
            return self.numeric();
        }
        if matches!(c, '"' | '\'') {
            return self.quoted(c);
        }
        if c == '#' {
            self.bump();
            if self.peek().is_some_and(name_char) || self.escape_at(0) {
                let id = self.ident_at(0);
                return TokenKind::Hash {
                    value: self.name(),
                    id,
                };
            }
            return TokenKind::Delim('#');
        }
        if c == '@' && self.ident_at(1) {
            self.bump();
            return TokenKind::AtKeyword(self.name());
        }
        if self.ident_at(0) {
            let name = self.name();
            if self.peek() == Some('(') {
                self.bump();
                if name.eq_ignore_ascii_case("url") {
                    let next = self.rest().chars().find(|&c| !whitespace(c));
                    if !matches!(next, Some('"' | '\'')) {
                        return self.url();
                    }
                }
                return TokenKind::Function(name);
            }
            return TokenKind::Ident(name);
        }
        self.bump();
        match c {
            ':' => TokenKind::Colon,
            ';' => TokenKind::Semicolon,
            ',' => TokenKind::Comma,
            '(' | '[' | '{' => TokenKind::Open(c),
            ')' | ']' | '}' => TokenKind::Close(c),
            '\\' => {
                self.error(ErrorKind::InvalidEscape);
                TokenKind::Delim(c)
            }
            _ => TokenKind::Delim(c),
        }
    }
}
