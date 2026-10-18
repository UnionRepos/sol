//! Document loading, script order, and queued navigation.
use crate::resources::raw_text;
use crate::Page;
use sol_dom::Document;
use sol_net::Response;
use sol_url::Url;

impl Page {
    /// Navigate to a URL: fetch, parse, style, lay out, run scripts.
    pub fn open(&mut self, url: &str) -> Result<(), String> {
        let u = Url::parse(url)?;
        self.open_url(&u)
    }

    pub fn open_url(&mut self, u: &Url) -> Result<(), String> {
        let resp = self.client.borrow_mut().get(u)?;
        if resp.status >= 400 {
            return Err(format!("HTTP {} {}", resp.status, resp.reason));
        }
        // A redirect can change both the directory and origin. Resolving later
        // resources against the requested URL would fetch the wrong documents.
        let final_url = resp.url.clone();
        self.load_response(&resp, &final_url)
    }

    /// Load a fetched response as a document (used by open + data URLs).
    fn load_response(&mut self, resp: &Response, url: &Url) -> Result<(), String> {
        let mime = resp.mime();
        if mime.starts_with("image/") {
            return Err("image response (use <img>)".into());
        }
        let html = resp.text();
        self.load_html(&html, url)
    }

    /// Load an HTML string (unit tests / about:blank-style loads).
    pub fn load_html(&mut self, html: &str, url: &Url) -> Result<(), String> {
        // Runtime hooks already hold this Rc. Replacing its contents preserves
        // their connection to the active document; replacing the Rc would leave
        // those hooks observing an obsolete document instead.
        *self.doc.borrow_mut() = Document::new();
        self.doc.borrow_mut().base_url = url.serialize();
        self.url = Some(url.clone());
        self.sheets.clear();
        self.images.clear();
        self.image_sizes.clear();
        self.js.rt.listeners.clear();
        self.js.rt.doc = Some(self.doc.clone());

        let parsed = sol_html::parse(html, &mut self.doc.borrow_mut());
        self.doc.borrow_mut().base_url = url.serialize();

        self.collect_sheets();
        self.load_images();
        self.restyle();
        self.relayout();

        self.title = self.doc.borrow().title().unwrap_or_default();

        // Scripts can query layout immediately and can depend on globals from
        // earlier scripts. The initial layout must exist before evaluation, and
        // network completion order must never become script execution order.
        for s in &parsed.scripts {
            let (src, inline) = {
                let d = self.doc.borrow();
                let e = d.node(*s).element();
                let src = e.map(|e| e.attr("src").map(|x| x.to_string())).flatten();
                let ty = e.map(|e| e.attr("type").unwrap_or("").to_string());
                if matches!(ty.as_deref(), Some(t) if !t.is_empty() && t != "text/javascript" && t != "module")
                {
                    continue;
                }
                (src, raw_text(&d, *s))
            };
            let code = match src {
                Some(s) => match self.fetch_text(&self.resolve(&s).ok_or("bad script src")?) {
                    Ok(t) => t,
                    Err(e) => {
                        self.log(&format!("script {}: {}", s, e));
                        continue;
                    }
                },
                None => inline,
            };
            if let Err(t) = self.js.eval(&code) {
                let msg = t.fatal.unwrap_or_else(|| t.v.to_str(&self.js.rt));
                self.log(&format!("script error: {msg}"));
            }
        }
        self.js.drain_microtasks();
        self.pump_timers(8);
        self.apply_nav();
        self.settle();
        Ok(())
    }

    /// Apply queued navigations (location.href etc.).
    pub(super) fn apply_nav(&mut self) {
        // Loading another document can evaluate scripts that enqueue more
        // locations. An owned snapshot releases the RefCell borrow before that
        // re-entry and keeps newly queued work outside this iteration.
        let pending: Vec<String> = std::mem::take(&mut *self.nav_queue.borrow_mut());
        for n in pending {
            if let Some(u) = self.resolve(&n) {
                if let Some(h) = &u.fragment {
                    // A fragment target can change :target styling without a
                    // fetch; discarding the document here would lose its state.
                    let id = h.clone();
                    let d = self.doc.borrow();
                    if let Some(n) = d.get_element_by_id(&id) {
                        self.interact.target = Some(n);
                        self.dirty = true;
                        continue;
                    }
                    drop(d);
                }
                let cur = u.serialize();
                if cur == self.url.as_ref().map(|x| x.serialize()).unwrap_or_default() {
                    continue;
                }
                self.interact.visited.insert(cur.clone());
                if let Err(e) = self.open(&cur) {
                    self.log(&format!("nav {cur}: {e}"));
                }
                return; // one nav per turn
            }
        }
    }

    /// Resolve a (possibly relative) reference against the page URL.
    pub fn resolve(&self, href: &str) -> Option<Url> {
        let href = href.trim();
        // Resource and navigation resolution must not treat executable source
        // as a fetchable document URL; script evaluation has a separate owner.
        if href.starts_with("javascript:") {
            return None;
        }
        match &self.url {
            Some(u) => u.join(href).ok(),
            None => Url::parse(href).ok(),
        }
    }
}
