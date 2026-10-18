//! Runtime values, heap-allocated objects, mark-sweep GC.

use crate::ast::FuncDef;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

pub type ObjRef = usize;
/// "no prototype" sentinel
pub const NO_PROTO: ObjRef = usize::MAX;
pub type EnvRef = Rc<RefCell<Env>>;
pub type LayoutQuery = Rc<dyn Fn(sol_dom::NodeId) -> (f32, f32, f32, f32)>;
pub type Navigate = Rc<dyn Fn(&str)>;
pub type FetchResponse = (u16, String, Vec<(String, String)>);
pub type Fetcher = Rc<dyn Fn(&str, &str, Option<&str>) -> Result<FetchResponse, String>>;

#[derive(Clone, Debug)]
pub enum V {
    Undef,
    Null,
    Bool(bool),
    Num(f64),
    Str(Rc<String>),
    Obj(ObjRef),
}

impl V {
    pub fn s(s: impl Into<String>) -> V {
        V::Str(Rc::new(s.into()))
    }
    pub fn truthy(&self, rt: &Rt) -> bool {
        match self {
            V::Undef | V::Null => false,
            V::Bool(b) => *b,
            V::Num(n) => *n != 0.0 && !n.is_nan(),
            V::Str(s) => !s.is_empty(),
            V::Obj(o) => !rt.obj(*o).is_nullish_obj(),
        }
    }
    pub fn to_str(&self, rt: &Rt) -> String {
        match self {
            V::Undef => "undefined".into(),
            V::Null => "null".into(),
            V::Bool(b) => b.to_string(),
            V::Num(n) => num_str(*n),
            V::Str(s) => s.to_string(),
            V::Obj(o) => rt.obj(*o).to_string(rt),
        }
    }
    pub fn to_num(&self, rt: &Rt) -> f64 {
        match self {
            V::Undef => f64::NAN,
            V::Null => 0.0,
            V::Bool(b) => *b as u8 as f64,
            V::Num(n) => *n,
            V::Str(s) => s.trim().parse().unwrap_or(f64::NAN),
            V::Obj(o) => rt.obj(*o).to_num(rt),
        }
    }
    pub fn is_nullish(&self) -> bool {
        matches!(self, V::Undef | V::Null)
    }
    pub fn type_name(&self, rt: &Rt) -> &'static str {
        match self {
            V::Undef => "undefined",
            V::Null => "object",
            V::Bool(_) => "boolean",
            V::Num(_) => "number",
            V::Str(_) => "string",
            V::Obj(o) => match rt.obj(*o).kind {
                ObjKind::Func { .. } | ObjKind::Native { .. } | ObjKind::Class { .. } => "function",
                _ => "object",
            },
        }
    }
}

