//! sol_dom: DOM core — a generational arena tree with stable `NodeId`s.
//!
//! Design notes:
//! - Nodes live in a flat `Arena<Node>`; parent/children are `GKey`s, so
//!   there are no Rc cycles and detach is O(1) at the slot level.
//! - Mutation helpers (`append`, `insert_before`, `remove`) maintain
//!   child lists and keep an `id` index live.
//! - Dirty marking supports incremental restyle/relayout: a mutation
//!   flags `DIRTY_STYLE` on the node and `DIRTY_STYLE_DESC` on ancestors
//!   so the style pass can skip clean subtrees.

use sol_util::{Arena, GKey};
use std::collections::HashMap;

pub type NodeId = GKey<Node>;

pub const DIRTY_STYLE: u8 = 1; // this node (+subtree) needs restyle
pub const DIRTY_STYLE_DESC: u8 = 2; // some descendant needs restyle
pub const DIRTY_LAYOUT: u8 = 4; // geometry needs recomputation
pub const NODE_HIDDEN: u8 = 8; // display:none style hint? (unused)

#[derive(Debug)]
pub enum NodeData {
    Document,
    DocumentFragment,
    Doctype {
        name: String,
        public_id: String,
        system_id: String,
    },
    Element(ElementData),
    Text(String),
    Comment(String),
}

#[derive(Debug)]
pub struct ElementData {
    /// lowercase tag name
    pub tag: Box<str>,
    /// insertion-ordered attributes; names lowercase
    pub attrs: Vec<(Box<str>, String)>,
    pub namespace: Namespace,
    /// per-element event listener handles (js registry ids)
    pub listeners: Vec<(Box<str>, u64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    Html,
    Svg,
    MathMl,
}

impl ElementData {
    pub fn new(tag: &str) -> ElementData {
        ElementData {
            tag: tag.to_ascii_lowercase().into_boxed_str(),
            attrs: Vec::new(),
            namespace: Namespace::Html,
            listeners: Vec::new(),
        }
    }
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k.as_ref() == name)
            .map(|(_, v)| v.as_str())
    }
    pub fn has_attr(&self, name: &str) -> bool {
        self.attr(name).is_some()
    }
    pub fn set_attr(&mut self, name: &str, value: &str) {
        if let Some(p) = self.attrs.iter_mut().find(|(k, _)| k.as_ref() == name) {
            p.1 = value.to_string();
        } else {
            self.attrs.push((name.into(), value.to_string()));
        }
    }
    pub fn remove_attr(&mut self, name: &str) {
        self.attrs.retain(|(k, _)| k.as_ref() != name);
    }
    pub fn has_class(&self, cls: &str) -> bool {
        self.attr("class")
            .map(|c| c.split_ascii_whitespace().any(|x| x == cls))
            .unwrap_or(false)
    }
    pub fn classes(&self) -> Vec<&str> {
        self.attr("class")
            .map(|c| c.split_ascii_whitespace().collect())
            .unwrap_or_default()
    }
    pub fn id(&self) -> Option<&str> {
        self.attr("id")
    }
}

#[derive(Debug)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub data: NodeData,
    pub flags: u8,
}

impl Node {
    pub fn is_element(&self) -> bool {
        matches!(self.data, NodeData::Element(_))
    }
    pub fn is_text(&self) -> bool {
        matches!(self.data, NodeData::Text(_))
    }
    pub fn element(&self) -> Option<&ElementData> {
        match &self.data {
            NodeData::Element(e) => Some(e),
            _ => None,
        }
    }
    pub fn element_mut(&mut self) -> Option<&mut ElementData> {
        match &mut self.data {
            NodeData::Element(e) => Some(e),
            _ => None,
        }
    }
    /// DOM nodeType number.
    pub fn node_type(&self) -> u8 {
        match self.data {
            NodeData::Document => 9,
            NodeData::DocumentFragment => 11,
            NodeData::Element(_) => 1,
            NodeData::Text(_) => 3,
            NodeData::Comment(_) => 8,
            NodeData::Doctype { .. } => 10,
        }
    }
    pub fn node_name(&self) -> String {
        match &self.data {
            NodeData::Document => "#document".into(),
            NodeData::DocumentFragment => "#document-fragment".into(),
            NodeData::Element(e) => e.tag.to_ascii_uppercase(),
            NodeData::Text(_) => "#text".into(),
            NodeData::Comment(_) => "#comment".into(),
            NodeData::Doctype { name, .. } => name.clone(),
        }
    }
}

