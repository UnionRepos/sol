//! DOM bindings: document, elements, events, classList, style,
//! location, storage. Element wrappers are `ObjKind::Dom` objects;
//! dynamic properties resolve through `get_prop`/`set_prop` hooks
//! called from the evaluator.

use crate::builtins::{alloc_arr, alloc_ord, native};
use crate::eval::{Res, I};
use crate::rt::Throw;
use crate::rt::{Obj, ObjKind, Rt, V};
use sol_dom::{NodeData, NodeId};
use std::cell::RefCell;
use std::rc::Rc;

type Doc = Rc<RefCell<sol_dom::Document>>;

fn put_fn<F>(rt: &mut Rt, o: usize, name: &str, f: F)
where
    F: Fn(&mut I, V, &[V]) -> Result<V, Throw> + 'static,
{
    let v = native(rt, name, f);
    rt.set_ne(o, name, v);
}

fn arg(args: &[V], i: usize) -> V {
    args.get(i).cloned().unwrap_or(V::Undef)
}

fn doc_of(rt: &Rt) -> Option<Doc> {
    rt.doc.clone()
}

/// Wrap a DOM node as a JS object.
pub fn wrap(rt: &mut Rt, node: NodeId) -> V {
    let is_doc = rt
        .doc
        .as_ref()
        .map(|d| d.borrow().root == node)
        .unwrap_or(false);
    let proto = if is_doc { rt.node_proto } else { rt.dom_proto };
    V::Obj(rt.alloc(Obj::new(ObjKind::Dom { node }, proto)))
}

fn wrap_doc(rt: &mut Rt) -> V {
    match &rt.doc {
        Some(d) => {
            let root = d.borrow().root;
            V::Obj(rt.alloc(Obj::new(ObjKind::Dom { node: root }, rt.node_proto)))
        }
        None => V::Null,
    }
}

fn nodelist(rt: &mut Rt, nodes: Vec<NodeId>) -> V {
    let o = rt.alloc(Obj::new(ObjKind::NodeList(nodes), rt.dom_proto));
    let v = V::Obj(o);
    put_fn(rt, o, "item", move |i, _t, a| {
        let n = arg(a, 0).to_num(i.rt) as usize;
        if let V::Obj(r) = &_t {
            let hit = if let ObjKind::NodeList(ns) = &i.rt.obj(*r).kind {
                ns.get(n).copied()
            } else {
                None
            };
            if let Some(id) = hit {
                return Ok(wrap(i.rt, id));
            }
        }
        Ok(V::Null)
    });
    put_fn(rt, o, "forEach", |i, t, a| {
        let f = arg(a, 0);
        if let V::Obj(r) = t.clone() {
            if let ObjKind::NodeList(ns) = &i.rt.obj(r).kind {
                let ns = ns.clone();
                for (k, n) in ns.iter().enumerate() {
                    let el = wrap(i.rt, *n);
                    i.call(f.clone(), t.clone(), vec![el, V::Num(k as f64), t.clone()])?;
                }
            }
        }
        Ok(V::Undef)
    });
    v
}

fn this_node(i: &I, this: &V) -> Option<NodeId> {
    if let V::Obj(r) = this {
        match &i.rt.obj(*r).kind {
            ObjKind::Dom { node } => Some(*node),
            ObjKind::StyleObj { node } | ObjKind::ClassList { node } => Some(*node),
            _ => None,
        }
    } else {
        None
    }
}

// ---------- generic element methods (live on dom_proto) ----------

fn e_getattr(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Null);
    };
    let name = arg(args, 0).to_str(i.rt).to_ascii_lowercase();
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Null);
    };
    let v = d.borrow().attr(n, &name).map(|v| v.to_string());
    Ok(match v {
        Some(v) => V::s(v),
        None => V::Null,
    })
}
fn e_setattr(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Undef);
    };
    let name = arg(args, 0).to_str(i.rt).to_ascii_lowercase();
    let v = arg(args, 1).to_str(i.rt);
    if let Some(d) = doc_of(i.rt) {
        d.borrow_mut().set_attribute(n, &name, &v);
    }
    Ok(V::Undef)
}
fn e_rmattr(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Undef);
    };
    let name = arg(args, 0).to_str(i.rt).to_ascii_lowercase();
    if let Some(d) = doc_of(i.rt) {
        d.borrow_mut().remove_attribute(n, &name);
    }
    Ok(V::Undef)
}
fn e_hasattr(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Bool(false));
    };
    let name = arg(args, 0).to_str(i.rt).to_ascii_lowercase();
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Bool(false));
    };
    let has = d.borrow().attr(n, &name).is_some();
    Ok(V::Bool(has))
}
fn e_togattr(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Bool(false));
    };
    let name = arg(args, 0).to_str(i.rt).to_ascii_lowercase();
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Bool(false));
    };
    let mut d = d.borrow_mut();
    if d.attr(n, &name).is_some() {
        d.remove_attribute(n, &name);
        Ok(V::Bool(false))
    } else {
        d.set_attribute(n, &name, "");
        Ok(V::Bool(true))
    }
}
fn e_attrnames(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let _ = args;
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Obj(alloc_arr(i.rt, vec![])));
    };
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Obj(alloc_arr(i.rt, vec![])));
    };
    let names: Vec<V> = d
        .borrow()
        .node(n)
        .element()
        .map(|e| e.attrs.iter().map(|(k, _)| V::s(k.to_string())).collect())
        .unwrap_or_default();
    Ok(V::Obj(alloc_arr(i.rt, names)))
}

