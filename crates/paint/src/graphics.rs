//! Affine geometry, rounded paths, and bounded device-space coverage masks.
use super::*;
use tiny_skia::{FillRule, Mask, Path, PathBuilder, Point, Transform};

pub type Radii = [(f32, f32); 4];

pub(super) struct DeviceMask {
    pub x: i32,
    pub y: i32,
    pub mask: Mask,
}
impl DeviceMask {
    pub fn coverage(&self, x: i32, y: i32) -> u8 {
        let (x, y) = (x - self.x, y - self.y);
        if x < 0 || y < 0 || x >= self.mask.width() as i32 || y >= self.mask.height() as i32 {
            return 0;
        }
        self.mask.data()[y as usize * self.mask.width() as usize + x as usize]
    }
}

pub fn radii(style: &Style, rect: Rect) -> Radii {
    let mut result = [(0.0, 0.0); 4];
    for (i, name) in [
        "border-top-left-radius",
        "border-top-right-radius",
        "border-bottom-right-radius",
        "border-bottom-left-radius",
    ]
    .iter()
    .enumerate()
    {
        if let Some(values) = style.get(name).filter(|v| !v.is_empty()) {
            result[i].0 =
                sol_css::style::value_px(&values[0], 16.0, rect.w as f64).unwrap_or(0.0) as f32;
            result[i].1 =
                sol_css::style::value_px(values.get(1).unwrap_or(&values[0]), 16.0, rect.h as f64)
                    .unwrap_or(0.0) as f32;
        }
    }
    normalize(rect, result)
}

fn normalize(rect: Rect, mut radii: Radii) -> Radii {
    for (x, y) in &mut radii {
        *x = if x.is_finite() { x.max(0.0) } else { 0.0 };
        *y = if y.is_finite() { y.max(0.0) } else { 0.0 };
    }
    let mut scale = 1.0f32;
    for (size, sum) in [
        (rect.w, radii[0].0 + radii[1].0),
        (rect.w, radii[2].0 + radii[3].0),
        (rect.h, radii[0].1 + radii[3].1),
        (rect.h, radii[1].1 + radii[2].1),
    ] {
        if sum > 0.0 {
            scale = scale.min((size / sum).max(0.0));
        }
    }
    for (x, y) in &mut radii {
        *x *= scale;
        *y *= scale;
    }
    radii
}

pub fn inset_radii(radii: Radii, edges: sol_layout::Edges) -> Radii {
    [
        (radii[0].0 - edges.left, radii[0].1 - edges.top),
        (radii[1].0 - edges.right, radii[1].1 - edges.top),
        (radii[2].0 - edges.right, radii[2].1 - edges.bottom),
        (radii[3].0 - edges.left, radii[3].1 - edges.bottom),
    ]
}

fn rounded_path(rect: Rect, radii: Radii) -> Option<Path> {
    tiny_skia::Rect::from_xywh(rect.x, rect.y, rect.w, rect.h)?;
    let r = normalize(rect, radii);
    let k = 0.5522848;
    let (x, y, right, bottom) = (rect.x, rect.y, rect.right(), rect.bottom());
    let mut p = PathBuilder::new();
    p.move_to(x + r[0].0, y);
    p.line_to(right - r[1].0, y);
    p.cubic_to(
        right - r[1].0 + k * r[1].0,
        y,
        right,
        y + r[1].1 - k * r[1].1,
        right,
        y + r[1].1,
    );
    p.line_to(right, bottom - r[2].1);
    p.cubic_to(
        right,
        bottom - r[2].1 + k * r[2].1,
        right - r[2].0 + k * r[2].0,
        bottom,
        right - r[2].0,
        bottom,
    );
    p.line_to(x + r[3].0, bottom);
    p.cubic_to(
        x + r[3].0 - k * r[3].0,
        bottom,
        x,
        bottom - r[3].1 + k * r[3].1,
        x,
        bottom - r[3].1,
    );
    p.line_to(x, y + r[0].1);
    p.cubic_to(
        x,
        y + r[0].1 - k * r[0].1,
        x + r[0].0 - k * r[0].0,
        y,
        x + r[0].0,
        y,
    );
    p.close();
    p.finish()
}

