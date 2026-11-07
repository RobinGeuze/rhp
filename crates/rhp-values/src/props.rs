//! An object's property table: declared properties in fixed slots, dynamic
//! ones in a map made on demand (php's `properties_table` plus the
//! `properties` HashTable, see [`PropTable`]).

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
    /// `None`: no declared properties.
    layout: Option<Rc<PropLayout>>,
    slots: Box<[Value]>,
    dynamic: Option<Box<Map>>,
}

impl PropTable {
    /// An empty table without declared slots.
    pub fn new() -> Self {
        Self::default()
    }

    /// A table with `layout`'s slots, all empty.
    pub fn with_layout(layout: &Rc<PropLayout>) -> Self {
        if layout.is_empty() {
            return Self::default();
        }
        PropTable {
            layout: Some(layout.clone()),
            slots: (0..layout.len()).map(|_| Value::Undef).collect(),
            dynamic: None,
        }
    }

    /// The slot of declared key `key`.
    #[inline]
    pub fn slot_of(&self, key: &PhpStr) -> Option<usize> {
        let i = self.layout.as_deref()?.index_of(key)?;
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
            layout: self.layout.as_deref(),
            slots: self.slots.iter().enumerate(),
            dynamic: self.dynamic.as_deref().map(|d| d.iter()),
        }
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&PhpStr, &mut Value)> + '_ {
        let layout = self.layout.as_deref();
        let slots = self
            .slots
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

    /// One past the last position.
    pub fn pos_len(&self) -> usize {
        self.slots.len() + self.dynamic.as_deref().map_or(0, |d| d.len())
    }

    /// The entry at position `pos`; an empty slot is [`Value::Undef`].
    pub fn get_index(&self, pos: usize) -> Option<(&PhpStr, &Value)> {
        let n = self.slots.len();
        if pos < n {
            let k = self.layout.as_deref()?.key(pos);
            return Some((k, &self.slots[pos]));
        }
        self.dynamic.as_deref()?.get_index(pos - n)
    }

    pub fn get_index_mut(&mut self, pos: usize) -> Option<(&PhpStr, &mut Value)> {
        let n = self.slots.len();
        if pos < n {
            let k = self.layout.as_deref()?.key(pos);
            return Some((k, &mut self.slots[pos]));
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

/// Empty slots stay empty (`Value::clone` reads them as null).
impl Clone for PropTable {
    fn clone(&self) -> Self {
        PropTable {
            layout: self.layout.clone(),
            slots: self.slots.iter().map(Value::copy_slot).collect(),
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
