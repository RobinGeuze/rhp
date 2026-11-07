//! The cycle collector's root buffer (php's Zend/zend_gc.c).
//!
//! Values are `Rc`s, so a cycle keeps itself alive. php finds such garbage
//! with a synchronous trial-deletion collector (Bacon and Rajan): when a
//! reference to an array or object goes away and the count stays above
//! zero, the value becomes a *possible root*. The collector (in the VM,
//! which knows what generators and native objects hold) walks the graph
//! from the roots, subtracts the counts the graph itself accounts for, and
//! whatever is left at zero and not reachable from a live node is garbage.
//!
//! This module keeps the buffer: the possible roots in php's order, with
//! php's slot reuse, threshold and statistics, so that `gc_status()` and
//! the automatic runs match php. The VM calls [`possible_root`] where php
//! releases a variable, a property, an element or a frame's `$this`; the
//! buffer holds `Weak`s, and a freed value leaves its slot (php's
//! `GC_REMOVE_FROM_BUFFER`), see [`object_freed`] and [`array_freed`].
//!
//! When the buffer reaches the threshold php collects right there. Here a
//! drop may happen anywhere (inside a builtin, in a `Drop`), so the run is
//! due at the VM's next instruction boundary ([`crate::mem::MemState::tick`]);
//! roots added meanwhile wait in a list and enter the buffer after the run,
//! as they would in php.
//!
//! Everything here is thread-local, like the object handle store: one
//! engine runs per thread at a time.

use std::cell::{Cell, RefCell};
use std::mem::ManuallyDrop;
use std::rc::{Rc, Weak};

use std::sync::atomic::{AtomicUsize, Ordering};

use rustc_hash::FxHashMap;

/// Threads with buffered values or flagged handles: a free takes the
/// thread-local path only when some thread has some. Changed only when a
/// thread's state turns empty or non-empty.
static ANY: AtomicUsize = AtomicUsize::new(0);

/// Run `f` on this thread's counters, keeping [`ANY`] in step.
fn counted(st: &State, f: impl FnOnce()) {
    let live = |st: &State| st.arrays.get() != 0 || st.handles.get() != 0 || st.flagged.get();
    let before = live(st);
    f();
    match (before, live(st)) {
        (false, true) => {
            ANY.fetch_add(1, Ordering::Relaxed);
        }
        (true, false) => {
            ANY.fetch_sub(1, Ordering::Relaxed);
        }
        _ => {}
    }
}

use crate::{ArrayData, ClosureData, ObjectData, Value};

/// php's GC_FIRST_ROOT: slot 0 is never used.
pub const FIRST_ROOT: u32 = 1;
const DEFAULT_BUF_SIZE: u32 = 16 * 1024;
const BUF_GROW_STEP: u32 = 128 * 1024;
const MAX_BUF_SIZE: u32 = 0x4000_0000;
const THRESHOLD_DEFAULT: u32 = 10000 + FIRST_ROOT;
const THRESHOLD_STEP: u32 = 10000;
const THRESHOLD_MAX: u32 = 1_000_000_000;
const THRESHOLD_TRIGGER: u32 = 100;

/// A buffered value: what php's `zend_refcounted *` in a root slot is.
#[derive(Clone)]
pub enum Node {
    Obj(Weak<RefCell<ObjectData>>),
    Closure(Weak<ClosureData>),
    Arr(Weak<ArrayData>),
}

impl Node {
    /// The value, if it is still alive.
    pub fn upgrade(&self) -> Option<Value> {
        Some(match self {
            Node::Obj(w) => Value::Object(w.upgrade()?),
            Node::Closure(w) => Value::Closure(w.upgrade()?),
            Node::Arr(w) => Value::Array(crate::PhpArray(w.upgrade()?)),
        })
    }

    /// Note a closure entering or leaving the buffer on the closure (the
    /// release fast path reads it). The buffer is changed between
    /// instructions, when no object is borrowed.
    fn set_buffered(&self, on: bool) {
        match self {
            Node::Closure(w) => {
                if let Some(c) = w.upgrade() {
                    c.gc_buffered.set(on);
                }
            }
            Node::Obj(_) | Node::Arr(_) => {}
        }
    }

