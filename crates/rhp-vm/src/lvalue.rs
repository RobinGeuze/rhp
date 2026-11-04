//! Array element reads and lvalue paths (the cursor opcodes): nested writes,
//! references, unset, variable variables and ArrayAccess.

use std::cell::RefCell;
use std::rc::Rc;

use rhp_builtins as builtins;
use rhp_bytecode::Reg;
use rhp_values::{
    conv, ArrayKey, ClassId, ObjectData, ObjectHandle, PhpArray, PhpStr, Thrown, Value,
};

use crate::classes::PropRes;
use crate::types::type_lc;
use crate::{classes, deref, Vm};

/// A pending lvalue step (an evaluated key, an append, or an object property).
#[derive(Clone)]
pub(crate) enum CursorStep {
    Key(Value),
    Append,
    Prop(rhp_values::PhpStr),
}

/// Where a cursor path first meets a property php routes through `__get`
/// (missing or inaccessible on its object, with an unguarded `__get`).
pub(crate) enum MagicSplit {
    /// No such property: the plain path functions handle the whole path.
    Plain,
    /// The final step is such a property of this object.
    Leaf(ObjectHandle, PhpStr),
    /// Step `.0` (not the last) is such a property: writes below it go into
    /// whatever `__get` returns.
    Nested(usize, ObjectHandle, PhpStr),
    /// Step `.0` (a key / append) indexes this ArrayAccess object: the leaf
    /// is `offsetSet`/`offsetUnset`, a deeper path goes through `offsetGet`.
    Offset(usize, ObjectHandle),
}

/// How an ArrayAccess element is fetched below a nested write (php's
/// BP_VAR_W / BP_VAR_RW / BP_VAR_UNSET).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DimFetch {
    W,
    Rw,
    Unset,
}

/// Where an lvalue cursor is rooted.
#[derive(Clone)]
pub(crate) enum CursorRoot {
    /// A register (compiled variable) in the current frame.
    Cv(Reg),
    /// A shared static-property cell.
    Cell(Rc<RefCell<Value>>),
}

/// The fetch kind of a dimension read: php's `BP_VAR_R`, `BP_VAR_IS`
/// (isset / `??`) and `ZEND_FETCH_LIST_R`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DimMode {
    Read,
    Is,
    List,
}

/// Read the value at a cursor path (Null if any step is absent); non-mutating.
/// Property-name resolution context for cursor paths: the class table and
/// the calling scope, so `$o->p[...]` / `&$o->p` hit the same slot (and the
/// same visibility errors) as a plain `$o->p`.
struct PropCtx<'a> {
    classes: &'a [classes::ClassEntry],
    scope: Option<ClassId>,
    /// The called (LSB) class, named by asymmetric-visibility errors.
    called: Option<ClassId>,
}

impl PropCtx<'_> {
    /// Storage key for `name` on an instance of `cls`, or the access error.
    fn key(&self, cls: ClassId, name: &PhpStr) -> Result<PhpStr, Thrown> {
        classes::prop_key(self.classes, cls, self.scope, name)
    }

    /// Storage key for writing through `$o->name` (with `rest` steps still to
    /// go): also enforces `private(set)` / `protected(set)`. Writing into an
    /// object held by such a property is allowed (handle semantics); anything
    /// else is php's "Cannot (indirectly) modify private(set) property".
    fn write_key(
        &self,
        od: &ObjectData,
        name: &PhpStr,
        leaf_op: &str,
        rest: &[CursorStep],
    ) -> Result<PhpStr, Thrown> {
        let key = self.key(od.class, name)?;
        if let Err(e) = classes::set_access_ok(self.classes, od.class, self.scope, &key) {
            let holds_object = !rest.is_empty()
                && matches!(od.properties.get(&key).map(deref), Some(Value::Object(_)));
            if !holds_object {
                let op = if rest.is_empty() {
                    leaf_op
                } else {
                    "indirectly modify"
                };
                return Err(classes::set_access_err(self.classes, self.called, op, e));
            }
        }
        Ok(key)
    }
}

thread_local! {
    /// Diagnostics raised inside the pure cursor-path walkers
    /// ([`assign_path`] / [`make_ref_path`] / [`unset_path`]), emitted by
    /// the VM right after the walk ([`Vm::flush_path_diags`]).
    static PATH_DIAGS: RefCell<Vec<(i64, String)>> = const { RefCell::new(Vec::new()) };
}

impl Vm {
    /// Read the value at the current cursor (root + steps) for a compound
    /// assignment / increment: a magic property along the way reads via `__get`.
    pub(crate) fn cursor_read(&mut self, top: usize) -> Result<Value, Thrown> {
        self.cursor_settle_offsets(top)?;
        let steps = std::mem::take(&mut self.cursor_steps);
        let r = self.cursor_read_magic(top, &steps, false);
        self.cursor_steps = steps;
        r
    }

    /// `incdec`: for `++`/`--` (else a compound assignment), which only
    /// changes the error for a string offset.
    pub(crate) fn cursor_read_magic(
        &mut self,
        top: usize,
        steps: &[CursorStep],
        incdec: bool,
    ) -> Result<Value, Thrown> {
        match self.cursor_magic_split(top, steps) {
            MagicSplit::Plain => {
                self.cursor_rw_checks(top, steps, incdec)?;
                Ok(self.cursor_read_steps(top, steps))
            }
            MagicSplit::Leaf(h, name) => self.prop_fetch(top, Value::Object(h), name),
            MagicSplit::Nested(i, h, name) => {
                let got = self.fetching_as(builtins::MagicFetch::Write, |vm| {
                    vm.prop_fetch(top, Value::Object(h), name)
                })?;
                let rest = &steps[i + 1..];
                if let Value::Object(o) = deref(&got) {
                    // An object holds on: its own magic or ArrayAccess
                    // takes the rest of the path.
                    let cell = Rc::new(RefCell::new(Value::Object(o)));
                    return self
                        .with_cursor_root(cell, |vm| vm.cursor_read_magic(top, rest, incdec));
                }
                let cx = PropCtx {
                    classes: &self.classes,
                    scope: self.stack[top].defining_class,
                    called: self.stack[top].called_class,
                };
                Ok(read_path(&got, rest, &cx))
            }
            MagicSplit::Offset(i, h) => {
                let key = aa_key(&steps[i]).unwrap_or(Value::Null);
                if i + 1 == steps.len() {
                    return Ok(deref(&self.aa_offset_get(&h, key)?));
                }
                let cell = self.aa_fetch_w(&h, &steps[i], key, DimFetch::Rw)?;
                let rest = &steps[i + 1..];
                self.with_cursor_root(cell, |vm| vm.cursor_read_magic(top, rest, incdec))
            }
        }
    }

    /// Drop the cursor's hold on a by-name / static-property cell once the
    /// write is done, so a value released later (`unset($$n)`) is freed then.
    pub(crate) fn cursor_release(&mut self) {
        if matches!(self.cursor_root, CursorRoot::Cell(_)) {
            self.cursor_root = CursorRoot::Cv(0);
        }
    }

