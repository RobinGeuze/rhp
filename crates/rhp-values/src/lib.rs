//! rhp runtime value model — the zval analogue.
//!
//! Storage, PHP key/number normalization rules and the operators on values
//! ([`ops`]). No AST. Everything here is owned and
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
pub mod handles;
pub mod mem;
pub mod ops;
pub mod thrown;

pub use handles::{alloc_object_id, release_object_id};
pub use thrown::Thrown;

// php's error levels (Zend/zend_errors.h).
pub const E_ERROR: i64 = 1;
pub const E_WARNING: i64 = 2;
pub const E_PARSE: i64 = 4;
pub const E_NOTICE: i64 = 8;
pub const E_CORE_ERROR: i64 = 16;
pub const E_CORE_WARNING: i64 = 32;
pub const E_COMPILE_ERROR: i64 = 64;
pub const E_COMPILE_WARNING: i64 = 128;
pub const E_USER_ERROR: i64 = 256;
pub const E_USER_WARNING: i64 = 512;
pub const E_USER_NOTICE: i64 = 1024;
pub const E_STRICT: i64 = 2048;
pub const E_RECOVERABLE_ERROR: i64 = 4096;
pub const E_DEPRECATED: i64 = 8192;
pub const E_USER_DEPRECATED: i64 = 16384;
/// Every level but E_STRICT (php 8.4+).
pub const E_ALL: i64 = 30719;

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
    /// A resource (stream, …): an opaque id + type name. The actual stream state
    /// (buffers, file handles, the stdout/stderr target) lives in the VM, keyed
    /// by this id, so this crate stays I/O-free.
    Resource(ResourceHandle),
}

/// A resource value's identity: an id (shared with the VM's resource table) and
/// its php type name (e.g. "stream").
pub struct ResourceData {
    pub id: u32,
    pub type_name: &'static str,
}

pub type ResourceHandle = Rc<ResourceData>;

thread_local! {
    /// Ids of stream resources whose last `Value` reference was dropped. The
    /// VM drains this and frees the backing stream (php's refcount-0 resource
    /// destructor: closes the descriptor, releases flock locks, flushes).
    static DROPPED_STREAMS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

/// Forget queued stream drops (a new engine on this thread: resource ids
/// restart, so a stale id would free the new request's stream).
pub fn reset_dropped_streams() {
    let _ = DROPPED_STREAMS.try_with(|q| {
        if let Ok(mut q) = q.try_borrow_mut() {
            q.clear();
        }
    });
}

impl Drop for ResourceData {
    fn drop(&mut self) {
        // Streams and processes are freed on their last reference (php's
        // refcount-0 resource destructors); other native resources live on.
        if self.type_name == "stream" || self.type_name == "process" {
            let id = self.id;
            let _ = DROPPED_STREAMS.try_with(|q| {
                if let Ok(mut q) = q.try_borrow_mut() {
                    q.push(id);
                }
            });
        }
    }
}

/// Take the ids of stream resources released since the last call.
pub fn take_dropped_streams() -> Vec<u32> {
    DROPPED_STREAMS
        .try_with(|q| {
            let mut q = q.borrow_mut();
            if q.is_empty() {
                Vec::new()
            } else {
                std::mem::take(&mut *q)
            }
        })
        .unwrap_or_default()
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
    /// A first-class callable (`strlen(...)`, `$o->m(...)`): calling this closure
    /// forwards its arguments to the wrapped callable value. `fnidx`/`captures`
    /// are unused when this is `Some`.
    pub wrapped: Option<Box<Value>>,
    /// The Closure object's handle ([`alloc_object_id`]).
    pub id: u32,
    /// The closure's own static variables, by name, once its body bound
    /// them (php copies a closure's static variables into each Closure
    /// object). Unbound ones still have their declared initial value.
    pub statics: RefCell<Vec<(PhpStr, PhpRef)>>,
}

impl ClosureData {
    /// A copy of the static variables' current values, for a new Closure
    /// made from this one (`bind`, `clone`: zend_array_dup).
    pub fn statics_copy(&self) -> RefCell<Vec<(PhpStr, PhpRef)>> {
        RefCell::new(
            self.statics
                .borrow()
                .iter()
                .map(|(k, v)| (k.clone(), Rc::new(RefCell::new(v.borrow().clone()))))
                .collect(),
        )
    }
}

impl Drop for ClosureData {
    fn drop(&mut self) {
        // What the closure holds is freed before its handle goes back.
        self.captures.clear();
        self.bound_this = None;
        self.wrapped = None;
        self.statics.get_mut().clear();
        release_object_id(self.id);
    }
}

pub type ClosureHandle = Rc<ClosureData>;

/// The object handle of an object or closure value (`spl_object_id`).
pub fn object_id(v: &Value) -> Option<u32> {
    match v {
        Value::Object(h) => Some(h.borrow().id),
        Value::Closure(c) => Some(c.id),
        _ => None,
    }
}

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
    /// Rarely used state: the native payload and the lazy object info.
    /// Boxed so that objects with neither pay a pointer.
    extra: Option<Box<ObjectExtra>>,
}

