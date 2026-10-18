//! Shadows blur coverage before applying color so translucent shadows do not gain
//! dark fringes from repeatedly filtering already composited RGB pixels. Bounded
//! masks keep large spread and blur declarations from growing temporary storage
//! independently of the renderer's other resource limits.
use crate::{graphics, Canvas, DisplayList, Prim, Radii, Rgba};
use sol_css::{Style, Value};
use sol_layout::Rect;
use tiny_skia::{FillRule, Mask, Path, Transform};

#[derive(Clone, Debug)]
pub struct Shadow {
    pub dx: f32,
    pub dy: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Rgba,
    pub inset: bool,
}
fn shadows(style: &Style, name: &str) -> Vec<Shadow> {
    let mut out = Vec::new();
    for group in style
        .get(name)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .split(|v| matches!(v,Value::Ident(s) if s==","))
    {
        let mut lengths = Vec::new();
        let mut color = None;
        let mut inset = false;
        let mut valid = true;
        for value in group {
            match value {
                Value::Color(r, g, b, a) => color = Some(Rgba(*r, *g, *b, *a)),
                Value::Ident(s) if s == "inset" && name == "box-shadow" => inset = true,
                Value::Ident(s) if s == "currentcolor" => {}
                Value::Length(..) | Value::Number(0.0) | Value::Math(..) => {
                    if let Some(value) =
                        sol_css::style::value_px(value, 16.0, 0.0).filter(|v| v.is_finite())
                    {
                        lengths.push(value as f32)
                    } else {
                        valid = false
                    }
                }
                _ => valid = false,
            }
        }
        if !valid
            || !(2..=if name == "box-shadow" { 4 } else { 3 }).contains(&lengths.len())
            || lengths.get(2).is_some_and(|v| *v < 0.0)
        {
            continue;
        }
        let color = color
            .or_else(|| style.color("color").map(|c| Rgba(c[0], c[1], c[2], c[3])))
            .unwrap_or(Rgba(0, 0, 0, 255));
        out.push(Shadow {
            dx: lengths[0],
            dy: lengths[1],
            blur: lengths.get(2).copied().unwrap_or(0.0),
            spread: lengths.get(3).copied().unwrap_or(0.0),
            color,
            inset,
        });
    }
    out
}
pub(super) fn boxes(style: &Style, rect: Rect, radii: Radii, inset: bool, dl: &mut DisplayList) {
    for shadow in shadows(style, "box-shadow")
        .into_iter()
        .rev()
        .filter(|s| s.inset == inset)
    {
        dl.prims.push(Prim::Shadow {
            rect,
            radii,
            shadow,
        });
    }
}
pub(super) fn text(
    style: &Style,
    x: f32,
    baseline: f32,
    face_idx: usize,
    text: &str,
    dl: &mut DisplayList,
) {
    for shadow in shadows(style, "text-shadow").into_iter().rev() {
        dl.prims.push(Prim::TextShadow {
            x,
            baseline,
            face_idx,
            text: text.into(),
            shadow,
        });
    }
}
impl Canvas {
    pub(super) fn shadow(&mut self, rect: Rect, radii: Radii, shadow: &Shadow) {
        let original_radii = radii;
        let spread = if shadow.inset {
            -shadow.spread
        } else {
            shadow.spread
        };
        let shape = Rect {
            x: rect.x + shadow.dx - spread,
            y: rect.y + shadow.dy - spread,
            w: rect.w + 2.0 * spread,
            h: rect.h + 2.0 * spread,
        };
        let radii = radii.map(|(x, y)| ((x + spread).max(0.0), (y + spread).max(0.0)));
        if shadow.inset {
            self.push_rounded_clip(rect, original_radii);
        }
        let path = graphics::rounded_path(shape, radii);
        self.shadow_path(
            path,
            shadow,
            if shadow.inset { Some(rect) } else { None },
            if shadow.inset {
                None
            } else {
                graphics::rounded_path(rect, original_radii)
            },
        );
        if shadow.inset {
            self.pop_clip();
        }
    }
    pub(super) fn text_shadow(
        &mut self,
        face: &sol_text::Face,
        x: f32,
        baseline: f32,
        text: &str,
        shadow: &Shadow,
    ) {
        let mut builder = tiny_skia::PathBuilder::new();
        for glyph in face.shape(text).glyphs {
            if let Some(path) = glyph.font.glyph(glyph.id).path {
                let scale =
                    face.scale() * face.font.units_per_em as f32 / glyph.font.units_per_em as f32;
                if let Some(path) = path.transform(Transform::from_row(
                    scale,
                    0.0,
                    0.0,
                    -scale,
                    x + glyph.x + shadow.dx,
                    baseline - glyph.y + shadow.dy,
                )) {
                    builder.push_path(&path);
                }
            }
        }
        self.shadow_path(builder.finish(), shadow, None, None);
    }
    fn shadow_path(
        &mut self,
        path: Option<Path>,
        shadow: &Shadow,
        inset: Option<Rect>,
        exclusion: Option<Path>,
    ) {
        if self.suppress {
            return;
        }
        let scale = (self.transform.sx.hypot(self.transform.ky))
            .max(self.transform.kx.hypot(self.transform.sy));
        let radius = (shadow.blur * scale / 2.0).ceil().clamp(0.0, 128.0) as usize;
        let extension = radius as f32 * 3.0 + 2.0;
        let path = path.and_then(|path| path.transform(self.transform));
        let rect = if let Some(rect) = inset {
            crate::transformed_rect(rect, self.transform)
        } else if let Some(path) = &path {
            let b = path.bounds();
            Rect {
                x: b.x() - extension,
                y: b.y() - extension,
                w: b.width() + extension * 2.0,
                h: b.height() + extension * 2.0,
            }
        } else {
            return;
        };
        let Some(visible) = self.visible_rect(rect) else {
            return;
        };
        let x = (visible.x - extension).floor() as i32;
        let y = (visible.y - extension).floor() as i32;
        let w = (visible.right() + extension).ceil() as i32 - x;
        let h = (visible.bottom() + extension).ceil() as i32 - y;
        if w <= 0 || h <= 0 || w as u64 * h as u64 > 16 * 1024 * 1024 {
            return;
        }
        let Some(mut mask) = Mask::new(w as u32, h as u32) else {
            return;
        };
        if let Some(path) = path {
            mask.fill_path(
                &path,
                FillRule::Winding,
                true,
                Transform::from_translate(-x as f32, -y as f32),
            );
        }
        if inset.is_some() {
            for value in mask.data_mut() {
                *value = 255 - *value;
            }
        }
        if radius > 0 {
            let mut scratch = vec![0u8; mask.data().len()];
            for _ in 0..3 {
                blur(
                    mask.data(),
                    &mut scratch,
                    w as usize,
                    h as usize,
                    radius,
                    false,
                );
                blur(
                    &scratch,
                    mask.data_mut(),
                    w as usize,
                    h as usize,
                    radius,
                    true,
                );
            }
        }
        let exclusion = exclusion
            .and_then(|path| path.transform(self.transform))
            .and_then(|path| {
                let mut mask = Mask::new(w as u32, h as u32)?;
                mask.fill_path(
                    &path,
                    FillRule::Winding,
                    true,
                    Transform::from_translate(-x as f32, -y as f32),
                );
                Some(mask)
            });
        for row in visible.y.floor() as i32..visible.bottom().ceil() as i32 {
            for col in visible.x.floor() as i32..visible.right().ceil() as i32 {
                let index = ((row - y) * w + col - x) as usize;
                let coverage = (mask.data()[index] as u32
                    * (255 - exclusion.as_ref().map_or(0, |mask| mask.data()[index]) as u32)
                    / 255) as u8;
                self.blend(col, row, shadow.color, coverage);
            }
        }
    }
}
fn blur(input: &[u8], output: &mut [u8], w: usize, h: usize, radius: usize, vertical: bool) {
    let (lines, length, step) = if vertical { (w, h, w) } else { (h, w, 1) };
    for line in 0..lines {
        let start = if vertical { line } else { line * w };
        let mut sum = 0u32;
        for i in 0..radius.min(length) {
            sum += input[start + i * step] as u32;
        }
        for i in 0..length {
            if i + radius < length {
                sum += input[start + (i + radius) * step] as u32;
            }
            if i > radius {
                sum -= input[start + (i - radius - 1) * step] as u32;
            }
            output[start + i * step] = ((sum + radius as u32) / (2 * radius + 1) as u32) as u8;
        }
    }
}
