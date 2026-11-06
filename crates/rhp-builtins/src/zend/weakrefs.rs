//! WeakReference and WeakMap (Zend/zend_weakrefs.c).
//!
//! A WeakReference's payload and a WeakMap's entries hold `Weak` pointers
//! to their referents, so they keep nothing alive. The registry
//! (php's `EG(weakrefs)`) maps the handle of every weakly referenced object
//! or closure to the WeakReference and WeakMaps that refer to it. Such an
//! object carries php's IS_OBJ_WEAKLY_REFERENCED flag; freeing it calls
//! [`notify`] (from rhp-values, after the payload and before the
//! properties and the handle go, as `zend_object_std_dtor` does), which
//! clears the WeakReference and removes the WeakMap entries at once.
//!
//! A handle is only reused after its object was freed, and [`notify`]
//! drops the handle's registry entry then, so a handle in the registry
//! always names a live object.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};

use rhp_values::ops::deref;
use rhp_values::{conv, ClosureData, ObjectData, ObjectHandle, PhpArray, PhpRef, Thrown, Value};
use rustc_hash::FxHashMap;

use crate::zend::internal_iterator::{self, IteratorFuncs};
use crate::zend::object_handlers::{
    self, ClassHandlers, CloneObj, DimType, DimensionHandlers, ObjectHandlers, PropPurpose,
};
use crate::Host;

/// A weakly held object or closure.
#[derive(Clone)]
enum Referent {
    Obj(Weak<RefCell<ObjectData>>),
    Closure(Weak<ClosureData>),
}

impl Referent {
    /// `v`'s handle and a weak pointer to it, marking it weakly referenced.
    fn of(v: &Value) -> Option<(u32, Referent)> {
        match v {
            Value::Object(h) => {
                let mut o = h.borrow_mut();
                o.set_weakly_referenced();
                Some((o.id, Referent::Obj(Rc::downgrade(h))))
            }
            Value::Closure(c) => {
                c.weakly_referenced.set(true);
                Some((c.id, Referent::Closure(Rc::downgrade(c))))
            }
            Value::Ref(r) => Referent::of(&r.borrow()),
            _ => None,
        }
    }

    fn get(&self) -> Option<Value> {
        match self {
            Referent::Obj(w) => w.upgrade().map(Value::Object),
            Referent::Closure(w) => w.upgrade().map(Value::Closure),
        }
    }
}

/// What refers weakly to an object: a WeakReference or a WeakMap.
enum Target {
    Ref(Weak<RefState>),
    Map(Weak<MapState>),
}

impl Target {
    fn is(&self, p: *const ()) -> bool {
        match self {
            Target::Ref(w) => w.as_ptr() as *const () == p,
            Target::Map(w) => w.as_ptr() as *const () == p,
        }
    }
}

thread_local! {
    /// php's `EG(weakrefs)`, keyed by handle.
    static REGISTRY: RefCell<FxHashMap<u32, Vec<Target>>> = RefCell::new(FxHashMap::default());
}

/// `zend_weakref_register`.
fn register(id: u32, t: Target) {
    REGISTRY.with(|r| r.borrow_mut().entry(id).or_default().push(t));
}

/// `zend_weakref_unregister` (without the unref, which the caller does).
fn unregister(id: u32, p: *const ()) {
    let _ = REGISTRY.try_with(|r| {
        let Ok(mut r) = r.try_borrow_mut() else {
            return;
        };
        if let Some(v) = r.get_mut(&id) {
            v.retain(|t| !t.is(p));
            if v.is_empty() {
                r.remove(&id);
            }
        }
    });
}

/// `zend_weakrefs_notify`: the object or closure with handle `id` is being
/// freed. Installed as rhp-values' weak notify hook.
pub fn notify(id: u32) {
    let Ok(Some(targets)) = REGISTRY.try_with(|r| r.try_borrow_mut().ok()?.remove(&id)) else {
        return;
    };
    for t in targets {
        match t {
            Target::Ref(w) => {
                if let Some(s) = w.upgrade() {
                    s.referent.replace(None);
                }
            }
            Target::Map(w) => {
                if let Some(m) = w.upgrade() {
                    // The value goes after the map borrow ends: freeing it
                    // may reach this map again.
                    let e = m.inner.borrow_mut().remove(id);
                    drop(e);
                }
            }
        }
    }
}