    /// The allocation's address (stable while the `Weak` lives).
    pub fn addr(&self) -> usize {
        match self {
            Node::Obj(w) => w.as_ptr() as usize,
            Node::Closure(w) => w.as_ptr() as usize,
            Node::Arr(w) => w.as_ptr() as usize,
        }
    }
}

/// A root buffer slot (php's tagged `gc_root_buffer.ref`).
#[derive(Clone, Default)]
pub enum Slot {
    #[default]
    Unused,
    Root(Node),
    /// Found to be garbage by the running collection.
    Garbage(Node),
    /// Garbage whose destructor the running collection calls.
    DtorGarbage(Node),
}

impl Slot {
    pub fn node(&self) -> Option<&Node> {
        match self {
            Slot::Unused => None,
            Slot::Root(n) | Slot::Garbage(n) | Slot::DtorGarbage(n) => Some(n),
        }
    }
}

/// php's `zend_gc_globals`.
struct Buf {
    slots: Vec<Slot>,
    /// Free slots below `first_unused`, most recently freed last (php's
    /// `unused` list).
    unused: Vec<u32>,
    first_unused: u32,
    num_roots: u32,
    threshold: u32,
    /// php's `buf_size`: the slots it has allocated.
    buf_size: u32,
    /// The slot of each buffered object or closure, by handle.
    by_handle: Vec<u32>,
    /// The slot of each buffered array, by address.
    by_addr: FxHashMap<usize, u32>,
    /// Roots added once the buffer filled, before the due run.
    deferred: Vec<(Node, Key)>,
}

/// Per-thread collector state outside the buffer (cheap to read in hooks).
struct State {
    enabled: Cell<bool>,
    active: Cell<bool>,
    protected: Cell<bool>,
    /// Releases make no roots ([`drop_quietly`]).
    quiet: Cell<bool>,
    full: Cell<bool>,
    /// A run is due at the next instruction boundary.
    due: Cell<bool>,
    runs: Cell<u32>,
    collected: Cell<u32>,
    collector_time: Cell<u64>,
    dtor_time: Cell<u64>,
    free_time: Cell<u64>,
    /// Arrays in the buffer: `array_freed` looks only when there are some.
    arrays: Cell<u32>,
    /// Objects and closures in the buffer, and whether any handle has
    /// flags: `object_freed` looks only when there are some.
    handles: Cell<u32>,
    flagged: Cell<bool>,
    /// The object last found in the buffer (by address): a frame's `$this`
    /// released call after call is checked here first.
    last: Cell<usize>,
    /// Handles whose object has one reference more than php counts (the
    /// VM's destructor tracking list holds it), and handles whose
    /// destructor a collection called.
    flags: ManuallyDrop<RefCell<Vec<u8>>>,
    buf: ManuallyDrop<RefCell<Buf>>,
}

thread_local! {
    static STATE: State = const {
        State {
            enabled: Cell::new(true),
            active: Cell::new(false),
            protected: Cell::new(false),
            quiet: Cell::new(false),
            full: Cell::new(false),
            due: Cell::new(false),
            runs: Cell::new(0),
            collected: Cell::new(0),
            collector_time: Cell::new(0),
            dtor_time: Cell::new(0),
            free_time: Cell::new(0),
            arrays: Cell::new(0),
            handles: Cell::new(0),
            flagged: Cell::new(false),
            last: Cell::new(0),
            flags: ManuallyDrop::new(RefCell::new(Vec::new())),
            buf: ManuallyDrop::new(RefCell::new(Buf {
                slots: Vec::new(),
                unused: Vec::new(),
                first_unused: FIRST_ROOT,
                num_roots: 0,
                threshold: THRESHOLD_DEFAULT,
                buf_size: DEFAULT_BUF_SIZE,
                by_handle: Vec::new(),
                by_addr: FxHashMap::with_hasher(rustc_hash::FxBuildHasher),
                deferred: Vec::new(),
            })),
        }
    };
}

