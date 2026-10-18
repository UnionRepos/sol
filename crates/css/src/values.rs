//! CSS value model + color parsing.

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Ident(String),
    Number(f64),
    Percentage(f64),
    Length(f64, Unit),
    Str(String),
    Color(u8, u8, u8, u8),
    Url(String),
    Func(String, Vec<Value>),
    Keyword(String), // e.g. "none", "auto", handled like Ident but kept for clarity
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unit {
    Px,
    Em,
    Rem,
    Ex,
    Ch,
    Vw,
    Vh,
    Vmin,
    Vmax,
    Cm,
    Mm,
    In,
    Pt,
    Pc,
    Q,
    S,
    Ms,
    Deg,
    Rad,
    Grad,
    Turn,
    Hz,
    Khz,
    Dpi,
    Dpcm,
    Dppx,
    Fr,
    Pct,
}

pub fn parse_value(s: &str) -> Vec<Value> {
    let mut vals = Vec::new();
    let mut p = VP { s, i: 0 };
    loop {
        p.ws();
        if p.done() {
            break;
        }
        if let Some(v) = p.one() {
            vals.push(v);
        } else {
            break;
        }
        // '/' separates e.g. font shorthand's size/line-height
        if p.peek() == Some('/') {
            p.i += 1;
            vals.push(Value::Ident("/".into()));
        } else if p.peek() == Some(',') {
            p.i += 1;
            vals.push(Value::Ident(",".into()));
        }
    }
    vals
}

/// First value convenience.
pub fn value1(s: &str) -> Option<Value> {
    parse_value(s).into_iter().next()
}

