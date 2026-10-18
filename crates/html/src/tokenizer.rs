//! HTML5 tokenizer — a faithful implementation of the core state machine:
//! Data / Rcdata / Rawtext / ScriptData / Plaintext, tag & attribute
//! states, comments, DOCTYPE, and character references (named + numeric).

use crate::entities;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Doctype {
        name: String,
        public_id: Option<String>,
        system_id: Option<String>,
        force_quirks: bool,
    },
    StartTag {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    EndTag {
        name: String,
    },
    Comment(String),
    /// A run of character data (already entity-decoded).
    Text(String),
    Eof,
}

/// Element context decides whether markup-like characters belong to source
/// text. Keeping that decision with the tree builder prevents script/style
/// contents from accidentally becoming ordinary tags or character references.
#[derive(Clone, Copy, PartialEq)]
pub enum RawKind {
    None,
    Rcdata,  // <title>, <textarea>
    Rawtext, // <style>, <xmp>, <iframe>, <noembed>, <noframes>, <noscript(scripting on)>
    Script,  // <script>
    Plaintext,
}

pub struct Tokenizer {
    input: Vec<char>,
    pos: usize,
    /// tag name that ends a raw-text run (e.g. "script")
    raw_end: String,
    pub raw_kind: RawKind,
}

impl Tokenizer {
    pub fn new(input: &str) -> Tokenizer {
        Tokenizer {
            input: input.chars().collect(),
            pos: 0,
            raw_end: String::new(),
            raw_kind: RawKind::None,
        }
    }

    /// The matching closing tag must remain recognizable while other tag-like
    /// input stays text, or embedded source could prematurely change the tree.
    pub fn enter_raw(&mut self, kind: RawKind, tag: &str) {
        self.raw_kind = kind;
        self.raw_end = tag.to_ascii_lowercase();
    }

