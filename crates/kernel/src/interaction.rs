//! CSS pseudo-state and user input dispatch.
use crate::Page;
use sol_css::style::PseudoState;
use sol_dom::NodeId;

/// Interactive state for CSS pseudo-classes, fed by the surf frontend.
#[derive(Default)]
pub struct Interact {
    pub hover: Option<NodeId>,
    pub active: Option<NodeId>,
    pub focus: Option<NodeId>,
    pub visited: std::collections::HashSet<String>,
    pub target: Option<NodeId>,
}

impl PseudoState for Interact {
    fn is_hovered(&self, n: NodeId) -> bool {
        self.hover == Some(n)
    }
    fn is_active(&self, n: NodeId) -> bool {
        self.active == Some(n)
    }
    fn is_focused(&self, n: NodeId) -> bool {
        self.focus == Some(n)
    }
    fn is_visited(&self, href: &str) -> bool {
        self.visited.contains(href)
    }
    fn is_target(&self, n: NodeId) -> bool {
        self.target == Some(n)
    }
}

impl Page {
    /// Script handlers can mutate the document or request navigation, so their
    /// event sequence and resulting refresh must precede the anchor fallback.
    pub fn click(&mut self, x: f32, y: f32) -> bool {
        let Some(n) = self.hit(x, y) else {
            return false;
        };
        self.interact.active = Some(n);
        self.interact.focus = Some(n);
        self.js.fire_event(n, "mousedown");
        self.js.fire_event(n, "mouseup");
        self.js.fire_event(n, "click");
        // A hit may identify text or another descendant inside a link. The
        // ancestor walk preserves link activation for the whole clickable area.
        let mut nav = None;
        {
            let d = self.doc.borrow();
            let mut cur = Some(n);
            while let Some(c) = cur {
                if let Some(e) = d.node(c).element() {
                    if &*e.tag == "a" {
                        if let Some(h) = e.attr("href") {
                            nav = Some(h.to_string());
                        }
                        break;
                    }
                }
                cur = d.parent(c);
            }
        }
        self.settle();
        if let Some(h) = nav {
            if let Some(u) = self.resolve(&h) {
                let us = u.serialize();
                let cur_base = self
                    .url
                    .as_ref()
                    .map(|x| x.serialize().split('#').next().unwrap_or("").to_string());
                let new_base = us.split('#').next().unwrap_or("").to_string();
                if Some(&new_base) == cur_base.as_ref() && u.fragment.is_some() {
                    // Re-fetching a fragment-only destination would discard DOM
                    // changes even though the target belongs to this document.
                    let id = u.fragment.clone().unwrap();
                    let t = self.doc.borrow().get_element_by_id(&id);
                    if let Some(t) = t {
                        self.interact.target = Some(t);
                        self.dirty = true;
                        self.settle();
                        return true;
                    }
                }
                if Some(&new_base) != cur_base.as_ref() || u.fragment.is_none() {
                    self.interact.visited.insert(us.clone());
                    if let Err(e) = self.open(&us) {
                        self.log(&format!("nav {us}: {e}"));
                    }
                    return true;
                }
            }
        }
        self.dirty
    }

    /// Document listeners need notification so script-driven changes can join
    /// the same rendering refresh as the edited control value.
    pub fn set_input(&mut self, n: NodeId, v: &str) {
        {
            let mut d = self.doc.borrow_mut();
            d.set_attribute(n, "value", v);
        }
        // Handlers expect the edited value to be visible during notification.
        // The mutable DOM borrow must end first because those handlers can read
        // or change the same document through the runtime's shared handle.
        self.js.fire_event(n, "input");
        self.js.fire_event(n, "change");
        self.dirty = true;
        self.settle();
    }
}