/// A new request (`zend_weakrefs_init`).
pub fn reset() {
    let _ = REGISTRY.try_with(|r| {
        if let Ok(mut r) = r.try_borrow_mut() {
            *r = FxHashMap::default();
        }
    });
}

// --- WeakReference -----------------------------------------------------------

/// A WeakReference's payload (`zend_weakref`).
struct RefState {
    referent: RefCell<Option<(u32, Referent)>>,
    /// The WeakReference object itself, for `create()` to hand out again.
    this: Weak<RefCell<ObjectData>>,
}

impl Drop for RefState {
    /// `zend_weakref_free`.
    fn drop(&mut self) {
        if let Some((id, _)) = self.referent.get_mut().take() {
            unregister(id, self as *const RefState as *const ());
        }
    }
}

/// `zend_weakref_find`: the live WeakReference of handle `id`.
fn find_weakref(id: u32) -> Option<ObjectHandle> {
    REGISTRY.with(|r| {
        r.borrow().get(&id)?.iter().find_map(|t| match t {
            Target::Ref(w) => w.upgrade()?.this.upgrade(),
            Target::Map(_) => None,
        })
    })
}

/// `__weakref($op, ...)`: WeakReference's methods.
fn fn_weakref(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = args.first().map(conv::to_string_bytes).unwrap_or_default();
    let arg = args.get(1).map(deref).unwrap_or(Value::Null);
    match &op[..] {
        b"create" => {
            let Some(id) = rhp_values::object_id(&arg) else {
                return Ok(Value::Null);
            };
            if let Some(h) = find_weakref(id) {
                return Ok(Value::Object(h));
            }
            let Some(Value::Object(h)) = host.instantiate(b"WeakReference", Vec::new()) else {
                return Err(Thrown::error("Error", "Cannot instantiate WeakReference"));
            };
            let Some((id, referent)) = Referent::of(&arg) else {
                return Ok(Value::Null);
            };
            let st = Rc::new(RefState {
                referent: RefCell::new(Some((id, referent))),
                this: Rc::downgrade(&h),
            });
            register(id, Target::Ref(Rc::downgrade(&st)));
            object_handlers::set_payload(&h, st);
            Ok(Value::Object(h))
        }
        b"get" => Ok(weakref_get(&arg).unwrap_or(Value::Null)),
        _ => Err(Thrown::error(
            "Error",
            "Direct instantiation of WeakReference is not allowed, use WeakReference::create instead",
        )),
    }
}

fn weakref_get(this: &Value) -> Option<Value> {
    let st = object_handlers::payload::<RefState>(this)?;
    let r = st.referent.borrow();
    r.as_ref()?.1.get()
}

/// `zend_weakref_get_debug_info`.
fn weakref_properties_for(
    _: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    if purpose != PropPurpose::Debug {
        return Ok(None);
    }
    let mut t = PhpArray::new();
    let v = weakref_get(&Value::Object(h.clone())).unwrap_or(Value::Null);
    t.insert(rhp_values::ArrayKey::from_bytes(b"object"), v);
    Ok(Some(t))
}

static WEAKREF_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(weakref_properties_for),
    clone_obj: CloneObj::None,
    ..ObjectHandlers::STD
};

// --- WeakMap -----------------------------------------------------------------

/// One WeakMap entry.
struct Entry {
    id: u32,
    key: Referent,
    value: Value,
    /// The value was made a reference by a write fetch (`$m[$o][] = …`):
    /// php unwraps such a reference again once nothing else holds it
    /// (ZVAL_UNREF after FETCH_DIM_W), unlike one made by `=&` or by
    /// `foreach` by reference.
    fetch_ref: bool,
}

