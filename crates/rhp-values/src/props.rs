//! An object's property table: declared properties in fixed slots, dynamic
//! ones in a map made on demand (php's `properties_table` plus the
//! `properties` HashTable, see [`PropTable`]).

use std::ptr::NonNull;
use std::rc::Rc;

use indexmap::{IndexMap, IndexSet};
use rustc_hash::FxBuildHasher;

use crate::{PhpStr, Value};

type Map = IndexMap<PhpStr, Value, FxBuildHasher>;

/// A class's declared property slots (php's `properties_info_table`): the
/// storage keys in slot order, parent slots first. Built when the class is
/// linked and shared by its objects.
pub struct PropLayout {
    keys: IndexSet<PhpStr, FxBuildHasher>,
}

impl PropLayout {
    pub fn new(keys: impl IntoIterator<Item = PhpStr>) -> Rc<Self> {
        Rc::new(PropLayout {
            keys: keys.into_iter().collect(),
        })
    }

    /// The slot of storage key `key`.
    #[inline]
    pub fn index_of(&self, key: &PhpStr) -> Option<usize> {
        self.keys.get_index_of(key)
    }

    /// The storage key of slot `i`.
    #[inline]
    pub fn key(&self, i: usize) -> &PhpStr {
        &self.keys[i]
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// An object's properties, keyed by storage key (the php-mangled
/// `"\0Class\0name"` for a private property, the plain name otherwise).
///
/// A declared property lives in its slot ([`PropLayout`]); an unset or
/// uninitialized one is [`Value::Undef`] there and counts as absent, and
/// setting it again fills the same slot. Other keys are dynamic properties,
/// kept in insertion order after the slots. That is the order php's
/// properties table has (rebuild_object_properties).
///
/// Positions (`get_index`, the iterators of a foreach over the object)
/// count every slot, empty ones included, then the dynamic properties.
#[derive(Default)]
pub struct PropTable {
    slots: Slots,
    dynamic: Option<Box<Map>>,
}

/// The declared slots: a `[Value]` the length of the layout, behind a thin
/// pointer (the layout knows the length), so that an object fits a 64-byte
/// allocation.
struct Slots {
    /// `None`: no declared properties. Never an empty layout.
    layout: Option<Rc<PropLayout>>,
    /// Owns `layout.len()` values (dangling when there is no layout).
    ptr: NonNull<Value>,
}

impl Slots {
    fn new(layout: &Rc<PropLayout>) -> Self {
        if layout.is_empty() {
            return Self::default();
        }
        let b: Box<[Value]> = (0..layout.len()).map(|_| Value::Undef).collect();
        Self::from_box(layout.clone(), b)
    }

    fn from_box(layout: Rc<PropLayout>, b: Box<[Value]>) -> Self {
        debug_assert_eq!(b.len(), layout.len());
        let ptr = NonNull::new(Box::into_raw(b) as *mut Value).expect("box pointer");
        Slots {
            layout: Some(layout),
            ptr,
        }
    }

    #[inline]
    fn layout(&self) -> Option<&PropLayout> {
        self.layout.as_deref()
    }

    /// The layout and the values, borrowed apart.
    #[inline]
    fn split_mut(&mut self) -> (Option<&PropLayout>, &mut [Value]) {
        let n = self.layout.as_deref().map_or(0, |l| l.len());
        // SAFETY: `ptr` owns `n` values (see the field), disjoint from
        // `layout`.
        let vals = unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), n) };
        (self.layout.as_deref(), vals)
    }
}

impl Default for Slots {
    fn default() -> Self {
        Slots {
            layout: None,
            ptr: NonNull::dangling(),
        }
    }
}

impl std::ops::Deref for Slots {
    type Target = [Value];
    #[inline]
    fn deref(&self) -> &[Value] {
        let n = self.layout.as_deref().map_or(0, |l| l.len());
        // SAFETY: `ptr` owns `n` values (see the field).
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), n) }
    }
}

impl std::ops::DerefMut for Slots {
    #[inline]
    fn deref_mut(&mut self) -> &mut [Value] {
        self.split_mut().1
    }
}

impl Drop for Slots {
    fn drop(&mut self) {
        let n = self.len();
        if n != 0 {
            // SAFETY: made by `from_box` from a box of `n` values.
            drop(unsafe {
                Box::from_raw(std::ptr::slice_from_raw_parts_mut(self.ptr.as_ptr(), n))
            });
        }
    }
}

/// Empty slots stay empty (`Value::clone` reads them as null).
impl Clone for Slots {
    fn clone(&self) -> Self {
        match &self.layout {
            None => Self::default(),
            Some(l) => Self::from_box(l.clone(), self.iter().map(Value::copy_slot).collect()),
        }
    }
}

impl PropTable {
    /// An empty table without declared slots.
    pub fn new() -> Self {
        Self::default()
    }

    /// A table with `layout`'s slots, all empty.
    pub fn with_layout(layout: &Rc<PropLayout>) -> Self {
        PropTable {
            slots: Slots::new(layout),
            dynamic: None,
        }
    }