pub struct Document {
    pub arena: Arena<Node>,
    pub root: NodeId,
    /// maps id attr → node; kept consistent by set_attribute/remove paths
    ids: HashMap<Box<str>, NodeId>,
    pub quirks_mode: bool,
    /// document URL for resolving resources
    pub base_url: String,
    /// <base href> override if present
    pub base_href: Option<String>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    pub fn new() -> Document {
        let mut arena = Arena::new();
        let root = arena.insert(Node {
            parent: None,
            children: vec![],
            data: NodeData::Document,
            flags: DIRTY_STYLE | DIRTY_LAYOUT,
        });
        Document {
            arena,
            root,
            ids: HashMap::new(),
            quirks_mode: false,
            base_url: String::new(),
            base_href: None,
        }
    }

    pub fn node(&self, id: NodeId) -> &Node {
        self.arena.get(id).expect("dead node")
    }
    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        self.arena.get_mut(id).expect("dead node")
    }
    pub fn alive(&self, id: NodeId) -> bool {
        self.arena.contains(id)
    }

    pub fn create_element(&mut self, tag: &str) -> NodeId {
        self.arena.insert(Node {
            parent: None,
            children: vec![],
            data: NodeData::Element(ElementData::new(tag)),
            flags: DIRTY_STYLE | DIRTY_LAYOUT,
        })
    }
    pub fn create_text(&mut self, text: &str) -> NodeId {
        self.arena.insert(Node {
            parent: None,
            children: vec![],
            data: NodeData::Text(text.to_string()),
            flags: DIRTY_LAYOUT,
        })
    }
    pub fn create_comment(&mut self, text: &str) -> NodeId {
        self.arena.insert(Node {
            parent: None,
            children: vec![],
            data: NodeData::Comment(text.to_string()),
            flags: 0,
        })
    }
    pub fn create_doctype(&mut self, name: &str, public_id: &str, system_id: &str) -> NodeId {
        self.arena.insert(Node {
            parent: None,
            children: vec![],
            data: NodeData::Doctype {
                name: name.to_string(),
                public_id: public_id.to_string(),
                system_id: system_id.to_string(),
            },
            flags: 0,
        })
    }
    pub fn create_fragment(&mut self) -> NodeId {
        self.arena.insert(Node {
            parent: None,
            children: vec![],
            data: NodeData::DocumentFragment,
            flags: DIRTY_STYLE,
        })
    }

    /// Append `child` to `parent`, detaching from any previous parent.
    pub fn append(&mut self, parent: NodeId, child: NodeId) {
        self.detach(child);
        self.node_mut(child).parent = Some(parent);
        self.node_mut(parent).children.push(child);
        self.reindex_subtree(child);
        self.mark_dirty(parent);
    }

    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, before: NodeId) {
        self.detach(child);
        self.node_mut(child).parent = Some(parent);
        let kids = &mut self.node_mut(parent).children;
        if let Some(i) = kids.iter().position(|&k| k == before) {
            kids.insert(i, child);
        } else {
            kids.push(child);
        }
        self.reindex_subtree(child);
        self.mark_dirty(parent);
    }

    /// Detach from parent without deleting the node.
    pub fn detach(&mut self, node: NodeId) {
        if let Some(p) = self.node(node).parent {
            let kids = &mut self.node_mut(p).children;
            kids.retain(|&k| k != node);
            self.node_mut(node).parent = None;
            self.mark_dirty(p);
        }
    }

    /// Remove node and its whole subtree from the document.
    pub fn remove(&mut self, node: NodeId) {
        self.detach(node);
        let kids = self.node(node).children.clone();
        for c in kids {
            self.remove(c);
        }
        if let NodeData::Element(e) = &self.node(node).data {
            if let Some(id) = e.id() {
                let id = id.to_string();
                self.ids.remove(id.as_str());
            }
        }
        self.arena.remove(node);
    }

    fn reindex_subtree(&mut self, root: NodeId) {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let (idval, kids): (Option<String>, Vec<NodeId>) = {
                let node = self.node(id);
                let idval = match &node.data {
                    NodeData::Element(e) => e.id().map(|s| s.to_string()),
                    _ => None,
                };
                (idval, node.children.clone())
            };
            if let Some(v) = idval {
                self.ids.insert(v.into_boxed_str(), id);
            }
            stack.extend_from_slice(&kids);
        }
    }

    /// Flag style+layout dirty from `node` down to its leaves.
    pub fn mark_dirty(&mut self, node: NodeId) {
        if !self.alive(node) {
            return;
        }
        self.mark_style_subtree(node);
        // A parent's geometry depends on descendant content. Keeping invalidation
        // visible along the ancestor chain lets a consumer distinguish a clean
        // ancestor from one whose subtree changed.
        let mut p = self.node(node).parent;
        while let Some(id) = p {
            let n = self.node_mut(id);
            if n.flags & DIRTY_STYLE_DESC != 0 && n.flags & DIRTY_LAYOUT != 0 {
                break;
            }
            n.flags |= DIRTY_STYLE_DESC | DIRTY_LAYOUT;
            p = n.parent;
        }
    }

    /// True if any node is flagged dirty; clears all dirty flags.
    /// Cheap aggregate for "did the DOM change since last check".
    pub fn take_dirty(&mut self) -> bool {
        let mut dirty = false;
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            let n = self.node_mut(id);
            if n.flags & (DIRTY_STYLE | DIRTY_LAYOUT | DIRTY_STYLE_DESC) != 0 {
                dirty = true;
                n.flags &= !(DIRTY_STYLE | DIRTY_LAYOUT | DIRTY_STYLE_DESC);
            }
            stack.extend_from_slice(&n.children);
        }
        dirty
    }

    /// Set DIRTY_STYLE on node and all descendants.
    pub fn mark_style_subtree(&mut self, node: NodeId) {
        if !self.alive(node) {
            return;
        }
        let mut stack = vec![node];
        while let Some(id) = stack.pop() {
            let n = self.node_mut(id);
            n.flags |= DIRTY_STYLE | DIRTY_LAYOUT;
            stack.extend_from_slice(&n.children);
        }
    }

    // ---------------- traversal ----------------

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.node(id).children
    }
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).parent
    }
    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        let p = self.parent(id)?;
        let kids = &self.node(p).children;
        let i = kids.iter().position(|&k| k == id)?;
        kids.get(i + 1).copied()
    }
    pub fn prev_sibling(&self, id: NodeId) -> Option<NodeId> {
        let p = self.parent(id)?;
        let kids = &self.node(p).children;
        let i = kids.iter().position(|&k| k == id)?;
        i.checked_sub(1).map(|j| kids[j])
    }
    pub fn ancestors(&self, id: NodeId) -> Ancestors<'_> {
        Ancestors {
            doc: self,
            cur: self.parent(id),
        }
    }
    pub fn descendants(&self, root: NodeId) -> Descendants<'_> {
        Descendants {
            doc: self,
            stack: vec![root],
        }
    }
    /// First element child of a node.
    pub fn first_element_child(&self, id: NodeId) -> Option<NodeId> {
        self.node(id)
            .children
            .iter()
            .copied()
            .find(|&c| self.node(c).is_element())
    }
    /// Element children in order.
    pub fn element_children(&self, id: NodeId) -> Vec<NodeId> {
        self.node(id)
            .children
            .iter()
            .copied()
            .filter(|&c| self.node(c).is_element())
            .collect()
    }

    // ---------------- queries ----------------

    pub fn document_element(&self) -> Option<NodeId> {
        self.node(self.root)
            .children
            .iter()
            .copied()
            .find(|&c| self.node(c).is_element())
    }
    pub fn head(&self) -> Option<NodeId> {
        let de = self.document_element()?;
        self.element_children(de)
            .into_iter()
            .find(|&c| self.tag(c) == "head")
    }
    pub fn body(&self) -> Option<NodeId> {
        let de = self.document_element()?;
        self.element_children(de)
            .into_iter()
            .find(|&c| matches!(self.tag(c), "body" | "frameset"))
    }
    pub fn title(&self) -> Option<String> {
        let head = self.head()?;
        let t = self
            .element_children(head)
            .into_iter()
            .find(|&c| self.tag(c) == "title")?;
        Some(self.text_content(t))
    }

    pub fn tag(&self, id: NodeId) -> &str {
        match self.node(id).element() {
            Some(e) => &e.tag,
            None => "",
        }
    }
    pub fn is_tag(&self, id: NodeId, t: &str) -> bool {
        self.tag(id) == t
    }
    pub fn attr(&self, id: NodeId, name: &str) -> Option<&str> {
        self.node(id).element().and_then(|e| e.attr(name))
    }

    pub fn get_element_by_id(&self, idv: &str) -> Option<NodeId> {
        self.ids.get(idv).copied().filter(|&n| self.alive(n))
    }
    pub fn get_elements_by_tag_name(&self, tag: &str) -> Vec<NodeId> {
        let tag_l = tag.to_ascii_lowercase();
        self.descendants(self.root)
            .filter(|&n| self.node(n).is_element() && (tag_l == "*" || self.tag(n) == tag_l))
            .collect()
    }
    pub fn get_elements_by_class_name(&self, cls: &str) -> Vec<NodeId> {
        self.descendants(self.root)
            .filter(|&n| {
                self.node(n)
                    .element()
                    .map(|e| e.has_class(cls))
                    .unwrap_or(false)
            })
            .collect()
    }

    // Mutations must keep selector inputs, the ID index, and invalidation in
    // agreement. Callers bypassing this entry point through node_mut need to
    // uphold those same obligations themselves.
    pub fn set_attribute(&mut self, id: NodeId, name: &str, value: &str) {
        let name_l = name.to_ascii_lowercase();
        if let Some(e) = self.node_mut(id).element_mut() {
            e.set_attr(&name_l, value);
        } else {
            return;
        }
        if name_l == "id" {
            self.reindex_subtree(id);
        }
        self.mark_dirty(id);
    }
    pub fn remove_attribute(&mut self, id: NodeId, name: &str) {
        let name_l = name.to_ascii_lowercase();
        if let Some(e) = self.node_mut(id).element_mut() {
            e.remove_attr(&name_l);
        }
        if name_l == "id" {
            // stale ids point at a live node still — rebuild lazily
            self.ids.retain(|_, &mut v| v != id);
        }
        self.mark_dirty(id);
    }

    // ---------------- text ----------------

    pub fn text_content(&self, id: NodeId) -> String {
        let mut s = String::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            match &self.node(n).data {
                NodeData::Text(t) => s.push_str(t),
                NodeData::Element(e) if &*e.tag == "script" || &*e.tag == "style" => {}
                _ => {
                    let kids = &self.node(n).children;
                    for c in kids.iter().rev() {
                        stack.push(*c);
                    }
                }
            }
        }
        s
    }

    /// Replace all children with a single text node.
    pub fn set_text_content(&mut self, id: NodeId, text: &str) {
        let kids = self.node(id).children.clone();
        for c in kids {
            self.remove(c);
        }
        if !text.is_empty() {
            let t = self.create_text(text);
            self.append(id, t);
        }
    }

    // ---------------- serialization ----------------

    pub fn inner_html(&self, id: NodeId) -> String {
        let mut s = String::new();
        for &c in &self.node(id).children {
            self.serialize_node(c, &mut s);
        }
        s
    }
    pub fn outer_html(&self, id: NodeId) -> String {
        let mut s = String::new();
        self.serialize_node(id, &mut s);
        s
    }

    fn serialize_node(&self, id: NodeId, out: &mut String) {
        let node = self.node(id);
        match &node.data {
            NodeData::Document | NodeData::DocumentFragment => {
                for &c in &node.children {
                    self.serialize_node(c, out);
                }
            }
            NodeData::Doctype { name, .. } => {
                out.push_str("<!DOCTYPE ");
                out.push_str(name);
                out.push('>');
            }
            NodeData::Text(t) => out.push_str(&escape_text(t)),
            NodeData::Comment(c) => {
                out.push_str("<!--");
                out.push_str(c);
                out.push_str("-->");
            }
            NodeData::Element(e) => {
                out.push('<');
                out.push_str(&e.tag);
                for (k, v) in &e.attrs {
                    out.push(' ');
                    out.push_str(k);
                    out.push_str("=\"");
                    out.push_str(&escape_attr(v));
                    out.push('"');
                }
                out.push('>');
                const VOID: &[&str] = &[
                    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta",
                    "param", "source", "track", "wbr",
                ];
                if !VOID.contains(&e.tag.as_ref()) {
                    const RAW: &[&str] = &["script", "style", "textarea", "title"];
                    for &c in &node.children {
                        if RAW.contains(&e.tag.as_ref()) {
                            if let NodeData::Text(t) = &self.node(c).data {
                                out.push_str(t);
                                continue;
                            }
                            self.serialize_node(c, out);
                        } else {
                            self.serialize_node(c, out);
                        }
                    }
                    out.push_str("</");
                    out.push_str(&e.tag);
                    out.push('>');
                }
            }
        }
    }
}

