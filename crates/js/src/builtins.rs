//! Built-in globals: Object/Array/String/Number/Math/JSON/console,
//! Promise, timers, Map/Set/Date, Error types.

use crate::eval::{Res, I};
use crate::rt::{num_str, Obj, ObjKind, Rt, Throw, NO_PROTO, V};
use std::cell::RefCell;
use std::rc::Rc;

pub fn alloc_ord(rt: &mut Rt) -> usize {
    rt.alloc(Obj::new(ObjKind::Ordinary, rt.obj_proto))
}

pub fn alloc_arr(rt: &mut Rt, items: Vec<V>) -> usize {
    rt.alloc(Obj::new(ObjKind::Arr(RefCell::new(items)), rt.arr_proto))
}

pub fn alloc_promise(rt: &mut Rt) -> usize {
    rt.alloc(Obj::new(
        ObjKind::Promise {
            state: std::cell::Cell::new(0),
            value: RefCell::new(V::Undef),
            thens: RefCell::new(Vec::new()),
        },
        rt.promise_proto,
    ))
}

pub fn native<F>(rt: &mut Rt, name: &str, f: F) -> V
where
    F: Fn(&mut I, V, &[V]) -> Result<V, Throw> + 'static,
{
    let o = rt.alloc(Obj::new(
        ObjKind::Native {
            name: name.to_string(),
            f: Rc::new(f),
        },
        rt.func_proto,
    ));
    rt.set(o, "name", V::s(name));
    V::Obj(o)
}

fn put(rt: &mut Rt, obj: usize, name: &str, v: V) {
    rt.set(obj, name, v);
}

fn put_fn<F>(rt: &mut Rt, obj: usize, name: &str, f: F)
where
    F: Fn(&mut I, V, &[V]) -> Result<V, Throw> + 'static,
{
    let v = native(rt, name, f);
    rt.set_ne(obj, name, v);
}

fn arg(args: &[V], i: usize) -> V {
    args.get(i).cloned().unwrap_or(V::Undef)
}

fn need_arr(i: &mut I, this: &V) -> Result<usize, Throw> {
    if let V::Obj(r) = this {
        if let ObjKind::Arr(_) = i.rt.obj(*r).kind {
            return Ok(*r);
        }
    }
    Err(err_of(i.rt, "TypeError: array method on non-array"))
}

fn arr_items(rt: &Rt, r: usize) -> Vec<V> {
    match &rt.obj(r).kind {
        ObjKind::Arr(a) => a.borrow().clone(),
        _ => vec![],
    }
}

/// Returns the registered array method as a native value, if `name` is
/// one of the array builtins.
pub fn array_method(rt: &mut Rt, name: &str, _arr: usize) -> Option<V> {
    let f: fn(&mut I, V, &[V]) -> Result<V, Throw> = match name {
        "push" => a_push,
        "pop" => a_pop,
        "shift" => a_shift,
        "unshift" => a_unshift,
        "slice" => a_slice,
        "splice" => a_splice,
        "indexOf" => a_indexof,
        "lastIndexOf" => a_lastindexof,
        "includes" => a_includes,
        "join" => a_join,
        "concat" => a_concat,
        "reverse" => a_reverse,
        "sort" => a_sort,
        "forEach" => a_foreach,
        "map" => a_map,
        "filter" => a_filter,
        "find" => a_find,
        "findIndex" => a_findindex,
        "some" => a_some,
        "every" => a_every,
        "reduce" => a_reduce,
        "reduceRight" => a_reduceright,
        "flat" => a_flat,
        "flatMap" => a_flatmap,
        "fill" => a_fill,
        "keys" | "values" => a_values_iter,
        "entries" => a_entries_iter,
        "toString" => a_join,
        _ => return None,
    };
    Some(native(rt, name, f))
}

fn a_push(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    if let ObjKind::Arr(a) = &i.rt.obj(r).kind {
        a.borrow_mut().extend(args.iter().cloned());
        return Ok(V::Num(a.borrow().len() as f64));
    }
    Err(err_of(i.rt, "not array"))
}
fn a_pop(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    if let ObjKind::Arr(a) = &i.rt.obj(r).kind {
        return Ok(a.borrow_mut().pop().unwrap_or(V::Undef));
    }
    Err(err_of(i.rt, "not array"))
}
fn a_shift(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    if let ObjKind::Arr(a) = &i.rt.obj(r).kind {
        return Ok(if a.borrow().is_empty() {
            V::Undef
        } else {
            a.borrow_mut().remove(0)
        });
    }
    Err(err_of(i.rt, "not array"))
}
fn a_unshift(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    if let ObjKind::Arr(a) = &i.rt.obj(r).kind {
        let mut a = a.borrow_mut();
        for (k, v) in args.iter().enumerate() {
            a.insert(k, v.clone());
        }
        return Ok(V::Num(a.len() as f64));
    }
    Err(err_of(i.rt, "not array"))
}
fn a_slice(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let items = arr_items(i.rt, r);
    let n = items.len() as i64;
    let norm = |v: &V, def: i64| {
        let xv = v.to_num(i.rt);
        if xv.is_nan() {
            return def;
        }
        let x = xv as i64;
        if x < 0 {
            (n + x).max(0)
        } else {
            x.min(n)
        }
    };
    let s = norm(&arg(args, 0), 0);
    let e = if args.is_empty() {
        n
    } else {
        norm(&arg(args, 1), n)
    };
    let out: Vec<V> = items[s.max(0).min(n) as usize..e.max(0).min(n) as usize].to_vec();
    Ok(V::Obj(alloc_arr(i.rt, out)))
}
fn a_splice(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let items = arr_items(i.rt, r);
    let n = items.len() as i64;
    let start = {
        let x = arg(args, 0).to_num(i.rt) as i64;
        if x < 0 {
            (n + x).max(0)
        } else {
            x.min(n)
        }
    };
    let del = if args.len() > 1 {
        (arg(args, 1).to_num(i.rt) as i64).clamp(0, n - start)
    } else {
        n - start
    };
    let mut a = items;
    let removed: Vec<V> = a.drain(start as usize..(start + del) as usize).collect();
    for (k, v) in args.iter().skip(2).enumerate() {
        a.insert(start as usize + k, v.clone());
    }
    if let ObjKind::Arr(ar) = &i.rt.obj(r).kind {
        *ar.borrow_mut() = a;
    }
    Ok(V::Obj(alloc_arr(i.rt, removed)))
}
fn a_indexof(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let items = arr_items(i.rt, r);
    let needle = arg(args, 0);
    for (k, v) in items.iter().enumerate() {
        if crate::eval::strict_eq(v, &needle, i.rt) {
            return Ok(V::Num(k as f64));
        }
    }
    Ok(V::Num(-1.0))
}
fn a_lastindexof(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let items = arr_items(i.rt, r);
    let needle = arg(args, 0);
    for (k, v) in items.iter().enumerate().rev() {
        if crate::eval::strict_eq(v, &needle, i.rt) {
            return Ok(V::Num(k as f64));
        }
    }
    Ok(V::Num(-1.0))
}
fn a_includes(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let items = arr_items(i.rt, r);
    let needle = arg(args, 0);
    Ok(V::Bool(items.iter().any(|v| {
        crate::eval::strict_eq(v, &needle, i.rt)
            || (matches!(v, V::Num(a) if a.is_nan()) && matches!(needle, V::Num(b) if b.is_nan()))
    })))
}
fn a_join(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let items = arr_items(i.rt, r);
    let sep = if args.is_empty() {
        ",".into()
    } else {
        arg(args, 0).to_str(i.rt)
    };
    let s = items
        .iter()
        .map(|v| {
            if v.is_nullish() {
                String::new()
            } else {
                v.to_str(i.rt)
            }
        })
        .collect::<Vec<_>>()
        .join(&sep);
    Ok(V::s(s))
}
fn a_concat(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let mut out = arr_items(i.rt, r);
    for a in args {
        if let V::Obj(ar) = a {
            if let ObjKind::Arr(v) = &i.rt.obj(*ar).kind {
                out.extend(v.borrow().iter().cloned());
                continue;
            }
        }
        out.push(a.clone());
    }
    Ok(V::Obj(alloc_arr(i.rt, out)))
}
fn a_reverse(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    if let ObjKind::Arr(a) = &i.rt.obj(r).kind {
        a.borrow_mut().reverse();
    }
    Ok(this)
}
fn a_sort(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let cmp_fn = arg(args, 0);
    let mut items = arr_items(i.rt, r);
    if matches!(cmp_fn, V::Obj(_)) {
        // insertion sort with comparator (stable enough)
        for a in 1..items.len() {
            let mut b = a;
            while b > 0 {
                let r2 = i.call(
                    cmp_fn.clone(),
                    V::Undef,
                    vec![items[b - 1].clone(), items[b].clone()],
                )?;
                if r2.to_num(i.rt) > 0.0 {
                    items.swap(b - 1, b);
                    b -= 1;
                } else {
                    break;
                }
            }
        }
    } else {
        items.sort_by_key(|x| x.to_str(i.rt));
    }
    if let ObjKind::Arr(a) = &i.rt.obj(r).kind {
        *a.borrow_mut() = items;
    }
    Ok(this)
}
fn iter_args(i: &mut I, args: &[V]) -> Result<(V, V), Throw> {
    let f = arg(args, 0);
    if !matches!(f, V::Obj(_)) {
        return Err(err_of(i.rt, "TypeError: not a function"));
    }
    Ok((f, arg(args, 1)))
}
fn a_foreach(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let (f, t) = iter_args(i, args)?;
    for (k, v) in arr_items(i.rt, r).iter().enumerate() {
        i.call(
            f.clone(),
            t.clone(),
            vec![v.clone(), V::Num(k as f64), this.clone()],
        )?;
    }
    Ok(V::Undef)
}
fn a_map(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let (f, t) = iter_args(i, args)?;
    let mut out = Vec::new();
    for (k, v) in arr_items(i.rt, r).iter().enumerate() {
        out.push(i.call(
            f.clone(),
            t.clone(),
            vec![v.clone(), V::Num(k as f64), this.clone()],
        )?);
    }
    Ok(V::Obj(alloc_arr(i.rt, out)))
}
fn a_filter(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let (f, t) = iter_args(i, args)?;
    let mut out = Vec::new();
    for (k, v) in arr_items(i.rt, r).iter().enumerate() {
        if i.call(
            f.clone(),
            t.clone(),
            vec![v.clone(), V::Num(k as f64), this.clone()],
        )?
        .truthy(i.rt)
        {
            out.push(v.clone());
        }
    }
    Ok(V::Obj(alloc_arr(i.rt, out)))
}
fn a_find(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let (f, t) = iter_args(i, args)?;
    for (k, v) in arr_items(i.rt, r).iter().enumerate() {
        if i.call(
            f.clone(),
            t.clone(),
            vec![v.clone(), V::Num(k as f64), this.clone()],
        )?
        .truthy(i.rt)
        {
            return Ok(v.clone());
        }
    }
    Ok(V::Undef)
}
fn a_findindex(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let (f, t) = iter_args(i, args)?;
    for (k, v) in arr_items(i.rt, r).iter().enumerate() {
        if i.call(
            f.clone(),
            t.clone(),
            vec![v.clone(), V::Num(k as f64), this.clone()],
        )?
        .truthy(i.rt)
        {
            return Ok(V::Num(k as f64));
        }
    }
    Ok(V::Num(-1.0))
}
fn a_some(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let (f, t) = iter_args(i, args)?;
    for (k, v) in arr_items(i.rt, r).iter().enumerate() {
        if i.call(
            f.clone(),
            t.clone(),
            vec![v.clone(), V::Num(k as f64), this.clone()],
        )?
        .truthy(i.rt)
        {
            return Ok(V::Bool(true));
        }
    }
    Ok(V::Bool(false))
}
fn a_every(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let (f, t) = iter_args(i, args)?;
    for (k, v) in arr_items(i.rt, r).iter().enumerate() {
        if !i
            .call(
                f.clone(),
                t.clone(),
                vec![v.clone(), V::Num(k as f64), this.clone()],
            )?
            .truthy(i.rt)
        {
            return Ok(V::Bool(false));
        }
    }
    Ok(V::Bool(true))
}
fn a_reduce(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let f = arg(args, 0);
    let items = arr_items(i.rt, r);
    let (mut acc, start) = if args.len() > 1 {
        (arg(args, 1), 0)
    } else if !items.is_empty() {
        (items[0].clone(), 1)
    } else {
        return Err(err_of(i.rt, "TypeError: reduce of empty"));
    };
    for (k, v) in items.iter().enumerate().skip(start) {
        acc = i.call(
            f.clone(),
            V::Undef,
            vec![acc, v.clone(), V::Num(k as f64), this.clone()],
        )?;
    }
    Ok(acc)
}
fn a_reduceright(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let f = arg(args, 0);
    let items = arr_items(i.rt, r);
    let (mut acc, start) = if args.len() > 1 {
        (arg(args, 1), items.len() as i64 - 1)
    } else if !items.is_empty() {
        (items[items.len() - 1].clone(), items.len() as i64 - 2)
    } else {
        return Err(err_of(i.rt, "TypeError: reduce of empty"));
    };
    for k in (0..=start).rev() {
        acc = i.call(
            f.clone(),
            V::Undef,
            vec![
                acc,
                items[k as usize].clone(),
                V::Num(k as f64),
                this.clone(),
            ],
        )?;
    }
    Ok(acc)
}
fn a_flat(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let depth = if args.is_empty() {
        1
    } else {
        arg(args, 0).to_num(i.rt) as i64
    };
    fn flat(rt: &Rt, items: &[V], d: i64, out: &mut Vec<V>) {
        for v in items {
            if d > 0 {
                if let V::Obj(r) = v {
                    if let ObjKind::Arr(a) = &rt.obj(*r).kind {
                        flat(rt, &a.borrow(), d - 1, out);
                        continue;
                    }
                }
            }
            out.push(v.clone());
        }
    }
    let items = arr_items(i.rt, r);
    let mut out = Vec::new();
    flat(i.rt, &items, depth, &mut out);
    Ok(V::Obj(alloc_arr(i.rt, out)))
}
fn a_flatmap(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let (f, t) = iter_args(i, args)?;
    let mut out = Vec::new();
    for (k, v) in arr_items(i.rt, r).iter().enumerate() {
        let r2 = i.call(
            f.clone(),
            t.clone(),
            vec![v.clone(), V::Num(k as f64), this.clone()],
        )?;
        if let V::Obj(ar) = r2 {
            if let ObjKind::Arr(a) = &i.rt.obj(ar).kind {
                out.extend(a.borrow().iter().cloned());
                continue;
            }
        }
        out.push(r2);
    }
    Ok(V::Obj(alloc_arr(i.rt, out)))
}
fn a_fill(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    if let ObjKind::Arr(a) = &i.rt.obj(r).kind {
        let mut a = a.borrow_mut();
        let n = a.len() as i64;
        let s = {
            let x = arg(args, 1).to_num(i.rt) as i64;
            if x < 0 {
                (n + x).max(0)
            } else {
                x.min(n)
            }
        };
        let e = {
            let x = arg(args, 2).to_num(i.rt) as i64;
            if args.len() < 3 {
                n
            } else if x < 0 {
                (n + x).max(0)
            } else {
                x.min(n)
            }
        };
        for k in s..e {
            a[k as usize] = arg(args, 0);
        }
    }
    Ok(this)
}
fn a_values_iter(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let items = arr_items(i.rt, r);
    let o = i.rt.alloc(Obj::new(
        ObjKind::Iter(RefCell::new(items.into_iter())),
        i.rt.iter_proto,
    ));
    Ok(V::Obj(o))
}
fn a_entries_iter(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    let r = need_arr(i, &this)?;
    let pairs: Vec<(usize, V)> = arr_items(i.rt, r).into_iter().enumerate().collect();
    let items: Vec<V> = pairs
        .into_iter()
        .map(|(k, v)| V::Obj(alloc_arr(i.rt, vec![V::Num(k as f64), v])))
        .collect();
    let o = i.rt.alloc(Obj::new(
        ObjKind::Iter(RefCell::new(items.into_iter())),
        i.rt.iter_proto,
    ));
    Ok(V::Obj(o))
}

