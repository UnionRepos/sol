use crate::{Canvas, Radii, Rgba};
use sol_css::{Style, Value};
use sol_img::Image;
use sol_layout::Rect;
use std::{collections::HashMap, rc::Rc};

fn comma(v: &Value) -> bool {
    matches!(v,Value::Ident(s) if s==",")
}
fn layers<'a>(style: &'a Style, name: &str) -> Vec<&'a [Value]> {
    style
        .get(name)
        .map(|v| v.split(comma).collect())
        .unwrap_or_default()
}
fn layer<'a>(values: &[&'a [Value]], index: usize) -> &'a [Value] {
    if values.is_empty() {
        &[]
    } else {
        values[index % values.len()]
    }
}
fn keyword(values: &[Value], default: &str) -> String {
    values
        .first()
        .and_then(|v| {
            if let Value::Ident(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| default.into())
}
fn px(v: &Value, base: f32) -> Option<f32> {
    sol_css::style::value_px(v, 16.0, base as f64)
        .map(|n| n as f32)
        .filter(|n| n.is_finite())
}
fn color(v: &Value, style: &Style) -> Option<Rgba> {
    match v {
        Value::Color(r, g, b, a) => Some(Rgba(*r, *g, *b, *a)),
        Value::Ident(s) if s == "currentcolor" => style
            .color("color")
            .map(|c| Rgba(c[0], c[1], c[2], c[3]))
            .or(Some(Rgba(0, 0, 0, 255))),
        _ => None,
    }
}
fn area(name: &str, border: Rect, padding: Rect, content: Rect) -> Rect {
    match name {
        "content-box" => content,
        "padding-box" => padding,
        _ => border,
    }
}
fn inset(radii: Radii, border: Rect, area: Rect) -> Radii {
    crate::graphics::inset_radii(
        radii,
        sol_layout::Edges {
            top: area.y - border.y,
            right: border.right() - area.right(),
            bottom: border.bottom() - area.bottom(),
            left: area.x - border.x,
        },
    )
}
fn offset(value: Option<&Value>, space: f32, horizontal: bool) -> f32 {
    match value {
        Some(Value::Ident(s)) => match s.as_str() {
            "center" => space / 2.0,
            "right" if horizontal => space,
            "bottom" if !horizontal => space,
            _ => 0.0,
        },
        Some(value) => px(value, space).unwrap_or(0.0),
        None => 0.0,
    }
}
pub(super) fn position(values: &[Value], w: f32, h: f32) -> (f32, f32) {
    let mut values = values.to_vec();
    if values.len() == 1 {
        if matches!(&values[0],Value::Ident(s) if matches!(s.as_str(),"top"|"bottom")) {
            values.insert(0, Value::Ident("center".into()));
        } else {
            values.push(Value::Ident("center".into()));
        }
    }
    if values.len() == 2
        && matches!(&values[0],Value::Ident(s) if matches!(s.as_str(),"top"|"bottom"))
    {
        values.swap(0, 1);
    }
    if values.len() > 2 {
        let mut x = None;
        let mut y = None;
        let mut i = 0;
        while i < values.len() {
            if let Value::Ident(edge) = &values[i] {
                let distance = values
                    .get(i + 1)
                    .filter(|v| !matches!(v, Value::Ident(_)))
                    .and_then(|v| {
                        px(
                            v,
                            if matches!(edge.as_str(), "left" | "right") {
                                w
                            } else {
                                h
                            },
                        )
                    });
                match edge.as_str() {
                    "left" => x = Some(distance.unwrap_or(0.0)),
                    "right" => x = Some(w - distance.unwrap_or(0.0)),
                    "top" => y = Some(distance.unwrap_or(0.0)),
                    "bottom" => y = Some(h - distance.unwrap_or(0.0)),
                    "center" => {
                        if x.is_none() {
                            x = Some(w / 2.0)
                        } else {
                            y = Some(h / 2.0)
                        }
                    }
                    _ => {}
                }
                if distance.is_some() {
                    i += 1;
                }
            }
            i += 1;
        }
        return (x.unwrap_or(0.0), y.unwrap_or(0.0));
    }
    (
        offset(values.first(), w, true),
        offset(values.get(1), h, false),
    )
}
fn size(values: &[Value], area: Rect, natural: (f32, f32)) -> (f32, f32) {
    let (nw, nh) = natural;
    if let Some(Value::Ident(name)) = values.first() {
        if matches!(name.as_str(), "cover" | "contain") && nw > 0.0 && nh > 0.0 {
            let factor = if name == "cover" {
                (area.w / nw).max(area.h / nh)
            } else {
                (area.w / nw).min(area.h / nh)
            };
            return (nw * factor, nh * factor);
        }
    }
    let w = values.first().and_then(|v| px(v, area.w));
    let h = values.get(1).and_then(|v| px(v, area.h));
    match (w, h) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, if nw > 0.0 { nh * w / nw } else { area.h }),
        (None, Some(h)) => (if nh > 0.0 { nw * h / nh } else { area.w }, h),
        _ => natural,
    }
}
fn coordinate(
    value: f32,
    start: f32,
    size: f32,
    mode: &str,
    origin: f32,
    extent: f32,
) -> Option<f32> {
    if size <= 0.0 || !size.is_finite() {
        return None;
    }
    if mode == "no-repeat" {
        let local = value - start;
        return (local >= 0.0 && local < size).then_some(local);
    }
    if mode == "space" {
        let count = (extent / size).floor();
        if count >= 2.0 {
            let gap = (extent - count * size) / (count - 1.0);
            let local = (value - origin).rem_euclid(size + gap);
            return (local < size).then_some(local);
        }
        return coordinate(value, start, size, "no-repeat", origin, extent);
    }
    Some((value - start).rem_euclid(size))
}

