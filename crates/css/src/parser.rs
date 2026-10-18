//! CSS stylesheet parser — rules, declarations, at-rules, values.

use crate::values::Value;

#[derive(Debug, Clone)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub enum Rule {
    Style(StyleRule),
    Media(MediaRule),
    FontFace(Vec<Decl>),
    /// @import url — kernel resolves and splices
    Import(String),
    /// @charset, @namespace, unknown at-rules
    Other(String),
}

#[derive(Debug, Clone)]
pub struct MediaRule {
    /// simplified: list of media queries (comma separated), each is a
    /// conjunction of (feature:value) or media-type keywords
    pub queries: Vec<MediaQuery>,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub struct MediaQuery {
    pub medium: Option<String>,
    /// (feature, value) e.g. ("max-width", "800px")
    pub features: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct StyleRule {
    pub selectors: Vec<crate::selector::Selector>,
    pub decls: Vec<Decl>,
}

#[derive(Debug, Clone)]
pub struct Decl {
    pub name: String,
    pub value: Vec<Value>,
    pub important: bool,
}

pub fn parse(css: &str) -> Stylesheet {
    let mut p = P { s: css, i: 0 };
    let mut rules = Vec::new();
    loop {
        p.skip_ws_comments();
        if p.eof() {
            break;
        }
        if p.peek() == Some('@') {
            p.at_rule(&mut rules);
        } else if let Some(r) = p.style_rule() {
            rules.push(r);
        }
    }
    Stylesheet { rules }
}

/// Parse a declaration block (used for style="" attributes).
pub fn parse_decls(s: &str) -> Vec<Decl> {
    let mut p = P { s, i: 0 };
    p.decls_until(None)
}

struct P<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> P<'a> {
    fn eof(&self) -> bool {
        self.i >= self.s.len()
    }
    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }
    #[allow(dead_code)]
    fn peek_at(&self, off: usize) -> Option<char> {
        self.s[self.i..].chars().nth(off)
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += c.len_utf8();
        Some(c)
    }
    fn skip_ws_comments(&mut self) {
        loop {
            while let Some(c) = self.peek() {
                if !c.is_whitespace() {
                    break;
                }
                self.i += c.len_utf8();
            }
            if self.s[self.i..].starts_with("/*") {
                if let Some(end) = self.s[self.i + 2..].find("*/") {
                    self.i += end + 4;
                } else {
                    self.i = self.s.len();
                }
            } else {
                break;
            }
        }
    }
    fn ident(&mut self) -> String {
        let start = self.i;
        if self.peek() == Some('-') {
            self.i += 1;
        }
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '-' || c == '_' || c >= '\u{80}' {
                self.i += c.len_utf8();
            } else if c == '\\' {
                self.i += 1;
                if self.peek().is_some() {
                    self.i += self.peek().unwrap().len_utf8();
                }
            } else {
                break;
            }
        }
        self.s[start..self.i].to_string()
    }

    fn eat_char(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn at_rule(&mut self, rules: &mut Vec<Rule>) {
        self.bump(); // '@'
        let name = self.ident().to_ascii_lowercase();
        self.skip_ws_comments();
        match name.as_str() {
            "media" | "supports" => {
                // read prelude to '{'
                let start = self.i;
                while let Some(c) = self.peek() {
                    if c == '{' || c == ';' {
                        break;
                    }
                    self.i += c.len_utf8();
                }
                let prelude = self.s[start..self.i].trim().to_string();
                if self.peek() == Some('{') {
                    self.bump();
                    let mut inner = Vec::new();
                    loop {
                        self.skip_ws_comments();
                        if self.eof() || self.peek() == Some('}') {
                            self.bump();
                            break;
                        }
                        if self.peek() == Some('@') {
                            self.at_rule(&mut inner);
                        } else if let Some(r) = self.style_rule() {
                            inner.push(r);
                        }
                    }
                    if name == "media" {
                        rules.push(Rule::Media(MediaRule {
                            queries: parse_media(&prelude),
                            rules: inner,
                        }));
                    } else {
                        // @supports: evaluate statically — unsupported → skip,
                        // but conservatively include if it mentions any feature
                        if supports_query(&prelude) {
                            rules.extend(inner);
                        }
                    }
                } else {
                    self.bump(); // ';'
                }
            }
            "import" => {
                let start = self.i;
                while let Some(c) = self.peek() {
                    if c == ';' {
                        break;
                    }
                    self.i += c.len_utf8();
                }
                let pre = self.s[start..self.i].trim().to_string();
                self.bump();
                if let Some(url) = extract_url(&pre) {
                    rules.push(Rule::Import(url));
                }
            }
            "font-face" => {
                self.skip_ws_comments();
                if self.peek() == Some('{') {
                    self.bump();
                    let decls = self.decls_until(Some('}'));
                    rules.push(Rule::FontFace(decls));
                }
            }
            "charset" | "namespace" => {
                // skip to ';'
                while let Some(c) = self.bump() {
                    if c == ';' {
                        break;
                    }
                }
            }
            _ => {
                // unknown: skip to matching block or ';'
                let mut depth = 0;
                while let Some(c) = self.bump() {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth <= 0 {
                                break;
                            }
                        }
                        ';' if depth == 0 => break,
                        _ => {}
                    }
                }
            }
        }
    }

    fn style_rule(&mut self) -> Option<Rule> {
        // read selector prelude to '{'
        let start = self.i;
        let mut depth_paren = 0;
        while let Some(c) = self.peek() {
            match c {
                '(' => depth_paren += 1,
                ')' => depth_paren -= 1,
                '{' if depth_paren <= 0 => break,
                ';' if depth_paren <= 0 => {
                    // malformed: skip
                    self.i += c.len_utf8();
                    return None;
                }
                _ => {}
            }
            self.i += c.len_utf8();
        }
        if self.peek() != Some('{') {
            return None;
        }
        let sel_text = self.s[start..self.i].trim().to_string();
        self.bump(); // '{'
        let decls = self.decls_until(Some('}'));
        let selectors = crate::selector::parse_selector_list(&sel_text);
        if selectors.is_empty() {
            return None;
        }
        Some(Rule::Style(StyleRule { selectors, decls }))
    }

    /// Parse declarations until '}' or EOF.
    fn decls_until(&mut self, end: Option<char>) -> Vec<Decl> {
        let mut out = Vec::new();
        loop {
            self.skip_ws_comments();
            match self.peek() {
                None => break,
                Some(c) if Some(c) == end => {
                    self.bump();
                    break;
                }
                Some(';') => {
                    self.bump();
                }
                _ => {
                    let name = self.ident().to_ascii_lowercase();
                    self.skip_ws_comments();
                    if !self.eat_char(':') {
                        // garbage — skip to ; or }
                        while let Some(c) = self.bump() {
                            if c == ';' || Some(c) == end {
                                break;
                            }
                        }
                        continue;
                    }
                    self.skip_ws_comments();
                    let vstart = self.i;
                    let mut depth = 0;
                    while let Some(c) = self.peek() {
                        match c {
                            '(' | '[' => depth += 1,
                            ')' | ']' => depth -= 1,
                            ';' | '}' if depth <= 0 => break,
                            _ => {}
                        }
                        self.i += c.len_utf8();
                    }
                    let mut val_text = self.s[vstart..self.i].trim().to_string();
                    let mut important = false;
                    if let Some(stripped) = strip_important(&val_text) {
                        val_text = stripped;
                        important = true;
                    }
                    if self.peek() == Some(';') {
                        self.bump();
                    }
                    if !name.is_empty() && !val_text.is_empty() {
                        out.push(Decl {
                            name,
                            value: crate::values::parse_value(&val_text),
                            important,
                        });
                    }
                }
            }
        }
        out
    }
}