    /// The diagnostics of php's read-for-write fetches along a compound
    /// assignment / `++` path (FETCH_DIM_RW / FETCH_OBJ_RW, then the leaf
    /// read): "Undefined array key" for each key missing from an array (or
    /// indexed under a null about to be autovivified), "Undefined property"
    /// for an absent property, and the "must not be accessed before
    /// initialization" Error for an uninitialized typed one. Containers the
    /// write itself rejects (scalars, strings, ArrayAccess) end the walk.
    fn cursor_rw_checks(
        &mut self,
        top: usize,
        steps: &[CursorStep],
        incdec: bool,
    ) -> Result<(), Thrown> {
        // `None`: the slot does not exist yet (the write creates it as null).
        let mut cur = Some(deref(&self.cursor_root_value(top)));
        let mut warnings: Vec<String> = Vec::new();
        for (i, step) in steps.iter().enumerate() {
            cur = match (cur, step) {
                // A string container: no read-modify-write through its offsets.
                (Some(Value::Str(_)), CursorStep::Append) => {
                    self.flush_rw_warnings(warnings)?;
                    return Err(Thrown::error(
                        "Error",
                        "[] operator not supported for strings",
                    ));
                }
                (Some(Value::Str(_)), CursorStep::Key(k)) => {
                    self.flush_rw_warnings(warnings)?;
                    let cx = PropCtx {
                        classes: &self.classes,
                        scope: self.stack[top].defining_class,
                        called: self.stack[top].called_class,
                    };
                    let checked = string_offset(k, false, &cx);
                    self.flush_path_diags()?;
                    checked?;
                    let rest = &steps[i + 1..];
                    return Err(match rest.first() {
                        None if incdec => {
                            Thrown::error("Error", "Cannot increment/decrement string offsets")
                        }
                        None => Thrown::error(
                            "Error",
                            "Cannot use assign-op operators with string offsets",
                        ),
                        Some(_) => string_offset_misuse(rest, false),
                    });
                }
                (None | Some(Value::Null), CursorStep::Key(k)) => {
                    match ArrayKey::from_value(&deref(k)) {
                        Some(key) => warnings.push(undefined_key_msg(&key)),
                        None => break,
                    }
                    None
                }
                (Some(Value::Array(a)), CursorStep::Key(k)) => {
                    let Some(key) = ArrayKey::from_value(&deref(k)) else {
                        break;
                    };
                    match a.get(&key) {
                        Some(v) => Some(deref(v)),
                        None => {
                            warnings.push(undefined_key_msg(&key));
                            None
                        }
                    }
                }
                (None | Some(Value::Null) | Some(Value::Array(_)), CursorStep::Append) => None,
                (Some(Value::Object(h)), CursorStep::Prop(name)) => {
                    let cls = h.borrow().class;
                    let scope = self.stack[top].defining_class;
                    let (key, dynamic) =
                        match classes::resolve_prop(&self.classes, cls, scope, name) {
                            PropRes::Slot(k) => (k, false),
                            PropRes::Dynamic => (name.clone(), true),
                            PropRes::Denied(_) => break,
                        };
                    let present = h.borrow().properties.get(&key).map(deref);
                    match present {
                        Some(v) => Some(v),
                        None => {
                            // Absent and typed: never initialized, or unset
                            // with no `__get` to defer to.
                            if let Some(e) = self.typed_slot(cls, &key) {
                                return Err(e);
                            }
                            if dynamic {
                                // zend_std_get_property_ptr_ptr (BP_VAR_RW):
                                // the dynamic-property rules, then the
                                // property exists (null) before the warning.
                                self.flush_rw_warnings(std::mem::take(&mut warnings))?;
                                self.check_dynamic_prop(top, cls, name)?;
                                h.borrow_mut().properties.insert(key, Value::Null);
                            }
                            warnings.push(format!(
                                "Undefined property: {}::${}",
                                String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                                String::from_utf8_lossy(name.as_bytes())
                            ));
                            None
                        }
                    }
                }
                _ => break,
            };
        }
        self.flush_rw_warnings(warnings)
    }

    fn flush_rw_warnings(&mut self, warnings: Vec<String>) -> Result<(), Thrown> {
        for w in warnings {
            self.diag(2, &w)?;
        }
        Ok(())
    }

    /// The cursor root's current value.
    fn cursor_root_value(&self, top: usize) -> Value {
        match &self.cursor_root {
            CursorRoot::Cv(r) => self.stack[top].regs[*r as usize].clone(),
            CursorRoot::Cell(rc) => rc.borrow().clone(),
        }
    }

    /// Walk the cursor path to the first property php would route through
    /// `__get` (zend_std_get_property_ptr_ptr returns NULL for a missing or
    /// inaccessible property when the class has an unguarded `__get`).
    pub(crate) fn cursor_magic_split(&self, top: usize, steps: &[CursorStep]) -> MagicSplit {
        let scope = self.stack[top].defining_class;
        let mut cur = self.cursor_root_value(top);
        for (i, step) in steps.iter().enumerate() {
            let next = match (deref(&cur), step) {
                (Value::Array(a), CursorStep::Key(k)) => {
                    match ArrayKey::from_value(k).and_then(|key| a.get(&key).cloned()) {
                        Some(v) => v,
                        None => return MagicSplit::Plain,
                    }
                }
                (Value::Object(h), CursorStep::Prop(name)) => {
                    let cls = h.borrow().class;
                    let present = match classes::resolve_prop(&self.classes, cls, scope, name) {
                        PropRes::Slot(k) => h.borrow().properties.get(&k).cloned(),
                        PropRes::Dynamic => h.borrow().properties.get(name).cloned(),
                        PropRes::Denied(_) => None,
                    };
                    let uninit = match classes::resolve_prop(&self.classes, cls, scope, name) {
                        PropRes::Slot(k) => present.is_none() && self.uninit_typed(&h, &k),
                        _ => false,
                    };
                    match present {
                        Some(v) => v,
                        None if uninit => return MagicSplit::Plain,
                        None if self.magic(&h, cls, b"__get", name).is_some() => {
                            return if i + 1 == steps.len() {
                                MagicSplit::Leaf(h, name.clone())
                            } else {
                                MagicSplit::Nested(i, h, name.clone())
                            };
                        }
                        None => return MagicSplit::Plain,
                    }
                }
                (Value::Object(h), CursorStep::Key(_) | CursorStep::Append) => {
                    let cls = h.borrow().class;
                    return if self.class_implements(cls, b"ArrayAccess") {
                        MagicSplit::Offset(i, h)
                    } else {
                        MagicSplit::Plain
                    };
                }
                _ => return MagicSplit::Plain,
            };
            cur = next;
        }
        MagicSplit::Plain
    }

    /// How many compiler-temporary registers (not CVs) across the live frames
    /// hold a `Ref` to `rc`. Such leftovers are not PHP-visible references.
    pub(crate) fn temp_ref_holders(&self, rc: &Rc<RefCell<Value>>) -> usize {
        self.stack
            .iter()
            .map(|f| {
                let ncv = f.proto.num_cvs as usize;
                f.regs
                    .iter()
                    .skip(ncv)
                    .filter(|v| matches!(v, Value::Ref(r) if Rc::ptr_eq(r, rc)))
                    .count()
            })
            .sum()
    }

    /// Run `f` with the cursor temporarily re-rooted at `cell`.
    fn with_cursor_root<T>(
        &mut self,
        cell: Rc<RefCell<Value>>,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let saved = std::mem::replace(&mut self.cursor_root, CursorRoot::Cell(cell));
        let r = f(self);
        self.cursor_root = saved;
        r
    }

    /// The cell a nested write below a magic property lands in: the `__get`
    /// result itself when it is a reference, else a temporary (php: "Indirect
    /// modification of overloaded property has no effect", which still lets
    /// writes reach an object the temporary holds).
    fn magic_write_cell(
        &mut self,
        top: usize,
        h: ObjectHandle,
        name: PhpStr,
    ) -> Result<Rc<RefCell<Value>>, Thrown> {
        let got = self.fetching_as(builtins::MagicFetch::Write, |vm| {
            vm.prop_fetch(top, Value::Object(h), name)
        })?;
        Ok(match got {
            Value::Ref(rc) => rc,
            other => Rc::new(RefCell::new(other)),
        })
    }

