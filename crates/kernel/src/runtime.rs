//! JavaScript host hooks and bounded event turns.
use crate::Page;
use sol_dom::NodeId;
use sol_url::Url;
use std::rc::Rc;

impl Page {
    /// Capturing shared handles lets callbacks survive document loads without
    /// borrowing the Page for the lifetime of the JavaScript realm.
    pub(super) fn wire_js(&mut self) {
        let client = self.client.clone();
        self.js.rt.fetcher = Some(Rc::new(
            move |url: &str, method: &str, body: Option<&str>| {
                let Ok(u) = Url::parse(url) else {
                    return Err(format!("bad url {url}"));
                };
                let mut c = client.borrow_mut();
                let req = sol_net::Request {
                    method: method.to_string(),
                    url: u,
                    headers: vec![],
                    body: body.map(|b| b.as_bytes().to_vec()).unwrap_or_default(),
                };
                match c.request(req) {
                    Ok(r) => Ok((
                        r.status,
                        r.text(),
                        r.headers
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    )),
                    Err(e) => Err(e),
                }
            },
        ));
        let rects = self.rects.clone();
        self.js.rt.layout_query = Some(Rc::new(move |n: NodeId| {
            rects
                .borrow()
                .get(&n)
                .map(|r| (r.x, r.y, r.w, r.h))
                .unwrap_or((0.0, 0.0, 0.0, 0.0))
        }));
        // Immediate navigation could replace a document while its listener or
        // script still runs. A queue postpones that replacement until control
        // returns to the kernel, after the current runtime borrow has ended.
        let nav = self.nav_queue.clone();
        self.js.rt.navigate = Some(Rc::new(move |u: &str| {
            nav.borrow_mut().push(u.to_string());
        }));
    }

    /// Mutations and pseudo-state can invalidate the same rendering snapshot.
    /// One refresh boundary keeps later paint and geometry queries consistent
    /// without rebuilding for every individual DOM operation.
    pub fn settle(&mut self) {
        // Promise callbacks may mutate the DOM before a timer observes it.
        // Draining them first preserves that ordering at the refresh boundary.
        self.js.drain_microtasks();
        self.pump_timers(4);
        if self.dirty || self.doc.borrow_mut().take_dirty() || self.animation_timeline.active {
            self.collect_sheets();
            self.refresh_rendering();
            self.dirty = false;
        }
        self.apply_nav();
    }

    /// A callback can schedule another timer indefinitely. A fixed number of
    /// rounds lets page loading and settling return instead of waiting for the
    /// timer queue to become empty; it is not a time limit on each callback.
    pub(super) fn pump_timers(&mut self, rounds: usize) {
        for _ in 0..rounds {
            if self.js.rt.timers.is_empty() {
                break;
            }
            match self.js.run_due_timers() {
                Some(_) => std::thread::sleep(std::time::Duration::from_millis(2)),
                None => break,
            }
        }
    }
}
