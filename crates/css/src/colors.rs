//! Conversions finish in the canvas's sRGB space so wide-gamut syntax does not
//! force layout, image codecs, and compositing to disagree about stored pixels.
use crate::{values::Unit, Value};
fn scalar(v: &Value, scale: f64) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Percentage(n) => Some(n * scale / 100.0),
        Value::Ident(s) if s == "none" => Some(0.0),
        _ => None,
    }
}
fn hue(v: &Value) -> Option<f64> {
    Some(
        match v {
            Value::Number(n) => *n,
            Value::Length(n, unit) => match unit {
                Unit::Deg => *n,
                Unit::Rad => n.to_degrees(),
                Unit::Grad => n * 0.9,
                Unit::Turn => n * 360.0,
                _ => return None,
            },
            Value::Ident(s) if s == "none" => 0.0,
            _ => return None,
        }
        .to_radians(),
    )
}
fn matrix(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    m.map(|row| row.iter().zip(v).map(|(a, b)| a * b).sum())
}
fn linear(v: f64) -> f64 {
    if v.abs() <= 0.04045 {
        v / 12.92
    } else {
        v.signum() * ((v.abs() + 0.055) / 1.055).powf(2.4)
    }
}
fn encoded(v: f64) -> f64 {
    if v.abs() <= 0.0031308 {
        12.92 * v
    } else {
        v.signum() * (1.055 * v.abs().powf(1.0 / 2.4) - 0.055)
    }
}
fn rgb(xyz: [f64; 3]) -> [f64; 3] {
    matrix(
        [
            [3.2409699419, -1.5373831776, -0.4986107603],
            [-0.9692436363, 1.8759675015, 0.0415550574],
            [0.0556300797, -0.2039769589, 1.0569715142],
        ],
        xyz,
    )
    .map(encoded)
}
fn d65(xyz: [f64; 3]) -> [f64; 3] {
    matrix(
        [
            [0.9554734215, -0.0230984549, 0.0632592432],
            [-0.0283697093, 1.0099953981, 0.0210414412],
            [0.0123140149, -0.0205076493, 1.3303659262],
        ],
        xyz,
    )
}
fn oklab(v: [f64; 3]) -> [f64; 3] {
    let [lightness, a, b] = v;
    let l = (lightness + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m = (lightness - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s = (lightness - 0.0894841775 * a - 1.2914855480 * b).powi(3);
    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
    ]
    .map(encoded)
}
fn to_oklab(rgb: [f64; 3]) -> [f64; 3] {
    let lms = matrix(
        [
            [0.4122214708, 0.5363325363, 0.0514459929],
            [0.2119034982, 0.6806995451, 0.1073969566],
            [0.0883024619, 0.2817188376, 0.6299787005],
        ],
        rgb.map(linear),
    )
    .map(f64::cbrt);
    matrix(
        [
            [0.2104542553, 0.793617785, -0.0040720468],
            [1.9779984951, -2.428592205, 0.4505937099],
            [0.0259040371, 0.7827717662, -0.808675766],
        ],
        lms,
    )
}
fn rgba(rgb: [f64; 3], alpha: f64) -> Value {
    let [r, g, b] = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    Value::Color(r, g, b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}
pub(crate) fn parse(name: &str, args: &[Value]) -> Option<Value> {
    if name == "color-mix" {
        return mix(args);
    }
    if !matches!(name, "lab" | "lch" | "oklab" | "oklch" | "color") {
        return None;
    }
    let slash = args
        .iter()
        .position(|v| matches!(v,Value::Ident(s) if s=="/"));
    let alpha = if let Some(slash) = slash {
        if args.len() != slash + 2 {
            return None;
        }
        scalar(&args[slash + 1], 1.0)?
    } else {
        1.0
    };
    let values = &args[..slash.unwrap_or(args.len())];
    let result = if name == "color" {
        let [Value::Ident(profile), a, b, c] = values else {
            return None;
        };
        let values = [scalar(a, 1.0)?, scalar(b, 1.0)?, scalar(c, 1.0)?];
        match profile.as_str() {
            "srgb" => values,
            "srgb-linear" => values.map(encoded),
            "display-p3" => rgb(matrix(
                [
                    [0.4865709486, 0.2656676932, 0.1982172852],
                    [0.2289745641, 0.6917385218, 0.0792869141],
                    [0.0, 0.0451133819, 1.0439443689],
                ],
                values.map(linear),
            )),
            "xyz" | "xyz-d65" => rgb(values),
            "xyz-d50" => rgb(d65(values)),
            "a98-rgb" => rgb(matrix(
                [
                    [0.5766690429, 0.1855582379, 0.1882286462],
                    [0.2973449753, 0.6273635663, 0.0752914585],
                    [0.0270313614, 0.0706888525, 0.9913375368],
                ],
                values.map(|v| v.signum() * v.abs().powf(563.0 / 256.0)),
            )),
            "rec2020" => rgb(matrix(
                [
                    [0.6369580483, 0.1446169036, 0.1688809752],
                    [0.262700212, 0.6779980715, 0.0593017165],
                    [0.0, 0.028072693, 1.0609850577],
                ],
                values.map(|value| value.signum() * value.abs().powf(2.4)),
            )),
            _ => return None,
        }
    } else {
        let [l, a, b] = values else { return None };
        let polar = name.ends_with("lch");
        let ok = name.starts_with("ok");
        let l = scalar(l, if ok { 1.0 } else { 100.0 })?.clamp(0.0, if ok { 1.0 } else { 100.0 });
        let a = scalar(
            a,
            if ok {
                0.4
            } else if polar {
                150.0
            } else {
                125.0
            },
        )?;
        let b = if polar {
            hue(b)?
        } else {
            scalar(b, if ok { 0.4 } else { 125.0 })?
        };
        let (a, b) = if polar {
            let c = a.max(0.0);
            (c * b.cos(), c * b.sin())
        } else {
            (a, b)
        };
        if ok {
            oklab([l, a, b])
        } else {
            let fy = (l + 16.0) / 116.0;
            let fx = fy + a / 500.0;
            let fz = fy - b / 200.0;
            let inverse = |v: f64| {
                if v > 6.0 / 29.0 {
                    v.powi(3)
                } else {
                    3.0 * (6.0f64 / 29.0).powi(2) * (v - 4.0 / 29.0)
                }
            };
            rgb(d65([
                inverse(fx) * 0.9642956764,
                inverse(fy),
                inverse(fz) * 0.8251046025,
            ]))
        }
    };
    Some(rgba(result, alpha))
}
fn mix(args: &[Value]) -> Option<Value> {
    let groups: Vec<_> = args
        .split(|v| matches!(v,Value::Ident(s) if s==","))
        .collect();
    let [space, left, right] = groups.as_slice() else {
        return None;
    };
    let [Value::Ident(in_), Value::Ident(space)] = *space else {
        return None;
    };
    if in_ != "in" {
        return None;
    }
    let component = |values: &[Value]| {
        let [color, rest @ ..] = values else {
            return None;
        };
        let Value::Color(r, g, b, a) = color else {
            return None;
        };
        let weight = match rest {
            [] => None,
            [Value::Percentage(n)] if (0.0..=100.0).contains(n) => Some(*n / 100.0),
            _ => return None,
        };
        Some((
            [*r as f64 / 255.0, *g as f64 / 255.0, *b as f64 / 255.0],
            *a as f64 / 255.0,
            weight,
        ))
    };
    let (mut a, aa, wa) = component(left)?;
    let (mut b, ba, wb) = component(right)?;
    let wa = wa.unwrap_or_else(|| wb.map_or(0.5, |w| 1.0 - w));
    let wb = wb.unwrap_or(1.0 - wa);
    let sum = wa + wb;
    if sum <= 0.0 {
        return None;
    }
    let multiplier = sum.min(1.0);
    let wa = wa / sum;
    let wb = wb / sum;
    match space.as_str() {
        "srgb" => {}
        "srgb-linear" => {
            a = a.map(linear);
            b = b.map(linear);
        }
        "oklab" => {
            a = to_oklab(a);
            b = to_oklab(b);
        }
        _ => return None,
    }
    let alpha = aa * wa + ba * wb;
    let values = std::array::from_fn(|i| {
        if alpha == 0.0 {
            0.0
        } else {
            (a[i] * aa * wa + b[i] * ba * wb) / alpha
        }
    });
    let values = match space.as_str() {
        "srgb-linear" => values.map(encoded),
        "oklab" => oklab(values),
        _ => values,
    };
    Some(rgba(values, alpha * multiplier))
}
