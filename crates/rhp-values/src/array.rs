//! PHP arrays: [`PhpArray`] (a copy-on-write handle) over [`ArrayData`]
//! (php's HashTable, Zend/zend_hash.c). Every access goes through the
//! methods here.
//!
//! Two layouts, chosen the way zend_hash chooses them:
//! - packed: values where the position is the integer key (16 bytes per
//!   element, plus 8 for the block as php's HT_PACKED_SIZE). Holes left by
//!   removals are `Value::Undef`, as php's IS_UNDEF slots.
//! - hash: 32-byte buckets in insertion order followed by an index of
//!   `2 * size` u32 slots, one block of 40 bytes an element as php's
//!   HT_SIZE (linear probing; php chains through the same number of slots).
//!   A removed bucket stays as a hole until a rehash compacts the table, so
//!   positions stay put meanwhile.
//!
//! The table is one raw block, so `ArrayData` is 40 bytes and its `Rc`
//! box 56, as php's zend_array.
//!
//! Positions (php's HashPosition: the internal pointer, foreach and the
//! HashTable iterators) are slot numbers in either layout, holes included.
//! The table size grows by doubling from 8, as nTableSize does, so the
//! allocations match php's.

use std::alloc::{alloc, dealloc, handle_alloc_error, realloc, Layout};
use std::hash::Hasher;
use std::ptr::{self, NonNull};
use std::rc::Rc;

use rustc_hash::FxHasher;

use crate::{gc, ArrayKey, PhpStr, Value};

/// HT_MIN_SIZE.
const MIN_SIZE: u32 = 8;
/// HT_MAX_SIZE on 64-bit hosts.
const MAX_SIZE: u32 = 0x4000_0000;
/// An empty index slot.
const EMPTY: u32 = u32::MAX;
/// The bit of `ArrayData::size` that marks the hash layout.
const HASH: u32 = 1 << 31;

/// A hash-layout element: the value, the integer key or the string key's
/// hash, and the string key (php's Bucket). `val` is `Undef` for a hole.
struct Bucket {
    val: Value,
    h: u64,
    key: Option<PhpStr>,
}

impl Bucket {
    #[inline]
    fn array_key(&self) -> ArrayKey {
        match &self.key {
            Some(s) => ArrayKey::Str(s.clone()),
            None => ArrayKey::Int(self.h as i64),
        }
    }
}

// The block layouts below assume these sizes (and alignment 8).
const _: () = assert!(
    std::mem::size_of::<Bucket>() == 32
        && std::mem::size_of::<Value>() == 16
        && std::mem::align_of::<Bucket>() <= 8
        && std::mem::align_of::<Value>() <= 8
);

/// A read view of the table.
enum View<'a> {
    /// Values by position (`len()` is nNumUsed).
    Packed(&'a [Value]),
    /// Buckets (`len()` is nNumUsed) and the index (`2 * size` slots).
    Hash(&'a [Bucket], &'a [u32]),
}

/// A write view of the table.
enum ViewMut<'a> {
    Packed(&'a mut [Value]),
    Hash(&'a mut [Bucket], &'a mut [u32]),
}

/// The block of a packed table of `size` elements (HT_PACKED_SIZE_EX).
#[inline]
fn packed_layout(size: u32) -> Layout {
    Layout::from_size_align(16 * size as usize + 8, 8).expect("table size")
}

/// The block of a hash table of `size` elements (HT_SIZE_EX).
#[inline]
fn hash_layout(size: u32) -> Layout {
    Layout::from_size_align(40 * size as usize, 8).expect("table size")
}

/// The pointer of an unallocated table: dangling, aligned for values.
#[inline]
fn dangling() -> NonNull<u8> {
    NonNull::<u64>::dangling().cast()
}

/// Allocate `layout`.
#[inline]
fn alloc_block(layout: Layout) -> NonNull<u8> {
    // SAFETY: the layouts are never zero-sized (size >= 8).
    match NonNull::new(unsafe { alloc(layout) }) {
        Some(p) => p,
        None => handle_alloc_error(layout),
    }
}

/// The hash of a string key.
#[inline]
fn str_hash(b: &[u8]) -> u64 {
    let mut h = FxHasher::default();
    h.write(b);
    h.finish()
}