impl Entry {
    fn normalize(&mut self) {
        if self.fetch_ref {
            if let Value::Ref(r) = &self.value {
                if Rc::strong_count(r) == 1 {
                    let v = r.borrow().clone();
                    self.value = v;
                    self.fetch_ref = false;
                }
            } else {
                self.fetch_ref = false;
            }
        }
    }

    /// The value's slot as a reference (php's ZVAL_MAKE_REF).
    fn make_ref(&mut self, fetch: bool) -> PhpRef {
        self.normalize();
        match &self.value {
            Value::Ref(r) => r.clone(),
            v => {
                let r = Rc::new(RefCell::new(v.clone()));
                self.value = Value::Ref(r.clone());
                self.fetch_ref = fetch;
                r
            }
        }
    }
}

/// The entries in insertion order. A sequence number stands for php's
/// bucket position: iterators keep one, so removing entries never moves
/// them and new entries are visited, as with a HashTable iterator.
#[derive(Default)]
struct MapInner {
    entries: BTreeMap<u64, Entry>,
    by_id: FxHashMap<u32, u64>,
    next_seq: u64,
}

impl MapInner {
    fn get_mut(&mut self, id: u32) -> Option<&mut Entry> {
        let seq = *self.by_id.get(&id)?;
        let e = self.entries.get_mut(&seq)?;
        e.normalize();
        Some(e)
    }

    fn remove(&mut self, id: u32) -> Option<Entry> {
        let seq = self.by_id.remove(&id)?;
        self.entries.remove(&seq)
    }

    fn insert(&mut self, e: Entry) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.by_id.insert(e.id, seq);
        self.entries.insert(seq, e);
    }

    /// The first entry at or after position `pos`.
    fn at(&mut self, pos: u64) -> Option<(u64, &mut Entry)> {
        let (seq, e) = self.entries.range_mut(pos..).next()?;
        e.normalize();
        Some((*seq, e))
    }
}

/// A WeakMap's payload (`zend_weakmap`).
#[derive(Default)]
struct MapState {
    inner: RefCell<MapInner>,
}

impl MapState {
    fn ptr(&self) -> *const () {
        self as *const MapState as *const ()
    }
}

impl Drop for MapState {
    /// `zend_weakmap_free_obj`: unregister every key, then free the values.
    fn drop(&mut self) {
        let inner = std::mem::take(self.inner.get_mut());
        let p = self.ptr();
        for e in inner.entries.values() {
            unregister(e.id, p);
        }
        drop(inner);
    }
}

/// The WeakMap `h`'s entries (made on first use).
fn map_state(h: &ObjectHandle) -> Rc<MapState> {
    if let Some(s) = object_handlers::payload_of::<MapState>(h) {
        return s;
    }
    let s = Rc::new(MapState::default());
    object_handlers::set_payload(h, s.clone());
    s
}

fn key_error() -> Thrown {
    Thrown::type_error("WeakMap key must be an object")
}

fn append_error() -> Thrown {
    Thrown::error("Error", "Cannot append to WeakMap")
}

/// The handle of a WeakMap key, or php's TypeError.
fn key_id(offset: &Value) -> Result<u32, Thrown> {
    match deref(offset) {
        v @ (Value::Object(_) | Value::Closure(_)) => Ok(rhp_values::object_id(&v).unwrap_or(0)),
        _ => Err(key_error()),
    }
}

/// `zend_weakmap_read_dimension`.
fn weakmap_read(
    host: &mut dyn Host,
    h: &ObjectHandle,
    offset: Option<&Value>,
    ty: DimType,
) -> Result<Option<Value>, Thrown> {
    let offset = offset.ok_or_else(append_error)?;
    let id = key_id(offset)?;
    let st = map_state(h);
    let mut inner = st.inner.borrow_mut();
    let Some(e) = inner.get_mut(id) else {
        drop(inner);
        if ty == DimType::Is {
            return Ok(None);
        }
        let key = deref(offset);
        let class = host.object_class_name(&key).unwrap_or_default();
        return Err(Thrown::error(
            "Error",
            format!(
                "Object {}#{} not contained in WeakMap",
                String::from_utf8_lossy(&class),
                id
            ),
        ));
    };
    Ok(Some(match ty {
        DimType::W | DimType::Rw => Value::Ref(e.make_ref(true)),
        _ => deref(&e.value),
    }))
}