fn e_append(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Null);
    };
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Null);
    };
    for a in args {
        match a {
            V::Obj(r) => {
                if let ObjKind::Dom { node } = i.rt.obj(*r).kind {
                    d.borrow_mut().append(n, node);
                }
            }
            v => {
                let t = d.borrow_mut().create_text(&v.to_str(i.rt));
                d.borrow_mut().append(n, t);
            }
        }
    }
    Ok(V::Null)
}
fn e_appendchild(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Null);
    };
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Null);
    };
    if let V::Obj(r) = arg(args, 0) {
        if let ObjKind::Dom { node } = i.rt.obj(r).kind {
            d.borrow_mut().append(n, node);
        }
        return Ok(V::Obj(r));
    }
    Ok(V::Null)
}
fn e_removechild(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Null);
    };
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Null);
    };
    if let V::Obj(r) = arg(args, 0) {
        if let ObjKind::Dom { node } = i.rt.obj(r).kind {
            if d.borrow().parent(node) == Some(n) {
                d.borrow_mut().detach(node);
            }
        }
        return Ok(V::Obj(r));
    }
    Ok(V::Null)
}
fn e_insertbefore(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Null);
    };
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Null);
    };
    if let V::Obj(r) = arg(args, 0) {
        if let ObjKind::Dom { node } = i.rt.obj(r).kind {
            let before = match arg(args, 1) {
                V::Obj(br) => match i.rt.obj(br).kind {
                    ObjKind::Dom { node } => Some(node),
                    _ => None,
                },
                _ => None,
            };
            match before {
                Some(b) => d.borrow_mut().insert_before(n, node, b),
                None => d.borrow_mut().append(n, node),
            }
        }
        return Ok(V::Obj(r));
    }
    Ok(V::Null)
}
fn e_replacechild(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Null);
    };
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Null);
    };
    let (new, old) = (arg(args, 0), arg(args, 1));
    if let (V::Obj(nr), V::Obj(or)) = (&new, &old) {
        let nn = if let ObjKind::Dom { node } = &i.rt.obj(*nr).kind {
            Some(*node)
        } else {
            None
        };
        let on = if let ObjKind::Dom { node } = &i.rt.obj(*or).kind {
            Some(*node)
        } else {
            None
        };
        if let (Some(nn), Some(on)) = (nn, on) {
            let mut d = d.borrow_mut();
            d.insert_before(n, nn, on);
            d.detach(on);
        }
        return Ok(old);
    }
    Ok(V::Null)
}
fn e_clonenode(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Null);
    };
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Null);
    };
    let deep = arg(args, 0).truthy(i.rt);
    fn clone_rec(d: &mut sol_dom::Document, n: NodeId, deep: bool) -> NodeId {
        enum Tmp {
            El(Box<str>, Vec<(Box<str>, String)>, sol_dom::Namespace),
            Text(String),
            Comment(String),
            Other,
        }
        let tmp = match &d.node(n).data {
            NodeData::Element(e) => Tmp::El(e.tag.clone(), e.attrs.clone(), e.namespace),
            NodeData::Text(t) => Tmp::Text(t.clone()),
            NodeData::Comment(t) => Tmp::Comment(t.clone()),
            _ => Tmp::Other,
        };
        let new = match tmp {
            Tmp::El(tag, attrs, ns) => {
                let c = d.create_element(&tag);
                if let Some(el) = d.node_mut(c).element_mut() {
                    el.attrs = attrs;
                    el.namespace = ns;
                }
                c
            }
            Tmp::Text(t) => d.create_text(&t),
            Tmp::Comment(t) => d.create_comment(&t),
            Tmp::Other => d.create_element("span"),
        };
        if deep {
            let kids = d.children(n).to_vec();
            for k in kids {
                let kc = clone_rec(d, k, true);
                d.append(new, kc);
            }
        }
        new
    }
    let c = clone_rec(&mut d.borrow_mut(), n, deep);
    Ok(wrap(i.rt, c))
}
fn e_contains(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Bool(false));
    };
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Bool(false));
    };
    if let V::Obj(r) = arg(args, 0) {
        let on = if let ObjKind::Dom { node } = &i.rt.obj(r).kind {
            Some(*node)
        } else {
            None
        };
        if let Some(node) = on {
            if node == n {
                return Ok(V::Bool(true));
            }
            let d = d.borrow();
            for a in d.ancestors(node) {
                if a == n {
                    return Ok(V::Bool(true));
                }
            }
        }
    }
    Ok(V::Bool(false))
}
fn e_haschildren(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Bool(false));
    };
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Bool(false));
    };
    let has = !d.borrow().children(n).is_empty();
    Ok(V::Bool(has))
}

fn matches_sel(doc: &sol_dom::Document, el: NodeId, sel_str: &str) -> bool {
    if let Some(sel) = sol_css::parse_selector(sel_str) {
        sol_css::style::matches(doc, el, &sel, &sol_css::NoState)
    } else {
        false
    }
}

fn query_sel(doc: &sol_dom::Document, root: NodeId, sel_str: &str, all: bool) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if doc.node(n).is_element() && n != root && matches_sel(doc, n, sel_str) {
            out.push(n);
            if !all {
                return out;
            }
        }
        for k in doc.children(n).iter().rev() {
            stack.push(*k);
        }
    }
    out
}

fn insert_before_sibling(d: &mut sol_dom::Document, node: NodeId, new: NodeId) {
    if let Some(p) = d.parent(node) {
        d.insert_before(p, new, node);
    }
}
fn insert_after_sibling(d: &mut sol_dom::Document, node: NodeId, new: NodeId) {
    if let Some(p) = d.parent(node) {
        match d.next_sibling(node) {
            Some(s) => d.insert_before(p, new, s),
            None => d.append(p, new),
        }
    }
}

fn e_query_impl(i: &mut I, this: V, args: &[V], all: bool) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(if all { nodelist(i.rt, vec![]) } else { V::Null });
    };
    let sel = arg(args, 0).to_str(i.rt);
    let Some(d) = doc_of(i.rt) else {
        return Ok(if all { nodelist(i.rt, vec![]) } else { V::Null });
    };
    let hits = query_sel(&d.borrow(), n, &sel, all);
    if all {
        Ok(nodelist(i.rt, hits))
    } else {
        Ok(hits.first().map(|&x| wrap(i.rt, x)).unwrap_or(V::Null))
    }
}
fn e_qs(i: &mut I, this: V, args: &[V]) -> Res<V> {
    e_query_impl(i, this, args, false)
}
fn e_qsa(i: &mut I, this: V, args: &[V]) -> Res<V> {
    e_query_impl(i, this, args, true)
}
fn e_tagname(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Obj(alloc_arr(i.rt, vec![])));
    };
    let t = arg(args, 0).to_str(i.rt).to_ascii_lowercase();
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Obj(alloc_arr(i.rt, vec![])));
    };
    let d = d.borrow();
    let mut out = Vec::new();
    for x in d.descendants(n) {
        if d.is_tag(x, &t) || t == "*" && d.node(x).is_element() {
            out.push(x);
        }
    }
    Ok(nodelist(i.rt, out))
}
fn e_classname(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Obj(alloc_arr(i.rt, vec![])));
    };
    let c = arg(args, 0).to_str(i.rt);
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Obj(alloc_arr(i.rt, vec![])));
    };
    let d = d.borrow();
    let mut out = Vec::new();
    for x in d.descendants(n) {
        if d.node(x)
            .element()
            .map(|e| e.has_class(&c))
            .unwrap_or(false)
        {
            out.push(x);
        }
    }
    Ok(nodelist(i.rt, out))
}
fn e_matches(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Bool(false));
    };
    let sel = arg(args, 0).to_str(i.rt);
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Bool(false));
    };
    let m = matches_sel(&d.borrow(), n, &sel);
    Ok(V::Bool(m))
}
fn e_closest(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Null);
    };
    let sel = arg(args, 0).to_str(i.rt);
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Null);
    };
    let d = d.borrow();
    let mut cur = Some(n);
    while let Some(x) = cur {
        if d.node(x).is_element() && matches_sel(&d, x, &sel) {
            let hit = x;
            drop(d);
            return Ok(wrap(i.rt, hit));
        }
        cur = d.parent(x);
    }
    Ok(V::Null)
}
fn e_remove(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Undef);
    };
    if let Some(d) = doc_of(i.rt) {
        d.borrow_mut().detach(n);
    }
    Ok(V::Undef)
}
fn e_before(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Undef);
    };
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Undef);
    };
    for a in args {
        match a {
            V::Obj(r) => {
                if let ObjKind::Dom { node } = i.rt.obj(*r).kind {
                    insert_before_sibling(&mut d.borrow_mut(), n, node);
                }
            }
            v => {
                let t = d.borrow_mut().create_text(&v.to_str(i.rt));
                insert_before_sibling(&mut d.borrow_mut(), n, t);
            }
        }
    }
    Ok(V::Undef)
}
fn e_after(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Undef);
    };
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Undef);
    };
    for a in args {
        match a {
            V::Obj(r) => {
                if let ObjKind::Dom { node } = i.rt.obj(*r).kind {
                    insert_after_sibling(&mut d.borrow_mut(), n, node);
                }
            }
            v => {
                let t = d.borrow_mut().create_text(&v.to_str(i.rt));
                insert_after_sibling(&mut d.borrow_mut(), n, t);
            }
        }
    }
    Ok(V::Undef)
}
fn e_replacewith(i: &mut I, this: V, args: &[V]) -> Res<V> {
    e_after(i, this.clone(), args)?;
    e_remove(i, this, &[])
}
fn e_insertadjhtml(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Undef);
    };
    let pos = arg(args, 0).to_str(i.rt).to_ascii_lowercase();
    let html = arg(args, 1).to_str(i.rt);
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Undef);
    };
    let mut d = d.borrow_mut();
    let tag = d.tag(n).to_string();
    let kids = sol_html::parse_fragment(&html, &tag, &mut d);
    match pos.as_str() {
        "beforeend" | "" => {
            for k in kids {
                d.append(n, k);
            }
        }
        "afterbegin" => {
            let first = d.children(n).first().copied();
            for (idx, k) in kids.iter().enumerate() {
                match first {
                    Some(f) => d.insert_before(n, *k, f),
                    None => d.append(n, *k),
                }
                let _ = idx;
            }
        }
        "beforebegin" => {
            for k in kids {
                insert_before_sibling(&mut d, n, k);
            }
        }
        "afterend" => {
            for k in kids.into_iter().rev() {
                insert_after_sibling(&mut d, n, k);
            }
        }
        _ => {}
    }
    Ok(V::Undef)
}
fn e_normalize(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Undef);
    };
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Undef);
    };
    let mut d = d.borrow_mut();
    let kids = d.children(n).to_vec();
    // merge adjacent text nodes
    let mut prev_text: Option<NodeId> = None;
    for k in kids {
        let is_text = d.node(k).is_text();
        if is_text {
            if let Some(p) = prev_text {
                let t = d.text_content(k);
                if let NodeData::Text(pt) = &mut d.node_mut(p).data {
                    pt.push_str(&t);
                }
                d.detach(k);
            } else {
                prev_text = Some(k);
            }
        } else {
            prev_text = None;
        }
    }
    Ok(V::Undef)
}
fn e_rect(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Obj(alloc_ord(i.rt)));
    };
    let o = alloc_ord(i.rt);
    let (x, y, w, h) =
        i.rt.layout_query
            .as_ref()
            .map(|q| q(n))
            .unwrap_or((0.0, 0.0, 0.0, 0.0));
    i.rt.set(o, "x", V::Num(x as f64));
    i.rt.set(o, "y", V::Num(y as f64));
    i.rt.set(o, "width", V::Num(w as f64));
    i.rt.set(o, "height", V::Num(h as f64));
    i.rt.set(o, "top", V::Num(y as f64));
    i.rt.set(o, "left", V::Num(x as f64));
    i.rt.set(o, "right", V::Num((x + w) as f64));
    i.rt.set(o, "bottom", V::Num((y + h) as f64));
    Ok(V::Obj(o))
}
fn e_scrollintoview(i: &mut I, _t: V, _a: &[V]) -> Res<V> {
    let _ = i;
    Ok(V::Undef)
}