/// The home slot of hash `h` in an index with `mask + 1` slots.
#[inline]
fn home(h: u64, mask: usize) -> usize {
    (h.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as usize & mask
}

/// zend_hash_check_size: the power of two (at least 8) that holds `n`.
fn check_size(n: usize) -> u32 {
    if n <= MIN_SIZE as usize {
        MIN_SIZE
    } else {
        (n.min(MAX_SIZE as usize) as u32).next_power_of_two()
    }
}

/// A key to look up, its string hash computed once.
enum Probe<'a> {
    Int(i64),
    Str(&'a [u8], u64),
}

impl<'a> Probe<'a> {
    #[inline]
    fn of(k: &'a ArrayKey) -> Self {
        match k {
            ArrayKey::Int(n) => Probe::Int(*n),
            ArrayKey::Str(s) => Probe::Str(s.as_bytes(), str_hash(s.as_bytes())),
        }
    }

    #[inline]
    fn h(&self) -> u64 {
        match *self {
            Probe::Int(n) => n as u64,
            Probe::Str(_, h) => h,
        }
    }

    #[inline]
    fn matches(&self, b: &Bucket) -> bool {
        match *self {
            Probe::Int(n) => b.key.is_none() && b.h == n as u64,
            Probe::Str(s, h) => b.h == h && b.key.as_ref().is_some_and(|k| k.as_bytes() == s),
        }
    }
}

/// The bucket for `p`, if present.
#[inline]
fn find_bucket(buckets: &[Bucket], index: &[u32], p: &Probe) -> Option<usize> {
    let mask = index.len() - 1;
    let mut i = home(p.h(), mask);
    loop {
        let e = index[i];
        if e == EMPTY {
            return None;
        }
        if p.matches(&buckets[e as usize]) {
            return Some(e as usize);
        }
        i = (i + 1) & mask;
    }
}

/// Add bucket `idx` with hash `h` to the index.
#[inline]
fn index_add(index: &mut [u32], h: u64, idx: usize) {
    let mask = index.len() - 1;
    let mut i = home(h, mask);
    while index[i] != EMPTY {
        i = (i + 1) & mask;
    }
    index[i] = idx as u32;
}

/// Take bucket `idx` out of the index (backward-shift deletion, so no
/// tombstones are left).
fn index_del(buckets: &[Bucket], index: &mut [u32], idx: usize) {
    let mask = index.len() - 1;
    let mut i = home(buckets[idx].h, mask);
    while index[i] != idx as u32 {
        i = (i + 1) & mask;
    }
    let mut hole = i;
    let mut j = i;
    loop {
        j = (j + 1) & mask;
        let e = index[j];
        if e == EMPTY {
            break;
        }
        let k = home(buckets[e as usize].h, mask);
        // Move `e` into the hole unless its home lies cyclically in
        // (hole, j].
        let stays = if hole <= j {
            hole < k && k <= j
        } else {
            hole < k || k <= j
        };
        if !stays {
            index[hole] = e;
            hole = j;
        }
    }
    index[hole] = EMPTY;
}

/// The array buffer: php's HashTable.
pub struct ArrayData {
    /// The table block (packed or hash layout, see `HASH`), or dangling
    /// while none is allocated (HASH_FLAG_UNINITIALIZED).
    data: NonNull<u8>,
    /// nNumUsed.
    used: u32,
    /// nTableSize (a power of two), with the `HASH` bit for the hash
    /// layout.
    size: u32,
    /// nNumOfElements.
    count: u32,
    /// nInternalPointer, a position (`current`/`next`/`prev`/`end`/
    /// `reset`/`key`, and where foreach by reference starts).
    pos: u32,
    /// php's nNextFreeElement: the key `$a[] = ...` uses. `i64::MIN` until an
    /// integer key is inserted (a fresh append then uses 0). Saturates at
    /// `i64::MAX`; an append fails while that key is occupied.
    next_free: i64,
    /// Positions of the active HashTable iterators (array_walk, foreach by
    /// reference), by id. Copied along with the array, as zend_array_dup
    /// copies iterators. Boxed: one word in every array.
    #[allow(clippy::box_collection)]
    iters: Option<Box<Vec<(u32, usize)>>>,
}

/// php's `zend_array_dup`: a reference held only by the source array
/// (refcount 1) is not shared with the copy; the copy gets the referenced
/// value (`zend_array_dup_element`), unless that value is the source array
/// itself (the self-reference guard). A packed copy keeps its holes; a hash
/// copy is compacted, moving the pointer and iterators along.
impl Clone for ArrayData {
    fn clone(&self) -> Self {
        let src = self as *const ArrayData as usize;
        let dup = |v: &Value| -> Value {
            match v {
                Value::Undef => Value::Undef,
                Value::Ref(rc) if Rc::strong_count(rc) == 1 => match rc.try_borrow() {
                    Ok(b) => match &*b {
                        Value::Array(a) if a.data_ptr() == src => v.clone(),
                        other => other.clone(),
                    },
                    Err(_) => v.clone(),
                },
                _ => v.clone(),
            }
        };
        let used = self.used();
        // zend_array_dup: a pointer past the end restarts at 0.
        let mut pos = if (self.pos as usize) < used {
            self.pos
        } else {
            0
        };
        let mut iters = self.iters.clone();
        let mut out = ArrayData::new();
        out.next_free = self.next_free;
        out.iters = None;
        if self.count == 0 {
            out.iters = iters;
            return out;
        }
        out.size = self.size;
        out.count = self.count;
        match self.view() {
            View::Packed(v) => {
                out.data = alloc_block(packed_layout(self.size()));
                let dst = out.data.as_ptr() as *mut Value;
                for (i, x) in v.iter().enumerate() {
                    // SAFETY: i < used <= size slots; `out.used` counts what
                    // is written, so a panic leaves it consistent.
                    unsafe { ptr::write(dst.add(i), dup(x)) };
                    out.used += 1;
                }
            }
            View::Hash(b, _) => {
                out.data = alloc_block(hash_layout(self.size()));
                out.index_mut().fill(EMPTY);
                let holes = self.count as usize != used;
                // One past the previous live position: iterators on the
                // holes before an element move with it, the pointer only
                // when on it (zend_array_dup_elements).
                let mut from = 0usize;
                for (idx, p) in b.iter().enumerate() {
                    if matches!(p.val, Value::Undef) {
                        continue;
                    }
                    let to = out.used as usize;
                    if holes && to != idx {
                        if self.pos as usize == idx {
                            pos = to as u32;
                        }
                        if let Some(it) = &mut iters {
                            for e in it.iter_mut() {
                                if e.1 >= from && e.1 <= idx {
                                    e.1 = to;
                                }
                            }
                        }
                    }
                    from = idx + 1;
                    let bucket = Bucket {
                        val: dup(&p.val),
                        h: p.h,
                        key: p.key.clone(),
                    };
                    out.push_bucket(bucket);
                }
            }
        }
        out.pos = pos;
        out.iters = iters;
        out
    }
}

impl Drop for ArrayData {
    fn drop(&mut self) {
        gc::array_freed(self as *const ArrayData as usize);
        for v in self.values() {
            gc::possible_root(v);
        }
        self.free_table();
    }
}

enum IterInner<'a> {
    Packed(std::iter::Enumerate<std::slice::Iter<'a, Value>>),
    Hash(std::slice::Iter<'a, Bucket>),
}

/// Iterator over (key, value) in order.
pub struct Iter<'a> {
    inner: IterInner<'a>,
    left: usize,
}

impl<'a> Iterator for Iter<'a> {
    type Item = (ArrayKey, &'a Value);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        let r = match &mut self.inner {
            IterInner::Packed(it) => it
                .find(|(_, v)| !matches!(v, Value::Undef))
                .map(|(i, v)| (ArrayKey::Int(i as i64), v)),
            IterInner::Hash(it) => it
                .find(|b| !matches!(b.val, Value::Undef))
                .map(|b| (b.array_key(), &b.val)),
        };
        self.left -= 1;
        r
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        let dense = match &self.inner {
            IterInner::Packed(it) => it.len() == self.left,
            IterInner::Hash(it) => it.len() == self.left,
        };
        if !dense {
            for _ in 0..n {
                self.next()?;
            }
            return self.next();
        }
        if n >= self.left {
            self.left = 0;
            return None;
        }
        self.left -= n + 1;
        match &mut self.inner {
            IterInner::Packed(it) => it.nth(n).map(|(i, v)| (ArrayKey::Int(i as i64), v)),
            IterInner::Hash(it) => it.nth(n).map(|b| (b.array_key(), &b.val)),
        }
    }
}

impl DoubleEndedIterator for Iter<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        let r = match &mut self.inner {
            IterInner::Packed(it) => it
                .rfind(|(_, v)| !matches!(v, Value::Undef))
                .map(|(i, v)| (ArrayKey::Int(i as i64), v)),
            IterInner::Hash(it) => it
                .rfind(|b| !matches!(b.val, Value::Undef))
                .map(|b| (b.array_key(), &b.val)),
        };
        self.left -= 1;
        r
    }
}

impl ExactSizeIterator for Iter<'_> {}

/// Iterator over the values in order.
pub struct Values<'a> {
    inner: ValuesInner<'a>,
    left: usize,
}