    fn peek_n(&self, n: usize) -> Option<char> {
        self.input.get(self.pos + n).copied()
    }
    pub fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }
    pub fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }
    fn eat_str(&self, s: &str) -> bool {
        for (i, c) in s.chars().enumerate() {
            match self.peek_n(i) {
                Some(x) if x.eq_ignore_ascii_case(&c) => {}
                _ => return false,
            }
        }
        true
    }
    pub fn next_token(&mut self) -> Token {
        match self.raw_kind {
            RawKind::Rcdata => self.raw_text(true),
            RawKind::Rawtext | RawKind::Script => self.raw_text(false),
            RawKind::Plaintext => self.plaintext(),
            RawKind::None => self.data(),
        }
    }

    fn data(&mut self) -> Token {
        let mut text = String::new();
        loop {
            match self.peek() {
                None => {
                    return if text.is_empty() {
                        Token::Eof
                    } else {
                        Token::Text(text)
                    };
                }
                Some('<') => {
                    if !text.is_empty() {
                        return Token::Text(text);
                    }
                    if let Some(t) = self.tag_open() {
                        return t;
                    }
                    text.push('<');
                    self.pos += 1;
                }
                Some('&') => {
                    text.push(self.char_ref(true));
                }
                Some(c) => {
                    text.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    fn raw_text(&mut self, entities: bool) -> Token {
        // Emit "</end>" handling: scan until </raw_end followed by
        // whitespace, '/', or '>'.
        let mut text = String::new();
        loop {
            match self.peek() {
                None => {
                    self.raw_kind = RawKind::None;
                    return if text.is_empty() {
                        Token::Eof
                    } else {
                        Token::Text(text)
                    };
                }
                Some('<') if self.peek_n(1) == Some('/') && self.eat_end_tag() => {
                    if text.is_empty() {
                        // consume the end tag and return it
                        self.pos += 2; // "</"
                        let name = self.read_tag_name();
                        self.skip_ws();
                        if self.peek() == Some('>') {
                            self.pos += 1;
                        }
                        self.raw_kind = RawKind::None;
                        return Token::EndTag { name };
                    }
                    return Token::Text(text);
                }
                Some('&') if entities => text.push(self.char_ref(true)),
                Some(c) => {
                    text.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    fn eat_end_tag(&self) -> bool {
        // current pos at '<', pos+1 == '/'
        let name_chars: Vec<char> = self.raw_end.chars().collect();
        for (i, &ec) in name_chars.iter().enumerate() {
            match self.peek_n(2 + i) {
                Some(c) if c.eq_ignore_ascii_case(&ec) => {}
                _ => return false,
            }
        }
        match self.peek_n(2 + name_chars.len()) {
            Some('>') | Some('/') | None => true,
            Some(c) => c.is_ascii_whitespace(),
        }
    }

    fn read_tag_name(&mut self) -> String {
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.') {
                name.push(c.to_ascii_lowercase());
                self.pos += 1;
            } else {
                break;
            }
        }
        name
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    fn plaintext(&mut self) -> Token {
        if self.pos >= self.input.len() {
            return Token::Eof;
        }
        let text: String = self.input[self.pos..].iter().collect();
        self.pos = self.input.len();
        Token::Text(text)
    }

    /// Called at '<' in data state. Returns None if it's just a literal '<'.
    fn tag_open(&mut self) -> Option<Token> {
        debug_assert_eq!(self.peek(), Some('<'));
        match self.peek_n(1) {
            Some('!') => {
                self.pos += 2;
                Some(self.markup_declaration())
            }
            Some('/') => {
                match self.peek_n(2) {
                    Some(c) if c.is_ascii_alphabetic() => {
                        self.pos += 2;
                        let name = self.read_tag_name();
                        self.skip_ws();
                        if self.peek() == Some('>') {
                            self.pos += 1;
                        } else {
                            // bogus end tag: skip to '>'
                            while let Some(c) = self.bump() {
                                if c == '>' {
                                    break;
                                }
                            }
                        }
                        Some(Token::EndTag { name })
                    }
                    Some('>') => {
                        self.pos += 3;
                        Some(Token::Text(String::new()))
                    }
                    _ => None,
                }
            }
            Some('?') => {
                // bogus comment
                self.pos += 2;
                Some(self.bogus_comment())
            }
            Some(c) if c.is_ascii_alphabetic() => {
                self.pos += 1;
                Some(self.start_tag())
            }
            _ => None,
        }
    }

    fn bogus_comment(&mut self) -> Token {
        let mut c = String::new();
        while let Some(ch) = self.bump() {
            if ch == '>' {
                break;
            }
            c.push(ch);
        }
        Token::Comment(c)
    }

    fn markup_declaration(&mut self) -> Token {
        if self.eat_str("--") {
            self.pos += 2;
            return self.comment();
        }
        if self.eat_str("[cdata[") {
            // In HTML content CDATA is a bogus comment; treat contents as text
            self.pos += 7;
            let mut text = String::new();
            loop {
                if self.peek().is_none() {
                    break;
                }
                if self.eat_str("]]>") {
                    self.pos += 3;
                    break;
                }
                text.push(self.bump().unwrap());
            }
            return Token::Text(text);
        }
        if self.eat_str("doctype") {
            self.pos += 7;
            return self.doctype();
        }
        self.bogus_comment()
    }

    fn comment(&mut self) -> Token {
        // already past "<!--"
        let mut c = String::new();
        loop {
            match self.peek() {
                None => return Token::Comment(c),
                Some('-') => {
                    if self.eat_str("-->") {
                        self.pos += 3;
                        return Token::Comment(c);
                    }
                    if self.eat_str("--!>") {
                        self.pos += 4;
                        return Token::Comment(c);
                    }
                    if self.eat_str("--!") {
                        self.pos += 3;
                        continue;
                    }
                    if self.peek_n(1) == Some('-') && self.peek_n(2) != Some('>') {
                        // "--" inside comment: per spec this is a parse error
                        // but browsers keep both dashes
                        c.push('-');
                        self.pos += 1;
                        continue;
                    }
                    c.push('-');
                    self.pos += 1;
                }
                Some(ch) => {
                    c.push(ch);
                    self.pos += 1;
                }
            }
        }
    }

    fn doctype(&mut self) -> Token {
        self.skip_ws();
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() || c == '>' {
                break;
            }
            name.push(c.to_ascii_lowercase());
            self.pos += 1;
        }
        self.skip_ws();
        let mut public_id = None;
        let mut system_id = None;
        if self.eat_str("public") {
            self.pos += 6;
            self.skip_ws();
            public_id = self.quoted_string();
            self.skip_ws();
            system_id = self.quoted_string();
        } else if self.eat_str("system") {
            self.pos += 6;
            self.skip_ws();
            system_id = self.quoted_string();
        }
        while let Some(c) = self.bump() {
            if c == '>' {
                break;
            }
        }
        let name = if name.is_empty() { "html".into() } else { name };
        let force_quirks = name != "html"
            || public_id
                .as_deref()
                .map(|p| p.contains("XHTML") && !p.contains("1.0"))
                .unwrap_or(false)
            || public_id
                .as_deref()
                .map(|p| {
                    let l = p.to_ascii_lowercase();
                    l.starts_with("-//w3c//dtd html 3") || l.starts_with("-//w3c//dtd html 4.0")
                })
                .unwrap_or(false);
        Token::Doctype {
            name,
            public_id,
            system_id,
            force_quirks,
        }
    }

    fn quoted_string(&mut self) -> Option<String> {
        let q = self.peek()?;
        if q != '"' && q != '\'' {
            return None;
        }
        self.pos += 1;
        let mut s = String::new();
        while let Some(c) = self.bump() {
            if c == q {
                break;
            }
            s.push(c);
        }
        Some(s)
    }

    fn start_tag(&mut self) -> Token {
        let name = self.read_tag_name();
        let mut attrs: Vec<(String, String)> = Vec::new();
        let mut self_closing = false;
        loop {
            self.skip_ws();
            match self.peek() {
                None | Some('>') => {
                    self.pos += 1;
                    break;
                }
                Some('/') => {
                    self.pos += 1;
                    if self.peek() == Some('>') {
                        self.pos += 1;
                        self_closing = true;
                        break;
                    }
                    continue;
                }
                Some(_) => {
                    // attribute name
                    let mut aname = String::new();
                    while let Some(c) = self.peek() {
                        if c.is_ascii_whitespace() || matches!(c, '=' | '>' | '/') {
                            break;
                        }
                        aname.push(c.to_ascii_lowercase());
                        self.pos += 1;
                    }
                    self.skip_ws();
                    let value = if self.peek() == Some('=') {
                        self.pos += 1;
                        self.skip_ws();
                        match self.peek() {
                            Some('"') | Some('\'') => {
                                let q = self.bump().unwrap();
                                let mut v = String::new();
                                while let Some(c) = self.peek() {
                                    if c == q {
                                        self.pos += 1;
                                        break;
                                    }
                                    if c == '&' {
                                        v.push(self.char_ref(true));
                                    } else {
                                        v.push(c);
                                        self.pos += 1;
                                    }
                                }
                                v
                            }
                            _ => {
                                let mut v = String::new();
                                while let Some(c) = self.peek() {
                                    if c.is_ascii_whitespace() || c == '>' {
                                        break;
                                    }
                                    if c == '&' {
                                        v.push(self.char_ref(true));
                                    } else {
                                        v.push(c);
                                        self.pos += 1;
                                    }
                                }
                                v
                            }
                        }
                    } else {
                        String::new()
                    };
                    if !aname.is_empty() && !attrs.iter().any(|(k, _)| *k == aname) {
                        attrs.push((aname, value));
                    }
                }
            }
        }
        Token::StartTag {
            name,
            attrs,
            self_closing,
        }
    }

    /// Character reference starting at '&'.
    fn char_ref(&mut self, _in_attr: bool) -> char {
        debug_assert_eq!(self.peek(), Some('&'));
        // numeric?
        match self.peek_n(1) {
            Some('#') => {
                self.pos += 2;
                let hex = matches!(self.peek(), Some('x') | Some('X'));
                if hex {
                    self.pos += 1;
                }
                let mut num = String::new();
                while let Some(c) = self.peek() {
                    if (hex && c.is_ascii_hexdigit()) || (!hex && c.is_ascii_digit()) {
                        num.push(c);
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                if num.is_empty() {
                    return '&';
                }
                if self.peek() == Some(';') {
                    self.pos += 1;
                }
                let v = u32::from_str_radix(&num, if hex { 16 } else { 10 }).unwrap_or(0xfffd);
                return map_codepoint(v);
            }
            Some(c) if c.is_ascii_alphanumeric() => {
                // longest named match (max ~10 chars)
                let mut best: Option<(usize, char)> = None;
                let mut name = String::new();
                for i in 1..=31 {
                    match self.peek_n(i) {
                        Some(c) if c.is_ascii_alphanumeric() => name.push(c),
                        _ => break,
                    }
                    if let Some(ch) = entities::lookup(&name) {
                        // require trailing ';' or (for a few legacy refs) allow missing
                        let next = self.peek_n(i + 1);
                        if next == Some(';') {
                            best = Some((i + 1, ch));
                        } else if LEGACY_NO_SEMI.contains(&name.as_str()) {
                            best = Some((i, ch));
                        }
                    }
                }
                if let Some((len, ch)) = best {
                    self.pos += 1 + len;
                    return ch;
                }
                return '&';
            }
            _ => {}
        }
        '&'
    }
}

const LEGACY_NO_SEMI: &[&str] = &[
    "amp", "lt", "gt", "quot", "nbsp", "copy", "reg", "deg", "para", "middot", "frac12", "frac14",
    "frac34", "plusmn", "times", "divide", "sup1", "sup2", "sup3", "not", "micro", "pound", "yen",
    "sect", "uml", "acute", "cedil", "macr", "laquo", "raquo", "iexcl", "iquest", "curren",
    "brvbar", "ordf", "ordm", "shy", "szlig", "AElig", "ETH", "THORN", "aacute", "eacute",
    "iacute", "oacute", "uacute", "yacute", "agrave", "egrave", "igrave", "ograve", "ugrave",
    "acirc", "ecirc", "icirc", "ocirc", "ucirc", "auml", "euml", "iuml", "ouml", "uuml", "atilde",
    "otilde", "ntilde", "ccedil", "aring", "oslash", "aelig", "eth", "thorn",
];

fn map_codepoint(v: u32) -> char {
    match v {
        0 => '\u{fffd}',
        0x80 => '€',
        0x82 => '‚',
        0x83 => 'ƒ',
        0x84 => '„',
        0x85 => '…',
        0x86 => '†',
        0x87 => '‡',
        0x88 => 'ˆ',
        0x89 => '‰',
        0x8a => 'Š',
        0x8b => '‹',
        0x8c => 'Œ',
        0x8e => 'Ž',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201c}',
        0x94 => '\u{201d}',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x98 => '˜',
        0x99 => '™',
        0x9a => 'š',
        0x9b => '›',
        0x9c => 'œ',
        0x9e => 'ž',
        0x9f => 'Ÿ',
        x if x > 0x10ffff || (0xd800..=0xdfff).contains(&x) => '\u{fffd}',
        x => char::from_u32(x).unwrap_or('\u{fffd}'),
    }
}