/// See [`ObjectData::payload`] and [`ObjectData::lazy`].
#[derive(Default)]
struct ObjectExtra {
    /// Native state (see [`Payload`]).
    native: Option<Payload>,
    /// Lazy object state (php's IS_OBJ_LAZY_* flags plus its
    /// `zend_lazy_object_info`).
    lazy: Option<LazyInfo>,
}

/// A lazy object's state (Zend/zend_lazy_objects.c). php keeps the info in
/// a table keyed by handle; here it lives with the object, so it goes away
/// with it.
pub struct LazyInfo {
    /// php's object flags: [`LazyInfo::UNINIT`] and [`LazyInfo::PROXY`].
    /// Both are clear while the initializer runs (re-entrancy guard), which
    /// is when the info exists but the object is not lazy.
    pub obj_flags: u8,
    /// php's `zend_lazy_object_flags_t` (`STRATEGY_*`, `INITIALIZED`,
    /// `SKIP_*`).
    pub flags: u8,
    /// The initializer or factory (until a proxy is initialized).
    pub initializer: Value,
    /// An initialized proxy's real instance.
    pub instance: Option<ObjectHandle>,
    /// Storage keys of the declared slots still lazy (IS_PROP_LAZY).
    pub lazy_props: Vec<PhpStr>,
}

impl LazyInfo {
    pub const UNINIT: u8 = 1;
    pub const PROXY: u8 = 2;
    pub const STRATEGY_PROXY: u8 = 1 << 0;
    pub const STRATEGY_GHOST: u8 = 1 << 1;
    pub const INITIALIZED: u8 = 1 << 2;
    pub const SKIP_INITIALIZATION_ON_SERIALIZE: u8 = 1 << 3;
    pub const SKIP_DESTRUCTOR: u8 = 1 << 4;
}

/// The native state an internal class keeps with each of its objects: what
/// php's extensions keep in the struct around `zend_object` (a libxml node,
/// a socket, a curl handle). Set by the builtin that creates the object,
/// read by builtins and object handlers with [`ObjectData::payload`], and
/// dropped with the object's storage, which makes the payload's `Drop`
/// php's `free_obj`: it runs once, when the last reference goes, with no
/// host, so it must not call into PHP (work that may, such as flushing to a
/// user stream, belongs in the class's `dtor_obj` handler).
///
/// Contract:
/// - The payload is the only owner of per-object native state: no table
///   keyed by object id or address. State that lives in the object moves
///   with it. (ext/xml also keeps weak references to its parsers, only to
///   break their cycles at the next request; see `ext/xml`.)
/// - Read it as an `Rc<T>` clone and release the object borrow before
///   calling into PHP or the host; mutate through a `Cell`/`RefCell` inside
///   `T`. A payload never holds a strong reference to its own object.
/// - `Rc<dyn Any>` because objects are `Rc` and stay on their VM's thread
///   (docs/design/libraries.md, Threading). Today's payloads are `!Send`
///   (`Cell`, `RefCell`, raw library pointers). If objects ever move
///   between threads, the alias becomes `Arc<dyn Any + Send + Sync>` and
///   every payload type changes with it: `Mutex` or atomics instead of
///   `Cell`/`RefCell`, and a `Send` wrapper for each library handle whose
///   library allows that.
pub type Payload = Rc<dyn std::any::Any>;

impl ObjectData {
    pub fn new(
        class: ClassId,
        id: u32,
        properties: IndexMap<PhpStr, Value, FxBuildHasher>,
    ) -> Self {
        ObjectData {
            class,
            id,
            properties,
            extra: None,
        }
    }

    /// The lazy object state; `None` for ordinary objects.
    #[inline]
    pub fn lazy(&self) -> Option<&LazyInfo> {
        self.extra.as_deref()?.lazy.as_ref()
    }

    /// The lazy object state, mutably.
    #[inline]
    pub fn lazy_mut(&mut self) -> Option<&mut LazyInfo> {
        self.extra.as_deref_mut()?.lazy.as_mut()
    }

    /// Make the object lazy with `info`, returning the previous state.
    pub fn set_lazy(&mut self, info: LazyInfo) -> Option<LazyInfo> {
        self.extra
            .get_or_insert_with(Default::default)
            .lazy
            .replace(info)
    }

