//! JS parser → AST. Covers a practical ES subset.

use crate::lex::{lex, TPart, T};
use std::rc::Rc;

#[derive(Debug, Clone)]
pub enum E {
    Num(f64),
    Str(String),
    Bool(bool),
    Null,
    Undef,
    Ident(String),
    This,
    Arr(Vec<Option<E>>),
    Obj(Vec<(String, E)>),
    /// regex literal (body, flags)
    Regex(String, String),
    Unary(&'static str, Box<E>),
    Binary(&'static str, Box<E>, Box<E>),
    Assign(&'static str, Box<E>, Box<E>),
    Ternary(Box<E>, Box<E>, Box<E>),
    Call(Box<E>, Vec<E>),
    New(Box<E>, Vec<E>),
    Member(Box<E>, String),
    Index(Box<E>, Box<E>),
    /// (params, body, name)
    Func(Rc<FuncDef>),
    Class(Rc<ClassDef>),
    Template(Vec<TE>),
    Seq(Vec<E>),
    Paren(Box<E>),
    Spread(Box<E>),
    Arrow(Rc<FuncDef>),
    Await(Box<E>),
    Update(&'static str, bool, Box<E>), // op, prefix, operand
}

#[derive(Debug, Clone)]
pub enum TE {
    Str(String),
    Expr(E),
}

#[derive(Debug)]
pub struct FuncDef {
    pub name: Option<String>,
    pub params: Vec<String>,
    pub body: Vec<S>,
    pub is_arrow: bool,
}

#[derive(Debug)]
pub struct ClassDef {
    pub name: Option<String>,
    pub extends: Option<E>,
    pub methods: Vec<(String, Rc<FuncDef>, MethodKind)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MethodKind {
    Normal,
    Ctor,
    Get,
    Set,
    Static,
}

#[derive(Debug, Clone)]
pub enum S {
    Expr(E),
    Var(Vec<(String, Option<E>)>, &'static str), // kind
    FuncDecl(Rc<FuncDef>),
    ClassDecl(Rc<ClassDef>),
    If(Box<E>, Vec<S>, Vec<S>),
    For(Option<Box<S>>, Option<E>, Option<E>, Vec<S>),
    ForIn(Box<S>, E, Vec<S>),
    ForOf(Box<S>, E, Vec<S>),
    While(E, Vec<S>),
    DoWhile(E, Vec<S>),
    Return(Option<E>),
    Break(Option<String>),
    Continue(Option<String>),
    Block(Vec<S>),
    Try(Vec<S>, Option<(String, Vec<S>)>, Option<Vec<S>>),
    Throw(E),
    Switch(E, Vec<(Option<E>, Vec<S>)>),
    Empty,
    Labeled(String, Box<S>),
    Debugger,
}

pub fn parse(src: &str) -> Result<Vec<S>, String> {
    let toks = lex(src)?;
    let mut p = P { t: toks, i: 0 };
    let mut body = Vec::new();
    while !p.at_eof() {
        body.push(p.stmt()?);
    }
    Ok(body)
}

/// Re-parse a sub-expression token stream (template parts).
fn parse_tokens(toks: Vec<T>) -> Result<E, String> {
    let mut p = P { t: toks, i: 0 };
    let e = p.expr()?;
    Ok(e)
}

struct P {
    t: Vec<T>,
    i: usize,
}

impl P {
    fn peek(&self) -> &T {
        self.t.get(self.i).unwrap_or(&T::Eof)
    }
    fn peek2(&self) -> &T {
        self.t.get(self.i + 1).unwrap_or(&T::Eof)
    }
    fn bump(&mut self) -> T {
        let t = self.t.get(self.i).cloned().unwrap_or(T::Eof);
        self.i += 1;
        t
    }
    fn at_eof(&self) -> bool {
        matches!(self.peek(), T::Eof)
    }
    fn at_p(&self, p: &str) -> bool {
        matches!(self.peek(), T::Punct(x) if *x == p)
    }
    fn eat_p(&mut self, p: &str) -> bool {
        if self.at_p(p) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect_p(&mut self, p: &str) -> Result<(), String> {
        if self.eat_p(p) {
            Ok(())
        } else {
            Err(format!("expected '{p}', got {:?}", self.peek()))
        }
    }
    fn at_kw(&self, k: &str) -> bool {
        matches!(self.peek(), T::Kw(x) if *x == k)
    }
    fn eat_kw(&mut self, k: &str) -> bool {
        if self.at_kw(k) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn ident(&mut self) -> Result<String, String> {
        match self.bump() {
            T::Ident(s) => Ok(s),
            t => Err(format!("expected ident, got {t:?}")),
        }
    }

    // ---- statements ----
    fn stmt(&mut self) -> Result<S, String> {
        match self.peek().clone() {
            T::Punct(";") => {
                self.i += 1;
                Ok(S::Empty)
            }
            T::Punct("{") => Ok(S::Block(self.block()?)),
            T::Kw("var") | T::Kw("let") | T::Kw("const") => {
                let k = match self.bump() {
                    T::Kw(k) => k,
                    _ => unreachable!(),
                };
                let decls = self.var_decls()?;
                self.eat_p(";");
                Ok(S::Var(decls, k))
            }
            T::Kw("function") => {
                self.i += 1;
                Ok(S::FuncDecl(Rc::new(self.func(false)?)))
            }
            T::Kw("class") => {
                self.i += 1;
                Ok(S::ClassDecl(Rc::new(self.class()?)))
            }
            T::Kw("if") => {
                self.i += 1;
                self.expect_p("(")?;
                let c = self.expr()?;
                self.expect_p(")")?;
                let then = vec![self.stmt()?];
                let then = if let S::Block(b) = &then[0] {
                    b.clone()
                } else {
                    then
                };
                let els = if self.eat_kw("else") {
                    let e = vec![self.stmt()?];
                    if let S::Block(b) = &e[0] {
                        b.clone()
                    } else {
                        e
                    }
                } else {
                    Vec::new()
                };
                Ok(S::If(Box::new(c), then, els))
            }
            T::Kw("for") => {
                self.i += 1;
                self.expect_p("(")?;
                // for-in / for-of detection
                if matches!(self.peek(), T::Kw("let") | T::Kw("const") | T::Kw("var")) {
                    let k = match self.bump() {
                        T::Kw(k) => k,
                        _ => unreachable!(),
                    };
                    let name = self.ident()?;
                    if self.eat_kw("of") {
                        let rhs = self.expr()?;
                        self.expect_p(")")?;
                        let body = vec![self.stmt()?];
                        let body = if let S::Block(b) = &body[0] {
                            b.clone()
                        } else {
                            body
                        };
                        return Ok(S::ForOf(Box::new(S::Var(vec![(name, None)], k)), rhs, body));
                    }
                    if self.eat_kw("in") {
                        let rhs = self.expr()?;
                        self.expect_p(")")?;
                        let body = vec![self.stmt()?];
                        let body = if let S::Block(b) = &body[0] {
                            b.clone()
                        } else {
                            body
                        };
                        return Ok(S::ForIn(Box::new(S::Var(vec![(name, None)], k)), rhs, body));
                    }
                    // regular for with declaration
                    let init = if self.eat_p("=") {
                        Some(self.expr()?)
                    } else {
                        None
                    };
                    let mut decls = vec![(name, init)];
                    while self.eat_p(",") {
                        let n = self.ident()?;
                        let i2 = if self.eat_p("=") {
                            Some(self.expr()?)
                        } else {
                            None
                        };
                        decls.push((n, i2));
                    }
                    self.expect_p(";")?;
                    let cond = if self.at_p(";") {
                        None
                    } else {
                        Some(self.expr()?)
                    };
                    self.expect_p(";")?;
                    let step = if self.at_p(")") {
                        None
                    } else {
                        Some(self.expr()?)
                    };
                    self.expect_p(")")?;
                    let body = vec![self.stmt()?];
                    let body = if let S::Block(b) = &body[0] {
                        b.clone()
                    } else {
                        body
                    };
                    return Ok(S::For(Some(Box::new(S::Var(decls, k))), cond, step, body));
                }
                if self.at_p(";") {
                    self.i += 1;
                    let cond = if self.at_p(";") {
                        None
                    } else {
                        Some(self.expr()?)
                    };
                    self.expect_p(";")?;
                    let step = if self.at_p(")") {
                        None
                    } else {
                        Some(self.expr()?)
                    };
                    self.expect_p(")")?;
                    let body = vec![self.stmt()?];
                    let body = if let S::Block(b) = &body[0] {
                        b.clone()
                    } else {
                        body
                    };
                    return Ok(S::For(None, cond, step, body));
                }
                // expression or ident for-in/of
                let first = self.expr()?;
                if self.eat_kw("of") {
                    let rhs = self.expr()?;
                    self.expect_p(")")?;
                    let body = vec![self.stmt()?];
                    let body = if let S::Block(b) = &body[0] {
                        b.clone()
                    } else {
                        body
                    };
                    return Ok(S::ForOf(Box::new(S::Expr(first)), rhs, body));
                }
                if self.eat_kw("in") {
                    let rhs = self.expr()?;
                    self.expect_p(")")?;
                    let body = vec![self.stmt()?];
                    let body = if let S::Block(b) = &body[0] {
                        b.clone()
                    } else {
                        body
                    };
                    return Ok(S::ForIn(Box::new(S::Expr(first)), rhs, body));
                }
                self.expect_p(";")?;
                let cond = if self.at_p(";") {
                    None
                } else {
                    Some(self.expr()?)
                };
                self.expect_p(";")?;
                let step = if self.at_p(")") {
                    None
                } else {
                    Some(self.expr()?)
                };
                self.expect_p(")")?;
                let body = vec![self.stmt()?];
                let body = if let S::Block(b) = &body[0] {
                    b.clone()
                } else {
                    body
                };
                Ok(S::For(Some(Box::new(S::Expr(first))), cond, step, body))
            }
            T::Kw("while") => {
                self.i += 1;
                self.expect_p("(")?;
                let c = self.expr()?;
                self.expect_p(")")?;
                let body = vec![self.stmt()?];
                let body = if let S::Block(b) = &body[0] {
                    b.clone()
                } else {
                    body
                };
                Ok(S::While(c, body))
            }
            T::Kw("do") => {
                self.i += 1;
                let body = vec![self.stmt()?];
                let body = if let S::Block(b) = &body[0] {
                    b.clone()
                } else {
                    body
                };
                if !self.eat_kw("while") {
                    return Err("do without while".into());
                }
                self.expect_p("(")?;
                let c = self.expr()?;
                self.expect_p(")")?;
                self.eat_p(";");
                Ok(S::DoWhile(c, body))
            }
            T::Kw("return") => {
                self.i += 1;
                if self.at_p(";") || self.at_p("}") || self.at_eof() {
                    self.eat_p(";");
                    return Ok(S::Return(None));
                }
                let e = self.expr()?;
                self.eat_p(";");
                Ok(S::Return(Some(e)))
            }
            T::Kw("break") | T::Kw("continue") => {
                let is_break = matches!(self.bump(), T::Kw("break"));
                let label = match self.peek() {
                    T::Ident(s) => {
                        let s = s.clone();
                        self.i += 1;
                        Some(s)
                    }
                    _ => None,
                };
                self.eat_p(";");
                Ok(if is_break {
                    S::Break(label)
                } else {
                    S::Continue(label)
                })
            }
            T::Kw("try") => {
                self.i += 1;
                let block = self.block()?;
                let catch = if self.eat_kw("catch") {
                    let param = if self.eat_p("(") {
                        let n = self.ident()?;
                        self.expect_p(")")?;
                        n
                    } else {
                        "e".into()
                    };
                    Some((param, self.block()?))
                } else {
                    None
                };
                let fin = if self.eat_kw("finally") {
                    Some(self.block()?)
                } else {
                    None
                };
                Ok(S::Try(block, catch, fin))
            }
            T::Kw("throw") => {
                self.i += 1;
                let e = self.expr()?;
                self.eat_p(";");
                Ok(S::Throw(e))
            }
            T::Kw("switch") => {
                self.i += 1;
                self.expect_p("(")?;
                let e = self.expr()?;
                self.expect_p(")")?;
                self.expect_p("{")?;
                let mut cases = Vec::new();
                loop {
                    if self.eat_p("}") {
                        break;
                    }
                    if self.eat_kw("case") {
                        let ce = self.expr()?;
                        self.expect_p(":")?;
                        let mut body = Vec::new();
                        while !self.at_p("}") && !self.at_kw("case") && !self.at_kw("default") {
                            body.push(self.stmt()?);
                        }
                        cases.push((Some(ce), body));
                    } else if self.eat_kw("default") {
                        self.expect_p(":")?;
                        let mut body = Vec::new();
                        while !self.at_p("}") && !self.at_kw("case") {
                            body.push(self.stmt()?);
                        }
                        cases.push((None, body));
                    } else {
                        return Err(format!("bad switch: {:?}", self.peek()));
                    }
                }
                Ok(S::Switch(e, cases))
            }
            T::Kw("debugger") => {
                self.i += 1;
                self.eat_p(";");
                Ok(S::Debugger)
            }
            T::Ident(s) if matches!(self.peek2(), T::Punct(":")) => {
                self.i += 2;
                let inner = self.stmt()?;
                Ok(S::Labeled(s, Box::new(inner)))
            }
            _ => {
                let e = self.expr()?;
                self.eat_p(";");
                Ok(S::Expr(e))
            }
        }
    }

    fn block(&mut self) -> Result<Vec<S>, String> {
        self.expect_p("{")?;
        let mut out = Vec::new();
        while !self.eat_p("}") {
            if self.at_eof() {
                return Err("unterminated block".into());
            }
            out.push(self.stmt()?);
        }
        Ok(out)
    }

    fn var_decls(&mut self) -> Result<Vec<(String, Option<E>)>, String> {
        let mut out = Vec::new();
        loop {
            if self.at_p("[") || self.at_p("{") {
                // destructuring — skip pattern, bind nothing useful
                // (simple implementation: treat as anonymous)
                let _ = self.pattern()?;
                let init = if self.eat_p("=") {
                    Some(self.expr()?)
                } else {
                    None
                };
                out.push((format!("$d{}", out.len()), init));
            } else {
                let name = self.ident()?;
                let init = if self.eat_p("=") {
                    Some(self.expr()?)
                } else {
                    None
                };
                out.push((name, init));
            }
            if !self.eat_p(",") {
                break;
            }
        }
        Ok(out)
    }

    /// Consume a destructuring pattern without producing bindings.
    fn pattern(&mut self) -> Result<(), String> {
        if self.at_p("[") {
            self.i += 1;
            while !self.eat_p("]") {
                if self.at_p("[") || self.at_p("{") {
                    self.pattern()?;
                } else if matches!(self.peek(), T::Ident(_)) {
                    self.i += 1;
                } else if self.eat_p("=") {
                    let _ = self.expr()?;
                } else if self.eat_p("...") {
                    let _ = self.ident()?;
                }
                self.eat_p(",");
            }
            Ok(())
        } else {
            self.expect_p("{")?;
            while !self.eat_p("}") {
                if matches!(self.peek(), T::Ident(_)) {
                    self.i += 1;
                    if self.eat_p(":") {
                        if self.at_p("[") || self.at_p("{") {
                            self.pattern()?;
                        } else {
                            let _ = self.ident()?;
                        }
                    }
                } else if matches!(self.peek(), T::Str(_)) {
                    self.i += 1;
                    self.expect_p(":")?;
                    let _ = self.ident()?;
                } else if self.eat_p("...") {
                    let _ = self.ident()?;
                }
                self.eat_p(",");
            }
            Ok(())
        }
    }

    fn func(&mut self, _expr_ctx: bool) -> Result<FuncDef, String> {
        let name = if matches!(self.peek(), T::Ident(_)) {
            Some(self.ident()?)
        } else {
            None
        };
        self.expect_p("(")?;
        let mut params = Vec::new();
        if !self.at_p(")") {
            loop {
                if self.eat_p("...") {
                    let n = self.ident()?;
                    params.push(format!("...{n}"));
                    break;
                }
                if self.at_p("[") || self.at_p("{") {
                    self.pattern()?;
                    params.push(format!("$d{}", params.len()));
                } else {
                    params.push(self.ident()?);
                }
                if self.eat_p("=") {
                    let _ = self.expr()?; // default value — evaluated later (not bound)
                }
                if !self.eat_p(",") {
                    break;
                }
            }
        }
        self.expect_p(")")?;
        let body = self.block()?;
        Ok(FuncDef {
            name,
            params,
            body,
            is_arrow: false,
        })
    }

    fn class(&mut self) -> Result<ClassDef, String> {
        let name = if matches!(self.peek(), T::Ident(_)) {
            Some(self.ident()?)
        } else {
            None
        };
        let extends = if self.eat_kw("extends") {
            Some(self.expr()?)
        } else {
            None
        };
        self.expect_p("{")?;
        let mut methods = Vec::new();
        while !self.eat_p("}") {
            let mut kind = MethodKind::Normal;
            if self.eat_kw("static") {
                kind = MethodKind::Static;
            }
            let key = match self.bump() {
                T::Ident(s) => s,
                T::Str(s) => s,
                T::Num(n) => format!("{n}"),
                T::Kw(k) => k.to_string(),
                t => return Err(format!("bad class member {t:?}")),
            };
            if key == "get" && !self.at_p("(") {
                let k2 = self.ident()?;
                self.expect_p("(")?;
                self.expect_p(")")?;
                let body = self.block()?;
                methods.push((
                    k2.clone(),
                    Rc::new(FuncDef {
                        name: Some(k2),
                        params: vec![],
                        body,
                        is_arrow: false,
                    }),
                    MethodKind::Get,
                ));
                continue;
            }
            if key == "set" && !self.at_p("(") {
                let k2 = self.ident()?;
                self.expect_p("(")?;
                let p1 = self.ident()?;
                self.expect_p(")")?;
                let body = self.block()?;
                methods.push((
                    k2.clone(),
                    Rc::new(FuncDef {
                        name: Some(k2),
                        params: vec![p1],
                        body,
                        is_arrow: false,
                    }),
                    MethodKind::Set,
                ));
                continue;
            }
            let is_ctor = key == "constructor";
            let f = {
                self.expect_p("(")?;
                let mut params = Vec::new();
                if !self.at_p(")") {
                    loop {
                        if self.eat_p("...") {
                            let n = self.ident()?;
                            params.push(format!("...{n}"));
                            break;
                        }
                        params.push(self.ident()?);
                        if self.eat_p("=") {
                            let _ = self.expr()?;
                        }
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                }
                self.expect_p(")")?;
                let body = self.block()?;
                FuncDef {
                    name: Some(key.clone()),
                    params,
                    body,
                    is_arrow: false,
                }
            };
            methods.push((
                key,
                Rc::new(f),
                if is_ctor { MethodKind::Ctor } else { kind },
            ));
        }
        Ok(ClassDef {
            name,
            extends,
            methods,
        })
    }

    // ---- expressions ----
    pub fn expr(&mut self) -> Result<E, String> {
        self.assign()
    }

    fn assign(&mut self) -> Result<E, String> {
        // arrow fn detection: ident => or (params) =>
        if matches!(self.peek(), T::Ident(_)) && matches!(self.peek2(), T::Punct("=>")) {
            let name = self.ident()?;
            self.i += 1;
            let body = self.arrow_body()?;
            return Ok(E::Arrow(Rc::new(FuncDef {
                name: None,
                params: vec![name],
                body,
                is_arrow: true,
            })));
        }
        let e = self.cond()?;
        match self.peek().clone() {
            T::Punct(op)
                if matches!(
                    op,
                    "=" | "+="
                        | "-="
                        | "*="
                        | "/="
                        | "%="
                        | "<<="
                        | ">>="
                        | ">>>="
                        | "&="
                        | "|="
                        | "^="
                        | "**="
                        | "&&="
                        | "||="
                        | "??="
                ) =>
            {
                self.i += 1;
                // destructuring assignment
                if matches!(e, E::Arr(_) | E::Obj(_)) && op == "=" {
                    let rhs = self.assign()?;
                    return Ok(E::Assign("=destr", Box::new(e), Box::new(rhs)));
                }
                let rhs = self.assign()?;
                Ok(E::Assign(op, Box::new(e), Box::new(rhs)))
            }
            T::Punct("=>") => {
                // (a,b) => — e must have been a paren/comma expr
                self.i += 1;
                let params = match e {
                    E::Ident(s) => vec![s],
                    E::Paren(pe) => match *pe {
                        E::Seq(items) => items
                            .into_iter()
                            .map(|x| match x {
                                E::Ident(s) => Ok(s),
                                _ => Err("bad arrow param".to_string()),
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                        E::Ident(s) => vec![s],
                        _ => return Err("bad arrow params".into()),
                    },
                    E::Seq(items) => items
                        .into_iter()
                        .map(|x| match x {
                            E::Ident(s) => Ok(s),
                            _ => Err("bad arrow param".to_string()),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => return Err("bad arrow params".into()),
                };
                let body = self.arrow_body()?;
                Ok(E::Arrow(Rc::new(FuncDef {
                    name: None,
                    params,
                    body,
                    is_arrow: true,
                })))
            }
            _ => Ok(e),
        }
    }

    /// Speculatively parse `( params ) =>` — returns None if not an arrow.
    /// Caller must restore self.i on None.
    fn try_arrow_params(&mut self) -> Option<FuncDef> {
        // caller already consumed `(`; self.i is at first param or `)`
        let save = self.i;
        let mut params = Vec::new();
        let ok = (|| -> bool {
            if self.at_p(")") {
                self.i += 1;
                return true;
            }
            loop {
                if self.eat_p("...") {
                    match self.ident() {
                        Ok(n) => {
                            params.push(format!("...{n}"));
                            if self.eat_p(")") {
                                return true;
                            }
                            return false;
                        }
                        Err(_) => return false,
                    }
                }
                match self.bump() {
                    T::Ident(s2) => params.push(s2),
                    _ => return false,
                }
                if self.eat_p("=") {
                    if self.assign().is_err() {
                        return false;
                    }
                }
                if self.eat_p(",") {
                    continue;
                }
                if self.eat_p(")") {
                    return true;
                }
                return false;
            }
        })();
        if !ok || !self.at_p("=>") {
            self.i = save;
            return None;
        }
        self.i += 1;
        let body = self.arrow_body().ok()?;
        Some(FuncDef {
            name: None,
            params,
            body,
            is_arrow: true,
        })
    }

    fn arrow_body(&mut self) -> Result<Vec<S>, String> {
        if self.at_p("{") {
            self.block()
        } else {
            let e = self.assign()?;
            Ok(vec![S::Return(Some(e))])
        }
    }

    fn cond(&mut self) -> Result<E, String> {
        let c = self.binary(0)?;
        if self.eat_p("?") {
            let a = self.assign()?;
            self.expect_p(":")?;
            let b = self.assign()?;
            return Ok(E::Ternary(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    fn binary(&mut self, min_prec: u8) -> Result<E, String> {
        let mut l = self.unary()?;
        loop {
            let (op, prec) = match self.peek() {
                T::Punct("||") => ("||", 1),
                T::Punct("??") => ("??", 1),
                T::Punct("&&") => ("&&", 2),
                T::Punct("|") => ("|", 3),
                T::Punct("^") => ("^", 4),
                T::Punct("&") => ("&", 5),
                T::Punct("==") => ("==", 6),
                T::Punct("!=") => ("!=", 6),
                T::Punct("===") => ("===", 6),
                T::Punct("!==") => ("!==", 6),
                T::Punct("<") => ("<", 7),
                T::Punct(">") => (">", 7),
                T::Punct("<=") => ("<=", 7),
                T::Punct(">=") => (">=", 7),
                T::Kw("instanceof") => ("instanceof", 7),
                T::Kw("in") => ("in", 7),
                T::Punct("<<") => ("<<", 8),
                T::Punct(">>") => (">>", 8),
                T::Punct(">>>") => (">>>", 8),
                T::Punct("+") => ("+", 9),
                T::Punct("-") => ("-", 9),
                T::Punct("*") => ("*", 10),
                T::Punct("/") => ("/", 10),
                T::Punct("%") => ("%", 10),
                T::Punct("**") => ("**", 11),
                _ => break,
            };
            if prec < min_prec {
                break;
            }
            self.i += 1;
            // ** is right-assoc
            let next_min = if op == "**" { prec } else { prec + 1 };
            let r = self.binary(next_min)?;
            l = E::Binary(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn unary(&mut self) -> Result<E, String> {
        match self.peek().clone() {
            T::Punct("!") => {
                self.i += 1;
                Ok(E::Unary("!", Box::new(self.unary()?)))
            }
            T::Punct("~") => {
                self.i += 1;
                Ok(E::Unary("~", Box::new(self.unary()?)))
            }
            T::Punct("-") => {
                self.i += 1;
                Ok(E::Unary("-", Box::new(self.unary()?)))
            }
            T::Punct("+") => {
                self.i += 1;
                Ok(E::Unary("+", Box::new(self.unary()?)))
            }
            T::Punct("++") | T::Punct("--") => {
                let op = if matches!(self.peek(), T::Punct("++")) {
                    "++"
                } else {
                    "--"
                };
                self.i += 1;
                let e = self.unary()?;
                Ok(E::Update(op, true, Box::new(e)))
            }
            T::Kw("typeof") => {
                self.i += 1;
                Ok(E::Unary("typeof", Box::new(self.unary()?)))
            }
            T::Kw("void") => {
                self.i += 1;
                Ok(E::Unary("void", Box::new(self.unary()?)))
            }
            T::Kw("delete") => {
                self.i += 1;
                Ok(E::Unary("delete", Box::new(self.unary()?)))
            }
            T::Kw("await") => {
                self.i += 1;
                Ok(E::Await(Box::new(self.unary()?)))
            }
            T::Kw("new") => {
                self.i += 1;
                let callee = self.member_nc()?;
                let args = if self.at_p("(") { self.args()? } else { vec![] };
                let e = E::New(Box::new(callee), args);
                self.member_suffix(e, true)
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> Result<E, String> {
        let e = self.member()?;
        match self.peek() {
            T::Punct("++") => {
                self.i += 1;
                Ok(E::Update("++", false, Box::new(e)))
            }
            T::Punct("--") => {
                self.i += 1;
                Ok(E::Update("--", false, Box::new(e)))
            }
            _ => Ok(e),
        }
    }

    fn member(&mut self) -> Result<E, String> {
        let e = self.primary()?;
        self.member_suffix(e, true)
    }

    /// member access without call parens (for `new` callee parsing).
    fn member_nc(&mut self) -> Result<E, String> {
        let e = self.primary()?;
        self.member_suffix(e, false)
    }

    fn member_suffix(&mut self, mut e: E, call_ok: bool) -> Result<E, String> {
        loop {
            match self.peek().clone() {
                T::Punct(".") => {
                    self.i += 1;
                    let name = match self.bump() {
                        T::Ident(s) => s,
                        T::Kw(k) => k.to_string(),
                        t => return Err(format!("bad member {t:?}")),
                    };
                    e = E::Member(Box::new(e), name);
                }
                T::Punct("?.") => {
                    self.i += 1;
                    if self.at_p("(") {
                        let args = self.args()?;
                        e = E::Call(Box::new(e), args);
                        e = E::Unary("?.call", Box::new(e));
                        continue;
                    }
                    let name = match self.bump() {
                        T::Ident(s) => s,
                        T::Kw(k) => k.to_string(),
                        t => return Err(format!("bad member {t:?}")),
                    };
                    e = E::Binary("?.", Box::new(e), Box::new(E::Str(name)));
                }
                T::Punct("[") => {
                    self.i += 1;
                    let idx = self.expr()?;
                    self.expect_p("]")?;
                    e = E::Index(Box::new(e), Box::new(idx));
                }
                T::Punct("(") if call_ok => {
                    let args = self.args()?;
                    e = E::Call(Box::new(e), args);
                }
                T::Template(_) if call_ok => {
                    // tagged template — treat as call with parts
                    if let T::Template(parts) = self.bump() {
                        let mut es = Vec::new();
                        let mut s = String::new();
                        for p in parts {
                            match p {
                                TPart::Str(t) => s.push_str(&t),
                                TPart::Expr(ts) => es.push(parse_tokens(ts)?),
                            }
                        }
                        es.insert(0, E::Str(s));
                        e = E::Call(Box::new(e), es);
                    }
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn args(&mut self) -> Result<Vec<E>, String> {
        self.expect_p("(")?;
        let mut out = Vec::new();
        if !self.at_p(")") {
            loop {
                if self.eat_p("...") {
                    out.push(E::Spread(Box::new(self.assign()?)));
                } else {
                    out.push(self.assign()?);
                }
                if !self.eat_p(",") {
                    break;
                }
            }
        }
        self.expect_p(")")?;
        Ok(out)
    }

    fn primary(&mut self) -> Result<E, String> {
        match self.bump() {
            T::Num(n) => Ok(E::Num(n)),
            T::Str(s) => Ok(E::Str(s)),
            T::Regex(b, f) => Ok(E::Regex(b, f)),
            T::Kw("true") => Ok(E::Bool(true)),
            T::Kw("false") => Ok(E::Bool(false)),
            T::Kw("null") => Ok(E::Null),
            T::Kw("undefined") => Ok(E::Undef),
            T::Kw("this") => Ok(E::This),
            T::Kw("super") => Ok(E::Ident("super".into())),
            T::Kw("function") => Ok(E::Func(Rc::new(self.func(true)?))),
            T::Kw("class") => Ok(E::Class(Rc::new(self.class()?))),
            T::Kw(k) => Err(format!("unexpected kw {k}")),
            T::Ident(s) => Ok(E::Ident(s)),
            T::Punct("(") => {
                let save = self.i;
                if let Some(def) = self.try_arrow_params() {
                    return Ok(E::Arrow(Rc::new(def)));
                }
                self.i = save;
                let e = self.expr()?;
                // could be seq for arrow — parse commas
                if self.at_p(",") {
                    let mut items = vec![e];
                    while self.eat_p(",") {
                        items.push(self.expr()?);
                    }
                    self.expect_p(")")?;
                    return Ok(E::Paren(Box::new(E::Seq(items))));
                }
                self.expect_p(")")?;
                Ok(E::Paren(Box::new(e)))
            }
            T::Punct("[") => {
                let mut items = Vec::new();
                if !self.at_p("]") {
                    loop {
                        if self.at_p(",") {
                            items.push(None);
                            self.i += 1;
                            continue;
                        }
                        if self.eat_p("...") {
                            items.push(Some(E::Spread(Box::new(self.assign()?))));
                        } else {
                            items.push(Some(self.assign()?));
                        }
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                }
                self.expect_p("]")?;
                Ok(E::Arr(items))
            }
            T::Punct("{") => {
                let mut items = Vec::new();
                while !self.at_p("}") {
                    if self.eat_p("...") {
                        let e = self.assign()?;
                        items.push(("$spread".into(), e));
                    } else {
                        let key = match self.bump() {
                            T::Ident(s) => s,
                            T::Str(s) => s,
                            T::Num(n) => {
                                if n.fract() == 0.0 {
                                    format!("{}", n as i64)
                                } else {
                                    format!("{n}")
                                }
                            }
                            T::Kw(k) => k.to_string(),
                            t => return Err(format!("bad obj key {t:?}")),
                        };
                        // method shorthand
                        if self.at_p("(") {
                            self.i += 1;
                            let mut params = Vec::new();
                            if !self.at_p(")") {
                                loop {
                                    params.push(self.ident()?);
                                    if self.eat_p("=") {
                                        let _ = self.expr()?;
                                    }
                                    if !self.eat_p(",") {
                                        break;
                                    }
                                }
                            }
                            self.expect_p(")")?;
                            let body = self.block()?;
                            items.push((
                                key.clone(),
                                E::Func(Rc::new(FuncDef {
                                    name: Some(key),
                                    params,
                                    body,
                                    is_arrow: false,
                                })),
                            ));
                        } else if self.eat_p(":") {
                            let v = self.assign()?;
                            items.push((key, v));
                        } else {
                            items.push((key.clone(), E::Ident(key)));
                        }
                    }
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p("}")?;
                Ok(E::Obj(items))
            }
            T::Template(parts) => {
                let mut out = Vec::new();
                for p in parts {
                    match p {
                        TPart::Str(s) => out.push(TE::Str(s)),
                        TPart::Expr(ts) => out.push(TE::Expr(parse_tokens(ts)?)),
                    }
                }
                Ok(E::Template(out))
            }
            t => Err(format!("unexpected token {t:?}")),
        }
    }
}
