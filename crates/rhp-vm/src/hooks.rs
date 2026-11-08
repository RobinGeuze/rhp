//! Property hooks at run time: php's hook branches of zend_std_read_property,
//! write_property, has_property and unset_property, the "inside its own
//! hook" rule, `parent::$prop::get()`, and the property tables of hooked
//! objects (zend_property_hooks.c).
//!
//! Only classes with a hooked property (`ClassEntry::hooked`) come here, so
//! other property accesses pay one field test.

use std::rc::Rc;

use rhp_bytecode::{HookKind, Reg};
use rhp_values::{ArrayKey, ClassId, ObjectHandle, PhpArray, PhpStr, Thrown, Value, Visibility};

use crate::classes::{self, PropEntry, PropRes, ResolvedMethod};
use crate::{deref, Vm};

/// What a hooked property access does.
pub(crate) enum HookRoute {
    /// Run this hook.
    Call(ResolvedMethod),
    /// Use the backing slot, as for a plain property.
    Backing,
    /// Fail with this error.
    Fail(Thrown),
}

fn err(msg: String) -> Thrown {
    Thrown::error("Error", msg)
}

impl Vm {
    /// The declared entry `key` of `cls` when it has hooks.
    pub(crate) fn hooked_entry(&self, cls: ClassId, key: &PhpStr) -> Option<&PropEntry> {
        let e = &self.classes[cls as usize];
        if !e.hooked {
            return None;
        }
        e.props.get(key).filter(|pe| pe.hooks.is_some())
    }

    /// php's zend_is_in_hook + zend_should_call_hook: false while one of
    /// this property's own hooks (any in its prototype chain) runs on `h`.
    pub(crate) fn should_call_hook(&self, h: &ObjectHandle, pe: &PropEntry) -> bool {
        if self.hook_bypass {
            return false;
        }
        let Some(f) = self.stack.last() else {
            return true;
        };
        let Some(name) = f.proto.hook_prop() else {
            return true;
        };
        if name != pe.name.as_bytes() {
            return true;
        }
        let Some(d) = f.defining_class else {
            return true;
        };
        let de = &self.classes[d as usize];
        let same = de
            .own_private
            .get(&pe.name)
            .or_else(|| de.prop_visible.get(&pe.name))
            .and_then(|k| de.props.get(k))
            .is_some_and(|dp| dp.prototype == pe.prototype);
        if !same {
            return true;
        }
        let this = f
            .proto
            .this_cv
            .and_then(|cv| match f.regs.get(cv as usize) {
                // Also inside the hook of an initialized proxy whose
                // instance is `h`.
                Some(Value::Object(o)) => Some(
                    Rc::ptr_eq(o, h)
                        || o.borrow()
                            .lazy_instance()
                            .is_some_and(|i| Rc::ptr_eq(&i, h)),
                ),
                _ => None,
            })
            .unwrap_or(false);
        !this
    }

    /// The route of a read of hooked property `pe` on `h` (`cls` is its
    /// class, for messages).
    pub(crate) fn hook_read_route(
        &self,
        h: &ObjectHandle,
        cls: ClassId,
        pe: &PropEntry,
    ) -> HookRoute {
        let hooks = pe.hooks.as_ref().expect("hooked");
        let name = || {
            format!(
                "{}::${}",
                String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                String::from_utf8_lossy(pe.name.as_bytes())
            )
        };
        match &hooks[HookKind::Get as usize] {
            None if pe.virt => HookRoute::Fail(err(format!("Property {} is write-only", name()))),
            None => HookRoute::Backing,
            Some(get) if self.should_call_hook(h, pe) => HookRoute::Call(get.clone()),
            Some(_) if pe.virt => HookRoute::Fail(err(format!(
                "Must not read from virtual property {}",
                name()
            ))),
            Some(_) => HookRoute::Backing,
        }
    }

    /// The route of a write; `scope` for the asymmetric-visibility check.
    pub(crate) fn hook_write_route(
        &self,
        h: &ObjectHandle,
        cls: ClassId,
        key: &PhpStr,
        scope: Option<ClassId>,
        called: Option<ClassId>,
    ) -> HookRoute {
        let pe = &self.classes[cls as usize].props[key];
        let hooks = pe.hooks.as_ref().expect("hooked");
        let name = || {
            format!(
                "{}::${}",
                String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                String::from_utf8_lossy(pe.name.as_bytes())
            )
        };
        match &hooks[HookKind::Set as usize] {
            None if pe.virt => HookRoute::Fail(err(format!("Property {} is read-only", name()))),
            None => HookRoute::Backing,
            Some(set) if self.should_call_hook(h, pe) => {
                if let Err(e) = classes::set_access_ok(&self.classes, cls, scope, key) {
                    return HookRoute::Fail(classes::set_access_err(
                        &self.classes,
                        called,
                        "modify",
                        e,
                    ));
                }
                HookRoute::Call(set.clone())
            }
            Some(_) if pe.virt => HookRoute::Fail(err(format!(
                "Must not write to virtual property {}",
                name()
            ))),
            Some(_) => HookRoute::Backing,
        }
    }