enum ValuesInner<'a> {
    Packed(std::slice::Iter<'a, Value>),
    Hash(std::slice::Iter<'a, Bucket>),
}

impl<'a> Iterator for Values<'a> {
    type Item = &'a Value;
    #[inline]
    fn next(&mut self) -> Option<&'a Value> {
        if self.left == 0 {
            return None;
        }
        let r = match &mut self.inner {
            ValuesInner::Packed(it) => it.find(|v| !matches!(v, Value::Undef)),
            ValuesInner::Hash(it) => it.find(|b| !matches!(b.val, Value::Undef)).map(|b| &b.val),
        };
        self.left -= 1;
        r
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}

impl DoubleEndedIterator for Values<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        let r = match &mut self.inner {
            ValuesInner::Packed(it) => it.rfind(|v| !matches!(v, Value::Undef)),
            ValuesInner::Hash(it) => it.rfind(|b| !matches!(b.val, Value::Undef)).map(|b| &b.val),
        };
        self.left -= 1;
        r
    }
}

impl ExactSizeIterator for Values<'_> {}

enum IterMutInner<'a> {
    Packed(std::iter::Enumerate<std::slice::IterMut<'a, Value>>),
    Hash(std::slice::IterMut<'a, Bucket>),
}

/// Iterator over (key, mutable value) in order.
pub struct IterMut<'a> {
    inner: IterMutInner<'a>,
    left: usize,
}

impl<'a> Iterator for IterMut<'a> {
    type Item = (ArrayKey, &'a mut Value);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        let r = match &mut self.inner {
            IterMutInner::Packed(it) => it
                .find(|(_, v)| !matches!(v, Value::Undef))
                .map(|(i, v)| (ArrayKey::Int(i as i64), v)),
            IterMutInner::Hash(it) => it
                .find(|b| !matches!(b.val, Value::Undef))
                .map(|b| (b.array_key(), &mut b.val)),
        };
        self.left -= 1;
        r
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}

impl DoubleEndedIterator for IterMut<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        let r = match &mut self.inner {
            IterMutInner::Packed(it) => it
                .rfind(|(_, v)| !matches!(v, Value::Undef))
                .map(|(i, v)| (ArrayKey::Int(i as i64), v)),
            IterMutInner::Hash(it) => it
                .rfind(|b| !matches!(b.val, Value::Undef))
                .map(|b| (b.array_key(), &mut b.val)),
        };
        self.left -= 1;
        r
    }
}

impl ExactSizeIterator for IterMut<'_> {}

impl ArrayData {
    fn new() -> Self {
        ArrayData {
            data: dangling(),
            used: 0,
            size: MIN_SIZE,
            count: 0,
            pos: 0,
            next_free: i64::MIN,
            iters: None,
        }
    }

    // --- the raw table ----------------------------------------------------

    /// nTableSize.
    #[inline]
    fn size(&self) -> u32 {
        self.size & !HASH
    }

    /// Whether the table has the hash layout.
    #[inline]
    fn is_hash(&self) -> bool {
        self.size & HASH != 0
    }

    /// Whether a block is allocated.
    #[inline]
    fn allocated(&self) -> bool {
        self.data != dangling()
    }