// ---------- string methods (via StrObj proto) ----------

fn s_charat(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s = this.to_str(i.rt);
    let n = arg(args, 0).to_num(i.rt) as usize;
    Ok(V::s(
        s.chars().nth(n).map(|c| c.to_string()).unwrap_or_default(),
    ))
}
fn s_charcodeat(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s = this.to_str(i.rt);
    let n = arg(args, 0).to_num(i.rt) as usize;
    Ok(s.chars()
        .nth(n)
        .map(|c| V::Num(c as u32 as f64))
        .unwrap_or(V::Num(f64::NAN)))
}
fn s_codepointat(i: &mut I, this: V, args: &[V]) -> Res<V> {
    s_charcodeat(i, this, args)
}
fn s_includes(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s = this.to_str(i.rt);
    let n = arg(args, 0).to_str(i.rt);
    Ok(V::Bool(s.contains(&n)))
}
fn s_indexof(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s = this.to_str(i.rt);
    let n = arg(args, 0).to_str(i.rt);
    let from = arg(args, 1).to_num(i.rt).max(0.0) as usize;
    let hay: Vec<char> = s.chars().collect();
    let sub: Vec<char> = n.chars().collect();
    for k in from..=hay.len().saturating_sub(sub.len()) {
        if hay[k..k + sub.len()] == sub[..] {
            return Ok(V::Num(k as f64));
        }
    }
    if sub.is_empty() {
        return Ok(V::Num(from.min(hay.len()) as f64));
    }
    Ok(V::Num(-1.0))
}
fn s_lastindexof(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s = this.to_str(i.rt);
    let n = arg(args, 0).to_str(i.rt);
    match s.rfind(&n) {
        Some(byte) => Ok(V::Num(s[..byte].chars().count() as f64)),
        None => Ok(V::Num(-1.0)),
    }
}
fn s_slice(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s: Vec<char> = this.to_str(i.rt).chars().collect();
    let n = s.len() as i64;
    let norm = |v: &V, def: i64| {
        if matches!(v, V::Undef) {
            def
        } else {
            let x = v.to_num(i.rt) as i64;
            if x < 0 {
                (n + x).max(0)
            } else {
                x.min(n)
            }
        }
    };
    let a = norm(&arg(args, 0), 0);
    let b = norm(&arg(args, 1), n);
    if a >= b {
        return Ok(V::s(""));
    }
    Ok(V::s(s[a as usize..b as usize].iter().collect::<String>()))
}
fn s_substring(i: &mut I, this: V, args: &[V]) -> Res<V> {
    s_slice(i, this, args)
}
fn s_substr(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s: Vec<char> = this.to_str(i.rt).chars().collect();
    let n = s.len() as i64;
    let a = {
        let x = arg(args, 0).to_num(i.rt) as i64;
        if x < 0 {
            (n + x).max(0)
        } else {
            x.min(n)
        }
    };
    let len = if args.len() > 1 {
        arg(args, 1).to_num(i.rt) as i64
    } else {
        n
    };
    let b = (a + len).min(n);
    Ok(V::s(
        s[a.max(0) as usize..b.max(a) as usize]
            .iter()
            .collect::<String>(),
    ))
}
fn s_split(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s = this.to_str(i.rt);
    let sep = arg(args, 0);
    let parts: Vec<V> = match &sep {
        V::Undef => vec![V::s(s)],
        V::Obj(r) if matches!(i.rt.obj(*r).kind, ObjKind::Regex { .. }) => {
            // split by regex
            let (body, flags) = match &i.rt.obj(*r).kind {
                ObjKind::Regex { body, flags, .. } => (body.clone(), flags.clone()),
                _ => unreachable!(),
            };
            let re = crate::regex::compile(&body, &flags).map_err(|e| err_of(i.rt, e))?;
            let mut out = Vec::new();
            let mut last = 0usize;
            let chars: Vec<char> = s.chars().collect();
            let mut pos = 0usize;
            while pos <= chars.len() {
                if let Some((a, b, _)) = re.find(&s, pos) {
                    if b == a && b == last {
                        pos += 1;
                        continue;
                    }
                    out.push(V::s(chars[last..a].iter().collect::<String>()));
                    last = b;
                    pos = b;
                } else {
                    break;
                }
            }
            out.push(V::s(chars[last..].iter().collect::<String>()));
            out
        }
        _ => {
            let sep = sep.to_str(i.rt);
            if sep.is_empty() {
                s.chars().map(|c| V::s(c.to_string())).collect()
            } else {
                s.split(&sep).map(V::s).collect()
            }
        }
    };
    let parts = if args.len() > 1 {
        let lim = arg(args, 1).to_num(i.rt) as usize;
        parts.into_iter().take(lim).collect()
    } else {
        parts
    };
    Ok(V::Obj(alloc_arr(i.rt, parts)))
}
fn s_replace(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s = this.to_str(i.rt);
    let pat = arg(args, 0);
    let rep = arg(args, 1);
    if let V::Obj(r) = pat {
        if let ObjKind::Regex { body, flags, .. } = &i.rt.obj(r).kind {
            let (body, flags) = (body.clone(), flags.clone());
            let re = crate::regex::compile(&body, &flags).map_err(|e| err_of(i.rt, e))?;
            let global = flags.contains('g');
            let chars: Vec<char> = s.chars().collect();
            let mut out = String::new();
            let mut last = 0usize;
            let mut pos = 0usize;
            while let Some((a, b, caps)) = re.find(&s, pos) {
                out.push_str(&chars[last..a].iter().collect::<String>());
                let matched: String = chars[a..b].iter().collect();
                out.push_str(&expand_replacement(
                    &rep.to_str(i.rt),
                    &matched,
                    &caps,
                    &chars,
                ));
                last = b;
                pos = if b == a { b + 1 } else { b };
                if !global {
                    break;
                }
                if pos > chars.len() {
                    break;
                }
            }
            out.push_str(&chars[last..].iter().collect::<String>());
            return Ok(V::s(out));
        }
    }
    let needle = pat.to_str(i.rt);
    match s.find(&needle) {
        Some(byte) => {
            let before = &s[..byte];
            let after = &s[byte + needle.len()..];
            let replacement = rep.to_str(i.rt);
            let replacement = replacement
                .replace("$$", "\u{1}")
                .replace("$&", &needle)
                .replace('\u{1}', "$");
            Ok(V::s(format!("{before}{replacement}{after}")))
        }
        None => Ok(V::s(s)),
    }
}
fn expand_replacement(
    rep: &str,
    matched: &str,
    caps: &[Option<(usize, usize)>],
    chars: &[char],
) -> String {
    let mut out = String::new();
    let mut it = rep.chars().peekable();
    while let Some(c) = it.next() {
        if c == '$' {
            match it.next() {
                Some('$') => out.push('$'),
                Some('&') => out.push_str(matched),
                Some(d) if d.is_ascii_digit() => {
                    let mut n = d.to_string();
                    if it.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                        n.push(it.next().unwrap());
                    }
                    if let Ok(gi) = n.parse::<usize>() {
                        if let Some(Some((a, b))) = caps.get(gi) {
                            out.push_str(&chars[*a..*b].iter().collect::<String>());
                            continue;
                        }
                        if let Some(Some((a, b))) = caps.get(n[..1].parse::<usize>().unwrap_or(0)) {
                            out.push_str(&chars[*a..*b].iter().collect::<String>());
                            continue;
                        }
                    }
                    out.push('$');
                    out.push_str(&n);
                }
                Some('`') => {}
                Some('\'') => {}
                Some(c2) => {
                    out.push('$');
                    out.push(c2);
                }
                None => out.push('$'),
            }
        } else {
            out.push(c);
        }
    }
    out
}
fn s_match(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s = this.to_str(i.rt);
    let pat = arg(args, 0);
    let (body, flags) = match &pat {
        V::Obj(r) => match &i.rt.obj(*r).kind {
            ObjKind::Regex { body, flags, .. } => (body.clone(), flags.clone()),
            _ => (pat.to_str(i.rt), String::new()),
        },
        _ => (pat.to_str(i.rt), String::new()),
    };
    let re = crate::regex::compile(&body, &flags).map_err(|e| err_of(i.rt, e))?;
    let chars: Vec<char> = s.chars().collect();
    if flags.contains('g') {
        let mut out = Vec::new();
        let mut pos = 0usize;
        while let Some((a, b, _)) = re.find(&s, pos) {
            out.push(V::s(chars[a..b].iter().collect::<String>()));
            pos = if b == a { b + 1 } else { b };
            if pos > chars.len() {
                break;
            }
        }
        return Ok(if out.is_empty() {
            V::Null
        } else {
            V::Obj(alloc_arr(i.rt, out))
        });
    }
    match re.find(&s, 0) {
        Some((a, b, caps)) => {
            let mut items = vec![V::s(chars[a..b].iter().collect::<String>())];
            for c in caps.iter().skip(1) {
                items.push(match c {
                    Some((x, y)) => V::s(chars[*x..*y].iter().collect::<String>()),
                    None => V::Undef,
                });
            }
            let arr = alloc_arr(i.rt, items);
            i.rt.set(arr, "index", V::Num(a as f64));
            i.rt.set(arr, "input", V::s(s));
            Ok(V::Obj(arr))
        }
        None => Ok(V::Null),
    }
}
fn s_search(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s = this.to_str(i.rt);
    let pat = arg(args, 0);
    let (body, flags) = match &pat {
        V::Obj(r) => match &i.rt.obj(*r).kind {
            ObjKind::Regex { body, flags, .. } => (body.clone(), flags.clone()),
            _ => (pat.to_str(i.rt), String::new()),
        },
        _ => (pat.to_str(i.rt), String::new()),
    };
    let re = crate::regex::compile(&body, &flags).map_err(|e| err_of(i.rt, e))?;
    Ok(match re.find(&s, 0) {
        Some((a, _, _)) => V::Num(a as f64),
        None => V::Num(-1.0),
    })
}
fn s_test_impl(i: &mut I, this: V, args: &[V]) -> Res<V> {
    // RegExp.prototype.test
    let s = arg(args, 0).to_str(i.rt);
    if let V::Obj(r) = &this {
        let meta = if let ObjKind::Regex { body, flags, last } = &i.rt.obj(*r).kind {
            Some((
                body.clone(),
                flags.clone(),
                if flags.contains('g') { last.get() } else { 0 },
            ))
        } else {
            None
        };
        if let Some((body, flags, start)) = meta {
            let re = crate::regex::compile(&body, &flags).map_err(|e| err_of(i.rt, e))?;
            match re.find(&s, start) {
                Some((_, b, _)) => {
                    if flags.contains('g') {
                        if let ObjKind::Regex { last, .. } = &i.rt.obj(*r).kind {
                            last.set(b);
                        }
                    }
                    return Ok(V::Bool(true));
                }
                None => {
                    if let ObjKind::Regex { last, .. } = &i.rt.obj(*r).kind {
                        last.set(0);
                    }
                    return Ok(V::Bool(false));
                }
            }
        }
    }
    Err(err_of(i.rt, "not a regex"))
}
fn s_exec_impl(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s = arg(args, 0).to_str(i.rt);
    if let V::Obj(r) = &this {
        if let ObjKind::Regex { body, flags, .. } = &i.rt.obj(*r).kind {
            let re = crate::regex::compile(body, flags).map_err(|e| err_of(i.rt, e))?;
            let chars: Vec<char> = s.chars().collect();
            match re.find(&s, 0) {
                Some((a, b, caps)) => {
                    let mut items = vec![V::s(chars[a..b].iter().collect::<String>())];
                    for c in caps.iter().skip(1) {
                        items.push(match c {
                            Some((x, y)) => V::s(chars[*x..*y].iter().collect::<String>()),
                            None => V::Undef,
                        });
                    }
                    let arr = alloc_arr(i.rt, items);
                    i.rt.set(arr, "index", V::Num(a as f64));
                    i.rt.set(arr, "input", V::s(s));
                    return Ok(V::Obj(arr));
                }
                None => return Ok(V::Null),
            }
        }
    }
    Err(err_of(i.rt, "not a regex"))
}
fn s_trim(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    Ok(V::s(this.to_str(i.rt).trim().to_string()))
}
fn s_trimstart(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    Ok(V::s(this.to_str(i.rt).trim_start().to_string()))
}
fn s_trimend(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    Ok(V::s(this.to_str(i.rt).trim_end().to_string()))
}
fn s_tolower(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    Ok(V::s(this.to_str(i.rt).to_lowercase()))
}
fn s_toupper(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    Ok(V::s(this.to_str(i.rt).to_uppercase()))
}
fn s_startswith(i: &mut I, this: V, args: &[V]) -> Res<V> {
    Ok(V::Bool(
        this.to_str(i.rt).starts_with(&arg(args, 0).to_str(i.rt)),
    ))
}
fn s_endswith(i: &mut I, this: V, args: &[V]) -> Res<V> {
    Ok(V::Bool(
        this.to_str(i.rt).ends_with(&arg(args, 0).to_str(i.rt)),
    ))
}
fn s_repeat(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let n = arg(args, 0).to_num(i.rt).max(0.0) as usize;
    Ok(V::s(this.to_str(i.rt).repeat(n.min(1_000_000))))
}
fn s_padstart(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s = this.to_str(i.rt);
    let n = arg(args, 0).to_num(i.rt).max(0.0) as usize;
    let fill = arg(args, 1).to_str(i.rt);
    let fill = if fill.is_empty() { " ".into() } else { fill };
    let mut out = s.clone();
    while out.chars().count() < n {
        out = format!("{fill}{out}");
    }
    Ok(V::s(out.chars().take(n).collect::<String>()))
}
fn s_padend(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s = this.to_str(i.rt);
    let n = arg(args, 0).to_num(i.rt).max(0.0) as usize;
    let fill = arg(args, 1).to_str(i.rt);
    let fill = if fill.is_empty() { " ".into() } else { fill };
    let mut out = s;
    while out.chars().count() < n {
        out.push_str(&fill);
    }
    Ok(V::s(out.chars().take(n).collect::<String>()))
}
fn s_replaceall(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s = this.to_str(i.rt);
    if let V::Obj(r) = arg(args, 0) {
        if let ObjKind::Regex { body, flags, .. } = &i.rt.obj(r).kind {
            let flags2 = if flags.contains('g') {
                flags.clone()
            } else {
                format!("{flags}g")
            };
            let re_obj = i.rt.alloc(Obj::new(
                ObjKind::Regex {
                    body: body.clone(),
                    flags: flags2,
                    last: std::cell::Cell::new(0),
                },
                i.rt.re_proto,
            ));
            return s_replace(i, this, &[V::Obj(re_obj), arg(args, 1)]);
        }
    }
    let pat = arg(args, 0).to_str(i.rt);
    let rep = arg(args, 1).to_str(i.rt);
    Ok(V::s(s.replace(&pat, &rep)))
}
fn s_at(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let s: Vec<char> = this.to_str(i.rt).chars().collect();
    let n = s.len() as i64;
    let x = arg(args, 0).to_num(i.rt) as i64;
    let idx = if x < 0 { n + x } else { x };
    Ok(s.get(idx as usize)
        .map(|c| V::s(c.to_string()))
        .unwrap_or(V::Undef))
}
fn s_tostring(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    Ok(V::s(this.to_str(i.rt)))
}