/// `zend_weakmap_write_dimension`. A `Value::Ref` is stored as it is
/// (`$m[$o] =& $v`).
fn weakmap_write(
    _: &mut dyn Host,
    h: &ObjectHandle,
    offset: Option<&Value>,
    value: Value,
) -> Result<(), Thrown> {
    let offset = offset.ok_or_else(append_error)?;
    let key = deref(offset);
    let id = key_id(&key)?;
    let st = map_state(h);
    let mut inner = st.inner.borrow_mut();
    if let Some(e) = inner.get_mut(id) {
        // The old value is freed after the new one is in place (its
        // destructor may change the map).
        let old = std::mem::replace(&mut e.value, value);
        e.fetch_ref = false;
        drop(inner);
        drop(old);
        return Ok(());
    }
    let Some((id, referent)) = Referent::of(&key) else {
        return Err(key_error());
    };
    register(id, Target::Map(Rc::downgrade(&st)));
    inner.insert(Entry {
        id,
        key: referent,
        value,
        fetch_ref: false,
    });
    Ok(())
}

/// `zend_weakmap_has_dimension`.
fn weakmap_has(
    host: &mut dyn Host,
    h: &ObjectHandle,
    offset: &Value,
    check_empty: bool,
) -> Result<bool, Thrown> {
    let id = key_id(offset)?;
    let st = map_state(h);
    let v = match st.inner.borrow_mut().get_mut(id) {
        Some(e) => deref(&e.value),
        None => return Ok(false),
    };
    if check_empty {
        return object_handlers::is_true(host, &v);
    }
    Ok(!matches!(v, Value::Null))
}

/// `zend_weakmap_unset_dimension`.
fn weakmap_unset(_: &mut dyn Host, h: &ObjectHandle, offset: &Value) -> Result<(), Thrown> {
    let id = key_id(offset)?;
    let st = map_state(h);
    let e = st.inner.borrow_mut().remove(id);
    if e.is_some() {
        unregister(id, st.ptr());
    }
    drop(e);
    Ok(())
}

/// `zend_weakmap_get_properties_for`: the entries as key/value pairs, for
/// debug output only.
fn weakmap_properties_for(
    _: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    if purpose != PropPurpose::Debug {
        return Ok(None);
    }
    let st = map_state(h);
    let mut inner = st.inner.borrow_mut();
    let mut t = PhpArray::new();
    for e in inner.entries.values_mut() {
        e.normalize();
        let Some(k) = e.key.get() else { continue };
        let mut pair = PhpArray::new();
        pair.insert(rhp_values::ArrayKey::from_bytes(b"key"), k);
        pair.insert(rhp_values::ArrayKey::from_bytes(b"value"), e.value.clone());
        t.push(Value::Array(pair));
    }
    Ok(Some(t))
}

/// `zend_weakmap_clone_obj`.
fn weakmap_clone(_: &mut dyn Host, src: &ObjectHandle, dst: &ObjectHandle) -> Result<(), Thrown> {
    let from = map_state(src);
    let to = map_state(dst);
    let from = from.inner.borrow();
    let mut inner = to.inner.borrow_mut();
    for e in from.entries.values() {
        register(e.id, Target::Map(Rc::downgrade(&to)));
        inner.insert(Entry {
            id: e.id,
            key: e.key.clone(),
            value: e.value.clone(),
            fetch_ref: false,
        });
    }
    Ok(())
}

/// A WeakMap iterator (`zend_weakmap_iterator`): the map and a position.
struct MapIter {
    map: ObjectHandle,
    pos: Cell<u64>,
}