    /// A write to the declared slot `key` of a hooked class: `Ok(None)` when a
    /// set hook took it, else the value back for the backing slot.
    #[cold]
    #[inline(never)]
    pub(crate) fn hooked_store(
        &mut self,
        h: &ObjectHandle,
        cls: ClassId,
        key: &PhpStr,
        scope: Option<ClassId>,
        called: Option<ClassId>,
        val: Value,
    ) -> Result<Option<Value>, Thrown> {
        if self.hooked_entry(cls, key).is_none() {
            return Ok(Some(val));
        }
        match self.hook_write_route(h, cls, key, scope, called) {
            HookRoute::Call(set) => self.call_set_hook(&set, h, val).map(|()| None),
            HookRoute::Fail(e) => Err(e),
            HookRoute::Backing => Ok(Some(val)),
        }
    }

    /// Run a get hook on `h`.
    pub(crate) fn call_get_hook(
        &mut self,
        get: &ResolvedMethod,
        h: &ObjectHandle,
    ) -> Result<Value, Thrown> {
        let cls = h.borrow().class;
        self.call_sync(get, Some(h.clone()), cls, Vec::new())
    }

    /// Run a set hook on `h` with `val`; the caller's `strict_types` decides
    /// how the value parameter is checked.
    pub(crate) fn call_set_hook(
        &mut self,
        set: &ResolvedMethod,
        h: &ObjectHandle,
        val: Value,
    ) -> Result<(), Thrown> {
        let cls = h.borrow().class;
        let strict = self
            .stack
            .iter()
            .rev()
            .find(|f| !f.proto.is_internal())
            .is_some_and(|f| f.proto.strict_types);
        self.call_sync_mode(set, Some(h.clone()), cls, vec![val], Some(strict))?;
        Ok(())
    }

    /// `Cannot unset hooked property C::$p`.
    pub(crate) fn hooked_unset_err(&self, cls: ClassId, pe: &PropEntry) -> Thrown {
        err(format!(
            "Cannot unset hooked property {}::${}",
            String::from_utf8_lossy(self.class(cls).name.as_bytes()),
            String::from_utf8_lossy(pe.name.as_bytes())
        ))
    }

    /// A write fetch through a hooked property (`$o->p[] = 1`, `&$o->p`,
    /// `$o->p->q = 1`): the get hook's result when it is a reference or an
    /// object, else php's "Indirect modification" Error.
    pub(crate) fn hook_write_fetch(
        &mut self,
        top: usize,
        h: &ObjectHandle,
        name: &PhpStr,
    ) -> Result<Value, Thrown> {
        // A prelude class's get_property_ptr_ptr refusal (DatePeriod).
        let cls = h.borrow().class;
        if let Some(f) = self.class(cls).handlers.and_then(|t| t.write_property) {
            f(self, h, name.as_bytes())?;
        }
        let got = self.prop_fetch_raw(top, h, name)?;
        match &got {
            Value::Ref(_) | Value::Object(_) => Ok(got),
            _ => {
                let cls = h.borrow().class;
                Err(err(format!(
                    "Indirect modification of {}::${} is not allowed",
                    String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                    String::from_utf8_lossy(name.as_bytes())
                )))
            }
        }
    }

    /// A read of `$h->name` that keeps a by-reference get hook's reference.
    fn prop_fetch_raw(
        &mut self,
        top: usize,
        h: &ObjectHandle,
        name: &PhpStr,
    ) -> Result<Value, Thrown> {
        let cls = h.borrow().class;
        let scope = self.stack[top].defining_class;
        if let PropRes::Slot(k) = classes::resolve_prop(&self.classes, cls, scope, name) {
            if let Some(r) = self.hooked_read(h, cls, &k) {
                return r;
            }
        }
        self.prop_fetch(top, Value::Object(h.clone()), name.clone())
    }