// ---------- object ----------

fn o_keys(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    if let V::Obj(r) = arg(args, 0) {
        let mut ks: Vec<V> = Vec::new();
        if let ObjKind::Arr(a) = &i.rt.obj(r).kind {
            for k in 0..a.borrow().len() {
                ks.push(V::s(k.to_string()));
            }
        }
        let (ks2, ne) = {
            let o = i.rt.obj(r);
            (
                o.props.borrow().keys().cloned().collect::<Vec<_>>(),
                o.non_enum.borrow().clone(),
            )
        };
        for k in ks2 {
            if !ne.contains(&k) {
                ks.push(V::s(k));
            }
        }
        return Ok(V::Obj(alloc_arr(i.rt, ks)));
    }
    Ok(V::Obj(alloc_arr(i.rt, vec![])))
}
fn o_values(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    if let V::Obj(r) = arg(args, 0) {
        let mut vs: Vec<V> = Vec::new();
        if let ObjKind::Arr(a) = &i.rt.obj(r).kind {
            vs.extend(a.borrow().iter().cloned());
        }
        let (pairs2, ne) = {
            let o = i.rt.obj(r);
            (
                o.props
                    .borrow()
                    .iter()
                    .map(|(a, b)| (a.clone(), b.clone()))
                    .collect::<Vec<_>>(),
                o.non_enum.borrow().clone(),
            )
        };
        for (k, v) in pairs2 {
            if !ne.contains(&k) {
                vs.push(v);
            }
        }
        return Ok(V::Obj(alloc_arr(i.rt, vs)));
    }
    Ok(V::Obj(alloc_arr(i.rt, vec![])))
}
fn o_entries(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    if let V::Obj(r) = arg(args, 0) {
        let (pairs, ne) = {
            let o = i.rt.obj(r);
            (
                o.props
                    .borrow()
                    .iter()
                    .map(|(a, b)| (a.clone(), b.clone()))
                    .collect::<Vec<_>>(),
                o.non_enum.borrow().clone(),
            )
        };
        let mut es: Vec<V> = Vec::new();
        for (k, v) in pairs {
            if ne.contains(&k) {
                continue;
            }
            let p = alloc_arr(i.rt, vec![V::s(k), v]);
            es.push(V::Obj(p));
        }
        return Ok(V::Obj(alloc_arr(i.rt, es)));
    }
    Ok(V::Obj(alloc_arr(i.rt, vec![])))
}
fn o_assign(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let dst = arg(args, 0);
    if let V::Obj(dr) = dst {
        for src in args.iter().skip(1) {
            if let V::Obj(sr) = src {
                let pairs: Vec<(String, V)> =
                    i.rt.obj(*sr)
                        .props
                        .borrow()
                        .iter()
                        .map(|(a, b)| (a.clone(), b.clone()))
                        .collect();
                for (k, v) in pairs {
                    i.rt.set(dr, &k, v);
                }
            }
        }
    }
    Ok(dst)
}
fn o_create(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let proto = match arg(args, 0) {
        V::Obj(r) => r,
        _ => i.rt.obj_proto,
    };
    let o = i.rt.alloc(Obj::new(ObjKind::Ordinary, proto));
    Ok(V::Obj(o))
}
fn o_freeze(_i: &mut I, _t: V, args: &[V]) -> Res<V> {
    Ok(arg(args, 0))
}
fn o_getproto(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    if let V::Obj(r) = arg(args, 0) {
        let p = i.rt.obj(r).proto.get();
        return Ok(if p == NO_PROTO { V::Null } else { V::Obj(p) });
    }
    Ok(V::Null)
}
fn o_setproto(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    if let V::Obj(r) = arg(args, 0) {
        let p = match arg(args, 1) {
            V::Obj(p) => p,
            _ => NO_PROTO,
        };
        i.rt.obj(r).proto.set(p);
    }
    Ok(arg(args, 0))
}
fn o_defineprop(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let target = arg(args, 0);
    let key = arg(args, 1).to_str(i.rt);
    let desc = arg(args, 2);
    if let (V::Obj(tr), V::Obj(dr)) = (&target, &desc) {
        let mut has_v = false;
        let mut val = V::Undef;
        let mut get = V::Undef;
        let mut set = V::Undef;
        if let Some(v) = i.rt.get_raw(*dr, "value") {
            val = v;
            has_v = true;
        }
        if let Some(v) = i.rt.get_raw(*dr, "get") {
            get = v;
        }
        if let Some(v) = i.rt.get_raw(*dr, "set") {
            set = v;
        }
        if has_v {
            i.rt.set(*tr, &key, val);
        }
        if !get.is_nullish() || !set.is_nullish() {
            i.rt.obj(*tr).accessors.borrow_mut().insert(key, (get, set));
        }
    }
    Ok(target)
}
fn o_defineprops(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let target = arg(args, 0);
    if let (V::Obj(_tr), V::Obj(props)) = (target.clone(), arg(args, 1)) {
        let keys: Vec<String> = i.rt.obj(props).props.borrow().keys().cloned().collect();
        for k in keys {
            if let Some(d) = i.rt.get_raw(props, &k) {
                let desc_args = vec![target.clone(), V::s(k), d];
                o_defineprop(i, V::Undef, &desc_args)?;
            }
        }
    }
    Ok(target)
}
fn o_hasown(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let key = arg(args, 0).to_str(i.rt);
    if let V::Obj(r) = this {
        return Ok(V::Bool(i.rt.obj(r).props.borrow().contains_key(&key)));
    }
    Ok(V::Bool(false))
}
fn o_tostring(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    Ok(V::s(this.to_str(i.rt)))
}
fn o_is(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    Ok(V::Bool(crate::eval::strict_eq(
        &arg(args, 0),
        &arg(args, 1),
        i.rt,
    )))
}