    /// Promote the cursor target to a shared cell (`&$o->p[...]`, by-ref
    /// argument passing); a magic property along the way yields its `__get`
    /// result (or a temporary) like php's BP_VAR_W fetch.
    pub(crate) fn cursor_make_ref(
        &mut self,
        top: usize,
        steps: &[CursorStep],
    ) -> Result<Rc<RefCell<Value>>, Thrown> {
        self.cursor_readonly_check(top, steps, true)?;
        let (i, h, name) = match self.cursor_magic_split(top, steps) {
            MagicSplit::Plain => {
                self.cursor_dyn_check(top, steps, true)?;
                let cx = PropCtx {
                    classes: &self.classes,
                    scope: self.stack[top].defining_class,
                    called: self.stack[top].called_class,
                };
                let r = match &self.cursor_root {
                    CursorRoot::Cv(r) => {
                        let r = *r as usize;
                        make_ref_path(&mut self.stack[top].regs[r], steps, &cx)
                    }
                    CursorRoot::Cell(rc) => {
                        let rc = rc.clone();
                        let c = make_ref_path(&mut rc.borrow_mut(), steps, &cx);
                        c
                    }
                };
                self.flush_path_diags()?;
                return r;
            }
            MagicSplit::Leaf(h, name) => (steps.len() - 1, h, name),
            MagicSplit::Nested(i, h, name) => (i, h, name),
            MagicSplit::Offset(i, h) => {
                let key = aa_key(&steps[i]).unwrap_or(Value::Null);
                let cell = self.aa_fetch_w(&h, &steps[i], key, DimFetch::W)?;
                let rest = &steps[i + 1..];
                if rest.is_empty() {
                    return Ok(cell);
                }
                return self.with_cursor_root(cell, |vm| vm.cursor_make_ref(top, rest));
            }
        };
        let cell = self.magic_write_cell(top, h, name)?;
        let rest = &steps[i + 1..];
        if rest.is_empty() {
            return Ok(cell);
        }
        self.with_cursor_root(cell, |vm| vm.cursor_make_ref(top, rest))
    }

    fn cursor_read_steps(&self, top: usize, steps: &[CursorStep]) -> Value {
        let cx = PropCtx {
            classes: &self.classes,
            scope: self.stack[top].defining_class,
            called: self.stack[top].called_class,
        };
        match &self.cursor_root {
            CursorRoot::Cv(r) => read_path(&self.stack[top].regs[*r as usize], steps, &cx),
            CursorRoot::Cell(rc) => read_path(&rc.borrow(), steps, &cx),
        }
    }

    /// Readonly properties along a cursor path: writing the property itself
    /// follows the readonly rules; writing *through* it (`$o->ro[] = 1`) or
    /// taking a reference is "Cannot indirectly modify readonly property",
    /// unless it holds an object.
    fn cursor_readonly_check(
        &mut self,
        top: usize,
        steps: &[CursorStep],
        make_ref: bool,
    ) -> Result<(), Thrown> {
        let scope = self.stack[top].defining_class;
        let mut cur = self.cursor_root_value(top);
        for (i, step) in steps.iter().enumerate() {
            let next = match (deref(&cur), step) {
                (Value::Array(a), CursorStep::Key(k)) => {
                    match ArrayKey::from_value(k).and_then(|key| a.get(&key).cloned()) {
                        Some(v) => v,
                        None => return Ok(()),
                    }
                }
                (Value::Object(h), CursorStep::Prop(name)) => {
                    let cls = h.borrow().class;
                    let PropRes::Slot(key) = classes::resolve_prop(&self.classes, cls, scope, name)
                    else {
                        return Ok(());
                    };
                    let val = h.borrow().properties.get(&key).cloned();
                    if let Some(pe) = self.class(cls).props.get(&key).filter(|pe| pe.readonly) {
                        let last = i + 1 == steps.len();
                        if last && !make_ref {
                            self.check_readonly_write(&h, &key, val.is_some(), scope)?;
                        } else if !matches!(val.as_ref().map(deref), Some(Value::Object(_))) {
                            return Err(Thrown::error(
                                "Error",
                                format!(
                                    "Cannot indirectly modify readonly property {}::${}",
                                    String::from_utf8_lossy(
                                        self.class(pe.declaring_class).name.as_bytes()
                                    ),
                                    String::from_utf8_lossy(pe.name.as_bytes())
                                ),
                            ));
                        }
                    }
                    match val {
                        Some(v) => v,
                        None => return Ok(()),
                    }
                }
                _ => return Ok(()),
            };
            cur = next;
        }
        Ok(())
    }

    /// The dynamic-property rules (deprecation, or php's "Cannot create
    /// dynamic property" Error) for the first absent undeclared property
    /// a write or reference along `steps` creates (zend_std_get_property_
    /// ptr_ptr for the fetches, zend_std_write_property for a leaf with no
    /// `__set`). `make_ref`: the leaf is fetched for a reference.
    fn cursor_dyn_check(
        &mut self,
        top: usize,
        steps: &[CursorStep],
        make_ref: bool,
    ) -> Result<(), Thrown> {
        let scope = self.stack[top].defining_class;
        let mut cur = self.cursor_root_value(top);
        for (i, step) in steps.iter().enumerate() {
            let next = match (deref(&cur), step) {
                (Value::Array(a), CursorStep::Key(k)) => {
                    match ArrayKey::from_value(k).and_then(|key| a.get(&key).cloned()) {
                        Some(v) => v,
                        None => return Ok(()),
                    }
                }
                (Value::Object(h), CursorStep::Prop(name)) => {
                    let cls = h.borrow().class;
                    let (key, dynamic) =
                        match classes::resolve_prop(&self.classes, cls, scope, name) {
                            PropRes::Slot(k) => (k, false),
                            PropRes::Dynamic => (name.clone(), true),
                            PropRes::Denied(_) => return Ok(()),
                        };
                    let val = h.borrow().properties.get(&key).cloned();
                    match val {
                        Some(v) => v,
                        None => {
                            let leaf = i + 1 == steps.len();
                            let set =
                                !make_ref && leaf && self.magic(&h, cls, b"__set", name).is_some();
                            if dynamic && !set {
                                self.check_dynamic_prop(top, cls, name)?;
                            }
                            return Ok(());
                        }
                    }
                }
                _ => return Ok(()),
            };
            cur = next;
        }
        Ok(())
    }

    /// Write `v` at the current cursor (root + steps).
    pub(crate) fn cursor_write(
        &mut self,
        top: usize,
        steps: &[CursorStep],
        v: Value,
    ) -> Result<(), Thrown> {
        self.cursor_readonly_check(top, steps, false)?;
        match self.cursor_magic_split(top, steps) {
            MagicSplit::Plain => self.cursor_dyn_check(top, steps, false)?,
            MagicSplit::Leaf(h, name) => return self.prop_store(top, Value::Object(h), name, v),
            MagicSplit::Nested(i, h, name) => {
                let cell = self.magic_write_cell(top, h, name)?;
                let rest = &steps[i + 1..];
                return self.with_cursor_root(cell, |vm| vm.cursor_write(top, rest, v));
            }
            // `$obj[$k] = v` / `$obj[] = v` on an ArrayAccess object ->
            // offsetSet; below it, the element is fetched with offsetGet.
            MagicSplit::Offset(i, h) => {
                let key = aa_key(&steps[i]).unwrap_or(Value::Null);
                if i + 1 == steps.len() {
                    let cls = h.borrow().class;
                    if let Some(m) = self.lookup_method(cls, b"offsetset") {
                        self.call_sync(&m, Some(h), cls, vec![key, v])?;
                    }
                    return Ok(());
                }
                let cell = self.aa_fetch_w(&h, &steps[i], key, DimFetch::W)?;
                let rest = &steps[i + 1..];
                return self.with_cursor_root(cell, |vm| vm.cursor_write(top, rest, v));
            }
        }
        let cx = PropCtx {
            classes: &self.classes,
            scope: self.stack[top].defining_class,
            called: self.stack[top].called_class,
        };
        let r = match &self.cursor_root {
            CursorRoot::Cv(r) => {
                let r = *r as usize;
                assign_path(&mut self.stack[top].regs[r], steps, v, &cx)
            }
            CursorRoot::Cell(rc) => {
                let rc = rc.clone();
                let r = assign_path(&mut rc.borrow_mut(), steps, v, &cx);
                r
            }
        };
        self.flush_path_diags()?;
        // The overwritten element may have held the last reference to an
        // object: its destructor runs before the next statement, as in php.
        if !self.destructibles.is_empty() {
            self.dtor_check = true;
        }
        r
    }

