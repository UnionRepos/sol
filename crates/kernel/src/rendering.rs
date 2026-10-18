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
        self.animation_timeline.time = self.animation_time.unwrap_or_else(|| {
            if self.animation_initialized {
                self.animation_epoch.elapsed().as_secs_f64()
            } else {
                0.0
            }
        });
        sol_css::containers::set_boxes(self.container_boxes.clone());
        self.styles = sol_css::style::style_document_with_timeline(
            &self.doc.borrow(),
            &sheets,
            st,
            &mut self.animation_timeline,
        );
        self.animation_initialized = true;
    }

    pub fn refresh_rendering(&mut self) {
        let override_time = self.animation_time;
        // A single sampled time keeps container feedback about size alone; an
        // advancing animation could otherwise prevent an otherwise stable layout
        // from converging and make screenshots depend on refresh duration.
        for _ in 0..8 {
            self.restyle();
            self.animation_time = Some(self.animation_timeline.time);
            self.load_backgrounds();
            let previous = self.container_boxes.clone();
            self.relayout();
            if previous == self.container_boxes {
                break;
            }
        }
        self.animation_time = override_time;
    }

    pub fn sample_animations(&mut self, seconds: f64) {
        if !seconds.is_finite() || seconds < 0.0 {
            return;
        }
        self.animation_time = Some(seconds);
        self.refresh_rendering();
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
        self.container_boxes.clear();
        let mut boxes = Vec::new();
        out.root.walk(&mut boxes);
        for box_ in boxes {
            let Some(node) = box_
                .node
                .filter(|node| self.doc.borrow().node(*node).is_element())
            else {
                continue;
            };
            let Some(style) = &box_.style else { continue };
            let kind = style.ident("container-type").unwrap_or_default();
            if matches!(kind.as_str(), "size" | "inline-size") {
                let content = box_.content_rect();
                let names = style
                    .get("container-name")
                    .into_iter()
                    .flatten()
                    .filter_map(|v| {
                        if let sol_css::Value::Ident(name) = v {
                            Some(name.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                self.container_boxes.insert(
                    node,
                    sol_css::containers::Size {
                        width: content.w as f64,
                        height: content.h as f64,
                        names,
                        block: kind == "size",
                    },
                );
            }
        }
        self.layout = Some(out);
    }

    /// Render to a canvas (full page, top-aligned viewport).
    pub fn render(&self) -> Canvas {
        let (vw, vh) = (self.js.rt.viewport.0, self.js.rt.viewport.1);
        sol_css::style::viewport::WIDTH.set(vw as f64);
        sol_css::style::viewport::HEIGHT.set(vh as f64);
        sol_css::style::viewport::ROOT_FONT_SIZE.set(
            self.doc
                .borrow()
                .document_element()
                .and_then(|node| self.styles.get(&node))
                .and_then(|style| style.length_px("font-size", 16.0, 16.0))
                .unwrap_or(16.0),
        );
        let mut c = Canvas::new(vw, vh);
        c.clear(sol_paint::Rgba(255, 255, 255, 255));
        let fallback = self.fonts.face("sans-serif", 16.0, false, false);
        if let Some(o) = &self.layout {
            sol_paint::render_with_resources(
                &self.dl,
                &o.faces,
                &self.images,
                &self.background_images,
                &fallback,
                &mut c,
            )
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
        self.dl.hit(x, y)
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