// ---------- events ----------

fn e_addlistener(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Undef);
    };
    let ty = arg(args, 0).to_str(i.rt);
    if let V::Obj(cb) = arg(args, 1) {
        i.rt.listeners.push((n, ty, cb));
    }
    Ok(V::Undef)
}
fn e_rmlistener(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Undef);
    };
    let ty = arg(args, 0).to_str(i.rt);
    if let V::Obj(cb) = arg(args, 1) {
        i.rt.listeners
            .retain(|(x, t, c)| !(*x == n && *t == ty && *c == cb));
    }
    Ok(V::Undef)
}
fn e_click(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Undef);
    };
    dispatch(i, n, "click");
    Ok(V::Undef)
}
fn e_focus(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Undef);
    };
    dispatch(i, n, "focus");
    Ok(V::Undef)
}
fn e_blur(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Undef);
    };
    dispatch(i, n, "blur");
    Ok(V::Undef)
}
fn e_dispatch(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Bool(false));
    };
    if let V::Obj(r) = arg(args, 0) {
        if let ObjKind::Event { name, .. } = &i.rt.obj(r).kind {
            let ty = name.clone();
            dispatch(i, n, &ty);
            return Ok(V::Bool(true));
        }
    }
    Ok(V::Bool(false))
}

fn mk_event(i: &mut I, ty: &str, target: NodeId) -> usize {
    let tv = wrap(i.rt, target);
    let proto = i.rt.obj_proto;
    let o = i.rt.alloc(Obj::new(
        ObjKind::Event {
            name: ty.to_string(),
            target: RefCell::new(tv.clone()),
            current: RefCell::new(V::Undef),
            flags: std::cell::Cell::new(0),
        },
        proto,
    ));
    i.rt.set(o, "type", V::s(ty));
    i.rt.set(o, "target", tv);
    i.rt.set(o, "bubbles", V::Bool(true));
    i.rt.set(o, "cancelable", V::Bool(true));
    i.rt.set(o, "defaultPrevented", V::Bool(false));
    i.rt.set(o, "timeStamp", V::Num(sol_util::now_ms() as f64));
    put_fn(i.rt, o, "preventDefault", move |i2, _t, _a| {
        if let ObjKind::Event { flags, .. } = &i2.rt.obj(o).kind {
            flags.set(flags.get() | 1);
            i2.rt.set(o, "defaultPrevented", V::Bool(true));
        }
        Ok(V::Undef)
    });
    put_fn(i.rt, o, "stopPropagation", move |i2, _t, _a| {
        if let ObjKind::Event { flags, .. } = &i2.rt.obj(o).kind {
            flags.set(flags.get() | 2);
        }
        Ok(V::Undef)
    });
    put_fn(i.rt, o, "stopImmediatePropagation", move |i2, _t, _a| {
        if let ObjKind::Event { flags, .. } = &i2.rt.obj(o).kind {
            flags.set(flags.get() | 4);
        }
        Ok(V::Undef)
    });
    o
}

/// Dispatch an event at `node` — bubbles up to the document root.
pub fn dispatch(i: &mut I, node: NodeId, ty: &str) {
    // path: node → ancestors → document root
    let mut path = vec![node];
    if let Some(d) = doc_of(i.rt) {
        let d = d.borrow();
        for a in d.ancestors(node) {
            path.push(a);
        }
    }
    let ev = mk_event(i, ty, node);
    for n in path {
        if let ObjKind::Event { flags, .. } = &i.rt.obj(ev).kind {
            if flags.get() & 2 != 0 {
                break;
            }
        }
        let cbs: Vec<usize> =
            i.rt.listeners
                .iter()
                .filter(|(x, t, _)| *x == n && t == ty)
                .map(|(_, _, c)| *c)
                .collect();
        if cbs.is_empty() {
            continue;
        }
        let cur = wrap(i.rt, n);
        if let ObjKind::Event { current, .. } = &i.rt.obj(ev).kind {
            *current.borrow_mut() = cur;
        }
        for cb in cbs {
            if let ObjKind::Event { flags, .. } = &i.rt.obj(ev).kind {
                if flags.get() & 4 != 0 {
                    break;
                }
            }
            let _ = i.call(V::Obj(cb), V::Undef, vec![V::Obj(ev)]);
        }
    }
}

// ---------- classList ----------

fn cl_add(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Undef);
    };
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Undef);
    };
    let mut d = d.borrow_mut();
    for a in args {
        let c = a.to_str(i.rt);
        let mut cls: Vec<String> = d
            .attr(n, "class")
            .map(|s| s.split_ascii_whitespace().map(|x| x.to_string()).collect())
            .unwrap_or_default();
        if !cls.contains(&c) {
            cls.push(c);
        }
        d.set_attribute(n, "class", &cls.join(" "));
    }
    Ok(V::Undef)
}
fn cl_remove(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Undef);
    };
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Undef);
    };
    let mut d = d.borrow_mut();
    for a in args {
        let c = a.to_str(i.rt);
        let cls: Vec<String> = d
            .attr(n, "class")
            .map(|s| s.split_ascii_whitespace().map(|x| x.to_string()).collect())
            .unwrap_or_default();
        d.set_attribute(
            n,
            "class",
            &cls.into_iter()
                .filter(|x| *x != c)
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    Ok(V::Undef)
}
fn cl_toggle(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Bool(false));
    };
    let c = arg(args, 0).to_str(i.rt);
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Bool(false));
    };
    let has = d
        .borrow()
        .node(n)
        .element()
        .map(|e| e.has_class(&c))
        .unwrap_or(false);
    if has {
        cl_remove(i, this, args)?;
        Ok(V::Bool(false))
    } else {
        cl_add(i, this, args)?;
        Ok(V::Bool(true))
    }
}
fn cl_contains(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Bool(false));
    };
    let c = arg(args, 0).to_str(i.rt);
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Bool(false));
    };
    let has = d
        .borrow()
        .node(n)
        .element()
        .map(|e| e.has_class(&c))
        .unwrap_or(false);
    Ok(V::Bool(has))
}

