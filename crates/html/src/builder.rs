//! HTML5 tree construction — insertion-mode state machine.
//!
//! Implements the modes that cover real-world documents: Initial,
//! BeforeHtml, BeforeHead, InHead, AfterHead, InBody, Text (raw text),
//! the full InTable family (InTable/InCaption/InColumnGroup/
//! InTableBody/InRow/InCell), InSelect, AfterBody and AfterAfterBody.
//! Includes implied end tags, active-formatting-elements with the
//! adoption-agency algorithm, scope checks, and foster parenting for
//! mis-nested table content.

use crate::tokenizer::{RawKind, Token, Tokenizer};
use sol_dom::{Document, NodeData, NodeId};

const SPECIAL: &[&str] = &[
    "address",
    "applet",
    "area",
    "article",
    "aside",
    "base",
    "basefont",
    "bgsound",
    "blockquote",
    "body",
    "br",
    "button",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dir",
    "div",
    "dl",
    "dt",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "iframe",
    "img",
    "input",
    "keygen",
    "li",
    "link",
    "listing",
    "main",
    "marquee",
    "menu",
    "meta",
    "nav",
    "noembed",
    "noframes",
    "noscript",
    "object",
    "ol",
    "p",
    "param",
    "plaintext",
    "pre",
    "script",
    "section",
    "select",
    "source",
    "style",
    "summary",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
    "wbr",
    "xmp",
];

const FORMATTING: &[&str] = &[
    "a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong", "tt", "u",
];

const BUTTON_SCOPE: &[&str] = &[
    "applet", "caption", "html", "table", "td", "th", "marquee", "object", "template", "button",
];
const DEFAULT_SCOPE: &[&str] = &[
    "applet", "caption", "html", "table", "td", "th", "marquee", "object", "template",
];
const LIST_SCOPE: &[&str] = &[
    "applet", "caption", "html", "table", "td", "th", "marquee", "object", "template", "ol", "ul",
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    AfterHead,
    InBody,
    Text,
    InTable,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InSelect,
    AfterBody,
    AfterAfterBody,
    InFrameset,
}

/// A text char batch from the tokenizer is delivered as String tokens;
/// whitespace-only handling per insertion mode.
pub struct TreeBuilder<'a> {
    doc: &'a mut Document,
    open: Vec<NodeId>,
    afe: Vec<AfeItem>,
    mode: Mode,
    orig_mode: Mode,
    head: Option<NodeId>,
    form: Option<NodeId>,
    fragment: Option<NodeId>,
    fragment_ctx: String,
    /// text of the raw-text element currently being built
    text_node: Option<NodeId>,
    tokenizer: Tokenizer,
    frameset_ok: bool,
    foster_parenting: bool,
    seen_frameset: bool,
    /// buffered doctype quirks
    quirks: bool,
}

#[derive(Clone)]
enum AfeItem {
    Element(NodeId),
    Marker,
}

/// Keeping script handles separate lets the host choose execution timing after
/// tree construction, without coupling the parser to a JavaScript runtime.
pub struct Parsed {
    /// Retaining the nodes preserves source/type attributes for the host's later
    /// decision, and retaining their order preserves dependencies between scripts.
    pub scripts: Vec<NodeId>,
}

pub fn parse(input: &str, doc: &mut Document) -> Parsed {
    TreeBuilder::new(input, doc).run()
}

/// innerHTML fragment parsing — parses `input` as children of a context
/// element `ctx_tag` (e.g. "div", "table", "tbody").
// A fragment's insertion rules depend on its container: table content cannot
// be parsed as though it appeared in an ordinary body. Carrying the context
// avoids producing a different subtree when innerHTML is used instead of markup.
pub fn parse_fragment(input: &str, ctx_tag: &str, doc: &mut Document) -> Vec<NodeId> {
    let mut tb = TreeBuilder::new(input, doc);
    let f = tb.doc.create_fragment();
    tb.fragment = Some(f);
    tb.fragment_ctx = ctx_tag.to_ascii_lowercase();
    tb.run();
    take_fragment_children()
}

impl<'a> TreeBuilder<'a> {
    fn new(input: &str, doc: &'a mut Document) -> TreeBuilder<'a> {
        TreeBuilder {
            doc,
            open: Vec::new(),
            afe: Vec::new(),
            mode: Mode::Initial,
            orig_mode: Mode::Initial,
            head: None,
            form: None,
            fragment: None,
            fragment_ctx: String::new(),
            tokenizer: Tokenizer::new(input),
            frameset_ok: true,
            foster_parenting: false,
            seen_frameset: false,
            text_node: None,
            quirks: false,
        }
    }

    fn run(mut self) -> Parsed {
        let mut scripts = Vec::new();
        if self.fragment.is_some() {
            // set up synthetic context: html > (ctx or body)
            self.setup_fragment();
        }
        loop {
            let tok = self.tokenizer.next_token();
            let done = matches!(tok, Token::Eof);
            self.process(tok, &mut scripts);
            if done {
                break;
            }
        }
        if let Some(f) = self.fragment {
            // The spec's "children of the html element" — take the children
            // of the synthetic <html> placeholder inside the fragment.
            let html_root = self
                .doc
                .children(f)
                .iter()
                .copied()
                .find(|&c| self.doc.node(c).is_element());
            let kids = match html_root {
                Some(h) => self.doc.children(h).to_vec(),
                None => vec![],
            };
            for c in &kids {
                self.doc.detach(*c);
            }
            self.doc.remove(f);
            FRAG_OUT.with(|o| o.borrow_mut().extend(kids));
        }
        self.doc.quirks_mode = self.quirks;
        Parsed { scripts }
    }

    fn setup_fragment(&mut self) {
        // Per HTML5 fragment algorithm: create html root under fragment,
        // select insertion mode from context tag.
        let frag = self.fragment.unwrap();
        let html = self.doc.create_element("html");
        self.doc.append(frag, html);
        self.open.push(html);
        self.mode = match self.fragment_ctx.as_str() {
            "title" | "textarea" => {
                self.tokenizer
                    .enter_raw(RawKind::Rcdata, &self.fragment_ctx.clone());
                Mode::Text
            }
            "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript" => {
                self.tokenizer
                    .enter_raw(RawKind::Rawtext, &self.fragment_ctx.clone());
                Mode::Text
            }
            "script" => {
                self.tokenizer.enter_raw(RawKind::Script, "script");
                Mode::Text
            }
            "table" => Mode::InTable,
            "tbody" | "thead" | "tfoot" => Mode::InTableBody,
            "tr" => Mode::InRow,
            "td" | "th" => Mode::InCell,
            "colgroup" => Mode::InColumnGroup,
            "select" => Mode::InSelect,
            _ => Mode::InBody,
        };
        if self.mode == Mode::Text {
            // raw text goes under the synthetic html element
            self.text_node = self.open.last().copied();
        }
        self.head = None;
        self.orig_mode = self.mode;
    }

    // ------------- helpers -------------

    fn cur(&self) -> Option<NodeId> {
        self.open.last().copied()
    }
    fn cur_tag(&self) -> String {
        self.cur()
            .map(|n| self.doc.tag(n).to_string())
            .unwrap_or_default()
    }
    fn adjusted_insertion(&self) -> NodeId {
        self.cur().or(self.fragment).unwrap_or(self.doc.root)
    }

    fn insert_element(&mut self, tag: &str, attrs: &[(String, String)]) -> NodeId {
        let el = self.doc.create_element(tag);
        for (k, v) in attrs {
            if let Some(e) = self.doc.node_mut(el).element_mut() {
                e.set_attr(k, v);
            }
        }
        self.insert_at(el);
        self.open.push(el);
        el
    }

    fn insert_at(&mut self, el: NodeId) {
        if self.foster_parenting {
            if let Some((p, before)) = self.foster_location() {
                match before {
                    Some(b) => self.doc.insert_before(p, el, b),
                    None => self.doc.append(p, el),
                }
                return;
            }
        }
        let parent = self.adjusted_insertion();
        self.doc.append(parent, el);
    }

    /// Location for foster-parented content: before nearest table, into its
    /// parent, or previous sibling.
    fn foster_location(&self) -> Option<(NodeId, Option<NodeId>)> {
        for &n in self.open.iter().rev() {
            let t = self.doc.tag(n);
            if t == "table" {
                if let Some(p) = self.doc.parent(n) {
                    return Some((p, Some(n)));
                }
                // table has no parent: use element below it on the stack
                let idx = self.open.iter().position(|&x| x == n).unwrap();
                if idx > 0 {
                    return Some((self.open[idx - 1], None));
                }
                return None;
            }
        }
        None
    }

    fn insert_text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        if self.foster_parenting {
            if let Some((p, before)) = self.foster_location() {
                let t = self.doc.create_text(s);
                match before {
                    Some(b) => self.doc.insert_before(p, t, b),
                    None => self.doc.append(p, t),
                }
                return;
            }
        }
        let parent = self.adjusted_insertion();
        // merge with previous text sibling
        if let Some(&last) = self.doc.children(parent).last() {
            if let NodeData::Text(t) = &mut self.doc.node_mut(last).data {
                t.push_str(s);
                return;
            }
        }
        let t = self.doc.create_text(s);
        self.doc.append(parent, t);
    }

