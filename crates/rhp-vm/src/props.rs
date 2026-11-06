//! Object properties: fetch, store, isset/unset, magic accessors, typed and
//! readonly slots, and `clone`.

use std::cell::RefCell;
use std::rc::Rc;

use rhp_builtins as builtins;
use rhp_bytecode::Reg;
use rhp_values::{
    ArrayKey, ClassId, ClassKind, ObjectData, ObjectHandle, PhpArray, PhpStr, Thrown, Value,
};

use crate::calls::is_handler_accessor;
use crate::classes::{Deny, PropRes};
use crate::{classes, deref, Vm};

impl Vm {
    /// Whether `v` is an instance of the named class/interface (or a Closure for
    /// the `Closure` type). Unresolvable target class → lenient `true` (the class
    /// may be autoloadable; we don't reject on a name we can't see).
    /// Read property `pname` off `o` (visibility-checked, `__get` fallback).
    pub(crate) fn prop_fetch(
        &mut self,
        top: usize,
        o: Value,
        pname: PhpStr,
    ) -> Result<Value, Thrown> {
        let scope = self.stack[top].defining_class;
        self.prop_fetch_as(o, pname, scope)
    }

    /// [`Vm::prop_fetch`] from an explicit scope (php's `EG(fake_scope)`,
    /// as `ReflectionProperty::getValue` reads).
    pub(crate) fn prop_fetch_as(
        &mut self,
        o: Value,
        pname: PhpStr,
        scope: Option<ClassId>,
    ) -> Result<Value, Thrown> {
        Ok(match o {
            Value::Object(h) => {
                let cls = h.borrow().class;
                let mut absent_slot = None;
                let denied = match classes::resolve_prop(&self.classes, cls, scope, &pname) {
                    PropRes::Slot(k) => {
                        if self.classes[cls as usize].hooked {
                            if let Some(r) = self.hooked_read(&h, cls, &k) {
                                return r.map(|v| deref(&v));
                            }
                        }
                        if let Some(v) = h.borrow().properties.get(&k) {
                            return Ok(deref(v));
                        }
                        // A lazy slot skips __get() and initializes.
                        if self.lazy_slot(&h, Some(&k)) {
                            let inst = self.lazy_init(&h)?;
                            return self.prop_fetch_as(Value::Object(inst), pname, scope);
                        }
                        // Skip __get() for an uninitialized typed property.
                        if self.uninit_typed(&h, &k) {
                            if let Some(e) = self.typed_slot(cls, &k) {
                                return Err(e);
                            }
                        }
                        absent_slot = Some(k);
                        None
                    }
                    PropRes::Dynamic => {
                        self.static_as_instance_notice(cls, scope, &pname, b"__get")?;
                        if Some(cls) == self.incomplete_class {
                            if self.pending_magic_fetch == builtins::MagicFetch::Write {
                                return Err(self.incomplete_error(&h, "modify a property"));
                            }
                            self.incomplete_warning(&h)?;
                            return Ok(Value::Null);
                        }
                        if let Some(v) = h.borrow().properties.get(&pname) {
                            return Ok(deref(v));
                        }
                        None
                    }
                    PropRes::Denied(vis) => Some(vis),
                };
                if let Some(inst) = self.proxy_guarded(&h, b"__get", &pname) {
                    return self.prop_fetch_as(Value::Object(inst), pname, scope);
                }
                match self.magic(&h, cls, b"__get", &pname) {
                    Some(getter) => {
                        let write = self.pending_magic_fetch == builtins::MagicFetch::Write
                            && !getter.proto.is_internal();
                        let arg = Value::str(pname.as_bytes().to_vec());
                        let r = self.call_magic(&getter, &h, cls, b"__get", &pname, vec![arg])?;
                        if write && !matches!(r, Value::Ref(_) | Value::Object(_)) {
                            let msg = format!(
                                "Indirect modification of overloaded property {}::${} has no effect",
                                String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                                String::from_utf8_lossy(pname.as_bytes())
                            );
                            self.diag(builtins::E_NOTICE, &msg)?;
                        }
                        // (A built-in stand-in accessor is its class's own
                        // read_property handler: no check.)
                        match absent_slot {
                            Some(k)
                                if !getter.proto.is_internal()
                                    && self
                                        .class(cls)
                                        .props
                                        .get(&k)
                                        .is_some_and(|pe| pe.ty.is_some()) =>
                            {
                                let strict = getter.proto.strict_types;
                                self.verify_magic_get(cls, &k, r, strict)?
                            }
                            _ => r,
                        }
                    }
                    None if denied.is_none() && self.lazy_slot(&h, absent_slot.as_ref()) => {
                        let inst = self.lazy_init(&h)?;
                        let guarded = self.lookup_method(cls, b"__get").is_some();
                        return self.with_guard(&inst, guarded, b"__get", &pname, |vm| {
                            vm.prop_fetch_as(Value::Object(inst.clone()), pname.clone(), scope)
                        });
                    }
                    None => match denied {
                        Some(vis) => {
                            return Err(classes::prop_access_err(
                                &self.classes,
                                cls,
                                pname.as_bytes(),
                                vis,
                            ))
                        }
                        None => {
                            if let Some(e) =
                                absent_slot.as_ref().and_then(|k| self.typed_slot(cls, k))
                            {
                                return Err(e);
                            }
                            let msg = format!(
                                "Undefined property: {}::${}",
                                String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                                String::from_utf8_lossy(pname.as_bytes())
                            );
                            self.diag(builtins::E_WARNING, &msg)?;
                            Value::Null
                        }
                    },
                }
            }
            Value::Closure(_) => {
                let msg = format!(
                    "Undefined property: Closure::${}",
                    String::from_utf8_lossy(pname.as_bytes())
                );
                self.diag(builtins::E_WARNING, &msg)?;
                Value::Null
            }
            other => {
                let msg = format!(
                    "Attempt to read property \"{}\" on {}",
                    String::from_utf8_lossy(pname.as_bytes()),
                    rhp_builtins::zval_value_name(self, &other)
                );
                self.diag(builtins::E_WARNING, &msg)?;
                Value::Null
            }
        })
    }

