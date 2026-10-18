//! sol_kernel — page orchestration.
//!
//! Central ownership keeps the DOM, JS realm, and rendering caches in agreement
//! while private modules can evolve without exposing pipeline internals.
//!
//! Location changes are deferred so a script cannot replace the document while
//! its event callback is using it. Fetch hooks remain synchronous; queuing a
//! navigation does not make JavaScript or network access nonblocking.

mod interaction;
mod navigation;
mod rendering;
mod resources;
mod runtime;
mod stylesheets;

pub use interaction::Interact;

use sol_css::parser::Stylesheet;
use sol_css::Style;
use sol_dom::{Document, NodeId};
use sol_img::Image;
use sol_js::Js;
use sol_layout::Out;
use sol_net::Client;
use sol_paint::DisplayList;
use sol_text::Fonts;
use sol_url::Url;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A loaded web page: document + JS realm + style/layout/paint caches.
pub struct Page {
    pub doc: Rc<RefCell<Document>>,
    pub js: Js,
    pub url: Option<Url>,
    pub title: String,
    // Host callbacks outlive the borrow used to install them. Shared handles
    // let those callbacks reach current page state without retaining `&mut Page`
    // or duplicating the cookie jar, cache, and geometry snapshot.
    client: Rc<RefCell<Client>>,
    fonts: Fonts,
    ua: Stylesheet,
    sheets: Vec<Stylesheet>,
    stylesheet_fingerprint: Option<u64>,
    pub styles: HashMap<NodeId, Rc<Style>>,
    images: HashMap<NodeId, Rc<Image>>,
    image_sizes: HashMap<NodeId, (u32, u32)>,
    background_images: HashMap<String, Rc<Image>>,
    pub layout: Option<Out>,
    pub dl: DisplayList,
    rects: Rc<RefCell<HashMap<NodeId, sol_layout::Rect>>>,
    nav_queue: Rc<RefCell<Vec<String>>>,
    dirty: bool,
    animation_timeline: sol_css::animations::Timeline,
    animation_epoch: std::time::Instant,
    animation_time: Option<f64>,
    animation_initialized: bool,
    container_boxes: HashMap<NodeId, sol_css::containers::Size>,
    pub interact: Interact,
    pub console: Rc<RefCell<Vec<String>>>,
    /// Concurrent independent subresource requests, clamped to 1..8.
    pub resource_parallelism: usize,
}

impl Page {
    /// Empty page about:blank.
    pub fn new(vw: u32, vh: u32) -> Page {
        let doc = Rc::new(RefCell::new(Document::new()));
        let mut js = Js::new();
        js.rt.viewport = (vw, vh);
        let console = js.console();
        let mut p = Page {
            doc,
            js,
            url: None,
            title: String::new(),
            client: Rc::new(RefCell::new(Client::new())),
            fonts: Fonts::discover(),
            ua: sol_css::parser::parse(sol_css::UA_CSS),
            sheets: Vec::new(),
            stylesheet_fingerprint: None,
            styles: HashMap::new(),
            images: HashMap::new(),
            image_sizes: HashMap::new(),
            background_images: HashMap::new(),
            layout: None,
            dl: DisplayList::default(),
            rects: Rc::new(RefCell::new(HashMap::new())),
            nav_queue: Rc::new(RefCell::new(Vec::new())),
            dirty: true,
            animation_timeline: sol_css::animations::Timeline::default(),
            animation_epoch: std::time::Instant::now(),
            animation_time: None,
            animation_initialized: false,
            container_boxes: HashMap::new(),
            interact: Interact::default(),
            console,
            resource_parallelism: 4,
        };
        p.wire_js();
        p.js.set_document(p.doc.clone());
        p
    }

    /// Configure networking or inspect cache statistics for this page.
    pub fn network_client(&self) -> std::cell::RefMut<'_, Client> {
        self.client.borrow_mut()
    }

    /// console.log lines accumulated since load.
    pub fn console_lines(&self) -> Vec<String> {
        self.console.borrow().clone()
    }

    fn log(&self, m: &str) {
        self.console.borrow_mut().push(m.to_string());
    }

    /// Serialize current DOM (debug helper).
    pub fn outer_html(&self) -> String {
        {
            let d = self.doc.borrow();
            let r = d.root;
            d.outer_html(r)
        }
    }
}