// ---------- style proxy ----------

fn style_get(node: NodeId, d: &sol_dom::Document, name: &str) -> String {
    let kebab = camel_to_kebab(name);
    d.attr(node, "style")
        .map(|s| {
            s.split(';')
                .filter_map(|d2| {
                    let mut it = d2.splitn(2, ':');
                    Some((
                        it.next()?.trim().to_ascii_lowercase(),
                        it.next().unwrap_or("").trim().to_string(),
                    ))
                })
                .find(|(k, _)| *k == kebab)
                .map(|(_, v)| v)
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

fn style_set(node: NodeId, d: &mut sol_dom::Document, name: &str, v: &str) {
    let kebab = camel_to_kebab(name);
    let cur = d.attr(node, "style").unwrap_or("").to_string();
    let mut decls: Vec<(String, String)> = cur
        .split(';')
        .filter_map(|d2| {
            let mut it = d2.splitn(2, ':');
            let k = it.next()?.trim().to_ascii_lowercase();
            if k.is_empty() {
                return None;
            }
            Some((k, it.next().unwrap_or("").trim().to_string()))
        })
        .collect();
    if v.is_empty() {
        decls.retain(|(k, _)| *k != kebab);
    } else if let Some(e) = decls.iter_mut().find(|(k, _)| *k == kebab) {
        e.1 = v.to_string();
    } else {
        decls.push((kebab, v.to_string()));
    }
    let s = decls
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("; ");
    d.set_attribute(node, "style", &s);
}

fn camel_to_kebab(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    if s == "cssFloat" || s == "float" {
        return "float".into();
    }
    out
}

fn st_setprop(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::Undef);
    };
    let name = arg(args, 0).to_str(i.rt);
    let v = arg(args, 1).to_str(i.rt);
    if let Some(d) = doc_of(i.rt) {
        style_set(n, &mut d.borrow_mut(), &name, &v);
    }
    Ok(V::Undef)
}
fn st_getprop(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let Some(n) = this_node(i, &this) else {
        return Ok(V::s(""));
    };
    let name = arg(args, 0).to_str(i.rt);
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::s(""));
    };
    let v = style_get(n, &d.borrow(), &name);
    Ok(V::s(v))
}
fn st_rmprop(i: &mut I, this: V, args: &[V]) -> Res<V> {
    st_setprop(i, this, args)
}

// ---------- document methods ----------

fn d_create_el(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let tag = arg(args, 0).to_str(i.rt);
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Null);
    };
    let n = d.borrow_mut().create_element(&tag);
    Ok(wrap(i.rt, n))
}
fn d_create_text(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let s = arg(args, 0).to_str(i.rt);
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Null);
    };
    let n = d.borrow_mut().create_text(&s);
    Ok(wrap(i.rt, n))
}
fn d_create_comment(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let s = arg(args, 0).to_str(i.rt);
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Null);
    };
    let n = d.borrow_mut().create_comment(&s);
    Ok(wrap(i.rt, n))
}
fn d_byid(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let id = arg(args, 0).to_str(i.rt);
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Null);
    };
    let hit = d.borrow().get_element_by_id(&id);
    Ok(hit.map(|n| wrap(i.rt, n)).unwrap_or(V::Null))
}
fn d_create_event(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let _ty = arg(args, 0).to_str(i.rt);
    let o = alloc_ord(i.rt);
    let init = native(i.rt, "initEvent", move |i2, _t, a| {
        let ty = arg(a, 0).to_str(i2.rt);
        i2.rt.set(o, "type", V::s(ty));
        Ok(V::Undef)
    });
    i.rt.set(o, "initEvent", init);
    Ok(V::Obj(o))
}
fn d_write(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    // document.write during parse is a parser concern; post-load it
    // appends to body as innerHTML
    let html = args.iter().map(|v| v.to_str(i.rt)).collect::<String>();
    let Some(d) = doc_of(i.rt) else {
        return Ok(V::Undef);
    };
    if let Some(b) = d.borrow().body() {
        let mut d = d.borrow_mut();
        let kids = sol_html::parse_fragment(&html, "body", &mut d);
        for k in kids {
            d.append(b, k);
        }
    }
    Ok(V::Undef)
}

// ---------- location ----------

fn loc_part(rt: &Rt, part: &str) -> String {
    let base = rt
        .doc
        .as_ref()
        .map(|d| d.borrow().base_url.clone())
        .unwrap_or_default();
    if base.is_empty() {
        return String::new();
    }
    match sol_url::Url::parse(&base) {
        Ok(u) => match part {
            "protocol" => format!("{}:", u.scheme),
            "hostname" => u.host.clone(),
            "host" => u.host_and_port(),
            "port" => u.port.map(|p| p.to_string()).unwrap_or_default(),
            "pathname" => u.path.clone(),
            "search" => u.query.map(|q| format!("?{q}")).unwrap_or_default(),
            "hash" => u.fragment.map(|f| format!("#{f}")).unwrap_or_default(),
            "origin" => u.origin(),
            _ => base,
        },
        Err(_) => base,
    }
}

fn loc_assign(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let url = arg(args, 0).to_str(i.rt);
    if let Some(n) = &i.rt.navigate.clone() {
        n(&url);
    }
    Ok(V::Undef)
}
fn loc_reload(i: &mut I, _t: V, _a: &[V]) -> Res<V> {
    let url = loc_part(i.rt, "href");
    if !url.is_empty() {
        if let Some(n) = &i.rt.navigate.clone() {
            n(&url);
        }
    }
    Ok(V::Undef)
}

// ---------- storage ----------

fn st_getitem(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let k = arg(args, 0).to_str(i.rt);
    Ok(match i.rt.storage.borrow().get(&k) {
        Some(v) => V::s(v.clone()),
        None => V::Null,
    })
}
fn st_setitem(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let k = arg(args, 0).to_str(i.rt);
    let v = arg(args, 1).to_str(i.rt);
    i.rt.storage.borrow_mut().insert(k, v);
    Ok(V::Undef)
}
fn st_rmitem(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let k = arg(args, 0).to_str(i.rt);
    i.rt.storage.borrow_mut().remove(&k);
    Ok(V::Undef)
}
fn st_clear(i: &mut I, _t: V, _a: &[V]) -> Res<V> {
    i.rt.storage.borrow_mut().clear();
    Ok(V::Undef)
}
fn st_key(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let n = arg(args, 0).to_num(i.rt) as usize;
    let s = i.rt.storage.borrow();
    Ok(s.keys().nth(n).map(|k| V::s(k.clone())).unwrap_or(V::Null))
}

// ---------- dynamic property dispatch ----------