    fn pop(&mut self) -> Option<NodeId> {
        self.open.pop()
    }
    fn pop_until(&mut self, tags: &[&str]) {
        while let Some(n) = self.pop() {
            if tags.contains(&self.doc.tag(n)) {
                break;
            }
        }
    }
    fn has_in_scope(&self, scope: &[&str], tag: &str) -> bool {
        for &n in self.open.iter().rev() {
            let t = self.doc.tag(n);
            if t == tag {
                return true;
            }
            if scope.contains(&t) {
                return false;
            }
        }
        false
    }
    fn has_in_button_scope(&self, tag: &str) -> bool {
        self.has_in_scope(BUTTON_SCOPE, tag)
    }
    fn has_in_default_scope(&self, tag: &str) -> bool {
        self.has_in_scope(DEFAULT_SCOPE, tag)
    }
    fn has_in_list_scope(&self, tag: &str) -> bool {
        self.has_in_scope(LIST_SCOPE, tag)
    }
    fn has_in_table_scope(&self, tag: &str) -> bool {
        self.has_in_scope(&["html", "table", "template"], tag)
    }
    fn has_in_select_scope(&self, tag: &str) -> bool {
        for &n in self.open.iter().rev() {
            let t = self.doc.tag(n);
            if t == tag {
                return true;
            }
            if t != "optgroup" && t != "option" {
                return false;
            }
        }
        false
    }

    fn gen_implied_end(&mut self, except: &[&str]) {
        const IMPLIED: &[&str] = &[
            "dd", "dt", "li", "optgroup", "option", "p", "rb", "rp", "rt", "rtc",
        ];
        loop {
            let t = self.cur_tag();
            if IMPLIED.contains(&t.as_str()) && !except.contains(&t.as_str()) {
                self.pop();
            } else {
                break;
            }
        }
    }

    fn close_p_if_button_scope(&mut self) {
        if self.has_in_button_scope("p") {
            self.gen_implied_end(&["p"]);
            self.pop_until(&["p"]);
        }
    }