    /// A read of the declared slot `key` when it is hooked: the get hook's
    /// result, or its error; `None` to read the backing slot.
    #[cold]
    #[inline(never)]
    pub(crate) fn hooked_read(
        &mut self,
        h: &ObjectHandle,
        cls: ClassId,
        key: &PhpStr,
    ) -> Option<Result<Value, Thrown>> {
        let pe = self.hooked_entry(cls, key)?;
        match self.hook_read_route(h, cls, pe) {
            HookRoute::Backing => None,
            HookRoute::Fail(e) => Some(Err(e)),
            HookRoute::Call(get) => Some(self.call_get_hook(&get, h)),
        }
    }

    /// Whether a cursor step `$o->name` goes through a hook (php's
    /// get_property_ptr_ptr returning NULL for it).
    #[inline]
    pub(crate) fn cursor_hooked(&self, top: usize, h: &ObjectHandle, name: &PhpStr) -> bool {
        let cls = h.borrow().class;
        self.classes[cls as usize].hooked && self.cursor_hooked_slow(top, h, cls, name)
    }

    #[cold]
    #[inline(never)]
    fn cursor_hooked_slow(
        &self,
        top: usize,
        h: &ObjectHandle,
        cls: ClassId,
        name: &PhpStr,
    ) -> bool {
        let scope = self.stack[top].defining_class;
        let PropRes::Slot(k) = classes::resolve_prop(&self.classes, cls, scope, name) else {
            return false;
        };
        match self.hooked_entry(cls, &k) {
            Some(pe) => pe.virt || self.should_call_hook(h, pe),
            None => false,
        }
    }

    /// `parent::$prop::get()` / `::set()` (INIT_PARENT_PROPERTY_HOOK_CALL):
    /// the parent's hook as a call, or php's trampoline: a plain read or
    /// write of the property as from inside its hook.
    pub(crate) fn exec_parent_hook(
        &mut self,
        top: usize,
        dst: Reg,
        pname: PhpStr,
        kind: HookKind,
        args: HookArgs,
    ) -> Result<(), Thrown> {
        let f = &self.stack[top];
        let this = f.proto.this_cv.and_then(|cv| match &f.regs[cv as usize] {
            Value::Object(o) => Some(o.clone()),
            _ => None,
        });
        let called = f.called_class;
        let Some(parent) = f.defining_class.and_then(|d| self.class(d).parent) else {
            return Err(err(
                "Cannot use \"parent\" when current class scope has no parent".into(),
            ));
        };
        let pe = self
            .class(parent)
            .prop_visible
            .get(&pname)
            .and_then(|k| self.class(parent).props.get(k))
            .cloned();
        let pc = String::from_utf8_lossy(self.class(parent).name.as_bytes()).into_owned();
        let pn = String::from_utf8_lossy(pname.as_bytes()).into_owned();
        let Some(pe) = pe else {
            return Err(err(format!("Undefined property {pc}::${pn}")));
        };
        if pe.visibility == Visibility::Private {
            return Err(err(format!("Cannot access private property {pc}::${pn}")));
        }
        let Some(obj) = this else {
            return Err(err("Using $this when not in object context".into()));
        };
        if let Some(hook) = pe.hooks.as_ref().and_then(|h| h[kind as usize].clone()) {
            let cls = called.unwrap_or_else(|| obj.borrow().class);
            return match args {
                HookArgs::Window { base, argc } => {
                    self.enter_method(top, &hook, Some(obj), cls, dst, base, argc)
                }
                HookArgs::Array(r) => {
                    let arr = self.reg_get(top, r);
                    let (argv, named) = self.split_call_args(&arr)?;
                    let argv =
                        self.place_named_args(&hook.proto, Some(hook.defining_class), argv, named)?;
                    self.push_method_frame(
                        &hook,
                        Some(obj),
                        cls,
                        crate::calls::Args::Slice(&argv),
                        dst,
                    )
                }
            };
        }
        // php's trampoline takes one parameter, `$value`, for `set`.
        let argv: Vec<Value> = match args {
            HookArgs::Window { base, argc } => {
                (0..argc).map(|i| self.reg_get(top, base + i)).collect()
            }
            HookArgs::Array(r) => {
                let arr = self.reg_get(top, r);
                let (mut argv, named) = self.split_call_args(&arr)?;
                for (n, v) in named {
                    let shown = String::from_utf8_lossy(n.as_bytes()).into_owned();
                    if kind != HookKind::Set || n.as_bytes() != b"value" {
                        return Err(err(format!("Unknown named parameter ${shown}")));
                    }
                    if !argv.is_empty() {
                        return Err(err(format!(
                            "Named parameter ${shown} overwrites previous argument"
                        )));
                    }
                    argv.push(v);
                }
                argv
            }
        };
        let argc = argv.len();
        let decl =
            String::from_utf8_lossy(self.class(pe.declaring_class).name.as_bytes()).into_owned();
        let want = match kind {
            HookKind::Get => 0,
            HookKind::Set => 1,
        };
        if argc != want {
            let s = if want == 1 { "" } else { "s" };
            return Err(Thrown::error(
                "ArgumentCountError",
                format!(
                    "{decl}::${pn}::{}() expects exactly {want} argument{s}, {argc} given",
                    kind.name()
                ),
            ));
        }
        let saved = std::mem::replace(&mut self.hook_bypass, true);
        let r = match kind {
            HookKind::Get => self.prop_fetch(top, Value::Object(obj), pname),
            HookKind::Set => {
                let v = deref(&argv[0]);
                self.prop_store(top, Value::Object(obj), pname, v.clone())
                    .map(|()| v)
            }
        };
        self.hook_bypass = saved;
        let v = r?;
        if dst != Reg::MAX {
            self.reg_set(top, dst, v);
        }
        Ok(())
    }

