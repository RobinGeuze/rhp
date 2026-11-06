//! Lazy objects (Zend/zend_lazy_objects.c): ghosts, initialized in place by
//! an initializer, and proxies, whose factory returns the real instance
//! that later accesses are forwarded to.
//!
//! The state lives in `ObjectData::lazy` (see [`LazyInfo`]). A lazy
//! object's declared slots are absent from `properties`, so property
//! accesses reach the absent-slot branches of `props.rs`/`lvalue.rs`, and
//! only there check for laziness (php's IS_PROP_LAZY slot flag): ordinary
//! objects pay nothing on the hit path.

use std::cell::RefCell;
use std::rc::Rc;

use indexmap::IndexMap;
use rhp_values::{ClassId, ClassKind, LazyInfo, ObjectData, ObjectHandle, PhpStr, Thrown, Value};
use rustc_hash::FxBuildHasher;

use crate::{classes, deref, Vm};

fn cname(vm: &Vm, cls: ClassId) -> String {
    String::from_utf8_lossy(vm.class(cls).name.as_bytes()).into_owned()
}

impl Vm {
    /// Whether an absent property reached on `h` triggers initialization
    /// (zend_lazy_object_must_init plus the slot's IS_PROP_LAZY flag):
    /// `key` is the declared slot, `None` for a dynamic property.
    #[inline]
    pub(crate) fn lazy_slot(&self, h: &ObjectHandle, key: Option<&PhpStr>) -> bool {
        let b = h.borrow();
        let Some(l) = b.lazy() else {
            return false;
        };
        match l.obj_flags {
            0 => false,
            // An initialized proxy: every slot forwards.
            LazyInfo::PROXY => true,
            _ => key.is_none_or(|k| l.lazy_props.contains(k)),
        }
    }