fn with<T: Default>(f: impl FnOnce(&State) -> T) -> T {
    STATE.try_with(f).unwrap_or_default()
}

/// Grow `v` to hold `n` more elements outside the request's memory
/// accounting (php's root buffer is persistent memory).
fn reserve<T>(v: &mut Vec<T>, n: usize) {
    if v.capacity() - v.len() < n {
        let more = n.max(v.capacity()).max(64);
        crate::mem::uncounted(|| v.reserve(more));
    }
}

impl Buf {
    fn slot_of(&self, node: &Node) -> u32 {
        match node {
            Node::Arr(w) => self
                .by_addr
                .get(&(w.as_ptr() as usize))
                .copied()
                .unwrap_or(0),
            Node::Obj(w) => match w.upgrade() {
                Some(h) => match h.try_borrow() {
                    Ok(o) => self.by_handle.get(o.id as usize).copied().unwrap_or(0),
                    Err(_) => 0,
                },
                None => 0,
            },
            Node::Closure(w) => match w.upgrade() {
                Some(c) => self.by_handle.get(c.id as usize).copied().unwrap_or(0),
                None => 0,
            },
        }
    }

    fn index_of(&self, key: Key) -> u32 {
        match key {
            Key::Handle(id) => self.by_handle.get(id as usize).copied().unwrap_or(0),
            Key::Addr(a) => self.by_addr.get(&a).copied().unwrap_or(0),
        }
    }

    fn set_index(&mut self, key: Key, idx: u32) {
        match key {
            Key::Handle(id) => {
                let i = id as usize;
                if i >= self.by_handle.len() {
                    let n = i + 1 - self.by_handle.len();
                    reserve(&mut self.by_handle, n);
                    self.by_handle.resize(i + 1, 0);
                }
                self.by_handle[i] = idx;
            }
            Key::Addr(a) => {
                if idx == 0 {
                    self.by_addr.remove(&a);
                } else {
                    if self.by_addr.len() == self.by_addr.capacity() {
                        let more = self.by_addr.len().max(64);
                        crate::mem::uncounted(|| self.by_addr.reserve(more));
                    }
                    self.by_addr.insert(a, idx);
                }
            }
        }
    }

    /// php's GC_HAS_UNUSED / GC_FETCH_UNUSED / GC_FETCH_NEXT_UNUSED with
    /// growing: a slot for a new entry, or `None` when the buffer is at
    /// its maximum.
    fn fetch_slot(&mut self, st: &State) -> Option<u32> {
        if let Some(i) = self.unused.pop() {
            return Some(i);
        }
        if self.first_unused == self.buf_size {
            self.grow(st);
            if self.first_unused == self.buf_size {
                return None;
            }
        }
        let i = self.first_unused;
        self.first_unused += 1;
        Some(i)
    }

    /// gc_grow_root_buffer.
    fn grow(&mut self, st: &State) {
        if self.buf_size >= MAX_BUF_SIZE && !st.full.get() {
            // php warns "GC buffer overflow (GC disabled)" here; a billion
            // roots does not fit in a test.
            st.active.set(true);
            st.protected.set(true);
            st.full.set(true);
            return;
        }
        let new = if self.buf_size < BUF_GROW_STEP {
            self.buf_size * 2
        } else {
            self.buf_size + BUF_GROW_STEP
        };
        self.buf_size = new.min(MAX_BUF_SIZE);
    }

    fn put(&mut self, idx: u32, slot: Slot) {
        let i = idx as usize;
        if i >= self.slots.len() {
            let n = i + 1 - self.slots.len();
            reserve(&mut self.slots, n);
            self.slots.resize(i + 1, Slot::Unused);
        }
        self.slots[i] = slot;
    }