pub fn num_str(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    if n.fract() == 0.0 && n.abs() < 9e15 {
        return format!("{}", n as i64);
    }
    let s = format!("{n}");
    s
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Prop {
    Normal,
    Get,
    Set,
}

pub enum ObjKind {
    Ordinary,
    Arr(RefCell<Vec<V>>),
    Func {
        def: Rc<FuncDef>,
        env: EnvRef,
        arrow_this: Option<V>,
    },
    /// (name, fn(this, args, rt) -> Result<V, Throw>)
    Native {
        name: String,
        f: NativeFn,
    },
    Class {
        ctor: Option<Rc<FuncDef>>,
        env: EnvRef,
        methods: Vec<(String, ObjRef)>,
        proto: ObjRef,
        sup: Option<ObjRef>,
    },
    Regex {
        body: String,
        flags: String,
        last: Cell<usize>,
    },
    /// promise: 0 pending, 1 fulfilled, 2 rejected
    Promise {
        state: Cell<u8>,
        value: RefCell<V>,
        thens: RefCell<Vec<(ObjRef, ObjRef)>>,
    },
    /// DOM element wrapper
    Dom {
        node: sol_dom::NodeId,
    },
    /// event object
    Event {
        name: String,
        target: RefCell<V>,
        current: RefCell<V>,
        flags: Cell<u8>,
    },
    /// element style proxy (el.style)
    StyleObj {
        node: sol_dom::NodeId,
    },
    /// element classList proxy
    ClassList {
        node: sol_dom::NodeId,
    },
    /// document.location / window.location
    Location,
    /// localStorage / sessionStorage
    Storage,
    /// node list snapshot
    NodeList(Vec<sol_dom::NodeId>),
    /// boxed primitives for method access
    StrObj(Rc<String>),
    NumObj(f64),
    BoolObj(bool),
    Math,
    Json,
    Console,
    Err(String),
    Iter(RefCell<std::vec::IntoIter<V>>),
    MapObj(RefCell<Vec<(V, V)>>),
    SetObj(RefCell<Vec<V>>),
    DateObj(f64),
}

pub struct Obj {
    pub mark: Cell<bool>,
    pub props: RefCell<HashMap<String, V>>,
    /// props not visible to for-in / Object.keys
    pub non_enum: RefCell<std::collections::HashSet<String>>,
    /// accessor props: name -> (getter, setter)
    pub accessors: RefCell<HashMap<String, (V, V)>>,
    pub proto: Cell<ObjRef>, // 0 = none (object_ref for null-proto sentinel)
    pub kind: ObjKind,
    pub extensible: Cell<bool>,
}

impl Obj {
    pub fn new(kind: ObjKind, proto: ObjRef) -> Obj {
        Obj {
            mark: Cell::new(false),
            props: RefCell::new(HashMap::new()),
            non_enum: RefCell::new(std::collections::HashSet::new()),
            accessors: RefCell::new(HashMap::new()),
            proto: Cell::new(proto),
            kind,
            extensible: Cell::new(true),
        }
    }
    pub fn is_nullish_obj(&self) -> bool {
        false
    }
    pub fn to_string(&self, rt: &Rt) -> String {
        match &self.kind {
            ObjKind::Arr(a) => a
                .borrow()
                .iter()
                .map(|v| {
                    if v.is_nullish() {
                        String::new()
                    } else {
                        v.to_str(rt)
                    }
                })
                .collect::<Vec<_>>()
                .join(","),
            ObjKind::Func { def, .. } => format!(
                "function {}() {{ [code] }}",
                def.name.clone().unwrap_or_default()
            ),
            ObjKind::Native { name, .. } => format!("function {name}() {{ [native code] }}"),
            ObjKind::StrObj(s) => s.to_string(),
            ObjKind::NumObj(n) => num_str(*n),
            ObjKind::BoolObj(b) => b.to_string(),
            ObjKind::Err(m) => format!("Error: {m}"),
            ObjKind::DateObj(t) => format!("Date({})", num_str(*t)),
            ObjKind::Dom { node } => format!("[object HTMLElement#{:?}]", node),
            ObjKind::StyleObj { .. } => "[object CSSStyleDeclaration]".into(),
            ObjKind::ClassList { .. } => "[object DOMTokenList]".into(),
            ObjKind::Location => "[object Location]".into(),
            ObjKind::Storage => "[object Storage]".into(),
            ObjKind::NodeList(_) => "[object NodeList]".into(),
            ObjKind::Regex { body, flags, .. } => format!("/{body}/{flags}"),
            ObjKind::Class { .. } => "class {}".into(),
            ObjKind::MapObj(_) => "[object Map]".into(),
            ObjKind::SetObj(_) => "[object Set]".into(),
            _ => "[object Object]".into(),
        }
    }
    pub fn to_num(&self, rt: &Rt) -> f64 {
        match &self.kind {
            ObjKind::StrObj(s) => s.trim().parse().unwrap_or(f64::NAN),
            ObjKind::NumObj(n) => *n,
            ObjKind::BoolObj(b) => *b as u8 as f64,
            ObjKind::Arr(a) => {
                let a = a.borrow();
                if a.len() == 1 {
                    a[0].to_num(rt)
                } else {
                    f64::NAN
                }
            }
            _ => f64::NAN,
        }
    }
}

pub type NativeFn = Rc<dyn Fn(&mut crate::eval::I, V, &[V]) -> Result<V, Throw>>;

/// A thrown JS value (or fatal error).
pub struct Throw {
    pub v: V,
    pub fatal: Option<String>,
}

impl std::fmt::Debug for Throw {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.fatal {
            Some(m) => write!(f, "Throw(fatal: {m})"),
            None => write!(f, "Throw(val)"),
        }
    }
}

impl Throw {
    pub fn fatal(msg: impl Into<String>) -> Throw {
        Throw {
            v: V::Undef,
            fatal: Some(msg.into()),
        }
    }
    pub fn val(v: V) -> Throw {
        Throw { v, fatal: None }
    }
}

