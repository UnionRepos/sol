//! JS lexer.

#[derive(Clone, Debug, PartialEq)]
pub enum T {
    Num(f64),
    Str(String),
    Template(Vec<TPart>),
    Regex(String, String),
    Ident(String),
    Kw(&'static str),
    Punct(&'static str),
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TPart {
    Str(String),
    Expr(Vec<T>),
}

const PUNCTS: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
    "<<", ">>", "**", "{", "}", "(", ")", "[", "]", ";", ",", "<", ">", "+", "-", "*", "/", "%",
    "&", "|", "^", "!", "~", "?", ":", "=", ".",
];

const KWS: &[&str] = &[
    "var",
    "let",
    "const",
    "function",
    "return",
    "if",
    "else",
    "for",
    "while",
    "do",
    "break",
    "continue",
    "new",
    "delete",
    "typeof",
    "instanceof",
    "in",
    "of",
    "try",
    "catch",
    "finally",
    "throw",
    "switch",
    "case",
    "default",
    "class",
    "extends",
    "super",
    "this",
    "null",
    "true",
    "false",
    "undefined",
    "void",
    "yield",
    "async",
    "await",
    "static",
    "get",
    "set",
    "debugger",
];

pub struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
    /// regex allowed context: after operator/keyword/comma/paren
    pub allow_regex: bool,
}

pub fn lex(input: &str) -> Result<Vec<T>, String> {
    let mut lx = Lexer {
        s: input.as_bytes(),
        i: 0,
        allow_regex: true,
    };
    let mut out = Vec::new();
    loop {
        let t = lx.next()?;
        let is_eof = t == T::Eof;
        // track regex context
        lx.allow_regex = matches!(
            t,
            T::Punct("(")
                | T::Punct("=")
                | T::Punct(",")
                | T::Punct("[")
                | T::Punct("{")
                | T::Punct(";")
                | T::Punct(":")
                | T::Punct("!")
                | T::Punct("?")
                | T::Punct("&&")
                | T::Punct("||")
                | T::Punct("??")
                | T::Punct("return2")
                | T::Kw("return")
                | T::Kw("typeof")
                | T::Kw("case")
                | T::Kw("do")
                | T::Kw("else")
                | T::Kw("in")
                | T::Kw("of")
                | T::Kw("new")
                | T::Kw("throw")
                | T::Kw("void")
        );
        out.push(t);
        if is_eof {
            return Ok(out);
        }
    }
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn peek2(&self) -> Option<u8> {
        self.s.get(self.i + 1).copied()
    }
    fn bump(&mut self) -> Option<u8> {
        let c = self.peek();
        if c.is_some() {
            self.i += 1;
        }
        c
    }
    fn skip_ws_comments(&mut self) -> Result<(), String> {
        loop {
            match self.peek() {
                Some(c) if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' => {
                    self.i += 1;
                }
                Some(b'/') if self.peek2() == Some(b'/') => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.i += 1;
                    }
                }
                Some(b'/') if self.peek2() == Some(b'*') => {
                    self.i += 2;
                    loop {
                        match (self.peek(), self.peek2()) {
                            (Some(b'*'), Some(b'/')) => {
                                self.i += 2;
                                break;
                            }
                            (Some(_), _) => self.i += 1,
                            (None, _) => return Err("unterminated comment".into()),
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn next(&mut self) -> Result<T, String> {
        self.skip_ws_comments()?;
        let Some(c) = self.peek() else {
            return Ok(T::Eof);
        };
        // numbers
        if c.is_ascii_digit()
            || (c == b'.' && self.peek2().map(|d| d.is_ascii_digit()).unwrap_or(false))
        {
            return self.num();
        }
        // strings
        if c == b'"' || c == b'\'' {
            return self.string(c);
        }
        if c == b'`' {
            return self.template();
        }
        // ident/keyword
        if c == b'_' || c == b'$' || c.is_ascii_alphabetic() || c >= 0x80 {
            return self.ident();
        }
        // regex literal
        if c == b'/'
            && self.allow_regex
            && self.peek2() != Some(b'=')
            && self.peek2() != Some(b'/')
            && self.peek2() != Some(b'*')
        {
            return self.regex();
        }
        // punctuators
        for p in PUNCTS {
            if self.s[self.i..].starts_with(p.as_bytes()) {
                self.i += p.len();
                return Ok(T::Punct(p));
            }
        }
        Err(format!("lex: unexpected byte {:#x} at {}", c, self.i))
    }

    fn num(&mut self) -> Result<T, String> {
        let start = self.i;
        if self.peek() == Some(b'0') && matches!(self.peek2(), Some(b'x') | Some(b'X')) {
            self.i += 2;
            let hs = self.i;
            while let Some(c) = self.peek() {
                if c.is_ascii_hexdigit() {
                    self.i += 1;
                } else {
                    break;
                }
            }
            let v = u64::from_str_radix(&self.str_range(hs, self.i), 16)
                .map_err(|_| "bad hex".to_string())?;
            return Ok(T::Num(v as f64));
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == b'_' {
                self.i += 1;
            } else {
                break;
            }
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    self.i += 1;
                } else {
                    break;
                }
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            let save = self.i;
            self.i += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            let ds = self.i;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    self.i += 1;
                } else {
                    break;
                }
            }
            if self.i == ds {
                self.i = save;
            }
        }
        let v: f64 = self
            .str_range(start, self.i)
            .replace('_', "")
            .parse()
            .map_err(|_| "bad number".to_string())?;
        Ok(T::Num(v))
    }

    fn str_range(&self, a: usize, b: usize) -> String {
        String::from_utf8_lossy(&self.s[a..b]).into_owned()
    }

    fn string(&mut self, q: u8) -> Result<T, String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err("unterminated string".into()),
                Some(c) if c == q => return Ok(T::Str(out)),
                Some(b'\\') => match self.bump() {
                    Some(b'n') => out.push('\n'),
                    Some(b't') => out.push('\t'),
                    Some(b'r') => out.push('\r'),
                    Some(b'b') => out.push('\u{8}'),
                    Some(b'f') => out.push('\u{c}'),
                    Some(b'v') => out.push('\u{b}'),
                    Some(b'0') => out.push('\0'),
                    Some(b'x') => {
                        let h = self.str_range(self.i, (self.i + 2).min(self.s.len()));
                        self.i += 2;
                        out.push(
                            char::from_u32(u32::from_str_radix(&h, 16).unwrap_or(0))
                                .unwrap_or('\0'),
                        );
                    }
                    Some(b'u') => {
                        if self.peek() == Some(b'{') {
                            self.i += 1;
                            let st = self.i;
                            while self.peek() != Some(b'}') {
                                self.i += 1;
                            }
                            let h = self.str_range(st, self.i);
                            self.i += 1;
                            out.push(
                                char::from_u32(u32::from_str_radix(&h, 16).unwrap_or(0))
                                    .unwrap_or('\u{fffd}'),
                            );
                        } else {
                            let h = self.str_range(self.i, (self.i + 4).min(self.s.len()));
                            self.i += 4;
                            out.push(
                                char::from_u32(u32::from_str_radix(&h, 16).unwrap_or(0))
                                    .unwrap_or('\u{fffd}'),
                            );
                        }
                    }
                    Some(b'\n') => {}
                    Some(c) => out.push(c as char),
                    None => return Err("unterminated escape".into()),
                },
                Some(c) => {
                    // multi-byte utf8 passthrough
                    if c < 0x80 {
                        out.push(c as char);
                    } else {
                        let start = self.i - 1;
                        let mut end = self.i;
                        while end < self.s.len() && (self.s[end] & 0xC0) == 0x80 {
                            end += 1;
                        }
                        self.i = end;
                        out.push_str(&self.str_range(start, end));
                    }
                }
            }
        }
    }

    fn template(&mut self) -> Result<T, String> {
        // backtick template: return token parts with sub-token lists
        self.i += 1;
        let mut parts = Vec::new();
        let mut cur = String::new();
        loop {
            match self.bump() {
                None => return Err("unterminated template".into()),
                Some(b'`') => {
                    if !cur.is_empty() {
                        parts.push(TPart::Str(std::mem::take(&mut cur)));
                    }
                    return Ok(T::Template(parts));
                }
                Some(b'$') if self.peek() == Some(b'{') => {
                    self.i += 1;
                    if !cur.is_empty() {
                        parts.push(TPart::Str(std::mem::take(&mut cur)));
                    }
                    // lex inner expression until matching }
                    let mut depth = 1;
                    let start = self.i;
                    while depth > 0 {
                        match self.bump() {
                            None => return Err("unterminated template expr".into()),
                            Some(b'{') => depth += 1,
                            Some(b'}') => depth -= 1,
                            _ => {}
                        }
                    }
                    let inner = self.str_range(start, self.i - 1);
                    let toks = lex(&inner)?;
                    parts.push(TPart::Expr(toks));
                }
                Some(b'\\') => match self.bump() {
                    Some(b'n') => cur.push('\n'),
                    Some(b't') => cur.push('\t'),
                    Some(b'`') => cur.push('`'),
                    Some(b'$') => cur.push('$'),
                    Some(c) => {
                        cur.push('\\');
                        cur.push(c as char);
                    }
                    None => return Err("unterminated template escape".into()),
                },
                Some(c) => {
                    if c < 0x80 {
                        cur.push(c as char);
                    } else {
                        let start = self.i - 1;
                        let mut end = self.i;
                        while end < self.s.len() && (self.s[end] & 0xC0) == 0x80 {
                            end += 1;
                        }
                        self.i = end;
                        cur.push_str(&self.str_range(start, end));
                    }
                }
            }
        }
    }

    fn regex(&mut self) -> Result<T, String> {
        self.i += 1;
        let mut body = String::new();
        let mut in_class = false;
        loop {
            match self.bump() {
                None => return Err("unterminated regex".into()),
                Some(b'\\') => {
                    body.push('\\');
                    if let Some(c) = self.bump() {
                        body.push(c as char);
                    }
                }
                Some(b'[') => {
                    in_class = true;
                    body.push('[');
                }
                Some(b']') => {
                    in_class = false;
                    body.push(']');
                }
                Some(b'/') if !in_class => break,
                Some(c) => body.push(c as char),
            }
        }
        let mut flags = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphabetic() {
                flags.push(c as char);
                self.i += 1;
            } else {
                break;
            }
        }
        Ok(T::Regex(body, flags))
    }

    fn ident(&mut self) -> Result<T, String> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c == b'_' || c == b'$' || c.is_ascii_alphanumeric() || c >= 0x80 {
                self.i += 1;
            } else {
                break;
            }
        }
        let s = self.str_range(start, self.i);
        if let Some(kw) = KWS.iter().find(|k| **k == s) {
            return Ok(T::Kw(kw));
        }
        Ok(T::Ident(s))
    }
}