    /// php's zend_class_can_be_lazy, with its errors: internal classes and
    /// their subclasses cannot be lazy, except stdClass.
    pub(crate) fn lazy_class_check(&self, cls: ClassId) -> Result<(), Thrown> {
        let internal = |c: ClassId| {
            (c as usize) < self.internal_classes
                && !self
                    .class(c)
                    .name
                    .as_bytes()
                    .eq_ignore_ascii_case(b"stdClass")
        };
        if internal(cls) {
            return Err(Thrown::error(
                "Error",
                format!(
                    "Cannot make instance of internal class lazy: {} is internal",
                    cname(self, cls)
                ),
            ));
        }
        let mut p = self.class(cls).parent;
        while let Some(c) = p {
            if internal(c) {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Cannot make instance of internal class lazy: {} inherits internal class {}",
                        cname(self, cls),
                        cname(self, c)
                    ),
                ));
            }
            p = self.class(c).parent;
        }
        Ok(())
    }

    /// zend_object_make_lazy: a new lazy instance of `cls` (`obj` None), or
    /// `obj` reset as lazy. Returns the object.
    pub(crate) fn make_lazy(
        &mut self,
        obj: Option<ObjectHandle>,
        cls: ClassId,
        initializer: Value,
        flags: u8,
    ) -> Result<ObjectHandle, Thrown> {
        self.lazy_class_check(cls)?;
        let mut lazy_props = Vec::new();
        let h = match obj {
            None => {
                let e = self.class(cls);
                if e.kind != ClassKind::Class || e.flags.is_abstract {
                    let what = match e.kind {
                        ClassKind::Interface => "interface",
                        ClassKind::Trait => "trait",
                        ClassKind::Enum => "enum",
                        ClassKind::Class => "abstract class",
                    };
                    return Err(Thrown::error(
                        "Error",
                        format!("Cannot instantiate {what} {}", cname(self, cls)),
                    ));
                }
                self.update_class_constants(cls)?;
                let id = self.alloc_object_id();
                let props: IndexMap<PhpStr, Value, FxBuildHasher> =
                    IndexMap::with_hasher(FxBuildHasher);
                let h: ObjectHandle = Rc::new(RefCell::new(ObjectData::new(cls, id, props)));
                self.track_destructible(&h);
                for (k, pe) in &self.classes[cls as usize].props {
                    if !pe.virt {
                        lazy_props.push(k.clone());
                    }
                }
                h
            }
            Some(h) => {
                let (is_lazy, has_info) = {
                    let b = h.borrow();
                    (b.is_lazy(), b.lazy().is_some())
                };
                if is_lazy {
                    // An initialized proxy: drop its instance.
                    let old = h.borrow_mut().take_lazy();
                    drop(old);
                    // php releases the instance here: its destructor runs
                    // now, while the object is not lazy.
                    self.collect_released_now()?;
                } else {
                    if has_info {
                        return Err(Thrown::error(
                            "Error",
                            "Can not reset an object while it is being initialized",
                        ));
                    }
                    if flags & LazyInfo::SKIP_DESTRUCTOR == 0 {
                        if let Some(i) = self.destructibles.iter().position(|d| Rc::ptr_eq(d, &h)) {
                            let d = self.destructibles.remove(i);
                            self.call_destructor(d)?;
                        }
                    }
                }
                // php clears IS_OBJ_DESTRUCTOR_CALLED: the destructor may run
                // again.
                if !self.destructibles.iter().any(|d| Rc::ptr_eq(d, &h)) {
                    self.track_destructible(&h);
                }
                let ocls = h.borrow().class;
                let oid = h.borrow().id;
                let final_cls = self.class(ocls).flags.is_final;
                let reset: Vec<PhpStr> = self.classes[cls as usize]
                    .props
                    .iter()
                    .filter(|(_, pe)| !pe.virt)
                    .map(|(k, _)| k.clone())
                    .collect();
                let mut removed = Vec::new();
                {
                    let mut b = h.borrow_mut();
                    let ocls_e = &self.classes[ocls as usize];
                    // Dynamic properties go.
                    let keys: Vec<PhpStr> = b
                        .properties
                        .keys()
                        .filter(|k| !ocls_e.props.contains_key(*k))
                        .cloned()
                        .collect();
                    for k in keys {
                        removed.extend(b.remove_prop(&k));
                    }
                    for k in reset {
                        let present = b.properties.contains_key(&k);
                        if present {
                            let keep = ocls_e
                                .props
                                .get(&k)
                                .is_some_and(|pe| pe.readonly && (final_cls || pe.is_final));
                            if keep {
                                continue;
                            }
                            removed.extend(b.remove_prop(&k));
                        }
                        lazy_props.push(k);
                    }
                }
                if let Some(ks) = self.unset_typed_props.get_mut(&oid) {
                    ks.retain(|k| !lazy_props.contains(k));
                }
                drop(removed);
                h
            }
        };
        // No lazy slot: the object is not lazy at all.
        if lazy_props.is_empty() {
            return Ok(h);
        }
        let mut obj_flags = LazyInfo::UNINIT;
        if flags & LazyInfo::STRATEGY_PROXY != 0 {
            obj_flags |= LazyInfo::PROXY;
        }
        let old = h.borrow_mut().set_lazy(LazyInfo {
            obj_flags,
            flags,
            initializer,
            instance: None,
            lazy_props,
        });
        drop(old);
        Ok(h)
    }

    /// Fill the slots `keys` of `h` with their declared defaults (a typed
    /// property without one stays uninitialized).
    fn lazy_fill_defaults(&mut self, h: &ObjectHandle, keys: &[PhpStr]) {
        let cls = h.borrow().class;
        let mut b = h.borrow_mut();
        for k in keys {
            let Some(pe) = self.classes[cls as usize].props.get(k) else {
                continue;
            };
            if pe.starts_uninit() || pe.virt {
                continue;
            }
            let v = pe.default.clone();
            *classes::prop_slot_mut(&self.classes, &mut b, k.clone()) = v;
        }
    }

    /// zend_lazy_object_init: initialize `h` (an initialized proxy returns
    /// its instance). Returns the object accesses go to.
    pub(crate) fn lazy_init(&mut self, h: &ObjectHandle) -> Result<ObjectHandle, Thrown> {
        let (obj_flags, inst) = {
            let b = h.borrow();
            match b.lazy() {
                Some(l) => (l.obj_flags, l.instance.clone()),
                None => return Ok(h.clone()),
            }
        };
        if obj_flags == 0 {
            return Ok(h.clone());
        }
        if obj_flags == LazyInfo::PROXY {
            let inst = inst.expect("initialized proxy");
            if inst.borrow().is_lazy() {
                return self.lazy_init(&inst);
            }
            return Ok(inst);
        }
        if obj_flags & LazyInfo::PROXY != 0 {
            return self.lazy_init_proxy(h);
        }
        // Ghost: prevent re-entrant initialization, snapshot the state, give
        // the lazy slots their defaults and run the initializer.
        let id = h.borrow().id;
        let (init, lazy) = {
            let mut b = h.borrow_mut();
            let l = b.lazy_mut().expect("lazy");
            l.obj_flags &= !LazyInfo::UNINIT;
            (l.initializer.clone(), std::mem::take(&mut l.lazy_props))
        };
        let snapshot = h.borrow().properties.clone();
        let unset_snapshot = self.unset_typed_props.get(&id).cloned();
        self.lazy_fill_defaults(h, &lazy);
        let r = self.call_callable_value(&init, vec![Value::Object(h.clone())]);
        let err = match r {
            Err(e) => Some(e),
            Ok(v) if !matches!(deref(&v), Value::Null) => {
                drop(v);
                Some(Thrown::type_error(
                    "Lazy object initializer must return NULL or no value",
                ))
            }
            Ok(_) => None,
        };
        if let Some(e) = err {
            // zend_lazy_object_revert_init.
            let old = std::mem::replace(&mut h.borrow_mut().properties, snapshot);
            match unset_snapshot {
                Some(ks) => {
                    self.unset_typed_props.insert(id, ks);
                }
                None => {
                    self.unset_typed_props.remove(&id);
                }
            }
            if let Some(l) = h.borrow_mut().lazy_mut() {
                l.lazy_props = lazy;
                l.obj_flags |= LazyInfo::UNINIT;
            }
            drop(old);
            return Err(e);
        }
        drop(snapshot);
        let info = h.borrow_mut().take_lazy();
        drop(info);
        Ok(h.clone())
    }

    /// zend_lazy_object_init_proxy.
    fn lazy_init_proxy(&mut self, h: &ObjectHandle) -> Result<ObjectHandle, Thrown> {
        let factory = {
            let mut b = h.borrow_mut();
            let l = b.lazy_mut().expect("lazy");
            l.obj_flags = 0;
            l.initializer.clone()
        };
        let restore = |h: &ObjectHandle| {
            if let Some(l) = h.borrow_mut().lazy_mut() {
                l.obj_flags = LazyInfo::UNINIT | LazyInfo::PROXY;
            }
        };
        let r = match self.call_callable_value(&factory, vec![Value::Object(h.clone())]) {
            Ok(v) => deref(&v),
            Err(e) => {
                restore(h);
                return Err(e);
            }
        };
        let cls = h.borrow().class;
        let Value::Object(inst) = r else {
            restore(h);
            return Err(Thrown::type_error(format!(
                "Lazy proxy factory must return an instance of a class compatible with {}, {} returned",
                cname(self, cls),
                rhp_builtins::zval_value_name(self, &r)
            )));
        };
        let icls = inst.borrow().class;
        if !self.lazy_compatible(icls, cls) {
            restore(h);
            return Err(Thrown::type_error(format!(
                "The real instance class {} is not compatible with the proxy class {}. The proxy must be a instance of the same class as the real instance, or a sub-class with no additional properties, and no overrides of the __destructor or __clone methods.",
                cname(self, icls),
                cname(self, cls)
            )));
        }
        if Rc::ptr_eq(&inst, h) || inst.borrow().is_lazy() {
            restore(h);
            return Err(Thrown::error(
                "Error",
                "Lazy proxy factory must return a non-lazy object",
            ));
        }
        let (old_init, old_props) = {
            let mut b = h.borrow_mut();
            let l = b.lazy_mut().expect("lazy");
            let old = std::mem::replace(&mut l.initializer, Value::Null);
            l.instance = Some(inst.clone());
            l.flags |= LazyInfo::INITIALIZED;
            l.obj_flags = LazyInfo::PROXY;
            l.lazy_props.clear();
            (old, std::mem::take(&mut b.properties))
        };
        let id = h.borrow().id;
        self.unset_typed_props.remove(&id);
        drop(old_props);
        drop(old_init);
        Ok(inst)
    }

    /// zend_lazy_object_compatible: the real instance's class is the
    /// proxy's, or a parent with the same properties, destructor and clone.
    fn lazy_compatible(&self, real: ClassId, proxy: ClassId) -> bool {
        if real == proxy {
            return true;
        }
        if !self.class(proxy).ancestors.contains(&real) {
            return false;
        }
        let (r, p) = (self.class(real), self.class(proxy));
        if r.props.len() + r.static_props.len() != p.props.len() + p.static_props.len() {
            return false;
        }
        let same = |m: &[u8]| {
            let a = self.lookup_method(real, m).map(|m| m.proto);
            let b = self.lookup_method(proxy, m).map(|m| m.proto);
            match (a, b) {
                (None, None) => true,
                (Some(a), Some(b)) => Rc::ptr_eq(&a, &b),
                _ => false,
            }
        };
        same(b"__destruct") && same(b"__clone")
    }

    /// zend_lazy_object_mark_as_initialized: the lazy slots take their
    /// defaults, the initializer is not called.
    pub(crate) fn lazy_mark_initialized(&mut self, h: &ObjectHandle) {
        let lazy = {
            let mut b = h.borrow_mut();
            match b.lazy_mut() {
                Some(l) => {
                    l.obj_flags = 0;
                    std::mem::take(&mut l.lazy_props)
                }
                None => return,
            }
        };
        self.lazy_fill_defaults(h, &lazy);
        let info = h.borrow_mut().take_lazy();
        drop(info);
    }

    /// The slot `key` of the uninitialized lazy `h` is no longer lazy
    /// (skipLazyInitialization / setRawValueWithoutLazyInitialization).
    /// Returns whether it was. The object stops being lazy with its last
    /// lazy slot (zend_lazy_object_realize).
    pub(crate) fn lazy_unmark_slot(&mut self, h: &ObjectHandle, key: &PhpStr) -> bool {
        let mut b = h.borrow_mut();
        let Some(l) = b.lazy_mut() else {
            return false;
        };
        let Some(i) = l.lazy_props.iter().position(|k| k == key) else {
            return false;
        };
        l.lazy_props.remove(i);
        if l.lazy_props.is_empty() && l.obj_flags & LazyInfo::UNINIT != 0 {
            let info = b.take_lazy();
            drop(b);
            drop(info);
        }
        true
    }

    /// Mark the slot `key` lazy again (an exception prevented the update).
    pub(crate) fn lazy_remark_slot(&mut self, h: &ObjectHandle, key: &PhpStr) {
        let mut b = h.borrow_mut();
        if let Some(l) = b.lazy_mut() {
            if !l.lazy_props.contains(key) {
                l.lazy_props.push(key.clone());
            }
        }
    }

    /// php's get_properties on a lazy object (zend_lazy_object_get_properties):
    /// initialize it and return the object whose properties to use.
    #[inline]
    pub(crate) fn lazy_props_object(&mut self, h: &ObjectHandle) -> Result<ObjectHandle, Thrown> {
        if !h.borrow().is_lazy() {
            return Ok(h.clone());
        }
        self.lazy_init(h)
    }

    /// The object an initialized proxy forwards to (else `h`).
    #[inline]
    pub(crate) fn lazy_target(&self, h: &ObjectHandle) -> ObjectHandle {
        rhp_values::lazy_target(h)
    }

    /// The `__reflect('lazy_*', ...)` ops behind ReflectionClass's and
    /// ReflectionProperty's lazy object methods (ext/reflection).
    pub(crate) fn lazy_reflect_op(&mut self, op: &[u8], args: &[Value]) -> Result<Value, Thrown> {
        let arg = |i: usize| args.get(i).map(deref).unwrap_or(Value::Null);
        let obj = |i: usize| match arg(i) {
            Value::Object(h) => Ok(h),
            _ => Err(Thrown::type_error("object expected")),
        };
        match op {
            // (class, object|null, initializer, flags)
            b"lazy_make" => {
                let name = rhp_values::conv::to_string_bytes(&arg(0));
                let cls = self.resolve_class_by_name(&name)?;
                let o = match arg(1) {
                    Value::Object(h) => Some(h),
                    _ => None,
                };
                let flags = rhp_values::conv::to_int(&arg(3)) as u8;
                let h = self.make_lazy(o, cls, arg(2), flags)?;
                Ok(Value::Object(h))
            }
            // 0: not lazy, 1: uninitialized, 2: initialized proxy
            b"lazy_state" => {
                let h = obj(0)?;
                let b = h.borrow();
                Ok(Value::Int(if b.is_lazy_uninit() {
                    1
                } else if b.is_lazy() {
                    2
                } else {
                    0
                }))
            }
            b"lazy_init" | b"lazy_mark" => {
                let h = obj(0)?;
                if h.borrow().is_lazy_uninit() {
                    if op == b"lazy_init" {
                        self.lazy_init(&h)?;
                    } else {
                        self.lazy_mark_initialized(&h);
                    }
                }
                if h.borrow().is_lazy_uninit() {
                    return Err(Thrown::error("Error", "Lazy object initialization failed"));
                }
                Ok(Value::Object(self.lazy_target_once(&h)))
            }
            b"lazy_initializer" => {
                let h = obj(0)?;
                let b = h.borrow();
                Ok(match b.lazy() {
                    Some(l) if l.obj_flags & LazyInfo::UNINIT != 0 => l.initializer.clone(),
                    _ => Value::Null,
                })
            }
            // (method, object, class, name[, value])
            b"lazy_prop" => self.lazy_prop_op(
                &rhp_values::conv::to_string_bytes(&arg(0)),
                obj(1)?,
                &rhp_values::conv::to_string_bytes(&arg(2)),
                PhpStr::from_bytes(rhp_values::conv::to_string_bytes(&arg(3))),
                arg(4),
            ),
            _ => Err(Thrown::error(
                "Error",
                format!("unknown reflection op {}", String::from_utf8_lossy(op)),
            )),
        }
    }

    /// zend_lazy_object_get_instance: an initialized proxy's instance (one
    /// level), else the object.
    fn lazy_target_once(&self, h: &ObjectHandle) -> ObjectHandle {
        h.borrow().lazy_instance().unwrap_or_else(|| h.clone())
    }

    /// ReflectionProperty::skipLazyInitialization (`skip`),
    /// setRawValueWithoutLazyInitialization (`set`) and isLazy (`is`).
    fn lazy_prop_op(
        &mut self,
        method: &[u8],
        h: ObjectHandle,
        class: &[u8],
        name: PhpStr,
        value: Value,
    ) -> Result<Value, Thrown> {
        let h = self.lazy_target(&h);
        let rcls = self.resolve_class_by_name(class)?;
        let ocls = h.borrow().class;
        // reflection_property_get_effective_prop: a non-private property
        // as the object's class redeclares it.
        let key = self.decl_prop_key(rcls, &name);
        let pe = key.as_ref().and_then(|k| {
            let own = self.class(rcls).props.get(k)?;
            if own.visibility == rhp_values::Visibility::Private {
                Some(own.clone())
            } else {
                self.class(ocls).props.get(k).or(Some(own)).cloned()
            }
        });
        let is_static =
            key.is_none() && self.class(rcls).static_props.contains_key(name.as_bytes());
        if method == b"is" {
            return Ok(Value::Bool(match (&key, &pe) {
                (Some(k), Some(pe)) if !pe.virt => {
                    let b = h.borrow();
                    b.lazy().is_some_and(|l| l.lazy_props.contains(k))
                }
                _ => false,
            }));
        }
        let what = if method == b"skip" {
            "skipLazyInitialization"
        } else {
            "setRawValueWithoutLazyInitialization"
        };
        let n = String::from_utf8_lossy(name.as_bytes()).into_owned();
        if is_static {
            let decl = self
                .class(rcls)
                .static_props
                .get(name.as_bytes())
                .map(|sp| sp.declaring_class)
                .unwrap_or(rcls);
            return Err(Thrown::error(
                "ReflectionException",
                format!(
                    "Can not use {what} on static property {}::${n}",
                    cname(self, decl)
                ),
            ));
        }
        let (Some(key), Some(pe)) = (key, pe) else {
            return Err(Thrown::error(
                "ReflectionException",
                format!(
                    "Can not use {what} on dynamic property {}::${n}",
                    cname(self, rcls)
                ),
            ));
        };
        if pe.virt {
            return Err(Thrown::error(
                "ReflectionException",
                format!(
                    "Can not use {what} on virtual property {}::${n}",
                    cname(self, pe.declaring_class)
                ),
            ));
        }
        let was_lazy = {
            let b = h.borrow();
            b.lazy().is_some_and(|l| l.lazy_props.contains(&key))
        };
        if method == b"skip" {
            if was_lazy {
                self.lazy_fill_defaults(&h, std::slice::from_ref(&key));
                self.lazy_unmark_slot(&h, &key);
            }
            return Ok(Value::Null);
        }
        // setRawValueWithoutLazyInitialization: a raw write (no hooks, no
        // __set) to the no longer lazy slot.
        if was_lazy {
            let mut b = h.borrow_mut();
            if let Some(l) = b.lazy_mut() {
                l.lazy_props.retain(|k| *k != key);
            }
        }
        let r = self.lazy_raw_write(&h, ocls, &key, rcls, value);
        let present = h.borrow().properties.contains_key(&key);
        if r.is_err() && was_lazy && !present && h.borrow().is_lazy_uninit() {
            self.lazy_remark_slot(&h, &key);
        } else if was_lazy {
            let done = {
                let b = h.borrow();
                b.lazy()
                    .is_some_and(|l| l.obj_flags & LazyInfo::UNINIT != 0 && l.lazy_props.is_empty())
            };
            if done {
                let info = h.borrow_mut().take_lazy();
                drop(info);
            }
        }
        r.map(|()| Value::Null)
    }

    /// A write of `val` to the declared slot `key`, as from `scope`,
    /// bypassing hooks and `__set` (the slot counts as uninitialized).
    fn lazy_raw_write(
        &mut self,
        h: &ObjectHandle,
        cls: ClassId,
        key: &PhpStr,
        scope: ClassId,
        val: Value,
    ) -> Result<(), Thrown> {
        let present = h.borrow().properties.contains_key(key);
        self.check_readonly_write(h, key, present, Some(scope), Some(scope), "modify")?;
        let val = self.verify_prop_type(cls, key, val, false)?;
        let old = {
            let mut b = h.borrow_mut();
            let slot = classes::prop_slot_mut(&self.classes, &mut b, key.clone());
            match slot {
                Value::Ref(rc) => {
                    let rc = rc.clone();
                    drop(b);
                    let old = std::mem::replace(&mut *rc.borrow_mut(), val);
                    old
                }
                other => std::mem::replace(other, val),
            }
        };
        drop(old);
        Ok(())
    }

    /// get_properties_for(`purpose`) of a lazy object: initialize it
    /// (serialize with SKIP_INITIALIZATION_ON_SERIALIZE does not); a proxy
    /// shows its instance's table.
    pub(crate) fn lazy_properties_for(
        &mut self,
        h: &ObjectHandle,
        purpose: rhp_builtins::zend::object_handlers::PropPurpose,
    ) -> Result<Option<rhp_values::PhpArray>, Thrown> {
        use rhp_builtins::zend::object_handlers::PropPurpose as P;
        // (array) never initializes (zend_get_properties_no_lazy_init).
        let skip = purpose == P::ArrayCast
            || purpose == P::Serialize && {
                let b = h.borrow();
                b.lazy()
                    .is_some_and(|l| l.flags & LazyInfo::SKIP_INITIALIZATION_ON_SERIALIZE != 0)
            };
        let inst = if skip {
            self.lazy_target(h)
        } else {
            self.lazy_init(h)?
        };
        if Rc::ptr_eq(&inst, h) {
            return self.hooked_view(h, purpose);
        }
        if let Some(v) = rhp_builtins::zend::object_handlers::properties_for(self, &inst, purpose)?
        {
            return Ok(Some(v));
        }
        Ok(Some(match purpose {
            P::GetObjectVars => match rhp_builtins::Host::object_vars(self, &Value::Object(inst)) {
                Some(Value::Array(a)) => a,
                _ => rhp_values::PhpArray::new(),
            },
            _ => match self.object_to_array_mangled(&inst) {
                Value::Array(a) => a,
                _ => rhp_values::PhpArray::new(),
            },
        }))
    }

    /// zend_lazy_object_clone: initialize, then clone. A ghost clones as
    /// usual (the caller runs `__clone`); a proxy's clone is a new proxy
    /// for a clone of the instance (whose `__clone` runs here), and the
    /// proxy's own `__clone` does not run.
    pub(crate) fn lazy_clone(&mut self, h: &ObjectHandle) -> Result<ObjectHandle, Thrown> {
        if h.borrow().is_lazy_uninit() {
            self.lazy_init(h)?;
        }
        let Some(inst) = h.borrow().lazy_instance() else {
            return self.clone_members(h);
        };
        let (cls, flags) = {
            let b = h.borrow();
            (b.class, b.lazy().map_or(0, |l| l.flags))
        };
        // The new proxy is allocated before the instance's clone, as in php
        // (object ids).
        let id = self.alloc_object_id();
        let props: IndexMap<PhpStr, Value, FxBuildHasher> = IndexMap::with_hasher(FxBuildHasher);
        let p: ObjectHandle = Rc::new(RefCell::new(ObjectData::new(cls, id, props)));
        let new_inst = self.clone_members(&inst)?;
        let icls = new_inst.borrow().class;
        if let Some(m) = self.lookup_method(icls, b"__clone") {
            self.call_sync(&m, Some(new_inst.clone()), icls, Vec::new())?;
        }
        p.borrow_mut().set_lazy(LazyInfo {
            obj_flags: LazyInfo::PROXY,
            flags,
            initializer: Value::Null,
            instance: Some(new_inst),
            lazy_props: Vec::new(),
        });
        self.track_destructible(&p);
        Ok(p)
    }
}