    /// The table a hooked class shows for `purpose` (see
    /// `Host::hooked_properties_for`).
    pub(crate) fn hooked_view(
        &mut self,
        obj: &ObjectHandle,
        purpose: rhp_builtins::zend::object_handlers::PropPurpose,
    ) -> Result<Option<PhpArray>, Thrown> {
        use rhp_builtins::zend::object_handlers::PropPurpose as P;
        let cls = obj.borrow().class;
        if !self.classes[cls as usize].hooked {
            return Ok(None);
        }
        let scope = match purpose {
            P::GetObjectVars => Some(self.stack.last().and_then(|f| f.defining_class)),
            P::Json | P::VarExport => None,
            _ => return Ok(None),
        };
        self.hooked_props_table(obj, scope).map(Some)
    }

    /// php's zho_build_properties_ex + the readers of get_object_vars /
    /// json_encode / var_export: every declared property (parents first,
    /// keyed as php's property table: mangled for non-public), read through
    /// its get hook from its declaring class's scope; a virtual property
    /// without a get hook is left out. Dynamic properties follow.
    /// `scope`: only the properties visible from it, by unmangled name
    /// (get_object_vars); `None` lists all under mangled names.
    pub(crate) fn hooked_props_table(
        &mut self,
        h: &ObjectHandle,
        scope: Option<Option<ClassId>>,
    ) -> Result<PhpArray, Thrown> {
        let cls = h.borrow().class;
        let props = &self.classes[cls as usize].props;
        let entries: Vec<(PhpStr, PropEntry)> = self
            .hooked_order(cls)
            .into_iter()
            .filter_map(|i| props.get_index(i))
            .map(|(k, pe)| (k.clone(), pe.clone()))
            .collect();
        let mut out = PhpArray::new();
        for (key, pe) in entries {
            if self.internal_slot(cls, &key) {
                continue;
            }
            let shown = match scope {
                Some(s) => match classes::slot_accessible(&self.classes, cls, s, &key) {
                    Some(n) => n,
                    None => continue,
                },
                None => classes::mangled_prop_key(self.class(cls), &key),
            };
            let v = match &pe.hooks {
                Some(hooks) => {
                    if hooks[HookKind::Get as usize].is_none() {
                        if pe.virt {
                            continue;
                        }
                        // A backed read without a get hook is silent:
                        // uninitialized reads as null.
                        let v = h.borrow().props().get(&key).map(deref);
                        out.insert(ArrayKey::from_php_str(&shown), v.unwrap_or(Value::Null));
                        continue;
                    }
                    let mut v = self.prop_fetch_as(
                        Value::Object(h.clone()),
                        pe.name.clone(),
                        Some(pe.declaring_class),
                    )?;
                    if let Value::Ref(rc) = v {
                        v = rc.borrow().clone();
                    }
                    v
                }
                None => match h.borrow().props().get(&key) {
                    Some(Value::Ref(rc)) if Rc::strong_count(rc) == 1 => rc.borrow().clone(),
                    Some(v) => v.clone(),
                    None => continue,
                },
            };
            out.insert(ArrayKey::from_php_str(&shown), v);
        }
        let dynamic: Vec<(PhpStr, Value)> = {
            let b = h.borrow();
            let entry = self.class(cls);
            b.props()
                .iter()
                .filter(|(k, _)| !entry.props.contains_key(*k))
                .map(|(k, v)| (k.clone(), deref(v)))
                .collect()
        };
        for (k, v) in dynamic {
            if let Some(s) = scope {
                if classes::slot_accessible(&self.classes, cls, s, &k).is_none() {
                    continue;
                }
            }
            out.insert(ArrayKey::from_php_str(&k), v);
        }
        Ok(out)
    }
}