    /// Put `node` (with buffer key `key`) in a fresh slot.
    fn insert(&mut self, st: &State, node: Node, key: Key, garbage: bool) -> u32 {
        let Some(idx) = self.fetch_slot(st) else {
            return 0;
        };
        counted(st, || {
            if matches!(node, Node::Arr(_)) {
                st.arrays.set(st.arrays.get() + 1);
            } else {
                st.handles.set(st.handles.get() + 1);
            }
        });
        node.set_buffered(true);
        self.put(
            idx,
            if garbage {
                Slot::Garbage(node)
            } else {
                Slot::Root(node)
            },
        );
        self.set_index(key, idx);
        self.num_roots += 1;
        idx
    }

    /// gc_remove_from_roots: slot `idx` goes on the unused list.
    fn remove(&mut self, st: &State, idx: u32) -> Option<Node> {
        let node = match std::mem::replace(self.slots.get_mut(idx as usize)?, Slot::Unused) {
            Slot::Unused => return None,
            Slot::Root(n) | Slot::Garbage(n) | Slot::DtorGarbage(n) => n,
        };
        counted(st, || {
            if let Node::Arr(_) = node {
                st.arrays.set(st.arrays.get().saturating_sub(1));
            } else {
                st.handles.set(st.handles.get().saturating_sub(1));
            }
        });
        node.set_buffered(false);
        st.last.set(0);
        reserve(&mut self.unused, 1);
        self.unused.push(idx);
        self.num_roots -= 1;
        Some(node)
    }
}

/// How a buffered value is found again.
#[derive(Clone, Copy)]
enum Key {
    Handle(u32),
    Addr(usize),
}

fn key_of(node: &Node) -> Option<Key> {
    Some(match node {
        Node::Arr(w) => Key::Addr(w.as_ptr() as usize),
        Node::Obj(w) => Key::Handle(w.upgrade()?.try_borrow().ok()?.id),
        Node::Closure(w) => Key::Handle(w.upgrade()?.id),
    })
}

/// A reference to `v` (a variable, a property, an element, a frame's
/// `$this`) is about to go away (php's `zval_ptr_dtor` with
/// `gc_check_possible_root`): if `v` stays referenced, it becomes a
/// possible root. A reference adds the value it holds.
#[inline]
pub fn possible_root(v: &Value) {
    let shared = match v {
        Value::Object(h) => Rc::strong_count(h) > 1,
        // An empty array cannot be in a cycle (php's `[]` is the immutable
        // zend_empty_array).
        Value::Array(a) => Rc::strong_count(&a.0) > 1 && !a.0.is_empty(),
        Value::Ref(r) => Rc::strong_count(r) > 1,
        Value::Closure(c) => Rc::strong_count(c) > 1 && !c.gc_buffered.get(),
        _ => false,
    };
    if shared {
        possible_root_slow(v);
    }
}

#[cold]
#[inline(never)]
fn possible_root_slow(v: &Value) {
    match v {
        Value::Object(h) => {
            let addr = Rc::as_ptr(h) as usize;
            let _ = STATE.try_with(|st| {
                if st.last.get() == addr {
                    return;
                }
                let Ok(o) = h.try_borrow() else { return };
                let id = o.id;
                drop(o);
                // The destructor list's reference is not php's.
                let extra = st.flagged.get()
                    && st.flags.try_borrow().is_ok_and(|f| {
                        f.get(id as usize)
                            .is_some_and(|x| x & (TRACKED | HELD) != 0)
                    });
                if Rc::strong_count(h) - usize::from(extra) > 1
                    && add_root_in(st, || Node::Obj(Rc::downgrade(h)), Key::Handle(id))
                {
                    st.last.set(addr);
                }
            });
        }
        Value::Closure(c) => add_root(|| Node::Closure(Rc::downgrade(c)), Key::Handle(c.id)),
        Value::Array(a) => add_array_root(a),
        Value::Ref(r) => {
            if let Ok(inner) = r.try_borrow() {
                check_possible_root(&inner);
            }
        }
        _ => {}
    }
}

/// php's `gc_check_possible_root` on a value whose count was just
/// decremented elsewhere: buffer it when it is not yet.
pub fn check_possible_root(v: &Value) {
    match v {
        Value::Object(h) => {
            if let Ok(o) = h.try_borrow() {
                let id = o.id;
                drop(o);
                add_root(|| Node::Obj(Rc::downgrade(h)), Key::Handle(id));
            }
        }
        Value::Closure(c) => add_root(|| Node::Closure(Rc::downgrade(c)), Key::Handle(c.id)),
        Value::Array(a) => add_array_root(a),
        _ => {}
    }
}