/// Lexical environment for function scopes.
pub struct Env {
    pub vars: RefCell<HashMap<String, V>>,
    pub parent: Option<EnvRef>,
    /// this + function object for the frame
    pub this: RefCell<V>,
    pub func: RefCell<Option<ObjRef>>,
    pub new_target: RefCell<V>,
}

impl Env {
    pub fn new(parent: Option<EnvRef>) -> EnvRef {
        Rc::new(RefCell::new(Env {
            vars: RefCell::new(HashMap::new()),
            parent,
            this: RefCell::new(V::Undef),
            func: RefCell::new(None),
            new_target: RefCell::new(V::Undef),
        }))
    }
    pub fn get(env: &EnvRef, name: &str) -> Option<V> {
        let mut cur = Some(env.clone());
        while let Some(e) = cur {
            if let Some(v) = e.borrow().vars.borrow().get(name) {
                return Some(v.clone());
            }
            cur = e.borrow().parent.clone();
        }
        None
    }
    pub fn set(env: &EnvRef, name: &str, v: V) -> bool {
        let mut cur = Some(env.clone());
        while let Some(e) = cur {
            if e.borrow().vars.borrow().contains_key(name) {
                e.borrow().vars.borrow_mut().insert(name.to_string(), v);
                return true;
            }
            cur = e.borrow().parent.clone();
        }
        false
    }
    pub fn declare(env: &EnvRef, name: &str, v: V) {
        env.borrow().vars.borrow_mut().insert(name.to_string(), v);
    }
}

/// The runtime: object heap + roots + queues.
pub struct Rt {
    pub heap: Vec<Option<Box<Obj>>>,
    pub free: Vec<usize>,
    /// object proto
    pub obj_proto: ObjRef,
    pub arr_proto: ObjRef,
    pub func_proto: ObjRef,
    pub str_proto: ObjRef,
    pub num_proto: ObjRef,
    pub bool_proto: ObjRef,
    pub err_proto: ObjRef,
    pub re_proto: ObjRef,
    /// lazily-installed protos for builtin object kinds
    pub promise_proto: ObjRef,
    pub map_proto: ObjRef,
    pub set_proto: ObjRef,
    pub date_proto: ObjRef,
    pub iter_proto: ObjRef,
    pub dom_proto: ObjRef,
    pub node_proto: ObjRef,
    pub global_env: EnvRef,
    /// the `globalThis` object (props mirror global env vars)
    pub global_obj: ObjRef,
    /// microtask queue: callable ObjRefs
    pub microtasks: std::collections::VecDeque<V>,
    /// timers: (deadline_ms, callback, id)
    pub timers: Vec<(u64, V, u32)>,
    pub next_timer: u32,
    /// console output sink
    pub console_out: Rc<RefCell<Vec<String>>>,
    /// DOM document handle (set by kernel)
    pub doc: Option<Rc<RefCell<sol_dom::Document>>>,
    /// event listeners: (node, type, callback obj)
    pub listeners: Vec<(sol_dom::NodeId, String, ObjRef)>,
    /// localStorage backing
    pub storage: RefCell<HashMap<String, String>>,
    /// cookie jar for document.cookie
    pub cookie: RefCell<String>,
    /// viewport size for window.innerWidth/Height
    pub viewport: (u32, u32),
    /// layout query hook: node -> (x,y,w,h); set by kernel
    pub layout_query: Option<LayoutQuery>,
    /// navigation hook (location.href = ...)
    pub navigate: Option<Navigate>,
    /// fetch hook for XHR/fetch (set by kernel): (url, method, body) -> (status, body, headers)
    pub fetcher: Option<Fetcher>,
    pub allocs: usize,
    pub gc_threshold: usize,
    /// currently-executing roots beyond env: pinned ObjRefs
    pub pinned: Vec<ObjRef>,
    /// event loop turn: script stack depth
    pub depth: usize,
}

impl Default for Rt {
    fn default() -> Self {
        Self::new()
    }
}

