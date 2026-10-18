//! Minimal backtracking regex engine (subset of JS RegExp).
//! Supports: literals, ., *, +, ?, {n,m}, [...], [^...], ranges,
//! \d \D \w \W \s \S \b escapes, groups (), alternation |,
//! anchors ^ $, i/g flags.

#[derive(Debug, Clone)]
enum N {
    Char(char),
    Any,
    Class {
        neg: bool,
        items: Vec<CItem>,
    },
    Group(usize, Box<N>), // capture index
    Seq(Vec<N>),
    Alt(Vec<N>),
    Rep {
        n: Box<N>,
        min: u32,
        max: u32,
        greedy: bool,
    },
    Anchor(Anchor),
    Backref(usize),
    WordB(bool), // \b or \B
}

#[derive(Debug, Clone)]
enum CItem {
    Ch(char),
    Range(char, char),
    Digit(bool),
    Word(bool),
    Space(bool),
}

#[derive(Debug, Clone, Copy)]
enum Anchor {
    Start,
    End,
}

#[derive(Debug)]
pub struct Re {
    root: N,
    pub icase: bool,
    pub global: bool,
    pub groups: usize,
}

pub fn compile(body: &str, flags: &str) -> Result<Re, String> {
    let mut p = ReP {
        s: body.chars().collect(),
        i: 0,
        groups: 0,
    };
    let root = p.alt()?;
    if p.i < p.s.len() {
        return Err(format!("regex: trailing at {}", p.i));
    }
    Ok(Re {
        root: N::Group(0, Box::new(root)),
        icase: flags.contains('i'),
        global: flags.contains('g'),
        groups: p.groups,
    })
}

struct ReP {
    s: Vec<char>,
    i: usize,
    groups: usize,
}