fn add_array_root(a: &crate::PhpArray) {
    if a.0.is_empty() {
        return;
    }
    let addr = Rc::as_ptr(&a.0) as usize;
    add_root(|| Node::Arr(Rc::downgrade(&a.0)), Key::Addr(addr));
}

/// gc_possible_root: buffer `node` unless it is already.
fn add_root(node: impl FnOnce() -> Node, key: Key) {
    let _ = STATE.try_with(|st| add_root_in(st, node, key));
}

/// [`add_root`] on this thread's state: whether the value is in the
/// buffer now.
fn add_root_in(st: &State, node: impl FnOnce() -> Node, key: Key) -> bool {
    {
        if st.protected.get() || st.quiet.get() {
            return false;
        }
        let Ok(mut b) = st.buf.try_borrow_mut() else {
            return false;
        };
        let idx = match key {
            Key::Handle(0) => return false,
            Key::Handle(id) => b.by_handle.get(id as usize).copied().unwrap_or(0),
            Key::Addr(a) => b.by_addr.get(&a).copied().unwrap_or(0),
        };
        if idx != 0 {
            return idx < DEFERRED;
        }
        let node = node();
        if st.due.get() {
            b.set_index(key, DEFERRED);
            reserve(&mut b.deferred, 1);
            b.deferred.push((node, key));
            return false;
        }
        let full = b.unused.is_empty() && b.first_unused >= b.threshold;
        if full && st.enabled.get() && !st.active.get() {
            // gc_possible_root_when_full: php collects now and buffers
            // the value afterwards.
            st.due.set(true);
            crate::mem::state().gc_due(true);
            b.set_index(key, DEFERRED);
            reserve(&mut b.deferred, 1);
            b.deferred.push((node, key));
            return false;
        }
        b.insert(st, node, key, false) != 0
    }
}

/// A collection is due ([`add_root`] found the buffer full).
#[inline]
pub fn due() -> bool {
    with(|st| st.due.get())
}

/// The due collection is starting: clear the flag. The deferred roots
/// stay until [`flush_deferred`].
pub fn clear_due() {
    with(|st| st.due.set(false));
    crate::mem::state().gc_due(false);
}

/// The roots waiting for the due run, alive (php holds the value whose
/// release filled the buffer during the run).
pub fn deferred_values() -> Vec<Value> {
    with(|st| match st.buf.try_borrow() {
        Ok(b) => b.deferred.iter().filter_map(|(n, _)| n.upgrade()).collect(),
        Err(_) => Vec::new(),
    })
}

/// After the due run: buffer the roots that waited for it.
pub fn flush_deferred() {
    let nodes = with(|st| match st.buf.try_borrow_mut() {
        Ok(mut b) => {
            let nodes = std::mem::take(&mut b.deferred);
            for &(_, key) in &nodes {
                if b.index_of(key) == DEFERRED {
                    b.set_index(key, 0);
                }
            }
            nodes
        }
        Err(_) => Vec::new(),
    });
    // php adds them after the run without checking the threshold again
    // (gc_possible_root_when_full): no run is due from these.
    let was = active();
    set_active(true);
    for (n, _) in &nodes {
        if let Some(v) = n.upgrade() {
            check_possible_root(&v);
        }
    }
    set_active(was);
}

/// gc_adjust_threshold, after an automatic run that collected `count`.
pub fn adjust_threshold(count: u32) {
    with(|st| {
        let Ok(mut b) = st.buf.try_borrow_mut() else {
            return;
        };
        if count < THRESHOLD_TRIGGER || b.num_roots >= b.threshold {
            if b.threshold < THRESHOLD_MAX {
                let new = (b.threshold + THRESHOLD_STEP).min(THRESHOLD_MAX);
                if new > b.buf_size {
                    b.grow(st);
                }
                if new <= b.buf_size {
                    b.threshold = new;
                }
            }
        } else if b.threshold > THRESHOLD_DEFAULT {
            b.threshold = (b.threshold - THRESHOLD_STEP).max(THRESHOLD_DEFAULT);
        }
    })
}