/// Called by the evaluator when reading a property of a Dom/NodeList/
/// StyleObj/ClassList/Location/Storage/Event object. Returning
/// `Some(v)` short-circuits the generic lookup.
pub fn get_prop(i: &mut I, r: usize, name: &str) -> Res<Option<V>> {
    let (kind_node, kind_tag): (Option<NodeId>, u8) = match &i.rt.obj(r).kind {
        ObjKind::Dom { node } => (Some(*node), 0),
        ObjKind::NodeList(ns) => {
            let ns = ns.clone();
            return Ok(match name {
                "length" => Some(V::Num(ns.len() as f64)),
                _ if name
                    .parse::<usize>()
                    .ok()
                    .map(|x| x < ns.len())
                    .unwrap_or(false) =>
                {
                    let x = name.parse::<usize>().unwrap();
                    Some(wrap(i.rt, ns[x]))
                }
                _ => None,
            });
        }
        ObjKind::StyleObj { node } => {
            let n = *node;
            return Ok(match name {
                "setProperty" | "getPropertyValue" | "removeProperty" | "cssText" => None,
                _ => {
                    let Some(d) = doc_of(i.rt) else {
                        return Ok(None);
                    };
                    let d = d.borrow();
                    if name
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_lowercase())
                        .unwrap_or(false)
                        && !name.starts_with('_')
                    {
                        let v = style_get(n, &d, name);
                        Some(if v.is_empty() { V::s("") } else { V::s(v) })
                    } else {
                        None
                    }
                }
            });
        }
        ObjKind::ClassList { node } => {
            let n = *node;
            return Ok(match name {
                "length" => {
                    let Some(d) = doc_of(i.rt) else {
                        return Ok(None);
                    };
                    let cnt = d
                        .borrow()
                        .node(n)
                        .element()
                        .map(|e| e.classes().len())
                        .unwrap_or(0);
                    Some(V::Num(cnt as f64))
                }
                "value" | "toString" => {
                    let Some(d) = doc_of(i.rt) else {
                        return Ok(None);
                    };
                    let x = V::s(d.borrow().attr(n, "class").unwrap_or("").to_string());
                    Some(x)
                }
                _ => {
                    let Some(d) = doc_of(i.rt) else {
                        return Ok(None);
                    };
                    let has = d
                        .borrow()
                        .node(n)
                        .element()
                        .map(|e| e.has_class(name))
                        .unwrap_or(false);
                    if has {
                        Some(V::s(name))
                    } else {
                        None
                    }
                }
            });
        }
        ObjKind::Location => {
            return Ok(match name {
                "href" | "toString" => Some(V::s(loc_part(i.rt, "href"))),
                "protocol" | "hostname" | "host" | "port" | "pathname" | "search" | "hash"
                | "origin" => Some(V::s(loc_part(i.rt, name))),
                _ => None,
            });
        }
        ObjKind::Storage => {
            return Ok(match name {
                "length" => Some(V::Num(i.rt.storage.borrow().len() as f64)),
                _ => {
                    let v = i.rt.storage.borrow().get(name).cloned();
                    v.map(V::s)
                }
            });
        }
        ObjKind::Event {
            name: ty,
            current,
            flags,
            ..
        } => {
            let ty = ty.clone();
            let cur = current.borrow().clone();
            let fl = flags.get();
            return Ok(match name {
                "type" => Some(V::s(ty)),
                "currentTarget" => Some(cur),
                "eventPhase" => Some(V::Num(3.0)),
                "defaultPrevented" => Some(V::Bool(fl & 1 != 0)),
                _ => None,
            });
        }
        _ => (None, 0),
    };
    let Some(n) = kind_node else {
        let _ = kind_tag;
        return Ok(None);
    };
    let Some(d) = doc_of(i.rt) else {
        return Ok(None);
    };
    let is_document = d.borrow().root == n;
    if is_document {
        return document_prop(i, r, name);
    }
    element_prop(i, n, name)
}

fn document_prop(i: &mut I, _r: usize, name: &str) -> Res<Option<V>> {
    let Some(d) = doc_of(i.rt) else {
        return Ok(None);
    };
    let v = match name {
        "documentElement" => d.borrow().document_element().map(|n| wrap(i.rt, n)),
        "body" => d.borrow().body().map(|n| wrap(i.rt, n)),
        "head" => d.borrow().head().map(|n| wrap(i.rt, n)),
        "title" => Some(V::s(d.borrow().title().unwrap_or_default())),
        "URL" | "documentURI" | "baseURI" => Some(V::s(d.borrow().base_url.clone())),
        "cookie" => Some(V::s(i.rt.cookie.borrow().clone())),
        "readyState" => Some(V::s("complete")),
        "characterSet" | "charset" | "inputEncoding" => Some(V::s("UTF-8")),
        "contentType" => Some(V::s("text/html")),
        "doctype" => Some(V::s("[object DocumentType]")),
        "location" => Some(make_location(i.rt)),
        "domain" => Some(V::s(loc_part(i.rt, "hostname"))),
        "origin" => Some(V::s(loc_part(i.rt, "origin"))),
        "activeElement" => d.borrow().body().map(|n| wrap(i.rt, n)),
        "documentMode" => Some(V::s("css1compat")),
        "hidden" | "webkitHidden" => Some(V::Bool(false)),
        "visibilityState" => Some(V::s("visible")),
        "nodeType" => Some(V::Num(9.0)),
        "nodeName" => Some(V::s("#document")),
        "defaultView" => Some(
            i.rt.global_env
                .borrow()
                .vars
                .borrow()
                .get("window")
                .cloned()
                .unwrap_or(V::Undef),
        ),
        _ => None,
    };
    Ok(v)
}

fn make_location(rt: &mut Rt) -> V {
    let o = rt.alloc(Obj::new(ObjKind::Location, rt.obj_proto));
    put_fn(rt, o, "assign", loc_assign);
    put_fn(rt, o, "replace", loc_assign);
    put_fn(rt, o, "reload", loc_reload);
    V::Obj(o)
}

