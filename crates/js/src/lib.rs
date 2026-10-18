//! sol_js — self-contained ECMAScript subset engine.
//!
//! Tree-walk interpreter with mark-sweep GC, closures, classes,
//! promises + microtask queue, timers, regex engine, and DOM bindings.
//! Zero dependencies outside sol_* crates + std.

pub mod ast;
pub mod builtins;
pub mod dom_api;
pub mod eval;
pub mod lex;
pub mod regex;
pub mod rt;

pub use eval::I;
pub use rt::{ObjKind, Rt, Throw, V};

use std::cell::RefCell;
use std::rc::Rc;

/// A JS realm: heap + globals + builtins, ready to eval scripts.
pub struct Js {
    pub rt: Rt,
}

impl Js {
    /// New realm with standard builtins installed.
    pub fn new() -> Js {
        let mut rt = Rt::new();
        {
            let mut i = I::new(&mut rt);
            builtins::install(&mut i);
            dom_api::install(&mut i);
        }
        Js { rt }
    }

    /// Attach a live DOM document (enables document.* APIs).
    pub fn set_document(&mut self, doc: Rc<RefCell<sol_dom::Document>>) {
        self.rt.doc = Some(doc);
        let mut i = I::new(&mut self.rt);
        dom_api::refresh_globals(&mut i);
    }

    /// Parse + run a script at global scope. Returns last value.
    pub fn eval(&mut self, src: &str) -> Result<V, Throw> {
        let prog = ast::parse(src).map_err(|e| Throw::fatal(format!("SyntaxError: {e}")))?;
        let v = {
            let mut i = I::new(&mut self.rt);
            i.run(&prog)
        };
        self.drain_microtasks();
        v
    }

    /// Parse only (syntax check).
    pub fn parse(src: &str) -> Result<(), String> {
        ast::parse(src).map(|_| ())
    }

    /// Run pending microtasks (promise callbacks, queueMicrotask).
    pub fn drain_microtasks(&mut self) {
        let mut guard = 0usize;
        loop {
            let task = self.rt.microtasks.pop_front();
            let Some(task) = task else { break };
            guard += 1;
            // A promise callback can enqueue another callback forever. Bounding
            // queue draining prevents that pattern from monopolizing a host
            // turn, although an individual callback can still run indefinitely.
            if guard > 100_000 {
                break;
            }
            let mut i = I::new(&mut self.rt);
            let _ = i.call(task, V::Undef, vec![]);
        }
    }

    /// Fire due timers; returns ms until next pending timer (if any).
    pub fn run_due_timers(&mut self) -> Option<u64> {
        let now = sol_util::now_ms();
        // Timer callbacks may register more timers. A snapshot avoids mutating
        // the collection being traversed and defers newly scheduled callbacks
        // until a later host round rather than extending this one indefinitely.
        let (due, pending): (Vec<_>, Vec<_>) = std::mem::take(&mut self.rt.timers)
            .into_iter()
            .partition(|(t, _, _)| *t <= now);
        self.rt.timers = pending;
        for (_, cb, _) in due {
            let mut i = I::new(&mut self.rt);
            let _ = i.call(cb, V::Undef, vec![]);
        }
        self.drain_microtasks();
        self.rt
            .timers
            .iter()
            .map(|(t, _, _)| t.saturating_sub(sol_util::now_ms()))
            .min()
    }

    /// Call a JS function value.
    pub fn call(&mut self, f: V, this: V, args: Vec<V>) -> Result<V, Throw> {
        let mut i = I::new(&mut self.rt);
        i.call(f, this, args)
    }

    /// Fire a DOM event at `node` (bubbles up to the root).
    pub fn fire_event(&mut self, node: sol_dom::NodeId, ty: &str) {
        let mut i = I::new(&mut self.rt);
        dom_api::dispatch(&mut i, node, ty);
        self.drain_microtasks();
    }

    /// Console output lines accumulated by console.log etc.
    pub fn console(&self) -> Rc<RefCell<Vec<String>>> {
        self.rt.console_out.clone()
    }

    /// Force a GC pass.
    pub fn gc(&mut self) {
        let global_env = self.rt.global_env.clone();
        self.rt.gc(std::slice::from_ref(&global_env));
    }
}

impl Default for Js {
    fn default() -> Self {
        Self::new()
    }
}
