//! Stylesheet and image loading with ordering, deduplication, and budgets.
use crate::Page;
use sol_dom::{Document, NodeData, NodeId};
use sol_url::Url;
use std::collections::HashMap;
use std::rc::Rc;

impl Page {
    /// Fetch a text resource (css/script).
    pub(super) fn fetch_text(&self, u: &Url) -> Result<String, String> {
        let r = self.client.borrow_mut().get(u)?;
        if r.status >= 400 {
            return Err(format!("HTTP {}", r.status));
        }
        Ok(r.text())
    }

    /// Cascade precedence depends on document order, not request latency.
    /// Separate source positions let linked sheets load concurrently without
    /// allowing a slow earlier sheet to override a later inline rule.
    pub(super) fn collect_sheets(&mut self) {
        #[derive(Hash)]
        enum Source {
            Inline(String),
            Link(usize),
        }
        let mut sources = Vec::new();
        let mut urls = Vec::new();
        let mut media = Vec::new();
        {
            let d = self.doc.borrow();
            for n in d.descendants(d.root) {
                let Some(e) = d.node(n).element() else {
                    continue;
                };
                if e.attr("disabled").is_some() {
                    continue;
                }
                let count = sources.len();
                match &*e.tag {
                    "style" => sources.push(Source::Inline(raw_text(&d, n))),
                    "link"
                        if e.attr("rel")
                            .unwrap_or("")
                            .split_ascii_whitespace()
                            .any(|rel| rel.eq_ignore_ascii_case("stylesheet")) =>
                    {
                        if let Some(url) = e.attr("href").and_then(|href| self.resolve(href)) {
                            sources.push(Source::Link(urls.len()));
                            urls.push(url);
                        }
                    }
                    _ => {}
                }
                if sources.len() != count {
                    media.push(e.attr("media").unwrap_or("").to_string());
                }
            }
        }
        use std::hash::{Hash, Hasher};
        let mut fingerprint = std::collections::hash_map::DefaultHasher::new();
        sources.hash(&mut fingerprint);
        media.hash(&mut fingerprint);
        for url in &urls {
            url.serialize().hash(&mut fingerprint);
        }
        self.js.rt.viewport.hash(&mut fingerprint);
        let fingerprint = fingerprint.finish();
        if self.stylesheet_fingerprint == Some(fingerprint) {
            return;
        }
        self.stylesheet_fingerprint = Some(fingerprint);
        self.sheets.clear();
        self.fonts = sol_text::Fonts::discover();
        sol_css::style::viewport::WIDTH.set(self.js.rt.viewport.0 as f64);
        sol_css::style::viewport::HEIGHT.set(self.js.rt.viewport.1 as f64);
        let mut results = self
            .client
            .borrow_mut()
            .get_many(&urls, self.resource_parallelism)
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        // Keeping the positions separate also preserves repeated <link> entries:
        // fetch deduplication must not erase a stylesheet's cascade position.
        let mut retained_css = 0usize;
        let mut budget = crate::stylesheets::Budget::default();
        for (source, media) in sources.into_iter().zip(media) {
            let (text, base) = match source {
                Source::Inline(text) => (text, self.url.clone().unwrap()),
                Source::Link(index) => match results[index].take().unwrap() {
                    Ok(response) if response.status < 400 => (response.text(), response.url),
                    Ok(response) => {
                        self.log(&format!(
                            "stylesheet {}: HTTP {}",
                            urls[index], response.status
                        ));
                        continue;
                    }
                    Err(error) => {
                        self.log(&format!("stylesheet {}: {error}", urls[index]));
                        continue;
                    }
                },
            };
            let mut stack = std::collections::HashSet::from([base.serialize()]);
            let mut text = self.stylesheet_text(&text, &base, &mut budget, &mut stack);
            if !media.trim().is_empty() {
                text = format!("@media {media}{{{text}}}");
            }
            if text.len() > (16 * 1024 * 1024usize).saturating_sub(retained_css + 1) {
                self.log("stylesheet budget exceeded");
                continue;
            }
            retained_css += text.len() + 1;
            if !text.trim().is_empty() {
                self.sheets.push(sol_css::parser::parse(&text));
            }
        }
        self.load_css_fonts();
    }

