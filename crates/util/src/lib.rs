//! sol_util: shared foundations for the sol browser kernel.
//!
//! - `define_id!` generates opaque 32-bit index newtypes.
//! - `Arena<T>` is a generational arena: keys stay valid after removals,
//!   stale keys are detected via a generation counter.
//! - `Interner` maps strings to `Sym` (cheap copyable id) for name compare.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::time::{SystemTime, UNIX_EPOCH};

/// Read an entire file using the standard filesystem API.
pub use std::fs::read as read_file;

#[macro_export]
macro_rules! define_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u32);
        impl $name {
            #[inline] pub const fn new(i: u32) -> Self { Self(i) }
            #[inline] pub fn idx(self) -> usize { self.0 as usize }
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.0)
            }
        }
    };
}

/// Interned string symbol.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sym(pub u32);

impl std::fmt::Debug for Sym {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sym({})", self.0)
    }
}

/// String interner. Symbols compare by id; resolve to get &str.
#[derive(Default)]
pub struct Interner {
    map: HashMap<Box<str>, Sym>,
    strings: Vec<Box<str>>,
}

impl Interner {
    pub fn new() -> Self {
        let mut it = Interner::default();
        // Pre-intern common empty string.
        it.intern("");
        it
    }
    pub fn intern(&mut self, s: &str) -> Sym {
        if let Some(&s2) = self.map.get(s) {
            return s2;
        }
        let sym = Sym(self.strings.len() as u32);
        let b: Box<str> = s.into();
        self.strings.push(b.clone());
        self.map.insert(b, sym);
        sym
    }
    pub fn resolve(&self, s: Sym) -> &str {
        &self.strings[s.0 as usize]
    }
    pub fn get(&self, s: &str) -> Option<Sym> {
        self.map.get(s).copied()
    }
}

/// Generational arena. `K` is a key type built via `define_key!` that
/// carries (index, generation). Use `PlainArena` if you don't need reuse.
pub struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
}

struct Slot<T> {
    gen: u32,
    val: Option<T>,
}

/// Key into an `Arena`/`GenMap`. Generic over the element type so keys are
/// type-safe: `GKey<Node>` cannot index `GKey<Edge>` arenas.
pub struct GKey<T> {
    pub idx: u32,
    pub gen: u32,
    _pd: PhantomData<fn() -> T>,
}

impl<T> Copy for GKey<T> {}
impl<T> Clone for GKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> PartialEq for GKey<T> {
    fn eq(&self, o: &Self) -> bool {
        self.idx == o.idx && self.gen == o.gen
    }
}
impl<T> Eq for GKey<T> {}
impl<T> std::hash::Hash for GKey<T> {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.idx.hash(h);
        self.gen.hash(h);
    }
}
impl<T> std::fmt::Debug for GKey<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GKey({}:{})", self.idx, self.gen)
    }
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }
}

impl<T> Arena<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, v: T) -> GKey<T> {
        if let Some(i) = self.free.pop() {
            let s = &mut self.slots[i as usize];
            // Slot reuse must not make an old node/object handle refer to a
            // different allocation. Generation changes preserve that distinction
            // even when the new value occupies exactly the same index.
            s.gen += 1;
            s.val = Some(v);
            GKey {
                idx: i,
                gen: s.gen,
                _pd: PhantomData,
            }
        } else {
            let idx = self.slots.len() as u32;
            self.slots.push(Slot {
                gen: 0,
                val: Some(v),
            });
            GKey {
                idx,
                gen: 0,
                _pd: PhantomData,
            }
        }
    }

    pub fn remove(&mut self, k: GKey<T>) -> Option<T> {
        let s = self.slots.get_mut(k.idx as usize)?;
        if s.gen != k.gen || s.val.is_none() {
            return None;
        }
        self.free.push(k.idx);
        s.val.take()
    }

    #[inline]
    pub fn get(&self, k: GKey<T>) -> Option<&T> {
        let s = self.slots.get(k.idx as usize)?;
        if s.gen == k.gen {
            s.val.as_ref()
        } else {
            None
        }
    }
    #[inline]
    pub fn get_mut(&mut self, k: GKey<T>) -> Option<&mut T> {
        let s = self.slots.get_mut(k.idx as usize)?;
        if s.gen == k.gen {
            s.val.as_mut()
        } else {
            None
        }
    }
    #[inline]
    pub fn contains(&self, k: GKey<T>) -> bool {
        self.get(k).is_some()
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.val.is_some()).count()
    }
    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(|s| s.val.is_none())
    }
    pub fn iter(&self) -> impl Iterator<Item = (GKey<T>, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.val.as_ref().map(|v| {
                (
                    GKey {
                        idx: i as u32,
                        gen: s.gen,
                        _pd: PhantomData,
                    },
                    v,
                )
            })
        })
    }
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.slots.iter().filter_map(|s| s.val.as_ref())
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.slots.iter_mut().filter_map(|s| s.val.as_mut())
    }
}

/// Simple non-generational dense arena used when keys outlive all removals.
/// Faster and simpler; `PKey` is a plain index.
pub struct DenseArena<T> {
    items: Vec<T>,
}

pub struct PKey<T> {
    pub idx: u32,
    _pd: PhantomData<fn() -> T>,
}
impl<T> Copy for PKey<T> {}
impl<T> Clone for PKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> PartialEq for PKey<T> {
    fn eq(&self, o: &Self) -> bool {
        self.idx == o.idx
    }
}
impl<T> Eq for PKey<T> {}
impl<T> std::hash::Hash for PKey<T> {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.idx.hash(h);
    }
}
impl<T> std::fmt::Debug for PKey<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PKey({})", self.idx)
    }
}
impl<T> PKey<T> {
    pub fn idx(self) -> usize {
        self.idx as usize
    }
}

impl<T> Default for DenseArena<T> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}
impl<T> DenseArena<T> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, v: T) -> PKey<T> {
        let idx = self.items.len() as u32;
        self.items.push(v);
        PKey {
            idx,
            _pd: PhantomData,
        }
    }
    pub fn get(&self, k: PKey<T>) -> &T {
        &self.items[k.idx as usize]
    }
    pub fn get_mut(&mut self, k: PKey<T>) -> &mut T {
        &mut self.items[k.idx as usize]
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = (PKey<T>, &T)> {
        self.items.iter().enumerate().map(|(i, v)| {
            (
                PKey {
                    idx: i as u32,
                    _pd: PhantomData,
                },
                v,
            )
        })
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.items.iter_mut()
    }
}

/// Milliseconds since unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_gen() {
        let mut a: Arena<u32> = Arena::new();
        let k1 = a.insert(10);
        let k2 = a.insert(20);
        assert_eq!(*a.get(k1).unwrap(), 10);
        a.remove(k1);
        assert!(a.get(k1).is_none());
        let k3 = a.insert(30); // reuses slot of k1 with new gen
        assert_eq!(k3.idx, k1.idx);
        assert_ne!(k3.gen, k1.gen);
        assert_eq!(*a.get(k3).unwrap(), 30);
        assert!(a.get(k1).is_none()); // stale key rejected
        assert_eq!(*a.get(k2).unwrap(), 20);
    }

    #[test]
    fn interner() {
        let mut i = Interner::new();
        let a = i.intern("hello");
        let b = i.intern("hello");
        let c = i.intern("world");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(i.resolve(a), "hello");
    }
}
