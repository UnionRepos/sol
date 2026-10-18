//! Style, layout, painting, and geometry queries using the same transforms.
use crate::Page;
use sol_css::parser::Stylesheet;
use sol_css::style::PseudoState;
use sol_css::NoState;
use sol_dom::NodeId;
use sol_img::Image;
use sol_layout::{layout, LBox, LayoutInput};
use sol_paint::{build_dl, Canvas, DisplayList};
use std::collections::HashMap;

impl Page {
    /// Recompute all computed styles (author sheets + UA).
    pub fn restyle(&mut self) {
        sol_css::style::viewport::WIDTH.set(self.js.rt.viewport.0 as f64);
        sol_css::style::viewport::HEIGHT.set(self.js.rt.viewport.1 as f64);
        let mut sheets: Vec<&Stylesheet> = vec![&self.ua];
        for s in &self.sheets {
            sheets.push(s);
        }
        let st: &dyn PseudoState = if self.interact.hover.is_some()
            || self.interact.active.is_some()
            || self.interact.focus.is_some()
            || self.interact.target.is_some()
            || !self.interact.visited.is_empty()
        {
            &self.interact as &dyn PseudoState
        } else {
            &NoState as &dyn PseudoState
        };
        self.styles = sol_css::style_document(&self.doc.borrow(), &sheets, st);
    }

    /// Rebuild layout + display list.
    pub fn relayout(&mut self) {
        let (vw, vh) = (self.js.rt.viewport.0 as f32, self.js.rt.viewport.1 as f32);
        let input = LayoutInput {
            styles: &self.styles,
            fonts: &self.fonts,
            viewport_w: vw,
            viewport_h: vh,
            image_sizes: &self.image_sizes,
        };
        let out = layout(&self.doc.borrow(), &input);
        self.dl = DisplayList::default();
        build_dl(&self.doc.borrow(), &out.root, &mut self.dl);
        // JavaScript geometry must agree with visible pixels after transforms.
        // Reusing the painting helpers prevents a translated element from being
        // drawn at one position but reported at its untransformed layout rect.
        {
            let mut m = self.rects.borrow_mut();
            m.clear();
            fn walk(
                b: &LBox,
                m: &mut HashMap<NodeId, sol_layout::Rect>,
                parent: sol_paint::Transform,
            ) {
                let transform = box_transform(b)
                    .map(|local| parent.pre_concat(local))
                    .unwrap_or(parent);
                if let Some(n) = b.node {
                    m.insert(n, sol_paint::transformed_rect(b.rect, transform));
                }
                for k in &b.kids {
                    walk(k, m, transform);
                }
            }
            walk(&out.root, &mut m, sol_paint::Transform::identity());
        }
        self.layout = Some(out);
    }

    /// Render to a canvas (full page, top-aligned viewport).
    pub fn render(&self) -> Canvas {
        let (vw, vh) = (self.js.rt.viewport.0, self.js.rt.viewport.1);
        let mut c = Canvas::new(vw, vh);
        c.clear(sol_paint::Rgba(255, 255, 255, 255));
        let fallback = self.fonts.face("sans-serif", 16.0, false, false);
        match &self.layout {
            Some(o) => sol_paint::render(&self.dl, &o.faces, &self.images, &fallback, &mut c),
            None => {}
        }
        c
    }

    /// Render + PNG-encode.
    pub fn render_png(&self) -> Vec<u8> {
        let c = self.render();
        let img = canvas_to_image(&c);
        sol_img::encode_png(&img)
    }

    /// Hit test: deepest box at (x,y) with a DOM node.
    pub fn hit(&self, x: f32, y: f32) -> Option<NodeId> {
        // Descendants may escape ancestor rects (abs-pos, overflow), so search
        // all kids; a box only claims the point if its own rect contains it.
        fn find(b: &LBox, x: f32, y: f32) -> Option<NodeId> {
            let (mut x, mut y) = (x, y);
            if let Some(matrix) = box_transform(b) {
                let inverse = matrix.invert()?;
                let mut point = sol_paint::Point::from_xy(x, y);
                inverse.map_point(&mut point);
                (x, y) = (point.x, point.y);
            }
            if let Some(style) = &b.style {
                let clips = style
                    .ident("overflow-x")
                    .or_else(|| style.ident("overflow"))
                    .is_some_and(|value| matches!(value.as_str(), "hidden" | "scroll" | "auto"));
                if clips && !sol_paint::contains_rounded(style, b.rect, x, y) {
                    return None;
                }
                if style
                    .ident("visibility")
                    .is_some_and(|value| value == "hidden" || value == "collapse")
                {
                    return None;
                }
            }
            // Later siblings are painted above earlier ones in this traversal.
            // Testing them first keeps overlapping links consistent with the
            // element the user can actually see at the pointer position.
            for k in b.kids.iter().rev() {
                if let Some(hit) = find(k, x, y) {
                    return Some(hit);
                }
            }
            if b.node.is_some()
                && b.rect.contains(x, y)
                && b.style
                    .as_ref()
                    .is_none_or(|style| sol_paint::contains_rounded(style, b.rect, x, y))
            {
                return b.node;
            }
            None
        }
        self.layout.as_ref().and_then(|o| find(&o.root, x, y))
    }
}

/// Convert paint canvas to an image for PNG encoding.
fn canvas_to_image(c: &Canvas) -> Image {
    let mut img = Image::new(c.w, c.h);
    for y in 0..c.h {
        for x in 0..c.w {
            let i = ((y * c.w + x) * 4) as usize;
            img.set(x, y, [c.px[i], c.px[i + 1], c.px[i + 2], c.px[i + 3]]);
        }
    }
    img
}

// Text and anonymous boxes can inherit their owner's style without establishing
// another transform. Applying that style again would translate or rotate the
// same content twice, so hit testing and DOM geometry follow paint's exclusions.
fn box_transform(b: &LBox) -> Option<sol_paint::Transform> {
    if matches!(
        b.kind,
        sol_layout::Kind::Text
            | sol_layout::Kind::Marker
            | sol_layout::Kind::Line
            | sol_layout::Kind::AnonymousBlock
            | sol_layout::Kind::Inline
    ) {
        return None;
    }
    b.style
        .as_ref()
        .and_then(|style| sol_paint::css_transform(style, b.rect))
}