fn element_prop(i: &mut I, n: NodeId, name: &str) -> Res<Option<V>> {
    let Some(d) = doc_of(i.rt) else {
        return Ok(None);
    };
    let node = d.borrow();
    let is_el = node.node(n).is_element();
    let v = match name {
        "nodeType" => Some(V::Num(node.node(n).node_type() as f64)),
        "nodeName" => Some(V::s(node.node(n).node_name())),
        "tagName" if is_el => Some(V::s(node.tag(n).to_ascii_uppercase())),
        "localName" if is_el => Some(V::s(node.tag(n).to_string())),
        "ownerDocument" => Some(wrap_doc(i.rt)),
        "parentNode" | "parentElement" => node.parent(n).map(|p| wrap(i.rt, p)).or(Some(V::Null)),
        "children" => Some(nodelist(i.rt, node.element_children(n))),
        "childNodes" => Some(nodelist(i.rt, node.children(n).to_vec())),
        "firstChild" => node.children(n).first().map(|&c| wrap(i.rt, c)),
        "lastChild" => node.children(n).last().map(|&c| wrap(i.rt, c)),
        "firstElementChild" => node.first_element_child(n).map(|c| wrap(i.rt, c)),
        "lastElementChild" => node.element_children(n).last().map(|&c| wrap(i.rt, c)),
        "nextSibling" => node.next_sibling(n).map(|s| wrap(i.rt, s)),
        "previousSibling" => node.prev_sibling(n).map(|s| wrap(i.rt, s)),
        "nextElementSibling" => {
            let mut s = node.next_sibling(n);
            while let Some(x) = s {
                if node.node(x).is_element() {
                    break;
                }
                s = node.next_sibling(x);
            }
            s.map(|x| wrap(i.rt, x))
        }
        "previousElementSibling" => {
            let mut s = node.prev_sibling(n);
            while let Some(x) = s {
                if node.node(x).is_element() {
                    break;
                }
                s = node.prev_sibling(x);
            }
            s.map(|x| wrap(i.rt, x))
        }
        "textContent" | "innerText" => Some(V::s(node.text_content(n))),
        "innerHTML" if is_el => Some(V::s(node.inner_html(n))),
        "outerHTML" if is_el => Some(V::s(node.outer_html(n))),
        "nodeValue" | "data" => match &node.node(n).data {
            NodeData::Text(t) | NodeData::Comment(t) => Some(V::s(t.clone())),
            _ => Some(V::Null),
        },
        "classList" if is_el => {
            drop(node);
            let proto = i.rt.dom_proto;
            let o = i.rt.alloc(Obj::new(ObjKind::ClassList { node: n }, proto));
            let cl = V::Obj(o);
            put_fn(i.rt, o, "add", cl_add);
            put_fn(i.rt, o, "remove", cl_remove);
            put_fn(i.rt, o, "toggle", cl_toggle);
            put_fn(i.rt, o, "contains", cl_contains);
            put_fn(i.rt, o, "replace", |i, t, a| {
                cl_remove(i, t.clone(), &a[..1.min(a.len())])?;
                cl_add(i, t, &a[1.min(a.len())..])
            });
            return Ok(Some(cl));
        }
        "style" if is_el => {
            drop(node);
            let proto = i.rt.dom_proto;
            let o = i.rt.alloc(Obj::new(ObjKind::StyleObj { node: n }, proto));
            let st = V::Obj(o);
            put_fn(i.rt, o, "setProperty", st_setprop);
            put_fn(i.rt, o, "getPropertyValue", st_getprop);
            put_fn(i.rt, o, "removeProperty", st_rmprop);
            i.rt.set(
                o,
                "cssText",
                V::s(d.borrow().attr(n, "style").unwrap_or("").to_string()),
            );
            return Ok(Some(st));
        }
        "attributes" if is_el => {
            let attrs: Vec<(Box<str>, String)> = node
                .node(n)
                .element()
                .map(|e| e.attrs.clone())
                .unwrap_or_default();
            drop(node);
            let pairs: Vec<V> = attrs
                .into_iter()
                .map(|(k, v)| {
                    let o = alloc_ord(i.rt);
                    i.rt.set(o, "name", V::s(k.to_string()));
                    i.rt.set(o, "value", V::s(v));
                    V::Obj(o)
                })
                .collect();
            return Ok(Some(V::Obj(alloc_arr(i.rt, pairs))));
        }
        "dataset" if is_el => {
            let o = alloc_ord(i.rt);
            if let Some(e) = node.node(n).element() {
                for (k, v) in &e.attrs {
                    if let Some(rest) = k.strip_prefix("data-") {
                        let camel: String = {
                            let mut out = String::new();
                            let mut up = false;
                            for c in rest.chars() {
                                if c == '-' {
                                    up = true;
                                } else if up {
                                    out.push(c.to_ascii_uppercase());
                                    up = false;
                                } else {
                                    out.push(c);
                                }
                            }
                            out
                        };
                        i.rt.set(o, &camel, V::s(v.clone()));
                    }
                }
            }
            return Ok(Some(V::Obj(o)));
        }
        "className" if is_el => Some(V::s(node.attr(n, "class").unwrap_or("").to_string())),
        "id" if is_el => Some(V::s(node.attr(n, "id").unwrap_or("").to_string())),
        "name" | "type" | "placeholder" | "title" | "alt" | "role" | "lang" | "dir" | "rel"
        | "target" | "method" | "action" | "for" | "min" | "max" | "step" | "size" | "rows"
        | "cols" | "wrap" | "pattern" | "accept" | "autocomplete" | "aria-label" => {
            Some(V::s(node.attr(n, name).unwrap_or("").to_string()))
        }
        "value" => {
            let tag = node.tag(n).to_string();
            Some(V::s(
                node.attr(n, "value")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        if tag == "textarea" {
                            node.text_content(n)
                        } else {
                            String::new()
                        }
                    }),
            ))
        }
        "href" | "src" => {
            let raw = node.attr(n, name).unwrap_or("").to_string();
            // resolve relative to base
            if raw.is_empty()
                || raw.starts_with("http")
                || raw.starts_with("data:")
                || raw.starts_with('/')
                || raw.starts_with('#')
            {
                Some(V::s(raw))
            } else {
                let base = node.base_url.clone();
                Some(V::s(
                    sol_url::Url::parse(&base)
                        .and_then(|u| u.join(&raw))
                        .map(|u| u.serialize())
                        .unwrap_or(raw),
                ))
            }
        }
        "htmlFor" => Some(V::s(node.attr(n, "for").unwrap_or("").to_string())),
        "checked" | "disabled" | "selected" | "required" | "multiple" | "readonly" | "readOnly"
        | "hidden" | "open" | "autofocus" | "draggable" | "contentEditable" | "spellcheck"
        | "async" | "defer" => {
            let attr = if name == "readOnly" {
                "readonly"
            } else if name == "htmlFor" {
                "for"
            } else {
                name
            };
            Some(V::Bool(
                node.node(n)
                    .element()
                    .map(|e| e.has_attr(attr))
                    .unwrap_or(false),
            ))
        }
        "class" => Some(V::s(node.attr(n, "class").unwrap_or("").to_string())),
        "onclick" | "ondblclick" | "onmousedown" | "onmouseup" | "onmouseover" | "onmouseout"
        | "onmousemove" | "onkeydown" | "onkeyup" | "onkeypress" | "oninput" | "onchange"
        | "onsubmit" | "onload" | "onerror" | "onfocus" | "onblur" | "onscroll" | "onresize"
        | "oncontextmenu" | "ontouchstart" | "ontouchend" | "ondragstart" | "ondrop"
        | "onanimationend" | "ontransitionend" => {
            let ty = &name[2..];
            let cb =
                i.rt.listeners
                    .iter()
                    .find(|(x, t, _)| *x == n && t == ty)
                    .map(|(_, _, c)| V::Obj(*c));
            return Ok(cb.or(Some(V::Null)));
        }
        "offsetWidth" | "offsetHeight" | "clientWidth" | "clientHeight" | "scrollWidth"
        | "scrollHeight" | "offsetTop" | "offsetLeft" | "scrollTop" | "scrollLeft"
        | "clientTop" | "clientLeft" => {
            let (x, y, w, h) =
                i.rt.layout_query
                    .as_ref()
                    .map(|q| q(n))
                    .unwrap_or((0.0, 0.0, 0.0, 0.0));
            Some(V::Num(match name {
                "offsetWidth" | "clientWidth" | "scrollWidth" => w,
                "offsetHeight" | "clientHeight" | "scrollHeight" => h,
                "offsetTop" | "clientTop" | "scrollTop" => y,
                "offsetLeft" | "clientLeft" | "scrollLeft" => x,
                _ => 0.0,
            } as f64))
        }
        "offsetParent" => node.parent(n).map(|p| wrap(i.rt, p)),
        _ => None,
    };
    Ok(v)
}