pub fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
pub fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
}

pub struct Ancestors<'a> {
    doc: &'a Document,
    cur: Option<NodeId>,
}
impl<'a> Iterator for Ancestors<'a> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let c = self.cur?;
        self.cur = self.doc.node(c).parent;
        Some(c)
    }
}

/// Pre-order traversal including the root itself.
pub struct Descendants<'a> {
    doc: &'a Document,
    stack: Vec<NodeId>,
}
impl<'a> Iterator for Descendants<'a> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let n = self.stack.pop()?;
        let kids = &self.doc.node(n).children;
        for c in kids.iter().rev() {
            self.stack.push(*c);
        }
        Some(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_ops() {
        let mut d = Document::new();
        let div = d.create_element("DIV");
        let span = d.create_element("span");
        let txt = d.create_text("hello");
        d.append(d.root, div);
        d.append(div, span);
        d.append(span, txt);
        assert_eq!(d.tag(div), "div");
        assert_eq!(d.text_content(div), "hello");
        assert_eq!(d.inner_html(div), "<span>hello</span>");
        d.set_attribute(span, "id", "x");
        assert_eq!(d.get_element_by_id("x"), Some(span));
        d.remove(div);
        assert!(!d.alive(span));
        assert_eq!(d.get_element_by_id("x"), None);
    }

    #[test]
    fn dirty_flags() {
        let mut d = Document::new();
        let a = d.create_element("a");
        let b = d.create_element("b");
        d.append(d.root, a);
        d.append(a, b);
        d.node_mut(a).flags = 0;
        d.node_mut(d.root).flags = 0;
        d.node_mut(b).flags = 0;
        d.mark_dirty(b);
        assert!(d.node(b).flags & DIRTY_STYLE != 0);
        assert!(d.node(a).flags & DIRTY_STYLE_DESC != 0);
    }
}