impl ReP {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.i += 1;
        }
        c
    }

    fn alt(&mut self) -> Result<N, String> {
        let mut alts = vec![self.seq()?];
        while self.peek() == Some('|') {
            self.i += 1;
            alts.push(self.seq()?);
        }
        Ok(if alts.len() == 1 {
            alts.pop().unwrap()
        } else {
            N::Alt(alts)
        })
    }

    fn seq(&mut self) -> Result<N, String> {
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None | Some('|') | Some(')') => break,
                _ => items.push(self.rep()?),
            }
        }
        Ok(if items.len() == 1 {
            items.pop().unwrap()
        } else {
            N::Seq(items)
        })
    }

    fn rep(&mut self) -> Result<N, String> {
        let mut atom = self.atom()?;
        loop {
            let (min, max) = match self.peek() {
                Some('*') => {
                    self.i += 1;
                    (0, u32::MAX)
                }
                Some('+') => {
                    self.i += 1;
                    (1, u32::MAX)
                }
                Some('?') => {
                    self.i += 1;
                    (0, 1)
                }
                Some('{') => {
                    let save = self.i;
                    self.i += 1;
                    let mut a = String::new();
                    while let Some(c) = self.peek() {
                        if c.is_ascii_digit() {
                            a.push(c);
                            self.i += 1;
                        } else {
                            break;
                        }
                    }
                    let (mn, mx) = if self.peek() == Some(',') {
                        self.i += 1;
                        let mut b = String::new();
                        while let Some(c) = self.peek() {
                            if c.is_ascii_digit() {
                                b.push(c);
                                self.i += 1;
                            } else {
                                break;
                            }
                        }
                        let mn: u32 = a.parse().unwrap_or(0);
                        let mx = if b.is_empty() {
                            u32::MAX
                        } else {
                            b.parse().unwrap_or(0)
                        };
                        (mn, mx)
                    } else {
                        let mn: u32 = a.parse().map_err(|_| "bad {} count".to_string())?;
                        (mn, mn)
                    };
                    if self.peek() != Some('}') {
                        self.i = save;
                        break;
                    }
                    self.i += 1;
                    (mn, mx)
                }
                _ => break,
            };
            let greedy = if self.peek() == Some('?') {
                self.i += 1;
                false
            } else {
                true
            };
            atom = N::Rep {
                n: Box::new(atom),
                min,
                max,
                greedy,
            };
        }
        Ok(atom)
    }

    fn atom(&mut self) -> Result<N, String> {
        match self.bump() {
            None => Err("regex: eof".into()),
            Some('(') => {
                if self.peek() == Some('?') {
                    self.i += 1;
                    match self.bump() {
                        Some(':') => {
                            // non-capturing
                            let n = self.alt()?;
                            self.expect(')')?;
                            return Ok(n);
                        }
                        Some('=') | Some('!') => {
                            // lookahead — approximate as empty match
                            let mut depth = 1;
                            while depth > 0 {
                                match self.bump() {
                                    None => break,
                                    Some('(') => depth += 1,
                                    Some(')') => depth -= 1,
                                    _ => {}
                                }
                            }
                            return Ok(N::Seq(vec![]));
                        }
                        _ => {}
                    }
                }
                self.groups += 1;
                let gi = self.groups;
                let n = self.alt()?;
                self.expect(')')?;
                Ok(N::Group(gi, Box::new(n)))
            }
            Some('[') => self.class(),
            Some('.') => Ok(N::Any),
            Some('^') => Ok(N::Anchor(Anchor::Start)),
            Some('$') => Ok(N::Anchor(Anchor::End)),
            Some('\\') => match self.bump() {
                Some('d') => Ok(N::Class {
                    neg: false,
                    items: vec![CItem::Digit(false)],
                }),
                Some('D') => Ok(N::Class {
                    neg: true,
                    items: vec![CItem::Digit(false)],
                }),
                Some('w') => Ok(N::Class {
                    neg: false,
                    items: vec![CItem::Word(false)],
                }),
                Some('W') => Ok(N::Class {
                    neg: true,
                    items: vec![CItem::Word(false)],
                }),
                Some('s') => Ok(N::Class {
                    neg: false,
                    items: vec![CItem::Space(false)],
                }),
                Some('S') => Ok(N::Class {
                    neg: true,
                    items: vec![CItem::Space(false)],
                }),
                Some('b') => Ok(N::WordB(true)),
                Some('B') => Ok(N::WordB(false)),
                Some('n') => Ok(N::Char('\n')),
                Some('t') => Ok(N::Char('\t')),
                Some('r') => Ok(N::Char('\r')),
                Some('0') => Ok(N::Char('\0')),
                Some('x') => {
                    let h: String = (0..2).filter_map(|_| self.bump()).collect();
                    let v = u32::from_str_radix(&h, 16).unwrap_or(0);
                    Ok(N::Char(char::from_u32(v).unwrap_or('\0')))
                }
                Some('u') => {
                    if self.peek() == Some('{') {
                        self.i += 1;
                        let mut h = String::new();
                        while self.peek() != Some('}') && self.peek().is_some() {
                            h.push(self.bump().unwrap());
                        }
                        self.i += 1;
                        Ok(N::Char(
                            char::from_u32(u32::from_str_radix(&h, 16).unwrap_or(0))
                                .unwrap_or('\u{fffd}'),
                        ))
                    } else {
                        let h: String = (0..4).filter_map(|_| self.bump()).collect();
                        Ok(N::Char(
                            char::from_u32(u32::from_str_radix(&h, 16).unwrap_or(0))
                                .unwrap_or('\u{fffd}'),
                        ))
                    }
                }
                Some(c) if c.is_ascii_digit() => Ok(N::Backref(c as usize - '0' as usize)),
                Some(c) => Ok(N::Char(c)),
                None => Err("regex: bad escape".into()),
            },
            Some(c) => Ok(N::Char(c)),
        }
    }

    fn class(&mut self) -> Result<N, String> {
        let neg = self.peek() == Some('^');
        if neg {
            self.i += 1;
        }
        let mut items = Vec::new();
        let mut first = true;
        loop {
            match self.peek() {
                None => return Err("unterminated class".into()),
                Some(']') if !first => {
                    self.i += 1;
                    break;
                }
                _ => {}
            }
            first = false;
            let lo = self.class_char()?;
            if self.peek() == Some('-') && self.s.get(self.i + 1) != Some(&']') {
                self.i += 1;
                let hi = self.class_char()?;
                match (&lo, &hi) {
                    (CItem::Ch(a), CItem::Ch(b)) => items.push(CItem::Range(*a, *b)),
                    _ => {
                        items.push(lo);
                        items.push(hi);
                    }
                }
            } else {
                items.push(lo);
            }
        }
        Ok(N::Class { neg, items })
    }

    fn class_char(&mut self) -> Result<CItem, String> {
        match self.bump() {
            Some('\\') => match self.bump() {
                Some('d') => Ok(CItem::Digit(false)),
                Some('D') => Ok(CItem::Digit(true)),
                Some('w') => Ok(CItem::Word(false)),
                Some('W') => Ok(CItem::Word(true)),
                Some('s') => Ok(CItem::Space(false)),
                Some('S') => Ok(CItem::Space(true)),
                Some('n') => Ok(CItem::Ch('\n')),
                Some('t') => Ok(CItem::Ch('\t')),
                Some('r') => Ok(CItem::Ch('\r')),
                Some('x') => {
                    let h: String = (0..2).filter_map(|_| self.bump()).collect();
                    Ok(CItem::Ch(
                        char::from_u32(u32::from_str_radix(&h, 16).unwrap_or(0)).unwrap_or('\0'),
                    ))
                }
                Some(c) => Ok(CItem::Ch(c)),
                None => Err("bad class escape".into()),
            },
            Some(c) => Ok(CItem::Ch(c)),
            None => Err("bad class".into()),
        }
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        if self.bump() == Some(c) {
            Ok(())
        } else {
            Err(format!("regex: expected {c}"))
        }
    }
}

struct M<'a> {
    s: Vec<char>,
    caps: Vec<Option<(usize, usize)>>,
    icase: bool,
    _p: std::marker::PhantomData<&'a ()>,
}

