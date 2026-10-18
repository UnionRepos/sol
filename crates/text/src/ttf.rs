//! Validated TrueType, OpenType/CFF, and font collection access.

use owned_ttf_parser::{AsFaceRef, GlyphId, OutlineBuilder, OwnedFace};

pub struct Font {
    face: OwnedFace,
    pub units_per_em: u16,
    pub ascent: i16,
    pub descent: i16,
    pub line_gap: i16,
    pub family: String,
    pub style: String,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Debug, Clone)]
pub struct Glyph {
    pub contours: Vec<Vec<Pt>>,
    pub x_min: i16,
    pub y_min: i16,
    pub x_max: i16,
    pub y_max: i16,
    /// Exact outline, including cubic curves in OpenType/CFF fonts.
    pub path: Option<tiny_skia::Path>,
}

#[derive(Debug, Clone, Copy)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
    pub on: bool,
}

impl Font {
    pub(crate) fn shaper(&self) -> rustybuzz::Face<'_> {
        rustybuzz::Face::from_face(self.face.as_face_ref().clone())
    }

    pub fn load(data: Vec<u8>) -> Result<Self, String> {
        Self::load_index(data, 0)
    }

    pub fn load_index(data: Vec<u8>, index: u32) -> Result<Self, String> {
        let face = OwnedFace::from_vec(data, index).map_err(|e| format!("invalid font: {e:?}"))?;
        let parsed = face.as_face_ref();
        let name = |id| {
            parsed
                .names()
                .into_iter()
                .filter(|name| name.name_id == id)
                .find_map(|name| name.to_string())
                .unwrap_or_default()
        };
        Ok(Self {
            units_per_em: parsed.units_per_em(),
            ascent: parsed.ascender(),
            descent: parsed.descender(),
            line_gap: parsed.line_gap(),
            family: name(owned_ttf_parser::name_id::FAMILY),
            style: name(owned_ttf_parser::name_id::SUBFAMILY),
            bold: parsed.is_bold(),
            italic: parsed.is_italic(),
            face,
        })
    }

    pub fn glyph_index(&self, c: char) -> u16 {
        self.face.as_face_ref().glyph_index(c).map_or(0, |id| id.0)
    }

    pub fn advance(&self, gid: u16) -> u16 {
        self.face
            .as_face_ref()
            .glyph_hor_advance(GlyphId(gid))
            .unwrap_or(0)
    }

    pub fn kerning(&self, left: u16, right: u16) -> i16 {
        let Some(kern) = self.face.as_face_ref().tables().kern else {
            return 0;
        };
        kern.subtables
            .into_iter()
            .filter(|table| table.horizontal && !table.variable && !table.has_cross_stream)
            .filter_map(|table| table.glyphs_kerning(GlyphId(left), GlyphId(right)))
            .fold(0i16, i16::saturating_add)
    }

    pub fn glyph(&self, gid: u16) -> Glyph {
        let mut outline = Outline::default();
        let bounds = self
            .face
            .as_face_ref()
            .outline_glyph(GlyphId(gid), &mut outline);
        let bounds = bounds.unwrap_or(owned_ttf_parser::Rect {
            x_min: 0,
            y_min: 0,
            x_max: 0,
            y_max: 0,
        });
        Glyph {
            contours: outline.contours,
            x_min: bounds.x_min,
            y_min: bounds.y_min,
            x_max: bounds.x_max,
            y_max: bounds.y_max,
            path: outline.path.finish(),
        }
    }
}

#[derive(Default)]
struct Outline {
    path: tiny_skia::PathBuilder,
    contours: Vec<Vec<Pt>>,
}

impl Outline {
    fn point(&mut self, x: f32, y: f32, on: bool) {
        if let Some(contour) = self.contours.last_mut() {
            contour.push(Pt { x, y, on });
        }
    }
}

impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to(x, y);
        self.contours.push(vec![Pt { x, y, on: true }]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(x, y);
        self.point(x, y, true);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.path.quad_to(x1, y1, x, y);
        self.point(x1, y1, false);
        self.point(x, y, true);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.path.cubic_to(x1, y1, x2, y2, x, y);
        self.point(x1, y1, false);
        self.point(x2, y2, false);
        self.point(x, y, true);
    }
    fn close(&mut self) {
        self.path.close();
    }
}