// ---------- number ----------

fn n_tofixed(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let n = this.to_num(i.rt);
    let d = arg(args, 0).to_num(i.rt).clamp(0.0, 20.0) as usize;
    Ok(V::s(format!("{n:.d$}")))
}
fn n_toprecision(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let n = this.to_num(i.rt);
    let d = arg(args, 0).to_num(i.rt).clamp(1.0, 21.0) as usize;
    Ok(V::s(format!("{n:.d$}")))
}
fn n_tostring(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let n = this.to_num(i.rt);
    let radix = arg(args, 0).to_num(i.rt) as u32;
    if (2..=36).contains(&radix) && n.fract() == 0.0 {
        let mut v = n as i64;
        let neg = v < 0;
        v = v.abs();
        let mut s = String::new();
        loop {
            let d = (v % radix as i64) as u32;
            s.insert(0, std::char::from_digit(d, radix).unwrap_or('0'));
            v /= radix as i64;
            if v == 0 {
                break;
            }
        }
        return Ok(V::s(if neg { format!("-{s}") } else { s }));
    }
    Ok(V::s(num_str(n)))
}

// ---------- json ----------

fn json_stringify(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    fn ser(rt: &Rt, v: &V, depth: usize) -> Option<String> {
        if depth > 64 {
            return None;
        }
        Some(match v {
            V::Undef => return None,
            V::Null => "null".into(),
            V::Bool(b) => b.to_string(),
            V::Num(n) => {
                if n.is_finite() {
                    num_str(*n)
                } else {
                    "null".into()
                }
            }
            V::Str(s) => format!("\"{}\"", esc(s)),
            V::Obj(r) => match &rt.obj(*r).kind {
                ObjKind::Arr(a) => {
                    let items: Vec<String> = a
                        .borrow()
                        .iter()
                        .map(|v| ser(rt, v, depth + 1).unwrap_or("null".into()))
                        .collect();
                    format!("[{}]", items.join(","))
                }
                ObjKind::StrObj(s) => format!("\"{}\"", esc(s)),
                ObjKind::NumObj(n) => num_str(*n),
                ObjKind::BoolObj(b) => b.to_string(),
                ObjKind::Func { .. } | ObjKind::Native { .. } | ObjKind::Dom { .. } => return None,
                _ => {
                    let props = rt.obj(*r).props.borrow();
                    let mut parts = Vec::new();
                    for (k, v) in props.iter() {
                        if let Some(sv) = ser(rt, v, depth + 1) {
                            parts.push(format!("\"{}\":{sv}", esc(k)));
                        }
                    }
                    format!("{{{}}}", parts.join(","))
                }
            },
        })
    }
    let v = arg(args, 0);
    match ser(i.rt, &v, 0) {
        Some(s) => Ok(V::s(s)),
        None => Ok(V::Undef),
    }
}

fn esc(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            '\n' => vec!['\\', 'n'],
            '\t' => vec!['\\', 't'],
            '\r' => vec!['\\', 'r'],
            c if (c as u32) < 0x20 => {
                let mut v = vec!['\\', 'u'];
                v.extend(format!("{:04x}", c as u32).chars());
                v
            }
            c => vec![c],
        })
        .collect()
}

fn json_parse(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let s = arg(args, 0).to_str(i.rt);
    match parse_json(&s) {
        Some(v) => json_to_v(i.rt, v),
        None => Err(err_of(i.rt, "SyntaxError: bad JSON")),
    }
}

enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

fn parse_json(s: &str) -> Option<J> {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    let v = jp(&c, &mut i)?;
    while i < c.len() && c[i].is_whitespace() {
        i += 1;
    }
    if i == c.len() {
        Some(v)
    } else {
        None
    }
}

fn jp(c: &[char], i: &mut usize) -> Option<J> {
    while *i < c.len() && c[*i].is_whitespace() {
        *i += 1;
    }
    match *c.get(*i)? {
        'n' if c[*i..].starts_with(&['n', 'u', 'l', 'l']) => {
            *i += 4;
            Some(J::Null)
        }
        't' if c[*i..].starts_with(&['t', 'r', 'u', 'e']) => {
            *i += 4;
            Some(J::Bool(true))
        }
        'f' if c[*i..].starts_with(&['f', 'a', 'l', 's', 'e']) => {
            *i += 5;
            Some(J::Bool(false))
        }
        '"' => {
            *i += 1;
            let mut s = String::new();
            loop {
                match c.get(*i)? {
                    '"' => {
                        *i += 1;
                        return Some(J::Str(s));
                    }
                    '\\' => {
                        *i += 1;
                        match c.get(*i)? {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            'r' => s.push('\r'),
                            'b' => s.push('\u{8}'),
                            'f' => s.push('\u{c}'),
                            'u' => {
                                let h: String = c[*i + 1..*i + 5].iter().collect();
                                *i += 4;
                                s.push(
                                    char::from_u32(u32::from_str_radix(&h, 16).ok()?)
                                        .unwrap_or('\u{fffd}'),
                                );
                            }
                            c2 => s.push(*c2),
                        }
                        *i += 1;
                    }
                    c2 => {
                        s.push(*c2);
                        *i += 1;
                    }
                }
            }
        }
        '[' => {
            *i += 1;
            let mut a = Vec::new();
            loop {
                while *i < c.len() && c[*i].is_whitespace() {
                    *i += 1;
                }
                if c.get(*i) == Some(&']') {
                    *i += 1;
                    return Some(J::Arr(a));
                }
                a.push(jp(c, i)?);
                while *i < c.len() && c[*i].is_whitespace() {
                    *i += 1;
                }
                match c.get(*i)? {
                    ',' => *i += 1,
                    ']' => {
                        *i += 1;
                        return Some(J::Arr(a));
                    }
                    _ => return None,
                }
            }
        }
        '{' => {
            *i += 1;
            let mut o = Vec::new();
            loop {
                while *i < c.len() && c[*i].is_whitespace() {
                    *i += 1;
                }
                if c.get(*i) == Some(&'}') {
                    *i += 1;
                    return Some(J::Obj(o));
                }
                let k = match jp(c, i)? {
                    J::Str(s) => s,
                    _ => return None,
                };
                while *i < c.len() && c[*i].is_whitespace() {
                    *i += 1;
                }
                if c.get(*i) != Some(&':') {
                    return None;
                }
                *i += 1;
                let v = jp(c, i)?;
                o.push((k, v));
                while *i < c.len() && c[*i].is_whitespace() {
                    *i += 1;
                }
                match c.get(*i)? {
                    ',' => *i += 1,
                    '}' => {
                        *i += 1;
                        return Some(J::Obj(o));
                    }
                    _ => return None,
                }
            }
        }
        _ => {
            let st = *i;
            while *i < c.len() && "-+.eE0123456789".contains(c[*i]) {
                *i += 1;
            }
            let s: String = c[st..*i].iter().collect();
            s.parse().ok().map(J::Num)
        }
    }
}

