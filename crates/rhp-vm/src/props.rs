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
                        if let Some(v) = h.borrow().properties.get(&pname) {
                            return Ok(deref(v));
                        }
                        None
                    }
                    PropRes::Denied(vis) => Some(vis),
                };
                match self.magic(&h, cls, b"__get", &pname) {
                    Some(getter) => {
                        let arg = Value::str(pname.as_bytes().to_vec());
                        self.call_magic(&getter, &h, cls, b"__get", &pname, vec![arg])?
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
        let present = match classes::resolve_prop(&self.classes, cls, scope, &pname) {
            PropRes::Slot(k)
                if self.classes[cls as usize].hooked && self.hooked_entry(cls, &k).is_some() =>
            {
                match self.hooked_read(&h, cls, &k) {
                    Some(r) => return Ok(!matches!(deref(&r?), Value::Null)),
                    None if self.uninit_typed(&h, &k) => return Ok(false),
                    None => h.borrow().properties.get(&k).cloned(),
                }
            }
            // Skip __isset() for an uninitialized typed property.
            PropRes::Slot(k) if self.uninit_typed(&h, &k) => return Ok(false),
            PropRes::Slot(k) => h.borrow().properties.get(&k).cloned(),
            PropRes::Dynamic => h.borrow().properties.get(&pname).cloned(),
            PropRes::Denied(_) => None,
        };
        Ok(match present {
            // A declared/dynamic property: set iff non-null.
            Some(v) => !matches!(deref(&v), Value::Null),
            None => match self.magic(&h, cls, b"__isset", &pname) {
                Some(m) => {
                    let nm = Value::str(pname.as_bytes().to_vec());
                    let r = self.call_magic(&m, &h, cls, b"__isset", &pname, vec![nm])?;
                    self.truthy(&r)?
                }
                None => false,
            },
        })
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
        let present = match classes::resolve_prop(&self.classes, cls, scope, &pname) {
            PropRes::Slot(k)
                if self.classes[cls as usize].hooked && self.hooked_entry(cls, &k).is_some() =>
            {
                match self.hooked_read(&h, cls, &k) {
                    Some(r) => return Ok(!self.truthy(&deref(&r?))?),
                    None if self.uninit_typed(&h, &k) => return Ok(true),
                    None => h.borrow().properties.get(&k).cloned(),
                }
            }
            PropRes::Slot(k) if self.uninit_typed(&h, &k) => return Ok(true),
            PropRes::Slot(k) => h.borrow().properties.get(&k).cloned(),
            PropRes::Dynamic => h.borrow().properties.get(&pname).cloned(),
            PropRes::Denied(_) => None,
        };
        // zend_is_true, through an object's cast_object.
        if let Some(v) = present {
            return Ok(!self.truthy(&deref(&v))?);
        }
        let nm = || Value::str(pname.as_bytes().to_vec());
        let Some(m) = self.magic(&h, cls, b"__isset", &pname) else {
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
        let present = match classes::resolve_prop(&self.classes, cls, scope, &pname) {
            PropRes::Slot(k)
                if self.classes[cls as usize].hooked && self.hooked_entry(cls, &k).is_some() =>
            {
                match self.hooked_read(&h, cls, &k) {
                    Some(r) => return r.map(|v| deref(&v)),
                    None if self.uninit_typed(&h, &k) => return Ok(Value::Null),
                    None => h.borrow().properties.get(&k).cloned(),
                }
            }
            PropRes::Slot(k) if self.uninit_typed(&h, &k) => return Ok(Value::Null),
            PropRes::Slot(k) => h.borrow().properties.get(&k).cloned(),
            PropRes::Dynamic => h.borrow().properties.get(&pname).cloned(),
            PropRes::Denied(_) => None,
        };
        if let Some(v) = present {
            return Ok(deref(&v));
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
        }
        let key = match &res {
            PropRes::Slot(k) => Some(k.clone()),
            PropRes::Dynamic => Some(pname.clone()),
            PropRes::Denied(_) => None,
        };
        let present = key
            .as_ref()
            .is_some_and(|k| h.borrow().properties.contains_key(k));
        // zend_std_unset_property: an initialized readonly property can never
        // be unset; otherwise the (explicit or implicit) set visibility rules.
        if let PropRes::Slot(k) = &res {
            if let Some(pe) = self.class(cls).props.get(k).filter(|pe| pe.readonly) {
                if present {
                    return Err(Thrown::error(
                        "Error",
                        format!(
                            "Cannot unset readonly property {}::${}",
                            String::from_utf8_lossy(self.class(pe.declaring_class).name.as_bytes()),
                            String::from_utf8_lossy(pe.name.as_bytes())
                        ),
                    ));
                }
            }
            let called = self.stack[top].called_class;
            if let Err(e) = classes::set_access_ok(&self.classes, cls, scope, k) {
                return Err(classes::set_access_err(&self.classes, called, "unset", e));
            }
            // A public readonly property is implicitly protected(set).
            if pe_public_implicit_set(self, cls, k) {
                let pe = &self.class(cls).props[k];
                let decl = pe.declaring_class;
                let related = scope.is_some_and(|s| {
                    s == decl
                        || self.class(s).ancestors.contains(&decl)
                        || self.class(decl).ancestors.contains(&s)
                });
                if !related {
                    let e = (rhp_values::Visibility::Protected, decl, pe.name.clone());
                    return Err(classes::set_access_err(&self.classes, called, "unset", e));
                }
            }
        }
        let was_uninit = matches!(&res, PropRes::Slot(k) if self.uninit_typed(&h, k));
        if present {
            let key = key.unwrap();
            h.borrow_mut().properties.shift_remove(&key);
            self.mark_typed_unset(&h, &key);
        } else if was_uninit {
            // Reset IS_PROP_UNINIT, bypassing __unset().
            self.mark_typed_unset(&h, key.as_ref().unwrap());
        } else if let Some(m) = self.magic(&h, cls, b"__unset", &pname) {
            let nm = Value::str(pname.as_bytes().to_vec());
            self.call_magic(&m, &h, cls, b"__unset", &pname, vec![nm])?;
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
        let r = self.call_sync(m, Some(h.clone()), cls, args);
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
        if let Value::Object(h) = o {
            let cls = h.borrow().class;
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
            let (key, denied) = match res {
                PropRes::Slot(k) => (k, None),
                PropRes::Dynamic => (pname.clone(), None),
                PropRes::Denied(vis) => (pname.clone(), Some(vis)),
            };
            let present = denied.is_none() && h.borrow().properties.contains_key(&key);
            // Writes to uninitialized typed properties bypass __set().
            let uninit = !present && denied.is_none() && !is_dyn && self.uninit_typed(&h, &key);
            if denied.is_none() {
                if let Err(e) = classes::set_access_ok(&self.classes, cls, scope, &key) {
                    // An unset asymmetric property still defers to `__set`.
                    if !present && !uninit {
                        if let Some(setter) = self.magic(&h, cls, b"__set", &pname) {
                            let nm = Value::str(pname.as_bytes().to_vec());
                            self.call_magic(&setter, &h, cls, b"__set", &pname, vec![nm, val])?;
                            return Ok(());
                        }
                    }
                    return Err(classes::set_access_err(&self.classes, called, "modify", e));
                }
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
                let present = h.borrow().properties.contains_key(&key);
                self.check_readonly_write(&h, &key, present, scope)?;
                self.verify_prop_type(cls, &key, val, strict)?
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
        let cname = String::from_utf8_lossy(e.name.as_bytes()).into_owned();
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
                arr.insert(ArrayKey::Str(name), deref(val).clone());
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
        let val = self.coerce_stringable(val, &ty, strict, Some(decl))?;
        match self.check_one(&val, &ty, strict, Some(decl)) {
            Ok(None) => Ok(val),
            Ok(Some(c)) => {
                self.coercion_diag(&val, &c)?;
                Ok(c)
            }
            Err(()) => Err(Thrown::type_error(format!(
                "Cannot assign {} to property {}::${} of type {}",
                rhp_builtins::zval_value_name(self, &deref(&val)),
                String::from_utf8_lossy(self.class(decl).name.as_bytes()),
                String::from_utf8_lossy(name.as_bytes()),
                self.type_string_resolved(&ty, Some(decl), Some(cls))
            ))),
        }
    }

    /// The readonly checks of a write to the declared slot `key` (`present`:
    /// already initialized): php's "Cannot modify readonly property" and,
    /// for an uninitialized one written outside its declaring class,
    /// "Cannot initialize readonly property … from scope/global scope".
    pub(crate) fn check_readonly_write(
        &mut self,
        obj: &ObjectHandle,
        key: &PhpStr,
        present: bool,
        scope: Option<ClassId>,
    ) -> Result<(), Thrown> {
        let cls = obj.borrow().class;
        let Some(pe) = self
            .classes
            .get(cls as usize)
            .and_then(|e| e.props.get(key))
        else {
            return Ok(());
        };
        if !pe.readonly {
            return Ok(());
        }
        let decl = pe.declaring_class;
        let name = pe.name.clone();
        let cname = String::from_utf8_lossy(self.class(decl).name.as_bytes()).into_owned();
        let p = String::from_utf8_lossy(name.as_bytes()).into_owned();
        // Redeclared in a child: the parent that declared it may still
        // initialize it (zend_verify_readonly_initialization_access).
        let parent_scope_ok = scope.is_some_and(|s| {
            s != decl
                && self.class(cls).ancestors.contains(&s)
                && self
                    .class(s)
                    .prop_visible
                    .get(&name)
                    .and_then(|k| self.class(s).props.get(k))
                    .is_some_and(|spe| spe.declaring_class == s)
        });
        if present {
            // php 8.3: during __clone each readonly property may be
            // reinitialized once (from the property's scope).
            let oid = obj.borrow().id;
            let in_clone = self.stack.iter().any(|f| {
                f.proto.name.as_bytes().eq_ignore_ascii_case(b"__clone")
                    && f.proto
                        .this_cv
                        .and_then(|cv| match f.regs.get(cv as usize) {
                            Some(Value::Object(o)) => Some(o.borrow().id == oid),
                            _ => None,
                        })
                        .unwrap_or(false)
            });
            if in_clone && (scope == Some(decl) || parent_scope_ok) && {
                let done = self.readonly_reinit.entry(oid).or_default();
                !done.contains(key) && {
                    done.push(key.clone());
                    true
                }
            } {
                return Ok(());
            }
            return Err(Thrown::error(
                "Error",
                format!("Cannot modify readonly property {cname}::${p}"),
            ));
        }
        // A public readonly property is implicitly protected(set).
        if pe_public_implicit_set(self, cls, key)
            && !scope.is_some_and(|s| {
                self.class(s).ancestors.contains(&decl) || self.class(decl).ancestors.contains(&s)
            })
        {
            let from = match scope {
                Some(s) => format!(
                    "scope {}",
                    String::from_utf8_lossy(self.class(s).name.as_bytes())
                ),
                None => "global scope".into(),
            };
            return Err(Thrown::error(
                "Error",
                format!("Cannot modify protected(set) readonly property {cname}::${p} from {from}"),
            ));
        }
        if scope != Some(decl) && !parent_scope_ok {
            let from = match scope {
                Some(s) => format!(
                    "scope {}",
                    String::from_utf8_lossy(self.class(s).name.as_bytes())
                ),
                None => "global scope".into(),
            };
            return Err(Thrown::error(
                "Error",
                format!("Cannot initialize readonly property {cname}::${p} from {from}"),
            ));
        }
        Ok(())
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
        let v = self.reg_get(top, src);
        if let Value::Closure(cd) = &v {
            // A cloned closure is a new Closure object.
            let copy = rhp_values::ClosureData {
                fnidx: cd.fnidx,
                captures: cd.captures.clone(),
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
            f(self, &h, &new_obj)?;
        }
        self.reg_set(top, dst, Value::Object(new_obj.clone()));
        if let Some(m) = self.lookup_method(cls, b"__clone") {
            self.enter_method(top, &m, Some(new_obj), cls, Reg::MAX, src, 0)?;
        }
        Ok(())
    }
}

/// A public readonly property without an explicit set visibility (php 8.4
/// makes it protected(set)).
fn pe_public_implicit_set(vm: &Vm, cls: ClassId, key: &PhpStr) -> bool {
    vm.class(cls).props.get(key).is_some_and(|pe| {
        pe.readonly
            && pe.visibility == rhp_values::Visibility::Public
            && pe.set_visibility.is_none()
    })
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