impl Vm {
    /// Whether foreach takes the hooked iterator: php's ce->get_iterator,
    /// set for a class with hooked properties and inherited by its children
    /// (a child that implements an abstract hooked property keeps it).
    pub(crate) fn iterates_hooked(&self, cls: ClassId) -> bool {
        let mut c = Some(cls);
        while let Some(id) = c {
            let e = &self.classes[id as usize];
            if e.hooked {
                return true;
            }
            c = e.parent;
        }
        false
    }

    /// The order zho_build_properties_ex walks `cls`'s properties in, as
    /// indexes into `props`: the root class's first, then each child's new
    /// ones. It differs from the slot order only where a redeclaration of an
    /// inherited virtual property took a new slot at the end.
    fn hooked_order(&self, cls: ClassId) -> Vec<usize> {
        let props = &self.classes[cls as usize].props;
        let Some(p) = self.classes[cls as usize].parent else {
            return (0..props.len()).collect();
        };
        let mut seen = vec![false; props.len()];
        let mut out = Vec::with_capacity(props.len());
        let pprops = &self.classes[p as usize].props;
        for pi in self.hooked_order(p) {
            let Some((k, _)) = pprops.get_index(pi) else {
                continue;
            };
            if let Some(i) = props.get_index_of(k) {
                if !seen[i] {
                    seen[i] = true;
                    out.push(i);
                }
            }
        }
        out.extend((0..props.len()).filter(|&i| !seen[i]));
        out
    }
}

/// One step of a foreach over a hooked object (zend_hooked_object_get_iterator).
enum IterStep {
    /// A plain slot, by storage key.
    Slot(PhpStr),
    /// A hooked property: (name, declaring class).
    Hooked(PhpStr, ClassId),
}

impl Vm {
    /// The foreach state of a hooked object: `[position, plan]`, the plan
    /// listing `[shown name, storage key or property name, declaring class
    /// or -1]` for the properties visible from `scope`, declared first.
    pub(crate) fn hooked_iter_state(&self, h: &ObjectHandle, scope: Option<ClassId>) -> Value {
        let b = h.borrow();
        let cls = b.class;
        let entry = self.class(cls);
        let mut plan = PhpArray::new();
        let mut add = |shown: PhpStr, key: PhpStr, decl: i64| {
            let mut e = PhpArray::new();
            e.push(Value::Str(shown));
            e.push(Value::Str(key));
            e.push(Value::Int(decl));
            plan.push(Value::Array(e));
        };
        for (key, pe) in self
            .hooked_order(cls)
            .into_iter()
            .filter_map(|i| entry.props.get_index(i))
        {
            if self.internal_slot(cls, key) {
                continue;
            }
            let Some(shown) = classes::slot_accessible(&self.classes, cls, scope, key) else {
                continue;
            };
            if pe.hooks.is_some() {
                add(shown, pe.name.clone(), pe.declaring_class as i64);
            } else {
                add(shown, key.clone(), -1);
            }
        }
        let mut st = PhpArray::new();
        st.push(Value::Int(0));
        st.push(Value::Array(plan));
        // Dynamic properties are walked live, by position.
        st.push(Value::Int(0));
        st.push(match scope {
            Some(s) => Value::Int(s as i64),
            None => Value::Int(-1),
        });
        Value::Array(st)
    }