    /// Drop the lazy state (returned, to drop after the borrow ends).
    pub fn take_lazy(&mut self) -> Option<LazyInfo> {
        let extra = self.extra.as_deref_mut()?;
        let l = extra.lazy.take();
        if extra.native.is_none() {
            self.extra = None;
        }
        l
    }

    /// php's zend_object_is_lazy (uninitialized, or a proxy).
    #[inline]
    pub fn is_lazy(&self) -> bool {
        self.lazy().is_some_and(|l| l.obj_flags != 0)
    }

    /// An uninitialized lazy object (`!zend_lazy_object_initialized`).
    #[inline]
    pub fn is_lazy_uninit(&self) -> bool {
        self.lazy()
            .is_some_and(|l| l.obj_flags & LazyInfo::UNINIT != 0)
    }

    /// An initialized lazy proxy's real instance.
    pub fn lazy_instance(&self) -> Option<ObjectHandle> {
        let l = self.lazy()?;
        if l.obj_flags == LazyInfo::PROXY {
            l.instance.clone()
        } else {
            None
        }
    }

    /// The object's native state, when it is a `T`.
    pub fn payload<T: std::any::Any>(&self) -> Option<Rc<T>> {
        self.extra.as_deref()?.native.clone()?.downcast::<T>().ok()
    }

    /// Whether the object carries native state.
    pub fn has_payload(&self) -> bool {
        self.extra.as_deref().is_some_and(|e| e.native.is_some())
    }

    /// Attach native state, returning the previous one. Drop that after the
    /// object borrow is released: its `Drop` may touch other objects.
    pub fn set_payload(&mut self, state: Payload) -> Option<Payload> {
        self.extra
            .get_or_insert_with(Default::default)
            .native
            .replace(state)
    }

    /// Detach the native state (php's `free_obj` run early, e.g. `close()`).
    pub fn take_payload(&mut self) -> Option<Payload> {
        let extra = self.extra.as_deref_mut()?;
        let p = extra.native.take();
        if extra.lazy.is_none() {
            self.extra = None;
        }
        p
    }
}

thread_local! {
    /// Called with an object's id when its storage is freed, for engine
    /// state still kept by object id (the internal array pointer of
    /// objects, archives not on the payload yet).
    static OBJECT_FREE_HOOK: std::cell::Cell<Option<fn(u32)>> = const { std::cell::Cell::new(None) };
}

/// The object an initialized lazy proxy forwards to (php's
/// zend_get_properties_no_lazy_init target); `h` itself otherwise.
pub fn lazy_target(h: &ObjectHandle) -> ObjectHandle {
    let mut cur = h.clone();
    loop {
        let next = cur.borrow().lazy_instance();
        match next {
            Some(i) => cur = i,
            None => return cur,
        }
    }
}

/// Install the object free hook (see [`ObjectData`]'s `Drop`).
pub fn set_object_free_hook(f: fn(u32)) {
    OBJECT_FREE_HOOK.with(|h| h.set(Some(f)));
}

impl Drop for ObjectData {
    fn drop(&mut self) {
        // php's free_obj: the native state goes first, then the lazy
        // info (zend_object_std_dtor), then the properties.
        if let Some(mut extra) = self.extra.take() {
            drop(extra.native.take());
            drop(extra.lazy.take());
        }
        // `try_with`: thread-local teardown may already have run.
        if let Ok(Some(f)) = OBJECT_FREE_HOOK.try_with(|h| h.get()) {
            f(self.id);
        }
        // The handle goes back after what the properties held (php frees the
        // object, then puts its handle on the free list).
        if !self.properties.is_empty() {
            self.properties.clear();
        }
        release_object_id(self.id);
    }
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

/// Class-level flags (php's `ce_flags`).
#[derive(Clone, Copy, Default, Debug)]
pub struct ClassFlags {
    pub is_abstract: bool,
    pub is_final: bool,
    pub is_readonly: bool,
    /// `#[\AllowDynamicProperties]` (php's ZEND_ACC_ALLOW_DYNAMIC_PROPERTIES).
    /// Inherited.
    pub allow_dynamic: bool,
    /// ZEND_ACC_NO_DYNAMIC_PROPERTIES: an internal class's `@strict-properties`
    /// (enums and readonly classes are checked by kind). Not inherited.
    pub no_dynamic: bool,
    /// ZEND_ACC_NOT_SERIALIZABLE (`@not-serializable`): serialize() and
    /// unserialize() throw. Inherited.
    pub not_serializable: bool,
    /// The objects have no `clone_obj` handler: `clone` throws "Trying to
    /// clone an uncloneable object of class X". php keeps this in the
    /// handler table, which subclasses inherit with `create_object`; set at
    /// link time from the class's handlers. Inherited.
    pub not_cloneable: bool,
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

