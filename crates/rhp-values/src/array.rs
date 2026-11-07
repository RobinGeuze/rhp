//! PHP arrays: [`PhpArray`] (a copy-on-write handle) over [`ArrayData`]
//! (php's HashTable). Every access goes through the methods here, so the
//! storage can change without touching the users.

use std::rc::Rc;

use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;

use crate::{gc, ArrayKey, Value};

/// The array buffer; insertion order preserved by IndexMap.
pub struct ArrayData {
    entries: IndexMap<ArrayKey, Value, FxBuildHasher>,
    /// php's nNextFreeElement: the key `$a[] = ...` uses. `i64::MIN` until an
    /// integer key is inserted (a fresh append then uses 0). Saturates at
    /// `i64::MAX`; an append fails while that key is occupied.
    next_free: i64,
    /// Internal pointer for `current`/`next`/`prev`/`end`/`reset`/`key`.
    /// `entries.len()` (or more) is past the end. A removal before it moves
    /// it back, so it keeps pointing at the same element (php's bucket
    /// position; a removed current element leaves it on the next one).
    pos: usize,
    /// Positions of the active HashTable iterators (array_walk), by id,
    /// updated on removal like `pos`. Copied along with the array, as
    /// zend_array_dup copies iterators. Boxed: one word in every array.
    #[allow(clippy::box_collection)]
    iters: Option<Box<Vec<(u32, usize)>>>,
}

/// php's `zend_array_dup`: a reference held only by the source array
/// (refcount 1) is not shared with the copy; the copy gets the referenced
/// value (`zend_array_dup_element`), unless that value is the source array
/// itself (the self-reference guard).
impl Clone for ArrayData {
    fn clone(&self) -> Self {
        let mut entries = self.entries.clone();
        let src = self as *const ArrayData as usize;
        for v in entries.values_mut() {
            let inner = match v {
                Value::Ref(rc) if Rc::strong_count(rc) == 2 => match rc.try_borrow() {
                    Ok(b) => match &*b {
                        Value::Array(a) if a.data_ptr() == src => None,
                        other => Some(other.clone()),
                    },
                    Err(_) => None,
                },
                _ => None,
            };
            if let Some(x) = inner {
                *v = x;
            }
        }
        ArrayData {
            entries,
            next_free: self.next_free,
            // zend_array_dup: a pointer past the end restarts at 0.
            pos: if self.pos < self.entries.len() {
                self.pos
            } else {
                0
            },
            iters: self.iters.clone(),
        }
    }
}

impl Drop for ArrayData {
    fn drop(&mut self) {
        gc::array_freed(self as *const ArrayData as usize);
        for v in self.entries.values() {
            gc::possible_root(v);
        }
    }
}

/// Iterator over (key, value) in order.
pub struct Iter<'a>(indexmap::map::Iter<'a, ArrayKey, Value>);

impl<'a> Iterator for Iter<'a> {
    type Item = (ArrayKey, &'a Value);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, v)| (k.clone(), v))
    }
    #[inline]
    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.0.nth(n).map(|(k, v)| (k.clone(), v))
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl DoubleEndedIterator for Iter<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(k, v)| (k.clone(), v))
    }
}

impl ExactSizeIterator for Iter<'_> {}

/// Iterator over (key, mutable value) in order.
pub struct IterMut<'a>(indexmap::map::IterMut<'a, ArrayKey, Value>);

impl<'a> Iterator for IterMut<'a> {
    type Item = (ArrayKey, &'a mut Value);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, v)| (k.clone(), v))
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl DoubleEndedIterator for IterMut<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(k, v)| (k.clone(), v))
    }
}

impl ExactSizeIterator for IterMut<'_> {}

impl ArrayData {
    fn new() -> Self {
        ArrayData {
            entries: IndexMap::with_hasher(FxBuildHasher),
            next_free: i64::MIN,
            pos: 0,
            iters: None,
        }
    }