type Caps = Vec<Option<(usize, usize)>>;

impl Re {
    /// Find first match at or after `start`. Returns (start,end,caps).
    pub fn find(&self, text: &str, start: usize) -> Option<(usize, usize, Caps)> {
        let chars: Vec<char> = text.chars().collect();
        let mut m = M {
            s: chars,
            caps: vec![None; self.groups + 1],
            icase: self.icase,
            _p: std::marker::PhantomData,
        };
        for pos in start..=m.s.len() {
            m.caps = vec![None; self.groups + 1];
            if let Some(end) = m.mat(&self.root, pos) {
                m.caps[0] = Some((pos, end));
                return Some((pos, end, m.caps));
            }
        }
        None
    }

    pub fn test(&self, text: &str) -> bool {
        self.find(text, 0).is_some()
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl<'a> M<'a> {
    fn eq(&self, a: char, b: char) -> bool {
        if self.icase {
            a.to_lowercase().eq(b.to_lowercase())
        } else {
            a == b
        }
    }

    /// Match node at pos; on success returns end position.
    fn mat(&mut self, n: &N, pos: usize) -> Option<usize> {
        match n {
            N::Char(c) => {
                if pos < self.s.len() && self.eq(self.s[pos], *c) {
                    Some(pos + 1)
                } else {
                    None
                }
            }
            N::Any => {
                if pos < self.s.len() && self.s[pos] != '\n' {
                    Some(pos + 1)
                } else {
                    None
                }
            }
            N::Class { neg, items } => {
                if pos >= self.s.len() {
                    return None;
                }
                let c = self.s[pos];
                let mut hit = false;
                for it in items {
                    let h = match it {
                        CItem::Ch(x) => self.eq(c, *x),
                        CItem::Range(a, b) => {
                            if self.icase {
                                let cl = c.to_lowercase().next().unwrap_or(c);
                                (cl >= *a && cl <= *b) || (c >= *a && c <= *b)
                            } else {
                                c >= *a && c <= *b
                            }
                        }
                        CItem::Digit(n2) => c.is_ascii_digit() != *n2,
                        CItem::Word(n2) => is_word(c) != *n2,
                        CItem::Space(n2) => c.is_whitespace() != *n2,
                    };
                    if h {
                        hit = true;
                        break;
                    }
                }
                if hit != *neg {
                    Some(pos + 1)
                } else {
                    None
                }
            }
            N::Group(gi, inner) => {
                let saved = self.caps[*gi];
                if let Some(end) = self.mat(inner, pos) {
                    self.caps[*gi] = Some((pos, end));
                    Some(end)
                } else {
                    self.caps[*gi] = saved;
                    None
                }
            }
            N::Seq(items) => {
                let mut p = pos;
                for it in items {
                    p = self.mat(it, p)?;
                }
                Some(p)
            }
            N::Alt(alts) => {
                for a in alts {
                    if let Some(e) = self.mat(a, pos) {
                        return Some(e);
                    }
                }
                None
            }
            N::Rep {
                n,
                min,
                max,
                greedy,
            } => self.rep(n, *min, *max, *greedy, pos, 0),
            N::Anchor(Anchor::Start) => {
                if pos == 0 {
                    Some(pos)
                } else {
                    None
                }
            }
            N::Anchor(Anchor::End) => {
                if pos == self.s.len() {
                    Some(pos)
                } else {
                    None
                }
            }
            N::Backref(gi) => {
                if let Some(Some((a, b))) = self.caps.get(*gi) {
                    let (a, b) = (*a, *b);
                    let len = b - a;
                    if pos + len <= self.s.len()
                        && (0..len).all(|k| self.eq(self.s[pos + k], self.s[a + k]))
                    {
                        Some(pos + len)
                    } else {
                        None
                    }
                } else {
                    Some(pos)
                }
            }
            N::WordB(want) => {
                let before = pos > 0 && is_word(self.s[pos - 1]);
                let after = pos < self.s.len() && is_word(self.s[pos]);
                if (before != after) == *want {
                    Some(pos)
                } else {
                    None
                }
            }
        }
    }

    fn rep(
        &mut self,
        n: &N,
        min: u32,
        max: u32,
        greedy: bool,
        pos: usize,
        count: u32,
    ) -> Option<usize> {
        // try one more iteration first if greedy
        if greedy && count < max {
            if let Some(e) = self.mat(n, pos) {
                if e != pos || count < min {
                    if let Some(e2) = self.rep(n, min, max, greedy, e, count + 1) {
                        return Some(e2);
                    }
                }
            }
        }
        if count >= min {
            return Some(pos);
        }
        // need at least min matches
        if count < max {
            if let Some(e) = self.mat(n, pos) {
                if e == pos && count < min {
                    return self.rep(n, min, max, greedy, e + 1, count + 1);
                }
                return self.rep(n, min, max, greedy, e, count + 1);
            }
        }
        None
    }
}