    /// The next element of a foreach over a hooked object: `(state, key,
    /// value)`, the value a reference when `by_ref`; `None` at the end.
    pub(crate) fn hooked_iter_next(
        &mut self,
        h: &ObjectHandle,
        state: &Value,
        by_ref: bool,
    ) -> Result<Option<(Value, Value, Value)>, Thrown> {
        let Value::Array(st) = state else {
            return Ok(None);
        };
        let at = |i: usize| st.nth(i).map(|e| e.1.clone());
        let (Some(Value::Int(mut pos)), Some(Value::Array(plan))) = (at(0), at(1)) else {
            return Ok(None);
        };
        let mut dyn_pos = match at(2) {
            Some(Value::Int(n)) => n as usize,
            _ => 0,
        };
        let scope = match at(3) {
            Some(Value::Int(n)) if n >= 0 => Some(n as ClassId),
            _ => None,
        };
        let cls = h.borrow().class;
        let state = |pos: i64, plan: &PhpArray, dyn_pos: usize| {
            let mut nst = PhpArray::new();
            nst.push(Value::Int(pos));
            nst.push(Value::Array(plan.clone()));
            nst.push(Value::Int(dyn_pos as i64));
            nst.push(match scope {
                Some(s) => Value::Int(s as i64),
                None => Value::Int(-1),
            });
            Value::Array(nst)
        };
        while let Some((_, Value::Array(e))) = plan.nth(pos as usize) {
            pos += 1;
            let get = |i: usize| e.nth(i).map(|x| x.1.clone()).unwrap_or(Value::Null);
            let (Value::Str(shown), Value::Str(key), Value::Int(decl)) = (get(0), get(1), get(2))
            else {
                continue;
            };
            let step = if decl < 0 {
                IterStep::Slot(key)
            } else {
                IterStep::Hooked(key, decl as ClassId)
            };
            let v = match step {
                IterStep::Slot(key) => {
                    let mut b = h.borrow_mut();
                    let Some(slot) = b.props_mut().get_mut(&key) else {
                        continue;
                    };
                    if !by_ref {
                        deref(slot)
                    } else {
                        if let Some(pe) = self.classes[cls as usize].props.get(&key) {
                            if pe.readonly {
                                return Err(err(format!(
                                    "Cannot acquire reference to readonly property {}::${}",
                                    String::from_utf8_lossy(
                                        self.class(pe.declaring_class).name.as_bytes()
                                    ),
                                    String::from_utf8_lossy(pe.name.as_bytes())
                                )));
                            }
                        }
                        let rc = match slot {
                            Value::Ref(rc) => rc.clone(),
                            other => {
                                let inner = std::mem::replace(other, Value::Null);
                                let rc = Rc::new(std::cell::RefCell::new(inner));
                                *other = Value::Ref(rc.clone());
                                rc
                            }
                        };
                        drop(b);
                        self.add_prop_ref_source(&rc, h, &key);
                        Value::Ref(rc)
                    }
                }
                IterStep::Hooked(name, decl) => {
                    let pk = classes::prop_key(&self.classes, cls, Some(decl), &name)?;
                    let Some(pe) = self.classes[cls as usize].props.get(&pk).cloned() else {
                        continue;
                    };
                    let get = pe
                        .hooks
                        .as_ref()
                        .and_then(|h| h[HookKind::Get as usize].clone());
                    if get.is_none() && pe.virt {
                        continue;
                    }
                    if by_ref && !get.as_ref().is_some_and(|g| g.proto.returns_ref) {
                        return Err(err(format!(
                            "Cannot create reference to property {}::${}",
                            String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                            String::from_utf8_lossy(name.as_bytes())
                        )));
                    }
                    match (by_ref, get) {
                        (true, Some(g)) => match self.call_get_hook(&g, h)? {
                            r @ Value::Ref(_) => r,
                            other => Value::Ref(Rc::new(std::cell::RefCell::new(other))),
                        },
                        _ => {
                            // An uninitialized backed property is skipped.
                            if get_is_backing_uninit(self, h, &pe, &pk) {
                                continue;
                            }
                            let v =
                                self.prop_fetch_as(Value::Object(h.clone()), name, Some(decl))?;
                            deref(&v)
                        }
                    }
                }
            };
            return Ok(Some((state(pos, &plan, dyn_pos), Value::Str(shown), v)));
        }
        // Then the dynamic properties, as they are now.
        loop {
            let mut b = h.borrow_mut();
            let Some((key, slot)) = b.props_mut().get_index_mut(dyn_pos) else {
                return Ok(None);
            };
            dyn_pos += 1;
            if self.classes[cls as usize].props.contains_key(key) {
                continue;
            }
            let key = key.clone();
            if classes::slot_accessible(&self.classes, cls, scope, &key).is_none() {
                continue;
            }
            let v = if by_ref {
                match slot {
                    Value::Ref(rc) => Value::Ref(rc.clone()),
                    other => {
                        let inner = std::mem::replace(other, Value::Null);
                        let rc = Rc::new(std::cell::RefCell::new(inner));
                        *other = Value::Ref(rc.clone());
                        Value::Ref(rc)
                    }
                }
            } else {
                deref(slot)
            };
            drop(b);
            return Ok(Some((state(pos, &plan, dyn_pos), Value::Str(key), v)));
        }
    }
}