/// An object or closure with handle `id` is being freed: it leaves the
/// buffer (php's `GC_REMOVE_FROM_BUFFER` in `zend_objects_store_del`).
#[inline]
pub fn object_freed(id: u32) {
    if ANY.load(Ordering::Relaxed) == 0 {
        return;
    }
    let _ = STATE.try_with(|st| {
        if st.handles.get() == 0 && !st.flagged.get() {
            return;
        }
        if st.flagged.get() {
            if let Ok(mut f) = st.flags.try_borrow_mut() {
                if let Some(x) = f.get_mut(id as usize) {
                    *x = 0;
                }
            }
        }
        if st.handles.get() == 0 {
            return;
        }
        let Ok(mut b) = st.buf.try_borrow_mut() else {
            return;
        };
        let Some(&idx) = b.by_handle.get(id as usize) else {
            return;
        };
        if idx != 0 {
            b.by_handle[id as usize] = 0;
            if idx < DEFERRED {
                drop(b.remove(st, idx));
            }
        }
    });
}

/// The index mark of a root waiting for the due run.
const DEFERRED: u32 = u32::MAX - 1;

/// An array at `addr` is being freed (see [`object_freed`]).
#[inline]
pub fn array_freed(addr: usize) {
    if ANY.load(Ordering::Relaxed) == 0 {
        return;
    }
    let _ = STATE.try_with(|st| {
        if st.arrays.get() == 0 {
            return;
        }
        let Ok(mut b) = st.buf.try_borrow_mut() else {
            return;
        };
        if let Some(idx) = b.by_addr.remove(&addr) {
            if idx < DEFERRED {
                drop(b.remove(st, idx));
            }
        }
    });
}

/// Handle `id`'s object is on the VM's destructor list (its destructor
/// has not run; the list holds one reference php does not count).
pub const TRACKED: u8 = 1;
/// Handle `id`'s object is kept by the VM after a fatal error (one
/// reference php does not count; its destructor never runs).
pub const HELD: u8 = 2;
/// The cycle collector called handle `id`'s destructor; the object is
/// still on the VM's destructor list until the collection ends.
pub const CALLED: u8 = 4;

/// Set or clear `flag` ([`TRACKED`], [`HELD`]) on handle `id`.
pub fn set_flag(id: u32, flag: u8, on: bool) {
    let _ = STATE.try_with(|st| {
        let Ok(mut f) = st.flags.try_borrow_mut() else {
            return;
        };
        let i = id as usize;
        if i >= f.len() {
            if !on {
                return;
            }
            let n = i + 1 - f.len();
            reserve(&mut f, n);
            f.resize(i + 1, 0);
        }
        if on {
            if !st.flagged.get() {
                counted(st, || st.flagged.set(true));
            }
            f[i] |= flag;
        } else {
            f[i] &= !flag;
        }
    });
}

/// The flags of handle `id`.
#[inline]
pub fn flags(id: u32) -> u8 {
    with(|st| match st.flags.try_borrow() {
        Ok(f) => f.get(id as usize).copied().unwrap_or(0),
        Err(_) => 0,
    })
}

/// The strong references to handle `id`'s object that php does not count.
#[inline]
pub fn extra_refs(id: u32) -> usize {
    usize::from(flags(id) & (TRACKED | HELD) != 0)
}

// --- the collector's view of the buffer -------------------------------------

/// The buffer's slots `FIRST_ROOT..first_unused`, as they are.
pub fn slots() -> Vec<Slot> {
    with(|st| match st.buf.try_borrow() {
        Ok(b) => {
            let end = (b.first_unused as usize).min(b.slots.len());
            b.slots
                .get(FIRST_ROOT as usize..end)
                .map(<[Slot]>::to_vec)
                .unwrap_or_default()
        }
        Err(_) => Vec::new(),
    })
}