    pub(crate) fn cursor_unset(&mut self, top: usize, steps: &[CursorStep]) -> Result<(), Thrown> {
        match self.cursor_magic_split(top, steps) {
            MagicSplit::Plain | MagicSplit::Leaf(..) => {}
            MagicSplit::Nested(i, h, name) => {
                let cell = self.magic_write_cell(top, h, name)?;
                let rest = &steps[i + 1..];
                return self.with_cursor_root(cell, |vm| vm.cursor_unset(top, rest));
            }
            // `unset($obj[$k])` on an ArrayAccess object -> offsetUnset; one
            // deeper in the path is fetched with offsetGet first.
            MagicSplit::Offset(i, h) => {
                let key = aa_key(&steps[i]).unwrap_or(Value::Null);
                if i + 1 == steps.len() {
                    let cls = h.borrow().class;
                    if let Some(m) = self.lookup_method(cls, b"offsetunset") {
                        self.call_sync(&m, Some(h), cls, vec![key])?;
                    }
                    return Ok(());
                }
                let cell = self.aa_fetch_w(&h, &steps[i], key, DimFetch::Unset)?;
                let rest = &steps[i + 1..];
                return self.with_cursor_root(cell, |vm| vm.cursor_unset(top, rest));
            }
        }
        let cx = PropCtx {
            classes: &self.classes,
            scope: self.stack[top].defining_class,
            called: self.stack[top].called_class,
        };
        let r = match &self.cursor_root {
            CursorRoot::Cv(r) => unset_path(&mut self.stack[top].regs[*r as usize], steps, &cx),
            CursorRoot::Cell(rc) => {
                let rc = rc.clone();
                let r = unset_path(&mut rc.borrow_mut(), steps, &cx);
                r
            }
        };
        self.flush_path_diags()?;
        // The removed element may have held the last reference to an object.
        if !self.destructibles.is_empty() {
            self.dtor_check = true;
        }
        r
    }

    /// Emit the diagnostics a cursor-path walk queued ([`path_diag`]).
    fn flush_path_diags(&mut self) -> Result<(), Thrown> {
        let diags = PATH_DIAGS.with(|d| std::mem::take(&mut *d.borrow_mut()));
        for (level, msg) in diags {
            self.diag(level, &msg)?;
        }
        Ok(())
    }

    /// `offsetGet($key)` on an ArrayAccess object.
    pub(crate) fn aa_offset_get(&mut self, h: &ObjectHandle, key: Value) -> Result<Value, Thrown> {
        let cls = h.borrow().class;
        match self.lookup_method(cls, b"offsetget") {
            Some(m) => self.call_sync(&m, Some(h.clone()), cls, vec![key]),
            None => Ok(Value::Null),
        }
    }

    /// `offsetExists($key)` on an ArrayAccess object, as a bool.
    pub(crate) fn aa_offset_exists(
        &mut self,
        h: &ObjectHandle,
        key: Value,
    ) -> Result<bool, Thrown> {
        let cls = h.borrow().class;
        let m = self.lookup_method(cls, b"offsetexists").unwrap();
        let r = self.call_sync(&m, Some(h.clone()), cls, vec![key])?;
        Ok(conv::to_bool(&r))
    }

    /// Before a read-modify-write (compound assignment / `++`) through an
    /// ArrayAccess object that is not the leaf: fetch that element once (php's
    /// single FETCH_DIM_RW), and re-root the open cursor at the result so the
    /// read and the following write both go there.
    pub(crate) fn cursor_settle_offsets(&mut self, top: usize) -> Result<(), Thrown> {
        loop {
            let steps = std::mem::take(&mut self.cursor_steps);
            match self.cursor_magic_split(top, &steps) {
                MagicSplit::Offset(i, h) if i + 1 < steps.len() => {
                    let key = aa_key(&steps[i]).unwrap_or(Value::Null);
                    let cell = self.aa_fetch_w(&h, &steps[i], key, DimFetch::Rw)?;
                    self.cursor_root = CursorRoot::Cell(cell);
                    self.cursor_steps = steps[i + 1..].to_vec();
                }
                _ => {
                    self.cursor_steps = steps;
                    return Ok(());
                }
            }
        }
    }

    /// Where the variable `name` of frame `top` lives, for a variable-variable:
    /// its compiled slot, else a cell in the frame's by-name table ({main}'s is
    /// the globals table, shared with `global`). `None` when the variable does
    /// not exist. (Auto-globals are resolved at compile time in php, so
    /// `$$n` with `$n = '_SERVER'` inside a function is undefined.)
    pub(crate) fn var_var_lookup(
        &self,
        top: usize,
        name: &[u8],
    ) -> Option<Result<Reg, Rc<RefCell<Value>>>> {
        let proto = &self.stack[top].proto;
        if let Some(i) = proto
            .cv_var_names
            .iter()
            .take(proto.num_cvs as usize)
            .position(|n| n.as_bytes() == name)
        {
            return Some(Ok(i as Reg));
        }
        // The global scope ({main}, top-level includes) is the global
        // symbol table.
        if self.stack[top].global_scope {
            return self.globals.get(name).map(|c| Err(c.clone()));
        }
        // An include/eval frame has no scope of its own: a variable it has no
        // slot for lives in the including scope.
        if let Some(caller) = self.stack[top].include_caller {
            return match self.var_var_lookup(caller, name)? {
                Ok(cv) => Some(Err(self.slot_cell_ro(caller, cv))),
                Err(cell) => Some(Err(cell)),
            };
        }
        self.stack[top].dyn_vars.get(name).map(|c| Err(c.clone()))
    }

    /// Make the variable `name` of frame `top` (a variable-variable) an alias
    /// of `cell`, as `global $$n` does.
    pub(crate) fn var_var_bind(&mut self, top: usize, name: &[u8], cell: Rc<RefCell<Value>>) {
        if self.stack[top].global_scope {
            // Re-points the symbol-table entry and every live global-scope
            // frame's slot of that name.
            self.rebind_global(name, cell);
            return;
        }
        let proto = &self.stack[top].proto;
        if let Some(i) = proto
            .cv_var_names
            .iter()
            .take(proto.num_cvs as usize)
            .position(|n| n.as_bytes() == name)
        {
            self.stack[top].regs[i] = Value::Ref(cell);
        } else if let Some(caller) = self.stack[top].include_caller {
            self.var_var_bind(caller, name, cell);
        } else {
            self.stack[top].dyn_vars.insert(name.to_vec(), cell);
        }
    }

    /// `unset($$n)`: drop frame `top`'s variable `name` (a compiled slot
    /// becomes null, as `unset($x)` does; a by-name variable is removed).
    pub(crate) fn var_var_unset(&mut self, top: usize, name: &[u8]) {
        if self.stack[top].global_scope {
            if self.globals.contains_key(name) {
                // Live global-scope frames lose the variable too.
                self.rebind_global(name, Rc::new(RefCell::new(Value::Null)));
                self.globals.shift_remove(name);
                self.ref_globals.remove(name);
                self.dtor_check = true;
            }
            return;
        }
        let proto = &self.stack[top].proto;
        if let Some(i) = proto
            .cv_var_names
            .iter()
            .take(proto.num_cvs as usize)
            .position(|n| n.as_bytes() == name)
        {
            self.reg_set(top, i as Reg, Value::Null);
        } else if let Some(caller) = self.stack[top].include_caller {
            self.var_var_unset(caller, name);
        } else if let Some(c) = self.stack[top].dyn_vars.remove(name) {
            if matches!(&*c.borrow(), Value::Object(_) | Value::Array(_)) {
                self.dtor_check = true;
            }
        }
    }

