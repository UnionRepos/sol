//! sol_text: Validated font parsing, antialiased glyph rasterization, and font discovery.
//!
//! - `Font` = parsed .ttf
//! - `Face` = font at a px size, with measuring + rasterize APIs
//! - `Fonts` = discovered system fonts with family/weight/style fallback

pub mod raster;
mod shaping;
pub use shaping::{visual_order, Direction, PositionedGlyph, ShapedText};
pub mod ttf;

pub use raster::Bitmap;
pub use ttf::Font;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Generic {
    Serif,
    Sans,
    Mono,
}

#[derive(Clone)]
pub struct Face {
    pub font: Rc<Font>,
    pub px: f32,
    pub direction: Direction,
    fallbacks: Rc<Vec<Rc<Font>>>,
}

impl Face {
    pub fn new(font: Rc<Font>, px: f32) -> Self {
        Self {
            font,
            px: font_size(px),
            direction: Direction::Auto,
            fallbacks: Rc::new(Vec::new()),
        }
    }
    pub fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }
    pub fn with_fallbacks(mut self, fonts: Vec<Rc<Font>>) -> Self {
        self.fallbacks = Rc::new(fonts);
        self
    }

    pub fn scale(&self) -> f32 {
        font_size(self.px) / self.font.units_per_em as f32
    }
    pub fn ascent(&self) -> f32 {
        self.font.ascent as f32 * self.scale()
    }
    pub fn descent(&self) -> f32 {
        self.font.descent as f32 * self.scale()
    }
    pub fn line_height(&self) -> f32 {
        (self.font.ascent as f32 - self.font.descent as f32 + self.font.line_gap as f32)
            * self.scale()
    }
    pub fn glyph_id(&self, c: char) -> u16 {
        self.font.glyph_index(c)
    }
    pub fn advance(&self, gid: u16) -> f32 {
        self.font.advance(gid) as f32 * self.scale()
    }
    /// Measurement and painting share the same shaping and fallback path.
    pub fn measure(&self, text: &str) -> f32 {
        self.shape(text).width
    }
    /// Rasterize a single glyph for painting.
    pub fn rasterize(&self, gid: u16) -> Bitmap {
        let g = self.font.glyph(gid);
        raster::rasterize(&g, self.scale())
    }
    /// Compatibility view of the primary font's glyph positions. Use `shape`
    /// when painting to retain fallback fonts and vertical mark offsets.
    pub fn layout(&self, text: &str) -> (f32, Vec<(u16, f32)>) {
        let shaped = self.shape(text);
        (
            shaped.width,
            shaped.glyphs.iter().map(|g| (g.id, g.x)).collect(),
        )
    }
}

/// Discovered fonts with a small match system.
#[derive(Clone)]
pub struct Fonts {
    /// (family lower, generic, weight-ish flags) -> font
    entries: Vec<Entry>,
    face_cache: RefCell<HashMap<(usize, u32), Face>>,
    default: usize,
    fallbacks: Rc<Vec<Rc<Font>>>,
}

#[derive(Clone)]
struct Entry {
    family: String,
    #[allow(dead_code)]
    style: String,
    generic: Generic,
    bold: bool,
    italic: bool,
    font: Rc<Font>,
    custom: bool,
}

impl Fonts {
    /// Scan once per thread, with a bundled fallback when system fonts are absent.
    pub fn discover() -> Fonts {
        // Font data is shared through Rc within a rendering thread. A thread
        // local cache avoids rescanning for every Page without pretending those
        // handles can be transferred safely between threads.
        thread_local! {
            static SYSTEM: Fonts = Fonts::from_dirs(system_dirs());
        }
        SYSTEM.with(Clone::clone)
    }

    /// Discover TTF, OpenType/CFF, and TTC fonts under the supplied directories.
    /// Traversal is deterministic and does not revisit symlink cycles.
    pub fn from_dirs(dirs: impl IntoIterator<Item = PathBuf>) -> Fonts {
        let mut entries = Vec::new();
        let mut visited = HashSet::new();
        let mut loaded_bytes = include_bytes!("../assets/DejaVuSans.ttf").len();
        let mut stack: Vec<_> = dirs.into_iter().collect();
        stack.sort();
        while let Some(path) = stack.pop() {
            let Ok(path) = path.canonicalize() else {
                continue;
            };
            if !visited.insert(path.clone()) {
                continue;
            }
            if path.is_dir() {
                if let Ok(dir) = std::fs::read_dir(&path) {
                    let mut children: Vec<_> = dir.flatten().map(|entry| entry.path()).collect();
                    children.sort();
                    stack.extend(children.into_iter().rev());
                }
                continue;
            }
            let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
            if !["ttf", "otf", "ttc"]
                .iter()
                .any(|e| ext.eq_ignore_ascii_case(e))
            {
                continue;
            }
            let Ok(file) = std::fs::File::open(&path) else {
                continue;
            };
            let mut data = Vec::new();
            const MAX_FONT_BYTES: u64 = 32 * 1024 * 1024;
            if file
                .take(MAX_FONT_BYTES + 1)
                .read_to_end(&mut data)
                .is_err()
                || data.len() as u64 > MAX_FONT_BYTES
            {
                continue;
            }
            let count = owned_ttf_parser::fonts_in_collection(&data)
                .unwrap_or(1)
                .min(256);
            for index in 0..count {
                if data.len() > 64 * 1024 * 1024 - loaded_bytes {
                    break;
                }
                if let Ok(font) = Font::load_index(data.clone(), index) {
                    loaded_bytes += data.len();
                    entries.push(Entry::new(font));
                }
            }
        }
        if !entries
            .iter()
            .any(|entry: &Entry| entry.family == "DejaVu Sans")
        {
            entries.push(Entry::new(
                Font::load(include_bytes!("../assets/DejaVuSans.ttf").to_vec())
                    .expect("bundled DejaVu Sans is valid"),
            ));
        }
        // pick a sane default: prefer DejaVu Sans Book
        let default = entries
            .iter()
            .position(|e| e.family.contains("DejaVu Sans") && !e.bold && !e.italic)
            .or_else(|| entries.iter().position(|e| e.generic == Generic::Sans))
            .unwrap_or(0);
        let fallbacks = Rc::new(entries.iter().map(|entry| entry.font.clone()).collect());
        Fonts {
            fallbacks,
            entries,
            face_cache: RefCell::new(HashMap::new()),
            default,
        }
    }

