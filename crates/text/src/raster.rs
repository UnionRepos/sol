//! Antialiased quadratic and cubic glyph rasterization with tiny-skia.

use crate::ttf::Glyph;
use tiny_skia::{FillRule, Paint, Pixmap, Transform};

#[derive(Clone, Default)]
pub struct Bitmap {
    pub w: u32,
    pub h: u32,
    /// coverage 0..255, row-major, top-down
    pub buf: Vec<u8>,
    pub left: i32,
    pub top: i32,
}

/// Rasterize an outline, bounding allocation even for hostile font sizes.
pub fn rasterize(g: &Glyph, scale: f32) -> Bitmap {
    let Some(path) = &g.path else {
        return Bitmap::default();
    };
    if !scale.is_finite() || scale <= 0.0 {
        return Bitmap::default();
    }
    let left = (g.x_min as f32 * scale).floor();
    let right = (g.x_max as f32 * scale).ceil();
    let bottom = (g.y_min as f32 * scale).floor();
    let top = (g.y_max as f32 * scale).ceil();
    let w = right - left;
    let h = top - bottom;
    if !(1.0..=2048.0).contains(&w) || !(1.0..=2048.0).contains(&h) {
        return Bitmap::default();
    }
    let Some(mut pixmap) = Pixmap::new(w as u32, h as u32) else {
        return Bitmap::default();
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    pixmap.fill_path(
        path,
        &paint,
        FillRule::Winding,
        Transform::from_row(scale, 0.0, 0.0, -scale, -left, top),
        None,
    );
    Bitmap {
        w: pixmap.width(),
        h: pixmap.height(),
        buf: pixmap.pixels().iter().map(|pixel| pixel.alpha()).collect(),
        left: left as i32,
        top: top as i32,
    }
}