/// Parse the supported CSS 2D transform functions around transform-origin.
pub fn css_transform(style: &Style, rect: Rect) -> Option<Transform> {
    let values = style.get("transform")?;
    let mut matrix = Transform::identity();
    let mut seen = false;
    for value in values {
        let Value::Func(name, args) = value else {
            return None;
        };
        let args: Vec<_> = args
            .iter()
            .filter(|value| !matches!(value, Value::Ident(s) if s == ","))
            .collect();
        let number = |index: usize| -> Option<f32> {
            match args.get(index)? {
                Value::Number(n) => Some(*n as f32),
                _ => None,
            }
        };
        let length = |index: usize, base: f32| -> Option<f32> {
            use sol_css::values::Unit;
            let value = args.get(index)?;
            match value {
                Value::Number(n) if *n == 0.0 => Some(0.0),
                Value::Percentage(_)
                | Value::Length(
                    _,
                    Unit::Px
                    | Unit::Em
                    | Unit::Rem
                    | Unit::Ex
                    | Unit::Ch
                    | Unit::Vw
                    | Unit::Vh
                    | Unit::Vmin
                    | Unit::Vmax
                    | Unit::Cm
                    | Unit::Mm
                    | Unit::In
                    | Unit::Pt
                    | Unit::Pc
                    | Unit::Q
                    | Unit::Pct,
                ) => sol_css::style::value_px(value, 16.0, base as f64).map(|n| n as f32),
                _ => None,
            }
        };
        let angle = |index: usize| -> Option<f32> {
            use sol_css::values::Unit;
            Some(match args.get(index)? {
                Value::Length(n, Unit::Deg) => *n as f32,
                Value::Length(n, Unit::Rad) => (*n as f32).to_degrees(),
                Value::Length(n, Unit::Turn) => *n as f32 * 360.0,
                Value::Length(n, Unit::Grad) => *n as f32 * 0.9,
                Value::Number(n) if *n == 0.0 => 0.0,
                _ => return None,
            })
        };
        let next = match name.as_str() {
            "translate" if (1..=2).contains(&args.len()) => Transform::from_translate(
                length(0, rect.w)?,
                if args.len() == 2 {
                    length(1, rect.h)?
                } else {
                    0.0
                },
            ),
            "translatex" if args.len() == 1 => Transform::from_translate(length(0, rect.w)?, 0.0),
            "translatey" if args.len() == 1 => Transform::from_translate(0.0, length(0, rect.h)?),
            "scale" if (1..=2).contains(&args.len()) => Transform::from_scale(
                number(0)?,
                if args.len() == 2 {
                    number(1)?
                } else {
                    number(0)?
                },
            ),
            "scalex" if args.len() == 1 => Transform::from_scale(number(0)?, 1.0),
            "scaley" if args.len() == 1 => Transform::from_scale(1.0, number(0)?),
            "rotate" if args.len() == 1 => Transform::from_rotate(angle(0)?),
            "skew" if (1..=2).contains(&args.len()) => Transform::from_row(
                1.0,
                if args.len() == 2 {
                    angle(1)?.to_radians().tan()
                } else {
                    0.0
                },
                angle(0)?.to_radians().tan(),
                1.0,
                0.0,
                0.0,
            ),
            "skewx" if args.len() == 1 => {
                Transform::from_row(1.0, 0.0, angle(0)?.to_radians().tan(), 1.0, 0.0, 0.0)
            }
            "skewy" if args.len() == 1 => {
                Transform::from_row(1.0, angle(0)?.to_radians().tan(), 0.0, 1.0, 0.0, 0.0)
            }
            "matrix" if args.len() == 6 => Transform::from_row(
                number(0)?,
                number(1)?,
                number(2)?,
                number(3)?,
                number(4)?,
                number(5)?,
            ),
            _ => return None,
        };
        matrix = matrix.pre_concat(next);
        seen = true;
    }
    if !seen || !matrix.is_finite() {
        return None;
    }
    let origin = style.get("transform-origin");
    let origin = origin.map(Vec::as_slice).unwrap_or(&[]);
    let resolve = |value: Option<&Value>, base: f32| -> f32 {
        match value {
            Some(Value::Ident(s)) if matches!(s.as_str(), "left" | "top") => 0.0,
            Some(Value::Ident(s)) if matches!(s.as_str(), "right" | "bottom") => base,
            Some(v) => {
                sol_css::style::value_px(v, 16.0, base as f64).unwrap_or(base as f64 / 2.0) as f32
            }
            None => base / 2.0,
        }
    };
    let (mut horizontal, mut vertical) = (origin.first(), origin.get(1));
    let is_vertical =
        |value: Option<&Value>| matches!(value,Some(Value::Ident(s)) if s=="top"||s=="bottom");
    let is_horizontal =
        |value: Option<&Value>| matches!(value,Some(Value::Ident(s)) if s=="left"||s=="right");
    if is_vertical(horizontal) || is_horizontal(vertical) {
        std::mem::swap(&mut horizontal, &mut vertical);
    }
    let (ox, oy) = (
        rect.x + resolve(horizontal, rect.w),
        rect.y + resolve(vertical, rect.h),
    );
    Some(
        Transform::from_translate(ox, oy)
            .pre_concat(matrix)
            .pre_concat(Transform::from_translate(-ox, -oy)),
    )
}