impl MapIter {
    fn with<R>(&self, f: impl FnOnce(Option<(u64, &mut Entry)>) -> R) -> R {
        let st = map_state(&self.map);
        let mut inner = st.inner.borrow_mut();
        f(inner.at(self.pos.get()))
    }
}

impl IteratorFuncs for MapIter {
    fn valid(&self, _: &mut dyn Host) -> Result<bool, Thrown> {
        Ok(self.with(|e| e.is_some()))
    }

    fn current(&self, _: &mut dyn Host) -> Result<Option<Value>, Thrown> {
        Ok(self.with(|e| e.map(|(_, e)| e.value.clone())))
    }

    fn key(&self, _: &mut dyn Host) -> Result<Option<Value>, Thrown> {
        Ok(Some(
            self.with(|e| e.and_then(|(_, e)| e.key.get()))
                .unwrap_or(Value::Null),
        ))
    }

    fn move_forward(&self, _: &mut dyn Host) -> Result<(), Thrown> {
        if let Some(seq) = self.with(|e| e.map(|(s, _)| s)) {
            self.pos.set(seq + 1);
        }
        Ok(())
    }

    fn rewind(&self, _: &mut dyn Host) -> Result<(), Thrown> {
        self.pos.set(0);
        Ok(())
    }

    fn current_ref(&self, _: &mut dyn Host) -> Result<Option<PhpRef>, Thrown> {
        Ok(self.with(|e| e.map(|(_, e)| e.make_ref(false))))
    }
}

/// `zend_weakmap_get_iterator`.
fn weakmap_get_iterator(
    host: &mut dyn Host,
    h: &ObjectHandle,
    _by_ref: bool,
) -> Result<ObjectHandle, Thrown> {
    internal_iterator::create(
        host,
        Box::new(MapIter {
            map: h.clone(),
            pos: Cell::new(0),
        }),
    )
}

static WEAKMAP_DIMENSION: DimensionHandlers = DimensionHandlers {
    read: weakmap_read,
    write: weakmap_write,
    has: weakmap_has,
    unset: weakmap_unset,
};

static WEAKMAP_HANDLERS: ObjectHandlers = ObjectHandlers {
    get_properties_for: Some(weakmap_properties_for),
    clone_obj: CloneObj::Custom(weakmap_clone),
    dimension: Some(&WEAKMAP_DIMENSION),
    get_iterator: Some(weakmap_get_iterator),
    ..ObjectHandlers::STD
};

/// `__weakmap($op, $this, ...)`: WeakMap's methods.
fn fn_weakmap(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = args.first().map(conv::to_string_bytes).unwrap_or_default();
    let Some(h) = args.get(1).and_then(object_handlers::object_of) else {
        return Ok(Value::Null);
    };
    let key = args.get(2).map(deref).unwrap_or(Value::Null);
    match &op[..] {
        b"get" => Ok(weakmap_read(host, &h, Some(&key), DimType::R)?.unwrap_or(Value::Null)),
        b"set" => {
            let v = args.get(3).map(deref).unwrap_or(Value::Null);
            weakmap_write(host, &h, Some(&key), v)?;
            Ok(Value::Null)
        }
        b"exists" => Ok(Value::Bool(weakmap_has(host, &h, &key, false)?)),
        b"unset" => {
            weakmap_unset(host, &h, &key)?;
            Ok(Value::Null)
        }
        b"count" => Ok(Value::Int(map_state(&h).inner.borrow().entries.len() as i64)),
        _ => Ok(Value::Object(weakmap_get_iterator(host, &h, false)?)),
    }
}

pub(crate) static CLASSES: &[ClassHandlers] = &[
    ClassHandlers {
        class: "WeakReference",
        handlers: &WEAKREF_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "WeakMap",
        handlers: &WEAKMAP_HANDLERS,
        create_object: true,
    },
];

pub fn register_functions(m: &mut crate::Registry) {
    crate::register_helpers(
        m,
        &[
            ("__weakref", fn_weakref, 1, Some(2)),
            ("__weakmap", fn_weakmap, 2, Some(4)),
        ],
    );
}