    fn reconstruct_afe(&mut self) {
        if self.afe.is_empty() {
            return;
        }
        // find earliest entry after marker/stack boundary
        let mut i = self.afe.len();
        loop {
            if i == 0 {
                break;
            }
            match &self.afe[i - 1] {
                AfeItem::Marker => break,
                AfeItem::Element(n) => {
                    if self.open.contains(n) {
                        break;
                    }
                }
            }
            i -= 1;
        }
        // re-create elements i..end
        let to_redo: Vec<NodeId> = self.afe[i..]
            .iter()
            .filter_map(|it| match it {
                AfeItem::Element(n) => Some(*n),
                _ => None,
            })
            .collect();
        for old in to_redo {
            let (tag, attrs) = {
                let e = self.doc.node(old).element().unwrap();
                (
                    e.tag.to_string(),
                    e.attrs
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.clone()))
                        .collect::<Vec<_>>(),
                )
            };
            let newel = self.doc.create_element(&tag);
            for (k, v) in &attrs {
                if let Some(e) = self.doc.node_mut(newel).element_mut() {
                    e.set_attr(k, v);
                }
            }
            self.insert_at(newel);
            self.open.push(newel);
            // replace in afe
            for it in self.afe.iter_mut() {
                if let AfeItem::Element(n) = it {
                    if *n == old {
                        *n = newel;
                    }
                }
            }
        }
    }

    fn push_afe(&mut self, n: NodeId) {
        // Noah's ark rule: max 3 identical elements
        let tag = self.doc.tag(n).to_string();
        let mut count = 0;
        for it in self.afe.iter().rev() {
            match it {
                AfeItem::Marker => break,
                AfeItem::Element(e) => {
                    if self.doc.tag(*e) == tag {
                        count += 1;
                    }
                }
            }
        }
        if count >= 3 {
            for i in (0..self.afe.len()).rev() {
                if let AfeItem::Element(e) = self.afe[i] {
                    if self.doc.tag(e) == tag {
                        self.afe.remove(i);
                        break;
                    }
                }
            }
        }
        self.afe.push(AfeItem::Element(n));
    }

    fn clear_afe_to_marker(&mut self) {
        while let Some(it) = self.afe.pop() {
            if matches!(it, AfeItem::Marker) {
                break;
            }
        }
    }

    /// Adoption agency algorithm for mis-nested formatting end tags.
    fn adoption_agency(&mut self, tag: &str) -> bool {
        // returns true if fully handled
        let cur_tag = self.cur_tag();
        if cur_tag == tag {
            if let Some(c) = self.cur() {
                if !self.in_afe(c) {
                    self.pop();
                    return true;
                }
            }
        }
        for _ in 0..8 {
            // find formatting element
            let Some(fe_pos) = self.afe.iter().rposition(|it| match it {
                AfeItem::Element(n) => self.doc.tag(*n) == tag,
                _ => false,
            }) else {
                return false; // treat as normal end tag
            };
            let fe = match self.afe[fe_pos] {
                AfeItem::Element(n) => n,
                _ => unreachable!(),
            };
            if !self.open.contains(&fe) {
                self.afe.remove(fe_pos);
                return true;
            }
            if !self.has_in_default_scope(self.doc.tag(fe)) {
                return true;
            }
            if self.cur() != Some(fe) {
                // parse error; continue anyway
            }
            // find furthest block: topmost special element below fe on stack
            let fe_stack_pos = self.open.iter().position(|&n| n == fe).unwrap();
            let mut fb: Option<NodeId> = None;
            for &n in &self.open[fe_stack_pos + 1..] {
                if SPECIAL.contains(&self.doc.tag(n)) {
                    fb = Some(n);
                    break;
                }
            }
            let Some(fb) = fb else {
                // no furthest block: pop up to and including fe
                while let Some(n) = self.pop() {
                    if n == fe {
                        break;
                    }
                }
                self.afe.remove(fe_pos);
                return true;
            };
            // common ancestor = element below fe
            let ca = self.open[fe_stack_pos - 1];
            // inner loop: move nodes between fe and fb under fe
            let mut node = fb;
            let mut last = fb;
            let mut inner = 0;
            loop {
                inner += 1;
                // node = element above last in stack
                let pos = self.open.iter().position(|&n| n == node);
                let Some(mut np) = pos else { break };
                if np == 0 {
                    break;
                }
                np -= 1;
                node = self.open[np];
                if node == fe {
                    break;
                }
                if inner > 3 && self.in_afe(node) {
                    // remove from afe
                    if let Some(i) = self
                        .afe
                        .iter()
                        .position(|it| matches!(it, AfeItem::Element(e) if *e == node))
                    {
                        self.afe.remove(i);
                    }
                    self.open.retain(|&x| x != node);
                    continue;
                }
                if !self.in_afe(node) {
                    self.open.retain(|&x| x != node);
                    continue;
                }
                // create clone of node, put under fe... per spec: replace
                let (tag2, attrs) = {
                    let e = self.doc.node(node).element().unwrap();
                    (
                        e.tag.to_string(),
                        e.attrs
                            .iter()
                            .map(|(k, v)| (k.to_string(), v.clone()))
                            .collect::<Vec<_>>(),
                    )
                };
                let clone = self.doc.create_element(&tag2);
                for (k, v) in &attrs {
                    if let Some(e) = self.doc.node_mut(clone).element_mut() {
                        e.set_attr(k, v);
                    }
                }
                // replace node by clone in open & afe
                for x in self.open.iter_mut() {
                    if *x == node {
                        *x = clone;
                    }
                }
                for it in self.afe.iter_mut() {
                    if let AfeItem::Element(e) = it {
                        if *e == node {
                            *e = clone;
                        }
                    }
                }
                node = clone;
                if last == fb {
                    // move last under node
                }
                self.doc.detach(last);
                self.doc.append(node, last);
                last = node;
            }
            // move last under common ancestor (or foster if ca is table-ish)
            self.doc.detach(last);
            let ca_tag = self.doc.tag(ca);
            if matches!(ca_tag, "table" | "tbody" | "tfoot" | "thead" | "tr") {
                if let Some((p, before)) = self.foster_location() {
                    match before {
                        Some(b) => self.doc.insert_before(p, last, b),
                        None => self.doc.append(p, last),
                    }
                } else {
                    self.doc.append(ca, last);
                }
            } else {
                self.doc.append(ca, last);
            }
            // clone fe, move fb children into clone, append clone under fb
            let (tag3, attrs3) = {
                let e = self.doc.node(fe).element().unwrap();
                (
                    e.tag.to_string(),
                    e.attrs
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.clone()))
                        .collect::<Vec<_>>(),
                )
            };
            let fe2 = self.doc.create_element(&tag3);
            for (k, v) in &attrs3 {
                if let Some(e) = self.doc.node_mut(fe2).element_mut() {
                    e.set_attr(k, v);
                }
            }
            let fbkids = self.doc.children(fb).to_vec();
            for k in fbkids {
                self.doc.detach(k);
                self.doc.append(fe2, k);
            }
            self.doc.append(fb, fe2);
            // replace fe by fe2 in afe, remove fe from stack, insert fe2 under fb in stack
            self.afe
                .retain(|it| !matches!(it, AfeItem::Element(e) if *e == fe));
            self.open.retain(|&x| x != fe);
            let fbpos = self.open.iter().position(|&x| x == fb).unwrap();
            self.open.insert(fbpos + 1, fe2);
            self.afe.push(AfeItem::Element(fe2));
            // spec inserts at marker position; appending is equivalent here
        }
        true
    }

    fn in_afe(&self, n: NodeId) -> bool {
        self.afe
            .iter()
            .any(|it| matches!(it, AfeItem::Element(e) if *e == n))
    }

    // ------------- per-mode dispatch -------------

    fn process(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        let mode = self.mode;
        match mode {
            Mode::Initial => self.m_initial(tok, scripts),
            Mode::BeforeHtml => self.m_before_html(tok, scripts),
            Mode::BeforeHead => self.m_before_head(tok, scripts),
            Mode::InHead => self.m_in_head(tok, scripts),
            Mode::AfterHead => self.m_after_head(tok, scripts),
            Mode::InBody => self.m_in_body(tok, scripts),
            Mode::Text => self.m_text(tok),
            Mode::InTable => self.m_in_table(tok, scripts),
            Mode::InCaption => self.m_in_caption(tok, scripts),
            Mode::InColumnGroup => self.m_in_colgroup(tok, scripts),
            Mode::InTableBody => self.m_in_tbody(tok, scripts),
            Mode::InRow => self.m_in_row(tok, scripts),
            Mode::InCell => self.m_in_cell(tok, scripts),
            Mode::InSelect => self.m_in_select(tok, scripts),
            Mode::AfterBody => self.m_after_body(tok, scripts),
            Mode::AfterAfterBody => self.m_after_after_body(tok, scripts),
            Mode::InFrameset => self.m_in_frameset(tok, scripts),
        }
    }

    /// Dispatch `tok` to `mode`'s handler once. If the handler did not
    /// itself change the insertion mode, restore the previous mode —
    /// matching "process using the rules for X" in the spec.
    fn reprocess(&mut self, tok: Token, scripts: &mut Vec<NodeId>, mode: Mode) {
        let saved = self.mode;
        self.mode = mode;
        self.process(tok, scripts);
        if self.mode == mode {
            self.mode = saved;
        }
    }

    // --- Initial ---
    fn m_initial(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        match tok {
            Token::Doctype {
                name,
                public_id,
                force_quirks,
                ..
            } => {
                let dt = self
                    .doc
                    .create_doctype(&name, public_id.as_deref().unwrap_or(""), "");
                let root = self.doc.root;
                self.doc.append(root, dt);
                if force_quirks || name != "html" {
                    self.quirks = true;
                }
                self.mode = Mode::BeforeHtml;
            }
            Token::Text(s) if s.trim().is_empty() => {}
            Token::Comment(c) => {
                let n = self.doc.create_comment(&c);
                let root = self.doc.root;
                self.doc.append(root, n);
            }
            t => {
                self.quirks = true;
                self.mode = Mode::BeforeHtml;
                self.process(t, scripts);
            }
        }
    }

    // --- BeforeHtml ---
    fn m_before_html(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        match tok {
            Token::Doctype { .. } => {}
            Token::Comment(c) => {
                let n = self.doc.create_comment(&c);
                let root = self.doc.root;
                self.doc.append(root, n);
            }
            Token::Text(s) if s.trim().is_empty() => {}
            Token::StartTag { name, attrs, .. } if name == "html" => {
                let el = self.doc.create_element("html");
                for (k, v) in &attrs {
                    if let Some(e) = self.doc.node_mut(el).element_mut() {
                        e.set_attr(k, v);
                    }
                }
                if let Some(f) = self.fragment {
                    self.doc.append(f, el);
                } else {
                    let root = self.doc.root;
                    self.doc.append(root, el);
                }
                self.open.push(el);
                self.mode = Mode::BeforeHead;
            }
            Token::EndTag { name } if !matches!(name.as_str(), "head" | "body" | "html" | "br") => {
            }
            t => {
                let el = self.doc.create_element("html");
                if let Some(f) = self.fragment {
                    self.doc.append(f, el);
                } else {
                    let root = self.doc.root;
                    self.doc.append(root, el);
                }
                self.open.push(el);
                self.mode = Mode::BeforeHead;
                self.process(t, scripts);
            }
        }
    }

    // --- BeforeHead ---
    fn m_before_head(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        match tok {
            Token::Text(s) if s.trim().is_empty() => {}
            Token::Comment(c) => {
                let n = self.doc.create_comment(&c);
                let p = self.adjusted_insertion();
                self.doc.append(p, n);
            }
            Token::StartTag { name, attrs, .. } => match name.as_str() {
                "html" => self.reprocess(
                    Token::StartTag {
                        name,
                        attrs,
                        self_closing: false,
                    },
                    scripts,
                    Mode::InBody,
                ),
                "head" => {
                    let el = self.insert_element("head", &attrs);
                    self.head = Some(el);
                    self.mode = Mode::InHead;
                }
                _ => self.before_head_fallback(
                    Token::StartTag {
                        name,
                        attrs,
                        self_closing: false,
                    },
                    scripts,
                ),
            },
            Token::EndTag { name } if !matches!(name.as_str(), "head" | "body" | "html" | "br") => {
            }
            Token::EndTag { name } => self.before_head_fallback(Token::EndTag { name }, scripts),
            t => self.before_head_fallback(t, scripts),
        }
    }

    fn before_head_fallback(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        let el = self.insert_element("head", &[]);
        self.head = Some(el);
        self.mode = Mode::InHead;
        self.process(tok, scripts);
    }

    // --- InHead ---
    fn m_in_head(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        match tok {
            Token::Text(s) if s.trim().is_empty() => self.insert_text(&s),
            Token::Comment(c) => {
                let n = self.doc.create_comment(&c);
                let p = self.adjusted_insertion();
                self.doc.append(p, n);
            }
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => match name.as_str() {
                "html" => self.reprocess(
                    Token::StartTag {
                        name,
                        attrs,
                        self_closing,
                    },
                    scripts,
                    Mode::InBody,
                ),
                "base" | "basefont" | "bgsound" | "link" | "meta" => {
                    self.insert_element(&name, &attrs);
                    self.pop();
                }
                "title" => {
                    let el = self.insert_element("title", &attrs);
                    self.tokenizer.enter_raw(RawKind::Rcdata, "title");
                    self.orig_mode = self.mode;
                    self.mode = Mode::Text;
                    self.text_node = Some(el);
                }
                "style" | "noframes" | "noscript" | "noembed" => {
                    let el = self.insert_element(&name, &attrs);
                    self.tokenizer.enter_raw(RawKind::Rawtext, &name);
                    self.orig_mode = self.mode;
                    self.mode = Mode::Text;
                    self.text_node = Some(el);
                }
                "script" => {
                    let el = self.insert_element("script", &attrs);
                    scripts.push(el);
                    self.tokenizer.enter_raw(RawKind::Script, "script");
                    self.orig_mode = self.mode;
                    self.mode = Mode::Text;
                    self.text_node = Some(el);
                }
                "template" => {
                    let _el = self.insert_element("template", &attrs);
                    self.afe.push(AfeItem::Marker);
                    self.mode = Mode::InBody; // simplified: template content as normal
                }
                "head" => {}
                _ => self.in_head_fallback(
                    Token::StartTag {
                        name,
                        attrs,
                        self_closing,
                    },
                    scripts,
                ),
            },
            Token::EndTag { name } => match name.as_str() {
                "head" => {
                    self.pop();
                    self.mode = Mode::AfterHead;
                }
                "template" => {
                    if self.open.iter().any(|&n| self.doc.is_tag(n, "template")) {
                        self.pop_until(&["template"]);
                        self.clear_afe_to_marker();
                    }
                }
                "body" | "html" | "br" => self.in_head_fallback(Token::EndTag { name }, scripts),
                _ => {}
            },
            Token::Eof => self.in_head_fallback(tok, scripts),
            t => self.in_head_fallback(t, scripts),
        }
    }

    fn in_head_fallback(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        self.pop(); // head
        self.mode = Mode::AfterHead;
        self.process(tok, scripts);
    }

    // --- AfterHead ---
    fn m_after_head(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        match tok {
            Token::Text(s) if s.trim().is_empty() => self.insert_text(&s),
            Token::Comment(c) => {
                let n = self.doc.create_comment(&c);
                let p = self.adjusted_insertion();
                self.doc.append(p, n);
            }
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => match name.as_str() {
                "html" => self.reprocess(
                    Token::StartTag {
                        name,
                        attrs,
                        self_closing,
                    },
                    scripts,
                    Mode::InBody,
                ),
                "body" => {
                    self.insert_element("body", &attrs);
                    self.mode = Mode::InBody;
                    self.frameset_ok = false;
                }
                "frameset" => {
                    self.insert_element("frameset", &attrs);
                    self.mode = Mode::InFrameset;
                    self.seen_frameset = true;
                }
                "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script"
                | "style" | "template" | "title" => {
                    if let Some(h) = self.head {
                        self.open.push(h);
                    }
                    self.reprocess(
                        Token::StartTag {
                            name,
                            attrs,
                            self_closing,
                        },
                        scripts,
                        Mode::InHead,
                    );
                    if let Some(h) = self.head {
                        if self.open.last() == Some(&h) {
                            self.pop();
                        }
                    }
                }
                "head" => {}
                _ => {
                    self.insert_element("body", &[]);
                    self.mode = Mode::InBody;
                    self.frameset_ok = false;
                    self.process(
                        Token::StartTag {
                            name,
                            attrs,
                            self_closing,
                        },
                        scripts,
                    );
                }
            },
            Token::EndTag { name } => {
                if matches!(name.as_str(), "body" | "html" | "br") {
                    self.insert_element("body", &[]);
                    self.mode = Mode::InBody;
                    self.process(Token::EndTag { name }, scripts);
                } else if name == "template" {
                    self.reprocess(Token::EndTag { name }, scripts, Mode::InHead);
                }
            }
            t => {
                self.insert_element("body", &[]);
                self.mode = Mode::InBody;
                self.frameset_ok = false;
                self.process(t, scripts);
            }
        }
    }

    // --- InBody ---
    fn m_in_body(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        match tok {
            Token::Text(s) => {
                if s.chars().any(|c| !c.is_ascii_whitespace()) {
                    self.frameset_ok = false;
                }
                self.reconstruct_afe();
                self.insert_text(&s);
            }
            Token::Comment(c) => {
                let n = self.doc.create_comment(&c);
                let p = self.adjusted_insertion();
                self.doc.append(p, n);
            }
            Token::Doctype { .. } => {}
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => self.body_start(&name, &attrs, self_closing, scripts),
            Token::EndTag { name } => self.body_end(&name, scripts),
            Token::Eof => {}
        }
    }

    fn body_start(
        &mut self,
        name: &str,
        attrs: &[(String, String)],
        self_closing: bool,
        scripts: &mut Vec<NodeId>,
    ) {
        match name {
            "html" => {
                if let Some(&top) = self.open.first() {
                    for (k, v) in attrs {
                        if !self.doc.node(top).element().unwrap().has_attr(k) {
                            if let Some(e) = self.doc.node_mut(top).element_mut() {
                                e.set_attr(k, v);
                            }
                        }
                    }
                }
            }
            "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script" | "style"
            | "template" | "title" => {
                self.reprocess(
                    Token::StartTag {
                        name: name.into(),
                        attrs: attrs.to_vec(),
                        self_closing,
                    },
                    scripts,
                    Mode::InHead,
                );
            }
            "body" => {
                if self.open.len() > 1 {
                    let body = self.open[1];
                    if self.doc.is_tag(body, "body") {
                        for (k, v) in attrs {
                            if !self.doc.node(body).element().unwrap().has_attr(k) {
                                self.doc.set_attribute(body, k, v);
                            }
                        }
                        self.frameset_ok = false;
                    }
                }
            }
            "frameset" => {
                // ignore unless frameset_ok; per spec only if no body seen
            }
            "address" | "article" | "aside" | "blockquote" | "center" | "details" | "dialog"
            | "dir" | "div" | "dl" | "fieldset" | "figcaption" | "figure" | "footer" | "header"
            | "hgroup" | "main" | "menu" | "nav" | "ol" | "section" | "summary" | "ul" => {
                self.close_p_if_button_scope();
                self.insert_element(name, attrs);
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.close_p_if_button_scope();
                if matches!(
                    self.cur_tag().as_str(),
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                ) {
                    self.pop();
                }
                self.insert_element(name, attrs);
            }
            "pre" | "listing" => {
                self.close_p_if_button_scope();
                self.insert_element(name, attrs);
                // skip a single leading newline
                self.frameset_ok = false;
                if self.tokenizer.peek() == Some('\n') {
                    self.tokenizer.bump();
                }
            }
            "form" => {
                if self.form.is_none() {
                    self.close_p_if_button_scope();
                    let el = self.insert_element(name, attrs);
                    self.form = Some(el);
                } else {
                    // second form ignored
                }
            }
            "li" => {
                self.frameset_ok = false;
                // close open li
                for &n in self.open.clone().iter().rev() {
                    let t = self.doc.tag(n);
                    if t == "li" {
                        self.gen_implied_end(&["li"]);
                        self.pop_until(&["li"]);
                        break;
                    }
                    if SPECIAL.contains(&t) && !matches!(t, "address" | "div" | "p") {
                        break;
                    }
                }
                self.close_p_if_button_scope();
                self.insert_element(name, attrs);
            }
            "dd" | "dt" => {
                self.frameset_ok = false;
                for &n in self.open.clone().iter().rev() {
                    let t = self.doc.tag(n).to_string();
                    if t == "dd" || t == "dt" {
                        self.gen_implied_end(&[&t]);
                        self.pop_until(&[&t]);
                        break;
                    }
                    if SPECIAL.contains(&t.as_str())
                        && !matches!(t.as_str(), "address" | "div" | "p")
                    {
                        break;
                    }
                }
                self.close_p_if_button_scope();
                self.insert_element(name, attrs);
            }
            "plaintext" => {
                self.close_p_if_button_scope();
                self.insert_element(name, attrs);
                self.tokenizer.enter_raw(RawKind::Plaintext, "plaintext");
            }
            "button" => {
                if self.has_in_default_scope("button") {
                    self.gen_implied_end(&[]);
                    self.pop_until(&["button"]);
                }
                self.reconstruct_afe();
                self.insert_element(name, attrs);
                self.frameset_ok = false;
            }
            "a" => {
                // close an open <a> first via adoption agency
                if self
                    .afe
                    .iter()
                    .any(|it| matches!(it, AfeItem::Element(e) if self.doc.tag(*e) == "a"))
                {
                    self.adoption_agency("a");
                }
                self.reconstruct_afe();
                let el = self.insert_element(name, attrs);
                self.push_afe(el);
            }
            _ if FORMATTING.contains(&name) => {
                self.reconstruct_afe();
                let el = self.insert_element(name, attrs);
                self.push_afe(el);
            }
            "nobr" => {
                self.reconstruct_afe();
                if self.has_in_default_scope("nobr") {
                    self.adoption_agency("nobr");
                    self.reconstruct_afe();
                }
                let el = self.insert_element(name, attrs);
                self.push_afe(el);
            }
            "applet" | "marquee" | "object" => {
                self.reconstruct_afe();
                self.insert_element(name, attrs);
                self.afe.push(AfeItem::Marker);
                self.frameset_ok = false;
            }
            "table" => {
                if !self.doc.quirks_mode {
                    self.close_p_if_button_scope();
                }
                self.insert_element(name, attrs);
                self.frameset_ok = false;
                self.mode = Mode::InTable;
            }
            "area" | "br" | "embed" | "img" | "keygen" | "wbr" => {
                self.reconstruct_afe();
                self.insert_element(name, attrs);
                self.pop();
                self.frameset_ok = false;
            }
            "input" => {
                self.reconstruct_afe();
                let el = self.insert_element(name, attrs);
                self.pop();
                let ty = self.doc.attr(el, "type").unwrap_or("");
                if !ty.eq_ignore_ascii_case("hidden") {
                    self.frameset_ok = false;
                }
            }
            "param" | "source" | "track" | "hr" => {
                self.close_p_if_button_scope();
                self.insert_element(name, attrs);
                self.pop();
                self.frameset_ok = false;
            }
            "image" => {
                self.body_start("img", attrs, self_closing, scripts);
            }
            "textarea" => {
                let el = self.insert_element(name, attrs);
                if self.tokenizer.peek() == Some('\n') {
                    self.tokenizer.bump();
                }
                self.tokenizer.enter_raw(RawKind::Rcdata, "textarea");
                self.orig_mode = self.mode;
                self.mode = Mode::Text;
                self.text_node = Some(el);
                self.frameset_ok = false;
            }
            "xmp" => {
                self.close_p_if_button_scope();
                self.reconstruct_afe();
                let el = self.insert_element(name, attrs);
                self.tokenizer.enter_raw(RawKind::Rawtext, "xmp");
                self.orig_mode = self.mode;
                self.mode = Mode::Text;
                self.text_node = Some(el);
                self.frameset_ok = false;
            }
            "iframe" => {
                self.frameset_ok = false;
                let el = self.insert_element(name, attrs);
                self.tokenizer.enter_raw(RawKind::Rawtext, "iframe");
                self.orig_mode = self.mode;
                self.mode = Mode::Text;
                self.text_node = Some(el);
            }
            "noembed" => {
                let el = self.insert_element(name, attrs);
                self.tokenizer.enter_raw(RawKind::Rawtext, name);
                self.orig_mode = self.mode;
                self.mode = Mode::Text;
                self.text_node = Some(el);
            }
            "select" => {
                self.reconstruct_afe();
                self.insert_element(name, attrs);
                self.frameset_ok = false;
                self.mode = Mode::InSelect;
            }
            "optgroup" | "option" => {
                if self.cur_tag() == "option" {
                    self.pop();
                }
                self.reconstruct_afe();
                self.insert_element(name, attrs);
            }
            "rb" | "rtc" | "rp" | "rt" | "ruby" => {
                self.insert_element(name, attrs);
            }
            "caption" | "col" | "colgroup" | "frame" | "head" | "tbody" | "td" | "tfoot" | "th"
            | "thead" | "tr" => {
                // ignore in body
            }
            "svg" | "math" => {
                self.reconstruct_afe();
                let el = self.insert_element(name, attrs);
                if let Some(e) = self.doc.node_mut(el).element_mut() {
                    e.namespace = if name == "svg" {
                        sol_dom::Namespace::Svg
                    } else {
                        sol_dom::Namespace::MathMl
                    };
                }
                if self_closing {
                    self.pop();
                }
            }
            _ => {
                self.reconstruct_afe();
                self.insert_element(name, attrs);
                if self_closing {
                    self.pop();
                }
            }
        }
    }

    fn body_end(&mut self, name: &str, scripts: &mut Vec<NodeId>) {
        match name {
            "body" => {
                if self.open.iter().any(|&n| self.doc.is_tag(n, "body")) {
                    self.mode = Mode::AfterBody;
                }
            }
            "html" => {
                self.body_end("body", scripts);
                if self.mode == Mode::AfterBody {
                    self.reprocess(
                        Token::EndTag {
                            name: "html".into(),
                        },
                        scripts,
                        Mode::AfterBody,
                    );
                }
            }
            "address" | "article" | "aside" | "blockquote" | "button" | "center" | "details"
            | "dialog" | "dir" | "div" | "dl" | "fieldset" | "figcaption" | "figure" | "footer"
            | "header" | "hgroup" | "listing" | "main" | "menu" | "nav" | "ol" | "pre"
            | "section" | "summary" | "ul" => {
                if self.has_in_button_scope(name) {
                    self.gen_implied_end(&[]);
                    self.pop_until(&[name]);
                }
            }
            "form" => {
                let f = self.form.take();
                if let Some(f) = f {
                    if self.has_in_button_scope("form") {
                        self.gen_implied_end(&[]);
                        self.open.retain(|&x| x != f);
                    }
                } else if self.has_in_button_scope("form") {
                    self.gen_implied_end(&[]);
                    self.pop_until(&["form"]);
                }
            }
            "p" => {
                if !self.has_in_button_scope("p") {
                    self.insert_element("p", &[]);
                }
                self.close_p_if_button_scope();
            }
            "li" => {
                if self.has_in_list_scope("li") {
                    self.gen_implied_end(&["li"]);
                    self.pop_until(&["li"]);
                }
            }
            "dd" | "dt" => {
                if self.has_in_default_scope(name) {
                    self.gen_implied_end(&[name]);
                    self.pop_until(&[name]);
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if self.open.iter().any(|&n| {
                    let t = self.doc.tag(n);
                    matches!(t, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
                }) {
                    self.gen_implied_end(&[]);
                    while let Some(n) = self.pop() {
                        if matches!(self.doc.tag(n), "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                            break;
                        }
                    }
                }
            }
            "a" | "b" | "big" | "code" | "em" | "font" | "i" | "nobr" | "s" | "small"
            | "strike" | "strong" | "tt" | "u" => {
                self.adoption_agency(name);
            }
            "applet" | "marquee" | "object" => {
                if self.has_in_default_scope(name) {
                    self.gen_implied_end(&[]);
                    self.pop_until(&[name]);
                    self.clear_afe_to_marker();
                }
            }
            "br" => {
                // treat as <br> start tag (parse error recovery)
                self.reconstruct_afe();
                self.insert_element("br", &[]);
                self.pop();
            }
            "template" => {
                self.reprocess(Token::EndTag { name: name.into() }, scripts, Mode::InHead);
            }
            _ => {
                // any other end tag: walk stack for match
                for i in (0..self.open.len()).rev() {
                    let n = self.open[i];
                    if self.doc.tag(n) == name {
                        self.gen_implied_end(&[name]);
                        while self.open.len() > i {
                            self.pop();
                        }
                        break;
                    }
                    if SPECIAL.contains(&self.doc.tag(n)) {
                        break;
                    }
                }
            }
        }
    }

    // --- Text (raw text / rcdata / script) ---
    fn m_text(&mut self, tok: Token) {
        match tok {
            Token::Text(s) => {
                if let Some(el) = self.text_node {
                    let p = el;
                    if let Some(&last) = self.doc.children(p).last() {
                        if let NodeData::Text(t) = &mut self.doc.node_mut(last).data {
                            t.push_str(&s);
                            return;
                        }
                    }
                    let t = self.doc.create_text(&s);
                    self.doc.append(p, t);
                } else {
                    self.insert_text(&s);
                }
            }
            Token::EndTag { name } => {
                // pop the raw-text element
                if self.cur_tag() == name {
                    self.pop();
                } else {
                    self.pop_until(&[name.as_str()]);
                }
                self.text_node = None;
                self.mode = self.orig_mode;
            }
            Token::Eof => {
                if self.cur().is_some() {
                    self.pop();
                }
                self.text_node = None;
                self.mode = self.orig_mode;
            }
            _ => {}
        }
    }

    // --- Table modes ---
    fn m_in_table(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        match tok {
            Token::Text(s) => {
                // pending table text: if any non-ws, foster-parent all
                if s.chars().all(|c| c.is_ascii_whitespace()) {
                    self.insert_text(&s);
                } else {
                    self.foster_parenting = true;
                    self.reprocess(Token::Text(s), scripts, Mode::InBody);
                    self.foster_parenting = false;
                }
            }
            Token::Comment(c) => {
                let n = self.doc.create_comment(&c);
                let p = self.adjusted_insertion();
                self.doc.append(p, n);
            }
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => match name.as_str() {
                "caption" => {
                    self.clear_stack_to_table();
                    self.afe.push(AfeItem::Marker);
                    self.insert_element("caption", &attrs);
                    self.mode = Mode::InCaption;
                }
                "colgroup" => {
                    self.clear_stack_to_table();
                    self.insert_element("colgroup", &attrs);
                    self.mode = Mode::InColumnGroup;
                }
                "col" => {
                    self.m_in_table(
                        Token::StartTag {
                            name: "colgroup".into(),
                            attrs: vec![],
                            self_closing: false,
                        },
                        scripts,
                    );
                    self.reprocess(
                        Token::StartTag {
                            name,
                            attrs,
                            self_closing,
                        },
                        scripts,
                        Mode::InColumnGroup,
                    );
                }
                "tbody" | "tfoot" | "thead" => {
                    self.clear_stack_to_table();
                    self.insert_element(&name, &attrs);
                    self.mode = Mode::InTableBody;
                }
                "td" | "th" | "tr" => {
                    self.m_in_table(
                        Token::StartTag {
                            name: "tbody".into(),
                            attrs: vec![],
                            self_closing: false,
                        },
                        scripts,
                    );
                    self.reprocess(
                        Token::StartTag {
                            name,
                            attrs,
                            self_closing,
                        },
                        scripts,
                        Mode::InTableBody,
                    );
                }
                "table" => {
                    // nested table: close current first
                    self.m_in_table(
                        Token::EndTag {
                            name: "table".into(),
                        },
                        scripts,
                    );
                    if self.mode != Mode::InTable {
                        self.process(
                            Token::StartTag {
                                name,
                                attrs,
                                self_closing,
                            },
                            scripts,
                        );
                    }
                }
                "style" | "script" | "template" => {
                    self.reprocess(
                        Token::StartTag {
                            name,
                            attrs,
                            self_closing,
                        },
                        scripts,
                        Mode::InHead,
                    );
                }
                "input" => {
                    let ty = attrs
                        .iter()
                        .find(|(k, _)| k == "type")
                        .map(|(_, v)| v.as_str())
                        .unwrap_or("");
                    if ty.eq_ignore_ascii_case("hidden") {
                        self.insert_element("input", &attrs);
                        self.pop();
                    } else {
                        self.foster_parenting = true;
                        self.reprocess(
                            Token::StartTag {
                                name,
                                attrs,
                                self_closing,
                            },
                            scripts,
                            Mode::InBody,
                        );
                        self.foster_parenting = false;
                    }
                }
                "form" => {
                    if self.form.is_none() {
                        let el = self.insert_element("form", &attrs);
                        self.form = Some(el);
                        self.pop();
                    }
                }
                _ => {
                    self.foster_parenting = true;
                    self.reprocess(
                        Token::StartTag {
                            name,
                            attrs,
                            self_closing,
                        },
                        scripts,
                        Mode::InBody,
                    );
                    self.foster_parenting = false;
                }
            },
            Token::EndTag { name } => match name.as_str() {
                "table" => {
                    if self.has_in_table_scope("table") {
                        self.pop_until(&["table"]);
                        self.reset_insertion_mode();
                    }
                }
                "template" => self.reprocess(Token::EndTag { name }, scripts, Mode::InHead),
                "body" | "caption" | "col" | "colgroup" | "html" | "tbody" | "td" | "tfoot"
                | "th" | "thead" | "tr" => {}
                _ => {
                    self.foster_parenting = true;
                    self.reprocess(Token::EndTag { name }, scripts, Mode::InBody);
                    self.foster_parenting = false;
                }
            },
            Token::Eof => {}
            Token::Doctype { .. } => {}
        }
    }

    fn clear_stack_to_table(&mut self) {
        while let Some(&n) = self.open.last() {
            if matches!(self.doc.tag(n), "table" | "html" | "template") {
                break;
            }
            self.pop();
        }
    }
    fn clear_stack_to_table_body(&mut self) {
        while let Some(&n) = self.open.last() {
            if matches!(
                self.doc.tag(n),
                "tbody" | "tfoot" | "thead" | "html" | "template"
            ) {
                break;
            }
            self.pop();
        }
    }
    fn clear_stack_to_row(&mut self) {
        while let Some(&n) = self.open.last() {
            if matches!(self.doc.tag(n), "tr" | "html" | "template") {
                break;
            }
            self.pop();
        }
    }

    fn reset_insertion_mode(&mut self) {
        // simplified "reset the insertion mode appropriately"
        for &n in self.open.iter().rev() {
            match self.doc.tag(n) {
                "select" => {
                    self.mode = Mode::InSelect;
                    return;
                }
                "td" | "th" => {
                    self.mode = Mode::InCell;
                    return;
                }
                "tr" => {
                    self.mode = Mode::InRow;
                    return;
                }
                "tbody" | "thead" | "tfoot" => {
                    self.mode = Mode::InTableBody;
                    return;
                }
                "caption" => {
                    self.mode = Mode::InCaption;
                    return;
                }
                "colgroup" => {
                    self.mode = Mode::InColumnGroup;
                    return;
                }
                "table" => {
                    self.mode = Mode::InTable;
                    return;
                }
                "head" => {
                    self.mode = Mode::InBody;
                    return;
                }
                "body" => {
                    self.mode = Mode::InBody;
                    return;
                }
                "frameset" => {
                    self.mode = Mode::InFrameset;
                    return;
                }
                _ => {}
            }
        }
        self.mode = Mode::InBody;
    }

    fn m_in_caption(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        let is_cap_end =
            matches!(&tok, Token::EndTag { name } if name == "caption" || name == "table");
        let is_table_start = matches!(&tok, Token::StartTag { name, .. } if matches!(name.as_str(), "caption"|"col"|"colgroup"|"tbody"|"td"|"tfoot"|"th"|"thead"|"tr"));
        if is_cap_end || is_table_start {
            if self.has_in_table_scope("caption") {
                self.gen_implied_end(&[]);
                self.pop_until(&["caption"]);
                self.clear_afe_to_marker();
                self.mode = Mode::InTable;
            }
            if matches!(&tok, Token::EndTag { name } if name == "caption") {
                return;
            }
            self.process(tok, scripts);
            return;
        }
        self.reprocess(tok, scripts, Mode::InBody);
    }

    fn m_in_colgroup(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        match tok {
            Token::Text(s) if s.trim().is_empty() => self.insert_text(&s),
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => match name.as_str() {
                "col" => {
                    self.insert_element("col", &attrs);
                    self.pop();
                }
                "template" => self.reprocess(
                    Token::StartTag {
                        name,
                        attrs,
                        self_closing,
                    },
                    scripts,
                    Mode::InHead,
                ),
                _ => self.colgroup_fallback(
                    Token::StartTag {
                        name,
                        attrs,
                        self_closing,
                    },
                    scripts,
                ),
            },
            Token::EndTag { name } if name == "colgroup" => {
                if self.cur_tag() == "colgroup" {
                    self.pop();
                    self.mode = Mode::InTable;
                }
            }
            Token::Eof => self.reprocess(tok, scripts, Mode::InBody),
            t => self.colgroup_fallback(t, scripts),
        }
    }
    fn colgroup_fallback(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        if self.cur_tag() == "colgroup" {
            self.pop();
            self.mode = Mode::InTable;
        }
        self.process(tok, scripts);
    }

    fn m_in_tbody(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        match tok {
            Token::StartTag { name, attrs, .. } if name == "tr" => {
                self.clear_stack_to_table_body();
                self.insert_element("tr", &attrs);
                self.mode = Mode::InRow;
            }
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } if name == "td" || name == "th" => {
                self.clear_stack_to_table_body();
                self.insert_element("tr", &[]);
                self.mode = Mode::InRow;
                self.process(
                    Token::StartTag {
                        name,
                        attrs,
                        self_closing,
                    },
                    scripts,
                );
            }
            Token::EndTag { name } if name == "tbody" || name == "tfoot" || name == "thead" => {
                if self.has_in_table_scope(&name) {
                    self.clear_stack_to_table_body();
                    self.pop();
                    self.mode = Mode::InTable;
                }
            }
            Token::EndTag { name } if name == "table" => {
                if self.has_in_table_scope("tbody")
                    || self.has_in_table_scope("thead")
                    || self.has_in_table_scope("tfoot")
                {
                    self.clear_stack_to_table_body();
                    self.pop();
                    self.mode = Mode::InTable;
                    self.process(Token::EndTag { name }, scripts);
                }
            }
            Token::EndTag { name }
                if name == "body"
                    || name == "caption"
                    || name == "col"
                    || name == "colgroup"
                    || name == "html"
                    || name == "td"
                    || name == "th"
                    || name == "tr" => {}
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } if matches!(
                name.as_str(),
                "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead"
            ) =>
            {
                if self.has_in_table_scope("tbody")
                    || self.has_in_table_scope("thead")
                    || self.has_in_table_scope("tfoot")
                {
                    self.clear_stack_to_table_body();
                    self.pop();
                    self.mode = Mode::InTable;
                    self.process(
                        Token::StartTag {
                            name,
                            attrs,
                            self_closing,
                        },
                        scripts,
                    );
                }
            }
            t => self.reprocess(t, scripts, Mode::InTable),
        }
    }

    fn m_in_row(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        match tok {
            Token::StartTag { name, attrs, .. } if name == "td" || name == "th" => {
                self.clear_stack_to_row();
                self.insert_element(&name, &attrs);
                self.mode = Mode::InCell;
                self.afe.push(AfeItem::Marker);
            }
            Token::EndTag { name } if name == "tr" => {
                if self.has_in_table_scope("tr") {
                    self.clear_stack_to_row();
                    self.pop();
                    self.mode = Mode::InTableBody;
                }
            }
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } if matches!(
                name.as_str(),
                "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead" | "tr"
            ) =>
            {
                self.m_in_row(Token::EndTag { name: "tr".into() }, scripts);
                if self.mode != Mode::InRow {
                    self.process(
                        Token::StartTag {
                            name,
                            attrs,
                            self_closing,
                        },
                        scripts,
                    );
                }
            }
            Token::EndTag { name } if name == "table" => {
                self.m_in_row(Token::EndTag { name: "tr".into() }, scripts);
                if self.mode != Mode::InRow {
                    self.process(Token::EndTag { name }, scripts);
                }
            }
            Token::EndTag { name } if name == "tbody" || name == "tfoot" || name == "thead" => {
                if self.has_in_table_scope(&name) {
                    let n2 = name.clone();
                    self.m_in_row(Token::EndTag { name: "tr".into() }, scripts);
                    if self.mode != Mode::InRow {
                        self.process(Token::EndTag { name: n2 }, scripts);
                    }
                }
            }
            Token::EndTag { name }
                if name == "body"
                    || name == "caption"
                    || name == "col"
                    || name == "colgroup"
                    || name == "html"
                    || name == "td"
                    || name == "th" => {}
            t => self.reprocess(t, scripts, Mode::InTable),
        }
    }

    fn m_in_cell(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        match tok {
            Token::EndTag { name } if name == "td" || name == "th" => {
                if self.has_in_table_scope(&name) {
                    self.gen_implied_end(&[]);
                    self.pop_until(&["td", "th"]);
                    self.clear_afe_to_marker();
                    self.mode = Mode::InRow;
                }
            }
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } if matches!(
                name.as_str(),
                "caption" | "col" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr"
            ) =>
            {
                if self.has_in_table_scope("td") || self.has_in_table_scope("th") {
                    self.close_cell(scripts);
                    self.process(
                        Token::StartTag {
                            name,
                            attrs,
                            self_closing,
                        },
                        scripts,
                    );
                }
            }
            Token::EndTag { name }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html"
                ) => {}
            Token::EndTag { name }
                if matches!(name.as_str(), "table" | "tbody" | "tfoot" | "thead" | "tr") =>
            {
                if self.has_in_table_scope(&name) {
                    self.close_cell(scripts);
                    self.process(Token::EndTag { name }, scripts);
                }
            }
            t => self.reprocess(t, scripts, Mode::InBody),
        }
    }

    fn close_cell(&mut self, _scripts: &mut Vec<NodeId>) {
        self.gen_implied_end(&[]);
        self.pop_until(&["td", "th"]);
        self.clear_afe_to_marker();
        self.mode = Mode::InRow;
    }

    fn m_in_select(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        match tok {
            Token::Text(s) => self.insert_text(&s),
            Token::Comment(c) => {
                let n = self.doc.create_comment(&c);
                let p = self.adjusted_insertion();
                self.doc.append(p, n);
            }
            Token::StartTag { name, attrs, .. } => match name.as_str() {
                "html" => self.reprocess(
                    Token::StartTag {
                        name,
                        attrs,
                        self_closing: false,
                    },
                    scripts,
                    Mode::InBody,
                ),
                "option" => {
                    if self.cur_tag() == "option" {
                        self.pop();
                    }
                    self.insert_element("option", &attrs);
                }
                "optgroup" => {
                    if self.cur_tag() == "option" {
                        self.pop();
                    }
                    if self.cur_tag() == "optgroup" {
                        self.pop();
                    }
                    self.insert_element("optgroup", &attrs);
                }
                "select" => {
                    // treated as end tag
                    if self.has_in_select_scope("select") {
                        self.pop_until(&["select"]);
                        self.reset_insertion_mode();
                    }
                }
                "input" | "keygen" | "textarea" => {
                    if self.has_in_select_scope("select") {
                        self.pop_until(&["select"]);
                        self.reset_insertion_mode();
                        self.process(
                            Token::StartTag {
                                name,
                                attrs,
                                self_closing: false,
                            },
                            scripts,
                        );
                    }
                }
                "script" | "template" => {
                    self.reprocess(
                        Token::StartTag {
                            name,
                            attrs,
                            self_closing: false,
                        },
                        scripts,
                        Mode::InHead,
                    );
                }
                _ => {}
            },
            Token::EndTag { name } => match name.as_str() {
                "optgroup" => {
                    // pop option then optgroup
                    if self.cur_tag() == "option"
                        && self.open.len() >= 2
                        && self.doc.is_tag(self.open[self.open.len() - 2], "optgroup")
                    {
                        self.pop();
                    }
                    if self.cur_tag() == "optgroup" {
                        self.pop();
                    }
                }
                "option" => {
                    if self.cur_tag() == "option" {
                        self.pop();
                    }
                }
                "select" => {
                    if self.has_in_select_scope("select") {
                        self.pop_until(&["select"]);
                        self.reset_insertion_mode();
                    }
                }
                "template" => self.reprocess(Token::EndTag { name }, scripts, Mode::InHead),
                _ => {}
            },
            Token::Eof => self.reprocess(tok, scripts, Mode::InBody),
            Token::Doctype { .. } => {}
        }
    }

    fn m_after_body(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        match tok {
            Token::Text(s) if s.trim().is_empty() => {
                self.reprocess(Token::Text(s), scripts, Mode::InBody)
            }
            Token::Comment(c) => {
                let n = self.doc.create_comment(&c);
                if let Some(&html) = self.open.first() {
                    self.doc.append(html, n);
                }
            }
            Token::StartTag { name, attrs, .. } if name == "html" => self.reprocess(
                Token::StartTag {
                    name,
                    attrs,
                    self_closing: false,
                },
                scripts,
                Mode::InBody,
            ),
            Token::EndTag { name } if name == "html" => self.mode = Mode::AfterAfterBody,
            Token::Eof => {}
            t => {
                self.mode = Mode::InBody;
                self.process(t, scripts);
            }
        }
    }

    fn m_after_after_body(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        match tok {
            Token::Comment(c) => {
                let n = self.doc.create_comment(&c);
                let root = self.doc.root;
                self.doc.append(root, n);
            }
            Token::Text(s) if s.trim().is_empty() => {
                self.reprocess(Token::Text(s), scripts, Mode::InBody)
            }
            Token::StartTag { name, attrs, .. } if name == "html" => self.reprocess(
                Token::StartTag {
                    name,
                    attrs,
                    self_closing: false,
                },
                scripts,
                Mode::InBody,
            ),
            Token::Eof => {}
            t => {
                self.mode = Mode::InBody;
                self.process(t, scripts);
            }
        }
    }

    fn m_in_frameset(&mut self, tok: Token, scripts: &mut Vec<NodeId>) {
        match tok {
            Token::Text(s) if s.trim().is_empty() => self.insert_text(&s),
            Token::Comment(c) => {
                let n = self.doc.create_comment(&c);
                let p = self.adjusted_insertion();
                self.doc.append(p, n);
            }
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => match name.as_str() {
                "frameset" => {
                    self.insert_element("frameset", &attrs);
                }
                "frame" => {
                    self.insert_element("frame", &attrs);
                    self.pop();
                }
                "noframes" => {
                    self.reprocess(
                        Token::StartTag {
                            name,
                            attrs,
                            self_closing,
                        },
                        scripts,
                        Mode::InHead,
                    );
                }
                _ => {}
            },
            Token::EndTag { name } if name == "frameset" => {
                if self.open.len() > 1 {
                    self.pop();
                }
                if self.cur_tag() != "frameset" {
                    self.mode = Mode::AfterBody;
                }
            }
            Token::Eof => {}
            _ => {}
        }
    }
}

thread_local! {
    static FRAG_OUT: std::cell::RefCell<Vec<NodeId>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Take the nodes produced by the last `parse_fragment` call.
pub fn take_fragment_children() -> Vec<NodeId> {
    FRAG_OUT.with(|o| std::mem::take(&mut *o.borrow_mut()))
}
