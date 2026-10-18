//! sol_layout: layout box tree construction and reflow.
//!
//! Implements block flow (with margin collapsing), inline formatting
//! (line breaking, vertical-align, floats), tables (auto layout),
//! flexbox (row/column, wrap, justify/align), replaced elements,
//! and static/relative/absolute/fixed positioning.

pub mod block;
pub mod flex;
pub mod table;
pub mod tree;

use sol_css::Style;
use sol_dom::NodeId;
use sol_text::Face;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    pub fn horiz(&self) -> f32 {
        self.left + self.right
    }
    pub fn vert(&self) -> f32 {
        self.top + self.bottom
    }
}

#[derive(Debug, Clone)]
pub struct LBox {
    pub kind: Kind,
    pub node: Option<NodeId>,
    pub style: Option<Rc<Style>>,
    /// border box rect (absolute coords after final position fix)
    pub rect: Rect,
    pub margin: Edges,
    pub padding: Edges,
    pub border: Edges,
    pub kids: Vec<LBox>,
    /// text content for Kind::Text
    pub text: Option<TextRun>,
    /// distance from rect.y to baseline (text/replaced items in a line)
    pub baseline: f32,
    /// for floats: which side
    pub float: sol_css::Float,
    pub clear: sol_css::Clear,
}

#[derive(Debug, Clone)]
pub struct TextRun {
    pub text: String,
    pub face_idx: usize, // index into FaceSet
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Root,
    Block,
    Inline,
    InlineBlock,
    /// anonymous block wrapping an inline run
    AnonymousBlock,
    /// line box (inside an IFC)
    Line,
    Text,
    /// replaced element (img, input, select, canvas, iframe…)
    Replaced,
    Table,
    TableRow,
    TableCell,
    TableCaption,
    Marker,
}

/// Faces referenced by text runs — layout fills, paint resolves.
pub struct FaceSet {
    pub faces: Vec<Face>,
}

impl FaceSet {
    // Paint resolves text runs by face index rather than repeating family
    // selection. Preserving font identity, size, and direction here keeps the
    // glyphs painted later consistent with the metrics used for line breaking.
    pub fn push(&mut self, f: Face) -> usize {
        if let Some(i) = self.faces.iter().position(|x| {
            x.px == f.px && x.direction == f.direction && Rc::ptr_eq(&x.font, &f.font)
        }) {
            return i;
        }
        self.faces.push(f);
        self.faces.len() - 1
    }
    pub fn get(&self, i: usize) -> &Face {
        &self.faces[i]
    }
}

/// Input bundle for layout.
pub struct LayoutInput<'a> {
    pub styles: &'a HashMap<NodeId, Rc<Style>>,
    pub fonts: &'a sol_text::Fonts,
    pub viewport_w: f32,
    pub viewport_h: f32,
    /// image sizes discovered by kernel (natural w,h)
    pub image_sizes: &'a HashMap<NodeId, (u32, u32)>,
}

pub struct Out {
    pub root: LBox,
    pub faces: FaceSet,
}

/// Style lookup helper.
pub fn style_of<'a>(input: &'a LayoutInput, n: NodeId) -> Option<&'a Rc<Style>> {
    input.styles.get(&n)
}

/// Full layout: build the box tree then reflow into absolute rects.
pub fn layout(doc: &sol_dom::Document, input: &LayoutInput) -> Out {
    let mut faces = FaceSet { faces: Vec::new() };
    let mut root = {
        let mut ctx = tree::Ctx {
            doc,
            input,
            faces: &mut faces,
        };
        let mut r = tree::build(&mut ctx);
        block::layout_document(&mut ctx, &mut r, input.viewport_w, input.viewport_h);
        r
    };
    let _ = &mut root;
    Out { root, faces }
}

impl LBox {
    pub fn new(kind: Kind) -> LBox {
        LBox {
            kind,
            node: None,
            style: None,
            rect: Rect::default(),
            margin: Edges::default(),
            padding: Edges::default(),
            border: Edges::default(),
            kids: Vec::new(),
            text: None,
            baseline: 0.0,
            float: sol_css::Float::None,
            clear: sol_css::Clear::None,
        }
    }
    pub fn content_rect(&self) -> Rect {
        Rect {
            x: self.rect.x + self.border.left + self.padding.left,
            y: self.rect.y + self.border.top + self.padding.top,
            w: self.rect.w - self.border.horiz() - self.padding.horiz(),
            h: self.rect.h - self.border.vert() - self.padding.vert(),
        }
    }
    pub fn padding_rect(&self) -> Rect {
        Rect {
            x: self.rect.x + self.border.left,
            y: self.rect.y + self.border.top,
            w: self.rect.w - self.border.horiz(),
            h: self.rect.h - self.border.vert(),
        }
    }
    pub fn margin_box_w(&self) -> f32 {
        self.rect.w + self.margin.horiz()
    }

    /// Iterate boxes in paint order (roughly DFS).
    pub fn walk<'a>(&'a self, out: &mut Vec<&'a LBox>) {
        out.push(self);
        for k in &self.kids {
            k.walk(out);
        }
    }
}