/// php's `first_unused`.
pub fn first_unused() -> u32 {
    with(|st| {
        st.buf
            .try_borrow()
            .map(|b| b.first_unused)
            .unwrap_or(FIRST_ROOT)
    })
}

/// The slot at `idx`.
pub fn slot(idx: u32) -> Slot {
    with(|st| match st.buf.try_borrow() {
        Ok(b) => b.slots.get(idx as usize).cloned().unwrap_or(Slot::Unused),
        Err(_) => Slot::Unused,
    })
}

/// Retag slot `idx` (root, garbage, destructor garbage) keeping its node.
pub fn retag(idx: u32, f: impl FnOnce(Node) -> Slot) {
    with(|st| {
        if let Ok(mut b) = st.buf.try_borrow_mut() {
            if let Some(s) = b.slots.get_mut(idx as usize) {
                if let Some(n) = s.node().cloned() {
                    *s = f(n);
                }
            }
        }
    })
}

/// Take slot `idx` out of the buffer (gc_remove_from_roots and the index
/// reset). The returned node is dropped by the caller.
pub fn remove_slot(idx: u32) -> Option<Node> {
    with(|st| {
        let mut b = st.buf.try_borrow_mut().ok()?;
        let node = b.slots.get(idx as usize)?.node()?.clone();
        if let Some(key) = key_of(&node) {
            b.set_index(key, 0);
        } else if let Node::Arr(w) = &node {
            b.by_addr.remove(&(w.as_ptr() as usize));
        }
        b.remove(st, idx)
    })
}

/// The slot of a buffered `v` (0: not in the buffer).
pub fn slot_of(v: &Value) -> u32 {
    let node = match v {
        Value::Object(h) => Node::Obj(Rc::downgrade(h)),
        Value::Closure(c) => Node::Closure(Rc::downgrade(c)),
        Value::Array(a) => Node::Arr(Rc::downgrade(&a.0)),
        _ => return 0,
    };
    with(|st| st.buf.try_borrow().map(|b| b.slot_of(&node)).unwrap_or(0))
}

/// gc_add_garbage: put `v` in the buffer as garbage; its slot.
pub fn add_garbage(v: &Value) -> u32 {
    let (node, key) = match v {
        Value::Object(h) => {
            let Ok(o) = h.try_borrow() else { return 0 };
            (Node::Obj(Rc::downgrade(h)), Key::Handle(o.id))
        }
        Value::Closure(c) => (Node::Closure(Rc::downgrade(c)), Key::Handle(c.id)),
        Value::Array(a) => (
            Node::Arr(Rc::downgrade(&a.0)),
            Key::Addr(Rc::as_ptr(&a.0) as usize),
        ),
        _ => return 0,
    };
    with(|st| match st.buf.try_borrow_mut() {
        Ok(mut b) => b.insert(st, node, key, true),
        Err(_) => 0,
    })
}

/// gc_compact: php's two-finger compaction, moving the last entries into
/// the free slots (this decides the order of the next run).
pub fn compact() {
    with(|st| {
        let Ok(mut b) = st.buf.try_borrow_mut() else {
            return;
        };
        if b.num_roots + FIRST_ROOT != b.first_unused {
            if b.num_roots != 0 {
                let mut free = FIRST_ROOT as usize;
                let mut scan = b.first_unused as usize - 1;
                let end = b.num_roots as usize;
                let unused =
                    |b: &Buf, i: usize| matches!(b.slots.get(i), None | Some(Slot::Unused));
                while free < scan {
                    while !unused(&b, free) {
                        free += 1;
                    }
                    while unused(&b, scan) {
                        scan -= 1;
                    }
                    if scan > free {
                        let s = std::mem::replace(&mut b.slots[scan], Slot::Unused);
                        if let Some(key) = s.node().and_then(key_of) {
                            b.set_index(key, free as u32);
                        }
                        b.slots[free] = s;
                        free += 1;
                        scan -= 1;
                        if scan <= end {
                            break;
                        }
                    }
                }
            }
            b.unused.clear();
            b.first_unused = b.num_roots + FIRST_ROOT;
        }
    })
}