    /// Repeated image elements should not multiply decoding cost or pixel
    /// storage. Sharing decoded images also lets the page budget charge each
    /// unique allocation once rather than once per DOM consumer.
    pub(super) fn load_images(&mut self) {
        let mut jobs: Vec<(Url, Vec<NodeId>)> = Vec::new();
        let mut lookup = HashMap::new();
        {
            let d = self.doc.borrow();
            for n in d.descendants(d.root) {
                let Some(e) = d.node(n).element() else {
                    continue;
                };
                if &*e.tag != "img" {
                    continue;
                }
                let Some(mut url) = e.attr("src").and_then(|src| self.resolve(src)) else {
                    continue;
                };
                url.fragment = None;
                let key = url.serialize();
                let index = *lookup.entry(key).or_insert_with(|| {
                    jobs.push((url, Vec::new()));
                    jobs.len() - 1
                });
                jobs[index].1.push(n);
            }
        }
        let mut decoded_bytes = 0usize;
        // Retaining every encoded response until the entire document finishes
        // fetching would make memory usage grow with image count even when the
        // decoded-pixel budget rejects most of those images.
        for chunk in jobs.chunks(32) {
            let urls: Vec<_> = chunk.iter().map(|(url, _)| url.clone()).collect();
            let responses = self
                .client
                .borrow_mut()
                .get_many(&urls, self.resource_parallelism);
            for ((url, nodes), response) in chunk.iter().zip(responses) {
                let decoded = response.and_then(|response| {
                    if response.status >= 400 {
                        return Err(format!("HTTP {}", response.status));
                    }
                    sol_img::decode(&response.body)
                });
                match decoded {
                    Ok(image) if image.px.len() <= 64 * 1024 * 1024 - decoded_bytes => {
                        decoded_bytes += image.px.len();
                        let image = Rc::new(image);
                        for &node in nodes {
                            self.image_sizes.insert(node, (image.w, image.h));
                            self.images.insert(node, image.clone());
                        }
                    }
                    Ok(_) => self.log(&format!("img {url}: decoded image budget exceeded")),
                    Err(error) => self.log(&format!("img {url}: {error}")),
                }
            }
        }
    }
}

/// The document's text helper excludes script/style subtrees. Source extraction
/// must bypass that presentation-oriented filtering or inline programs and
/// stylesheets would appear empty to their respective parsers.
pub(super) fn raw_text(d: &Document, n: NodeId) -> String {
    let mut s = String::new();
    let mut stack: Vec<NodeId> = d.node(n).children.iter().rev().copied().collect();
    while let Some(c) = stack.pop() {
        match &d.node(c).data {
            NodeData::Text(t) => s.push_str(t),
            _ => stack.extend(d.node(c).children.iter().rev().copied()),
        }
    }
    s
}

impl Page {
    pub(super) fn load_backgrounds(&mut self) {
        let mut urls = std::collections::HashSet::new();
        for style in self.styles.values().flat_map(|style| {
            std::iter::once(style.as_ref()).chain(style.pseudos.values().map(Box::as_ref))
        }) {
            for value in style.get("background-image").into_iter().flatten() {
                if let sol_css::Value::Url(url) = value {
                    if !self.background_images.contains_key(url) {
                        urls.insert(url.clone());
                    }
                }
            }
        }
        let mut urls: Vec<_> = urls.into_iter().collect();
        urls.sort();
        let mut seen = std::collections::HashSet::new();
        let mut retained: usize = self
            .images
            .values()
            .chain(self.background_images.values())
            .filter(|image| seen.insert(Rc::as_ptr(image)))
            .map(|image| image.px.len())
            .sum();
        for chunk in urls.chunks(32) {
            let requests: Vec<_> = chunk
                .iter()
                .filter_map(|url| self.resolve(url).map(|resolved| (url.clone(), resolved)))
                .collect();
            let responses = self.client.borrow_mut().get_many(
                &requests
                    .iter()
                    .map(|(_, url)| url.clone())
                    .collect::<Vec<_>>(),
                self.resource_parallelism,
            );
            for ((key, url), response) in requests.into_iter().zip(responses) {
                let image = response.and_then(|response| {
                    if response.status >= 400 {
                        Err(format!("HTTP {}", response.status))
                    } else {
                        sol_img::decode(&response.body)
                    }
                });
                match image {
                    Ok(image)
                        if image.px.len() <= (64 * 1024 * 1024usize).saturating_sub(retained) =>
                    {
                        retained += image.px.len();
                        self.background_images.insert(key, Rc::new(image));
                    }
                    Ok(_) => self.log(&format!("background {url}: decoded image budget exceeded")),
                    Err(error) => self.log(&format!("background {url}: {error}")),
                }
            }
        }
    }
}
