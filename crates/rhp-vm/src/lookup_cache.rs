//! Run-time caches for lookups by name: php's run-time cache slots
//! (CACHE_ADDR / CACHE_PTR), which remember what a FETCH_OBJ_R,
//! INIT_METHOD_CALL or FETCH_CLASS resolved to.
//!
//! php keeps one slot per opline in the op_array. rhp's protos are shared
//! and never written (docs/design/bytecode.md §3.6), so the slots live in
//! the VM's own data instead: a small two-way table in each linked
//! class for its property, static property and method lookups, and one in the VM for class
//! names. An entry is keyed by the name's hash (a `PhpStr` caches it) and
//! holds only indices into the class's own tables, so a hit is checked
//! against the name the table stores and nothing outlives what it points
//! at. A miss falls back to the ordinary lookup, which fills the entry.
//!
//! Property entries also depend on the calling scope's class; when a class
//! id is reused (a class whose linking failed is removed), the VM empties
//! them all ([`Vm::flush_lookup_caches`]).

use std::cell::{Cell, OnceCell};

use rhp_values::{ClassId, PhpStr};

use crate::Vm;

/// Slots per class and kind; a power of two.
const SLOTS: usize = 16;
/// Slots of the VM's class-name cache; a power of two.
const CLASS_SLOTS: usize = 256;

#[derive(Clone, Copy, Default)]
struct Entry {
    /// The looked-up name's hash; 0 = empty (a `PhpStr` hash is never 0).
    hash: u64,
    /// The calling scope's class (`u32::MAX`: none), for properties.
    scope: u32,
    /// The index in the class's table (property slot, method entry, or
    /// class id).
    at: u32,
}