fn strip_important(v: &str) -> Option<String> {
    let t = v.trim_end();
    let lower = t.to_ascii_lowercase();
    if lower.ends_with("!important") {
        Some(t[..t.len() - "!important".len()].trim_end().to_string())
    } else if lower.ends_with("! important") {
        Some(t[..t.len() - "! important".len()].trim_end().to_string())
    } else {
        None
    }
}

fn extract_url(pre: &str) -> Option<String> {
    let t = pre.trim();
    if let Some(u) = t.strip_prefix("url(") {
        let u = u
            .trim_end_matches(')')
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        return Some(u.to_string());
    }
    let q = t.chars().next()?;
    if q == '"' || q == '\'' {
        let end = t[1..].find(q)?;
        return Some(t[1..1 + end].to_string());
    }
    None
}

fn parse_media(pre: &str) -> Vec<MediaQuery> {
    pre.split(',')
        .map(|q| {
            let mut medium = None;
            let mut features = Vec::new();
            // extract (...) groups as features; remaining words = medium
            let b: Vec<char> = q.chars().collect();
            let mut i = 0;
            let mut words = String::new();
            while i < b.len() {
                if b[i] == '(' {
                    let start = i + 1;
                    let mut depth = 1;
                    i += 1;
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
                    let inner: String = b[start..i].iter().collect();
                    if i < b.len() {
                        i += 1;
                    }
                    if let Some((f, v)) = inner.split_once(':') {
                        features.push((f.trim().to_string(), v.trim().to_string()));
                    } else if !inner.trim().is_empty() {
                        features.push((inner.trim().to_string(), String::new()));
                    }
                } else {
                    words.push(b[i]);
                    i += 1;
                }
            }
            let med: Vec<String> = words
                .split_whitespace()
                .filter(|w| !matches!(*w, "and" | "only"))
                .map(|w| w.to_string())
                .collect();
            if !med.is_empty() {
                medium = Some(med.join(" "));
            }
            MediaQuery { medium, features }
        })
        .collect()
}

/// Conservative @supports evaluation: true iff the feature name is one we
/// recognize (we can't evaluate arbitrary conditions yet — err on include).
fn supports_query(pre: &str) -> bool {
    let pre = pre.trim();
    if let Some(inner) = pre.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        if let Some((prop, _)) = inner.split_once(':') {
            return crate::style::is_known_property(&prop.trim().to_ascii_lowercase());
        }
    }
    true
}