    /// The slot of declared key `key`.
    #[inline]
    pub fn slot_of(&self, key: &PhpStr) -> Option<usize> {
        let i = self.slots.layout()?.index_of(key)?;
        (i < self.slots.len()).then_some(i)
    }

    /// Slot `i` (empty: [`Value::Undef`]).
    #[inline]
    pub fn slot(&self, i: usize) -> Option<&Value> {
        self.slots.get(i)
    }

    #[inline]
    pub fn slot_mut(&mut self, i: usize) -> Option<&mut Value> {
        self.slots.get_mut(i)
    }

    #[inline]
    pub fn get(&self, key: &PhpStr) -> Option<&Value> {
        match self.slot_of(key) {
            Some(i) => Some(&self.slots[i]).filter(|v| !v.is_undef()),
            None => self.dynamic.as_deref()?.get(key),
        }
    }

    /// [`PropTable::get`] for `key` when its slot `i` is known (the
    /// property's index in the class); a dynamic one by key.
    #[inline]
    pub fn get_at(&self, i: usize, key: &PhpStr) -> Option<&Value> {
        match self.slots.get(i) {
            Some(v) => (!v.is_undef()).then_some(v),
            None => self.get(key),
        }
    }

    #[inline]
    pub fn get_mut(&mut self, key: &PhpStr) -> Option<&mut Value> {
        match self.slot_of(key) {
            Some(i) => Some(&mut self.slots[i]).filter(|v| !v.is_undef()),
            None => self.dynamic.as_deref_mut()?.get_mut(key),
        }
    }

    #[inline]
    pub fn contains_key(&self, key: &PhpStr) -> bool {
        self.get(key).is_some()
    }

    /// Set `key`, returning the previous value. A declared key goes to its
    /// slot, a new dynamic one last.
    pub fn insert(&mut self, key: PhpStr, v: Value) -> Option<Value> {
        match self.slot_of(&key) {
            Some(i) => {
                let old = std::mem::replace(&mut self.slots[i], v);
                (!old.is_undef()).then_some(old)
            }
            None => self.dynamic.get_or_insert_default().insert(key, v),
        }
    }

    /// The value of `key`, set to null first when absent.
    pub fn get_or_insert_null(&mut self, key: PhpStr) -> &mut Value {
        match self.slot_of(&key) {
            Some(i) => {
                let s = &mut self.slots[i];
                if s.is_undef() {
                    *s = Value::Null;
                }
                s
            }
            None => self
                .dynamic
                .get_or_insert_default()
                .entry(key)
                .or_insert(Value::Null),
        }
    }

    /// [`PropTable::get_or_insert_null`] with the slot known (see
    /// [`PropTable::get_at`]).
    #[inline]
    pub fn get_or_insert_null_at(&mut self, i: usize, key: PhpStr) -> &mut Value {
        if i < self.slots.len() {
            let s = &mut self.slots[i];
            if s.is_undef() {
                *s = Value::Null;
            }
            return s;
        }
        self.get_or_insert_null(key)
    }

    /// The number of properties.
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|v| !v.is_undef()).count()
            + self.dynamic.as_deref().map_or(0, |d| d.len())
    }

    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(|v| v.is_undef())
            && self.dynamic.as_deref().is_none_or(|d| d.is_empty())
    }

    /// The properties in order.
    pub fn iter(&self) -> Iter<'_> {
        Iter {
            layout: self.slots.layout(),
            slots: self.slots.iter().enumerate(),
            dynamic: self.dynamic.as_deref().map(|d| d.iter()),
        }
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&PhpStr, &mut Value)> + '_ {
        let (layout, slots) = self.slots.split_mut();
        let slots = slots
            .iter_mut()
            .enumerate()
            .filter(|(_, v)| !v.is_undef())
            .map(move |(i, v)| (layout.expect("slots have a layout").key(i), v));
        slots.chain(
            self.dynamic
                .as_deref_mut()
                .into_iter()
                .flat_map(|d| d.iter_mut()),
        )
    }

    pub fn keys(&self) -> impl Iterator<Item = &PhpStr> + '_ {
        self.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> + '_ {
        self.iter().map(|(_, v)| v)
    }

    /// Whether there are dynamic properties.
    #[inline]
    pub fn has_dynamic(&self) -> bool {
        self.dynamic.as_deref().is_some_and(|d| !d.is_empty())
    }

    /// One past the last position.
    pub fn pos_len(&self) -> usize {
        self.slots.len() + self.dynamic.as_deref().map_or(0, |d| d.len())
    }

    /// The entry at position `pos`; an empty slot is [`Value::Undef`].
    pub fn get_index(&self, pos: usize) -> Option<(&PhpStr, &Value)> {
        let n = self.slots.len();
        if pos < n {
            let k = self.slots.layout()?.key(pos);
            return Some((k, &self.slots[pos]));
        }
        self.dynamic.as_deref()?.get_index(pos - n)
    }

    pub fn get_index_mut(&mut self, pos: usize) -> Option<(&PhpStr, &mut Value)> {
        let n = self.slots.len();
        if pos < n {
            let (layout, slots) = self.slots.split_mut();
            return Some((layout?.key(pos), &mut slots[pos]));
        }
        self.dynamic.as_deref_mut()?.get_index_mut(pos - n)
    }

    /// The position of property `key`.
    pub fn get_index_of(&self, key: &PhpStr) -> Option<usize> {
        match self.slot_of(key) {
            Some(i) => (!self.slots[i].is_undef()).then_some(i),
            None => Some(self.slots.len() + self.dynamic.as_deref()?.get_index_of(key)?),
        }
    }

    /// Remove `key`: its value, and for a dynamic property its position
    /// (the later ones move down one).
    pub(crate) fn remove(&mut self, key: &PhpStr) -> Option<(Option<usize>, Value)> {
        match self.slot_of(key) {
            Some(i) => {
                let old = std::mem::replace(&mut self.slots[i], Value::Undef);
                (!old.is_undef()).then_some((None, old))
            }
            None => {
                let d = self.dynamic.as_deref_mut()?;
                let (i, _, v) = d.shift_remove_full(key)?;
                Some((Some(self.slots.len() + i), v))
            }
        }
    }
}