    /// The cell behind CV `cv` of frame `top` when it is already a reference,
    /// else a detached copy (read-only use).
    fn slot_cell_ro(&self, top: usize, cv: Reg) -> Rc<RefCell<Value>> {
        match &self.stack[top].regs[cv as usize] {
            Value::Ref(rc) => rc.clone(),
            v => Rc::new(RefCell::new(v.clone())),
        }
    }

    /// [`Self::var_var_lookup`], creating the variable when it does not exist.
    pub(crate) fn var_var_slot(
        &mut self,
        top: usize,
        name: &[u8],
    ) -> Result<Reg, Rc<RefCell<Value>>> {
        let proto = &self.stack[top].proto;
        if let Some(i) = proto
            .cv_var_names
            .iter()
            .take(proto.num_cvs as usize)
            .position(|n| n.as_bytes() == name)
        {
            return Ok(i as Reg);
        }
        if self.stack[top].global_scope {
            return Err(self.global_cell(name));
        }
        if let Some(caller) = self.stack[top].include_caller {
            return Err(match self.var_var_slot(caller, name) {
                Ok(cv) => self.promote(caller, cv),
                Err(cell) => cell,
            });
        }
        Err(self.stack[top]
            .dyn_vars
            .entry(name.to_vec())
            .or_insert_with(|| Rc::new(RefCell::new(Value::Null)))
            .clone())
    }

    /// An ArrayAccess element fetched for writing below it (php's
    /// FETCH_DIM_W/RW/UNSET on an object): `offsetGet`, whose result is the
    /// cell further writes land in — a temporary unless it returned by
    /// reference ("Indirect modification of overloaded element" notice when
    /// that temporary is not an object, which writes could still reach).
    /// ArrayObject / ArrayIterator without a user `offsetGet` hand out their
    /// storage element directly (spl_array_get_dimension_ptr).
    pub(crate) fn aa_fetch_w(
        &mut self,
        h: &ObjectHandle,
        step: &CursorStep,
        key: Value,
        mode: DimFetch,
    ) -> Result<Rc<RefCell<Value>>, Thrown> {
        if let Some(cell) = self.spl_array_dim_ptr(h, step, &key, mode)? {
            return Ok(cell);
        }
        let fetch = match mode {
            DimFetch::W | DimFetch::Rw => builtins::MagicFetch::Write,
            DimFetch::Unset => builtins::MagicFetch::Read,
        };
        let got = self.fetching_as(fetch, |vm| vm.aa_offset_get(h, key))?;
        Ok(match got {
            Value::Ref(rc) => rc,
            other => {
                if !matches!(other, Value::Object(_) | Value::Closure(_)) {
                    self.indirect_overloaded_notice(h)?;
                }
                Rc::new(RefCell::new(other))
            }
        })
    }

    fn indirect_overloaded_notice(&mut self, h: &ObjectHandle) -> Result<(), Thrown> {
        let cls = h.borrow().class;
        let msg = format!(
            "Indirect modification of overloaded element of {} has no effect",
            String::from_utf8_lossy(self.class(cls).name.as_bytes())
        );
        self.diag(8, &msg)
    }

    /// The built-in ArrayObject / ArrayIterator class `h` derives from, when
    /// its `method` is still the built-in one (not overridden in user code).
    pub(crate) fn spl_array_owner(&self, h: &ObjectHandle, method: &[u8]) -> Option<ClassId> {
        let cls = h.borrow().class;
        let m = self.lookup_method(cls, method)?;
        let owner = self.class(m.defining_class).name.as_bytes();
        (owner == b"ArrayObject" || owner == b"ArrayIterator").then_some(m.defining_class)
    }

    /// A reference to the backing array of a built-in ArrayObject /
    /// ArrayIterator (`owner` declares the storage property).
    pub(crate) fn spl_array_storage(
        &mut self,
        h: &ObjectHandle,
        owner: ClassId,
    ) -> Result<Rc<RefCell<Value>>, Thrown> {
        let cx = PropCtx {
            classes: &self.classes,
            scope: Some(owner),
            called: Some(owner),
        };
        let mut obj = Value::Object(h.clone());
        make_ref_path(&mut obj, &[CursorStep::Prop(PhpStr::from("__s"))], &cx)
    }

    /// php's spl_array_get_dimension_ptr for a built-in ArrayObject /
    /// ArrayIterator whose `offsetGet` is not overridden: the storage element
    /// itself (created as null for a write; "Undefined array key" first for a
    /// read-modify-write). `None`: not such an object.
    fn spl_array_dim_ptr(
        &mut self,
        h: &ObjectHandle,
        step: &CursorStep,
        key: &Value,
        mode: DimFetch,
    ) -> Result<Option<Rc<RefCell<Value>>>, Thrown> {
        let Some(owner) = self.spl_array_owner(h, b"offsetget") else {
            return Ok(None);
        };
        // `$ao[][...]`: no offset — php's uninitialized_zval.
        if matches!(step, CursorStep::Append) {
            self.indirect_overloaded_notice(h)?;
            return Ok(Some(Rc::new(RefCell::new(Value::Null))));
        }
        let storage = self.spl_array_storage(h, owner)?;
        let akey = match deref(key) {
            Value::Null => {
                self.diag(
                    8192,
                    "Using null as an array offset is deprecated, use an empty string instead",
                )?;
                Some(rhp_values::ArrayKey::from_bytes(b""))
            }
            k => rhp_values::ArrayKey::from_value(&k),
        };
        let Some(akey) = akey else {
            return Err(Thrown::type_error(format!(
                "Cannot access offset of type {} on {}",
                type_lc(key),
                String::from_utf8_lossy(self.class(h.borrow().class).name.as_bytes())
            )));
        };
        let exists = matches!(&*storage.borrow(), Value::Array(a) if a.get(&akey).is_some());
        if !exists {
            match mode {
                DimFetch::Unset => {
                    self.indirect_overloaded_notice(h)?;
                    return Ok(Some(Rc::new(RefCell::new(Value::Null))));
                }
                DimFetch::Rw => {
                    let msg = match akey.to_value() {
                        Value::Int(n) => format!("Undefined array key {n}"),
                        k => format!(
                            "Undefined array key \"{}\"",
                            String::from_utf8_lossy(&conv::to_string_bytes(&k))
                        ),
                    };
                    self.diag(2, &msg)?;
                }
                DimFetch::W => {}
            }
        }
        let cx = PropCtx {
            classes: &self.classes,
            scope: Some(owner),
            called: Some(owner),
        };
        let mut slot = storage.borrow_mut();
        make_ref_path(&mut slot, &[CursorStep::Key(akey.to_value())], &cx).map(Some)
    }

    /// Promote a CV slot to a shared reference cell, returning the cell. If the
    /// slot already holds a `Ref`, its cell is reused.
    pub(crate) fn promote(&mut self, top: usize, cv: Reg) -> Rc<RefCell<Value>> {
        let f = &self.stack[top];
        if f.global_scope {
            if let Some(n) = f.proto.cv_var_names.get(cv as usize) {
                let n = n.as_bytes().to_vec();
                self.ref_globals.insert(n);
            }
        }
        let slot = &mut self.stack[top].regs[cv as usize];
        match slot {
            Value::Ref(rc) => rc.clone(),
            other => {
                let v = std::mem::replace(other, Value::Null);
                let rc = Rc::new(RefCell::new(v));
                *other = Value::Ref(rc.clone());
                rc
            }
        }
    }

    /// php's array-offset key conversion for a read (`zend_fetch_dimension_
    /// address_inner` / `slow_index_convert`), with its diagnostics. `None`:
    /// an illegal offset type (TypeError raised by the caller).
    fn dim_key(&mut self, key: &Value, quiet: bool) -> Result<Option<ArrayKey>, Thrown> {
        match key {
            Value::Null if !quiet => self.diag(
                8192,
                "Using null as an array offset is deprecated, use an empty string instead",
            )?,
            Value::Float(f) if !quiet && (!f.is_finite() || f.fract() != 0.0) => {
                let shown =
                    String::from_utf8_lossy(&rhp_values::fmt::float_serialize(*f)).into_owned();
                self.diag(
                    8192,
                    &format!("Implicit conversion from float {shown} to int loses precision"),
                )?
            }
            Value::Resource(r) if !quiet => self.diag(
                2,
                &format!(
                    "Resource ID#{} used as offset, casting to integer ({})",
                    r.id, r.id
                ),
            )?,
            _ => {}
        }
        Ok(ArrayKey::from_value(key))
    }

