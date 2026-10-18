//! Tree-walk interpreter.

use crate::ast::{ClassDef, FuncDef, MethodKind, E, S, TE};
use crate::rt::{Env, EnvRef, NativeFn, Obj, ObjKind, Rt, Throw, V};
use std::cell::RefCell;
use std::rc::Rc;

pub type Res<T> = Result<T, Throw>;

/// Control-flow signal inside a frame.
pub enum Flow {
    Break(Option<String>),
    Continue(Option<String>),
    Return(V),
}

fn err(rt: &mut Rt, msg: impl Into<String>) -> Throw {
    let m = msg.into();
    let o = rt.alloc(Obj::new(ObjKind::Err(m.clone()), rt.err_proto));
    rt.set(o, "message", V::s(m));
    Throw::val(V::Obj(o))
}

pub fn fatal(msg: impl Into<String>) -> Throw {
    Throw::fatal(msg)
}

/// Reference for assignment targets.
enum Ref2 {
    Var(String, EnvRef),
    Prop(V, V), // obj value, key value
}

pub struct I<'a> {
    pub rt: &'a mut Rt,
    pub env_stack: Vec<EnvRef>,
    pub steps: usize,
}

impl<'a> I<'a> {
    pub fn new(rt: &'a mut Rt) -> I<'a> {
        let env = rt.global_env.clone();
        I {
            rt,
            env_stack: vec![env],
            steps: 0,
        }
    }

    pub fn env(&self) -> EnvRef {
        self.env_stack.last().unwrap().clone()
    }

    fn tick(&mut self) -> Res<()> {
        self.steps += 1;
        if self.steps.is_multiple_of(100_000) {
            if self.rt.allocs > self.rt.gc_threshold {
                self.rt.gc(&self.env_stack.clone());
            }
            if self.steps > 20_000_000 {
                return Err(fatal("script step limit exceeded"));
            }
        }
        Ok(())
    }

    pub fn run(&mut self, body: &[S]) -> Res<V> {
        match self.exec_block(body, self.env())? {
            Ok2::V(v) | Ok2::Flow(Flow::Return(v)) => Ok(v),
            _ => Ok(V::Undef),
        }
    }

    /// Execute statements; returns last expr value.
    pub fn exec_block(&mut self, body: &[S], env: EnvRef) -> Res<Ok2> {
        self.env_stack.push(env);
        let res = self.exec_inner(body);
        self.env_stack.pop();
        res
    }

    fn exec_inner(&mut self, body: &[S]) -> Res<Ok2> {
        // hoist function declarations
        for s in body {
            if let S::FuncDecl(f) = s {
                let fo = self.mk_func(f);
                let name = f.name.clone().unwrap_or_default();
                if !name.is_empty() {
                    Env::declare(&self.env(), &name, fo);
                }
            }
        }
        let mut last = V::Undef;
        for s in body {
            self.tick()?;
            match self.stmt(s)? {
                Ok2::V(v) => last = v,
                Ok2::Flow(f) => return Ok(Ok2::Flow(f)),
            }
        }
        Ok(Ok2::V(last))
    }

    fn stmt(&mut self, s: &S) -> Res<Ok2> {
        match s {
            S::Expr(e) => Ok(Ok2::V(self.eval(e)?)),
            S::Var(decls, _kind) => {
                let mut last = V::Undef;
                for (name, init) in decls {
                    let v = match init {
                        Some(e) => self.eval(e)?,
                        None => V::Undef,
                    };
                    Env::declare(&self.env(), name, v.clone());
                    last = v;
                }
                Ok(Ok2::V(last))
            }
            S::FuncDecl(_) => Ok(Ok2::V(V::Undef)),
            S::ClassDecl(c) => {
                let co = self.mk_class(c)?;
                if let Some(n) = &c.name {
                    Env::declare(&self.env(), n, co.clone());
                }
                Ok(Ok2::V(co))
            }
            S::If(c, then, els) => {
                if self.eval(c)?.truthy(self.rt) {
                    self.exec_block(then, Env::new(Some(self.env())))
                } else {
                    self.exec_block(els, Env::new(Some(self.env())))
                }
            }
            S::Block(b) => self.exec_block(b, Env::new(Some(self.env()))),
            S::For(init, cond, step, body) => {
                let env = Env::new(Some(self.env()));
                self.env_stack.push(env.clone());
                let res = (|i2: &mut Self| -> Res<Ok2> {
                    if let Some(s0) = init {
                        i2.stmt(s0)?;
                    }
                    loop {
                        if let Some(c) = cond {
                            if !i2.eval(c)?.truthy(i2.rt) {
                                break;
                            }
                        }
                        match i2.exec_block(body, Env::new(Some(env.clone())))? {
                            Ok2::V(_) => {}
                            Ok2::Flow(Flow::Break(None)) => break,
                            Ok2::Flow(Flow::Break(l)) => {
                                return Ok(Ok2::Flow(Flow::Break(l)));
                            }
                            Ok2::Flow(Flow::Return(v)) => {
                                return Ok(Ok2::Flow(Flow::Return(v)));
                            }
                            Ok2::Flow(Flow::Continue(None)) => {}
                            Ok2::Flow(Flow::Continue(l)) => {
                                return Ok(Ok2::Flow(Flow::Continue(l)));
                            }
                        }
                        if let Some(st) = step {
                            i2.eval(st)?;
                        }
                    }
                    Ok(Ok2::V(V::Undef))
                })(self);
                self.env_stack.pop();
                res
            }
            S::ForIn(lhs, rhs, body) | S::ForOf(lhs, rhs, body) => {
                let of = matches!(s, S::ForOf(..));
                let rv = self.eval(rhs)?;
                let items = if of {
                    self.iter_values(&rv)?
                } else {
                    self.keys_of(&rv)?
                };
                for item in items {
                    let itenv = Env::new(Some(self.env()));
                    match &**lhs {
                        S::Var(decls, _) => {
                            for (name, _) in decls {
                                Env::declare(&itenv, name, item.clone());
                            }
                        }
                        S::Expr(e) => {
                            self.assign_to(e, item.clone())?;
                        }
                        _ => {}
                    }
                    match self.exec_block(body, itenv)? {
                        Ok2::V(_) => {}
                        Ok2::Flow(Flow::Break(None)) => break,
                        Ok2::Flow(Flow::Break(l)) => return Ok(Ok2::Flow(Flow::Break(l))),
                        Ok2::Flow(Flow::Return(v)) => return Ok(Ok2::Flow(Flow::Return(v))),
                        Ok2::Flow(Flow::Continue(None)) => {}
                        Ok2::Flow(Flow::Continue(l)) => return Ok(Ok2::Flow(Flow::Continue(l))),
                    }
                }
                Ok(Ok2::V(V::Undef))
            }
            S::While(c, body) => {
                while self.eval(c)?.truthy(self.rt) {
                    self.tick()?;
                    match self.exec_block(body, Env::new(Some(self.env())))? {
                        Ok2::V(_) => {}
                        Ok2::Flow(Flow::Break(None)) => break,
                        Ok2::Flow(Flow::Break(l)) => return Ok(Ok2::Flow(Flow::Break(l))),
                        Ok2::Flow(Flow::Return(v)) => return Ok(Ok2::Flow(Flow::Return(v))),
                        Ok2::Flow(Flow::Continue(None)) => {}
                        Ok2::Flow(Flow::Continue(l)) => return Ok(Ok2::Flow(Flow::Continue(l))),
                    }
                }
                Ok(Ok2::V(V::Undef))
            }
            S::DoWhile(c, body) => {
                loop {
                    match self.exec_block(body, Env::new(Some(self.env())))? {
                        Ok2::V(_) => {}
                        Ok2::Flow(Flow::Break(None)) => break,
                        Ok2::Flow(Flow::Break(l)) => return Ok(Ok2::Flow(Flow::Break(l))),
                        Ok2::Flow(Flow::Return(v)) => return Ok(Ok2::Flow(Flow::Return(v))),
                        Ok2::Flow(Flow::Continue(None)) => {}
                        Ok2::Flow(Flow::Continue(l)) => return Ok(Ok2::Flow(Flow::Continue(l))),
                    }
                    self.tick()?;
                    if !self.eval(c)?.truthy(self.rt) {
                        break;
                    }
                }
                Ok(Ok2::V(V::Undef))
            }
            S::Return(e) => {
                let v = match e {
                    Some(e) => self.eval(e)?,
                    None => V::Undef,
                };
                Ok(Ok2::Flow(Flow::Return(v)))
            }
            S::Break(l) => Ok(Ok2::Flow(Flow::Break(l.clone()))),
            S::Continue(l) => Ok(Ok2::Flow(Flow::Continue(l.clone()))),
            S::Try(block, catch, fin) => {
                let r = self.exec_block(block, Env::new(Some(self.env())));
                let r = match (r, catch) {
                    (Err(t), Some((param, cbody))) if t.fatal.is_none() => {
                        let cenv = Env::new(Some(self.env()));
                        Env::declare(&cenv, param, t.v);
                        self.exec_block(cbody, cenv)
                    }
                    (r, _) => r,
                };
                if let Some(fbody) = fin {
                    let r2 = self.exec_block(fbody, Env::new(Some(self.env())));
                    match (r, r2) {
                        (_, Err(t)) => return Err(t),
                        (Ok(Ok2::Flow(f)), _) => return Ok(Ok2::Flow(f)),
                        (Ok(v), _) => return Ok(v),
                        (Err(t), _) => return Err(t),
                    }
                }
                r
            }
            S::Throw(e) => Err(Throw::val(self.eval(e)?)),
            S::Switch(e, cases) => {
                let sv = self.eval(e)?;
                let mut matched = false;
                for (ce, body) in cases {
                    if !matched {
                        match ce {
                            Some(ce) => {
                                let cv = self.eval(ce)?;
                                if !strict_eq(&sv, &cv, self.rt) {
                                    continue;
                                }
                                matched = true;
                            }
                            None => matched = true,
                        }
                    }
                    match self.exec_block(body, Env::new(Some(self.env())))? {
                        Ok2::V(_) => {}
                        Ok2::Flow(Flow::Break(_)) => break,
                        Ok2::Flow(f) => return Ok(Ok2::Flow(f)),
                    }
                }
                Ok(Ok2::V(V::Undef))
            }
            S::Labeled(label, inner) => {
                let r = self.stmt(inner);
                match r {
                    Ok(Ok2::Flow(Flow::Break(Some(l)))) if l == *label => Ok(Ok2::V(V::Undef)),
                    r => r,
                }
            }
            S::Empty | S::Debugger => Ok(Ok2::V(V::Undef)),
        }
    }

    fn iter_values(&mut self, v: &V) -> Res<Vec<V>> {
        match v {
            V::Obj(r) => {
                let o = self.rt.obj(*r);
                match &o.kind {
                    ObjKind::Arr(a) => Ok(a.borrow().clone()),
                    ObjKind::StrObj(s) => Ok(s.chars().map(|c| V::s(c.to_string())).collect()),
                    ObjKind::SetObj(s) => Ok(s.borrow().clone()),
                    ObjKind::MapObj(m) => {
                        let pairs: Vec<(V, V)> = m.borrow().clone();
                        Ok(pairs
                            .into_iter()
                            .map(|(k, v)| V::Obj(self.alloc_arr(vec![k, v])))
                            .collect())
                    }
                    ObjKind::NodeList(ns) => {
                        let ns = ns.clone();
                        Ok(ns
                            .iter()
                            .map(|&n| crate::dom_api::wrap(self.rt, n))
                            .collect())
                    }
                    ObjKind::Iter(it) => Ok(it.borrow().as_slice().to_vec()),
                    _ => {
                        // iterable via Symbol-ish? fall back to props values
                        Ok(o.props.borrow().values().cloned().collect())
                    }
                }
            }
            V::Str(s) => Ok(s.chars().map(|c| V::s(c.to_string())).collect()),
            _ => Ok(vec![]),
        }
    }

    fn keys_of(&mut self, v: &V) -> Res<Vec<V>> {
        match v {
            V::Obj(r) => {
                let mut out: Vec<String> = Vec::new();
                let mut cur = *r;
                loop {
                    let (ks, ne) = {
                        let o = self.rt.obj(cur);
                        (
                            o.props.borrow().keys().cloned().collect::<Vec<_>>(),
                            o.non_enum.borrow().clone(),
                        )
                    };
                    for k in ks {
                        if !ne.contains(&k) && !out.contains(&k) {
                            out.push(k);
                        }
                    }
                    let p = self.rt.obj(cur).proto.get();
                    if p == crate::rt::NO_PROTO {
                        break;
                    }
                    cur = p;
                }
                if let ObjKind::Arr(a) = &self.rt.obj(*r).kind {
                    for i in 0..a.borrow().len() {
                        out.insert(i, i.to_string());
                    }
                }
                Ok(out.into_iter().map(V::s).collect())
            }
            _ => Ok(vec![]),
        }
    }

    fn alloc_arr(&mut self, items: Vec<V>) -> usize {
        let o = Obj::new(ObjKind::Arr(RefCell::new(items)), self.rt.arr_proto);
        self.rt.alloc(o)
    }

    // ---------- expressions ----------

    pub fn eval(&mut self, e: &E) -> Res<V> {
        self.tick()?;
        match e {
            E::Num(n) => Ok(V::Num(*n)),
            E::Str(s) => Ok(V::s(s.clone())),
            E::Bool(b) => Ok(V::Bool(*b)),
            E::Null => Ok(V::Null),
            E::Undef => Ok(V::Undef),
            E::This => Ok(self.env().borrow().this.borrow().clone()),
            E::Paren(e) | E::Await(e) => self.eval(e),
            E::Ident(name) => {
                if name == "super" {
                    return Err(err(self.rt, "super outside method"));
                }
                match Env::get(&self.env(), name) {
                    Some(v) => Ok(v),
                    None => Err(err(
                        self.rt,
                        format!("ReferenceError: {name} is not defined"),
                    )),
                }
            }
            E::Arr(items) => {
                let mut out = Vec::new();
                for it in items {
                    match it {
                        None => out.push(V::Undef),
                        Some(E::Spread(e)) => {
                            let v = self.eval(e)?;
                            out.extend(self.iter_values(&v)?);
                        }
                        Some(e) => out.push(self.eval(e)?),
                    }
                }
                Ok(V::Obj(self.alloc_arr(out)))
            }
            E::Obj(items) => {
                let o = self
                    .rt
                    .alloc(Obj::new(ObjKind::Ordinary, self.rt.obj_proto));
                for (k, e) in items {
                    if k == "$spread" {
                        let v = self.eval(e)?;
                        if let V::Obj(r) = v {
                            let pairs: Vec<(String, V)> = self
                                .rt
                                .obj(r)
                                .props
                                .borrow()
                                .iter()
                                .map(|(a, b)| (a.clone(), b.clone()))
                                .collect();
                            for (pk, pv) in pairs {
                                self.rt.set(o, &pk, pv);
                            }
                        }
                    } else {
                        let v = self.eval(e)?;
                        self.rt.set(o, k, v);
                    }
                }
                Ok(V::Obj(o))
            }
            E::Regex(body, flags) => {
                let o = self.rt.alloc(Obj::new(
                    ObjKind::Regex {
                        body: body.clone(),
                        flags: flags.clone(),
                        last: std::cell::Cell::new(0),
                    },
                    self.rt.re_proto,
                ));
                Ok(V::Obj(o))
            }
            E::Template(parts) => {
                let mut s = String::new();
                for p in parts {
                    match p {
                        TE::Str(t) => s.push_str(t),
                        TE::Expr(e) => s.push_str(&self.eval(e)?.to_str(self.rt)),
                    }
                }
                Ok(V::s(s))
            }
            E::Seq(items) => {
                let mut last = V::Undef;
                for e in items {
                    last = self.eval(e)?;
                }
                Ok(last)
            }
            E::Unary(op, e) => match *op {
                "!" => Ok(V::Bool(!self.eval(e)?.truthy(self.rt))),
                "~" => Ok(V::Num(!(self.eval(e)?.to_num(self.rt) as i32) as f64)),
                "-" => Ok(V::Num(-self.eval(e)?.to_num(self.rt))),
                "+" => Ok(V::Num(self.eval(e)?.to_num(self.rt))),
                "typeof" => {
                    // undeclared ident → "undefined" not error
                    if let E::Ident(n) = &**e {
                        if Env::get(&self.env(), n).is_none() {
                            return Ok(V::s("undefined"));
                        }
                    }
                    let v = self.eval(e)?;
                    Ok(V::s(v.type_name(self.rt)))
                }
                "void" => {
                    self.eval(e)?;
                    Ok(V::Undef)
                }
                "delete" => {
                    if let Some(Ref2::Prop(V::Obj(r), k)) = self.reference(e)? {
                        let key = k.to_str(self.rt);
                        self.rt.obj(r).props.borrow_mut().remove(&key);
                    }
                    Ok(V::Bool(true))
                }
                "?.call" => self.eval(e),
                _ => Err(fatal("bad unary")),
            },
            E::Binary(op, a, b) => self.binary(op, a, b),
            E::Ternary(c, a, b) => {
                if self.eval(c)?.truthy(self.rt) {
                    self.eval(a)
                } else {
                    self.eval(b)
                }
            }
            E::Assign(op, l, r) => {
                if *op == "=destr" {
                    let rv = self.eval(r)?;
                    self.destructure(l, rv.clone())?;
                    return Ok(rv);
                }
                let reference = self.reference(l)?;
                let rv = self.eval(r)?;
                let v = match *op {
                    "=" => rv,
                    _ => {
                        let cur = self.deref(&reference)?;
                        self.apply_assign_op(op, cur, rv)?
                    }
                };
                self.write_ref(reference, v.clone())?;
                Ok(v)
            }
            E::Update(op, prefix, e) => {
                let reference = self.reference(e)?;
                let cur = self.deref(&reference)?.to_num(self.rt);
                let d = if *op == "++" { 1.0 } else { -1.0 };
                let nv = V::Num(cur + d);
                self.write_ref(reference, nv.clone())?;
                Ok(if *prefix { nv } else { V::Num(cur) })
            }
            E::Member(o, name) => {
                let ov = self.eval(o)?;
                if ov.is_nullish() {
                    return Err(err(
                        self.rt,
                        format!("TypeError: cannot read '{name}' of {}", ov.to_str(self.rt)),
                    ));
                }
                self.get_prop(ov, name)
            }
            E::Index(o, i) => {
                let ov = self.eval(o)?;
                let iv = self.eval(i)?;
                if ov.is_nullish() {
                    return Err(err(self.rt, "TypeError: cannot index nullish"));
                }
                self.get_index(ov, iv)
            }
            E::Call(f, args) => {
                if let E::Ident(n) = &**f {
                    if n == "super" {
                        // super() inside a derived ctor: run parent ctor on `this`
                        let mut argv = Vec::new();
                        for a in args {
                            argv.push(self.eval(a)?);
                        }
                        let (nt, this) = {
                            let e = self.env();
                            let e = e.borrow();
                            let nt = e.new_target.borrow().clone();
                            let th = e.this.borrow().clone();
                            (nt, th)
                        };
                        let sup = if let V::Obj(r) = &nt {
                            if let ObjKind::Class { sup, .. } = &self.rt.obj(*r).kind {
                                *sup
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        return match sup {
                            Some(s) => {
                                self.construct_on(V::Obj(s), this, argv)?;
                                Ok(V::Undef)
                            }
                            None => Err(err(self.rt, "super() outside derived class")),
                        };
                    }
                }
                // f may be Member → this binding
                let (fv, this) = match &**f {
                    E::Member(o, name) => {
                        let ov = self.eval(o)?;
                        if ov.is_nullish() {
                            return Err(err(self.rt, format!("TypeError: cannot read '{name}'")));
                        }
                        let fv = self.get_prop(ov.clone(), name)?;
                        (fv, ov)
                    }
                    E::Index(o, i) => {
                        let ov = self.eval(o)?;
                        let iv = self.eval(i)?;
                        let fv = self.get_index(ov.clone(), iv)?;
                        (fv, ov)
                    }
                    E::Binary("?.", o, k) => {
                        let ov = self.eval(o)?;
                        if ov.is_nullish() {
                            return Ok(V::Undef);
                        }
                        if let E::Str(name) = &**k {
                            let fv = self.get_prop(ov.clone(), name)?;
                            (fv, ov)
                        } else {
                            (V::Undef, V::Undef)
                        }
                    }
                    _ => (self.eval(f)?, V::Undef),
                };
                let mut argv = Vec::new();
                for a in args {
                    if let E::Spread(e) = a {
                        let v = self.eval(e)?;
                        argv.extend(self.iter_values(&v)?);
                    } else {
                        argv.push(self.eval(a)?);
                    }
                }
                self.call(fv, this, argv)
            }
            E::New(f, args) => {
                let fv = self.eval(f)?;
                let mut argv = Vec::new();
                for a in args {
                    argv.push(self.eval(a)?);
                }
                self.construct(fv, argv)
            }
            E::Func(def) => Ok(self.mk_func(def)),
            E::Arrow(def) => Ok(self.mk_func(def)),
            E::Class(def) => self.mk_class(def),
            E::Spread(_) => Err(fatal("spread outside call")),
        }
    }

    fn mk_func(&mut self, def: &Rc<FuncDef>) -> V {
        let env = self.env();
        let arrow_this = if def.is_arrow {
            Some(self.env().borrow().this.borrow().clone())
        } else {
            None
        };
        let o = self.rt.alloc(Obj::new(
            ObjKind::Func {
                def: def.clone(),
                env,
                arrow_this,
            },
            self.rt.func_proto,
        ));
        // prototype object for `new`
        let proto = self
            .rt
            .alloc(Obj::new(ObjKind::Ordinary, self.rt.obj_proto));
        self.rt.set(proto, "constructor", V::Obj(o));
        self.rt.set(o, "prototype", V::Obj(proto));
        let len = def
            .params
            .iter()
            .take_while(|p| !p.starts_with("...") && !p.starts_with("$d"))
            .count();
        self.rt.set(o, "length", V::Num(len as f64));
        if let Some(n) = &def.name {
            self.rt.set(o, "name", V::s(n.clone()));
        }
        V::Obj(o)
    }

    fn mk_class(&mut self, def: &Rc<ClassDef>) -> Res<V> {
        let env = self.env();
        // proto for instances
        let sup_proto = if let Some(se) = &def.extends {
            match self.eval(se)? {
                V::Obj(r) => Some(r),
                _ => return Err(err(self.rt, "extends must be a class")),
            }
        } else {
            None
        };
        let inst_proto_base = sup_proto
            .and_then(|s| self.rt.get_raw(s, "prototype"))
            .and_then(|v| if let V::Obj(r) = v { Some(r) } else { None })
            .unwrap_or(self.rt.obj_proto);
        let inst_proto = self.rt.alloc(Obj::new(ObjKind::Ordinary, inst_proto_base));
        let mut ctor = None;
        let mut methods = Vec::new();
        let mut statics = Vec::new();
        for (name, f, kind) in &def.methods {
            let fo = self.rt.alloc(Obj::new(
                ObjKind::Func {
                    def: f.clone(),
                    env: env.clone(),
                    arrow_this: None,
                },
                self.rt.func_proto,
            ));
            match kind {
                MethodKind::Ctor => ctor = Some(f.clone()),
                MethodKind::Static => statics.push((name.clone(), fo)),
                MethodKind::Get => {
                    inst_proto_getset(self.rt, inst_proto, name, fo, true);
                }
                MethodKind::Set => {
                    inst_proto_getset(self.rt, inst_proto, name, fo, false);
                }
                MethodKind::Normal => {
                    self.rt.set(inst_proto, name, V::Obj(fo));
                    methods.push((name.clone(), fo));
                }
            }
        }
        let co = self.rt.alloc(Obj::new(
            ObjKind::Class {
                ctor,
                env,
                methods,
                proto: inst_proto,
                sup: sup_proto,
            },
            self.rt.func_proto,
        ));
        self.rt.set(inst_proto, "constructor", V::Obj(co));
        for (n, f) in statics {
            self.rt.set(co, &n, V::Obj(f));
        }
        if let Some(n) = &def.name {
            self.rt.set(co, "name", V::s(n.clone()));
        }
        Ok(V::Obj(co))
    }

    fn get_prop(&mut self, ov: V, name: &str) -> Res<V> {
        // primitives → boxed object lookup
        match ov.clone() {
            V::Obj(r) => self.get_obj_prop(r, name, ov),
            V::Str(s) => match name {
                "length" => Ok(V::Num(s.chars().count() as f64)),
                _ => {
                    let boxed = self
                        .rt
                        .alloc(Obj::new(ObjKind::StrObj(s.clone()), self.rt.str_proto));
                    self.get_obj_prop(boxed, name, ov)
                }
            },
            V::Num(n) => {
                let boxed = self
                    .rt
                    .alloc(Obj::new(ObjKind::NumObj(n), self.rt.num_proto));
                self.get_obj_prop(boxed, name, ov)
            }
            V::Bool(b) => {
                let boxed = self
                    .rt
                    .alloc(Obj::new(ObjKind::BoolObj(b), self.rt.bool_proto));
                self.get_obj_prop(boxed, name, ov)
            }
            _ => Ok(V::Undef),
        }
    }

    fn get_obj_prop(&mut self, r: usize, name: &str, this: V) -> Res<V> {
        if r == self.rt.global_obj {
            let g = self.rt.global_env.clone();
            if let Some(v) = Env::get(&g, name) {
                return Ok(v);
            }
        }
        // DOM-flavored objects get dynamic property dispatch first
        match &self.rt.obj(r).kind {
            ObjKind::Dom { .. }
            | ObjKind::NodeList(_)
            | ObjKind::StyleObj { .. }
            | ObjKind::ClassList { .. }
            | ObjKind::Location
            | ObjKind::Storage
            | ObjKind::Event { .. } => {
                if let Some(v) = crate::dom_api::get_prop(self, r, name)? {
                    return Ok(v);
                }
            }
            _ => {}
        }
        // array fast paths
        if let ObjKind::Arr(a) = &self.rt.obj(r).kind {
            if name == "length" {
                return Ok(V::Num(a.borrow().len() as f64));
            }
            if let Ok(i) = name.parse::<usize>() {
                return Ok(a.borrow().get(i).cloned().unwrap_or(V::Undef));
            }
        }
        if let ObjKind::Arr(_) = &self.rt.obj(r).kind {
            if let Some(v) = crate::builtins::array_method(self.rt, name, r) {
                return Ok(v);
            }
        }
        if let ObjKind::StrObj(s) = &self.rt.obj(r).kind {
            if name == "length" {
                return Ok(V::Num(s.chars().count() as f64));
            }
            if let Ok(i) = name.parse::<usize>() {
                return Ok(s
                    .chars()
                    .nth(i)
                    .map(|c| V::s(c.to_string()))
                    .unwrap_or(V::Undef));
            }
        }
        // accessor walk
        let mut cur = r;
        loop {
            let (getter, prop, proto) = {
                let o = self.rt.obj(cur);
                let g = o.accessors.borrow().get(name).map(|(g, _)| g.clone());
                let v = o.props.borrow().get(name).cloned();
                (g, v, o.proto.get())
            };
            if let Some(g) = getter {
                if !g.is_nullish() {
                    return self.call(g, this, vec![]);
                }
                return Ok(V::Undef);
            }
            if let Some(v) = prop {
                return Ok(v);
            }
            if proto == crate::rt::NO_PROTO {
                return Ok(V::Undef);
            }
            cur = proto;
        }
    }

    fn get_index(&mut self, ov: V, iv: V) -> Res<V> {
        if let V::Obj(r) = ov {
            if let ObjKind::Arr(a) = &self.rt.obj(r).kind {
                if let V::Num(n) = iv {
                    return Ok(a.borrow().get(n as usize).cloned().unwrap_or(V::Undef));
                }
            }
            if let ObjKind::StrObj(s) = &self.rt.obj(r).kind {
                if let V::Num(n) = iv {
                    return Ok(s
                        .chars()
                        .nth(n as usize)
                        .map(|c| V::s(c.to_string()))
                        .unwrap_or(V::Undef));
                }
            }
            return self.get_obj_prop(r, &iv.to_str(self.rt), ov);
        }
        if let V::Str(s) = ov {
            if let V::Num(n) = iv {
                return Ok(s
                    .chars()
                    .nth(n as usize)
                    .map(|c| V::s(c.to_string()))
                    .unwrap_or(V::Undef));
            }
        }
        Ok(V::Undef)
    }

    fn set_index(&mut self, ov: V, iv: V, val: V) -> Res<()> {
        if let V::Obj(r) = ov {
            if let ObjKind::Arr(a) = &self.rt.obj(r).kind {
                if let V::Num(n) = iv {
                    let i = n as usize;
                    let mut a = a.borrow_mut();
                    if i >= a.len() {
                        a.resize(i + 1, V::Undef);
                    }
                    a[i] = val;
                    return Ok(());
                }
            }
            let key = iv.to_str(self.rt);
            self.rt.set(r, &key, val);
        }
        Ok(())
    }

    fn reference(&mut self, e: &E) -> Res<Option<Ref2>> {
        Ok(match e {
            E::Ident(name) => Some(Ref2::Var(name.clone(), self.env())),
            E::Member(o, name) => {
                let ov = self.eval(o)?;
                Some(Ref2::Prop(ov, V::s(name.clone())))
            }
            E::Index(o, i) => {
                let ov = self.eval(o)?;
                let iv = self.eval(i)?;
                Some(Ref2::Prop(ov, iv))
            }
            _ => None,
        })
    }

    fn deref(&mut self, r: &Option<Ref2>) -> Res<V> {
        match r {
            Some(Ref2::Var(name, env)) => Env::get(env, name)
                .ok_or(())
                .map_err(|_| err(self.rt, format!("ReferenceError: {name}"))),
            Some(Ref2::Prop(ov, V::Str(k))) => self.get_prop(ov.clone(), k),
            Some(Ref2::Prop(ov, k)) => self.get_index(ov.clone(), k.clone()),
            _ => Err(err(self.rt, "invalid assignment target")),
        }
    }

    fn write_ref(&mut self, r: Option<Ref2>, v: V) -> Res<()> {
        match r {
            Some(Ref2::Var(name, env)) => {
                if !Env::set(&env, &name, v.clone()) {
                    let g = self.rt.global_env.clone();
                    Env::declare(&g, &name, v);
                }
            }
            Some(Ref2::Prop(V::Obj(ro), V::Str(k))) => {
                if crate::dom_api::set_prop(self, ro, &k, v.clone())? {
                    return Ok(());
                }
                if ro == self.rt.global_obj {
                    let g = self.rt.global_env.clone();
                    Env::declare(&g, &k, v.clone());
                }
                self.rt.set(ro, &k, v);
            }
            Some(Ref2::Prop(V::Obj(ro), k)) => {
                self.set_index(V::Obj(ro), k, v)?;
            }
            Some(Ref2::Prop(ov, k)) => {
                // primitive receiver: ignore
                let _ = (ov, k);
            }
            _ => return Err(err(self.rt, "invalid assignment target")),
        }
        Ok(())
    }

    fn assign_to(&mut self, l: &E, v: V) -> Res<()> {
        let r = self.reference(l)?;
        self.write_ref(r, v)
    }

    fn apply_assign_op(&mut self, op: &'static str, a: V, b: V) -> Res<V> {
        match op {
            "+=" => self.plus(a, b),
            "-=" => Ok(V::Num(a.to_num(self.rt) - b.to_num(self.rt))),
            "*=" => Ok(V::Num(a.to_num(self.rt) * b.to_num(self.rt))),
            "/=" => Ok(V::Num(a.to_num(self.rt) / b.to_num(self.rt))),
            "%=" => Ok(V::Num(a.to_num(self.rt) % b.to_num(self.rt))),
            "**=" => Ok(V::Num(a.to_num(self.rt).powf(b.to_num(self.rt)))),
            "<<=" => Ok(V::Num(
                ((a.to_num(self.rt) as i32) << (b.to_num(self.rt) as u32 & 31)) as f64,
            )),
            ">>=" => Ok(V::Num(
                ((a.to_num(self.rt) as i32) >> (b.to_num(self.rt) as u32 & 31)) as f64,
            )),
            ">>>=" => Ok(V::Num(
                ((a.to_num(self.rt) as u32) >> (b.to_num(self.rt) as u32 & 31)) as f64,
            )),
            "&=" => Ok(V::Num(
                ((a.to_num(self.rt) as i32) & (b.to_num(self.rt) as i32)) as f64,
            )),
            "|=" => Ok(V::Num(
                ((a.to_num(self.rt) as i32) | (b.to_num(self.rt) as i32)) as f64,
            )),
            "^=" => Ok(V::Num(
                ((a.to_num(self.rt) as i32) ^ (b.to_num(self.rt) as i32)) as f64,
            )),
            "&&=" => Ok(if a.truthy(self.rt) { b } else { a }),
            "||=" => Ok(if a.truthy(self.rt) { a } else { b }),
            "??=" => Ok(if a.is_nullish() { b } else { a }),
            _ => Err(fatal("bad assign op")),
        }
    }

    fn plus(&mut self, a: V, b: V) -> Res<V> {
        if matches!(a, V::Str(_)) || matches!(b, V::Str(_)) {
            return Ok(V::s(format!("{}{}", a.to_str(self.rt), b.to_str(self.rt))));
        }
        Ok(V::Num(a.to_num(self.rt) + b.to_num(self.rt)))
    }

    fn binary(&mut self, op: &'static str, ae: &E, be: &E) -> Res<V> {
        match op {
            "&&" => {
                let a = self.eval(ae)?;
                if !a.truthy(self.rt) {
                    return Ok(a);
                }
                self.eval(be)
            }
            "||" => {
                let a = self.eval(ae)?;
                if a.truthy(self.rt) {
                    return Ok(a);
                }
                self.eval(be)
            }
            "??" => {
                let a = self.eval(ae)?;
                if a.is_nullish() {
                    self.eval(be)
                } else {
                    Ok(a)
                }
            }
            "?." => {
                let a = self.eval(ae)?;
                if a.is_nullish() {
                    return Ok(V::Undef);
                }
                if let E::Str(name) = be {
                    self.get_prop(a, name)
                } else {
                    Ok(V::Undef)
                }
            }
            _ => {
                let a = self.eval(ae)?;
                let b = self.eval(be)?;
                self.arith(op, a, b)
            }
        }
    }

    fn arith(&mut self, op: &'static str, a: V, b: V) -> Res<V> {
        Ok(match op {
            "+" => return self.plus(a, b),
            "-" => V::Num(a.to_num(self.rt) - b.to_num(self.rt)),
            "*" => V::Num(a.to_num(self.rt) * b.to_num(self.rt)),
            "/" => V::Num(a.to_num(self.rt) / b.to_num(self.rt)),
            "%" => V::Num(a.to_num(self.rt) % b.to_num(self.rt)),
            "**" => V::Num(a.to_num(self.rt).powf(b.to_num(self.rt))),
            "==" => V::Bool(loose_eq(&a, &b, self.rt)),
            "!=" => V::Bool(!loose_eq(&a, &b, self.rt)),
            "===" => V::Bool(strict_eq(&a, &b, self.rt)),
            "!==" => V::Bool(!strict_eq(&a, &b, self.rt)),
            "<" => V::Bool(lt(&a, &b, self.rt)),
            ">" => V::Bool(lt(&b, &a, self.rt)),
            "<=" => V::Bool(!lt(&b, &a, self.rt)),
            ">=" => V::Bool(!lt(&a, &b, self.rt)),
            "&" => V::Num(((a.to_num(self.rt) as i32) & (b.to_num(self.rt) as i32)) as f64),
            "|" => V::Num(((a.to_num(self.rt) as i32) | (b.to_num(self.rt) as i32)) as f64),
            "^" => V::Num(((a.to_num(self.rt) as i32) ^ (b.to_num(self.rt) as i32)) as f64),
            "<<" => V::Num(((a.to_num(self.rt) as i32) << (b.to_num(self.rt) as u32 & 31)) as f64),
            ">>" => V::Num(((a.to_num(self.rt) as i32) >> (b.to_num(self.rt) as u32 & 31)) as f64),
            ">>>" => V::Num(((a.to_num(self.rt) as u32) >> (b.to_num(self.rt) as u32 & 31)) as f64),
            "in" => {
                let key = a.to_str(self.rt);
                if let V::Obj(r) = b {
                    V::Bool(self.rt.get_raw(r, &key).is_some() || has_own(self.rt, r, &key))
                } else {
                    V::Bool(false)
                }
            }
            "instanceof" => {
                let proto = match b {
                    V::Obj(r) => self.rt.get_raw(r, "prototype"),
                    _ => None,
                };
                let mut hit = false;
                if let (V::Obj(mut cur), Some(V::Obj(p))) = (a.clone(), proto) {
                    loop {
                        if cur == p {
                            hit = true;
                            break;
                        }
                        let next = self.rt.obj(cur).proto.get();
                        if next == crate::rt::NO_PROTO {
                            break;
                        }
                        cur = next;
                    }
                }
                V::Bool(hit)
            }
            _ => return Err(fatal(format!("bad op {op}"))),
        })
    }

    fn destructure(&mut self, l: &E, v: V) -> Res<()> {
        match l {
            E::Arr(items) => {
                let vals = self.iter_values(&v)?;
                for (i, it) in items.iter().enumerate() {
                    if let Some(e) = it {
                        let iv = vals.get(i).cloned().unwrap_or(V::Undef);
                        if let E::Spread(t) = e {
                            let rest = vals[i.min(vals.len())..].to_vec();
                            let arr = V::Obj(self.alloc_arr(rest));
                            self.assign_to(t, arr)?;
                        } else {
                            self.assign_to(e, iv)?;
                        }
                    }
                }
                Ok(())
            }
            E::Obj(items) => {
                for (k, e) in items {
                    let pv = self.get_prop(v.clone(), k)?;
                    self.assign_to(e, pv)?;
                }
                Ok(())
            }
            _ => Err(err(self.rt, "invalid destructure")),
        }
    }

    // ---------- calls ----------

    pub fn call(&mut self, fv: V, this: V, args: Vec<V>) -> Res<V> {
        let V::Obj(r) = fv else {
            return Err(err(
                self.rt,
                format!("TypeError: {} is not a function", fv.to_str(self.rt)),
            ));
        };
        enum C {
            N(NativeFn),
            F(Rc<FuncDef>, EnvRef, Option<V>),
            Class,
            Bad,
        }
        let c = match &self.rt.obj(r).kind {
            ObjKind::Native { f, .. } => C::N(f.clone()),
            ObjKind::Func {
                def,
                env,
                arrow_this,
            } => C::F(def.clone(), env.clone(), arrow_this.clone()),
            ObjKind::Class { .. } => C::Class,
            _ => C::Bad,
        };
        match c {
            C::N(f) => {
                self.rt.depth += 1;
                let r2 = f(self, this, &args);
                self.rt.depth -= 1;
                r2
            }
            C::F(def, env, arrow_this) => {
                let this = arrow_this.unwrap_or(this);
                self.invoke(&def, env, this, args, V::Undef)
            }
            C::Class => Err(err(
                self.rt,
                "TypeError: class cannot be invoked without new",
            )),
            C::Bad => Err(err(self.rt, "TypeError: not a function")),
        }
    }

    pub fn construct(&mut self, fv: V, args: Vec<V>) -> Res<V> {
        let V::Obj(r) = fv else {
            return Err(err(self.rt, "not a constructor"));
        };
        enum C {
            F(Rc<FuncDef>, EnvRef),
            K {
                ctor: Option<Rc<FuncDef>>,
                env: EnvRef,
                proto: usize,
                sup: Option<usize>,
            },
            N(NativeFn),
            Bad,
        }
        let c = match &self.rt.obj(r).kind {
            ObjKind::Func { def, env, .. } => C::F(def.clone(), env.clone()),
            ObjKind::Class {
                ctor,
                env,
                proto,
                sup,
                ..
            } => C::K {
                ctor: ctor.clone(),
                env: env.clone(),
                proto: *proto,
                sup: *sup,
            },
            ObjKind::Native { f, .. } => C::N(f.clone()),
            _ => C::Bad,
        };
        match c {
            C::F(def, env) => {
                let proto = match self.rt.get_raw(r, "prototype") {
                    Some(V::Obj(p)) => p,
                    _ => self.rt.obj_proto,
                };
                let this = self.rt.alloc(Obj::new(ObjKind::Ordinary, proto));
                let res = self.invoke(&def, env, V::Obj(this), args, fv.clone())?;
                match res {
                    V::Obj(_) => Ok(res),
                    _ => Ok(V::Obj(this)),
                }
            }
            C::K {
                ctor,
                env,
                proto,
                sup,
            } => {
                let this = self.rt.alloc(Obj::new(ObjKind::Ordinary, proto));
                if let Some(ctor) = ctor {
                    self.invoke(&ctor, env, V::Obj(this), args, fv.clone())?;
                } else if let Some(sup) = sup {
                    self.construct(V::Obj(sup), args)?;
                }
                Ok(V::Obj(this))
            }
            C::N(f) => f(self, V::Undef, &args),
            C::Bad => Err(err(self.rt, "not a constructor")),
        }
    }

    /// Run a constructor's body on an existing `this` (for `super()`).
    fn construct_on(&mut self, fv: V, this: V, args: Vec<V>) -> Res<()> {
        let V::Obj(r) = fv.clone() else {
            return Err(err(self.rt, "no super constructor"));
        };
        let c = match &self.rt.obj(r).kind {
            ObjKind::Class { ctor, env, sup, .. } => Some((ctor.clone(), env.clone(), *sup)),
            ObjKind::Func { def, env, .. } => Some((Some(def.clone()), env.clone(), None)),
            _ => None,
        };
        match c {
            Some((Some(ctor), env, _)) => {
                self.invoke(&ctor, env, this, args, fv)?;
                Ok(())
            }
            Some((None, _, Some(sup))) => self.construct_on(V::Obj(sup), this, args),
            _ => Err(err(self.rt, "no super constructor")),
        }
    }

    fn invoke(
        &mut self,
        def: &Rc<FuncDef>,
        closure: EnvRef,
        this: V,
        args: Vec<V>,
        new_target: V,
    ) -> Res<V> {
        let env = Env::new(Some(closure));
        *env.borrow().this.borrow_mut() = this;
        *env.borrow().new_target.borrow_mut() = new_target;
        let mut ai = 0;
        for p in &def.params {
            if let Some(rest) = p.strip_prefix("...") {
                let rest_v = V::Obj(self.alloc_arr(args[ai.min(args.len())..].to_vec()));
                Env::declare(&env, rest, rest_v);
                break;
            }
            if p.starts_with("$d") {
                // destructured param — skip binding (unsupported)
                ai += 1;
                continue;
            }
            Env::declare(&env, p, args.get(ai).cloned().unwrap_or(V::Undef));
            ai += 1;
        }
        match self.exec_block(&def.body, env)? {
            Ok2::Flow(Flow::Return(v)) => Ok(v),
            _ => Ok(V::Undef),
        }
    }
}

pub enum Ok2 {
    V(V),
    Flow(Flow),
}

fn inst_proto_getset(rt: &mut Rt, proto: usize, name: &str, f: usize, get: bool) {
    let mut acc = rt.obj(proto).accessors.borrow_mut();
    let e = acc.entry(name.to_string()).or_insert((V::Undef, V::Undef));
    if get {
        e.0 = V::Obj(f);
    } else {
        e.1 = V::Obj(f);
    }
}

fn has_own(rt: &Rt, r: usize, key: &str) -> bool {
    rt.obj(r).props.borrow().contains_key(key)
}

pub fn strict_eq(a: &V, b: &V, rt: &Rt) -> bool {
    match (a, b) {
        (V::Undef, V::Undef) | (V::Null, V::Null) => true,
        (V::Bool(x), V::Bool(y)) => x == y,
        (V::Num(x), V::Num(y)) => x == y,
        (V::Str(x), V::Str(y)) => x == y,
        (V::Obj(x), V::Obj(y)) => x == y,
        (V::Str(x), V::Obj(y)) | (V::Obj(y), V::Str(x)) => {
            if let ObjKind::StrObj(s) = &rt.obj(*y).kind {
                x == s
            } else {
                false
            }
        }
        (V::Num(x), V::Obj(y)) | (V::Obj(y), V::Num(x)) => {
            if let ObjKind::NumObj(n) = &rt.obj(*y).kind {
                x == n
            } else {
                false
            }
        }
        _ => false,
    }
}

pub fn loose_eq(a: &V, b: &V, rt: &Rt) -> bool {
    if strict_eq(a, b, rt) {
        return true;
    }
    match (a, b) {
        (V::Null, V::Undef) | (V::Undef, V::Null) => true,
        (V::Num(x), V::Str(_)) => *x == b.to_num(rt),
        (V::Str(_), V::Num(y)) => a.to_num(rt) == *y,
        (V::Bool(_), _) => loose_eq(&V::Num(a.to_num(rt)), b, rt),
        (_, V::Bool(_)) => loose_eq(a, &V::Num(b.to_num(rt)), rt),
        _ => false,
    }
}

fn lt(a: &V, b: &V, rt: &Rt) -> bool {
    if let (V::Str(x), V::Str(y)) = (a, b) {
        return x < y;
    }
    a.to_num(rt) < b.to_num(rt)
}