fn json_to_v(rt: &mut Rt, j: J) -> Res<V> {
    Ok(match j {
        J::Null => V::Null,
        J::Bool(b) => V::Bool(b),
        J::Num(n) => V::Num(n),
        J::Str(s) => V::s(s),
        J::Arr(a) => {
            let items: Result<Vec<V>, _> = a.into_iter().map(|x| json_to_v(rt, x)).collect();
            V::Obj(alloc_arr(rt, items?))
        }
        J::Obj(o) => {
            let r = alloc_ord(rt);
            for (k, v) in o {
                let vv = json_to_v(rt, v)?;
                rt.set(r, &k, vv);
            }
            V::Obj(r)
        }
    })
}

// ---------- misc natives ----------

fn f_parseint(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let s = arg(args, 0).to_str(i.rt);
    let s = s.trim_start();
    let radix = {
        let r = arg(args, 1).to_num(i.rt) as u32;
        if r == 0 {
            if s.starts_with("0x") || s.starts_with("0X") {
                16
            } else {
                10
            }
        } else {
            r.clamp(2, 36)
        }
    };
    let s = if radix == 16 {
        s.trim_start_matches("0x").trim_start_matches("0X")
    } else {
        s
    };
    let mut num = String::new();
    for (k, c) in s.chars().enumerate() {
        let ok = if k == 0 && (c == '-' || c == '+') {
            radix == 10
        } else {
            c.is_digit(radix)
        };
        if ok {
            num.push(c);
        } else {
            break;
        }
    }
    if num.len() <= 1
        && !num
            .chars()
            .next()
            .map(|c| c.is_digit(radix))
            .unwrap_or(false)
    {
        return Ok(V::Num(f64::NAN));
    }
    Ok(V::Num(
        num.parse::<i64>().map(|x| x as f64).unwrap_or(f64::NAN),
    ))
}
fn f_parsefloat(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let s = arg(args, 0).to_str(i.rt);
    let s = s.trim_start();
    let mut end = 0;
    for (k, c) in s.chars().enumerate() {
        if c.is_ascii_digit()
            || c == '.'
            || c == 'e'
            || c == 'E'
            || (k > 0
                && (c == '-' || c == '+')
                && (s.chars().nth(k - 1) == Some('e') || s.chars().nth(k - 1) == Some('E')))
            || (k == 0 && (c == '-' || c == '+'))
        {
            end = k + 1;
        } else {
            break;
        }
    }
    Ok(V::Num(s[..end].parse().unwrap_or(f64::NAN)))
}
fn f_isnan(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    Ok(V::Bool(arg(args, 0).to_num(i.rt).is_nan()))
}
fn f_isfinite(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    Ok(V::Bool(arg(args, 0).to_num(i.rt).is_finite()))
}
fn f_console(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let parts: Vec<String> = args.iter().map(|v| v.to_str(i.rt)).collect();
    i.rt.console_out.borrow_mut().push(parts.join(" "));
    Ok(V::Undef)
}
fn f_alert(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let parts: Vec<String> = args.iter().map(|v| v.to_str(i.rt)).collect();
    i.rt.console_out
        .borrow_mut()
        .push(format!("[alert] {}", parts.join(" ")));
    Ok(V::Undef)
}
fn f_settimeout(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let cb = arg(args, 0);
    let ms = arg(args, 1).to_num(i.rt).max(0.0) as u64;
    let id = i.rt.next_timer;
    i.rt.next_timer += 1;
    let now = sol_util::now_ms() as u64;
    i.rt.timers.push((now + ms, cb, id));
    Ok(V::Num(id as f64))
}
fn f_setinterval(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    f_settimeout(i, _t, args)
}
fn f_cleartimeout(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let id = arg(args, 0).to_num(i.rt) as u32;
    i.rt.timers.retain(|t| t.2 != id);
    Ok(V::Undef)
}
fn f_queuemicrotask(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    i.rt.microtasks.push_back(arg(args, 0));
    Ok(V::Undef)
}
fn f_encodeuri(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let s = arg(args, 0).to_str(i.rt);
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'();,/?:@&=+$#".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    Ok(V::s(out))
}
fn f_encodeuricomp(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let s = arg(args, 0).to_str(i.rt);
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    Ok(V::s(out))
}
fn f_decodeuricomp(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let s = arg(args, 0).to_str(i.rt);
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut k = 0;
    while k < b.len() {
        if b[k] == b'%' && k + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[k + 1..k + 3], 16) {
                out.push(v);
                k += 3;
                continue;
            }
        }
        out.push(b[k]);
        k += 1;
    }
    Ok(V::s(String::from_utf8_lossy(&out).into_owned()))
}
fn f_atob(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let s = arg(args, 0).to_str(i.rt);
    let bytes = b64_dec(&s);
    Ok(V::s(String::from_utf8_lossy(&bytes).into_owned()))
}
fn f_btoa(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let s = arg(args, 0).to_str(i.rt);
    Ok(V::s(b64_enc(s.as_bytes())))
}