    /// A dimension read `$c[$k]` with php's diagnostics: undefined keys,
    /// string offsets, scalar containers and ArrayAccess dispatch.
    pub(crate) fn dim_read(
        &mut self,
        container: &Value,
        key: &Value,
        mode: DimMode,
    ) -> Result<Value, Thrown> {
        let quiet = mode == DimMode::Is;
        match container {
            Value::Array(a) => {
                let Some(k) = self.dim_key(key, quiet)? else {
                    return Err(Thrown::type_error(format!(
                        "Cannot access offset of type {} on array",
                        self.offset_type_name(key)
                    )));
                };
                match a.get(&k) {
                    Some(v) => Ok(deref(v)),
                    None => {
                        if !quiet {
                            let msg = match &k {
                                ArrayKey::Int(n) => format!("Undefined array key {n}"),
                                ArrayKey::Str(s) => format!(
                                    "Undefined array key \"{}\"",
                                    String::from_utf8_lossy(s.as_bytes())
                                ),
                            };
                            self.diag(2, &msg)?;
                        }
                        Ok(Value::Null)
                    }
                }
            }
            Value::Object(h) => {
                let cls = h.borrow().class;
                if self.class_implements(cls, b"ArrayAccess") {
                    // zend_std_read_dimension(BP_VAR_IS): offsetExists first.
                    if quiet && !self.aa_offset_exists(h, key.clone())? {
                        return Ok(Value::Null);
                    }
                    return self.aa_offset_get(h, key.clone());
                }
                Err(Thrown::error(
                    "Error",
                    format!(
                        "Cannot use object of type {} as array",
                        String::from_utf8_lossy(self.class(cls).name.as_bytes())
                    ),
                ))
            }
            Value::Closure(_) => Err(Thrown::error(
                "Error",
                "Cannot use object of type Closure as array",
            )),
            Value::Str(s) if mode != DimMode::List => {
                let bytes = s.as_bytes();
                let offset = match key {
                    Value::Int(n) => *n,
                    Value::Str(ks) => {
                        let kb = ks.as_bytes();
                        match conv::scan_number(kb) {
                            Some((conv::Num::Int(n), end)) => {
                                let trailing =
                                    !kb[end..].iter().all(|b| b" \t\n\r\x0b\x0c".contains(b));
                                if trailing && !quiet {
                                    self.diag(
                                        2,
                                        &format!(
                                            "Illegal string offset \"{}\"",
                                            String::from_utf8_lossy(kb)
                                        ),
                                    )?;
                                }
                                n
                            }
                            _ => {
                                if quiet {
                                    return Ok(Value::Null);
                                }
                                return Err(Thrown::type_error(
                                    "Cannot access offset of type string on string",
                                ));
                            }
                        }
                    }
                    Value::Float(_) | Value::Null | Value::Bool(_) => {
                        if !quiet {
                            self.diag(2, "String offset cast occurred")?;
                        }
                        conv::to_int(key)
                    }
                    _ => {
                        if quiet {
                            return Ok(Value::Null);
                        }
                        return Err(Thrown::type_error(format!(
                            "Cannot access offset of type {} on string",
                            self.offset_type_name(key)
                        )));
                    }
                };
                let len = bytes.len() as i64;
                let i = if offset < 0 { len + offset } else { offset };
                if i >= 0 && i < len {
                    Ok(Value::str(vec![bytes[i as usize]]))
                } else if quiet {
                    Ok(Value::Null)
                } else {
                    self.diag(2, &format!("Uninitialized string offset {offset}"))?;
                    Ok(Value::str(""))
                }
            }
            Value::Null => Ok(Value::Null).and_then(|v| {
                if mode == DimMode::Read {
                    self.diag(2, "Trying to access array offset on null")?;
                }
                Ok(v)
            }),
            other => {
                if !quiet {
                    let ty = match other {
                        Value::Bool(true) => "true",
                        Value::Bool(false) => "false",
                        Value::Int(_) => "int",
                        Value::Float(_) => "float",
                        Value::Str(_) => "string",
                        Value::Resource(_) => "resource",
                        _ => "null",
                    };
                    let msg = if mode == DimMode::List {
                        format!("Cannot use {ty} as array")
                    } else {
                        format!("Trying to access array offset on {ty}")
                    };
                    self.diag(2, &msg)?;
                }
                Ok(Value::Null)
            }
        }
    }

    /// zend_zval_value_name for an illegal offset.
    fn offset_type_name(&self, key: &Value) -> String {
        match key {
            Value::Object(h) => {
                String::from_utf8_lossy(self.class(h.borrow().class).name.as_bytes()).into_owned()
            }
            Value::Closure(_) => "Closure".into(),
            v => v.type_name().to_string(),
        }
    }
}

/// `$container[$key]` read: array element (Null if absent) or string byte.
/// `isset($container[$key])` for arrays and strings (php's
/// `zend_isset_dim_slow` string-offset rules).
pub(crate) fn isset_dim(container: &Value, key: &Value) -> bool {
    match deref(container) {
        Value::Array(_) => !matches!(dim_fetch(container, key), Value::Null),
        Value::Str(s) => {
            let k = deref(key);
            let idx = match &k {
                Value::Int(n) => *n,
                Value::Null | Value::Bool(_) | Value::Float(_) => conv::to_int(&k),
                Value::Str(ks) => match conv::numeric_string(ks.as_bytes()) {
                    Some(conv::Num::Int(n)) => n,
                    _ => return false,
                },
                _ => return false,
            };
            let len = s.as_bytes().len() as i64;
            let i = if idx < 0 { idx + len } else { idx };
            i >= 0 && i < len
        }
        _ => false,
    }
}

#[allow(dead_code)]
fn dim_fetch(container: &Value, key: &Value) -> Value {
    match deref(container) {
        Value::Array(a) => match ArrayKey::from_value(key) {
            // A read yields the value, never the element's reference cell.
            Some(k) => a.get(&k).map(deref).unwrap_or(Value::Null),
            None => Value::Null,
        },
        Value::Str(s) => {
            let bytes = s.as_bytes();
            let len = bytes.len() as i64;
            let idx = conv::to_int(key);
            let i = if idx < 0 { len + idx } else { idx };
            if i >= 0 && i < len {
                Value::str(vec![bytes[i as usize]])
            } else {
                Value::str("")
            }
        }
        _ => Value::Null,
    }
}

fn read_path(slot: &Value, steps: &[CursorStep], cx: &PropCtx) -> Value {
    if let Value::Ref(r) = slot {
        return read_path(&r.borrow(), steps, cx);
    }
    let Some((head, rest)) = steps.split_first() else {
        return slot.clone();
    };
    match (slot, head) {
        (Value::Array(a), CursorStep::Key(k)) => match ArrayKey::from_value(k) {
            Some(key) => a
                .get(&key)
                .map(|c| read_path(c, rest, cx))
                .unwrap_or(Value::Null),
            None => Value::Null,
        },
        (Value::Object(h), CursorStep::Prop(name)) => {
            let od = h.borrow();
            match cx.key(od.class, name) {
                Ok(k) => od
                    .properties
                    .get(&k)
                    .map(|c| read_path(c, rest, cx))
                    .unwrap_or(Value::Null),
                Err(_) => Value::Null,
            }
        }
        _ => Value::Null,
    }
}