    /// Mutable access when this is the only handle to the buffer.
    pub fn get_mut(&mut self) -> Option<&mut BString> {
        Rc::get_mut(&mut self.0)
    }

    /// How many values share this buffer (debug_zval_dump's refcount).
    pub fn ref_count(&self) -> usize {
        Rc::strong_count(&self.0)
    }

    /// Whether both share one buffer.
    pub fn ptr_eq(&self, other: &PhpStr) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
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
            Value::Resource(r) => ArrayKey::Int(r.id as i64),
            Value::Ref(r) => return ArrayKey::from_value(&r.borrow()),
            Value::Array(_) | Value::Object(_) | Value::Closure(_) => return None,
        })
    }

    /// The key as a value (`array_keys`, `foreach`'s key, …).
    #[inline]
    pub fn to_value(&self) -> Value {
        match self {
            ArrayKey::Int(n) => Value::Int(*n),
            ArrayKey::Str(s) => Value::Str(s.clone()),
        }
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
pub struct ArrayData {
    pub entries: IndexMap<ArrayKey, Value, FxBuildHasher>,
    /// php's nNextFreeElement: the key `$a[] = ...` uses. `i64::MIN` until an
    /// integer key is inserted (a fresh append then uses 0). Saturates at
    /// `i64::MAX`; an append fails while that key is occupied.
    next_free: i64,
    /// Internal pointer for `current`/`next`/`prev`/`end`/`reset`/`key`.
    /// `entries.len()` (or more) is past the end. A removal before it moves
    /// it back, so it keeps pointing at the same element (php's bucket
    /// position; a removed current element leaves it on the next one).
    pub pos: usize,
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
            pos: self.pos,
            iters: self.iters.clone(),
        }
    }
}

impl ArrayData {
    fn new() -> Self {
        ArrayData {
            entries: IndexMap::with_hasher(FxBuildHasher),
            next_free: i64::MIN,
            pos: 0,
            iters: None,
        }
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

    /// Remove the entry at index `i`, keeping the internal pointer and the
    /// iterators on the same elements.
    pub fn remove_index(&mut self, i: usize) -> Option<(ArrayKey, Value)> {
        let r = self.entries.shift_remove_index(i)?;
        self.removed_at(i);
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

    /// Move every iterator `n` places on (zend_hash_iterators_advance).
    pub fn iters_advance(&mut self, n: usize) {
        if let Some(it) = &mut self.iters {
            for (_, p) in it.iter_mut() {
                *p += n;
            }
        }
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
}

/// A PHP array value. Clone is O(1) (Rc bump); mutation is COW via `make_mut`.
#[derive(Clone)]
pub struct PhpArray(Rc<ArrayData>);

impl PhpArray {
    pub fn new() -> Self {
        PhpArray(Rc::new(ArrayData::new()))
    }

    /// Identity of the shared array storage (php's `zend_array *`): equal for
    /// copies that have not been written since, so a recursive array reached
    /// through a reference is recognized (`*RECURSION*` / circular-reference
    /// guards in the dumpers and encoders).
    pub fn data_ptr(&self) -> usize {
        Rc::as_ptr(&self.0) as usize
    }

    pub fn len(&self) -> usize {
        self.0.entries.len()
    }

    /// php's nNextFreeElement (see [`ArrayData::next_free`]).
    pub fn next_free(&self) -> i64 {
        self.0.next_free
    }

    /// How many values share this storage (debug_zval_dump's refcount).
    pub fn ref_count(&self) -> usize {
        Rc::strong_count(&self.0)
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
        let data = self.make_mut();
        let Some(k) = data.next_append_key() else {
            return false;
        };
        data.bump_next_int(k);
        data.entries.insert(ArrayKey::Int(k), value);
        true
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

    /// COW mutable access to a freshly appended slot (`$a[]`); `None` when
    /// the next element is already occupied.
    pub fn append_mut(&mut self) -> Option<&mut Value> {
        let data = self.make_mut();
        let k = data.next_append_key()?;
        data.bump_next_int(k);
        Some(data.entries.entry(ArrayKey::Int(k)).or_insert(Value::Null))
    }

    /// COW mutable access to an existing slot (no insert).
    pub fn get_mut(&mut self, key: &ArrayKey) -> Option<&mut Value> {
        self.make_mut().entries.get_mut(key)
    }

    /// COW remove, preserving insertion order. `next_free` is not lowered.
    pub fn remove(&mut self, key: &ArrayKey) -> Option<Value> {
        let data = self.make_mut();
        let (i, _, v) = data.entries.shift_remove_full(key)?;
        data.removed_at(i);
        Some(v)
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
            Value::Resource(_) => "resource",
            Value::Ref(r) => r.borrow().type_name(),
        }
    }
}

#[cfg(test)]
mod tests;