pub fn b64_enc(d: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in d.chunks(3) {
        let n = ((c[0] as u32) << 16)
            | ((*c.get(1).unwrap_or(&0) as u32) << 8)
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub fn b64_dec(s: &str) -> Vec<u8> {
    let mut v = Vec::new();
    let b: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let mut acc = 0u32;
    let mut nbits = 0;
    for &c in &b {
        if c == b'=' {
            break;
        }
        let d = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        acc = (acc << 6) | d as u32;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            v.push((acc >> nbits) as u8);
        }
    }
    v
}

// ---------- Promise ----------

fn settle(i: &mut I, r: usize, st: u8, v: V) {
    if let ObjKind::Promise { state, value, .. } = &i.rt.obj(r).kind {
        if state.get() != 0 {
            return;
        }
        state.set(st);
        *value.borrow_mut() = v;
    }
    fire_then(i, r);
}

fn p_resolve_impl(i: &mut I, r: usize, v: V) {
    // thenable unwrapping: if v is a promise, subscribe to it
    if let V::Obj(o) = &v {
        if matches!(i.rt.obj(*o).kind, ObjKind::Promise { .. }) {
            let src = *o;
            let ok = native(i.rt, "__pok", move |i2, _t, a| {
                settle(i2, r, 1, arg(a, 0));
                Ok(V::Undef)
            });
            let err = native(i.rt, "__perr", move |i2, _t, a| {
                settle(i2, r, 2, arg(a, 0));
                Ok(V::Undef)
            });
            if let (V::Obj(a2), V::Obj(b2)) = (ok, err) {
                subscribe(i, src, a2, b2);
            }
            return;
        }
    }
    settle(i, r, 1, v);
}

/// Queue a task that calls cb(v) as a microtask.
fn queue_call(i: &mut I, cb: usize, v: V) {
    let task = native(i.rt, "__ptask", move |i2, _t, _a| {
        i2.call(V::Obj(cb), V::Undef, vec![v.clone()])
    });
    i.rt.microtasks.push_back(task);
}

fn subscribe(i: &mut I, r: usize, ok: usize, err: usize) {
    if let ObjKind::Promise {
        state,
        value,
        thens,
    } = &i.rt.obj(r).kind
    {
        if state.get() == 0 {
            thens.borrow_mut().push((ok, err));
        } else {
            let cb = if state.get() == 1 { ok } else { err };
            let v = value.borrow().clone();
            queue_call(i, cb, v);
        }
    }
}

fn fire_then(i: &mut I, r: usize) {
    let (state_v, v, thens) = match &i.rt.obj(r).kind {
        ObjKind::Promise {
            state,
            value,
            thens,
        } => (
            state.get(),
            value.borrow().clone(),
            std::mem::take(&mut *thens.borrow_mut()),
        ),
        _ => return,
    };
    for (on_ok, on_err) in thens {
        let cb = if state_v == 1 { on_ok } else { on_err };
        queue_call(i, cb, v.clone());
    }
}

fn p_ctor(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let ex = arg(args, 0);
    let pr = alloc_promise(i.rt);
    let res = native(i.rt, "resolve", move |i2, _t, a| {
        p_resolve_impl(i2, pr, arg(a, 0));
        Ok(V::Undef)
    });
    let rej = native(i.rt, "reject", move |i2, _t, a| {
        settle(i2, pr, 2, arg(a, 0));
        Ok(V::Undef)
    });
    if !ex.is_nullish() {
        i.call(ex, V::Undef, vec![res, rej])?;
    }
    Ok(V::Obj(pr))
}

fn p_then(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let V::Obj(r) = this else {
        return Err(err_of(i.rt, "then on non-promise"));
    };
    let on_ok = arg(args, 0);
    let on_err = arg(args, 1);
    let pr2 = alloc_promise(i.rt);
    let wrap = |i: &mut I, cb: V, ok: bool| -> usize {
        let V::Obj(o) = native(i.rt, "__then", move |i2, _t, a| {
            if cb.is_nullish() {
                settle(i2, pr2, if ok { 1 } else { 2 }, arg(a, 0));
                return Ok(V::Undef);
            }
            match i2.call(cb.clone(), V::Undef, vec![arg(a, 0)]) {
                Ok(v) => p_resolve_impl(i2, pr2, v),
                Err(t) => settle(i2, pr2, 2, t.v),
            }
            Ok(V::Undef)
        }) else {
            unreachable!()
        };
        o
    };
    let w_ok = wrap(i, on_ok, true);
    let w_err = wrap(i, on_err, false);
    subscribe(i, r, w_ok, w_err);
    Ok(V::Obj(pr2))
}

fn p_catch(i: &mut I, this: V, args: &[V]) -> Res<V> {
    p_then(i, this, &[V::Undef, arg(args, 0)])
}

fn p_finally(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let cb = arg(args, 0);
    p_then(i, this, &[cb.clone(), cb])
}

fn p_resolve(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let v = arg(args, 0);
    if let V::Obj(r) = &v {
        if matches!(i.rt.obj(*r).kind, ObjKind::Promise { .. }) {
            return Ok(v);
        }
    }
    let pr = alloc_promise(i.rt);
    settle(i, pr, 1, v);
    Ok(V::Obj(pr))
}
fn p_reject(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let pr = alloc_promise(i.rt);
    settle(i, pr, 2, arg(args, 0));
    Ok(V::Obj(pr))
}
fn p_all(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let mut items = Vec::new();
    if let V::Obj(r) = arg(args, 0) {
        if let ObjKind::Arr(a) = &i.rt.obj(r).kind {
            items = a.borrow().clone();
        }
    }
    let n = items.len();
    let pr = alloc_promise(i.rt);
    if n == 0 {
        let arr = V::Obj(alloc_arr(i.rt, vec![]));
        settle(i, pr, 1, arr);
        return Ok(V::Obj(pr));
    }
    let results = Rc::new(RefCell::new(vec![V::Undef; n]));
    let left = Rc::new(std::cell::Cell::new(n));
    for (k, v) in items.into_iter().enumerate() {
        let results2 = results.clone();
        let left2 = left.clone();
        let V::Obj(ok) = native(i.rt, "__all", move |i2, _t, a| {
            results2.borrow_mut()[k] = arg(a, 0);
            left2.set(left2.get() - 1);
            if left2.get() == 0 {
                let arr = alloc_arr(i2.rt, results2.borrow().clone());
                settle(i2, pr, 1, V::Obj(arr));
            }
            Ok(V::Undef)
        }) else {
            unreachable!()
        };
        let V::Obj(err) = native(i.rt, "__allerr", move |i2, _t, a| {
            settle(i2, pr, 2, arg(a, 0));
            Ok(V::Undef)
        }) else {
            unreachable!()
        };
        if let V::Obj(r2) = &v {
            if matches!(i.rt.obj(*r2).kind, ObjKind::Promise { .. }) {
                subscribe(i, *r2, ok, err);
                continue;
            }
        }
        queue_call(i, ok, v);
    }
    Ok(V::Obj(pr))
}
fn p_race(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let mut items = Vec::new();
    if let V::Obj(r) = arg(args, 0) {
        if let ObjKind::Arr(a) = &i.rt.obj(r).kind {
            items = a.borrow().clone();
        }
    }
    let pr = alloc_promise(i.rt);
    for v in items {
        if let V::Obj(r2) = &v {
            if matches!(i.rt.obj(*r2).kind, ObjKind::Promise { .. }) {
                let V::Obj(ok) = native(i.rt, "__race", move |i2, _t, a| {
                    settle(i2, pr, 1, arg(a, 0));
                    Ok(V::Undef)
                }) else {
                    unreachable!()
                };
                let V::Obj(err) = native(i.rt, "__race", move |i2, _t, a| {
                    settle(i2, pr, 2, arg(a, 0));
                    Ok(V::Undef)
                }) else {
                    unreachable!()
                };
                subscribe(i, *r2, ok, err);
                continue;
            }
        }
        settle(i, pr, 1, v);
        break;
    }
    Ok(V::Obj(pr))
}

// Map / Set
fn m_ctor(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let o = i.rt.alloc(Obj::new(
        ObjKind::MapObj(RefCell::new(Vec::new())),
        i.rt.map_proto,
    ));
    // new Map([[k,v],...])
    if let V::Obj(r) = arg(args, 0) {
        if let ObjKind::Arr(a) = &i.rt.obj(r).kind {
            let pairs: Vec<(V, V)> = a
                .borrow()
                .iter()
                .filter_map(|p| {
                    if let V::Obj(pr) = p {
                        if let ObjKind::Arr(pa) = &i.rt.obj(*pr).kind {
                            let pa = pa.borrow();
                            return Some((
                                pa.first().cloned().unwrap_or(V::Undef),
                                pa.get(1).cloned().unwrap_or(V::Undef),
                            ));
                        }
                    }
                    None
                })
                .collect();
            if let ObjKind::MapObj(m) = &i.rt.obj(o).kind {
                *m.borrow_mut() = pairs;
            }
        }
    }
    Ok(V::Obj(o))
}
fn m_set(i: &mut I, this: V, args: &[V]) -> Res<V> {
    if let V::Obj(r) = this {
        if let ObjKind::MapObj(m) = &i.rt.obj(r).kind {
            let k = arg(args, 0);
            let mut m = m.borrow_mut();
            if let Some(e) = m
                .iter_mut()
                .find(|(ek, _)| crate::eval::strict_eq(ek, &k, i.rt))
            {
                e.1 = arg(args, 1);
            } else {
                m.push((k, arg(args, 1)));
            }
        }
    }
    Ok(this)
}
fn m_get(i: &mut I, this: V, args: &[V]) -> Res<V> {
    if let V::Obj(r) = this {
        if let ObjKind::MapObj(m) = &i.rt.obj(r).kind {
            let k = arg(args, 0);
            for (ek, ev) in m.borrow().iter() {
                if crate::eval::strict_eq(ek, &k, i.rt) {
                    return Ok(ev.clone());
                }
            }
        }
    }
    Ok(V::Undef)
}
fn m_has(i: &mut I, this: V, args: &[V]) -> Res<V> {
    if let V::Obj(r) = this {
        if let ObjKind::MapObj(m) = &i.rt.obj(r).kind {
            let k = arg(args, 0);
            return Ok(V::Bool(
                m.borrow()
                    .iter()
                    .any(|(ek, _)| crate::eval::strict_eq(ek, &k, i.rt)),
            ));
        }
    }
    Ok(V::Bool(false))
}
fn m_delete(i: &mut I, this: V, args: &[V]) -> Res<V> {
    if let V::Obj(r) = this {
        if let ObjKind::MapObj(m) = &i.rt.obj(r).kind {
            let k = arg(args, 0);
            let mut m = m.borrow_mut();
            let before = m.len();
            m.retain(|(ek, _)| !crate::eval::strict_eq(ek, &k, i.rt));
            return Ok(V::Bool(m.len() != before));
        }
    }
    Ok(V::Bool(false))
}
fn m_size(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    if let V::Obj(r) = this {
        match &i.rt.obj(r).kind {
            ObjKind::MapObj(m) => return Ok(V::Num(m.borrow().len() as f64)),
            ObjKind::SetObj(s) => return Ok(V::Num(s.borrow().len() as f64)),
            _ => {}
        }
    }
    Ok(V::Num(0.0))
}
fn m_keys_iter(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    if let V::Obj(r) = this {
        if let ObjKind::MapObj(m) = &i.rt.obj(r).kind {
            let items: Vec<V> = m.borrow().iter().map(|(k, _)| k.clone()).collect();
            let o = i.rt.alloc(Obj::new(
                ObjKind::Iter(RefCell::new(items.into_iter())),
                i.rt.iter_proto,
            ));
            return Ok(V::Obj(o));
        }
    }
    Ok(V::Undef)
}
fn m_values_iter(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    if let V::Obj(r) = this {
        let items: Vec<V> = match &i.rt.obj(r).kind {
            ObjKind::MapObj(m) => m.borrow().iter().map(|(_, v)| v.clone()).collect(),
            ObjKind::SetObj(s) => s.borrow().clone(),
            _ => vec![],
        };
        let o = i.rt.alloc(Obj::new(
            ObjKind::Iter(RefCell::new(items.into_iter())),
            i.rt.iter_proto,
        ));
        return Ok(V::Obj(o));
    }
    Ok(V::Undef)
}
fn m_entries_iter(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    if let V::Obj(r) = this {
        let pairs: Vec<(V, V)> = match &i.rt.obj(r).kind {
            ObjKind::MapObj(m) => m.borrow().clone(),
            ObjKind::SetObj(s) => s.borrow().iter().map(|v| (v.clone(), v.clone())).collect(),
            _ => vec![],
        };
        let items: Vec<V> = pairs
            .into_iter()
            .map(|(k, v)| V::Obj(alloc_arr(i.rt, vec![k, v])))
            .collect();
        let o = i.rt.alloc(Obj::new(
            ObjKind::Iter(RefCell::new(items.into_iter())),
            i.rt.iter_proto,
        ));
        return Ok(V::Obj(o));
    }
    Ok(V::Undef)
}
fn m_foreach(i: &mut I, this: V, args: &[V]) -> Res<V> {
    let f = arg(args, 0);
    if let V::Obj(r) = this {
        let pairs: Vec<(V, V)> = match &i.rt.obj(r).kind {
            ObjKind::MapObj(m) => m.borrow().clone(),
            ObjKind::SetObj(s) => s.borrow().iter().map(|v| (v.clone(), v.clone())).collect(),
            _ => vec![],
        };
        for (k, v) in pairs {
            i.call(f.clone(), this.clone(), vec![v, k, this.clone()])?;
        }
    }
    Ok(V::Undef)
}
fn s_ctor(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let o = i.rt.alloc(Obj::new(
        ObjKind::SetObj(RefCell::new(Vec::new())),
        i.rt.set_proto,
    ));
    if let V::Obj(r) = arg(args, 0) {
        if let ObjKind::Arr(a) = &i.rt.obj(r).kind {
            let items = a.borrow().clone();
            if let ObjKind::SetObj(s) = &i.rt.obj(o).kind {
                *s.borrow_mut() = items;
            }
        }
    }
    Ok(V::Obj(o))
}
fn s_add(i: &mut I, this: V, args: &[V]) -> Res<V> {
    if let V::Obj(r) = this {
        if let ObjKind::SetObj(s) = &i.rt.obj(r).kind {
            let k = arg(args, 0);
            let mut s = s.borrow_mut();
            if !s.iter().any(|ek| crate::eval::strict_eq(ek, &k, i.rt)) {
                s.push(k);
            }
        }
    }
    Ok(this)
}
fn s_has(i: &mut I, this: V, args: &[V]) -> Res<V> {
    if let V::Obj(r) = this {
        if let ObjKind::SetObj(s) = &i.rt.obj(r).kind {
            let k = arg(args, 0);
            return Ok(V::Bool(
                s.borrow()
                    .iter()
                    .any(|ek| crate::eval::strict_eq(ek, &k, i.rt)),
            ));
        }
    }
    Ok(V::Bool(false))
}
fn s_delete(i: &mut I, this: V, args: &[V]) -> Res<V> {
    if let V::Obj(r) = this {
        if let ObjKind::SetObj(s) = &i.rt.obj(r).kind {
            let k = arg(args, 0);
            let mut s = s.borrow_mut();
            let before = s.len();
            s.retain(|ek| !crate::eval::strict_eq(ek, &k, i.rt));
            return Ok(V::Bool(s.len() != before));
        }
    }
    Ok(V::Bool(false))
}

// Date
fn d_ctor(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let t = if args.is_empty() {
        sol_util::now_ms() as f64
    } else {
        arg(args, 0).to_num(i.rt)
    };
    let proto = i.rt.date_proto;
    let o = i.rt.alloc(Obj::new(ObjKind::DateObj(t), proto));
    Ok(V::Obj(o))
}
fn d_now(_i: &mut I, _t: V, _a: &[V]) -> Res<V> {
    Ok(V::Num(sol_util::now_ms() as f64))
}
fn d_gettime(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    if let V::Obj(r) = this {
        if let ObjKind::DateObj(t) = &i.rt.obj(r).kind {
            return Ok(V::Num(*t));
        }
    }
    Ok(V::Num(f64::NAN))
}

// iterator next()
fn iter_next(i: &mut I, this: V, _a: &[V]) -> Res<V> {
    let res = alloc_ord(i.rt);
    if let V::Obj(r) = this {
        let next = if let ObjKind::Iter(it) = &i.rt.obj(r).kind {
            Some(it.borrow_mut().next())
        } else {
            None
        };
        if let Some(nx) = next {
            match nx {
                Some(v) => {
                    i.rt.set(res, "value", v);
                    i.rt.set(res, "done", V::Bool(false));
                }
                None => {
                    i.rt.set(res, "value", V::Undef);
                    i.rt.set(res, "done", V::Bool(true));
                }
            }
            return Ok(V::Obj(res));
        }
    }
    i.rt.set(res, "done", V::Bool(true));
    Ok(V::Obj(res))
}

// fetch → kernel fetcher
fn f_fetch(i: &mut I, _t: V, args: &[V]) -> Res<V> {
    let url = arg(args, 0).to_str(i.rt);
    let pr = alloc_promise(i.rt);
    let fetcher = i.rt.fetcher.clone();
    match fetcher {
        Some(f) => match f(&url, "GET", None) {
            Ok((status, body, headers)) => {
                let resp = i.rt.alloc(Obj::new(ObjKind::Ordinary, i.rt.obj_proto));
                i.rt.set(resp, "ok", V::Bool((200..300).contains(&status)));
                i.rt.set(resp, "status", V::Num(status as f64));
                let body2 = body.clone();
                let body3 = body.clone();
                let txt = native(i.rt, "text", move |i2, _t, _a| {
                    let pr2 = alloc_promise(i2.rt);
                    settle(i2, pr2, 1, V::s(body2.clone()));
                    Ok(V::Obj(pr2))
                });
                let jsn = native(i.rt, "json", move |i2, _t, _a| {
                    let v = json_parse(i2, V::Undef, &[V::s(body3.clone())])?;
                    let pr2 = alloc_promise(i2.rt);
                    settle(i2, pr2, 1, v);
                    Ok(V::Obj(pr2))
                });
                i.rt.set(resp, "text", txt);
                i.rt.set(resp, "json", jsn);
                let hdrs = i.rt.alloc(Obj::new(ObjKind::Ordinary, i.rt.obj_proto));
                for (k, v) in headers {
                    i.rt.set(hdrs, &k.to_ascii_lowercase(), V::s(v));
                }
                i.rt.set(resp, "headers", V::Obj(hdrs));
                p_resolve_impl(i, pr, V::Obj(resp));
            }
            Err(e) => settle(i, pr, 2, V::s(e)),
        },
        None => settle(i, pr, 2, V::s("fetch unavailable")),
    }
    Ok(V::Obj(pr))
}

// ---------- install ----------

pub fn install(i: &mut I) {
    let rt = &mut *i.rt;
    let g = rt.global_env.clone();

    // value globals
    let decl = |_rt: &mut Rt, name: &str, v: V| {
        crate::rt::Env::declare(&g, name, v);
    };
    decl(rt, "undefined", V::Undef);
    decl(rt, "NaN", V::Num(f64::NAN));
    decl(rt, "Infinity", V::Num(f64::INFINITY));
    let g1 = alloc_ord(rt);
    rt.global_obj = g1;
    // window === globalThis in browsers
    decl(rt, "globalThis", V::Obj(g1));
    decl(rt, "window", V::Obj(g1));

    // Function.prototype call/apply/bind
    let fp = rt.func_proto;
    put_fn(rt, fp, "call", |i, this, a| {
        let t = arg(a, 0);
        let rest: Vec<V> = if a.len() > 1 { a[1..].to_vec() } else { vec![] };
        i.call(this, t, rest)
    });
    put_fn(rt, fp, "apply", |i, this, a| {
        let t = arg(a, 0);
        let argv = match arg(a, 1) {
            V::Obj(r) => arr_items(i.rt, r),
            _ => vec![],
        };
        i.call(this, t, argv)
    });
    put_fn(rt, fp, "bind", |i, this, a| {
        let bound_this = arg(a, 0);
        let bound_args: Vec<V> = if a.len() > 1 { a[1..].to_vec() } else { vec![] };
        let fv = this.clone();
        Ok(native(i.rt, "bound", move |i2, _t, a2| {
            let mut argv = bound_args.clone();
            argv.extend_from_slice(a2);
            i2.call(fv.clone(), bound_this.clone(), argv)
        }))
    });

    // console
    let con = alloc_ord(rt);
    rt.obj_mut(con).kind = ObjKind::Console;
    put_fn(rt, con, "log", f_console);
    put_fn(rt, con, "warn", f_console);
    put_fn(rt, con, "error", f_console);
    put_fn(rt, con, "info", f_console);
    put_fn(rt, con, "debug", f_console);
    decl(rt, "console", V::Obj(con));
    let alert = native(rt, "alert", f_alert);
    decl(rt, "alert", alert);

    // Object
    let obj_ctor = native(rt, "Object", |i, _t, a| match arg(a, 0) {
        V::Undef | V::Null => Ok(V::Obj(alloc_ord(i.rt))),
        v @ V::Obj(_) => Ok(v),
        V::Str(s) => {
            let proto = i.rt.str_proto;
            let o = i.rt.alloc(Obj::new(ObjKind::StrObj(s), proto));
            Ok(V::Obj(o))
        }
        V::Num(n) => {
            let proto = i.rt.num_proto;
            let o = i.rt.alloc(Obj::new(ObjKind::NumObj(n), proto));
            Ok(V::Obj(o))
        }
        V::Bool(b) => {
            let proto = i.rt.bool_proto;
            let o = i.rt.alloc(Obj::new(ObjKind::BoolObj(b), proto));
            Ok(V::Obj(o))
        }
    });
    if let V::Obj(oc) = &obj_ctor {
        put_fn(rt, *oc, "keys", o_keys);
        put_fn(rt, *oc, "values", o_values);
        put_fn(rt, *oc, "entries", o_entries);
        put_fn(rt, *oc, "assign", o_assign);
        put_fn(rt, *oc, "create", o_create);
        put_fn(rt, *oc, "freeze", o_freeze);
        put_fn(rt, *oc, "getPrototypeOf", o_getproto);
        put_fn(rt, *oc, "setPrototypeOf", o_setproto);
        put_fn(rt, *oc, "defineProperty", o_defineprop);
        put_fn(rt, *oc, "defineProperties", o_defineprops);
        put_fn(rt, *oc, "is", o_is);
        // also install on proto
        put_fn(rt, rt.obj_proto, "hasOwnProperty", o_hasown);
        put_fn(rt, rt.obj_proto, "toString", o_tostring);
        put(rt, *oc, "prototype", V::Obj(rt.obj_proto));
    }
    decl(rt, "Object", obj_ctor);

    // Array ctor
    let arr_ctor = native(rt, "Array", |i, _t, a| {
        if a.len() == 1 && matches!(a[0], V::Num(_)) {
            let n = a[0].to_num(i.rt).max(0.0) as usize;
            return Ok(V::Obj(alloc_arr(i.rt, vec![V::Undef; n])));
        }
        Ok(V::Obj(alloc_arr(i.rt, a.to_vec())))
    });
    if let V::Obj(ac) = &arr_ctor {
        put_fn(rt, *ac, "isArray", |i, _t, a| {
            Ok(V::Bool(matches!(
                arg(a, 0),
                V::Obj(r) if matches!(i.rt.obj(r).kind, ObjKind::Arr(_))
            )))
        });
        put_fn(rt, *ac, "of", |i, _t, a| {
            Ok(V::Obj(alloc_arr(i.rt, a.to_vec())))
        });
        put_fn(rt, *ac, "from", |i, _t, a| {
            let (items, pairs) = match arg(a, 0) {
                V::Obj(r) => match &i.rt.obj(r).kind {
                    ObjKind::Arr(v) => (v.borrow().clone(), None),
                    ObjKind::StrObj(s) => (s.chars().map(|c| V::s(c.to_string())).collect(), None),
                    ObjKind::SetObj(s) => (s.borrow().clone(), None),
                    ObjKind::MapObj(m) => (Vec::new(), Some(m.borrow().clone())),
                    _ => (Vec::new(), None),
                },
                V::Str(s) => (s.chars().map(|c| V::s(c.to_string())).collect(), None),
                _ => (Vec::new(), None),
            };
            let items = match pairs {
                Some(pairs) => pairs
                    .into_iter()
                    .map(|(k, v)| V::Obj(alloc_arr(i.rt, vec![k, v])))
                    .collect(),
                None => items,
            };
            Ok(V::Obj(alloc_arr(i.rt, items)))
        });
        let ap = rt.arr_proto;
        put(rt, *ac, "prototype", V::Obj(ap));
    }
    decl(rt, "Array", arr_ctor);

    // String ctor + proto methods on str_proto
    let str_ctor = native(rt, "String", |i, _t, a| Ok(V::s(arg(a, 0).to_str(i.rt))));
    if let V::Obj(sc) = &str_ctor {
        put_fn(rt, *sc, "fromCharCode", |i, _t, a| {
            let s: String = a
                .iter()
                .map(|v| char::from_u32(v.to_num(i.rt) as u32).unwrap_or('\u{fffd}'))
                .collect();
            Ok(V::s(s))
        });
        put_fn(rt, *sc, "fromCodePoint", |i, _t, a| {
            let s: String = a
                .iter()
                .map(|v| char::from_u32(v.to_num(i.rt) as u32).unwrap_or('\u{fffd}'))
                .collect();
            Ok(V::s(s))
        });
        put(rt, *sc, "prototype", V::Obj(rt.str_proto));
    }
    decl(rt, "String", str_ctor);
    let sp = rt.str_proto;
    put_fn(rt, sp, "charAt", s_charat);
    put_fn(rt, sp, "charCodeAt", s_charcodeat);
    put_fn(rt, sp, "codePointAt", s_codepointat);
    put_fn(rt, sp, "includes", s_includes);
    put_fn(rt, sp, "indexOf", s_indexof);
    put_fn(rt, sp, "lastIndexOf", s_lastindexof);
    put_fn(rt, sp, "slice", s_slice);
    put_fn(rt, sp, "substring", s_substring);
    put_fn(rt, sp, "substr", s_substr);
    put_fn(rt, sp, "split", s_split);
    put_fn(rt, sp, "replace", s_replace);
    put_fn(rt, sp, "replaceAll", s_replaceall);
    put_fn(rt, sp, "match", s_match);
    put_fn(rt, sp, "search", s_search);
    put_fn(rt, sp, "trim", s_trim);
    put_fn(rt, sp, "trimStart", s_trimstart);
    put_fn(rt, sp, "trimEnd", s_trimend);
    put_fn(rt, sp, "toLowerCase", s_tolower);
    put_fn(rt, sp, "toUpperCase", s_toupper);
    put_fn(rt, sp, "startsWith", s_startswith);
    put_fn(rt, sp, "endsWith", s_endswith);
    put_fn(rt, sp, "repeat", s_repeat);
    put_fn(rt, sp, "padStart", s_padstart);
    put_fn(rt, sp, "padEnd", s_padend);
    put_fn(rt, sp, "at", s_at);
    put_fn(rt, sp, "toString", s_tostring);
    put_fn(rt, sp, "concat", |i, t, a| {
        let mut s = t.to_str(i.rt);
        for v in a {
            s.push_str(&v.to_str(i.rt));
        }
        Ok(V::s(s))
    });

    // Number
    let num_ctor = native(rt, "Number", |i, _t, a| Ok(V::Num(arg(a, 0).to_num(i.rt))));
    if let V::Obj(nc) = &num_ctor {
        put_fn(rt, *nc, "isInteger", |_i, _t, a| {
            Ok(V::Bool(matches!(arg(a, 0), V::Num(n) if n.fract() == 0.0)))
        });
        put_fn(rt, *nc, "isNaN", f_isnan);
        put_fn(rt, *nc, "isFinite", f_isfinite);
        put(
            rt,
            *nc,
            "MAX_SAFE_INTEGER",
            V::Num(9.007_199_254_740_991e15),
        );
        put(
            rt,
            *nc,
            "MIN_SAFE_INTEGER",
            V::Num(-9.007_199_254_740_991e15),
        );
        put(rt, *nc, "MAX_VALUE", V::Num(f64::MAX));
        put(rt, *nc, "MIN_VALUE", V::Num(f64::MIN_POSITIVE));
        put(rt, *nc, "NaN", V::Num(f64::NAN));
        put(rt, *nc, "POSITIVE_INFINITY", V::Num(f64::INFINITY));
        put(rt, *nc, "NEGATIVE_INFINITY", V::Num(f64::NEG_INFINITY));
        put(rt, *nc, "prototype", V::Obj(rt.num_proto));
        put_fn(rt, *nc, "parseInt", f_parseint);
        put_fn(rt, *nc, "parseFloat", f_parsefloat);
    }
    decl(rt, "Number", num_ctor);
    let np = rt.num_proto;
    put_fn(rt, np, "toFixed", n_tofixed);
    put_fn(rt, np, "toPrecision", n_toprecision);
    put_fn(rt, np, "toString", n_tostring);

    // Boolean
    let bool_ctor = native(rt, "Boolean", |i, _t, a| {
        Ok(V::Bool(arg(a, 0).truthy(i.rt)))
    });
    decl(rt, "Boolean", bool_ctor);

    // Math
    let math = alloc_ord(rt);
    rt.obj_mut(math).kind = ObjKind::Math;
    let mf = |_name: &'static str, f: fn(f64) -> f64| {
        move |i: &mut I, _t: V, a: &[V]| Ok(V::Num(f(arg(a, 0).to_num(i.rt))))
    };
    put_fn(rt, math, "abs", mf("abs", f64::abs));
    put_fn(rt, math, "floor", mf("floor", f64::floor));
    put_fn(rt, math, "ceil", mf("ceil", f64::ceil));
    put_fn(rt, math, "round", mf("round", f64::round));
    put_fn(rt, math, "trunc", mf("trunc", f64::trunc));
    put_fn(rt, math, "sqrt", mf("sqrt", f64::sqrt));
    put_fn(rt, math, "cbrt", mf("cbrt", f64::cbrt));
    put_fn(rt, math, "exp", mf("exp", f64::exp));
    put_fn(rt, math, "log", mf("log", f64::ln));
    put_fn(rt, math, "log2", mf("log2", f64::log2));
    put_fn(rt, math, "log10", mf("log10", f64::log10));
    put_fn(rt, math, "sin", mf("sin", f64::sin));
    put_fn(rt, math, "cos", mf("cos", f64::cos));
    put_fn(rt, math, "tan", mf("tan", f64::tan));
    put_fn(rt, math, "asin", mf("asin", f64::asin));
    put_fn(rt, math, "acos", mf("acos", f64::acos));
    put_fn(rt, math, "atan", mf("atan", f64::atan));
    put_fn(
        rt,
        math,
        "sign",
        mf("sign", |x| {
            if x.is_nan() {
                f64::NAN
            } else if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                x
            }
        }),
    );
    put_fn(rt, math, "pow", |i, _t, a| {
        Ok(V::Num(arg(a, 0).to_num(i.rt).powf(arg(a, 1).to_num(i.rt))))
    });
    put_fn(rt, math, "max", |i, _t, a| {
        let mut m = f64::NEG_INFINITY;
        for v in a {
            m = m.max(v.to_num(i.rt));
        }
        Ok(V::Num(m))
    });
    put_fn(rt, math, "min", |i, _t, a| {
        let mut m = f64::INFINITY;
        for v in a {
            m = m.min(v.to_num(i.rt));
        }
        Ok(V::Num(m))
    });
    put_fn(rt, math, "hypot", |i, _t, a| {
        let s: f64 = a.iter().map(|v| v.to_num(i.rt).powi(2)).sum();
        Ok(V::Num(s.sqrt()))
    });
    put_fn(rt, math, "atan2", |i, _t, a| {
        Ok(V::Num(arg(a, 0).to_num(i.rt).atan2(arg(a, 1).to_num(i.rt))))
    });
    put_fn(rt, math, "random", |i, _t, _a| {
        let _ = i;
        // deterministic-enough PRNG
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        Ok(V::Num((n as f64) / (u32::MAX as f64)))
    });
    put(rt, math, "PI", V::Num(std::f64::consts::PI));
    put(rt, math, "E", V::Num(std::f64::consts::E));
    put(rt, math, "LN2", V::Num(std::f64::consts::LN_2));
    put(rt, math, "LN10", V::Num(std::f64::consts::LN_10));
    put(rt, math, "SQRT2", V::Num(std::f64::consts::SQRT_2));
    decl(rt, "Math", V::Obj(math));

    // JSON
    let js = alloc_ord(rt);
    rt.obj_mut(js).kind = ObjKind::Json;
    put_fn(rt, js, "stringify", json_stringify);
    put_fn(rt, js, "parse", json_parse);
    decl(rt, "JSON", V::Obj(js));

    // functions
    for (name, f) in [
        ("parseInt", f_parseint as fn(&mut I, V, &[V]) -> Res<V>),
        ("parseFloat", f_parsefloat),
        ("isNaN", f_isnan),
        ("isFinite", f_isfinite),
        ("setTimeout", f_settimeout),
        ("setInterval", f_setinterval),
        ("clearTimeout", f_cleartimeout),
        ("clearInterval", f_cleartimeout),
        ("queueMicrotask", f_queuemicrotask),
        ("encodeURI", f_encodeuri),
        ("encodeURIComponent", f_encodeuricomp),
        ("decodeURIComponent", f_decodeuricomp),
        ("decodeURI", f_decodeuricomp),
        ("atob", f_atob),
        ("btoa", f_btoa),
        ("fetch", f_fetch),
    ] {
        let v = native(rt, name, f);
        decl(rt, name, v);
    }

    // Promise
    let promise_ctor = native(rt, "Promise", |i, _t, a| p_ctor(i, V::Undef, a));
    if let V::Obj(pc) = &promise_ctor {
        put_fn(rt, *pc, "resolve", p_resolve);
        put_fn(rt, *pc, "reject", p_reject);
        put_fn(rt, *pc, "all", p_all);
        put_fn(rt, *pc, "race", p_race);
        put(rt, *pc, "prototype", V::Obj(rt.promise_proto));
    }
    let pp = rt.promise_proto;
    put_fn(rt, pp, "then", p_then);
    put_fn(rt, pp, "catch", p_catch);
    put_fn(rt, pp, "finally", p_finally);
    rt.set(pp, "constructor", promise_ctor.clone());
    decl(rt, "Promise", promise_ctor);

    // RegExp ctor
    let re_ctor = native(rt, "RegExp", |i, _t, a| {
        let body = arg(a, 0).to_str(i.rt);
        let flags = arg(a, 1).to_str(i.rt);
        let o = i.rt.alloc(Obj::new(
            ObjKind::Regex {
                body,
                flags,
                last: std::cell::Cell::new(0),
            },
            i.rt.re_proto,
        ));
        Ok(V::Obj(o))
    });
    decl(rt, "RegExp", re_ctor);
    let rp = rt.re_proto;
    put_fn(rt, rp, "test", s_test_impl);
    put_fn(rt, rp, "exec", s_exec_impl);
    put_fn(rt, rp, "toString", |i, t, _a| Ok(V::s(t.to_str(i.rt))));

    // Map / Set
    let map_ctor = native(rt, "Map", |i, _t, a| m_ctor(i, V::Undef, a));
    if let V::Obj(mc) = &map_ctor {
        put(rt, *mc, "prototype", V::Obj(rt.map_proto));
    }
    decl(rt, "Map", map_ctor);
    let mp = rt.map_proto;
    put_fn(rt, mp, "set", m_set);
    put_fn(rt, mp, "get", m_get);
    put_fn(rt, mp, "has", m_has);
    put_fn(rt, mp, "delete", m_delete);
    put_fn(rt, mp, "forEach", m_foreach);
    put_fn(rt, mp, "keys", m_keys_iter);
    put_fn(rt, mp, "values", m_values_iter);
    put_fn(rt, mp, "entries", m_entries_iter);
    // size as accessor
    let size_get = native(rt, "size", m_size);
    rt.obj(mp)
        .accessors
        .borrow_mut()
        .insert("size".into(), (size_get.clone(), V::Undef));

    let set_ctor = native(rt, "Set", |i, _t, a| s_ctor(i, V::Undef, a));
    if let V::Obj(sc) = &set_ctor {
        put(rt, *sc, "prototype", V::Obj(rt.set_proto));
    }
    decl(rt, "Set", set_ctor);
    let sp2 = rt.set_proto;
    put_fn(rt, sp2, "add", s_add);
    put_fn(rt, sp2, "has", s_has);
    put_fn(rt, sp2, "delete", s_delete);
    put_fn(rt, sp2, "forEach", m_foreach);
    put_fn(rt, sp2, "values", m_values_iter);
    put_fn(rt, sp2, "keys", m_values_iter);
    put_fn(rt, sp2, "entries", m_entries_iter);
    rt.obj(sp2)
        .accessors
        .borrow_mut()
        .insert("size".into(), (size_get, V::Undef));

    // Date
    let date_ctor = native(rt, "Date", |i, _t, a| d_ctor(i, V::Undef, a));
    if let V::Obj(dc) = &date_ctor {
        put_fn(rt, *dc, "now", d_now);
        put(rt, *dc, "prototype", V::Obj(rt.date_proto));
    }
    decl(rt, "Date", date_ctor);
    let dp = rt.date_proto;
    put_fn(rt, dp, "getTime", d_gettime);
    put_fn(rt, dp, "valueOf", d_gettime);

    // Error ctors: each gets own prototype (child of err_proto) + name
    let ep = rt.err_proto;
    let err_proto_of = |rt: &mut Rt, name: &str| -> usize {
        let p = alloc_ord(rt);
        rt.obj_mut(p).proto.set(ep);
        rt.set_ne(p, "name", V::s(name));
        p
    };
    for name in [
        "Error",
        "TypeError",
        "ReferenceError",
        "SyntaxError",
        "RangeError",
    ] {
        let p = err_proto_of(rt, name);
        let f = native(rt, name, move |i, _t, a| {
            let m = arg(a, 0).to_str(i.rt);
            let o = i.rt.alloc(Obj::new(ObjKind::Err(m.clone()), p));
            i.rt.set_ne(o, "message", V::s(m));
            Ok(V::Obj(o))
        });
        if let V::Obj(fr) = &f {
            rt.set_ne(*fr, "prototype", V::Obj(p));
            rt.set_ne(p, "constructor", f.clone());
        }
        decl(rt, name, f);
    }
    // Error.prototype.name for the base
    rt.set_ne(rt.err_proto, "name", V::s("Error"));

    // iterator proto: gives .next()
    let ip = rt.iter_proto;
    put_fn(rt, ip, "next", iter_next);

    // Symbol stub
    let sym_ctor = native(rt, "Symbol", |i, _t, a| {
        let s = arg(a, 0).to_str(i.rt);
        Ok(V::s(format!("Symbol({s})")))
    });
    decl(rt, "Symbol", sym_ctor);
}

pub fn err_of(rt: &mut Rt, msg: impl Into<String>) -> Throw {
    let m = msg.into();
    let o = rt.alloc(Obj::new(ObjKind::Err(m.clone()), rt.err_proto));
    rt.set(o, "message", V::s(m));
    Throw::val(V::Obj(o))
}