/// Make `slot` an array (autovivifying null, and false with a deprecation),
/// or error on any other scalar (strings are handled by the caller).
fn autovivify(slot: &mut Value, cx: &PropCtx) -> Result<(), Thrown> {
    match slot {
        Value::Array(_) => Ok(()),
        Value::Null => {
            *slot = Value::Array(PhpArray::new());
            Ok(())
        }
        Value::Bool(false) => {
            path_diag(
                8192,
                "Automatic conversion of false to array is deprecated".into(),
            );
            *slot = Value::Array(PhpArray::new());
            Ok(())
        }
        Value::Object(h) => Err(Thrown::error(
            "Error",
            format!(
                "Cannot use object of type {} as array",
                h.try_borrow()
                    .map(
                        |b| String::from_utf8_lossy(cx.classes[b.class as usize].name.as_bytes())
                            .into_owned()
                    )
                    .unwrap_or_default()
            ),
        )),
        Value::Closure(_) => Err(Thrown::error(
            "Error",
            "Cannot use object of type Closure as array",
        )),
        _ => Err(Thrown::error(
            "Error",
            "Cannot use a scalar value as an array",
        )),
    }
}

fn path_diag(level: i64, msg: String) {
    PATH_DIAGS.with(|d| d.borrow_mut().push((level, msg)));
}

/// The array key for a write / fetch-for-write with `k`: php's null-offset
/// and fractional-float deprecations, the TypeError for an unusable key.
fn write_key_of(k: &Value, cx: &PropCtx) -> Result<ArrayKey, Thrown> {
    match deref(k) {
        Value::Null => path_diag(
            8192,
            "Using null as an array offset is deprecated, use an empty string instead".into(),
        ),
        Value::Float(f) if f.is_finite() && f.fract() != 0.0 => path_diag(
            8192,
            format!(
                "Implicit conversion from float {} to int loses precision",
                String::from_utf8_lossy(&conv::to_string_bytes(&Value::Float(f)))
            ),
        ),
        _ => {}
    }
    ArrayKey::from_value(k).ok_or_else(|| illegal_offset_on_array(k, cx))
}

/// php's zend_check_string_offset: the integer offset for a write / unset
/// of string offset `k` ("Illegal string offset" / "String offset cast
/// occurred" warnings; a non-integer key is a TypeError, or for unset php's
/// "Cannot unset string offsets").
fn string_offset(k: &Value, unset: bool, cx: &PropCtx) -> Result<i64, Thrown> {
    let illegal = |ty: String| {
        if unset {
            Thrown::error("Error", "Cannot unset string offsets")
        } else {
            Thrown::type_error(format!("Cannot access offset of type {ty} on string"))
        }
    };
    match deref(k) {
        Value::Int(n) => Ok(n),
        Value::Str(ks) => {
            let kb = ks.as_bytes();
            match conv::scan_number(kb) {
                Some((conv::Num::Int(n), end)) => {
                    let trailing = !kb[end..].iter().all(|b| b" \t\n\r\x0b\x0c".contains(b));
                    if trailing && !unset {
                        path_diag(
                            2,
                            format!("Illegal string offset \"{}\"", String::from_utf8_lossy(kb)),
                        );
                    }
                    Ok(n)
                }
                _ => Err(illegal("string".into())),
            }
        }
        v @ (Value::Float(_) | Value::Null | Value::Bool(_)) => {
            path_diag(2, "String offset cast occurred".into());
            Ok(conv::to_int(&v))
        }
        Value::Object(h) => Err(illegal(
            h.try_borrow()
                .map(|b| {
                    String::from_utf8_lossy(cx.classes[b.class as usize].name.as_bytes())
                        .into_owned()
                })
                .unwrap_or_default(),
        )),
        Value::Closure(_) => Err(illegal("Closure".into())),
        other => Err(illegal(type_lc(&other).to_string())),
    }
}

/// php's zend_wrong_string_offset_error for a fetch below a string offset
/// (`$str[0][…]`, `$str[0]->p`) or a reference to one.
fn string_offset_misuse(rest: &[CursorStep], make_ref: bool) -> Thrown {
    let msg = match rest.first() {
        None if make_ref => "Cannot create references to/from string offsets",
        Some(CursorStep::Prop(_)) => "Cannot use string offset as an object",
        _ => "Cannot use string offset as an array",
    };
    Thrown::error("Error", msg)
}

/// `$str[$k] = $value` (zend_assign_to_string_offset): replace one byte,
/// padding with spaces past the end.
fn assign_string_offset(
    slot: &mut Value,
    k: &Value,
    value: &Value,
    cx: &PropCtx,
) -> Result<(), Thrown> {
    let offset = string_offset(k, false, cx)?;
    let Value::Str(s) = slot else {
        unreachable!("caller checked for a string")
    };
    let len = s.as_bytes().len() as i64;
    if offset < -len {
        path_diag(2, format!("Illegal string offset {offset}"));
        return Ok(());
    }
    let at = if offset < 0 { offset + len } else { offset } as usize;
    let bytes = match deref(value) {
        Value::Array(_) => {
            path_diag(2, "Array to string conversion".into());
            b"Array".to_vec()
        }
        Value::Object(_) | Value::Closure(_) => {
            return Err(Thrown::unsupported(
                "string offset write of an object",
                "converting an object to string here needs __toString dispatch",
            ))
        }
        v => conv::to_string_bytes(&v),
    };
    let Some(&c) = bytes.first() else {
        return Err(Thrown::error(
            "Error",
            "Cannot assign an empty string to a string offset",
        ));
    };
    if bytes.len() != 1 {
        path_diag(
            2,
            "Only the first byte will be assigned to the string offset".into(),
        );
    }
    if at >= s.len() {
        // zend_string_extend to the new length (a realloc when unshared):
        // past php's hard limit the VM exits instead.
        let old = match s.get_mut() {
            Some(b) => rhp_values::mem::zstr_size(b.len()),
            None => 0,
        };
        if !rhp_values::mem::realloc_check(old, rhp_values::mem::zstr_size(at + 1)) {
            return Ok(());
        }
        let out = s.make_mut();
        let grow = at + 1 - out.len();
        out.reserve_exact(grow);
        out.resize(at, b' ');
        out.push(c);
    } else {
        s.make_mut()[at] = c;
    }
    Ok(())
}

/// Write `value` at a cursor path, autovivifying + COW-bubbling along the way.
fn assign_path(
    slot: &mut Value,
    steps: &[CursorStep],
    value: Value,
    cx: &PropCtx,
) -> Result<(), Thrown> {
    // `lval = &$x` (a `Ref` value at the leaf) rebinds the slot itself: it
    // does not write through a reference the slot already held.
    if steps.is_empty() && matches!(value, Value::Ref(_)) {
        *slot = value;
        return Ok(());
    }
    // Descend through a reference to its underlying storage.
    if let Value::Ref(rc) = slot {
        let mut inner = rc.borrow_mut();
        return assign_path(&mut inner, steps, value, cx);
    }
    let Some((head, rest)) = steps.split_first() else {
        *slot = value;
        return Ok(());
    };
    if let CursorStep::Prop(name) = head {
        // Descend into an object property (handle semantics: no COW).
        let handle = match slot {
            Value::Object(h) => h.clone(),
            // zend_throw_non_object_error: the property itself is assigned;
            // a write further down the path modifies it.
            other => {
                let shown = match other {
                    Value::Bool(true) => "true",
                    Value::Bool(false) => "false",
                    o => type_lc(o),
                };
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Attempt to {} property \"{}\" on {shown}",
                        if rest.is_empty() { "assign" } else { "modify" },
                        String::from_utf8_lossy(name.as_bytes())
                    ),
                ));
            }
        };
        let mut od = handle.borrow_mut();
        let key = cx.write_key(&od, name, "modify", rest)?;
        let child = classes::prop_slot_mut(cx.classes, &mut od, key);
        return assign_path(child, rest, value, cx);
    }
    if let Value::Str(_) = slot {
        let CursorStep::Key(k) = head else {
            return Err(Thrown::error(
                "Error",
                "[] operator not supported for strings",
            ));
        };
        if rest.is_empty() {
            return assign_string_offset(slot, k, &value, cx);
        }
        string_offset(k, false, cx)?;
        return Err(string_offset_misuse(rest, false));
    }
    autovivify(slot, cx)?;
    let Value::Array(arr) = slot else {
        unreachable!("autovivify ensures array")
    };
    let child = match head {
        CursorStep::Key(k) => arr.get_or_insert_mut(write_key_of(k, cx)?),
        CursorStep::Append => arr.append_mut(),
        CursorStep::Prop(_) => unreachable!("handled above"),
    };
    assign_path(child, rest, value, cx)
}

