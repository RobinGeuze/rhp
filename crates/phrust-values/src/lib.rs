//! phrust runtime value model — the zval analogue.
//!
//! Pure storage + PHP key/number normalization rules. No AST, no operator
//! semantics (those live in `phrust-semantics`). Everything here is owned and
//! `'static`: the arena holds the AST, the runtime holds owned copies.
//!
//! See `docs/design/value-model.md` for the full rationale.

use std::cell::RefCell;
use std::rc::Rc;

use bstr::BString;
use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;

pub mod conv;
pub mod fmt;
pub mod thrown;

pub use thrown::Thrown;

/// The runtime value. Owned, cheaply cloneable, `'static`.
///
/// Two sharing disciplines, kept strictly separate:
/// - value types with copy-on-write (`Str`, `Array`): `Rc<T>` + `make_mut`.
/// - handle/reference types (`Object`, `Ref`): `Rc<RefCell<T>>` (aliasing).
#[derive(Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(PhpStr),
    Array(PhpArray),
    Object(ObjectHandle),
    /// A PHP reference (`&`): an aliased slot. Not a PHP *type* — an internal
    /// marker that a slot is shared. See value-model.md §4.
    Ref(PhpRef),
    /// A closure / arrow function. Holds captured values + the proto index
    /// (the compiled proto lives in the VM, so this crate stays below bytecode).
    Closure(ClosureHandle),
}

/// A closure's runtime state.
pub struct ClosureData {
    /// Index into the program's closure protos (resolved by the VM).
    pub fnidx: u32,
    /// Captured outer variables, in capture order (bound to CVs 0..n at call).
    /// `$this`, when bound, is captured here like any other variable.
    pub captures: Vec<Value>,
    /// Bound `$this` (for closures created in an instance method).
    pub bound_this: Option<ObjectHandle>,
    /// The class scope the closure was defined in (`self::`, private access).
    pub bound_class: Option<ClassId>,
    /// The late-static-binding scope (`static::`) at closure creation.
    pub bound_static: Option<ClassId>,
}

pub type ClosureHandle = Rc<ClosureData>;

/// Interned class id; the class table lives in the engine, not the value.
pub type ClassId = u32;

/// Shared object handle (reference-handle semantics). Phase 7.
pub type ObjectHandle = Rc<RefCell<ObjectData>>;

/// A PHP reference: a shared, mutable value slot.
pub type PhpRef = Rc<RefCell<Value>>;

/// Object storage.
pub struct ObjectData {
    pub class: ClassId,
    pub id: u32,
    pub properties: IndexMap<PhpStr, Value, FxBuildHasher>,
}

/// What kind of class-like declaration a class table entry is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClassKind {
    Class,
    Interface,
    Trait,
    Enum,
}

/// Member visibility.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Visibility {
    Public,
    Protected,
    Private,
}

/// Class-level modifier flags.
#[derive(Clone, Copy, Default, Debug)]
pub struct ClassFlags {
    pub is_abstract: bool,
    pub is_final: bool,
    pub is_readonly: bool,
}

// ---------------------------------------------------------------------------
// PhpStr — binary-safe, copy-on-write byte string
// ---------------------------------------------------------------------------

/// Owned, binary-safe, copy-on-write PHP string. Clone is O(1); mutation is COW.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PhpStr(Rc<BString>);

impl PhpStr {
    pub fn from_bytes(b: impl Into<Vec<u8>>) -> Self {
        PhpStr(Rc::new(BString::new(b.into())))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// COW mutable access: clones the buffer iff it is currently shared.
    pub fn make_mut(&mut self) -> &mut BString {
        Rc::make_mut(&mut self.0)
    }
}

impl Default for PhpStr {
    fn default() -> Self {
        PhpStr::from_bytes(Vec::new())
    }
}

impl From<&[u8]> for PhpStr {
    fn from(b: &[u8]) -> Self {
        PhpStr::from_bytes(b.to_vec())
    }
}

impl From<&str> for PhpStr {
    fn from(s: &str) -> Self {
        PhpStr::from_bytes(s.as_bytes().to_vec())
    }
}

impl std::fmt::Debug for PhpStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

// ---------------------------------------------------------------------------
// PhpArray — ordered int|string map with COW value semantics
// ---------------------------------------------------------------------------

/// PHP array key: canonicalized to int-or-string.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum ArrayKey {
    Int(i64),
    Str(PhpStr),
}

impl ArrayKey {
    /// Normalize an arbitrary scalar into an array key, per PHP 8.5 rules
    /// (value-model.md §3b). Float/null/bool coercion and the canonical-
    /// integer-string rule are applied here.
    ///
    /// Returns `None` for value types that are not legal keys (array/object);
    /// the caller raises the appropriate `TypeError`.
    pub fn from_value(v: &Value) -> Option<ArrayKey> {
        Some(match v {
            Value::Int(n) => ArrayKey::Int(*n),
            Value::Bool(b) => ArrayKey::Int(if *b { 1 } else { 0 }),
            Value::Null => ArrayKey::Str(PhpStr::from_bytes(Vec::new())),
            Value::Float(f) => ArrayKey::Int(float_to_int(*f)),
            Value::Str(s) => ArrayKey::from_bytes(s.as_bytes()),
            Value::Ref(r) => return ArrayKey::from_value(&r.borrow()),
            Value::Array(_) | Value::Object(_) | Value::Closure(_) => return None,
        })
    }