pub(super) fn paint(
    canvas: &mut Canvas,
    border: Rect,
    padding: Rect,
    content: Rect,
    radii: Radii,
    style: &Style,
    images: &HashMap<String, Rc<Image>>,
) {
    let images_values = layers(style, "background-image");
    let clips = layers(style, "background-clip");
    let origins = layers(style, "background-origin");
    let clip = area(
        &keyword(
            layer(&clips, images_values.len().saturating_sub(1)),
            "border-box",
        ),
        border,
        padding,
        content,
    );
    if let Some(value) = style
        .get("background-color")
        .and_then(|v| v.first())
        .and_then(|value| color(value, style))
    {
        canvas.fill_rounded_rect(clip, inset(radii, border, clip), value);
    }
    let positions = layers(style, "background-position");
    let sizes = layers(style, "background-size");
    let repeats = layers(style, "background-repeat");
    let attachments = layers(style, "background-attachment");
    for index in (0..images_values.len().min(128)).rev() {
        let Some(value) = images_values[index].first() else {
            continue;
        };
        let image = if let Value::Url(url) = value {
            images.get(url).map(Rc::as_ref)
        } else {
            None
        };
        if image.is_none() && !matches!(value,Value::Func(name,_) if name.ends_with("gradient")) {
            continue;
        }
        let mut origin = area(
            &keyword(layer(&origins, index), "padding-box"),
            border,
            padding,
            content,
        );
        if keyword(layer(&attachments, index), "scroll") == "fixed" {
            origin = Rect {
                x: 0.0,
                y: 0.0,
                w: canvas.w as f32,
                h: canvas.h as f32,
            };
        }
        let natural = image
            .map(|img| (img.w as f32, img.h as f32))
            .unwrap_or((origin.w, origin.h));
        let (mut w, mut h) = size(layer(&sizes, index), origin, natural);
        let repeat = layer(&repeats, index);
        let mut rx = keyword(repeat, "repeat");
        let mut ry = keyword(repeat.get(1..).unwrap_or_default(), &rx);
        if rx == "repeat-x" {
            rx = "repeat".into();
            ry = "no-repeat".into();
        } else if rx == "repeat-y" {
            rx = "no-repeat".into();
            ry = "repeat".into();
        }
        if rx == "round" && w > 0.0 {
            w = origin.w / (origin.w / w).round().max(1.0);
        }
        if ry == "round" && h > 0.0 {
            h = origin.h / (origin.h / h).round().max(1.0);
        }
        if w <= 0.0 || h <= 0.0 || !w.is_finite() || !h.is_finite() {
            continue;
        }
        let (dx, dy) = position(layer(&positions, index), origin.w - w, origin.h - h);
        let gradient = if let Value::Func(name, args) = value {
            Gradient::new(name, args, w, h, style)
        } else {
            None
        };
        if image.is_none() && gradient.is_none() {
            continue;
        }
        let clip = area(
            &keyword(layer(&clips, index), "border-box"),
            border,
            padding,
            content,
        );
        canvas.push_rounded_clip(clip, inset(radii, border, clip));
        canvas.shade(clip, |x, y| {
            let x = coordinate(x, origin.x + dx, w, &rx, origin.x, origin.w)?;
            let y = coordinate(y, origin.y + dy, h, &ry, origin.y, origin.h)?;
            image
                .map(|image| sample(image, x / w, y / h))
                .or_else(|| gradient.as_ref().map(|g| g.sample(x, y)))
        });
        canvas.pop_clip();
    }
}