/// A backed property without a get hook whose slot is uninitialized.
fn get_is_backing_uninit(vm: &Vm, h: &ObjectHandle, pe: &PropEntry, key: &PhpStr) -> bool {
    pe.hooks
        .as_ref()
        .is_some_and(|hk| hk[HookKind::Get as usize].is_none())
        && !pe.virt
        && !h.borrow().props().contains_key(key)
        && vm.class(h.borrow().class).props.contains_key(key)
}

impl Vm {
    /// A class named by a type, autoloaded when unknown (php's variance
    /// checks load the classes they compare).
    fn type_class(&mut self, name: &[u8], scope: ClassId) -> Option<ClassId> {
        let lc = name.to_ascii_lowercase();
        match lc.as_slice() {
            b"self" => return Some(scope),
            b"parent" => return self.class(scope).parent,
            _ => {}
        }
        if let Ok(c) = self.resolve_class_by_name(name) {
            return Some(c);
        }
        match self.try_autoload(name) {
            Ok(true) => self.resolve_class_by_name(name).ok(),
            _ => None,
        }
    }

    /// Whether every value of type `sub` (declared in `sub_scope`) is
    /// accepted by `sup` (in `sup_scope`): a simple form of php's
    /// zend_perform_covariant_type_check, enough for property hook value
    /// types. An unknown class is incompatible.
    pub(crate) fn type_is_subtype(
        &mut self,
        sub: &rhp_bytecode::TypeHint,
        sub_scope: ClassId,
        sup: &rhp_bytecode::TypeHint,
        sup_scope: ClassId,
    ) -> bool {
        use rhp_bytecode::TypeKind as K;
        if sup.kinds.contains(&K::Mixed) {
            return true;
        }
        if sub.kinds.contains(&K::Mixed) {
            return false;
        }
        let sup_null = sup.nullable || sup.kinds.contains(&K::Null);
        if (sub.nullable || sub.kinds.contains(&K::Null)) && !sup_null {
            return false;
        }
        let mut atoms: Vec<K> = sub.kinds.clone();
        // An intersection is a subtype when any of its members is.
        let sub_groups: Vec<Vec<PhpStr>> = if sub.intersection {
            let g = atoms
                .drain(..)
                .filter_map(|k| match k {
                    K::Named(n) => Some(n),
                    _ => None,
                })
                .collect();
            vec![g]
        } else {
            sub.groups.clone()
        };
        for g in sub_groups {
            let mut ok = false;
            for n in g {
                if self.atom_in(&K::Named(n), sub_scope, sup, sup_scope) {
                    ok = true;
                    break;
                }
            }
            if !ok {
                return false;
            }
        }
        for a in atoms {
            if a == K::Null {
                continue;
            }
            if !self.atom_in(&a, sub_scope, sup, sup_scope) {
                return false;
            }
        }
        true
    }

    fn atom_in(
        &mut self,
        a: &rhp_bytecode::TypeKind,
        a_scope: ClassId,
        sup: &rhp_bytecode::TypeHint,
        sup_scope: ClassId,
    ) -> bool {
        use rhp_bytecode::TypeKind as K;
        let class_of = |k: &K| -> Option<Vec<u8>> {
            match k {
                K::Named(n) => Some(n.as_bytes().to_vec()),
                K::SelfType | K::StaticType => Some(b"self".to_vec()),
                K::Parent => Some(b"parent".to_vec()),
                _ => None,
            }
        };
        let a_class = match class_of(a) {
            Some(n) => match self.type_class(&n, a_scope) {
                Some(c) => Some(c),
                None => {
                    // php still loads the other side's classes.
                    for k in &sup.kinds {
                        if let Some(n) = class_of(k) {
                            self.type_class(&n, sup_scope);
                        }
                    }
                    return false;
                }
            },
            None => None,
        };
        if sup.intersection {
            let Some(c) = a_class else { return false };
            for k in &sup.kinds {
                let Some(n) = class_of(k) else { return false };
                match self.type_class(&n, sup_scope) {
                    Some(d) if self.class(c).ancestors.contains(&d) => {}
                    _ => return false,
                }
            }
            return true;
        }
        for k in &sup.kinds {
            let hit = match (a, k) {
                (x, y) if x == y && a_class.is_none() => true,
                (K::True | K::False, K::Bool) => true,
                (K::Array, K::Iterable) => true,
                _ => match a_class {
                    Some(c) => match k {
                        K::Object => true,
                        K::Iterable => {
                            let t = self.resolve_class_by_name(b"Traversable").ok();
                            t.is_some_and(|t| self.class(c).ancestors.contains(&t))
                        }
                        _ => match class_of(k) {
                            Some(n) => match self.type_class(&n, sup_scope) {
                                Some(d) => self.class(c).ancestors.contains(&d),
                                None => false,
                            },
                            None => false,
                        },
                    },
                    None => false,
                },
            };
            if hit {
                return true;
            }
        }
        for g in &sup.groups {
            let Some(c) = a_class else { continue };
            let mut all = true;
            for n in g {
                match self.type_class(n.as_bytes(), sup_scope) {
                    Some(d) if self.class(c).ancestors.contains(&d) => {}
                    _ => {
                        all = false;
                        break;
                    }
                }
            }
            if all {
                return true;
            }
        }
        false
    }