struct VP<'a> {
    s: &'a str,
    i: usize,
}
impl<'a> VP<'a> {
    fn done(&self) -> bool {
        self.i >= self.s.len()
    }
    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }
    fn ws(&mut self) {
        while let Some(c) = self.peek() {
            if !c.is_whitespace() {
                break;
            }
            self.i += c.len_utf8();
        }
    }
    fn ident(&mut self) -> String {
        let start = self.i;
        if self.peek() == Some('-') {
            self.i += 1;
        }
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || matches!(c, '-' | '_') || c >= '\u{80}' {
                self.i += c.len_utf8();
            } else {
                break;
            }
        }
        self.s[start..self.i].to_string()
    }
    fn one(&mut self) -> Option<Value> {
        let c = self.peek()?;
        match c {
            '#' => {
                self.i += 1;
                let start = self.i;
                while matches!(self.peek(), Some(x) if x.is_ascii_hexdigit()) {
                    self.i += 1;
                }
                Some(parse_hex(&self.s[start..self.i]).map_or(
                    Value::Ident(format!("#{}", &self.s[start..self.i])),
                    |(r, g, b, a)| Value::Color(r, g, b, a),
                ))
            }
            '"' | '\'' => {
                self.i += 1;
                let mut s = String::new();
                while let Some(x) = self.s[self.i..].chars().next() {
                    if x == c {
                        self.i += 1;
                        break;
                    }
                    if x == '\\' {
                        self.i += 1;
                    }
                    if let Some(y) = self.s[self.i..].chars().next() {
                        s.push(y);
                        self.i += y.len_utf8();
                    } else {
                        break;
                    }
                }
                Some(Value::Str(s))
            }
            c if c.is_ascii_digit()
                || c == '.'
                || (c == '-'
                    && self.s[self.i..]
                        .chars()
                        .nth(1)
                        .map_or(false, |n| n.is_ascii_digit() || n == '.'))
                || (c == '+'
                    && self.s[self.i..]
                        .chars()
                        .nth(1)
                        .map_or(false, |n| n.is_ascii_digit() || n == '.')) =>
            {
                // number [unit|%]
                let start = self.i;
                if matches!(self.peek(), Some('+') | Some('-')) {
                    self.i += 1;
                }
                while matches!(self.peek(), Some(x) if x.is_ascii_digit() || x == '.') {
                    self.i += 1;
                }
                // optional exponent: e[+-]?digits
                if matches!(self.peek(), Some('e') | Some('E')) {
                    let save = self.i;
                    self.i += 1;
                    if matches!(self.peek(), Some('+') | Some('-')) {
                        self.i += 1;
                    }
                    let ds = self.i;
                    while matches!(self.peek(), Some(x) if x.is_ascii_digit()) {
                        self.i += 1;
                    }
                    if self.i == ds {
                        self.i = save; // no digits after e — not an exponent
                    }
                }
                let numstr = &self.s[start..self.i];
                let num: f64 = numstr.parse().unwrap_or(0.0);
                if self.peek() == Some('%') {
                    self.i += 1;
                    return Some(Value::Percentage(num));
                }
                let unit_str = self.ident().to_ascii_lowercase();
                if unit_str.is_empty() {
                    Some(Value::Number(num))
                } else {
                    let unit = match unit_str.as_str() {
                        "px" => Unit::Px,
                        "em" => Unit::Em,
                        "rem" => Unit::Rem,
                        "ex" => Unit::Ex,
                        "ch" => Unit::Ch,
                        "vw" => Unit::Vw,
                        "vh" => Unit::Vh,
                        "vmin" => Unit::Vmin,
                        "vmax" => Unit::Vmax,
                        "cm" => Unit::Cm,
                        "mm" => Unit::Mm,
                        "in" => Unit::In,
                        "pt" => Unit::Pt,
                        "pc" => Unit::Pc,
                        "q" => Unit::Q,
                        "s" => Unit::S,
                        "ms" => Unit::Ms,
                        "deg" => Unit::Deg,
                        "rad" => Unit::Rad,
                        "grad" => Unit::Grad,
                        "turn" => Unit::Turn,
                        "hz" => Unit::Hz,
                        "khz" => Unit::Khz,
                        "dpi" => Unit::Dpi,
                        "dpcm" => Unit::Dpcm,
                        "dppx" => Unit::Dppx,
                        "fr" => Unit::Fr,
                        _ => return Some(Value::Ident(format!("{numstr}{unit_str}"))),
                    };
                    Some(Value::Length(num, unit))
                }
            }
            c if c.is_alphabetic() || c == '-' || c == '_' || c >= '\u{80}' => {
                let id = self.ident();
                if self.peek() == Some('(') {
                    self.i += 1;
                    // parse function args
                    let inner_start = self.i;
                    let mut depth = 1;
                    while let Some(x) = self.peek() {
                        if x == '(' {
                            depth += 1;
                        } else if x == ')' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        self.i += 1;
                    }
                    let inner = &self.s[inner_start..self.i];
                    if self.peek() == Some(')') {
                        self.i += 1;
                    }
                    let idl = id.to_ascii_lowercase();
                    // colors/functions get structured handling
                    match idl.as_str() {
                        "rgb" | "rgba" => {
                            let parts: Vec<f64> = inner
                                .split(|c| c == ',' || c == '/' || c == ' ')
                                .filter(|s| !s.is_empty())
                                .map(|s| s.trim_end_matches('%').parse().unwrap_or(0.0))
                                .collect();
                            let pct: Vec<bool> = inner
                                .split(|c| c == ',' || c == '/' || c == ' ')
                                .filter(|s| !s.is_empty())
                                .map(|s| s.ends_with('%'))
                                .collect();
                            if parts.len() >= 3 {
                                let f = |i: usize| -> u8 {
                                    let v = parts[i];
                                    let p = pct.get(i).copied().unwrap_or(false);
                                    (if p { v * 2.55 } else { v }).round().clamp(0.0, 255.0) as u8
                                };
                                let a = if parts.len() >= 4 {
                                    (parts[3].clamp(0.0, 1.0) * 255.0) as u8
                                } else {
                                    255
                                };
                                return Some(Value::Color(f(0), f(1), f(2), a));
                            }
                        }
                        "hsl" | "hsla" => {
                            let parts: Vec<f64> = inner
                                .split(|c| c == ',' || c == '/' || c == ' ')
                                .filter(|s| !s.is_empty())
                                .map(|s| s.trim_end_matches('%').parse().unwrap_or(0.0))
                                .collect();
                            if parts.len() >= 3 {
                                let (r, g, b) = hsl(parts[0], parts[1] / 100.0, parts[2] / 100.0);
                                let a = if parts.len() >= 4 {
                                    (parts[3].clamp(0.0, 1.0) * 255.0) as u8
                                } else {
                                    255
                                };
                                return Some(Value::Color(r, g, b, a));
                            }
                        }
                        "url" => {
                            let u = inner.trim().trim_matches('"').trim_matches('\'');
                            return Some(Value::Url(u.to_string()));
                        }
                        _ => {
                            return Some(Value::Func(idl, parse_value(inner)));
                        }
                    }
                    Some(Value::Func(idl, parse_value(inner)))
                } else {
                    let idl = id.to_ascii_lowercase();
                    if let Some(c) = named_color(&idl) {
                        Some(Value::Color(c.0, c.1, c.2, c.3))
                    } else {
                        Some(Value::Ident(idl))
                    }
                }
            }
            _ => {
                self.i += c.len_utf8();
                Some(Value::Ident(c.to_string()))
            }
        }
    }
}