    /// php's FETCH_OBJ_W / FETCH_OBJ_UNSET (`unset`): the property below a
    /// nested write. A non-object container is "Attempt to modify property"
    /// (null for an unset); an absent dynamic property without `__get` is
    /// created as null (zend_std_get_property_ptr_ptr).
    pub(crate) fn prop_fetch_w(
        &mut self,
        top: usize,
        o: Value,
        pname: PhpStr,
        unset: bool,
    ) -> Result<Value, Thrown> {
        let o = deref(&o);
        match &o {
            Value::Object(h) => {
                let cls = h.borrow().class;
                let scope = self.stack[top].defining_class;
                // A readonly / set-restricted property (zend_std_read_property
                // for BP_VAR_W/UNSET).
                if let PropRes::Slot(k) = classes::resolve_prop(&self.classes, cls, scope, &pname) {
                    if let Some(pe) = self.class(cls).props.get(&k) {
                        if pe.hooks.is_none()
                            && (pe.readonly
                                || (pe.set_visibility.is_some()
                                    && classes::set_access_ok(&self.classes, cls, scope, &k)
                                        .is_err()))
                        {
                            let v = h.borrow().properties.get(&k).map(deref);
                            match v {
                                Some(v @ Value::Object(_)) => return Ok(v),
                                None if unset => return Ok(Value::Null),
                                _ => return Err(self.indirect_modification_err(top, cls, &k)),
                            }
                        }
                    }
                }
                if Some(cls) != self.incomplete_class
                    && !h.borrow().is_lazy()
                    && matches!(
                        classes::resolve_prop(&self.classes, cls, scope, &pname),
                        PropRes::Dynamic
                    )
                    && !h.borrow().properties.contains_key(&pname)
                    && self.lookup_method(cls, b"__get").is_none()
                {
                    self.static_as_instance_notice(cls, scope, &pname, b"__get")?;
                    self.check_dynamic_prop(top, cls, &pname)?;
                    h.borrow_mut().properties.insert(pname, Value::Null);
                    return Ok(Value::Null);
                }
            }
            Value::Closure(_) => {}
            _ if unset => return Ok(Value::Null),
            other => {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Attempt to modify property \"{}\" on {}",
                        String::from_utf8_lossy(pname.as_bytes()),
                        rhp_builtins::zval_value_name(self, other)
                    ),
                ))
            }
        }
        self.fetching_as(builtins::MagicFetch::Write, |vm| {
            vm.prop_fetch(top, o, pname)
        })
    }

    /// "Cannot indirectly modify readonly property" / "Cannot indirectly
    /// modify private(set) property … from …" for the declared slot `key`.
    pub(crate) fn indirect_modification_err(
        &self,
        top: usize,
        cls: ClassId,
        key: &PhpStr,
    ) -> Thrown {
        let pe = &self.class(cls).props[key];
        if pe.readonly {
            return Thrown::error(
                "Error",
                format!(
                    "Cannot indirectly modify readonly property {}::${}",
                    String::from_utf8_lossy(self.class(pe.declaring_class).name.as_bytes()),
                    String::from_utf8_lossy(pe.name.as_bytes())
                ),
            );
        }
        let scope = self.stack[top].defining_class;
        match classes::set_access_ok(&self.classes, cls, scope, key) {
            Err(e) => classes::set_access_err(
                &self.classes,
                self.stack[top].called_class,
                "indirectly modify",
                e,
            ),
            Ok(()) => Thrown::error("Error", "Cannot indirectly modify property"),
        }
    }

    /// `isset($o->name)`: a set, non-null property reachable from the scope,
    /// else `__isset`, else false.
    pub(crate) fn prop_isset(
        &mut self,
        top: usize,
        o: Value,
        pname: PhpStr,
    ) -> Result<bool, Thrown> {
        let Value::Object(h) = o else {
            return Ok(false);
        };
        let cls = h.borrow().class;
        let scope = self.stack[top].defining_class;
        let res = classes::resolve_prop(&self.classes, cls, scope, &pname);
        let present = match &res {
            PropRes::Slot(k)
                if self.classes[cls as usize].hooked && self.hooked_entry(cls, k).is_some() =>
            {
                match self.hooked_read(&h, cls, k) {
                    Some(r) => return Ok(!matches!(deref(&r?), Value::Null)),
                    None => h.borrow().properties.get(k).cloned(),
                }
            }
            PropRes::Slot(k) => h.borrow().properties.get(k).cloned(),
            PropRes::Dynamic if Some(cls) == self.incomplete_class => {
                self.incomplete_warning(&h)?;
                return Ok(false);
            }
            PropRes::Dynamic => h.borrow().properties.get(&pname).cloned(),
            PropRes::Denied(_) => None,
        };
        if let Some(v) = present {
            // A declared/dynamic property: set iff non-null.
            return Ok(!matches!(deref(&v), Value::Null));
        }
        if let PropRes::Slot(k) = &res {
            // Skip __isset() for an uninitialized typed or lazy property.
            if self.lazy_slot(&h, Some(k)) {
                return self.lazy_isset(top, &h, pname, |vm, t, o, n| vm.prop_isset(t, o, n));
            }
            if self.uninit_typed(&h, k) {
                return Ok(false);
            }
        }
        if let Some(inst) = self.proxy_guarded(&h, b"__isset", &pname) {
            return self.prop_isset(top, Value::Object(inst), pname);
        }
        match self.magic(&h, cls, b"__isset", &pname) {
            Some(m) => {
                let nm = Value::str(pname.as_bytes().to_vec());
                let r = self.call_magic(&m, &h, cls, b"__isset", &pname, vec![nm])?;
                self.truthy(&r)
            }
            None if !matches!(res, PropRes::Slot(_)) && self.lazy_slot(&h, None) => {
                self.lazy_isset(top, &h, pname, |vm, t, o, n| vm.prop_isset(t, o, n))
            }
            None => Ok(false),
        }
    }

    /// has_property's lazy_init label: initialize `h`, then ask the
    /// instance, holding its `__isset` guard for the property.
    fn lazy_isset<T>(
        &mut self,
        top: usize,
        h: &ObjectHandle,
        pname: PhpStr,
        f: impl FnOnce(&mut Self, usize, Value, PhpStr) -> Result<T, Thrown>,
    ) -> Result<T, Thrown> {
        let inst = self.lazy_init(h)?;
        let icls = inst.borrow().class;
        let guarded = self.lookup_method(icls, b"__isset").is_some();
        let n = pname.clone();
        self.with_guard(&inst, guarded, b"__isset", &n, |vm| {
            f(vm, top, Value::Object(inst.clone()), pname)
        })
    }

    /// Run `f` holding the magic guard `which` for `name` on `h` (when
    /// `guard` is set and it is not held already).
    pub(crate) fn with_guard<T>(
        &mut self,
        h: &ObjectHandle,
        guard: bool,
        which: &'static [u8],
        name: &PhpStr,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let key = (h.borrow().id, which, name.clone());
        if !guard || self.prop_guards.contains(&key) {
            return f(self);
        }
        self.prop_guards.insert(key.clone());
        let r = f(self);
        self.prop_guards.remove(&key);
        r
    }

    /// For an initialized proxy whose instance holds the magic guard `which`
    /// for `name` (a recursive call from the instance's magic method,
    /// GH-21478): the instance, to forward to.
    fn proxy_guarded(
        &self,
        h: &ObjectHandle,
        which: &'static [u8],
        name: &PhpStr,
    ) -> Option<ObjectHandle> {
        let inst = h.borrow().lazy_instance()?;
        let id = inst.borrow().id;
        self.prop_guards
            .contains(&(id, which, name.clone()))
            .then_some(inst)
    }

    /// `empty($o->name)` (zend_std_has_property, ZEND_PROPERTY_NOT_EMPTY): a
    /// reachable set property's falsiness, else `__isset` and — only if it
    /// says yes — `__get`; inaccessible / missing without magic is empty.
    pub(crate) fn prop_empty(
        &mut self,
        top: usize,
        o: Value,
        pname: PhpStr,
    ) -> Result<bool, Thrown> {
        let Value::Object(h) = o else {
            return Ok(true);
        };
        let cls = h.borrow().class;
        let scope = self.stack[top].defining_class;
        let res = classes::resolve_prop(&self.classes, cls, scope, &pname);
        let present = match &res {
            PropRes::Slot(k)
                if self.classes[cls as usize].hooked && self.hooked_entry(cls, k).is_some() =>
            {
                match self.hooked_read(&h, cls, k) {
                    Some(r) => return Ok(!self.truthy(&deref(&r?))?),
                    None => h.borrow().properties.get(k).cloned(),
                }
            }
            PropRes::Slot(k) => h.borrow().properties.get(k).cloned(),
            PropRes::Dynamic if Some(cls) == self.incomplete_class => {
                self.incomplete_warning(&h)?;
                return Ok(true);
            }
            PropRes::Dynamic => h.borrow().properties.get(&pname).cloned(),
            PropRes::Denied(_) => None,
        };
        // zend_is_true, through an object's cast_object.
        if let Some(v) = present {
            return Ok(!self.truthy(&deref(&v))?);
        }
        if let PropRes::Slot(k) = &res {
            if self.lazy_slot(&h, Some(k)) {
                return self.lazy_isset(top, &h, pname, |vm, t, o, n| vm.prop_empty(t, o, n));
            }
            if self.uninit_typed(&h, k) {
                return Ok(true);
            }
        }
        if let Some(inst) = self.proxy_guarded(&h, b"__isset", &pname) {
            return self.prop_empty(top, Value::Object(inst), pname);
        }
        let nm = || Value::str(pname.as_bytes().to_vec());
        let Some(m) = self.magic(&h, cls, b"__isset", &pname) else {
            if !matches!(res, PropRes::Slot(_)) && self.lazy_slot(&h, None) {
                return self.lazy_isset(top, &h, pname, |vm, t, o, n| vm.prop_empty(t, o, n));
            }
            return Ok(true);
        };
        self.pending_magic_fetch = builtins::MagicFetch::Empty;
        let r = self.call_magic(&m, &h, cls, b"__isset", &pname, vec![nm()])?;
        if !self.truthy(&r)? {
            return Ok(true);
        }
        match self.magic(&h, cls, b"__get", &pname) {
            Some(g) => {
                let v = self.call_magic(&g, &h, cls, b"__get", &pname, vec![nm()])?;
                Ok(!self.truthy(&deref(&v))?)
            }
            None => Ok(true),
        }
    }

    /// `$o->name` in BP_VAR_IS mode (an isset / empty / `??` container): the
    /// reachable set property, else `__isset` then (if it says yes) `__get`,
    /// else null — never an access error (zend_std_read_property, BP_VAR_IS).
    pub(crate) fn prop_fetch_is(
        &mut self,
        top: usize,
        o: Value,
        pname: PhpStr,
    ) -> Result<Value, Thrown> {
        let Value::Object(h) = o else {
            return Ok(Value::Null);
        };
        let cls = h.borrow().class;
        let scope = self.stack[top].defining_class;
        let res = classes::resolve_prop(&self.classes, cls, scope, &pname);
        let present = match &res {
            PropRes::Slot(k)
                if self.classes[cls as usize].hooked && self.hooked_entry(cls, k).is_some() =>
            {
                match self.hooked_read(&h, cls, k) {
                    Some(r) => return r.map(|v| deref(&v)),
                    None => h.borrow().properties.get(k).cloned(),
                }
            }
            PropRes::Slot(k) => h.borrow().properties.get(k).cloned(),
            PropRes::Dynamic if Some(cls) == self.incomplete_class => {
                self.incomplete_warning(&h)?;
                return Ok(Value::Null);
            }
            PropRes::Dynamic => h.borrow().properties.get(&pname).cloned(),
            PropRes::Denied(_) => None,
        };
        if let Some(v) = present {
            return Ok(deref(&v));
        }
        let has_isset = self.lookup_method(cls, b"__isset").is_some();
        let lazy_fetch = |vm: &mut Self, pname: PhpStr| {
            let inst = vm.lazy_init(&h)?;
            let which: &'static [u8] = if has_isset { b"__isset" } else { b"__get" };
            let guarded = vm.lookup_method(cls, which).is_some();
            let n = pname.clone();
            vm.with_guard(&inst, guarded, which, &n, |vm| {
                vm.prop_fetch_is(top, Value::Object(inst.clone()), pname)
            })
        };
        if let PropRes::Slot(k) = &res {
            if self.lazy_slot(&h, Some(k)) {
                return lazy_fetch(self, pname);
            }
            if self.uninit_typed(&h, k) {
                return Ok(Value::Null);
            }
        }
        let which: &'static [u8] = if has_isset { b"__isset" } else { b"__get" };
        if let Some(inst) = self.proxy_guarded(&h, which, &pname) {
            return self.prop_fetch_is(top, Value::Object(inst), pname);
        }
        let nm = || Value::str(pname.as_bytes().to_vec());
        if let Some(m) = self.magic(&h, cls, b"__isset", &pname) {
            let r = self.call_magic(&m, &h, cls, b"__isset", &pname, vec![nm()])?;
            if !self.truthy(&r)? {
                return Ok(Value::Null);
            }
        }
        // (No `__isset`, or inside this property's own `__isset`: `__get`.)
        match self.magic(&h, cls, b"__get", &pname) {
            Some(g) => self.call_magic(&g, &h, cls, b"__get", &pname, vec![nm()]),
            None if !matches!(res, PropRes::Slot(_)) && self.lazy_slot(&h, None) => {
                lazy_fetch(self, pname)
            }
            None => Ok(Value::Null),
        }
    }

    /// `unset($o->name)`: remove the property reachable from the scope, else
    /// `__unset`, else an access error for an inaccessible declaration.
    pub(crate) fn prop_unset(&mut self, top: usize, o: Value, pname: PhpStr) -> Result<(), Thrown> {
        let Value::Object(h) = o else {
            return Ok(());
        };
        let cls = h.borrow().class;
        if Some(cls) == self.incomplete_class {
            return Err(self.incomplete_error(&h, "modify a property"));
        }
        if self.is_directory_class(cls) && self.class(cls).prop_visible.contains_key(&pname) {
            return Err(Thrown::error(
                "Error",
                format!(
                    "Cannot unset readonly property Directory::${}",
                    String::from_utf8_lossy(pname.as_bytes())
                ),
            ));
        }
        let scope = self.stack[top].defining_class;
        let res = classes::resolve_prop(&self.classes, cls, scope, &pname);
        if let PropRes::Slot(k) = &res {
            if let Some(pe) = self.hooked_entry(cls, k) {
                return Err(self.hooked_unset_err(cls, pe));
            }
        } else if matches!(res, PropRes::Dynamic) {
            self.static_as_instance_notice(cls, scope, &pname, b"__unset")?;
        }
        let key = match &res {
            PropRes::Slot(k) => Some(k.clone()),
            PropRes::Dynamic => Some(pname.clone()),
            PropRes::Denied(_) => None,
        };
        let present = key
            .as_ref()
            .is_some_and(|k| h.borrow().properties.contains_key(k));
        // zend_std_unset_property: the readonly and set visibility checks,
        // unless an unset (not uninitialized) slot defers to `__unset`.
        if let PropRes::Slot(k) = &res {
            let uninit = !present && self.uninit_typed(&h, k);
            if present || uninit || self.magic(&h, cls, b"__unset", &pname).is_none() {
                let called = self.stack[top].called_class;
                self.check_readonly_write(&h, k, present, scope, called, "unset")?;
            }
        }
        if !present {
            if let PropRes::Slot(k) = &res {
                if self.lazy_slot(&h, Some(k)) {
                    let inst = self.lazy_init(&h)?;
                    return self.prop_unset(top, Value::Object(inst), pname);
                }
            }
            if matches!(res, PropRes::Dynamic) {
                if let Some(inst) = self.proxy_guarded(&h, b"__unset", &pname) {
                    return self.prop_unset(top, Value::Object(inst), pname);
                }
            }
        }
        let was_uninit = matches!(&res, PropRes::Slot(k) if self.uninit_typed(&h, k));
        if present {
            let key = key.unwrap();
            self.consume_reinit(&h, &key);
            h.borrow_mut().properties.shift_remove(&key);
            self.mark_typed_unset(&h, &key);
        } else if was_uninit {
            // Reset IS_PROP_UNINIT, bypassing __unset().
            self.mark_typed_unset(&h, key.as_ref().unwrap());
        } else if let Some(m) = self.magic(&h, cls, b"__unset", &pname) {
            let nm = Value::str(pname.as_bytes().to_vec());
            self.call_magic(&m, &h, cls, b"__unset", &pname, vec![nm])?;
        } else if matches!(res, PropRes::Dynamic) && self.lazy_slot(&h, None) {
            let inst = self.lazy_init(&h)?;
            let guarded = self.lookup_method(cls, b"__unset").is_some();
            let n = pname.clone();
            return self.with_guard(&inst, guarded, b"__unset", &n, |vm| {
                vm.prop_unset(top, Value::Object(inst.clone()), pname)
            });
        } else if let PropRes::Denied(vis) = res {
            return Err(classes::prop_access_err(
                &self.classes,
                cls,
                pname.as_bytes(),
                vis,
            ));
        }
        Ok(())
    }

    /// zend_get_property_offset meeting a static property reached as
    /// `$o->name`: "Accessing static property C::$p as non static" (then a
    /// dynamic property is used), unless the class has the magic accessor
    /// `magic` (php's `silent`).
    pub(crate) fn static_as_instance_notice(
        &mut self,
        cls: ClassId,
        scope: Option<ClassId>,
        pname: &PhpStr,
        magic: &'static [u8],
    ) -> Result<(), Thrown> {
        if self.classes[cls as usize].static_props.is_empty()
            || !matches!(
                classes::static_as_instance(&self.classes, cls, scope, pname),
                Some(None)
            )
            || self.lookup_method(cls, magic).is_some()
        {
            return Ok(());
        }
        let msg = format!(
            "Accessing static property {}::${} as non static",
            String::from_utf8_lossy(self.class(cls).name.as_bytes()),
            String::from_utf8_lossy(pname.as_bytes())
        );
        self.diag(builtins::E_NOTICE, &msg)
    }

    /// Is the declared slot `key` of `h` an uninitialized typed property
    /// that was never `unset()` (php's IS_PROP_UNINIT)? Such a slot never
    /// routes through the magic accessors: writes go to the slot, reads are
    /// the "must not be accessed before initialization" Error.
    pub(crate) fn uninit_typed(&self, h: &ObjectHandle, key: &PhpStr) -> bool {
        let b = h.borrow();
        if b.properties.contains_key(key) {
            return false;
        }
        let typed = self
            .classes
            .get(b.class as usize)
            .and_then(|e| e.props.get(key))
            .is_some_and(|pe| pe.ty.is_some());
        typed
            && !self
                .unset_typed_props
                .get(&b.id)
                .is_some_and(|ks| ks.contains(key))
    }

    /// `unset()` of the declared typed slot `key`: from now on an absent
    /// slot defers to the magic accessors.
    pub(crate) fn mark_typed_unset(&mut self, h: &ObjectHandle, key: &PhpStr) {
        let b = h.borrow();
        let typed = self
            .classes
            .get(b.class as usize)
            .and_then(|e| e.props.get(key))
            .is_some_and(|pe| pe.ty.is_some());
        if typed {
            let ks = self.unset_typed_props.entry(b.id).or_default();
            if !ks.contains(key) {
                ks.push(key.clone());
            }
        }
    }

    /// The magic property accessor `which` (`__get`/`__set`/`__isset`/
    /// `__unset`) for `name` on `h`, unless that same accessor is already
    /// running for that property on that object (php's property guards: inside
    /// `__get('x')`, `$this->x` is a plain property access).
    pub(crate) fn magic(
        &self,
        h: &ObjectHandle,
        cls: ClassId,
        which: &'static [u8],
        name: &PhpStr,
    ) -> Option<classes::ResolvedMethod> {
        let id = h.borrow().id;
        if self.prop_guards.contains(&(id, which, name.clone())) {
            return None;
        }
        self.lookup_method(cls, which)
    }

    /// Run a magic accessor with its property guard held.
    fn call_magic(
        &mut self,
        m: &classes::ResolvedMethod,
        h: &ObjectHandle,
        cls: ClassId,
        which: &'static [u8],
        name: &PhpStr,
        args: Vec<Value>,
    ) -> Result<Value, Thrown> {
        let guard = (h.borrow().id, which, name.clone());
        self.prop_guards.insert(guard.clone());
        // The accessor may use the lvalue cursor of the write it serves.
        let r = self.with_saved_cursor(|vm| vm.call_sync(m, Some(h.clone()), cls, args));
        self.prop_guards.remove(&guard);
        r
    }

    /// `f` (a property or element fetch) with `mode` for the accessor it
    /// may call: the first synchronous call `f` makes takes it
    /// ([`Self::run_proto_sync`]).
    pub(crate) fn fetching_as<T>(
        &mut self,
        mode: builtins::MagicFetch,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        self.pending_magic_fetch = mode;
        let r = f(self);
        self.pending_magic_fetch = Default::default();
        r
    }

    /// php's static-property visibility check: a non-public static is reachable
    /// only from its declaring class (private) / a related class (protected);
    /// the error names the class the access was spelled against.
    pub(crate) fn check_static_prop_access(
        &self,
        top: usize,
        cls: ClassId,
        pname: &[u8],
    ) -> Result<(), Thrown> {
        if let Some(sp) = self.class(cls).static_props.get(pname) {
            if !self.static_prop_accessible(self.stack[top].defining_class, sp) {
                return Err(classes::prop_access_err(
                    &self.classes,
                    cls,
                    pname,
                    Deny::Vis(sp.visibility),
                ));
            }
        }
        Ok(())
    }

    /// Write `val` to property `pname` of `o` (visibility-checked, `__set` fallback).
    pub(crate) fn prop_store(
        &mut self,
        top: usize,
        o: Value,
        pname: PhpStr,
        val: Value,
    ) -> Result<(), Thrown> {
        let scope = self.stack[top].defining_class;
        let called = self.stack[top].called_class;
        let strict = self.stack[top].proto.strict_types;
        self.prop_store_as(top, o, pname, val, scope, called, strict)
    }

    /// [`Vm::prop_store`] from an explicit scope (php's `EG(fake_scope)`,
    /// as `ReflectionProperty::setValue` writes).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prop_store_as(
        &mut self,
        top: usize,
        o: Value,
        pname: PhpStr,
        val: Value,
        scope: Option<ClassId>,
        called: Option<ClassId>,
        strict: bool,
    ) -> Result<(), Thrown> {
        // Only the write `StoreProp` itself asked for reports its value.
        let want_result = std::mem::take(&mut self.want_typed_result);
        if let Value::Object(h) = o {
            let cls = h.borrow().class;
            if Some(cls) == self.incomplete_class {
                return Err(self.incomplete_error(&h, "modify a property"));
            }
            if self.is_directory_class(cls) {
                // Directory: readonly properties, no dynamic properties.
                let p = String::from_utf8_lossy(pname.as_bytes());
                return Err(Thrown::error(
                    "Error",
                    if self.class(cls).prop_visible.contains_key(&pname) {
                        format!("Cannot modify readonly property Directory::${p}")
                    } else {
                        format!("Cannot create dynamic property Directory::${p}")
                    },
                ));
            }
            let res = classes::resolve_prop(&self.classes, cls, scope, &pname);
            let val = match &res {
                PropRes::Slot(k) if self.classes[cls as usize].hooked => {
                    match self.hooked_store(&h, cls, k, scope, called, val)? {
                        Some(v) => v,
                        None => return Ok(()),
                    }
                }
                _ => val,
            };
            let is_dyn = matches!(res, PropRes::Dynamic);
            if is_dyn {
                self.static_as_instance_notice(cls, scope, &pname, b"__set")?;
            }
            let (key, denied) = match res {
                PropRes::Slot(k) => (k, None),
                PropRes::Dynamic => (pname.clone(), None),
                PropRes::Denied(vis) => (pname.clone(), Some(vis)),
            };
            let present = denied.is_none() && h.borrow().properties.contains_key(&key);
            if !present && denied.is_none() {
                // A lazy slot (bypassing __set()) or an undefined property
                // with no unguarded __set() initializes the object, and the
                // write goes to the instance (forward_write_to_lazy_object).
                let slot = if is_dyn { None } else { Some(&key) };
                if self.lazy_slot(&h, slot) {
                    if is_dyn {
                        if let Some(inst) = self.proxy_guarded(&h, b"__set", &pname) {
                            return self.prop_store_as(
                                top,
                                Value::Object(inst),
                                pname,
                                val,
                                scope,
                                called,
                                strict,
                            );
                        }
                    }
                    let has_set = self.lookup_method(cls, b"__set").is_some();
                    if !is_dyn || self.magic(&h, cls, b"__set", &pname).is_none() {
                        let inst = self.lazy_init(&h)?;
                        let n = pname.clone();
                        return self.with_guard(&inst, is_dyn && has_set, b"__set", &n, |vm| {
                            vm.prop_store_as(
                                top,
                                Value::Object(inst.clone()),
                                pname,
                                val,
                                scope,
                                called,
                                strict,
                            )
                        });
                    }
                }
            }
            // Writes to uninitialized typed properties bypass __set().
            let uninit = !present && denied.is_none() && !is_dyn && self.uninit_typed(&h, &key);
            // zend_std_write_property: an unset (not uninitialized) slot
            // with an unguarded `__set` defers to it.
            if denied.is_none()
                && !is_dyn
                && (present || uninit || self.magic(&h, cls, b"__set", &pname).is_none())
            {
                self.check_readonly_write(&h, &key, present, scope, called, "modify")?;
            }
            if !present && !uninit {
                // Inaccessible, or not currently set (dynamic / unset declared):
                // `__set` intercepts when defined.
                if let Some(setter) = self.magic(&h, cls, b"__set", &pname) {
                    let nm = Value::str(pname.as_bytes().to_vec());
                    self.call_magic(&setter, &h, cls, b"__set", &pname, vec![nm, val])?;
                    return Ok(());
                }
                if let Some(vis) = denied {
                    return Err(classes::prop_access_err(
                        &self.classes,
                        cls,
                        pname.as_bytes(),
                        vis,
                    ));
                }
                if is_dyn {
                    self.check_dynamic_prop(top, cls, &pname)?;
                }
            }
            if is_dyn && present && self.class(cls).kind == ClassKind::Enum {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Cannot modify readonly property {}::${}",
                        String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                        String::from_utf8_lossy(pname.as_bytes())
                    ),
                ));
            }
            let val = if is_dyn {
                val
            } else {
                let v = if self
                    .class(cls)
                    .props
                    .get(&key)
                    .is_some_and(|pe| pe.ty.is_some())
                {
                    let v = self.verify_prop_type(cls, &key, val, strict)?;
                    // The assignment's value is the coerced one (`StoreProp`).
                    if want_result {
                        self.typed_write_result = Some(v.clone());
                    }
                    v
                } else {
                    val
                };
                if present {
                    self.consume_reinit(&h, &key);
                }
                v
            };
            let mut od = h.borrow_mut();
            let slot = classes::prop_slot_mut(&self.classes, &mut od, key);
            match slot {
                Value::Ref(rc) => {
                    let rc = rc.clone();
                    drop(od);
                    *rc.borrow_mut() = val;
                }
                other => *other = val,
            }
            return Ok(());
        }
        if let Value::Closure(_) = o {
            return Err(closure_prop_error(pname.as_bytes()));
        }
        Err(Thrown::error(
            "Error",
            format!(
                "Attempt to assign property \"{}\" on {}",
                String::from_utf8_lossy(pname.as_bytes()),
                rhp_builtins::zval_value_name(self, &o)
            ),
        ))
    }

    /// Creating a dynamic property (zend_forbidden_dynamic_property /
    /// zend_deprecated_dynamic_property): an Error on enums and readonly
    /// classes, else deprecated unless the class (or a parent) allows it.
    pub(crate) fn check_dynamic_prop(
        &mut self,
        top: usize,
        cls: ClassId,
        pname: &PhpStr,
    ) -> Result<(), Thrown> {
        let e = self.class(cls);
        let cname = String::from_utf8_lossy(crate::errors::c_name(e.name.as_bytes())).into_owned();
        let p = String::from_utf8_lossy(pname.as_bytes()).into_owned();
        // Engine-internal (prelude) code manages its own objects, except
        // that a stand-in `__set` writes on behalf of the user's code.
        let f = &self.stack[top];
        let prelude = f.proto.is_internal()
            && !(f.proto.name.as_bytes().eq_ignore_ascii_case(b"__set")
                && f.defining_class
                    .is_some_and(|c| is_handler_accessor(self.class(c).name.as_bytes(), b"__set")));
        // ZEND_ACC_NO_DYNAMIC_PROPERTIES: enums, readonly classes and the
        // internal `@strict-properties` classes.
        if e.kind == ClassKind::Enum || e.flags.is_readonly || (e.flags.no_dynamic && !prelude) {
            return Err(Thrown::error(
                "Error",
                format!("Cannot create dynamic property {cname}::${p}"),
            ));
        }
        // ZEND_ACC_ALLOW_DYNAMIC_PROPERTIES (inherited at link time).
        if e.flags.allow_dynamic || prelude {
            return Ok(());
        }
        self.diag(
            builtins::E_DEPRECATED,
            &format!("Creation of dynamic property {cname}::${p} is deprecated"),
        )
    }

    /// incomplete_class.c's message for object `h`: what the script tried,
    /// and the class name the object remembers.
    fn incomplete_msg(&self, h: &ObjectHandle, what: &str) -> String {
        let name = match h
            .borrow()
            .properties
            .get(&PhpStr::from("__PHP_Incomplete_Class_Name"))
        {
            Some(Value::Str(s)) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
            _ => "unknown".to_string(),
        };
        format!(
            "The script tried to {what} on an incomplete object. Please ensure that the class definition \"{name}\" of the object you are trying to operate on was loaded _before_ unserialize() gets called or provide an autoloader to load the class definition"
        )
    }

    /// throw_incomplete_class_error.
    pub(crate) fn incomplete_error(&self, h: &ObjectHandle, what: &str) -> Thrown {
        Thrown::error("Error", self.incomplete_msg(h, what))
    }

    /// incomplete_class_message: reads and isset() warn.
    pub(crate) fn incomplete_warning(&mut self, h: &ObjectHandle) -> Result<(), Thrown> {
        let msg = self.incomplete_msg(h, "access a property");
        builtins::Host::docref_diagnostic(self, builtins::E_WARNING, msg.as_bytes())
    }

    /// The internal `Directory` class (readonly props, not dynamic, not
    /// cloneable, only created by `dir()`). User code cannot declare a class
    /// of that name, so the name identifies it.
    pub(crate) fn is_directory_class(&self, cls: ClassId) -> bool {
        let n = self.class(cls).name.as_bytes();
        n.len() == 9 && n.eq_ignore_ascii_case(b"Directory")
    }

    /// Objects of this class have no `clone_obj` handler ("Trying to clone
    /// an uncloneable object"): enums, and the classes whose handlers say so.
    pub(crate) fn uncloneable_class(&self, cls: ClassId) -> bool {
        self.class(cls).flags.not_cloneable
    }

    /// The storage key of property `name` on `obj` as its class sees it,
    /// ignoring visibility: the most-derived declaration (a private one is
    /// stored mangled, `"\0Class\0name"`), else the plain (dynamic) name.
    /// Engine-internal accesses such as Exception's private `trace` /
    /// `previous` go through this.
    pub(crate) fn declared_prop_key(&self, obj: &ObjectHandle, name: &[u8]) -> rhp_values::PhpStr {
        let name = rhp_values::PhpStr::from_bytes(name);
        let cls = obj.borrow().class;
        self.class(cls)
            .prop_visible
            .get(&name)
            .cloned()
            .unwrap_or(name)
    }

    /// `(array) $object`: every property, keyed with php's visibility mangling —
    /// public `name`, protected `"\0*\0name"`, private `"\0Class\0name"`.
    pub(crate) fn object_to_array_mangled(&self, h: &ObjectHandle) -> Value {
        self.prop_table_copy(h, false)
    }

    /// `(array)$obj` (zend_proptable_to_symtable): the mangled properties,
    /// keeping references other slots share.
    pub(crate) fn object_cast_array(&self, h: &ObjectHandle) -> Value {
        self.prop_table_copy(h, true)
    }

    fn prop_table_copy(&self, h: &ObjectHandle, keep_refs: bool) -> Value {
        let b = h.borrow();
        let cls = b.class;
        let mut a = PhpArray::new();
        let entry = self.class(cls);
        for (key, val) in &b.properties {
            if self.internal_slot(cls, key) {
                continue;
            }
            let key = classes::mangled_prop_key(entry, key);
            let v = if keep_refs {
                rhp_values::ops::copy_elem(val)
            } else {
                deref(val)
            };
            a.insert(ArrayKey::from_bytes(key.as_bytes()), v);
        }
        Value::Array(a)
    }

    /// Snapshot an object's accessible (public from `caller` scope) properties as
    /// an array, for `foreach` over a plain object.
    pub(crate) fn object_prop_array(&self, h: &ObjectHandle, top: usize) -> PhpArray {
        let caller = self.stack[top].defining_class;
        let b = h.borrow();
        let cls = b.class;
        let mut arr = PhpArray::new();
        for (key, val) in &b.properties {
            if self.internal_slot(cls, key) {
                continue;
            }
            if let Some(name) = classes::slot_accessible(&self.classes, cls, caller, key) {
                // Property names iterate as string keys, even numeric ones.
                arr.insert(ArrayKey::Str(classes::unmangle(&name)), deref(val).clone());
            }
        }
        arr
    }

    /// Whether class `c` is a built-in class implemented in the PHP prelude.
    fn is_prelude_class(&self, c: ClassId) -> bool {
        // Built-in classes come first; a prelude class declares methods
        // compiled from the prelude.
        (c as usize) < self.internal_classes
            && self
                .class(c)
                .methods
                .values()
                .any(|m| m.defining_class == c && m.proto.is_internal())
    }

    /// Reading the absent (uninitialized) slot `key` of a typed property:
    /// php's "must not be accessed before initialization" Error.
    pub(crate) fn typed_slot(&self, cls: ClassId, key: &PhpStr) -> Option<Thrown> {
        let pe = self.classes.get(cls as usize)?.props.get(key)?;
        pe.ty.as_ref()?;
        Some(Thrown::error(
            "Error",
            format!(
                "Typed property {}::${} must not be accessed before initialization",
                String::from_utf8_lossy(self.class(pe.declaring_class).name.as_bytes()),
                String::from_utf8_lossy(pe.name.as_bytes())
            ),
        ))
    }

    /// zend_verify_property_type for an assignment of `val` to the declared
    /// slot `key` of class `cls`: coercive (or strict) scalar conversion, the
    /// `__toString` step, else "Cannot assign T to property C::$p of type X".
    pub(crate) fn verify_prop_type(
        &mut self,
        cls: ClassId,
        key: &PhpStr,
        val: Value,
        strict: bool,
    ) -> Result<Value, Thrown> {
        let Some(pe) = self
            .classes
            .get(cls as usize)
            .and_then(|e| e.props.get(key))
        else {
            return Ok(val);
        };
        let Some(ty) = pe.ty.clone() else {
            return Ok(val);
        };
        let decl = pe.declaring_class;
        let name = pe.name.clone();
        self.verify_type_for_prop(&ty, decl, cls, name.as_bytes(), val, strict)
    }

    /// [`Vm::verify_prop_type`] for property `name` declared by `decl` with
    /// type `ty`, seen on class `cls` (static properties too).
    pub(crate) fn verify_type_for_prop(
        &mut self,
        ty: &Rc<rhp_bytecode::TypeHint>,
        decl: ClassId,
        cls: ClassId,
        name: &[u8],
        val: Value,
        strict: bool,
    ) -> Result<Value, Thrown> {
        let val = self.coerce_stringable(val, ty, strict, Some(decl))?;
        match self.check_one(&val, ty, strict, Some(decl)) {
            Ok(None) => Ok(val),
            Ok(Some(c)) => {
                self.coercion_diag(&val, &c)?;
                Ok(c)
            }
            Err(()) => Err(Thrown::type_error(format!(
                "Cannot assign {} to property {}::${} of type {}",
                rhp_builtins::zval_value_name(self, &deref(&val)),
                String::from_utf8_lossy(self.class(decl).name.as_bytes()),
                String::from_utf8_lossy(name),
                self.prop_type_string(ty, decl, Some(cls))
            ))),
        }
    }

    /// zend_verify_prop_assignable_by_ref_ex in the MAGIC_GET context: what
    /// `__get` returned for the unset typed slot `key`, coerced to its type.
    fn verify_magic_get(
        &mut self,
        cls: ClassId,
        key: &PhpStr,
        r: Value,
        strict: bool,
    ) -> Result<Value, Thrown> {
        let pe = &self.class(cls).props[key];
        let ty = pe.ty.clone().expect("typed");
        let decl = pe.declaring_class;
        let name = pe.name.clone();
        let (cell, val) = match &r {
            Value::Ref(rc) => (Some(rc.clone()), rc.borrow().clone()),
            v => (None, v.clone()),
        };
        let val = self.coerce_stringable(val, &ty, strict, Some(decl))?;
        let val = match self.check_one(&val, &ty, strict, Some(decl)) {
            Ok(None) => val,
            Ok(Some(c)) => {
                self.coercion_diag(&val, &c)?;
                c
            }
            Err(()) => {
                let dn = String::from_utf8_lossy(self.class(decl).name.as_bytes()).into_owned();
                return Err(Thrown::type_error(format!(
                    "Value of type {} returned from {dn}::__get() must be compatible with unset property {dn}::${} of type {}",
                    rhp_builtins::zval_type_name(self, &val),
                    String::from_utf8_lossy(name.as_bytes()),
                    self.prop_type_string(&ty, decl, Some(cls))
                )));
            }
        };
        Ok(match cell {
            Some(rc) => {
                *rc.borrow_mut() = val;
                Value::Ref(rc)
            }
            None => val,
        })
    }

    /// `++`/`--` turned the int in a plain typed property leaf into a
    /// float: unless the type takes floats, php's zend_throw_incdec_prop_error.
    pub(crate) fn incdec_typed_overflow(
        &mut self,
        top: usize,
        steps: &[crate::lvalue::CursorStep],
        delta: i8,
    ) -> Result<(), Thrown> {
        let (ty, decl, cls, name) = if steps.is_empty() {
            let Some(crate::lvalue::StaticRoot {
                cls,
                name,
                decl,
                ty: Some(ty),
                ..
            }) = self.cursor_static_prop()
            else {
                return Ok(());
            };
            (ty, decl, cls, name)
        } else {
            if !matches!(
                self.cursor_magic_split(top, steps),
                crate::lvalue::MagicSplit::Plain
            ) {
                return Ok(());
            }
            let Some((cls, key)) = self.cursor_leaf_typed(top, steps) else {
                return Ok(());
            };
            let pe = &self.class(cls).props[&key];
            (
                pe.ty.clone().expect("typed leaf"),
                pe.declaring_class,
                cls,
                pe.name.clone(),
            )
        };
        if ty.kinds.iter().any(|k| {
            matches!(
                k,
                rhp_bytecode::TypeKind::Float | rhp_bytecode::TypeKind::Mixed
            )
        }) {
            return Ok(());
        }
        let (what, end) = if delta > 0 {
            ("increment", "maximal")
        } else {
            ("decrement", "minimal")
        };
        Err(Thrown::type_error(format!(
            "Cannot {what} property {}::${} of type {} past its {end} value",
            String::from_utf8_lossy(self.class(decl).name.as_bytes()),
            String::from_utf8_lossy(name.as_bytes()),
            self.prop_type_string(&ty, decl, Some(cls))
        )))
    }

    /// php's readonly and asymmetric visibility checks on writing (`op`:
    /// "modify" / "unset") the declared slot `key` (`present`: initialized):
    /// "Cannot modify readonly property" for an initialized one (outside
    /// the `__clone` window), then the set visibility (a public readonly
    /// property is protected(set)), named after the called scope.
    pub(crate) fn check_readonly_write(
        &mut self,
        obj: &ObjectHandle,
        key: &PhpStr,
        present: bool,
        scope: Option<ClassId>,
        called: Option<ClassId>,
        op: &str,
    ) -> Result<(), Thrown> {
        let cls = obj.borrow().class;
        let Some(pe) = self
            .classes
            .get(cls as usize)
            .and_then(|e| e.props.get(key))
        else {
            return Ok(());
        };
        if !pe.readonly && pe.set_visibility.is_none() {
            return Ok(());
        }
        if pe.readonly && present && !self.reinitable(obj, key) {
            let what = if op == "unset" { "unset" } else { "modify" };
            return Err(Thrown::error(
                "Error",
                format!(
                    "Cannot {what} readonly property {}::${}",
                    String::from_utf8_lossy(self.class(pe.declaring_class).name.as_bytes()),
                    String::from_utf8_lossy(pe.name.as_bytes())
                ),
            ));
        }
        if let Err(e) = classes::set_access_ok(&self.classes, cls, scope, key) {
            return Err(classes::set_access_err(&self.classes, called, op, e));
        }
        Ok(())
    }

    /// The readonly slot `key` of `obj` may be modified once more: `obj`
    /// is a fresh clone whose `__clone` is running (IS_PROP_REINITABLE).
    fn reinitable(&self, obj: &ObjectHandle, key: &PhpStr) -> bool {
        if self.readonly_reinit.is_empty() {
            return false;
        }
        let oid = obj.borrow().id;
        let Some((depth, keys)) = self.readonly_reinit.get(&oid) else {
            return false;
        };
        let alive = self.stack.get(*depth).is_some_and(|f| {
            f.proto.name.as_bytes().eq_ignore_ascii_case(b"__clone")
                && f.proto.this_cv.is_some_and(|cv| {
                    matches!(f.regs.get(cv as usize), Some(Value::Object(o)) if Rc::ptr_eq(o, obj))
                })
        });
        alive && keys.contains(key)
    }

    /// A write or unset used up the `__clone` window of slot `key`.
    pub(crate) fn consume_reinit(&mut self, obj: &ObjectHandle, key: &PhpStr) {
        if self.readonly_reinit.is_empty() {
            return;
        }
        let oid = obj.borrow().id;
        if let Some((_, keys)) = self.readonly_reinit.get_mut(&oid) {
            keys.retain(|k| k != key);
        }
    }

    /// The frame at stack index `depth` returned: close the `__clone`
    /// windows it held, so a later direct `__clone()` call at the same
    /// depth gets none.
    pub(crate) fn close_reinit(&mut self, depth: usize) {
        self.readonly_reinit.retain(|_, (d, _)| *d < depth);
    }

    /// Open the `__clone` window of the clone `obj`, whose `__clone` frame
    /// is about to be pushed at stack index `depth`: each initialized
    /// readonly property may be modified once.
    pub(crate) fn open_reinit(&mut self, obj: &ObjectHandle, depth: usize) {
        let b = obj.borrow();
        let e = self.class(b.class);
        let keys: Vec<PhpStr> = e
            .props
            .iter()
            .filter(|(k, pe)| pe.readonly && b.properties.contains_key(*k))
            .map(|(k, _)| k.clone())
            .collect();
        if !keys.is_empty() {
            let id = b.id;
            drop(b);
            // Windows whose frame returned are closed.
            self.readonly_reinit.retain(|_, (d, _)| *d < depth);
            self.readonly_reinit.insert(id, (depth, keys));
        }
    }

    /// A prelude class's `__`-prefixed state slot (see
    /// [`builtins::Host::is_internal_slot`]): declared by a prelude class,
    /// so a user subclass's own `$__x` stays visible.
    pub(crate) fn internal_slot(&self, class: ClassId, key: &PhpStr) -> bool {
        let Some(pe) = self
            .classes
            .get(class as usize)
            .and_then(|e| e.props.get(key))
        else {
            return false;
        };
        pe.name.as_bytes().starts_with(b"__") && self.is_prelude_class(pe.declaring_class)
    }

    /// `CloneObj`: `clone` the value in `src` into `dst`, then push `__clone`.
    pub(crate) fn exec_clone(&mut self, top: usize, dst: Reg, src: Reg) -> Result<(), Thrown> {
        let v = self.reg_r(top, src)?;
        if let Value::Closure(cd) = &v {
            // A cloned closure is a new Closure object.
            let copy = rhp_values::ClosureData {
                fnidx: cd.fnidx,
                captures: cd.captures.iter().map(Value::copy_slot).collect(),
                bound_this: cd.bound_this.clone(),
                bound_class: cd.bound_class,
                bound_static: cd.bound_static,
                wrapped: cd.wrapped.clone(),
                id: rhp_values::alloc_object_id(),
                statics: cd.statics_copy(),
            };
            self.reg_set(top, dst, Value::Closure(Rc::new(copy)));
            return Ok(());
        }
        let Value::Object(h) = v else {
            return Err(Thrown::error(
                "Error",
                "__clone method called on non-object",
            ));
        };
        if h.borrow().is_lazy() {
            let new_obj = self.lazy_clone(&h)?;
            self.reg_set(top, dst, Value::Object(new_obj.clone()));
            let cls = new_obj.borrow().class;
            if !new_obj.borrow().is_lazy() {
                if let Some(m) = self.lookup_method(cls, b"__clone") {
                    self.open_reinit(&new_obj, self.stack.len());
                    self.enter_method(top, &m, Some(new_obj), cls, Reg::MAX, src, 0)?;
                }
            }
            return Ok(());
        }
        let new_obj = self.clone_members(&h)?;
        self.reg_set(top, dst, Value::Object(new_obj.clone()));
        let cls = new_obj.borrow().class;
        if let Some(m) = self.lookup_method(cls, b"__clone") {
            self.open_reinit(&new_obj, self.stack.len());
            self.enter_method(top, &m, Some(new_obj), cls, Reg::MAX, src, 0)?;
        }
        Ok(())
    }

    /// zend_objects_clone_obj without `__clone`: a new object with copies
    /// of `h`'s properties.
    pub(crate) fn clone_members(&mut self, h: &ObjectHandle) -> Result<ObjectHandle, Thrown> {
        // php's zval_add_ref: a reference only this object holds is
        // copied as a plain value, so the clone gets its own slot.
        // Holders other than the object and our copy that are
        // compiler temporaries (a dead `&` result, a by-ref
        // argument already passed) are not PHP references.
        let (cls, props) = {
            let od = h.borrow();
            let mut props = od.properties.clone();
            for v in props.values_mut() {
                if let Value::Ref(rc) = v {
                    if Rc::strong_count(rc) == 2 + self.temp_ref_holders(rc) {
                        let inner = rc.borrow().clone();
                        *v = inner;
                    }
                }
            }
            (od.class, props)
        };
        if self.uncloneable_class(cls) {
            return Err(Thrown::error(
                "Error",
                format!(
                    "Trying to clone an uncloneable object of class {}",
                    String::from_utf8_lossy(self.class(cls).name.as_bytes())
                ),
            ));
        }
        let id = self.alloc_object_id();
        // The clone keeps the property slots' unset flags.
        if let Some(ks) = self.unset_typed_props.get(&h.borrow().id).cloned() {
            self.unset_typed_props.insert(id, ks);
        }
        let new_obj: ObjectHandle = Rc::new(RefCell::new(ObjectData::new(cls, id, props)));
        self.track_destructible(&new_obj);
        if let Some(rhp_builtins::zend::object_handlers::CloneObj::Custom(f)) =
            self.class(cls).handlers.map(|t| t.clone_obj)
        {
            f(self, h, &new_obj)?;
        }
        Ok(new_obj)
    }
}

/// A Closure has no properties: writing one fails as a forbidden dynamic
/// property.
pub(crate) fn closure_prop_error(name: &[u8]) -> Thrown {
    Thrown::error(
        "Error",
        format!(
            "Cannot create dynamic property Closure::${}",
            String::from_utf8_lossy(name)
        ),
    )
}