    #[inline]
    fn view(&self) -> View<'_> {
        let used = self.used as usize;
        let p = self.data.as_ptr();
        if self.is_hash() {
            let size = self.size() as usize;
            // SAFETY: a hash block holds `size` buckets (the first `used`
            // initialized) and then `2 * size` index slots.
            unsafe {
                View::Hash(
                    std::slice::from_raw_parts(p as *const Bucket, used),
                    std::slice::from_raw_parts(p.add(32 * size) as *const u32, 2 * size),
                )
            }
        } else {
            // SAFETY: a packed block (or a dangling pointer with `used` 0)
            // holds `used` initialized values.
            unsafe { View::Packed(std::slice::from_raw_parts(p as *const Value, used)) }
        }
    }

    #[inline]
    fn view_mut(&mut self) -> ViewMut<'_> {
        let used = self.used as usize;
        let p = self.data.as_ptr();
        if self.is_hash() {
            let size = self.size() as usize;
            // SAFETY: as in `view`; the buckets and the index do not overlap.
            unsafe {
                ViewMut::Hash(
                    std::slice::from_raw_parts_mut(p as *mut Bucket, used),
                    std::slice::from_raw_parts_mut(p.add(32 * size) as *mut u32, 2 * size),
                )
            }
        } else {
            // SAFETY: as in `view`.
            unsafe { ViewMut::Packed(std::slice::from_raw_parts_mut(p as *mut Value, used)) }
        }
    }

    /// The index of a hash table.
    #[inline]
    fn index_mut(&mut self) -> &mut [u32] {
        match self.view_mut() {
            ViewMut::Hash(_, index) => index,
            ViewMut::Packed(_) => &mut [],
        }
    }

    /// Append a value to a packed table with room (used < size).
    #[inline]
    fn push_val(&mut self, v: Value) {
        debug_assert!(!self.is_hash() && self.allocated() && self.used < self.size());
        // SAFETY: slot `used` is inside the block and not initialized.
        unsafe {
            ptr::write(
                (self.data.as_ptr() as *mut Value).add(self.used as usize),
                v,
            )
        };
        self.used += 1;
    }

    /// Append a bucket to a hash table with room, indexing it.
    #[inline]
    fn push_bucket(&mut self, b: Bucket) {
        debug_assert!(self.is_hash() && self.used < self.size());
        let idx = self.used as usize;
        let h = b.h;
        // SAFETY: bucket `used` is inside the block and not initialized.
        unsafe { ptr::write((self.data.as_ptr() as *mut Bucket).add(idx), b) };
        self.used += 1;
        index_add(self.index_mut(), h, idx);
    }

    /// Drop the elements and free the block (the table becomes
    /// unallocated and packed; the size stays).
    fn free_table(&mut self) {
        if !self.allocated() {
            return;
        }
        let layout = if self.is_hash() {
            hash_layout(self.size())
        } else {
            packed_layout(self.size())
        };
        // Drop what is there; `used` is 0 first, so a panicking drop
        // cannot drop twice.
        let used = std::mem::replace(&mut self.used, 0) as usize;
        let p = self.data.as_ptr();
        // SAFETY: the first `used` slots are initialized, the block was
        // allocated with `layout`.
        unsafe {
            if self.size & HASH != 0 {
                ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(p as *mut Bucket, used));
            } else {
                ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(p as *mut Value, used));
            }
            dealloc(p, layout);
        }
        self.data = dangling();
        self.size = self.size();
    }

    /// Start an empty hash table of the current size
    /// (zend_hash_real_init_mixed).
    fn init_hash(&mut self) {
        debug_assert!(!self.allocated());
        self.data = alloc_block(hash_layout(self.size()));
        self.size |= HASH;
        self.index_mut().fill(EMPTY);
    }

    /// Number of elements (php's nNumOfElements).
    #[inline]
    pub fn len(&self) -> usize {
        self.count as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// One past the last position (php's nNumUsed): positions run from 0
    /// to this, some of them holes.
    #[inline]
    pub fn used(&self) -> usize {
        self.used as usize
    }

    /// Whether the table is packed (HT_IS_PACKED; an empty, never filled
    /// table counts as packed).
    #[inline]
    pub fn is_packed(&self) -> bool {
        !self.is_hash()
    }

    /// nTableSize.
    pub fn table_size(&self) -> usize {
        self.size() as usize
    }

    #[inline]
    pub fn get(&self, k: &ArrayKey) -> Option<&Value> {
        match self.view() {
            View::Packed(v) => match *k {
                ArrayKey::Int(n) => match v.get(usize::try_from(n).ok()?) {
                    Some(Value::Undef) | None => None,
                    Some(x) => Some(x),
                },
                ArrayKey::Str(_) => None,
            },
            View::Hash(b, index) => {
                let i = find_bucket(b, index, &Probe::of(k))?;
                Some(&b[i].val)
            }
        }
    }

    #[inline]
    pub fn get_mut(&mut self, k: &ArrayKey) -> Option<&mut Value> {
        match self.view_mut() {
            ViewMut::Packed(v) => match *k {
                ArrayKey::Int(n) => match v.get_mut(usize::try_from(n).ok()?) {
                    Some(Value::Undef) | None => None,
                    Some(x) => Some(x),
                },
                ArrayKey::Str(_) => None,
            },
            ViewMut::Hash(b, index) => {
                let i = find_bucket(b, index, &Probe::of(k))?;
                Some(&mut b[i].val)
            }
        }
    }

    #[inline]
    pub fn contains_key(&self, k: &ArrayKey) -> bool {
        self.get(k).is_some()
    }

    /// The position holding `k`.
    fn find_pos(&self, k: &ArrayKey) -> Option<usize> {
        match self.view() {
            View::Packed(v) => match *k {
                ArrayKey::Int(n) => {
                    let i = usize::try_from(n).ok()?;
                    (!matches!(v.get(i)?, Value::Undef)).then_some(i)
                }
                ArrayKey::Str(_) => None,
            },
            View::Hash(b, index) => find_bucket(b, index, &Probe::of(k)),
        }
    }

    #[inline]
    pub fn iter(&self) -> Iter<'_> {
        let inner = match self.view() {
            View::Packed(v) => IterInner::Packed(v.iter().enumerate()),
            View::Hash(b, _) => IterInner::Hash(b.iter()),
        };
        Iter {
            inner,
            left: self.count as usize,
        }
    }

    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_> {
        let left = self.count as usize;
        let inner = match self.view_mut() {
            ViewMut::Packed(v) => IterMutInner::Packed(v.iter_mut().enumerate()),
            ViewMut::Hash(b, _) => IterMutInner::Hash(b.iter_mut()),
        };
        IterMut { inner, left }
    }

    #[inline]
    pub fn keys(&self) -> impl DoubleEndedIterator<Item = ArrayKey> + ExactSizeIterator + '_ {
        self.iter().map(|(k, _)| k)
    }

    #[inline]
    pub fn values(&self) -> Values<'_> {
        let inner = match self.view() {
            View::Packed(v) => ValuesInner::Packed(v.iter()),
            View::Hash(b, _) => ValuesInner::Hash(b.iter()),
        };
        Values {
            inner,
            left: self.count as usize,
        }
    }

    #[inline]
    pub fn values_mut(
        &mut self,
    ) -> impl DoubleEndedIterator<Item = &mut Value> + ExactSizeIterator + '_ {
        self.iter_mut().map(|(_, v)| v)
    }

    /// The `i`th element in order (counting elements, not positions).
    pub fn nth(&self, i: usize) -> Option<(ArrayKey, &Value)> {
        if self.count as usize == self.used() {
            return self.at(i);
        }
        self.iter().nth(i)
    }

    /// The `i`th element in order, mutable.
    pub fn nth_mut(&mut self, i: usize) -> Option<(ArrayKey, &mut Value)> {
        if self.count as usize == self.used() {
            return self.at_mut(i);
        }
        self.iter_mut().nth(i)
    }

    /// The element at position `pos`, `None` for a hole or past the end.
    #[inline]
    pub fn at(&self, pos: usize) -> Option<(ArrayKey, &Value)> {
        match self.view() {
            View::Packed(v) => match v.get(pos)? {
                Value::Undef => None,
                x => Some((ArrayKey::Int(pos as i64), x)),
            },
            View::Hash(b, _) => {
                let b = b.get(pos)?;
                match b.val {
                    Value::Undef => None,
                    _ => Some((b.array_key(), &b.val)),
                }
            }
        }
    }

    /// The element at position `pos`, mutable.
    #[inline]
    pub fn at_mut(&mut self, pos: usize) -> Option<(ArrayKey, &mut Value)> {
        match self.view_mut() {
            ViewMut::Packed(v) => match v.get_mut(pos)? {
                Value::Undef => None,
                x => Some((ArrayKey::Int(pos as i64), x)),
            },
            ViewMut::Hash(b, _) => {
                let b = b.get_mut(pos)?;
                match b.val {
                    Value::Undef => None,
                    _ => Some((b.array_key(), &mut b.val)),
                }
            }
        }
    }

    /// Whether position `pos` holds an element.
    #[inline]
    fn live(&self, pos: usize) -> bool {
        match self.view() {
            View::Packed(v) => !matches!(v.get(pos), Some(Value::Undef) | None),
            View::Hash(b, _) => !matches!(b.get(pos).map(|b| &b.val), Some(Value::Undef) | None),
        }
    }

    /// The first element at position `pos` or later, with its position
    /// (_zend_hash_get_valid_pos and a fetch).
    #[inline]
    pub fn fetch_from(&self, pos: usize) -> Option<(usize, ArrayKey, &Value)> {
        let mut p = pos;
        match self.view() {
            View::Packed(v) => loop {
                match v.get(p)? {
                    Value::Undef => p += 1,
                    x => return Some((p, ArrayKey::Int(p as i64), x)),
                }
            },
            View::Hash(b, _) => loop {
                let x = b.get(p)?;
                if !matches!(x.val, Value::Undef) {
                    return Some((p, x.array_key(), &x.val));
                }
                p += 1;
            },
        }
    }

    /// The first live position at `pos` or later (`used()` if none):
    /// _zend_hash_get_valid_pos.
    #[inline]
    pub fn valid_pos(&self, pos: usize) -> usize {
        match self.fetch_from(pos) {
            Some((p, _, _)) => p,
            None => self.used(),
        }
    }

    /// The position of the last element.
    fn last_pos(&self) -> Option<usize> {
        (0..self.used()).rev().find(|&p| self.live(p))
    }

    // --- insertion --------------------------------------------------------

    /// The slot for `key`, added (as `Null`) when absent: php's
    /// _zend_hash_add_or_update_i / _zend_hash_index_add_or_update_i with
    /// HASH_LOOKUP. The bool tells whether it was added.
    fn lookup(&mut self, key: ArrayKey) -> (&mut Value, bool) {
        match key {
            ArrayKey::Int(h) => self.index_lookup(h),
            ArrayKey::Str(s) => self.str_lookup(s),
        }
    }

    fn index_lookup(&mut self, h: i64) -> (&mut Value, bool) {
        let hu = h as u64;
        let size = self.size() as u64;
        if !self.is_hash() {
            let used = self.used as u64;
            if !self.allocated() {
                if hu < size {
                    // zend_hash_real_init_packed.
                    self.data = alloc_block(packed_layout(self.size()));
                    return self.packed_add(h as usize);
                }
                self.init_hash();
            } else if hu < used {
                let live = matches!(self.view(), View::Packed(v) if !matches!(v[h as usize], Value::Undef));
                if live {
                    let ViewMut::Packed(v) = self.view_mut() else {
                        unreachable!()
                    };
                    return (&mut v[h as usize], false);
                }
                // "we have to keep the order :("
                self.packed_to_hash(self.size());
            } else if hu < size {
                return self.packed_add(h as usize);
            } else if (hu >> 1) < size && (self.size() >> 1) < self.count {
                // zend_hash_packed_grow.
                assert!(
                    self.size() < MAX_SIZE,
                    "Possible integer overflow in memory allocation"
                );
                let old = packed_layout(self.size());
                let new = packed_layout(self.size() * 2);
                // SAFETY: the block was allocated with `old`; values move
                // bitwise.
                let p = unsafe { realloc(self.data.as_ptr(), old, new.size()) };
                self.data = NonNull::new(p).unwrap_or_else(|| handle_alloc_error(new));
                self.size *= 2;
                return self.packed_add(h as usize);
            } else {
                let size = self.size();
                if self.used >= size {
                    assert!(
                        size < MAX_SIZE,
                        "Possible integer overflow in memory allocation"
                    );
                    self.packed_to_hash(size * 2);
                } else {
                    self.packed_to_hash(size);
                }
            }
        } else {
            let found = match self.view() {
                View::Hash(b, index) => find_bucket(b, index, &Probe::Int(h)),
                View::Packed(_) => None,
            };
            if let Some(i) = found {
                let ViewMut::Hash(b, _) = self.view_mut() else {
                    unreachable!()
                };
                return (&mut b[i].val, false);
            }
            self.grow_if_full();
        }
        self.bump_next_int(h);
        let v = self.hash_append(hu, None);
        (v, true)
    }

    /// add_to_packed: element `h` (at or after nNumUsed) of a packed table
    /// with room for it.
    #[inline]
    fn packed_add(&mut self, h: usize) -> (&mut Value, bool) {
        while (self.used as usize) < h {
            self.push_val(Value::Undef);
        }
        self.push_val(Value::Null);
        self.next_free = h as i64 + 1;
        self.count += 1;
        match self.view_mut() {
            ViewMut::Packed(v) => (&mut v[h], true),
            ViewMut::Hash(..) => unreachable!(),
        }
    }

    fn str_lookup(&mut self, s: PhpStr) -> (&mut Value, bool) {
        let h = str_hash(s.as_bytes());
        if !self.is_hash() {
            if self.allocated() {
                self.packed_to_hash(self.size());
                self.grow_if_full();
            } else {
                self.init_hash();
            }
        } else {
            let found = match self.view() {
                View::Hash(b, index) => find_bucket(b, index, &Probe::Str(s.as_bytes(), h)),
                View::Packed(_) => None,
            };
            if let Some(i) = found {
                let ViewMut::Hash(b, _) = self.view_mut() else {
                    unreachable!()
                };
                return (&mut b[i].val, false);
            }
            self.grow_if_full();
        }
        let v = self.hash_append(h, Some(s));
        (v, true)
    }

    /// Append a `Null` bucket to a hash table with room (nNumUsed <
    /// nTableSize).
    #[inline]
    fn hash_append(&mut self, h: u64, key: Option<PhpStr>) -> &mut Value {
        let idx = self.used as usize;
        self.push_bucket(Bucket {
            val: Value::Null,
            h,
            key,
        });
        self.count += 1;
        match self.view_mut() {
            ViewMut::Hash(b, _) => &mut b[idx].val,
            ViewMut::Packed(_) => unreachable!(),
        }
    }

    /// ZEND_HASH_IF_FULL_DO_RESIZE / zend_hash_do_resize.
    #[inline]
    fn grow_if_full(&mut self) {
        if self.used >= self.size() {
            self.do_resize();
        }
    }

    #[cold]
    fn do_resize(&mut self) {
        let count = self.count as usize;
        if self.used() > count + (count >> 5) {
            self.rehash();
        } else if self.size() < MAX_SIZE {
            // Double: a new block, the buckets copied over, the index
            // rebuilt.
            let old_size = self.size();
            let new = alloc_block(hash_layout(old_size * 2));
            // SAFETY: both blocks hold at least `used` buckets; the old
            // block's buckets move bitwise and it is freed without
            // dropping them.
            unsafe {
                ptr::copy_nonoverlapping(
                    self.data.as_ptr() as *const Bucket,
                    new.as_ptr() as *mut Bucket,
                    self.used as usize,
                );
                dealloc(self.data.as_ptr(), hash_layout(old_size));
            }
            self.data = new;
            self.size = (old_size * 2) | HASH;
            self.rehash();
        } else {
            panic!("Possible integer overflow in memory allocation");
        }
    }

    /// zend_hash_packed_to_hash, into a table of `size` elements.
    #[cold]
    fn packed_to_hash(&mut self, size: u32) {
        let old = self.data;
        let old_layout = packed_layout(self.size());
        let used = self.used as usize;
        self.data = alloc_block(hash_layout(size));
        self.size = size | HASH;
        self.used = 0;
        for i in 0..used {
            // SAFETY: the old block's first `used` values are initialized;
            // each is moved out once, and the block is freed without
            // dropping them.
            let val = unsafe { ptr::read((old.as_ptr() as *const Value).add(i)) };
            // SAFETY: bucket i is inside the new block.
            unsafe {
                ptr::write(
                    (self.data.as_ptr() as *mut Bucket).add(i),
                    Bucket {
                        val,
                        h: i as u64,
                        key: None,
                    },
                )
            };
            self.used += 1;
        }
        // SAFETY: allocated with `old_layout`.
        unsafe { dealloc(old.as_ptr(), old_layout) };
        self.rehash();
    }

    /// zend_hash_rehash: rebuild the index, compacting holes; the pointer
    /// and the iterators move with their elements.
    fn rehash(&mut self) {
        if !self.is_hash() {
            return;
        }
        self.index_mut().fill(EMPTY);
        if self.count == 0 {
            // Only holes: nothing to drop.
            self.used = 0;
            self.pos = 0;
            if let Some(it) = &mut self.iters {
                for e in it.iter_mut() {
                    e.1 = 0;
                }
            }
            return;
        }
        let old_used = self.used();
        let count = self.count as usize;
        let mut pos = self.pos as usize;
        let mut iters = self.iters.take();
        let ViewMut::Hash(b, index) = self.view_mut() else {
            unreachable!()
        };
        let mut j = 0usize;
        if count == old_used {
            for (i, p) in b.iter().enumerate() {
                index_add(index, p.h, i);
            }
            j = old_used;
        } else {
            for i in 0..old_used {
                if matches!(b[i].val, Value::Undef) {
                    continue;
                }
                if i != j {
                    b.swap(i, j);
                    if pos > j && pos <= i {
                        pos = j;
                    }
                    if let Some(it) = &mut iters {
                        // zend_hash_iterators_update at each moved position
                        // an iterator sits on (or a hole before it).
                        for e in it.iter_mut() {
                            if e.1 > j && e.1 <= i {
                                e.1 = j;
                            }
                        }
                    }
                }
                index_add(index, b[j].h, j);
                j += 1;
            }
            if let Some(it) = &mut iters {
                for e in it.iter_mut() {
                    if e.1 == old_used {
                        e.1 = j;
                    }
                }
            }
        }
        // The moved-out tail is holes: nothing to drop.
        self.used = j as u32;
        self.pos = pos as u32;
        self.iters = iters;
    }

    /// Insert or overwrite; returns the old value.
    pub fn insert(&mut self, key: ArrayKey, value: Value) -> Option<Value> {
        let value = match value {
            Value::Undef => Value::Null,
            v => v,
        };
        let (slot, added) = self.lookup(key);
        let old = std::mem::replace(slot, value);
        if added {
            return None;
        }
        gc::possible_root(&old);
        Some(old)
    }

    /// The slot at `key`, inserting `Null` if absent.
    #[inline]
    pub fn get_or_insert_mut(&mut self, key: ArrayKey) -> &mut Value {
        self.lookup(key).0
    }

    /// Append `$a[] = value`; false (nothing inserted) when the next
    /// element is occupied.
    #[inline]
    pub fn try_push(&mut self, value: Value) -> bool {
        match self.append_mut() {
            Some(slot) => {
                *slot = match value {
                    Value::Undef => Value::Null,
                    v => v,
                };
                true
            }
            None => false,
        }
    }

    /// A freshly appended `Null` slot (`$a[]`); `None` when the next
    /// element is occupied.
    #[inline]
    pub fn append_mut(&mut self) -> Option<&mut Value> {
        let k = self.next_append_key()?;
        // The fast path: a packed table with room at its end.
        if !self.is_hash() && self.allocated() && self.used as i64 == k && self.used < self.size() {
            self.next_free = k + 1;
            self.count += 1;
            self.push_val(Value::Null);
            let ViewMut::Packed(v) = self.view_mut() else {
                unreachable!()
            };
            return v.last_mut();
        }
        Some(self.index_lookup(k).0)
    }

    // --- removal ----------------------------------------------------------

    /// Remove `key`, keeping the order of the rest. `next_free` is not
    /// lowered.
    pub fn remove(&mut self, key: &ArrayKey) -> Option<Value> {
        let pos = self.find_pos(key)?;
        let (_, v) = self.remove_at(pos)?;
        gc::possible_root(&v);
        Some(v)
    }

    /// Remove the element at position `pos` (_zend_hash_del_el /
    /// zend_hash_packed_del_val): it becomes a hole; at the end the used
    /// part shrinks past trailing holes and the pointer and iterators are
    /// clamped to it. The value is the caller's (no gc root check).
    pub fn remove_at(&mut self, pos: usize) -> Option<(ArrayKey, Value)> {
        let (k, v, used) = match self.view_mut() {
            ViewMut::Packed(v) => {
                let slot = v.get_mut(pos)?;
                if matches!(slot, Value::Undef) {
                    return None;
                }
                let val = std::mem::replace(slot, Value::Undef);
                let mut used = v.len();
                if pos + 1 == used {
                    while used > 0 && matches!(v[used - 1], Value::Undef) {
                        used -= 1;
                    }
                }
                (ArrayKey::Int(pos as i64), val, used)
            }
            ViewMut::Hash(b, index) => {
                if matches!(b.get(pos)?.val, Value::Undef) {
                    return None;
                }
                index_del(b, index, pos);
                let p = &mut b[pos];
                let val = std::mem::replace(&mut p.val, Value::Undef);
                let k = match p.key.take() {
                    Some(s) => ArrayKey::Str(s),
                    None => ArrayKey::Int(p.h as i64),
                };
                let mut used = b.len();
                if pos + 1 == used {
                    while used > 0 && matches!(b[used - 1].val, Value::Undef) {
                        used -= 1;
                    }
                }
                (k, val, used)
            }
        };
        // Trailing holes hold nothing to drop.
        self.used = used as u32;
        self.count -= 1;
        let used = self.used();
        if pos >= used {
            self.pos = self.pos.min(used as u32);
            if let Some(it) = &mut self.iters {
                for e in it.iter_mut() {
                    e.1 = e.1.min(used);
                }
            }
        }
        Some((k, v))
    }

    /// Take all elements out, leaving the array empty and unallocated
    /// (pointer, iterators and next free key untouched).
    pub fn take_entries(&mut self) -> Vec<(ArrayKey, Value)> {
        let mut out = Vec::with_capacity(self.count as usize);
        if self.allocated() {
            let used = std::mem::replace(&mut self.used, 0) as usize;
            let p = self.data.as_ptr();
            for i in 0..used {
                // SAFETY: the first `used` slots are initialized and each is
                // moved out once; `used` is 0 so nothing drops them again.
                unsafe {
                    if self.is_hash() {
                        let b = ptr::read((p as *const Bucket).add(i));
                        if matches!(b.val, Value::Undef) {
                            continue;
                        }
                        let k = match b.key {
                            Some(s) => ArrayKey::Str(s),
                            None => ArrayKey::Int(b.h as i64),
                        };
                        out.push((k, b.val));
                    } else {
                        let v = ptr::read((p as *const Value).add(i));
                        if !matches!(v, Value::Undef) {
                            out.push((ArrayKey::Int(i as i64), v));
                        }
                    }
                }
            }
        }
        self.free_table();
        self.count = 0;
        out
    }

    /// Take the contents of `from` (table, size, count, next free key),
    /// keeping this array's iterators; the pointer is reset (array_unshift
    /// replacing its HashTable data).
    pub fn replace_contents(&mut self, mut from: ArrayData) {
        std::mem::swap(&mut self.data, &mut from.data);
        std::mem::swap(&mut self.used, &mut from.used);
        std::mem::swap(&mut self.size, &mut from.size);
        std::mem::swap(&mut self.count, &mut from.count);
        self.next_free = from.next_free;
        self.ptr_reset();
    }

    /// array_pop: remove the last element. nNextFreeElement steps back when
    /// it was the last integer key; the pointer is reset.
    pub fn pop(&mut self) -> Option<Value> {
        let p = self.last_pos()?;
        let (k, v) = self.remove_at(p)?;
        if let ArrayKey::Int(h) = k {
            if self.next_free.checked_sub(1) == Some(h) {
                self.next_free = h;
            }
        }
        self.ptr_reset();
        Some(v)
    }

    /// array_shift: remove the first element and renumber the integer
    /// keys; the pointer is reset.
    pub fn shift(&mut self) -> Option<Value> {
        let (p, _, _) = self.fetch_from(0)?;
        let (_, v) = self.remove_at(p)?;
        let mut iters = self.iters.take();
        match self.view_mut() {
            ViewMut::Packed(vals) => {
                let mut k = 0usize;
                for idx in 0..vals.len() {
                    if matches!(vals[idx], Value::Undef) {
                        continue;
                    }
                    if idx != k {
                        vals.swap(idx, k);
                        if let Some(it) = &mut iters {
                            for e in it.iter_mut() {
                                if e.1 == idx {
                                    e.1 = k;
                                }
                            }
                        }
                    }
                    k += 1;
                }
                // The tail is holes now.
                self.used = k as u32;
                self.next_free = k as i64;
                self.iters = iters;
            }
            ViewMut::Hash(b, _) => {
                let mut k = 0u64;
                let mut renumbered = false;
                for p in b.iter_mut() {
                    if matches!(p.val, Value::Undef) || p.key.is_some() {
                        continue;
                    }
                    if p.h != k {
                        p.h = k;
                        renumbered = true;
                    }
                    k += 1;
                }
                self.next_free = k as i64;
                self.iters = iters;
                if renumbered {
                    self.rehash();
                }
            }
        }
        self.ptr_reset();
        Some(v)
    }

    /// Update `next_free` after inserting integer key `k` into a hash.
    #[inline]
    fn bump_next_int(&mut self, k: i64) {
        if k >= self.next_free {
            self.next_free = if k < i64::MAX { k + 1 } else { i64::MAX };
        }
    }

    /// The key the next append uses, or `None` when it is occupied
    /// (zend_hash_next_index_insert failing).
    #[inline]
    pub fn next_append_key(&self) -> Option<i64> {
        let k = if self.next_free == i64::MIN {
            0
        } else {
            self.next_free
        };
        if k == i64::MAX && self.contains_key(&ArrayKey::Int(k)) {
            return None;
        }
        Some(k)
    }

    /// php's nNextFreeElement as array functions see it (`i64::MIN` when no
    /// integer key was ever inserted).
    pub fn next_free(&self) -> i64 {
        self.next_free
    }

    /// Set nNextFreeElement (array_pop / array_shift / renumbering).
    pub fn set_next_free(&mut self, n: i64) {
        self.next_free = n;
    }

    // --- the internal pointer ---------------------------------------------

    /// The internal pointer's raw position (nInternalPointer).
    #[inline]
    pub fn pos(&self) -> usize {
        self.pos as usize
    }

    /// Set the internal pointer's raw position.
    #[inline]
    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos as u32;
    }

    /// The internal pointer's element (zend_hash_get_current_pos).
    pub fn current(&self) -> Option<(ArrayKey, &Value)> {
        self.fetch_from(self.pos as usize).map(|(_, k, v)| (k, v))
    }

    /// `reset()`: zend_hash_internal_pointer_reset.
    pub fn ptr_reset(&mut self) {
        self.pos = self.valid_pos(0) as u32;
    }

    /// `end()`: zend_hash_internal_pointer_end.
    pub fn ptr_end(&mut self) {
        self.pos = self.last_pos().unwrap_or(self.used()) as u32;
    }

    /// `next()`: zend_hash_move_forward; stays past the end once there.
    pub fn ptr_next(&mut self) {
        let idx = self.valid_pos(self.pos as usize);
        if idx < self.used() {
            self.pos = self.valid_pos(idx + 1) as u32;
        }
    }

    /// `prev()`: zend_hash_move_backwards; past the end before the first.
    pub fn ptr_prev(&mut self) {
        let idx = self.pos as usize;
        if idx < self.used() {
            self.pos = (0..idx)
                .rev()
                .find(|&p| self.live(p))
                .unwrap_or(self.used()) as u32;
        }
    }

    // --- HashTable iterators ----------------------------------------------

    /// Register iterator `id` at position `pos` (zend_hash_iterator_add).
    pub fn iter_add(&mut self, id: u32, pos: usize) {
        let it = self.iters.get_or_insert_with(Default::default);
        match it.iter_mut().find(|(i, _)| *i == id) {
            Some(e) => e.1 = pos,
            None => it.push((id, pos)),
        }
    }

    /// The position of iterator `id`, if this array (or the array it was
    /// copied from) has it.
    pub fn iter_pos(&self, id: u32) -> Option<usize> {
        self.iters
            .as_ref()?
            .iter()
            .find(|(i, _)| *i == id)
            .map(|e| e.1)
    }

    /// Whether HashTable iterators are registered.
    pub fn has_iters(&self) -> bool {
        self.iters.is_some()
    }

    /// The registered HashTable iterators: (id, position).
    pub fn iters(&self) -> Vec<(u32, usize)> {
        self.iters.as_deref().cloned().unwrap_or_default()
    }

    /// Move every iterator `n` places on (zend_hash_iterators_advance).
    pub fn iters_advance(&mut self, n: usize) {
        if let Some(it) = &mut self.iters {
            for (_, p) in it.iter_mut() {
                *p += n;
            }
        }
    }

    /// Hand this array's iterators to `to`, positions unchanged (a table
    /// rebuilt in place, as zend_hash_sort and php_splice do).
    pub fn move_iters_to(&mut self, to: &mut ArrayData) {
        to.iters = self.iters.take();
    }

    /// Drop iterator `id` (zend_hash_iterator_del).
    pub fn iter_del(&mut self, id: u32) {
        if let Some(it) = &mut self.iters {
            it.retain(|(i, _)| *i != id);
            if it.is_empty() {
                self.iters = None;
            }
        }
    }

    /// The element at iterator `id` (from the internal pointer when the
    /// array does not carry it), moving the iterator to the position after
    /// it: FE_FETCH_RW. `None` at the end.
    pub fn iter_step(&mut self, id: u32) -> Option<(ArrayKey, &mut Value)> {
        let start = match self.iter_pos(id) {
            Some(p) => p,
            None => self.valid_pos(self.pos as usize),
        };
        let Some((pos, _, _)) = self.fetch_from(start) else {
            let used = self.used();
            self.iter_add(id, used);
            return None;
        };
        self.iter_add(id, pos + 1);
        self.at_mut(pos)
    }

    /// As [`ArrayData::iter_step`], but the iterator moves on to the next
    /// element (zend_hash_move_forward_ex), not the next position: array_walk.
    pub fn walk_step(&mut self, id: u32) -> Option<(ArrayKey, &mut Value)> {
        let start = match self.iter_pos(id) {
            Some(p) => p,
            None => self.valid_pos(self.pos as usize),
        };
        let Some((pos, _, _)) = self.fetch_from(start) else {
            let used = self.used();
            self.iter_add(id, used);
            return None;
        };
        let next = self.valid_pos(pos + 1);
        self.iter_add(id, next);
        self.at_mut(pos)
    }
}