impl Canvas {
    pub fn push_transform(&mut self, matrix: Transform) {
        self.transform_stack.push(self.transform);
        self.transform = self.transform.pre_concat(matrix);
    }
    pub fn pop_transform(&mut self) {
        if let Some(matrix) = self.transform_stack.pop() {
            self.transform = matrix;
        }
    }
    fn path_mask(&self, path: &Path, rule: FillRule) -> Option<DeviceMask> {
        let device = path.clone().transform(self.transform)?;
        let bounds = device.bounds();
        let visible = self.visible_rect(Rect {
            x: bounds.x(),
            y: bounds.y(),
            w: bounds.width(),
            h: bounds.height(),
        })?;
        let (x, y) = (visible.x.floor() as i32, visible.y.floor() as i32);
        let (w, h) = (
            (visible.right().ceil() as i32 - x) as u32,
            (visible.bottom().ceil() as i32 - y) as u32,
        );
        let mut mask = Mask::new(w, h)?;
        mask.fill_path(
            &device,
            rule,
            true,
            Transform::from_translate(-x as f32, -y as f32),
        );
        Some(DeviceMask { x, y, mask })
    }
    pub(super) fn fill_path(&mut self, path: &Path, rule: FillRule, color: Rgba) {
        if let Some(mask) = self.path_mask(path, rule) {
            for y in 0..mask.mask.height() as i32 {
                for x in 0..mask.mask.width() as i32 {
                    self.blend(
                        mask.x + x,
                        mask.y + y,
                        color,
                        mask.coverage(mask.x + x, mask.y + y),
                    );
                }
            }
        }
    }
    fn push_path_clip(&mut self, path: Option<Path>, rule: FillRule) {
        self.clip_stack.push((self.clip, self.masks.len()));
        if let Some(mask) = path.as_ref().and_then(|path| self.path_mask(path, rule)) {
            let retained: usize = self.masks.iter().map(|mask| mask.mask.data().len()).sum();
            if mask.mask.data().len() > 64 * 1024 * 1024usize - retained {
                self.clip.w = 0.0;
                self.clip.h = 0.0;
                return;
            }
            self.clip = Rect {
                x: mask.x as f32,
                y: mask.y as f32,
                w: mask.mask.width() as f32,
                h: mask.mask.height() as f32,
            };
            self.masks.push(mask);
        } else {
            self.clip.w = 0.0;
            self.clip.h = 0.0;
        }
    }
    pub fn fill_rounded_rect(&mut self, rect: Rect, radii: Radii, color: Rgba) {
        if self.transform.is_identity() && radii.iter().all(|&(x, y)| x == 0.0 && y == 0.0) {
            self.fill_rect(rect, color);
            return;
        }
        if let Some(path) = rounded_path(rect, radii) {
            self.fill_path(&path, FillRule::Winding, color);
        }
    }
    pub fn push_rounded_clip(&mut self, rect: Rect, radii: Radii) {
        self.push_path_clip(rounded_path(rect, radii), FillRule::Winding);
    }
    pub fn rounded_border(
        &mut self,
        rect: Rect,
        radii: Radii,
        edges: sol_layout::Edges,
        colors: [Rgba; 4],
        styles: [u8; 4],
    ) {
        let uniform = [edges.right, edges.bottom, edges.left]
            .iter()
            .all(|&w| w == edges.top)
            && colors.iter().all(|color| *color == colors[0])
            && styles.iter().all(|style| *style == styles[0]);
        // Dash generation runs in local coordinates. Bound the number of
        // segments before asking the stroker to expand a hostile huge path.
        if uniform
            && matches!(styles[0], 1..=3)
            && edges.top > 0.0
            && (rect.w + rect.h) / edges.top <= 32768.0
        {
            let width = edges.top;
            let strokes: Vec<(f32, f32)> = if styles[0] == 3 {
                vec![(width / 6.0, width / 3.0), (width * 5.0 / 6.0, width / 3.0)]
            } else {
                vec![(width / 2.0, width)]
            };
            for (inset, stroke_width) in strokes {
                let r = Rect {
                    x: rect.x + inset,
                    y: rect.y + inset,
                    w: rect.w - 2.0 * inset,
                    h: rect.h - 2.0 * inset,
                };
                let edge = sol_layout::Edges {
                    top: inset,
                    right: inset,
                    bottom: inset,
                    left: inset,
                };
                let Some(path) = rounded_path(r, inset_radii(radii, edge)) else {
                    continue;
                };
                let mut stroke = tiny_skia::Stroke {
                    width: stroke_width,
                    ..Default::default()
                };
                if styles[0] == 1 {
                    stroke.dash = tiny_skia::StrokeDash::new(vec![width * 3.0, width * 3.0], 0.0);
                }
                if styles[0] == 2 {
                    stroke.line_cap = tiny_skia::LineCap::Round;
                    stroke.dash = tiny_skia::StrokeDash::new(vec![width * 0.01, width * 2.0], 0.0);
                }
                if let Some(path) = path.stroke(&stroke, 1.0) {
                    self.fill_path(&path, FillRule::Winding, colors[0]);
                }
            }
            return;
        }
        let Some(outer) = rounded_path(rect, radii) else {
            return;
        };
        let inner_rect = Rect {
            x: rect.x + edges.left,
            y: rect.y + edges.top,
            w: rect.w - edges.horiz(),
            h: rect.h - edges.vert(),
        };
        let mut ring = PathBuilder::new();
        ring.push_path(&outer);
        if let Some(inner) = rounded_path(inner_rect, inset_radii(radii, edges)) {
            ring.push_path(&inner);
        }
        self.push_path_clip(ring.finish(), FillRule::EvenOdd);
        let outer_points = [
            (rect.x, rect.y),
            (rect.right(), rect.y),
            (rect.right(), rect.bottom()),
            (rect.x, rect.bottom()),
        ];
        let inner_points = [
            (inner_rect.x, inner_rect.y),
            (inner_rect.right(), inner_rect.y),
            (inner_rect.right(), inner_rect.bottom()),
            (inner_rect.x, inner_rect.bottom()),
        ];
        for side in 0..4 {
            if styles[side] == 255 {
                continue;
            }
            let next = (side + 1) % 4;
            let mut p = PathBuilder::new();
            p.move_to(outer_points[side].0, outer_points[side].1);
            p.line_to(outer_points[next].0, outer_points[next].1);
            p.line_to(inner_points[next].0, inner_points[next].1);
            p.line_to(inner_points[side].0, inner_points[side].1);
            p.close();
            if let Some(path) = p.finish() {
                self.fill_path(&path, FillRule::Winding, colors[side]);
            }
        }
        self.pop_clip();
    }
    pub(super) fn blit_transformed(&mut self, img: &Image, rect: Rect) {
        let Some(inverse) = self.transform.invert() else {
            return;
        };
        let Some(path) = rounded_path(rect, [(0.0, 0.0); 4]) else {
            return;
        };
        let Some(mask) = self.path_mask(&path, FillRule::Winding) else {
            return;
        };
        for y in mask.y..mask.y + mask.mask.height() as i32 {
            for x in mask.x..mask.x + mask.mask.width() as i32 {
                let mut p = Point::from_xy(x as f32 + 0.5, y as f32 + 0.5);
                inverse.map_point(&mut p);
                let sx =
                    ((p.x - rect.x) / rect.w * img.w as f32 - 0.5).clamp(0.0, (img.w - 1) as f32);
                let sy =
                    ((p.y - rect.y) / rect.h * img.h as f32 - 0.5).clamp(0.0, (img.h - 1) as f32);
                let (ix, iy) = (sx.floor() as u32, sy.floor() as u32);
                let (fx, fy) = (sx.fract(), sy.fract());
                let mut rgba = [0.0f32; 4];
                for (ix, iy, weight) in [
                    (ix, iy, (1.0 - fx) * (1.0 - fy)),
                    ((ix + 1).min(img.w - 1), iy, fx * (1.0 - fy)),
                    (ix, (iy + 1).min(img.h - 1), (1.0 - fx) * fy),
                    ((ix + 1).min(img.w - 1), (iy + 1).min(img.h - 1), fx * fy),
                ] {
                    let p = img.get(ix, iy);
                    let a = p[3] as f32 * weight;
                    for c in 0..3 {
                        rgba[c] += p[c] as f32 * a;
                    }
                    rgba[3] += a;
                }
                if rgba[3] > 0.0 {
                    self.blend(
                        x,
                        y,
                        Rgba(
                            (rgba[0] / rgba[3]).round() as u8,
                            (rgba[1] / rgba[3]).round() as u8,
                            (rgba[2] / rgba[3]).round() as u8,
                            rgba[3].round() as u8,
                        ),
                        mask.coverage(x, y),
                    );
                }
            }
        }
    }
}

