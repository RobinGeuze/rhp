//! Object handles (php's `zend_objects_store`, Zend/zend_objects_API.c).
//!
//! Every object php can show a `#N` for takes a handle from here: objects,
//! closures, generators, enum cases. A freed handle goes on a free list and
//! the next object takes the most recently freed one (LIFO), as
//! `zend_objects_store_put` does. Once shutdown destructors start, freed
//! handles are not reused any more (`EG_FLAGS_OBJECT_STORE_NO_REUSE`).
//!
//! php allocates the store itself as request memory (1024 slots, doubled
//! when full); the slots past the first 1024 are allocated here too, so
//! `memory_limit` sees them (handles not reused at shutdown fill it).
//!
//! The store is thread-local because handles are freed from `Drop`, where
//! there is no engine. One engine runs per thread at a time; each new
//! engine (and each engine's drop) resets it.

use std::cell::{Cell, RefCell};
use std::mem::ManuallyDrop;

struct Store {
    /// The next never-used handle (handles start at 1).
    next: Cell<u32>,
    /// Freed handles, most recently freed last. Never dropped (a few bytes
    /// per thread), so the store needs no thread-exit destructor and every
    /// access stays a plain thread-local read.
    free: ManuallyDrop<RefCell<Vec<u32>>>,
    /// Shutdown: freed handles are dropped, not reused.
    no_reuse: Cell<bool>,
    /// php's store size in slots (`objects_store.size`).
    size: Cell<u32>,
    /// The slots past the first [`INIT_SIZE`], as counted request memory.
    grown: ManuallyDrop<RefCell<Vec<usize>>>,
}

/// zend_objects_store_init's size (the slots counted in the baseline).
const INIT_SIZE: u32 = 1024;

thread_local! {
    static STORE: Store = const {
        Store {
            next: Cell::new(1),
            free: ManuallyDrop::new(RefCell::new(Vec::new())),
            no_reuse: Cell::new(false),
            size: Cell::new(INIT_SIZE),
            grown: ManuallyDrop::new(RefCell::new(Vec::new())),
        }
    };
}

/// A handle for a new object: the most recently freed one, else a new one.
#[inline]
pub fn alloc_object_id() -> u32 {
    STORE
        .try_with(|s| {
            if let Some(id) = s.free.try_borrow_mut().ok().and_then(|mut f| f.pop()) {
                return id;
            }
            let id = s.next.get();
            s.next.set(id + 1);
            if id >= s.size.get() {
                grow(s);
            }
            id
        })
        .unwrap_or(0)
}

/// The store is full: double it (`zend_objects_store_put`'s erealloc).
#[cold]
fn grow(s: &Store) {
    let size = s.size.get().saturating_mul(2);
    s.size.set(size);
    if let Ok(mut g) = s.grown.try_borrow_mut() {
        *g = Vec::with_capacity((size - INIT_SIZE) as usize);
    }
}

/// Give a freed object's handle back (`zend_objects_store_del`). Handle 0
/// (an enum case never accessed) and handles from an earlier engine on
/// this thread (at or above `next`) are ignored.
#[inline]
pub fn release_object_id(id: u32) {
    if id == 0 {
        return;
    }
    let _ = STORE.try_with(|s| {
        if s.no_reuse.get() || id >= s.next.get() {
            return;
        }
        if let Ok(mut f) = s.free.try_borrow_mut() {
            if f.len() == f.capacity() {
                // php's free list lives in the freed slots: growing ours is
                // not request memory (and it is never given back).
                let more = f.len().max(64);
                crate::mem::uncounted(|| f.reserve(more));
            }
            f.push(id);
        }
    });
}

/// Stop reusing freed handles (`zend_objects_store_call_destructors`).
pub fn stop_object_id_reuse() {
    let _ = STORE.try_with(|s| {
        s.no_reuse.set(true);
        if let Ok(mut f) = s.free.try_borrow_mut() {
            f.clear();
        }
    });
}

/// A new engine: handles restart at 1.
pub fn reset_object_ids() {
    let _ = STORE.try_with(|s| {
        s.next.set(1);
        s.no_reuse.set(false);
        s.size.set(INIT_SIZE);
        if let Ok(mut g) = s.grown.try_borrow_mut() {
            *g = Vec::new();
        }
        if let Ok(mut f) = s.free.try_borrow_mut() {
            f.clear();
        }
    });
}