impl Rt {
    pub fn new() -> Rt {
        let mut rt = Rt {
            heap: Vec::with_capacity(1024),
            free: Vec::new(),
            obj_proto: 0,
            arr_proto: 0,
            func_proto: 0,
            str_proto: 0,
            num_proto: 0,
            bool_proto: 0,
            err_proto: 0,
            re_proto: 0,
            promise_proto: 0,
            map_proto: 0,
            set_proto: 0,
            date_proto: 0,
            iter_proto: 0,
            dom_proto: 0,
            node_proto: 0,
            global_env: Env::new(None),
            global_obj: NO_PROTO,
            microtasks: std::collections::VecDeque::new(),
            timers: Vec::new(),
            next_timer: 1,
            console_out: Rc::new(RefCell::new(Vec::new())),
            doc: None,
            listeners: Vec::new(),
            storage: RefCell::new(HashMap::new()),
            cookie: RefCell::new(String::new()),
            viewport: (1024, 768),
            layout_query: None,
            navigate: None,
            fetcher: None,
            allocs: 0,
            gc_threshold: 100_000,
            pinned: Vec::new(),
            depth: 0,
        };
        rt.obj_proto = rt.alloc(Obj::new(ObjKind::Ordinary, NO_PROTO));
        rt.arr_proto = rt.alloc(Obj::new(ObjKind::Ordinary, rt.obj_proto));
        rt.func_proto = rt.alloc(Obj::new(ObjKind::Ordinary, rt.obj_proto));
        rt.str_proto = rt.alloc(Obj::new(ObjKind::Ordinary, rt.obj_proto));
        rt.num_proto = rt.alloc(Obj::new(ObjKind::Ordinary, rt.obj_proto));
        rt.bool_proto = rt.alloc(Obj::new(ObjKind::Ordinary, rt.obj_proto));
        rt.err_proto = rt.alloc(Obj::new(ObjKind::Ordinary, rt.obj_proto));
        rt.re_proto = rt.alloc(Obj::new(ObjKind::Ordinary, rt.obj_proto));
        rt.promise_proto = rt.alloc(Obj::new(ObjKind::Ordinary, rt.obj_proto));
        rt.map_proto = rt.alloc(Obj::new(ObjKind::Ordinary, rt.obj_proto));
        rt.set_proto = rt.alloc(Obj::new(ObjKind::Ordinary, rt.obj_proto));
        rt.date_proto = rt.alloc(Obj::new(ObjKind::Ordinary, rt.obj_proto));
        rt.iter_proto = rt.alloc(Obj::new(ObjKind::Ordinary, rt.obj_proto));
        rt.dom_proto = rt.alloc(Obj::new(ObjKind::Ordinary, rt.obj_proto));
        rt.node_proto = rt.alloc(Obj::new(ObjKind::Ordinary, rt.obj_proto));
        rt
    }

    pub fn alloc(&mut self, o: Obj) -> ObjRef {
        self.allocs += 1;
        if let Some(i) = self.free.pop() {
            self.heap[i] = Some(Box::new(o));
            i
        } else {
            self.heap.push(Some(Box::new(o)));
            self.heap.len() - 1
        }
    }

    pub fn obj(&self, r: ObjRef) -> &Obj {
        self.heap[r].as_ref().expect("use-after-free obj")
    }
    pub fn obj_mut(&mut self, r: ObjRef) -> &mut Obj {
        self.heap[r].as_mut().expect("use-after-free obj")
    }

    /// Get property with proto chain walk. Accessors are returned via
    /// `get_value` instead; this raw getter ignores getters.
    pub fn get_raw(&self, r: ObjRef, name: &str) -> Option<V> {
        let mut cur = r;
        loop {
            let o = self.obj(cur);
            if let Some(v) = o.props.borrow().get(name) {
                return Some(v.clone());
            }
            let p = o.proto.get();
            if p == NO_PROTO {
                return None;
            }
            cur = p;
        }
    }

    pub fn set(&mut self, r: ObjRef, name: &str, v: V) {
        let o = self.obj(r);
        o.non_enum.borrow_mut().remove(name);
        o.props.borrow_mut().insert(name.to_string(), v);
    }

    /// Set a non-enumerable own property (builtins, methods).
    pub fn set_ne(&mut self, r: ObjRef, name: &str, v: V) {
        let o = self.obj(r);
        o.non_enum.borrow_mut().insert(name.to_string());
        o.props.borrow_mut().insert(name.to_string(), v);
    }

    pub fn own_prop(&self, r: ObjRef, name: &str) -> Option<V> {
        self.obj(r).props.borrow().get(name).cloned()
    }