    /// Number of elements (php's nNumOfElements).
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// One past the last position (php's nNumUsed): positions run from 0
    /// to this, some of them holes.
    #[inline]
    pub fn used(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub fn get(&self, k: &ArrayKey) -> Option<&Value> {
        self.entries.get(k)
    }

    #[inline]
    pub fn get_mut(&mut self, k: &ArrayKey) -> Option<&mut Value> {
        self.entries.get_mut(k)
    }

    #[inline]
    pub fn contains_key(&self, k: &ArrayKey) -> bool {
        self.entries.contains_key(k)
    }

    #[inline]
    pub fn iter(&self) -> Iter<'_> {
        Iter(self.entries.iter())
    }

    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_> {
        IterMut(self.entries.iter_mut())
    }

    #[inline]
    pub fn keys(&self) -> impl DoubleEndedIterator<Item = ArrayKey> + ExactSizeIterator + '_ {
        self.entries.keys().cloned()
    }

    #[inline]
    pub fn values(&self) -> impl DoubleEndedIterator<Item = &Value> + ExactSizeIterator + '_ {
        self.entries.values()
    }

    #[inline]
    pub fn values_mut(
        &mut self,
    ) -> impl DoubleEndedIterator<Item = &mut Value> + ExactSizeIterator + '_ {
        self.entries.values_mut()
    }

    /// The `i`th element in order (counting elements, not positions).
    pub fn nth(&self, i: usize) -> Option<(ArrayKey, &Value)> {
        self.entries.get_index(i).map(|(k, v)| (k.clone(), v))
    }

    /// The `i`th element in order, mutable.
    pub fn nth_mut(&mut self, i: usize) -> Option<(ArrayKey, &mut Value)> {
        self.entries.get_index_mut(i).map(|(k, v)| (k.clone(), v))
    }

    /// The element at position `pos`, `None` for a hole or past the end.
    #[inline]
    pub fn at(&self, pos: usize) -> Option<(ArrayKey, &Value)> {
        self.entries.get_index(pos).map(|(k, v)| (k.clone(), v))
    }

    /// The element at position `pos`, mutable.
    #[inline]
    pub fn at_mut(&mut self, pos: usize) -> Option<(ArrayKey, &mut Value)> {
        self.entries.get_index_mut(pos).map(|(k, v)| (k.clone(), v))
    }

    /// The first element at position `pos` or later, with its position
    /// (php's _zend_hash_get_valid_pos and a fetch).
    #[inline]
    pub fn fetch_from(&self, pos: usize) -> Option<(usize, ArrayKey, &Value)> {
        self.entries
            .get_index(pos)
            .map(|(k, v)| (pos, k.clone(), v))
    }

    /// The first live position at `pos` or later (`used()` if none).
    #[inline]
    pub fn valid_pos(&self, pos: usize) -> usize {
        pos.min(self.entries.len())
    }

    /// Insert or overwrite; returns the old value.
    pub fn insert(&mut self, key: ArrayKey, value: Value) -> Option<Value> {
        if let ArrayKey::Int(n) = key {
            self.bump_next_int(n);
        }
        let old = self.entries.insert(key, value);
        if let Some(o) = &old {
            gc::possible_root(o);
        }
        old
    }

    /// The slot at `key`, inserting `Null` if absent.
    pub fn get_or_insert_mut(&mut self, key: ArrayKey) -> &mut Value {
        if let ArrayKey::Int(n) = key {
            self.bump_next_int(n);
        }
        self.entries.entry(key).or_insert(Value::Null)
    }

    /// Append `$a[] = value`; false (nothing inserted) when the next
    /// element is occupied.
    #[inline]
    pub fn try_push(&mut self, value: Value) -> bool {
        let Some(k) = self.next_append_key() else {
            return false;
        };
        self.bump_next_int(k);
        self.entries.insert(ArrayKey::Int(k), value);
        true
    }

    /// A freshly appended `Null` slot (`$a[]`); `None` when the next
    /// element is occupied.
    pub fn append_mut(&mut self) -> Option<&mut Value> {
        let k = self.next_append_key()?;
        self.bump_next_int(k);
        Some(self.entries.entry(ArrayKey::Int(k)).or_insert(Value::Null))
    }

    /// Remove `key`, keeping the order of the rest. `next_free` is not
    /// lowered.
    pub fn remove(&mut self, key: &ArrayKey) -> Option<Value> {
        let (i, _, v) = self.entries.shift_remove_full(key)?;
        self.removed_at(i);
        gc::possible_root(&v);
        Some(v)
    }

    /// Remove every element (positions, pointer and next free key reset as
    /// zend_hash_clean leaves them... the caller sets what it needs).
    pub fn clear(&mut self) {
        let old = std::mem::take(&mut self.entries);
        self.pos = 0;
        for v in old.values() {
            gc::possible_root(v);
        }
    }

    /// Keep only the elements `f` accepts, in order.
    pub fn retain(&mut self, mut f: impl FnMut(&ArrayKey, &mut Value) -> bool) {
        let mut i = 0;
        while i < self.entries.len() {
            let keep = {
                let (k, v) = self.entries.get_index_mut(i).expect("in range");
                f(k, v)
            };
            if keep {
                i += 1;
            } else {
                self.remove_at(i);
            }
        }
    }

    /// Take all elements out, leaving the array empty (pointer, iterators
    /// and next free key untouched).
    pub fn take_entries(&mut self) -> Vec<(ArrayKey, Value)> {
        std::mem::take(&mut self.entries).into_iter().collect()
    }

    /// Update `next_free` after inserting integer key `k` (append or explicit).
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
        if k == i64::MAX && self.entries.contains_key(&ArrayKey::Int(k)) {
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

    /// Remove the element at position `pos`, keeping the internal pointer
    /// and the iterators on the same elements.
    pub fn remove_at(&mut self, pos: usize) -> Option<(ArrayKey, Value)> {
        let r = self.entries.shift_remove_index(pos)?;
        self.removed_at(pos);
        gc::possible_root(&r.1);
        Some(r)
    }

    #[inline]
    fn removed_at(&mut self, i: usize) {
        if self.pos > i && self.pos != usize::MAX {
            self.pos -= 1;
        }
        if let Some(it) = &mut self.iters {
            for (_, p) in it.iter_mut() {
                if *p > i {
                    *p -= 1;
                }
            }
        }
    }

    // --- the internal pointer -------------------------------------------

    /// The internal pointer's raw position (nInternalPointer).
    #[inline]
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Set the internal pointer's raw position.
    #[inline]
    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }

    /// The internal pointer's element (zend_hash_get_current_pos).
    pub fn current(&self) -> Option<(ArrayKey, &Value)> {
        self.fetch_from(self.pos).map(|(_, k, v)| (k, v))
    }

    /// `reset()`: the pointer to the first element.
    pub fn ptr_reset(&mut self) {
        self.pos = 0;
    }

    /// `end()`: the pointer to the last element.
    pub fn ptr_end(&mut self) {
        self.pos = self.entries.len().saturating_sub(1);
    }

    /// `next()`: the pointer one element on; stays past the end once there.
    pub fn ptr_next(&mut self) {
        // php stops at the end (nNumUsed), so a later append is current.
        self.pos = self.pos.saturating_add(1).min(self.entries.len());
    }

    /// `prev()`: the pointer one element back; past the end before the
    /// first.
    pub fn ptr_prev(&mut self) {
        if self.pos == 0 || self.pos >= self.entries.len() {
            self.pos = self.entries.len();
        } else {
            self.pos -= 1;
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
    /// array does not carry it), moving the iterator past it: FE_FETCH_RW
    /// and array_walk. `None` at the end.
    pub fn iter_step(&mut self, id: u32) -> Option<(ArrayKey, &mut Value)> {
        let len = self.entries.len();
        let pos = self.iter_pos(id).unwrap_or(self.pos.min(len));
        if pos >= len {
            self.iter_add(id, len);
            return None;
        }
        self.iter_add(id, pos + 1);
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
        d.entries.reserve_exact(n);
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
        self.0.pos
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

    /// Whether php would hold this array packed (HT_IS_PACKED).
    pub fn is_packed(&self) -> bool {
        let mut size: i64 = 8;
        let mut used: i64 = 0;
        for (i, (k, _)) in self.iter().enumerate() {
            let ArrayKey::Int(k) = k else {
                return false;
            };
            if k < used {
                return false;
            }
            while k >= size {
                if (i as i64) <= size / 2 || k >= size * 2 {
                    return false;
                }
                size *= 2;
            }
            used = k + 1;
        }
        true
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
    pub fn values(&self) -> impl DoubleEndedIterator<Item = &Value> + ExactSizeIterator + '_ {
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