/// Called by the evaluator for property writes on Dom objects.
/// Returns true if the write was handled.
pub fn set_prop(i: &mut I, r: usize, name: &str, v: V) -> Res<bool> {
    let node = match &i.rt.obj(r).kind {
        ObjKind::Dom { node } => Some(*node),
        ObjKind::StyleObj { node } => {
            let n = *node;
            let Some(d) = doc_of(i.rt) else {
                return Ok(true);
            };
            style_set(n, &mut d.borrow_mut(), name, &v.to_str(i.rt));
            return Ok(true);
        }
        ObjKind::ClassList { node } => {
            let n = *node;
            if name == "value" {
                if let Some(d) = doc_of(i.rt) {
                    d.borrow_mut().set_attribute(n, "class", &v.to_str(i.rt));
                }
            }
            return Ok(true);
        }
        ObjKind::Location => {
            if name == "href" || name == "hash" || name == "pathname" || name == "search" {
                let url = if name == "href" {
                    v.to_str(i.rt)
                } else {
                    let mut u = loc_part(i.rt, "href");
                    let part = v.to_str(i.rt);
                    // naive splice for hash/search/pathname
                    if name == "hash" {
                        if let Some(h) = u.find('#') {
                            u.truncate(h);
                        }
                        format!("{u}#{}", part.trim_start_matches('#'))
                    } else if name == "search" {
                        if let Some(h) = u.find('?') {
                            u.truncate(h);
                        }
                        if let Some(h) = u.find('#') {
                            u.truncate(h);
                        }
                        format!("{u}?{}", part.trim_start_matches('?'))
                    } else {
                        part
                    }
                };
                if let Some(nv) = &i.rt.navigate.clone() {
                    nv(&url);
                }
            }
            return Ok(true);
        }
        ObjKind::Storage => {
            i.rt.storage
                .borrow_mut()
                .insert(name.to_string(), v.to_str(i.rt));
            return Ok(true);
        }
        _ => None,
    };
    let Some(n) = node else { return Ok(false) };
    let Some(d) = doc_of(i.rt) else {
        return Ok(false);
    };
    let is_document = d.borrow().root == n;
    if is_document {
        match name {
            "title" => {
                // find-or-create title element in head
                let mut d = d.borrow_mut();
                let t = d
                    .get_elements_by_tag_name("title")
                    .first()
                    .copied()
                    .or_else(|| {
                        if let Some(h) = d.head() {
                            let t = d.create_element("title");
                            d.append(h, t);
                            Some(t)
                        } else {
                            None
                        }
                    });
                if let Some(t) = t {
                    let text = v.to_str(i.rt);
                    d.set_text_content(t, &text);
                }
            }
            "cookie" => {
                *i.rt.cookie.borrow_mut() = v.to_str(i.rt);
            }
            _ => {}
        }
        return Ok(true);
    }
    let mut d = d.borrow_mut();
    match name {
        "textContent" | "innerText" => {
            let t = v.to_str(i.rt);
            d.set_text_content(n, &t);
        }
        "innerHTML" => {
            let html = v.to_str(i.rt);
            // remove children, insert parsed fragment
            for c in d.children(n).to_vec() {
                d.detach(c);
            }
            let tag = d.tag(n).to_string();
            let kids = sol_html::parse_fragment(&html, &tag, &mut d);
            for k in kids {
                d.append(n, k);
            }
        }
        "outerHTML" => {
            let html = v.to_str(i.rt);
            if let Some(p) = d.parent(n) {
                let tag = d.tag(p).to_string();
                let kids = sol_html::parse_fragment(&html, &tag, &mut d);
                for k in &kids {
                    d.insert_before(p, *k, n);
                }
                d.detach(n);
            }
        }
        "className" | "class" => {
            let s = v.to_str(i.rt);
            d.set_attribute(n, "class", &s);
        }
        "nodeValue" | "data" => {
            let s = v.to_str(i.rt);
            if let NodeData::Text(t) | NodeData::Comment(t) = &mut d.node_mut(n).data {
                *t = s;
            }
        }
        "onclick" | "ondblclick" | "onmousedown" | "onmouseup" | "onmouseover" | "onmouseout"
        | "onmousemove" | "onkeydown" | "onkeyup" | "onkeypress" | "oninput" | "onchange"
        | "onsubmit" | "onload" | "onerror" | "onfocus" | "onblur" | "onscroll" | "onresize"
        | "oncontextmenu" | "ontouchstart" | "ontouchend" | "ondragstart" | "ondrop"
        | "onanimationend" | "ontransitionend" => {
            let ty = &name[2..];
            i.rt.listeners.retain(|(x, t, _)| !(x == &n && t == ty));
            if let V::Obj(cb) = v {
                i.rt.listeners.push((n, ty.to_string(), cb));
            }
        }
        "checked" | "disabled" | "selected" | "required" | "multiple" | "readonly" | "readOnly"
        | "hidden" | "open" | "autofocus" | "draggable" | "contentEditable" | "spellcheck"
        | "async" | "defer" => {
            let attr = if name == "readOnly" { "readonly" } else { name };
            if v.truthy(i.rt) {
                d.set_attribute(n, attr, "");
            } else {
                d.remove_attribute(n, attr);
            }
        }
        _ => {
            let s = v.to_str(i.rt);
            d.set_attribute(n, &name.to_ascii_lowercase(), &s);
        }
    }
    Ok(true)
}

// ---------- install ----------