fn parse_hex(s: &str) -> Option<(u8, u8, u8, u8)> {
    match s.len() {
        3 => {
            let v = u32::from_str_radix(s, 16).ok()?;
            Some((
                ((v >> 8) & 15) as u8 * 17,
                ((v >> 4) & 15) as u8 * 17,
                (v & 15) as u8 * 17,
                255,
            ))
        }
        4 => {
            let v = u32::from_str_radix(s, 16).ok()?;
            Some((
                ((v >> 12) & 15) as u8 * 17,
                ((v >> 8) & 15) as u8 * 17,
                ((v >> 4) & 15) as u8 * 17,
                (v & 15) as u8 * 17,
            ))
        }
        6 => {
            let v = u32::from_str_radix(s, 16).ok()?;
            Some(((v >> 16) as u8, (v >> 8) as u8, v as u8, 255))
        }
        8 => {
            let v = u32::from_str_radix(s, 16).ok()?;
            Some(((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8))
        }
        _ => None,
    }
}

fn hsl(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let h = (h % 360.0 + 360.0) % 360.0 / 60.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    )
}

pub fn named_color(name: &str) -> Option<(u8, u8, u8, u8)> {
    Some(match name {
        "black" => (0, 0, 0, 255),
        "silver" => (192, 192, 192, 255),
        "gray" | "grey" => (128, 128, 128, 255),
        "white" => (255, 255, 255, 255),
        "maroon" => (128, 0, 0, 255),
        "red" => (255, 0, 0, 255),
        "purple" => (128, 0, 128, 255),
        "fuchsia" | "magenta" => (255, 0, 255, 255),
        "green" => (0, 128, 0, 255),
        "lime" => (0, 255, 0, 255),
        "olive" => (128, 128, 0, 255),
        "yellow" => (255, 255, 0, 255),
        "navy" => (0, 0, 128, 255),
        "blue" => (0, 0, 255, 255),
        "teal" => (0, 128, 128, 255),
        "aqua" | "cyan" => (0, 255, 255, 255),
        "orange" => (255, 165, 0, 255),
        "aliceblue" => (240, 248, 255, 255),
        "antiquewhite" => (250, 235, 215, 255),
        "aquamarine" => (127, 255, 212, 255),
        "azure" => (240, 255, 255, 255),
        "beige" => (245, 245, 220, 255),
        "bisque" => (255, 228, 196, 255),
        "blanchedalmond" => (255, 235, 205, 255),
        "blueviolet" => (138, 43, 226, 255),
        "brown" => (165, 42, 42, 255),
        "burlywood" => (222, 184, 135, 255),
        "cadetblue" => (95, 158, 160, 255),
        "chartreuse" => (127, 255, 0, 255),
        "chocolate" => (210, 105, 30, 255),
        "coral" => (255, 127, 80, 255),
        "cornflowerblue" => (100, 149, 237, 255),
        "cornsilk" => (255, 248, 220, 255),
        "crimson" => (220, 20, 60, 255),
        "darkblue" => (0, 0, 139, 255),
        "darkcyan" => (0, 139, 139, 255),
        "darkgoldenrod" => (184, 134, 11, 255),
        "darkgray" | "darkgrey" => (169, 169, 169, 255),
        "darkgreen" => (0, 100, 0, 255),
        "darkkhaki" => (189, 183, 107, 255),
        "darkmagenta" => (139, 0, 139, 255),
        "darkolivegreen" => (85, 107, 47, 255),
        "darkorange" => (255, 140, 0, 255),
        "darkorchid" => (153, 50, 204, 255),
        "darkred" => (139, 0, 0, 255),
        "darksalmon" => (233, 150, 122, 255),
        "darkseagreen" => (143, 188, 143, 255),
        "darkslateblue" => (72, 61, 139, 255),
        "darkslategray" | "darkslategrey" => (47, 79, 79, 255),
        "darkturquoise" => (0, 206, 209, 255),
        "darkviolet" => (148, 0, 211, 255),
        "deeppink" => (255, 20, 147, 255),
        "deepskyblue" => (0, 191, 255, 255),
        "dimgray" | "dimgrey" => (105, 105, 105, 255),
        "dodgerblue" => (30, 144, 255, 255),
        "firebrick" => (178, 34, 34, 255),
        "floralwhite" => (255, 250, 240, 255),
        "forestgreen" => (34, 139, 34, 255),
        "gainsboro" => (220, 220, 220, 255),
        "ghostwhite" => (248, 248, 255, 255),
        "gold" => (255, 215, 0, 255),
        "goldenrod" => (218, 165, 32, 255),
        "greenyellow" => (173, 255, 47, 255),
        "honeydew" => (240, 255, 240, 255),
        "hotpink" => (255, 105, 180, 255),
        "indianred" => (205, 92, 92, 255),
        "indigo" => (75, 0, 130, 255),
        "ivory" => (255, 255, 240, 255),
        "khaki" => (240, 230, 140, 255),
        "lavender" => (230, 230, 250, 255),
        "lavenderblush" => (255, 240, 245, 255),
        "lawngreen" => (124, 252, 0, 255),
        "lemonchiffon" => (255, 250, 205, 255),
        "lightblue" => (173, 216, 230, 255),
        "lightcoral" => (240, 128, 128, 255),
        "lightcyan" => (224, 255, 255, 255),
        "lightgoldenrodyellow" => (250, 250, 210, 255),
        "lightgray" | "lightgrey" => (211, 211, 211, 255),
        "lightgreen" => (144, 238, 144, 255),
        "lightpink" => (255, 182, 193, 255),
        "lightsalmon" => (255, 160, 122, 255),
        "lightseagreen" => (32, 178, 170, 255),
        "lightskyblue" => (135, 206, 250, 255),
        "lightslategray" | "lightslategrey" => (119, 136, 153, 255),
        "lightsteelblue" => (176, 196, 222, 255),
        "lightyellow" => (255, 255, 224, 255),
        "limegreen" => (50, 205, 50, 255),
        "linen" => (250, 240, 230, 255),
        "mediumaquamarine" => (102, 205, 170, 255),
        "mediumblue" => (0, 0, 205, 255),
        "mediumorchid" => (186, 85, 211, 255),
        "mediumpurple" => (147, 112, 219, 255),
        "mediumseagreen" => (60, 179, 113, 255),
        "mediumslateblue" => (123, 104, 238, 255),
        "mediumspringgreen" => (0, 250, 154, 255),
        "mediumturquoise" => (72, 209, 204, 255),
        "mediumvioletred" => (199, 21, 133, 255),
        "midnightblue" => (25, 25, 112, 255),
        "mintcream" => (245, 255, 250, 255),
        "mistyrose" => (255, 228, 225, 255),
        "moccasin" => (255, 228, 181, 255),
        "navajowhite" => (255, 222, 173, 255),
        "oldlace" => (253, 245, 230, 255),
        "olivedrab" => (107, 142, 35, 255),
        "orangered" => (255, 69, 0, 255),
        "orchid" => (218, 112, 214, 255),
        "palegoldenrod" => (238, 232, 170, 255),
        "palegreen" => (152, 251, 152, 255),
        "paleturquoise" => (175, 238, 238, 255),
        "palevioletred" => (219, 112, 147, 255),
        "papayawhip" => (255, 239, 213, 255),
        "peachpuff" => (255, 218, 185, 255),
        "peru" => (205, 133, 63, 255),
        "pink" => (255, 192, 203, 255),
        "plum" => (221, 160, 221, 255),
        "powderblue" => (176, 224, 230, 255),
        "rebeccapurple" => (102, 51, 153, 255),
        "rosybrown" => (188, 143, 143, 255),
        "royalblue" => (65, 105, 225, 255),
        "saddlebrown" => (139, 69, 19, 255),
        "salmon" => (250, 128, 114, 255),
        "sandybrown" => (244, 164, 96, 255),
        "seagreen" => (46, 139, 87, 255),
        "seashell" => (255, 245, 238, 255),
        "sienna" => (160, 82, 45, 255),
        "skyblue" => (135, 206, 235, 255),
        "slateblue" => (106, 90, 205, 255),
        "slategray" | "slategrey" => (112, 128, 144, 255),
        "snow" => (255, 250, 250, 255),
        "springgreen" => (0, 255, 127, 255),
        "steelblue" => (70, 130, 180, 255),
        "tan" => (210, 180, 140, 255),
        "thistle" => (216, 191, 216, 255),
        "tomato" => (255, 99, 71, 255),
        "turquoise" => (64, 224, 208, 255),
        "violet" => (238, 130, 238, 255),
        "wheat" => (245, 222, 179, 255),
        "whitesmoke" => (245, 245, 245, 255),
        "yellowgreen" => (154, 205, 50, 255),
        "transparent" => (0, 0, 0, 0),
        "currentcolor" => return None,
        _ => return None,
    })
}