    /// Apply the canonical-integer-string rule to raw bytes.
    pub fn from_bytes(s: &[u8]) -> ArrayKey {
        if let Some(n) = canonical_int_string(s) {
            ArrayKey::Int(n)
        } else {
            ArrayKey::Str(PhpStr::from_bytes(s.to_vec()))
        }
    }
}

impl std::fmt::Debug for ArrayKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArrayKey::Int(n) => write!(f, "{n}"),
            ArrayKey::Str(s) => write!(f, "{s:?}"),
        }
    }
}

/// A string is an integer key iff it is the canonical decimal form of an i64:
/// `^-?(0|[1-9][0-9]*)$`, in range, and not `"-0"`. Equivalent to
/// `(string)(int)s === s`.
fn canonical_int_string(s: &[u8]) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let (neg, digits) = match s[0] {
        b'-' => (true, &s[1..]),
        _ => (false, s),
    };
    if digits.is_empty() {
        return None;
    }
    // No leading zeros (except the single "0"), digits only.
    if digits.len() > 1 && digits[0] == b'0' {
        return None;
    }
    if !digits.iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // "-0" is not canonical (its canonical form is "0").
    if neg && digits == b"0" {
        return None;
    }
    // Parse with range check.
    let text = std::str::from_utf8(s).ok()?;
    text.parse::<i64>().ok()
}

/// PHP's `zend_dval_to_lval`: toward-zero truncation; NaN/±INF/out-of-range → 0.
/// Reused by float array keys and `(int)` casts.
pub fn float_to_int(f: f64) -> i64 {
    if f.is_nan() || f.is_infinite() {
        return 0;
    }
    let t = f.trunc();
    // PHP wraps out-of-range doubles modulo 2^64 rather than saturating, but for
    // in-range values this is just a cast. In-range covers all realistic cases;
    // the modular-wrap edge is handled explicitly to match PHP.
    if t >= i64::MIN as f64 && t < 9_223_372_036_854_775_808.0 {
        t as i64
    } else {
        // Match PHP's modulo-2^64 wrap for out-of-range finite doubles.
        float_to_int_wrapping(t)
    }
}

fn float_to_int_wrapping(t: f64) -> i64 {
    // Emulate zend_dval_to_lval's modular reduction for |t| >= 2^63.
    let two64 = 18_446_744_073_709_551_616.0_f64;
    let mut m = t % two64;
    if m < 0.0 {
        m += two64;
    }
    // m in [0, 2^64); fold into i64 via wrapping.
    if m >= 9_223_372_036_854_775_808.0 {
        (m - two64) as i64
    } else {
        m as i64
    }
}

/// The array buffer; insertion order preserved by IndexMap.
#[derive(Clone)]
pub struct ArrayData {
    pub entries: IndexMap<ArrayKey, Value, FxBuildHasher>,
    /// Next integer index for `$a[] = ...`. `None` = no int key ever inserted
    /// (fresh append is 0). High-water mark; never lowered by `unset`.
    pub next_int: Option<i64>,
    /// True once an append would overflow i64 (next element already occupied).
    pub append_disabled: bool,
}

impl ArrayData {
    fn new() -> Self {
        ArrayData {
            entries: IndexMap::with_hasher(FxBuildHasher),
            next_int: None,
            append_disabled: false,
        }
    }

    /// Update `next_int` after inserting integer key `k` (append or explicit).
    fn bump_next_int(&mut self, k: i64) {
        match k.checked_add(1) {
            Some(k1) => {
                self.next_int = Some(match self.next_int {
                    None => k1,
                    Some(n) => n.max(k1),
                });
            }
            None => {
                // k == i64::MAX: future appends overflow.
                self.next_int = Some(i64::MAX);
                self.append_disabled = true;
            }
        }
    }
}

/// A PHP array value. Clone is O(1) (Rc bump); mutation is COW via `make_mut`.
#[derive(Clone)]
pub struct PhpArray(Rc<ArrayData>);

impl PhpArray {
    pub fn new() -> Self {
        PhpArray(Rc::new(ArrayData::new()))
    }

    pub fn len(&self) -> usize {
        self.0.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.entries.is_empty()
    }

    pub fn get(&self, k: &ArrayKey) -> Option<&Value> {
        self.0.entries.get(k)
    }

