use super::{Face, Font};
use std::rc::Rc;
use unicode_bidi::{BidiInfo, Level};
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    #[default]
    Auto,
    Ltr,
    Rtl,
}
impl Direction {
    fn level(self) -> Option<Level> {
        match self {
            Self::Auto => None,
            Self::Ltr => Some(Level::ltr()),
            Self::Rtl => Some(Level::rtl()),
        }
    }
}

pub struct PositionedGlyph {
    pub id: u16,
    pub font: Rc<Font>,
    pub x: f32,
    /// Positive offsets move upwards from the baseline.
    pub y: f32,
    pub cluster: usize,
}
#[derive(Default)]
pub struct ShapedText {
    pub width: f32,
    pub glyphs: Vec<PositionedGlyph>,
}

struct Segment {
    start: usize,
    end: usize,
    font: Rc<Font>,
    script: Script,
}

impl Face {
    pub fn shape(&self, text: &str) -> ShapedText {
        let bidi = BidiInfo::new(text, self.direction.level());
        let mut output = ShapedText::default();
        for paragraph in &bidi.paragraphs {
            let (levels, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
            for run in runs {
                let rtl = levels[run.start].is_rtl();
                let mut segments: Vec<Segment> = Vec::new();
                let mut previous_script = Script::Common;
                // A combining sequence must choose fallback as a unit; selecting
                // a font per scalar can separate a mark from its base and lose
                // the shaping relationship that positions the mark correctly.
                for (offset, grapheme) in text[run.clone()].grapheme_indices(true) {
                    let start = run.start + offset;
                    let script = grapheme
                        .chars()
                        .map(|c| c.script())
                        .find(|s| {
                            !matches!(s, Script::Common | Script::Inherited | Script::Unknown)
                        })
                        .unwrap_or(previous_script);
                    let covers = |font: &Font| {
                        grapheme.chars().all(|c| {
                            c.is_control()
                                || matches!(c, '\u{200c}' | '\u{200d}' | '\u{fe00}'..='\u{fe0f}')
                                || font.glyph_index(c) != 0
                        })
                    };
                    let font = if covers(&self.font) {
                        self.font.clone()
                    } else {
                        self.fallbacks
                            .iter()
                            .find(|font| covers(font))
                            .cloned()
                            .unwrap_or_else(|| self.font.clone())
                    };
                    if let Some(last) = segments.last_mut().filter(|last| {
                        Rc::ptr_eq(&last.font, &font)
                            && (last.script == script || last.script == Script::Common)
                    }) {
                        last.end = start + grapheme.len();
                        if last.script == Script::Common {
                            last.script = script;
                        }
                    } else {
                        segments.push(Segment {
                            start,
                            end: start + grapheme.len(),
                            font,
                            script,
                        });
                    }
                    previous_script = script;
                }
                if rtl {
                    segments.reverse();
                }
                for segment in segments {
                    let mut buffer = rustybuzz::UnicodeBuffer::new();
                    buffer.push_str(&text[segment.start..segment.end]);
                    buffer.set_direction(if rtl {
                        rustybuzz::Direction::RightToLeft
                    } else {
                        rustybuzz::Direction::LeftToRight
                    });
                    let tag = rustybuzz::ttf_parser::Tag::from_bytes(
                        segment.script.short_name().as_bytes().try_into().unwrap(),
                    );
                    if let Some(script) = rustybuzz::Script::from_iso15924_tag(tag) {
                        buffer.set_script(script);
                    }
                    buffer.guess_segment_properties();
                    let shaped = rustybuzz::shape(&segment.font.shaper(), &[], buffer);
                    let scale = super::font_size(self.px) / segment.font.units_per_em as f32;
                    for (info, position) in
                        shaped.glyph_infos().iter().zip(shaped.glyph_positions())
                    {
                        output.glyphs.push(PositionedGlyph {
                            id: info.glyph_id as u16,
                            font: segment.font.clone(),
                            x: output.width + position.x_offset as f32 * scale,
                            y: position.y_offset as f32 * scale,
                            cluster: segment.start + info.cluster as usize,
                        });
                        output.width += position.x_advance as f32 * scale;
                    }
                }
            }
        }
        output
    }
}

/// Resolve visual ordering of the measured words/objects on one line.
/// Shaping retains each word's logical character order for GSUB/GPOS.
pub fn visual_order(words: &[&str], direction: Direction) -> Vec<usize> {
    let mut text = String::new();
    let mut offsets = Vec::with_capacity(words.len());
    for word in words {
        offsets.push(text.len());
        text.push_str(if word.is_empty() { "\u{fffc}" } else { word });
        text.push(' ');
    }
    let bidi = BidiInfo::new(&text, direction.level());
    let levels: Vec<_> = offsets.iter().map(|&offset| bidi.levels[offset]).collect();
    BidiInfo::reorder_visual(&levels)
}