/// Axis-aligned bounds of the transformed border box.
pub fn transformed_rect(rect: Rect, transform: Transform) -> Rect {
    let mut points = [
        Point::from_xy(rect.x, rect.y),
        Point::from_xy(rect.right(), rect.y),
        Point::from_xy(rect.right(), rect.bottom()),
        Point::from_xy(rect.x, rect.bottom()),
    ];
    transform.map_points(&mut points);
    let x = points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let y = points.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let right = points.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
    let bottom = points.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
    Rect {
        x,
        y,
        w: right - x,
        h: bottom - y,
    }
}

pub fn contains_rounded(style: &Style, rect: Rect, x: f32, y: f32) -> bool {
    if !rect.contains(x, y) {
        return false;
    }
    let radii = radii(style, rect);
    let centers = [
        (rect.x + radii[0].0, rect.y + radii[0].1),
        (rect.right() - radii[1].0, rect.y + radii[1].1),
        (rect.right() - radii[2].0, rect.bottom() - radii[2].1),
        (rect.x + radii[3].0, rect.bottom() - radii[3].1),
    ];
    for (i, ((cx, cy), (rx, ry))) in centers.into_iter().zip(radii).enumerate() {
        let corner = match i {
            0 => x < cx && y < cy,
            1 => x > cx && y < cy,
            2 => x > cx && y > cy,
            _ => x < cx && y > cy,
        };
        if corner && rx > 0.0 && ry > 0.0 {
            return ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) <= 1.0;
        }
    }
    true
}