/// A PHP array value. Clone is O(1) (Rc bump); mutation is COW via `make_mut`.
#[derive(Clone)]
pub struct PhpArray(pub(crate) Rc<ArrayData>);

impl PhpArray {
    pub fn new() -> Self {
        PhpArray(Rc::new(ArrayData::new()))
    }

    /// An empty array with room for `n` elements (array_init_size).
    pub fn with_capacity(n: usize) -> Self {
        let mut d = ArrayData::new();
        d.size = check_size(n);
        PhpArray(Rc::new(d))
    }

    /// An empty hash table with room for `n` elements (array_init_size and
    /// zend_hash_real_init_mixed).
    pub fn with_capacity_mixed(n: usize) -> Self {
        let mut d = ArrayData::new();
        d.size = check_size(n);
        d.init_hash();
        PhpArray(Rc::new(d))
    }

    /// Identity of the shared array storage (php's `zend_array *`): equal for
    /// copies that have not been written since, so a recursive array reached
    /// through a reference is recognized (`*RECURSION*` / circular-reference
    /// guards in the dumpers and encoders).
    pub fn data_ptr(&self) -> usize {
        Rc::as_ptr(&self.0) as usize
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether HashTable iterators are registered on this array.
    pub fn has_iters(&self) -> bool {
        self.0.iters.is_some()
    }

    /// The registered HashTable iterators: (id, position).
    pub fn iters(&self) -> Vec<(u32, usize)> {
        self.0.iters()
    }

    /// php's nNextFreeElement (see [`ArrayData::next_free`]).
    pub fn next_free(&self) -> i64 {
        self.0.next_free
    }

    /// The internal pointer (see [`ArrayData::pos`]), read without
    /// separating.
    pub fn pos(&self) -> usize {
        self.0.pos()
    }

    /// The internal pointer's element.
    pub fn current(&self) -> Option<(ArrayKey, &Value)> {
        self.0.current()
    }

    /// The shared storage (the cycle collector's graph walk).
    pub fn rc(&self) -> &Rc<ArrayData> {
        &self.0
    }

    /// The storage, read only.
    #[inline]
    pub fn data(&self) -> &ArrayData {
        &self.0
    }

    /// An array value for storage `rc`.
    pub fn from_rc(rc: Rc<ArrayData>) -> Self {
        PhpArray(rc)
    }

    /// How many values share this storage (debug_zval_dump's refcount).
    pub fn ref_count(&self) -> usize {
        Rc::strong_count(&self.0)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether the array is packed (HT_IS_PACKED).
    pub fn is_packed(&self) -> bool {
        self.0.is_packed()
    }

    #[inline]
    pub fn get(&self, k: &ArrayKey) -> Option<&Value> {
        self.0.get(k)
    }

    #[inline]
    pub fn contains_key(&self, k: &ArrayKey) -> bool {
        self.0.contains_key(k)
    }

    #[inline]
    pub fn iter(&self) -> Iter<'_> {
        self.0.iter()
    }

    #[inline]
    pub fn keys(&self) -> impl DoubleEndedIterator<Item = ArrayKey> + ExactSizeIterator + '_ {
        self.0.keys()
    }