    /// The type checks of hooked properties once the class is linked:
    /// a set hook's value type must accept the property type
    /// (zend_verify_property_hook_variance), and a property implementing
    /// an abstract set hook must accept the hook's value type
    /// (verify_property_type_compatibility).
    pub(crate) fn verify_hook_types(&mut self, id: ClassId) -> Result<(), Thrown> {
        let entries: Vec<PropEntry> = self.class(id).props.values().cloned().collect();
        let cn = String::from_utf8_lossy(self.class(id).name.as_bytes()).into_owned();
        for pe in entries {
            let pn = String::from_utf8_lossy(pe.name.as_bytes()).into_owned();
            let set = pe
                .hooks
                .as_ref()
                .and_then(|h| h[HookKind::Set as usize].clone());
            if pe.declaring_class == id {
                if let Some(set) = set.as_ref().filter(|s| s.defining_class == id) {
                    let p = set.proto.params.first();
                    if let (Some(vt), Some(pt)) = (p.and_then(|p| p.type_hint.as_ref()), &pe.ty) {
                        let (vt, pt) = (vt.clone(), (**pt).clone());
                        if !self.type_is_subtype(&pt, id, &vt, id) {
                            let vname = p
                                .map(|p| set.proto.names[p.name as usize].clone())
                                .unwrap_or_default();
                            let msg = format!(
                                "Type of parameter ${} of hook {cn}::${pn}::set must be compatible with property type",
                                String::from_utf8_lossy(vname.as_bytes())
                            );
                            let proto = self.class(id).proto.clone();
                            return Err(self.link_fatal(&proto, 64, &msg));
                        }
                    }
                }
            }
            // A property that implements an inherited abstract set hook
            // with a plain (backed) set.
            if pe.declaring_class != id || set.is_some() {
                continue;
            }
            let Some(ty) = pe.ty.clone() else { continue };
            let abstract_set = self.inherited_abstract_set(id, &pe);
            if let Some((owner, hook)) = abstract_set {
                let Some(st) = hook.proto.params.first().and_then(|p| p.type_hint.clone()) else {
                    continue;
                };
                if !self.type_is_subtype(&st, owner, &ty, id) {
                    let shown = self.type_string_resolved(&st, Some(owner), Some(owner));
                    let kind = match self.class(owner).kind {
                        rhp_values::ClassKind::Interface => "interface",
                        rhp_values::ClassKind::Trait => "trait",
                        _ => "class",
                    };
                    let msg = format!(
                        "Set type of {cn}::${pn} must be supertype of {shown} (as in {kind} {})",
                        String::from_utf8_lossy(self.class(owner).name.as_bytes())
                    );
                    let proto = self.class(id).proto.clone();
                    return Err(self.link_fatal(&proto, 64, &msg));
                }
            }
        }
        Ok(())
    }

    /// The abstract set hook of `pe`'s name in the parent or an implemented
    /// interface of `id`, with its owner.
    fn inherited_abstract_set(
        &self,
        id: ClassId,
        pe: &PropEntry,
    ) -> Option<(ClassId, ResolvedMethod)> {
        let e = self.class(id);
        let mut owners: Vec<ClassId> = e.parent.into_iter().collect();
        owners.extend(e.interfaces.iter().copied());
        for o in owners {
            let oe = self.class(o);
            let Some(ope) = oe.prop_visible.get(&pe.name).and_then(|k| oe.props.get(k)) else {
                continue;
            };
            if !ope.is_abstract {
                continue;
            }
            if let Some(h) = ope
                .hooks
                .as_ref()
                .and_then(|h| h[HookKind::Set as usize].clone())
            {
                if h.is_abstract {
                    return Some((ope.declaring_class, h));
                }
            }
        }
        None
    }
}

/// A parent hook call's arguments: a window, or a packed arguments array
/// (with named arguments).
#[derive(Clone, Copy)]
pub(crate) enum HookArgs {
    Window { base: Reg, argc: u16 },
    Array(Reg),
}