    pub fn register(
        &mut self,
        family: &str,
        data: Vec<u8>,
        bold: bool,
        italic: bool,
    ) -> Result<(), String> {
        if data.len() > 32 * 1024 * 1024 {
            return Err("font exceeds 32 MiB".into());
        }
        let font = Rc::new(Font::load(data)?);
        // A page-specific alias must win over a discovered family with the same
        // name, while keeping the thread-local system catalog unchanged.
        self.entries.insert(
            0,
            Entry {
                family: family.into(),
                style: String::new(),
                generic: Generic::Sans,
                bold,
                italic,
                font,
                custom: true,
            },
        );
        self.default += 1;
        self.face_cache.borrow_mut().clear();
        Ok(())
    }

    /// Resolve family list + weight + italic to a Face.
    pub fn face(&self, families: &str, px: f32, bold: bool, italic: bool) -> Face {
        let px = font_size(px);
        let idx = self.resolve(families, bold, italic);
        let key = (idx, px.to_bits());
        if let Some(f) = self.face_cache.borrow().get(&key) {
            return f.clone();
        }
        let f = Face {
            font: self.entries[idx].font.clone(),
            px,
            direction: Direction::Auto,
            fallbacks: self.fallbacks.clone(),
        };
        let mut cache = self.face_cache.borrow_mut();
        if cache.len() >= 512 {
            cache.clear();
        }
        cache.insert(key, f.clone());
        f
    }

    fn resolve(&self, families: &str, bold: bool, italic: bool) -> usize {
        for fam in families.split(',') {
            let fam = fam
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_ascii_lowercase();
            let generic = match fam.as_str() {
                "serif" => Some(Generic::Serif),
                "sans-serif" | "sans" | "helvetica" | "arial" => Some(Generic::Sans),
                "monospace" | "mono" | "courier" | "courier new" | "consolas" => {
                    Some(Generic::Mono)
                }
                "" => continue,
                _ => None,
            };
            if let Some(g) = generic {
                if let Some(i) = self.pick(|e| !e.custom && e.generic == g, bold, italic) {
                    return i;
                }
            } else {
                if let Some(i) = self.pick(|e| e.family.eq_ignore_ascii_case(&fam), bold, italic) {
                    return i;
                }
            }
        }
        self.default
    }

    fn pick(&self, pred: impl Fn(&Entry) -> bool, bold: bool, italic: bool) -> Option<usize> {
        let mut best: Option<(usize, i32)> = None;
        for (i, e) in self.entries.iter().enumerate() {
            if !pred(e) {
                continue;
            }
            let mut score = 0;
            if e.bold == bold {
                score += 2;
            }
            if e.italic == italic {
                score += 1;
            }
            if !e.bold && !e.italic {
                score += 1;
            }
            if best.is_none_or(|(_, s)| score > s) {
                best = Some((i, score));
            }
        }
        best.map(|(i, _)| i)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn font_size(px: f32) -> f32 {
    if px.is_finite() {
        px.clamp(0.0, 512.0)
    } else {
        16.0
    }
}

fn system_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<_> = [
        "/usr/share/fonts",
        "/usr/local/share/fonts",
        "/Library/Fonts",
        "/System/Library/Fonts",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.extend([
            home.join(".fonts"),
            home.join(".local/share/fonts"),
            home.join("Library/Fonts"),
        ]);
    }
    if let Some(windows) = std::env::var_os("WINDIR") {
        dirs.push(PathBuf::from(windows).join("Fonts"));
    }
    dirs
}

impl Entry {
    fn new(font: Font) -> Self {
        let family = font.family.to_ascii_lowercase();
        let generic =
            if family.contains("mono") || family.contains("code") || family.contains("console") {
                Generic::Mono
            } else if family.contains("serif") && !family.contains("sans") {
                Generic::Serif
            } else {
                Generic::Sans
            };
        Self {
            family: font.family.clone(),
            style: font.style.clone(),
            generic,
            bold: font.bold,
            italic: font.italic,
            font: Rc::new(font),
            custom: false,
        }
    }
}