    #[inline]
    pub fn values(&self) -> Values<'_> {
        self.0.values()
    }

    /// The `i`th element in order.
    #[inline]
    pub fn nth(&self, i: usize) -> Option<(ArrayKey, &Value)> {
        self.0.nth(i)
    }

    /// The first element.
    #[inline]
    pub fn first(&self) -> Option<(ArrayKey, &Value)> {
        self.0.iter().next()
    }

    /// The last element.
    #[inline]
    pub fn last(&self) -> Option<(ArrayKey, &Value)> {
        self.0.iter().next_back()
    }

    /// See [`ArrayData::fetch_from`] (foreach by value).
    #[inline]
    pub fn fetch_from(&self, pos: usize) -> Option<(usize, ArrayKey, &Value)> {
        self.0.fetch_from(pos)
    }

    /// The storage, taken out (copied when shared).
    pub fn into_data(self) -> ArrayData {
        Rc::try_unwrap(self.0).unwrap_or_else(|rc| (*rc).clone())
    }

    /// Make a packed table a hash (zend_hash_packed_to_hash), as php's
    /// sorts that keep the keys leave it.
    pub fn to_hash(&mut self) {
        let d = self.make_mut();
        if !d.is_hash() && d.allocated() {
            let size = d.size();
            d.packed_to_hash(size);
        }
    }

    /// COW mutable access to the backing buffer.
    #[inline]
    pub fn make_mut(&mut self) -> &mut ArrayData {
        Rc::make_mut(&mut self.0)
    }

    /// Insert/overwrite at an explicit key (COW). Updates the append counter.
    #[inline]
    pub fn insert(&mut self, key: ArrayKey, value: Value) {
        self.make_mut().insert(key, value);
    }

    /// Append `$a[] = value` (COW); nothing is inserted when the next
    /// element is already occupied (see [`PhpArray::try_push`]).
    #[inline]
    pub fn push(&mut self, value: Value) {
        self.try_push(value);
    }

    /// Append `$a[] = value` (COW). Returns false (and inserts nothing) when
    /// the next element is already occupied.
    #[inline]
    pub fn try_push(&mut self, value: Value) -> bool {
        self.make_mut().try_push(value)
    }

    /// COW mutable access to the slot at `key`, inserting `Null` if absent.
    /// Updates the append counter for integer keys. Used by nested lvalue writes.
    #[inline]
    pub fn get_or_insert_mut(&mut self, key: ArrayKey) -> &mut Value {
        self.make_mut().get_or_insert_mut(key)
    }

    /// COW mutable access to a freshly appended slot (`$a[]`); `None` when
    /// the next element is already occupied.
    #[inline]
    pub fn append_mut(&mut self) -> Option<&mut Value> {
        self.make_mut().append_mut()
    }

    /// COW mutable access to an existing slot (no insert).
    #[inline]
    pub fn get_mut(&mut self, key: &ArrayKey) -> Option<&mut Value> {
        self.make_mut().get_mut(key)
    }

    /// COW remove, preserving insertion order. `next_free` is not lowered.
    #[inline]
    pub fn remove(&mut self, key: &ArrayKey) -> Option<Value> {
        self.make_mut().remove(key)
    }
}

impl Default for PhpArray {
    fn default() -> Self {
        PhpArray::new()
    }
}

impl FromIterator<(ArrayKey, Value)> for PhpArray {
    fn from_iter<T: IntoIterator<Item = (ArrayKey, Value)>>(iter: T) -> Self {
        let mut a = PhpArray::new();
        for (k, v) in iter {
            a.insert(k, v);
        }
        a
    }
}

#[cfg(test)]
#[path = "array_tests.rs"]
mod tests;