/// php's `num_roots`.
pub fn num_roots() -> u32 {
    with(|st| st.buf.try_borrow().map(|b| b.num_roots).unwrap_or(0))
}

// --- flags and statistics ---------------------------------------------------

/// gc_enable(): the previous setting.
pub fn enable(on: bool) -> bool {
    with(|st| st.enabled.replace(on))
}

pub fn enabled() -> bool {
    with(|st| st.enabled.get())
}

/// `GC_G(gc_active)`: a collection is running.
pub fn active() -> bool {
    with(|st| st.active.get())
}

pub fn set_active(on: bool) {
    with(|st| st.active.set(on))
}

/// Drop `v` without making roots of what it releases: a prelude
/// function's temporaries, which php's C code never has.
pub fn drop_quietly(v: Value) {
    let prev = with(|st| st.quiet.replace(true));
    drop(v);
    with(|st| st.quiet.set(prev));
}

pub fn protected() -> bool {
    with(|st| st.protected.get())
}

/// The statistics `gc_status()` reports.
pub struct Status {
    pub running: bool,
    pub protected: bool,
    pub full: bool,
    pub runs: u32,
    pub collected: u32,
    pub threshold: u32,
    pub buf_size: u32,
    pub num_roots: u32,
    pub collector_time: u64,
    pub dtor_time: u64,
    pub free_time: u64,
}

pub fn status() -> Option<Status> {
    STATE
        .try_with(|st| {
            let b = st.buf.try_borrow().ok()?;
            Some(Status {
                running: st.active.get(),
                protected: st.protected.get(),
                full: st.full.get(),
                runs: st.runs.get(),
                collected: st.collected.get(),
                threshold: b.threshold,
                buf_size: b.buf_size,
                num_roots: b.num_roots,
                collector_time: st.collector_time.get(),
                dtor_time: st.dtor_time.get(),
                free_time: st.free_time.get(),
            })
        })
        .ok()
        .flatten()
}

/// A collection run started (`gc_runs++`).
pub fn count_run() {
    with(|st| st.runs.set(st.runs.get() + 1))
}

/// A run collected `n` (`collected += n`).
pub fn count_collected(n: u32) {
    with(|st| st.collected.set(st.collected.get() + n))
}

/// Add to the collector / destructor / free time (nanoseconds).
pub fn add_times(collector: u64, dtor: u64, free: u64) {
    with(|st| {
        st.collector_time.set(st.collector_time.get() + collector);
        st.dtor_time.set(st.dtor_time.get() + dtor);
        st.free_time.set(st.free_time.get() + free);
    })
}

/// gc_reset and the start of a request: an empty buffer, php's defaults.
/// The slots' `Weak`s are dropped here, outside any request accounting.
pub fn reset() {
    let _ = STATE.try_with(|st| {
        st.enabled.set(true);
        st.active.set(false);
        st.protected.set(false);
        st.full.set(false);
        st.due.set(false);
        st.runs.set(0);
        st.collected.set(0);
        st.collector_time.set(0);
        st.dtor_time.set(0);
        st.free_time.set(0);
        counted(st, || {
            st.arrays.set(0);
            st.handles.set(0);
            st.flagged.set(false);
        });
        st.last.set(0);
        if let Ok(mut f) = st.flags.try_borrow_mut() {
            f.clear();
        }
        let old = st.buf.try_borrow_mut().ok().map(|mut b| {
            let slots = std::mem::take(&mut b.slots);
            for n in slots.iter().filter_map(Slot::node) {
                n.set_buffered(false);
            }
            let deferred = std::mem::take(&mut b.deferred);
            b.unused.clear();
            b.first_unused = FIRST_ROOT;
            b.num_roots = 0;
            b.threshold = THRESHOLD_DEFAULT;
            b.buf_size = DEFAULT_BUF_SIZE;
            b.by_handle.clear();
            b.by_addr.clear();
            (slots, deferred)
        });
        crate::mem::uncounted(|| drop(old));
    });
    crate::mem::state().gc_due(false);
}