impl Canvas {
    pub(super) fn shade(&mut self, rect: Rect, sample: impl Fn(f32, f32) -> Option<Rgba>) {
        if self.suppress {
            return;
        }
        let Some(inverse) = self.transform.invert() else {
            return;
        };
        let Some(visible) = self.visible_rect(crate::transformed_rect(rect, self.transform)) else {
            return;
        };
        for y in visible.y.floor() as i32..visible.bottom().ceil() as i32 {
            for x in visible.x.floor() as i32..visible.right().ceil() as i32 {
                let mut p = crate::Point::from_xy(x as f32 + 0.5, y as f32 + 0.5);
                inverse.map_point(&mut p);
                if rect.contains(p.x, p.y) {
                    if let Some(color) = sample(p.x, p.y) {
                        self.blend(x, y, color, 255);
                    }
                }
            }
        }
    }
}
fn sample(image: &Image, x: f32, y: f32) -> Rgba {
    let x = (x * image.w as f32 - 0.5).clamp(0.0, image.w.saturating_sub(1) as f32);
    let y = (y * image.h as f32 - 0.5).clamp(0.0, image.h.saturating_sub(1) as f32);
    let ix = x.floor() as u32;
    let iy = y.floor() as u32;
    let fx = x.fract();
    let fy = y.fract();
    let mut out = [0.0; 4];
    for (x, y, w) in [
        (ix, iy, (1.0 - fx) * (1.0 - fy)),
        ((ix + 1).min(image.w - 1), iy, fx * (1.0 - fy)),
        (ix, (iy + 1).min(image.h - 1), (1.0 - fx) * fy),
        (
            (ix + 1).min(image.w - 1),
            (iy + 1).min(image.h - 1),
            fx * fy,
        ),
    ] {
        let i = ((y * image.w + x) * 4) as usize;
        let a = image.px[i + 3] as f32 / 255.0;
        for (channel, component) in out.iter_mut().take(3).enumerate() {
            *component += image.px[i + channel] as f32 * a * w;
        }
        out[3] += a * w;
    }
    if out[3] == 0.0 {
        return Rgba(0, 0, 0, 0);
    }
    Rgba(
        (out[0] / out[3]).round() as u8,
        (out[1] / out[3]).round() as u8,
        (out[2] / out[3]).round() as u8,
        (out[3] * 255.0).round() as u8,
    )
}

