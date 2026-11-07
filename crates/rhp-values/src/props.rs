//! An object's property table (see [`PropTable`]).

use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;

use crate::{PhpStr, Value};

type Map = IndexMap<PhpStr, Value, FxBuildHasher>;

/// An object's properties, keyed by storage key (the php-mangled
/// `"\0Class\0name"` for a private property, the plain name otherwise), in
/// php's order. Positions (`get_index`, the iterators of a foreach over the
/// object) count every entry.
#[derive(Clone, Default)]
pub struct PropTable {
    map: Map,
}

impl PropTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &PhpStr) -> Option<&Value> {
        self.map.get(key)
    }

    pub fn get_mut(&mut self, key: &PhpStr) -> Option<&mut Value> {
        self.map.get_mut(key)
    }

    pub fn contains_key(&self, key: &PhpStr) -> bool {
        self.map.contains_key(key)
    }

    /// Set `key`, returning the previous value. A new key goes last.
    pub fn insert(&mut self, key: PhpStr, v: Value) -> Option<Value> {
        self.map.insert(key, v)
    }

    /// The number of properties.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The properties in order.
    pub fn iter(&self) -> impl Iterator<Item = (&PhpStr, &Value)> + '_ {
        self.map.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&PhpStr, &mut Value)> + '_ {
        self.map.iter_mut()
    }

    pub fn keys(&self) -> impl Iterator<Item = &PhpStr> + '_ {
        self.map.keys()
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> + '_ {
        self.map.values()
    }

    /// One past the last position.
    pub fn pos_len(&self) -> usize {
        self.map.len()
    }

    /// The entry at position `pos`.
    pub fn get_index(&self, pos: usize) -> Option<(&PhpStr, &Value)> {
        self.map.get_index(pos)
    }

    pub fn get_index_mut(&mut self, pos: usize) -> Option<(&PhpStr, &mut Value)> {
        self.map.get_index_mut(pos)
    }

    /// The position of `key`.
    pub fn get_index_of(&self, key: &PhpStr) -> Option<usize> {
        self.map.get_index_of(key)
    }

    /// Remove `key`, returning its position and value.
    pub(crate) fn remove_full(&mut self, key: &PhpStr) -> Option<(usize, Value)> {
        self.map.shift_remove_full(key).map(|(i, _, v)| (i, v))
    }

    pub(crate) fn insert_at(&mut self, pos: usize, key: PhpStr, v: Value) {
        self.map.shift_insert(pos, key, v);
    }
}

impl std::ops::Index<usize> for PropTable {
    type Output = Value;
    fn index(&self, pos: usize) -> &Value {
        &self.map[pos]
    }
}

impl std::ops::IndexMut<usize> for PropTable {
    fn index_mut(&mut self, pos: usize) -> &mut Value {
        &mut self.map[pos]
    }
}

impl FromIterator<(PhpStr, Value)> for PropTable {
    fn from_iter<I: IntoIterator<Item = (PhpStr, Value)>>(it: I) -> Self {
        PropTable {
            map: it.into_iter().collect(),
        }
    }
}

impl<'a> IntoIterator for &'a PropTable {
    type Item = (&'a PhpStr, &'a Value);
    type IntoIter = indexmap::map::Iter<'a, PhpStr, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.map.iter()
    }
}