impl Clone for PropTable {
    fn clone(&self) -> Self {
        PropTable {
            slots: self.slots.clone(),
            dynamic: self.dynamic.clone(),
        }
    }
}

impl std::ops::Index<usize> for PropTable {
    type Output = Value;
    fn index(&self, pos: usize) -> &Value {
        self.get_index(pos).expect("property position").1
    }
}

impl std::ops::IndexMut<usize> for PropTable {
    fn index_mut(&mut self, pos: usize) -> &mut Value {
        self.get_index_mut(pos).expect("property position").1
    }
}

/// Dynamic properties only (no layout).
impl FromIterator<(PhpStr, Value)> for PropTable {
    fn from_iter<I: IntoIterator<Item = (PhpStr, Value)>>(it: I) -> Self {
        let mut t = PropTable::new();
        for (k, v) in it {
            t.insert(k, v);
        }
        t
    }
}

/// [`PropTable::iter`].
pub struct Iter<'a> {
    layout: Option<&'a PropLayout>,
    slots: std::iter::Enumerate<std::slice::Iter<'a, Value>>,
    dynamic: Option<indexmap::map::Iter<'a, PhpStr, Value>>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a PhpStr, &'a Value);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for (i, v) in self.slots.by_ref() {
            if !v.is_undef() {
                return Some((self.layout?.key(i), v));
            }
        }
        self.dynamic.as_mut()?.next()
    }
}

impl<'a> IntoIterator for &'a PropTable {
    type Item = (&'a PhpStr, &'a Value);
    type IntoIter = Iter<'a>;
    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> PhpStr {
        PhpStr::from(x)
    }

    fn keys(t: &PropTable) -> Vec<String> {
        t.keys()
            .map(|k| String::from_utf8_lossy(k.as_bytes()).into_owned())
            .collect()
    }

    fn table() -> PropTable {
        let l = PropLayout::new([s("a"), s("b"), s("c")]);
        let mut t = PropTable::with_layout(&l);
        t.insert(s("a"), Value::Int(1));
        t.insert(s("c"), Value::Int(3));
        t
    }

    #[test]
    fn an_object_fits_64_bytes() {
        // Rc counts + RefCell flag + ObjectData.
        assert_eq!(std::mem::size_of::<PropTable>(), 24);
        assert_eq!(
            16 + std::mem::size_of::<std::cell::RefCell<crate::ObjectData>>(),
            64
        );
    }

    #[test]
    fn declared_slots_keep_their_place() {
        let mut t = table();
        assert_eq!(keys(&t), ["a", "c"]);
        t.insert(s("d"), Value::Int(4));
        t.insert(s("b"), Value::Int(2));
        assert_eq!(keys(&t), ["a", "b", "c", "d"]);
        assert_eq!(t.remove(&s("a")).map(|r| r.0), Some(None));
        assert!(!t.contains_key(&s("a")));
        *t.get_or_insert_null(s("a")) = Value::Int(9);
        assert_eq!(keys(&t), ["a", "b", "c", "d"]);
        assert_eq!(t.len(), 4);
        assert_eq!(t.pos_len(), 4);
    }

    #[test]
    fn dynamic_properties_follow_in_insertion_order() {
        let mut t = table();
        t.insert(s("x"), Value::Int(1));
        t.insert(s("y"), Value::Int(2));
        assert_eq!(t.remove(&s("x")).map(|r| r.0), Some(Some(3)));
        t.insert(s("x"), Value::Int(3));
        assert_eq!(keys(&t), ["a", "c", "y", "x"]);
        assert_eq!(t.get_index_of(&s("x")), Some(4));
        assert_eq!(t.get_index_of(&s("b")), None);
        assert!(t.get_index(1).is_some_and(|(_, v)| v.is_undef()));
    }

    #[test]
    fn clone_keeps_empty_slots_empty() {
        let t = table().clone();
        assert!(t.get(&s("b")).is_none());
        assert!(t.slot(1).is_some_and(Value::is_undef));
        assert_eq!(keys(&t), ["a", "c"]);
        assert!(PropTable::new().clone().is_empty());
    }
}
