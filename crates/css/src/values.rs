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
    Components(Vec<crate::syntax::ComponentValue>),
    Math(String, Vec<crate::syntax::ComponentValue>),
    Dimension(f64, String),
    Block(char, Vec<Value>),
    Token(crate::syntax::TokenKind),
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
    from_components(&crate::syntax::parse_component_values(s).values)
}

pub fn value1(s: &str) -> Option<Value> {
    parse_value(s).into_iter().next()
}

pub(crate) fn from_components(values: &[crate::syntax::ComponentValue]) -> Vec<Value> {
    use crate::syntax::{ComponentValue as C, TokenKind as T};
    values
        .iter()
        .filter_map(|value| match value {
            C::Token(token) => Some(match &token.kind {
                T::Whitespace => return None,
                T::Ident(name) => {
                    if let Some((r, g, b, a)) = named_color(&name.to_ascii_lowercase()) {
                        Value::Color(r, g, b, a)
                    } else {
                        Value::Ident(if name.starts_with("--") {
                            name.clone()
                        } else {
                            name.to_ascii_lowercase()
                        })
                    }
                }
                T::String(s) => Value::Str(s.clone()),
                T::Url(url) => Value::Url(url.clone()),
                T::Hash { value, .. } => match parse_hex(value) {
                    Some((r, g, b, a)) => Value::Color(r, g, b, a),
                    None => Value::Token(token.kind.clone()),
                },
                T::Number(number) => Value::Number(number.value),
                T::Percentage(number) => Value::Percentage(number.value),
                T::Dimension(number, unit) => match parse_unit(unit) {
                    Some(unit) => Value::Length(number.value, unit),
                    None => Value::Dimension(number.value, unit.clone()),
                },
                T::Comma => Value::Ident(",".into()),
                T::Delim(c) if matches!(c, '/' | '+' | '-' | '*') => Value::Ident(c.to_string()),
                other => Value::Token(other.clone()),
            }),
            C::Function { name, value } => {
                let name = name.to_ascii_lowercase();
                if matches!(name.as_str(), "calc" | "min" | "max" | "clamp") {
                    return Some(Value::Math(name, value.clone()));
                }
                let args = if matches!(name.as_str(), "counter" | "counters") {
                    value
                        .iter()
                        .filter(|value| !value.is_whitespace())
                        .flat_map(|value| {
                            if let Some(T::Ident(name)) = value.kind() {
                                vec![Value::Ident(name.clone())]
                            } else {
                                from_components(std::slice::from_ref(value))
                            }
                        })
                        .collect()
                } else {
                    from_components(value)
                };
                if name == "url" {
                    if let [Value::Str(url)] = args.as_slice() {
                        return Some(Value::Url(url.clone()));
                    }
                }
                Some(
                    parse_color_function(&name, &args)
                        .or_else(|| crate::colors::parse(&name, &args))
                        .unwrap_or(Value::Func(name, args)),
                )
            }
            C::Block { opening, value } => Some(Value::Block(*opening, from_components(value))),
        })
        .collect()
}

fn parse_unit(unit: &str) -> Option<Unit> {
    Some(match unit.to_ascii_lowercase().as_str() {
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
        "dppx" | "x" => Unit::Dppx,
        "fr" => Unit::Fr,
        _ => return None,
    })
}

fn parse_color_function(name: &str, args: &[Value]) -> Option<Value> {
    if !matches!(name, "rgb" | "rgba" | "hsl" | "hsla" | "hwb") {
        return None;
    }
    let comma = args.iter().any(|v| matches!(v, Value::Ident(s) if s==","));
    let mut channels = Vec::new();
    let mut slash = None;
    for v in args {
        match v {
            Value::Ident(s) if s == "," => {
                if !comma {
                    return None;
                }
            }
            Value::Ident(s) if s == "/" => {
                if comma || slash.is_some() {
                    return None;
                }
                slash = Some(channels.len());
            }
            v => channels.push(v),
        }
    }
    if channels.len() != 3 && channels.len() != 4 {
        return None;
    }
    if slash.is_some_and(|n| n != 3)
        || (!comma && channels.len() == 4 && slash != Some(3))
        || (slash.is_some() && channels.len() != 4)
    {
        return None;
    }
    if comma {
        if !matches!(args.len(), 5 | 7)
            || !args.iter().enumerate().all(|(i, v)| {
                if i % 2 == 1 {
                    matches!(v,Value::Ident(s) if s==",")
                } else {
                    !matches!(v, Value::Ident(_))
                }
            })
        {
            return None;
        }
        if name == "hwb" {
            return None;
        }
    }
    let alpha = match channels.get(3) {
        None => 1.0,
        Some(Value::Number(n)) => *n,
        Some(Value::Percentage(n)) => n / 100.0,
        Some(Value::Ident(s)) if s == "none" && !comma => 0.0,
        _ => return None,
    };
    let alpha = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    let missing = |v: &Value| matches!(v,Value::Ident(s) if s=="none") && !comma;
    let (r, g, b) = match name {
        "rgb" | "rgba" => {
            if comma
                && !channels[..3].iter().all(|v| matches!(v, Value::Number(_)))
                && !channels[..3]
                    .iter()
                    .all(|v| matches!(v, Value::Percentage(_)))
            {
                return None;
            }
            let channel = |v: &Value| match v {
                Value::Number(n) => Some(n.clamp(0.0, 255.0).round() as u8),
                Value::Percentage(n) => Some((n / 100.0 * 255.0).clamp(0.0, 255.0).round() as u8),
                v if missing(v) => Some(0),
                _ => None,
            };
            (
                channel(channels[0])?,
                channel(channels[1])?,
                channel(channels[2])?,
            )
        }
        _ => {
            let hue = match channels[0] {
                Value::Number(n) => *n,
                Value::Length(n, Unit::Deg) => *n,
                Value::Length(n, Unit::Rad) => n.to_degrees(),
                Value::Length(n, Unit::Grad) => n * 0.9,
                Value::Length(n, Unit::Turn) => n * 360.0,
                v if missing(v) => 0.0,
                _ => return None,
            };
            let percent = |v: &Value| match v {
                Value::Percentage(n) => Some((n / 100.0).clamp(0.0, 1.0)),
                v if missing(v) => Some(0.0),
                _ => None,
            };
            let a = percent(channels[1])?;
            let b = percent(channels[2])?;
            if name == "hwb" {
                if a + b >= 1.0 {
                    let gray = (a / (a + b) * 255.0).round() as u8;
                    (gray, gray, gray)
                } else {
                    let (r, g, bl) = hsl(hue, 1.0, 0.5);
                    let f = |c: u8| ((c as f64 / 255.0 * (1.0 - a - b) + a) * 255.0).round() as u8;
                    (f(r), f(g), f(bl))
                }
            } else {
                hsl(hue, a, b)
            }
        }
    };
    Some(Value::Color(r, g, b, alpha))
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