#[inline(always)]
fn slot(hash: u64, scope: u32, n: usize) -> usize {
    let x = (hash ^ scope as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (x >> 40) as usize & (n - 1)
}

/// A lazily allocated two-way table: a key's slot and its neighbour.
#[derive(Default)]
struct Table(OnceCell<Box<[Cell<Entry>]>>);

impl Table {
    #[inline(always)]
    fn get(&self, hash: u64, scope: u32, n: usize) -> Option<u32> {
        let t = self.0.get()?;
        let i = slot(hash, scope, n);
        let e = t[i].get();
        if e.hash == hash && e.scope == scope {
            return Some(e.at);
        }
        let e = t[i ^ 1].get();
        (e.hash == hash && e.scope == scope).then_some(e.at)
    }

    #[cold]
    fn put(&self, hash: u64, scope: u32, at: u32, n: usize) {
        // Engine data php keeps in the op_array's run-time cache: not
        // counted in memory_get_usage().
        let t = rhp_values::mem::uncounted(|| {
            self.0
                .get_or_init(|| (0..n).map(|_| Cell::new(Entry::default())).collect())
        });
        // Two ways: a key whose slot is taken goes to its neighbour, so two
        // keys a loop uses together (a property read from the class's
        // scope and from outside) don't evict each other on every pass.
        let i = slot(hash, scope, n);
        let i = if t[i].get().hash != 0 && t[i ^ 1].get().hash == 0 {
            i ^ 1
        } else {
            i
        };
        t[i].set(Entry { hash, scope, at });
    }
}

/// A linked class's property and method lookup cache. Cloning gives an
/// empty one.
#[derive(Default)]
pub struct ClassCache {
    props: Table,
    methods: Table,
    statics: Table,
}

impl Clone for ClassCache {
    fn clone(&self) -> Self {
        ClassCache::default()
    }
}

#[inline(always)]
fn scope_key(scope: Option<ClassId>) -> u32 {
    scope.unwrap_or(u32::MAX)
}

impl ClassCache {
    /// The property slot `name` resolved to from `scope`, if cached.
    #[inline(always)]
    pub(crate) fn prop(&self, name: &PhpStr, scope: Option<ClassId>) -> Option<usize> {
        self.props
            .get(name.hash_value(), scope_key(scope), SLOTS)
            .map(|at| at as usize)
    }

    pub(crate) fn put_prop(&self, name: &PhpStr, scope: Option<ClassId>, at: usize) {
        self.props
            .put(name.hash_value(), scope_key(scope), at as u32, SLOTS);
    }

    /// The method-table index `name` resolved to, if cached.
    #[inline(always)]
    pub(crate) fn method(&self, name: &PhpStr) -> Option<usize> {
        self.methods
            .get(name.hash_value(), 0, SLOTS)
            .map(|at| at as usize)
    }

    pub(crate) fn put_method(&self, name: &PhpStr, at: usize) {
        self.methods.put(name.hash_value(), 0, at as u32, SLOTS);
    }

    /// The static-property table index `name` resolved to, if cached.
    #[inline(always)]
    pub(crate) fn static_prop(&self, name: &PhpStr) -> Option<usize> {
        self.statics
            .get(name.hash_value(), 0, SLOTS)
            .map(|at| at as usize)
    }

    pub(crate) fn put_static_prop(&self, name: &PhpStr, at: usize) {
        self.statics.put(name.hash_value(), 0, at as u32, SLOTS);
    }

    /// Forget the property entries (they name scope classes).
    pub(crate) fn flush(&mut self) {
        self.props = Table::default();
    }
}

/// The VM's class-name cache (FETCH_CLASS by name).
#[derive(Default)]
pub(crate) struct ClassNameCache(Table);

impl ClassNameCache {
    #[inline(always)]
    pub(crate) fn get(&self, name: &PhpStr) -> Option<ClassId> {
        self.0.get(name.hash_value(), 0, CLASS_SLOTS)
    }

    pub(crate) fn put(&self, name: &PhpStr, id: ClassId) {
        self.0.put(name.hash_value(), 0, id, CLASS_SLOTS);
    }
}

/// The class-name cache for names the VM holds only as bytes (catch of an
/// engine error, string callables, reflection and the other functions
/// that take a class name): keyed by a hash that ignores ASCII case, so
/// no lowercase copy is made.
#[derive(Default)]
pub(crate) struct ClassBytesCache(Table);

/// A hash of `name` that is the same for every ASCII case of it (and
/// never 0): eight bytes at a time, each with bit 0x20 set.
#[inline]
pub(crate) fn fold_hash(name: &[u8]) -> u64 {
    const K: u64 = 0x517c_c1b7_2722_0a95;
    const FOLD: u64 = 0x2020_2020_2020_2020;
    let mut h = name.len() as u64 ^ 0x9E37_79B9_7F4A_7C15;
    let (chunks, rest) = name.as_chunks::<8>();
    for c in chunks {
        let w = u64::from_le_bytes(*c) | FOLD;
        h = (h.rotate_left(5) ^ w).wrapping_mul(K);
    }
    if !rest.is_empty() {
        let mut b = [0u8; 8];
        b[..rest.len()].copy_from_slice(rest);
        let w = u64::from_le_bytes(b) | (FOLD >> (8 * (8 - rest.len())));
        h = (h.rotate_left(5) ^ w).wrapping_mul(K);
    }
    h | 1
}

impl ClassBytesCache {
    #[inline(always)]
    pub(crate) fn get(&self, hash: u64) -> Option<ClassId> {
        self.0.get(hash, 0, CLASS_SLOTS)
    }

    pub(crate) fn put(&self, hash: u64, id: ClassId) {
        self.0.put(hash, 0, id, CLASS_SLOTS);
    }
}

impl Vm {
    /// Empty the caches whose entries name a class id other than their own
    /// class: after a class was removed, its id is used again.
    pub(crate) fn flush_lookup_caches(&mut self) {
        for c in &mut self.classes {
            c.cache.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fold_hash;

    #[test]
    fn fold_hash_ignores_ascii_case_only() {
        for n in [
            &b"A"[..],
            b"Abcdefgh",
            b"Abcdefghi",
            b"Foo\\Bar\\BazQuxQuux",
        ] {
            assert_eq!(fold_hash(n), fold_hash(&n.to_ascii_lowercase()));
            assert_eq!(fold_hash(n), fold_hash(&n.to_ascii_uppercase()));
            assert_ne!(fold_hash(n) & 1, 0);
        }
        assert_ne!(fold_hash(b"abc"), fold_hash(b"abd"));
        // the length is part of it: a short tail isn't padded into a match
        assert_ne!(fold_hash(b"abc"), fold_hash(b"abc\0"));
        assert_ne!(fold_hash(b""), fold_hash(b"\0"));
    }
}