/// The key value for an ArrayAccess cursor step: `Key(k)` -> k, `Append` -> null
/// (`$obj[] = v` calls offsetSet(null, v)); `Prop` is not array access.
pub(crate) fn aa_key(step: &CursorStep) -> Option<Value> {
    match step {
        CursorStep::Key(k) => Some(k.clone()),
        CursorStep::Append => Some(Value::Null),
        CursorStep::Prop(_) => None,
    }
}

/// Navigate the cursor path and promote the target slot (a property or array
/// element) to a shared `Value::Ref` cell, returning the cell — so a by-ref
/// builtin can mutate the caller's property/element in place.
fn make_ref_path(
    slot: &mut Value,
    steps: &[CursorStep],
    cx: &PropCtx,
) -> Result<Rc<RefCell<Value>>, Thrown> {
    // Follow an existing reference: reuse it if this is the target, else descend.
    if let Value::Ref(rc) = slot {
        if steps.is_empty() {
            return Ok(rc.clone());
        }
        let rc = rc.clone();
        let r = make_ref_path(&mut rc.borrow_mut(), steps, cx);
        return r;
    }
    let Some((head, rest)) = steps.split_first() else {
        // Target slot: promote in place to a Ref cell.
        let v = std::mem::replace(slot, Value::Null);
        let rc = Rc::new(RefCell::new(v));
        *slot = Value::Ref(rc.clone());
        return Ok(rc);
    };
    if let CursorStep::Prop(name) = head {
        let handle = match slot {
            Value::Object(h) => h.clone(),
            Value::Null => {
                return Err(Thrown::error(
                    "Error",
                    "Attempt to reference property on null",
                ))
            }
            other => {
                return Err(Thrown::error(
                    "Error",
                    format!("Attempt to reference property on {}", type_lc(other)),
                ))
            }
        };
        let mut od = handle.borrow_mut();
        let key = cx.write_key(&od, name, "indirectly modify", rest)?;
        let child = classes::prop_slot_mut(cx.classes, &mut od, key);
        return make_ref_path(child, rest, cx);
    }
    if let Value::Str(_) = slot {
        let CursorStep::Key(k) = head else {
            return Err(Thrown::error(
                "Error",
                "[] operator not supported for strings",
            ));
        };
        string_offset(k, false, cx)?;
        return Err(string_offset_misuse(rest, true));
    }
    autovivify(slot, cx)?;
    let Value::Array(arr) = slot else {
        unreachable!("autovivify ensures array")
    };
    let child = match head {
        CursorStep::Key(k) => arr.get_or_insert_mut(write_key_of(k, cx)?),
        CursorStep::Append => arr.append_mut(),
        CursorStep::Prop(_) => unreachable!("handled above"),
    };
    make_ref_path(child, rest, cx)
}

/// Remove the leaf key of a cursor path (COW), descending to the parent first.
fn unset_path(slot: &mut Value, steps: &[CursorStep], cx: &PropCtx) -> Result<(), Thrown> {
    if let Value::Ref(rc) = slot {
        let mut inner = rc.borrow_mut();
        return unset_path(&mut inner, steps, cx);
    }
    let Some((head, rest)) = steps.split_first() else {
        return Ok(());
    };
    match head {
        CursorStep::Prop(name) => {
            let Value::Object(h) = slot else {
                return Ok(());
            };
            let h = h.clone();
            let mut od = h.borrow_mut();
            let Ok(key) = cx.key(od.class, name) else {
                return Ok(());
            };
            if rest.is_empty() {
                od.properties.shift_remove(&key);
            } else if let Some(child) = od.properties.get_mut(&key) {
                unset_path(child, rest, cx)?;
            }
        }
        CursorStep::Key(k) => {
            let arr = match slot {
                Value::Array(arr) => arr,
                // ZEND_UNSET_DIM / FETCH_DIM_UNSET on a string.
                Value::Str(_) if rest.is_empty() => {
                    return Err(Thrown::error("Error", "Cannot unset string offsets"))
                }
                Value::Str(_) => {
                    string_offset(k, true, cx)?;
                    return Err(string_offset_misuse(rest, false));
                }
                Value::Bool(false) if rest.is_empty() => {
                    path_diag(
                        8192,
                        "Automatic conversion of false to array is deprecated".into(),
                    );
                    return Ok(());
                }
                Value::Bool(true) | Value::Int(_) | Value::Float(_) | Value::Resource(_)
                    if rest.is_empty() =>
                {
                    return Err(Thrown::error(
                        "Error",
                        "Cannot unset offset in a non-array variable",
                    ))
                }
                _ => return Ok(()),
            };
            let Some(key) = ArrayKey::from_value(k) else {
                if !rest.is_empty() {
                    return Err(illegal_offset_on_array(k, cx));
                }
                let ty = match deref(k) {
                    Value::Object(h) => h
                        .try_borrow()
                        .map(|b| {
                            String::from_utf8_lossy(cx.classes[b.class as usize].name.as_bytes())
                                .into_owned()
                        })
                        .unwrap_or_default(),
                    Value::Closure(_) => "Closure".into(),
                    other => type_lc(&other).to_string(),
                };
                return Err(Thrown::type_error(format!(
                    "Cannot unset offset of type {ty} on array"
                )));
            };
            if rest.is_empty() {
                arr.remove(&key);
            } else if let Some(child) = arr.get_mut(&key) {
                unset_path(child, rest, cx)?;
            }
        }
        CursorStep::Append => {}
    }
    Ok(())
}

/// For by-reference foreach: promote the array element at insertion index `pos`
/// to a shared cell and return `(key, cell)`. The element in the array becomes a
/// `Value::Ref` to that cell, so a later write through the cell is visible in the
/// array. Returns `None` if `slot` is not an array or `pos` is out of range.
pub(crate) fn foreach_promote(
    slot: &mut Value,
    pos: usize,
) -> Option<(ArrayKey, Rc<RefCell<Value>>)> {
    let Value::Array(arr) = slot else { return None };
    if pos >= arr.len() {
        return None;
    }
    let data = arr.make_mut();
    let (k, v) = data.entries.get_index_mut(pos)?;
    let key = k.clone();
    let rc = match v {
        Value::Ref(rc) => rc.clone(),
        other => {
            let inner = std::mem::replace(other, Value::Null);
            let rc = Rc::new(RefCell::new(inner));
            *other = Value::Ref(rc.clone());
            rc
        }
    };
    Some((key, rc))
}

/// php's "Undefined array key" warning text for `k`.
fn undefined_key_msg(k: &ArrayKey) -> String {
    match k {
        ArrayKey::Int(n) => format!("Undefined array key {n}"),
        ArrayKey::Str(s) => format!(
            "Undefined array key \"{}\"",
            String::from_utf8_lossy(s.as_bytes())
        ),
    }
}

/// php's "Cannot access offset of type T on array" TypeError (an array
/// write/fetch-for-write with an unusable key).
fn illegal_offset_on_array(k: &Value, cx: &PropCtx) -> Thrown {
    let ty = match deref(k) {
        Value::Object(h) => h
            .try_borrow()
            .map(|b| {
                String::from_utf8_lossy(cx.classes[b.class as usize].name.as_bytes()).into_owned()
            })
            .unwrap_or_default(),
        Value::Closure(_) => "Closure".into(),
        other => type_lc(&other).to_string(),
    };
    Thrown::type_error(format!("Cannot access offset of type {ty} on array"))
}