    /// mark-sweep collection; roots = protos + global env + pins + queues
    pub fn gc(&mut self, env_stack: &[EnvRef]) {
        // mark
        let mut work = vec![
            self.obj_proto,
            self.arr_proto,
            self.func_proto,
            self.str_proto,
            self.num_proto,
            self.bool_proto,
            self.err_proto,
            self.re_proto,
            self.promise_proto,
            self.map_proto,
            self.set_proto,
            self.date_proto,
            self.iter_proto,
            self.dom_proto,
            self.node_proto,
        ];
        work.extend(self.pinned.iter().copied());
        for v in &self.microtasks {
            if let V::Obj(o) = v {
                work.push(*o);
            }
        }
        for (_, v, _) in &self.timers {
            if let V::Obj(o) = v {
                work.push(*o);
            }
        }
        for (_, _, cb) in &self.listeners {
            work.push(*cb);
        }
        // env roots: vars + this
        let mut envs: Vec<EnvRef> = vec![self.global_env.clone()];
        envs.extend(env_stack.iter().cloned());
        let mut seen_env = std::collections::HashSet::new();
        while let Some(e) = envs.pop() {
            let addr = Rc::as_ptr(&e) as usize;
            if !seen_env.insert(addr) {
                continue;
            }
            let e = e.borrow();
            for v in e.vars.borrow().values() {
                if let V::Obj(o) = v {
                    work.push(*o);
                }
            }
            if let V::Obj(o) = *e.this.borrow() {
                work.push(o);
            }
            if let Some(f) = *e.func.borrow() {
                work.push(f);
            }
            if let V::Obj(o) = *e.new_target.borrow() {
                work.push(o);
            }
            if let Some(p) = &e.parent {
                envs.push(p.clone());
            }
        }
        while let Some(r) = work.pop() {
            if r == 0 && work.is_empty() && false {
                continue;
            }
            if r >= self.heap.len() {
                continue;
            }
            let Some(o) = &self.heap[r] else {
                continue;
            };
            if o.mark.get() {
                continue;
            }
            o.mark.set(true);
            for v in o.props.borrow().values() {
                if let V::Obj(p) = v {
                    work.push(*p);
                }
            }
            for (a, b) in o.accessors.borrow().values() {
                if let V::Obj(p) = a {
                    work.push(*p);
                }
                if let V::Obj(p) = b {
                    work.push(*p);
                }
            }
            if o.proto.get() != NO_PROTO {
                work.push(o.proto.get());
            }
            match &o.kind {
                ObjKind::Arr(a) => {
                    for v in a.borrow().iter() {
                        if let V::Obj(p) = v {
                            work.push(*p);
                        }
                    }
                }
                ObjKind::Func {
                    env, arrow_this, ..
                } => {
                    envs.push(env.clone());
                    if let Some(V::Obj(p)) = arrow_this {
                        work.push(*p);
                    }
                }
                ObjKind::Class {
                    env,
                    methods,
                    proto,
                    ..
                } => {
                    envs.push(env.clone());
                    for (_, m) in methods {
                        work.push(*m);
                    }
                    work.push(*proto);
                }
                ObjKind::Promise { value, thens, .. } => {
                    if let V::Obj(p) = *value.borrow() {
                        work.push(p);
                    }
                    for (a, b) in thens.borrow().iter() {
                        work.push(*a);
                        work.push(*b);
                    }
                }
                ObjKind::Dom { .. }
                | ObjKind::StyleObj { .. }
                | ObjKind::ClassList { .. }
                | ObjKind::Location
                | ObjKind::Storage
                | ObjKind::NodeList(_) => {}
                ObjKind::Event {
                    target, current, ..
                } => {
                    if let V::Obj(p) = *target.borrow() {
                        work.push(p);
                    }
                    if let V::Obj(p) = *current.borrow() {
                        work.push(p);
                    }
                }
                ObjKind::Iter(it) => {
                    for v in it.borrow().as_slice() {
                        if let V::Obj(p) = v {
                            work.push(*p);
                        }
                    }
                }
                ObjKind::MapObj(m) => {
                    for (k, v) in m.borrow().iter() {
                        if let V::Obj(p) = k {
                            work.push(*p);
                        }
                        if let V::Obj(p) = v {
                            work.push(*p);
                        }
                    }
                }
                ObjKind::SetObj(s) => {
                    for v in s.borrow().iter() {
                        if let V::Obj(p) = v {
                            work.push(*p);
                        }
                    }
                }
                _ => {}
            }
        }
        // sweep
        for (i, slot) in self.heap.iter_mut().enumerate() {
            if let Some(o) = slot {
                if o.mark.get() {
                    o.mark.set(false);
                } else {
                    *slot = None;
                    self.free.push(i);
                }
            }
        }
        self.allocs = 0;
    }
}