    pub fn iter(&self) -> indexmap::map::Iter<'_, ArrayKey, Value> {
        self.0.entries.iter()
    }

    /// The (key, value) at insertion position `i` (for `foreach` iteration).
    pub fn get_index(&self, i: usize) -> Option<(&ArrayKey, &Value)> {
        self.0.entries.get_index(i)
    }

    pub fn next_int(&self) -> Option<i64> {
        self.0.next_int
    }

    /// COW mutable access to the backing buffer.
    pub fn make_mut(&mut self) -> &mut ArrayData {
        Rc::make_mut(&mut self.0)
    }

    /// Insert/overwrite at an explicit key (COW). Updates the append counter.
    pub fn insert(&mut self, key: ArrayKey, value: Value) {
        let data = self.make_mut();
        if let ArrayKey::Int(n) = key {
            data.bump_next_int(n);
        }
        data.entries.insert(key, value);
    }

    /// Append `$a[] = value` (COW). No-op insert if append is disabled.
    pub fn push(&mut self, value: Value) {
        let data = self.make_mut();
        if data.append_disabled {
            return;
        }
        let k = data.next_int.unwrap_or(0);
        data.bump_next_int(k);
        data.entries.insert(ArrayKey::Int(k), value);
    }

    /// COW mutable access to the slot at `key`, inserting `Null` if absent.
    /// Updates the append counter for integer keys. Used by nested lvalue writes.
    pub fn get_or_insert_mut(&mut self, key: ArrayKey) -> &mut Value {
        let data = self.make_mut();
        if let ArrayKey::Int(n) = key {
            data.bump_next_int(n);
        }
        data.entries.entry(key).or_insert(Value::Null)
    }

    /// COW mutable access to a freshly appended slot (`$a[]`).
    pub fn append_mut(&mut self) -> &mut Value {
        let data = self.make_mut();
        let k = data.next_int.unwrap_or(0);
        data.bump_next_int(k);
        data.entries.entry(ArrayKey::Int(k)).or_insert(Value::Null)
    }

    /// COW mutable access to an existing slot (no insert).
    pub fn get_mut(&mut self, key: &ArrayKey) -> Option<&mut Value> {
        self.make_mut().entries.get_mut(key)
    }

    /// COW remove, preserving insertion order. `next_int` is not lowered.
    pub fn remove(&mut self, key: &ArrayKey) -> Option<Value> {
        self.make_mut().entries.shift_remove(key)
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

// ---------------------------------------------------------------------------
// Value convenience constructors
// ---------------------------------------------------------------------------

impl Value {
    pub fn str(b: impl Into<Vec<u8>>) -> Value {
        Value::Str(PhpStr::from_bytes(b))
    }

    /// The PHP type name as reported by `gettype()`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "NULL",
            Value::Bool(_) => "boolean",
            Value::Int(_) => "integer",
            Value::Float(_) => "double",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) | Value::Closure(_) => "object",
            Value::Ref(r) => r.borrow().type_name(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> ArrayKey {
        ArrayKey::from_bytes(s.as_bytes())
    }

    #[test]
    fn canonical_int_string_rules() {
        assert_eq!(key("0"), ArrayKey::Int(0));
        assert_eq!(key("8"), ArrayKey::Int(8));
        assert_eq!(key("-5"), ArrayKey::Int(-5));
        assert_eq!(key("9223372036854775807"), ArrayKey::Int(i64::MAX));
        // stay strings:
        for s in ["08", "00", "01", "+1", " 1", "1.5", "-0", "9223372036854775808", ""] {
            assert!(matches!(key(s), ArrayKey::Str(_)), "{s:?} should be a string key");
        }
    }

    #[test]
    fn key_from_scalars() {
        assert_eq!(ArrayKey::from_value(&Value::Bool(true)), Some(ArrayKey::Int(1)));
        assert_eq!(ArrayKey::from_value(&Value::Bool(false)), Some(ArrayKey::Int(0)));
        assert_eq!(ArrayKey::from_value(&Value::Float(2.9)), Some(ArrayKey::Int(2)));
        assert_eq!(ArrayKey::from_value(&Value::Float(-2.9)), Some(ArrayKey::Int(-2)));
        assert_eq!(
            ArrayKey::from_value(&Value::Null),
            Some(ArrayKey::Str(PhpStr::from_bytes(Vec::new())))
        );
        assert_eq!(ArrayKey::from_value(&Value::Array(PhpArray::new())), None);
    }

    #[test]
    fn float_to_int_cases() {
        assert_eq!(float_to_int(2.9), 2);
        assert_eq!(float_to_int(-2.9), -2);
        assert_eq!(float_to_int(f64::NAN), 0);
        assert_eq!(float_to_int(f64::INFINITY), 0);
        assert_eq!(float_to_int(0.0), 0);
    }

    #[test]
    fn next_int_high_water() {
        // [10=>x][] -> keys 10,11
        let mut a = PhpArray::new();
        a.insert(ArrayKey::Int(10), Value::str("x"));
        a.push(Value::str("y"));
        let keys: Vec<_> = a.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(keys, vec![ArrayKey::Int(10), ArrayKey::Int(11)]);

        // negative first key: [-5=>a][] -> keys -5,-4
        let mut b = PhpArray::new();
        b.insert(ArrayKey::Int(-5), Value::str("a"));
        b.push(Value::str("b"));
        let keys: Vec<_> = b.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(keys, vec![ArrayKey::Int(-5), ArrayKey::Int(-4)]);
    }

    #[test]
    fn cow_isolation() {
        let mut a = PhpArray::new();
        a.push(Value::Int(1));
        let b = a.clone();
        a.push(Value::Int(2));
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 1); // b unaffected — COW
    }
}