struct Stop {
    position: f32,
    color: Rgba,
}
enum Geometry {
    Linear { dx: f32, dy: f32, length: f32 },
    Radial { cx: f32, cy: f32, rx: f32, ry: f32 },
    Conic { cx: f32, cy: f32, from: f32 },
}
struct Gradient {
    geometry: Geometry,
    stops: Vec<Stop>,
    repeat: bool,
    w: f32,
    h: f32,
}
fn angle(value: &Value) -> Option<f32> {
    use sol_css::values::Unit;
    match value {
        Value::Length(n, u) => Some(match u {
            Unit::Deg => n.to_radians(),
            Unit::Rad => *n,
            Unit::Grad => n * std::f64::consts::PI / 200.0,
            Unit::Turn => n * std::f64::consts::TAU,
            _ => return None,
        } as f32),
        Value::Number(0.0) => Some(0.0),
        _ => None,
    }
}
impl Gradient {
    fn new(name: &str, args: &[Value], w: f32, h: f32, style: &Style) -> Option<Self> {
        let groups: Vec<_> = args.split(comma).collect();
        let first = *groups.first()?;
        let control = first.first().is_none_or(|v| color(v, style).is_none());
        let control_values = if control { first } else { &[] };
        let groups = &groups[usize::from(control)..];
        let geometry = if name.ends_with("linear-gradient") {
            let (dx, dy) = if let Some(a) = control_values.first().and_then(angle) {
                (a.sin(), -a.cos())
            } else if matches!(control_values.first(),Some(Value::Ident(s)) if s=="to") {
                let mut dx: f32 = 0.0;
                let mut dy: f32 = 0.0;
                for v in &control_values[1..] {
                    if let Value::Ident(s) = v {
                        match s.as_str() {
                            "left" => dx = -1.0,
                            "right" => dx = 1.0,
                            "top" => dy = -1.0,
                            "bottom" => dy = 1.0,
                            _ => return None,
                        }
                    }
                }
                if dx != 0.0 && dy != 0.0 {
                    dx *= h;
                    dy *= w;
                }
                let length = dx.hypot(dy);
                if length == 0.0 {
                    return None;
                }
                (dx / length, dy / length)
            } else {
                (0.0, 1.0)
            };
            Geometry::Linear {
                dx,
                dy,
                length: (dx.abs() * w + dy.abs() * h).max(f32::EPSILON),
            }
        } else if name.ends_with("radial-gradient") {
            let at = control_values
                .iter()
                .position(|v| matches!(v,Value::Ident(s) if s=="at"));
            let (cx, cy) = at
                .map(|at| position(&control_values[at + 1..], w, h))
                .unwrap_or((w / 2.0, h / 2.0));
            let shape = &control_values[..at.unwrap_or(control_values.len())];
            let circle = shape
                .iter()
                .any(|v| matches!(v,Value::Ident(s) if s=="circle"));
            let closest = shape
                .iter()
                .any(|v| matches!(v,Value::Ident(s) if s.starts_with("closest")));
            let side = shape
                .iter()
                .any(|v| matches!(v,Value::Ident(s) if s.ends_with("side")));
            let mut rx = if closest {
                cx.min(w - cx)
            } else {
                cx.max(w - cx)
            };
            let mut ry = if closest {
                cy.min(h - cy)
            } else {
                cy.max(h - cy)
            };
            if circle {
                let r = if side {
                    if closest {
                        rx.min(ry)
                    } else {
                        rx.max(ry)
                    }
                } else {
                    rx.hypot(ry)
                };
                rx = r;
                ry = r;
            } else if !side {
                rx *= std::f32::consts::SQRT_2;
                ry *= std::f32::consts::SQRT_2;
            }
            let lengths: Vec<_> = shape.iter().filter_map(|v| px(v, w)).collect();
            if let Some(r) = lengths.first() {
                rx = *r;
                ry = lengths.get(1).copied().unwrap_or(*r);
            }
            Geometry::Radial {
                cx,
                cy,
                rx: rx.max(f32::EPSILON),
                ry: ry.max(f32::EPSILON),
            }
        } else if name.ends_with("conic-gradient") {
            let at = control_values
                .iter()
                .position(|v| matches!(v,Value::Ident(s) if s=="at"));
            let (cx, cy) = at
                .map(|at| position(&control_values[at + 1..], w, h))
                .unwrap_or((w / 2.0, h / 2.0));
            let from = control_values
                .windows(2)
                .find_map(|v| {
                    if matches!(&v[0],Value::Ident(s) if s=="from") {
                        angle(&v[1])
                    } else {
                        None
                    }
                })
                .unwrap_or(0.0);
            Geometry::Conic { cx, cy, from }
        } else {
            return None;
        };
        let distance = match geometry {
            Geometry::Linear { length, .. } => length,
            Geometry::Radial { rx, .. } => rx,
            Geometry::Conic { .. } => 1.0,
        };
        let mut raw = Vec::new();
        for group in groups {
            let c = color(group.first()?, style)?;
            let positions = &group[1..];
            if positions.len() > 2 {
                return None;
            }
            if positions.is_empty() {
                raw.push((None, c));
            } else {
                for value in positions {
                    let position = match value {
                        Value::Percentage(n) => *n as f32 / 100.0,
                        value if matches!(geometry, Geometry::Conic { .. }) => {
                            angle(value)? / std::f32::consts::TAU
                        }
                        value => px(value, distance)? / distance,
                    };
                    raw.push((Some(position), c));
                }
            }
        }
        if raw.len() < 2 {
            return None;
        }
        if raw[0].0.is_none() {
            raw[0].0 = Some(0.0);
        }
        let last = raw.len() - 1;
        if raw[last].0.is_none() {
            raw[last].0 = Some(1.0);
        }
        let mut previous = f32::NEG_INFINITY;
        for (position, _) in &mut raw {
            if let Some(p) = position {
                *p = p.max(previous);
                previous = *p;
            }
        }
        let mut start = 0;
        while start < last {
            let end = (start + 1..raw.len()).find(|&i| raw[i].0.is_some())?;
            let a = raw[start].0?;
            let b = raw[end].0?;
            for (index, item) in raw.iter_mut().enumerate().take(end).skip(start + 1) {
                item.0 = Some(a + (b - a) * (index - start) as f32 / (end - start) as f32);
            }
            start = end;
        }
        Some(Self {
            geometry,
            stops: raw
                .into_iter()
                .map(|(position, color)| Stop {
                    position: position.unwrap(),
                    color,
                })
                .collect(),
            repeat: name.starts_with("repeating-"),
            w,
            h,
        })
    }
    fn sample(&self, x: f32, y: f32) -> Rgba {
        let mut t = match self.geometry {
            Geometry::Linear { dx, dy, length } => {
                ((x - self.w / 2.0) * dx + (y - self.h / 2.0) * dy) / length + 0.5
            }
            Geometry::Radial { cx, cy, rx, ry } => ((x - cx) / rx).hypot((y - cy) / ry),
            Geometry::Conic { cx, cy, from } => {
                ((x - cx).atan2(-(y - cy)) - from).rem_euclid(std::f32::consts::TAU)
                    / std::f32::consts::TAU
            }
        };
        let first = &self.stops[0];
        let last = self.stops.last().unwrap();
        if self.repeat {
            let length = last.position - first.position;
            if length <= f32::EPSILON {
                return last.color;
            }
            t = (t - first.position).rem_euclid(length) + first.position;
        }
        if t < first.position {
            return first.color;
        }
        let end = self.stops.partition_point(|stop| stop.position <= t);
        if end == self.stops.len() {
            return last.color;
        }
        if end == 0 {
            return first.color;
        }
        let a = &self.stops[end - 1];
        let b = &self.stops[end];
        mix(
            a.color,
            b.color,
            (t - a.position) / (b.position - a.position),
        )
    }
}
fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let aa = a.3 as f32 / 255.0;
    let ba = b.3 as f32 / 255.0;
    let alpha = aa * (1.0 - t) + ba * t;
    if alpha <= 0.0 {
        return Rgba(0, 0, 0, 0);
    }
    let component =
        |a: u8, b: u8| ((a as f32 * aa * (1.0 - t) + b as f32 * ba * t) / alpha).round() as u8;
    Rgba(
        component(a.0, b.0),
        component(a.1, b.1),
        component(a.2, b.2),
        (alpha * 255.0).round() as u8,
    )
}