pub fn install(i: &mut I) {
    let rt = &mut *i.rt;
    let g = rt.global_env.clone();
    let dp = rt.dom_proto;
    let np = rt.node_proto;

    // element methods on dom_proto (also usable on document via node_proto)
    for p in [dp, np] {
        for (name, f) in [
            ("getAttribute", e_getattr as fn(&mut I, V, &[V]) -> Res<V>),
            ("setAttribute", e_setattr),
            ("removeAttribute", e_rmattr),
            ("hasAttribute", e_hasattr),
            ("toggleAttribute", e_togattr),
            ("getAttributeNames", e_attrnames),
            ("appendChild", e_appendchild),
            ("removeChild", e_removechild),
            ("insertBefore", e_insertbefore),
            ("replaceChild", e_replacechild),
            ("cloneNode", e_clonenode),
            ("contains", e_contains),
            ("hasChildNodes", e_haschildren),
            ("querySelector", e_qs),
            ("querySelectorAll", e_qsa),
            ("getElementsByTagName", e_tagname),
            ("getElementsByClassName", e_classname),
            ("matches", e_matches),
            ("closest", e_closest),
            ("remove", e_remove),
            ("before", e_before),
            ("after", e_after),
            ("append", e_append),
            ("prepend", |i, t, a| {
                // insert at start
                let Some(n) = this_node(i, &t) else {
                    return Ok(V::Undef);
                };
                let Some(d) = doc_of(i.rt) else {
                    return Ok(V::Undef);
                };
                let first = d.borrow().children(n).first().copied();
                for a in a.iter().rev() {
                    match a {
                        V::Obj(r) => {
                            if let ObjKind::Dom { node } = i.rt.obj(*r).kind {
                                match first {
                                    Some(f) => d.borrow_mut().insert_before(n, node, f),
                                    None => d.borrow_mut().append(n, node),
                                }
                            }
                        }
                        v => {
                            let t2 = d.borrow_mut().create_text(&v.to_str(i.rt));
                            match first {
                                Some(f) => d.borrow_mut().insert_before(n, t2, f),
                                None => d.borrow_mut().append(n, t2),
                            }
                        }
                    }
                }
                Ok(V::Undef)
            }),
            ("replaceWith", e_replacewith),
            ("insertAdjacentHTML", e_insertadjhtml),
            ("normalize", e_normalize),
            ("getBoundingClientRect", e_rect),
            ("scrollIntoView", e_scrollintoview),
            ("addEventListener", e_addlistener),
            ("removeEventListener", e_rmlistener),
            ("dispatchEvent", e_dispatch),
            ("click", e_click),
            ("focus", e_focus),
            ("blur", e_blur),
            ("scroll", e_scrollintoview),
        ] {
            let fv = native(rt, name, f);
            rt.set(p, name, fv);
        }
    }

    // document-only methods on node_proto
    for (name, f) in [
        (
            "createElement",
            d_create_el as fn(&mut I, V, &[V]) -> Res<V>,
        ),
        ("createElementNS", d_create_el),
        ("createTextNode", d_create_text),
        ("createComment", d_create_comment),
        ("createDocumentFragment", |i, _t, _a| {
            let Some(d) = doc_of(i.rt) else {
                return Ok(V::Null);
            };
            let n = d.borrow_mut().create_fragment();
            Ok(wrap(i.rt, n))
        }),
        ("getElementById", d_byid),
        ("createEvent", d_create_event),
        ("write", d_write),
        ("writeln", d_write),
        ("open", |_i, _t, _a| Ok(V::Undef)),
        ("close", |_i, _t, _a| Ok(V::Undef)),
    ] {
        let fv = native(rt, name, f);
        rt.set(np, name, fv);
    }

    // window = globalThis mirror object
    let win = alloc_ord(rt);
    rt.set(win, "self", V::Obj(win));
    rt.set(win, "top", V::Obj(win));
    rt.set(win, "parent", V::Obj(win));
    rt.set(win, "window", V::Obj(win));
    rt.set(win, "frames", V::Obj(win));
    let loc = make_location(rt);
    rt.set(win, "location", loc.clone());
    let (vw, vh) = rt.viewport;
    rt.set(win, "innerWidth", V::Num(vw as f64));
    rt.set(win, "innerHeight", V::Num(vh as f64));
    rt.set(win, "outerWidth", V::Num(vw as f64));
    rt.set(win, "outerHeight", V::Num(vh as f64));
    rt.set(win, "devicePixelRatio", V::Num(1.0));
    rt.set(win, "scrollX", V::Num(0.0));
    rt.set(win, "scrollY", V::Num(0.0));
    rt.set(win, "pageXOffset", V::Num(0.0));
    rt.set(win, "pageYOffset", V::Num(0.0));

    // screen
    let scr = alloc_ord(rt);
    rt.set(scr, "width", V::Num(vw as f64));
    rt.set(scr, "height", V::Num(vh as f64));
    rt.set(scr, "availWidth", V::Num(vw as f64));
    rt.set(scr, "availHeight", V::Num(vh as f64));
    rt.set(scr, "colorDepth", V::Num(24.0));
    rt.set(scr, "pixelDepth", V::Num(24.0));
    rt.set(win, "screen", V::Obj(scr));

    // navigator
    let nav = alloc_ord(rt);
    rt.set(nav, "userAgent", V::s("sol/1.0 (sol_browser)"));
    rt.set(nav, "platform", V::s("Linux"));
    rt.set(nav, "language", V::s("en-US"));
    let langs = alloc_arr(rt, vec![V::s("en-US")]);
    rt.set(nav, "languages", V::Obj(langs));
    rt.set(nav, "onLine", V::Bool(true));
    rt.set(nav, "cookieEnabled", V::Bool(true));
    rt.set(nav, "hardwareConcurrency", V::Num(4.0));
    rt.set(nav, "vendor", V::s("sol"));
    rt.set(nav, "appName", V::s("sol"));
    rt.set(win, "navigator", V::Obj(nav));

    // storage
    let storage = rt.alloc(Obj::new(ObjKind::Storage, rt.obj_proto));
    put_fn(rt, storage, "getItem", st_getitem);
    put_fn(rt, storage, "setItem", st_setitem);
    put_fn(rt, storage, "removeItem", st_rmitem);
    put_fn(rt, storage, "clear", st_clear);
    put_fn(rt, storage, "key", st_key);
    rt.set(win, "localStorage", V::Obj(storage));
    rt.set(win, "sessionStorage", V::Obj(storage));

    // history
    let hist = alloc_ord(rt);
    rt.set(hist, "length", V::Num(1.0));
    put_fn(rt, hist, "back", |_i, _t, _a| Ok(V::Undef));
    put_fn(rt, hist, "forward", |_i, _t, _a| Ok(V::Undef));
    put_fn(rt, hist, "go", |_i, _t, _a| Ok(V::Undef));
    rt.set(win, "history", V::Obj(hist));

    // misc window fns
    put_fn(rt, win, "requestAnimationFrame", |i, _t, a| {
        i.rt.microtasks.push_back(arg(a, 0));
        Ok(V::Num(i.rt.next_timer as f64 - 1.0))
    });
    put_fn(rt, win, "cancelAnimationFrame", |_i, _t, _a| Ok(V::Undef));
    put_fn(rt, win, "getComputedStyle", |i, _t, a| {
        let Some(n) = this_node(i, &arg(a, 0)) else {
            return Ok(V::Obj(alloc_ord(i.rt)));
        };
        let proto = i.rt.dom_proto;
        let o = i.rt.alloc(Obj::new(ObjKind::StyleObj { node: n }, proto));
        Ok(V::Obj(o))
    });
    put_fn(rt, win, "confirm", |_i, _t, _a| Ok(V::Bool(true)));
    put_fn(rt, win, "prompt", |_i, _t, _a| Ok(V::s("")));
    put_fn(rt, win, "print", |_i, _t, _a| Ok(V::Undef));
    put_fn(rt, win, "scrollTo", |_i, _t, _a| Ok(V::Undef));
    put_fn(rt, win, "scrollBy", |_i, _t, _a| Ok(V::Undef));
    put_fn(rt, win, "open", |_i, _t, _a| Ok(V::Null));
    put_fn(rt, win, "close", |_i, _t, _a| Ok(V::Undef));
    put_fn(rt, win, "DOMParser", |i, _t, _a| {
        let o = alloc_ord(i.rt);
        let parse = native(i.rt, "parseFromString", |i2, _t, a| {
            let html = arg(a, 0).to_str(i2.rt);
            let Some(d) = doc_of(i2.rt) else {
                return Ok(V::Null);
            };
            let mut d = d.borrow_mut();
            // parse into fresh document
            let mut doc2 = sol_dom::Document::new();
            let parsed = sol_html::parse(&html, &mut doc2);
            let _ = parsed;
            // move children into our doc? Simplest: swap doc2 root into
            // a wrapper whose rt.doc is shared — wrap doc2's root via
            // a detached Document — instead re-parse into our doc's
            // fragment and return a mini-doc. For simplicity, return
            // a wrapper over a fresh root copied into our doc.
            let frag = d.create_fragment();
            let kids = sol_html::parse_fragment(&html, "body", &mut d);
            for k in kids {
                d.append(frag, k);
            }
            Ok(wrap(i2.rt, frag))
        });
        i.rt.set(o, "parseFromString", parse);
        Ok(V::Obj(o))
    });

    crate::rt::Env::declare(&g, "window", V::Obj(win));
    crate::rt::Env::declare(&g, "location", loc);
    crate::rt::Env::declare(&g, "navigator", V::Obj(nav));
    crate::rt::Env::declare(&g, "screen", V::Obj(scr));
    crate::rt::Env::declare(&g, "localStorage", V::Obj(storage));
    crate::rt::Env::declare(&g, "sessionStorage", V::Obj(storage));
    crate::rt::Env::declare(&g, "history", V::Obj(hist));
    crate::rt::Env::declare(&g, "document", V::Null); // real doc set on set_document

    // Node constants
    let node_ctor = alloc_ord(rt);
    rt.set(node_ctor, "ELEMENT_NODE", V::Num(1.0));
    rt.set(node_ctor, "TEXT_NODE", V::Num(3.0));
    rt.set(node_ctor, "COMMENT_NODE", V::Num(8.0));
    rt.set(node_ctor, "DOCUMENT_NODE", V::Num(9.0));
    rt.set(node_ctor, "DOCUMENT_FRAGMENT_NODE", V::Num(11.0));
    crate::rt::Env::declare(&g, "Node", V::Obj(node_ctor));
    crate::rt::Env::declare(&g, "Element", V::Obj(alloc_ord(rt)));
    crate::rt::Env::declare(&g, "HTMLElement", V::Obj(alloc_ord(rt)));
    crate::rt::Env::declare(
        &g,
        "MutationObserver",
        native(rt, "MutationObserver", |i, _t, _a| {
            Ok(V::Obj(alloc_ord(i.rt)))
        }),
    );
}

/// Called after a document is attached: re-point `document` and
/// update `location`/`documentElement` access.
pub fn refresh_globals(i: &mut I) {
    let rt = &mut *i.rt;
    let doc_v = wrap_doc(rt);
    crate::rt::Env::set(&rt.global_env.clone(), "document", doc_v);
}
