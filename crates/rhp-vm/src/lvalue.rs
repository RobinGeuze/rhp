//! Array element reads and lvalue paths (the cursor opcodes): nested writes,
//! references, unset, variable variables and ArrayAccess.

use std::cell::RefCell;
use std::rc::Rc;

use rhp_builtins as builtins;
use rhp_builtins::zend::object_handlers::DimType;
use rhp_bytecode::{Reg, TypeKind};
use rhp_values::{
    conv, ArrayKey, ClassId, ObjectData, ObjectHandle, PhpArray, PhpStr, Thrown, Value,
};

use crate::classes::PropRes;
use crate::{classes, deref, scope, Vm};

/// The open cursor's steps: inline for the usual few, so an element
/// write allocates nothing for its path.
pub(crate) type CursorSteps = smallvec::SmallVec<[CursorStep; 4]>;

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
    /// Step `.0` reaches a lazy slot (or an undefined property) of this
    /// lazy object: it initializes, and the path continues on the instance.
    Lazy(usize, ObjectHandle),
    /// A property of this `__PHP_Incomplete_Class` object: writing through
    /// it throws.
    Incomplete(ObjectHandle),
}

/// A cursor rooted at a static property with a type or set visibility:
/// what its checks need.
#[derive(Clone)]
pub(crate) struct StaticRoot {
    pub(crate) cell: Rc<RefCell<Value>>,
    /// The class the property was reached through.
    pub(crate) cls: ClassId,
    pub(crate) name: PhpStr,
    pub(crate) decl: ClassId,
    pub(crate) set_vis: Option<rhp_values::Visibility>,
    pub(crate) ty: Option<Rc<rhp_bytecode::TypeHint>>,
}

/// The operation on a cursor rooted at a static property, for its
/// asymmetric visibility and type checks.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum StaticOp {
    /// `C::$p = v` (ZEND_ASSIGN_STATIC_PROP) or the write of `C::$p[..] = v`.
    Assign,
    /// `C::$p = &$x` (ZEND_ASSIGN_STATIC_PROP_REF).
    Ref,
    /// `C::$p op= v`, `++C::$p` (BP_VAR_RW).
    Rw,
    /// `&C::$p`, `C::$p[] = v` (ZEND_FETCH_STATIC_PROP_W).
    Fetch,
    /// `unset(C::$p[k])` (ZEND_FETCH_STATIC_PROP_UNSET).
    Unset,
}

/// What [`Vm::cursor_through_check`] found.
#[derive(PartialEq)]
pub(crate) enum Through {
    /// Nothing in the way.
    Plain,
    /// The reference target is a readonly / set-restricted property
    /// holding an object: the reference gets a copy.
    Copy,
    /// An unset below an uninitialized such property: nothing to do.
    Stop,
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
/// A typed slot a reference is being bound to.
enum RefBind {
    Static(ClassId, PhpStr),
    Prop(ObjectHandle, PhpStr),
}

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
    /// The typed references (writes through one go back to the VM).
    typed: &'a crate::refsrc::TypedRefs,
    scope: Option<ClassId>,
    /// The called (LSB) class, named by asymmetric-visibility errors.
    called: Option<ClassId>,
    /// The write is a `++`/`--` (zend_throw_non_object_error names it).
    incdec: bool,
}

impl PropCtx<'_> {
    /// zend_zval_type_name, with the class names this context knows.
    fn type_name(&self, v: &Value) -> String {
        rhp_builtins::zval_type_name_by(v, |o| self.class_name(o))
    }

    /// zend_zval_value_name, with the class names this context knows.
    fn value_name(&self, v: &Value) -> String {
        rhp_builtins::zval_value_name_by(v, |o| self.class_name(o))
    }

    fn class_name(&self, o: &Value) -> Option<Vec<u8>> {
        match o {
            Value::Object(h) => h
                .try_borrow()
                .ok()
                .map(|b| self.classes[b.class as usize].name.as_bytes().to_vec()),
            _ => None,
        }
    }

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
        let (key, dynamic) = match classes::resolve_prop(self.classes, od.class, self.scope, name) {
            PropRes::Slot(k) => (k, false),
            PropRes::Dynamic => (name.clone(), true),
            PropRes::Denied(vis) => {
                return Err(classes::prop_access_err(
                    self.classes,
                    od.class,
                    name.as_bytes(),
                    vis,
                ))
            }
        };
        let entry = &self.classes[od.class as usize];
        if dynamic
            && !entry.static_props.is_empty()
            && !entry.methods.contains_key(&b"__get"[..])
            && matches!(
                classes::static_as_instance(self.classes, od.class, self.scope, name),
                Some(None)
            )
        {
            path_diag(
                builtins::E_NOTICE,
                format!(
                    "Accessing static property {}::${} as non static",
                    String::from_utf8_lossy(entry.name.as_bytes()),
                    String::from_utf8_lossy(name.as_bytes())
                ),
            );
        }
        if let Err(e) = classes::set_access_ok(self.classes, od.class, self.scope, &key) {
            let holds_object = !rest.is_empty()
                && matches!(od.props().get(&key).map(deref), Some(Value::Object(_)));
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

/// What a read-modify-write fetch holds while an undefined-key warning
/// runs (see [`Vm::cursor_rw_checks`]).
enum RwHold {
    /// The array missing the key at path level `.0` (its clone makes a
    /// write by the handler separate it, as php's GC_ADDREF does).
    Array(usize, PhpArray),
    /// A null root php turns into an array first: destroyed when the
    /// variable no longer holds null.
    RootNull,
}

/// Where [`assign_path`] stopped for the VM: a write through a typed
/// reference (done by the VM, with the value), or an array that cannot be
/// auto-initialized inside a typed reference / typed property.
enum TypedStop {
    Leaf(Rc<RefCell<Value>>, Value),
    RefInit(Rc<RefCell<Value>>),
    PropInit(ClassId, PhpStr),
}

/// php's promotes_to_array: null, false (and an undefined slot).
fn promotes_to_array(v: &Value) -> bool {
    matches!(v, Value::Null | Value::Undef | Value::Bool(false))
}

thread_local! {
    /// Diagnostics raised inside the pure cursor-path walkers
    /// ([`assign_path`] / [`make_ref_path`] / [`unset_path`]), emitted by
    /// the VM right after the walk ([`Vm::flush_path_diags`]).
    static PATH_DIAGS: RefCell<Vec<(i64, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
    /// The value of the last string offset assignment: the byte written,
    /// or null when none was ([`assign_string_offset`]).
    static STRING_OFFSET_RESULT: RefCell<Option<Value>> = const { RefCell::new(None) };
    /// [`assign_string_offset`] stopped at an object value: the caller
    /// converts it with `__toString` and writes again.
    static STRING_OFFSET_OBJECT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The value of the string offset assignment the last cursor write made,
/// if it made one.
pub(crate) fn take_string_offset_result() -> Option<Value> {
    STRING_OFFSET_RESULT.with(|r| r.borrow_mut().take())
}

fn set_string_offset_result(v: Value) {
    STRING_OFFSET_RESULT.with(|r| *r.borrow_mut() = Some(v));
}

/// A new engine: nothing a walk of an earlier one left behind.
pub(crate) fn reset_path_diags() {
    PATH_DIAGS.with(|d| d.borrow_mut().clear());
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
            MagicSplit::Incomplete(h) => Err(self.incomplete_error(&h, "modify a property")),
            MagicSplit::Plain => {
                self.cursor_rw_checks(top, steps, incdec)?;
                if self.rw_aborted {
                    return Ok(Value::Null);
                }
                Ok(self.cursor_read_steps(top, steps))
            }
            // (A by-reference `__get` result is read as a copy.)
            MagicSplit::Leaf(h, name) => self
                .prop_fetch(top, Value::Object(h), name)
                .map(|v| deref(&v)),
            MagicSplit::Lazy(i, h) => {
                let cell = self.lazy_cursor_cell(&h)?;
                self.with_cursor_root(cell, |vm| vm.cursor_read_magic(top, &steps[i..], incdec))
            }
            MagicSplit::Nested(i, h, name) if self.cursor_hooked(top, &h, &name) => {
                let cell = self.magic_write_cell(top, h, name)?;
                let rest = &steps[i + 1..];
                self.with_cursor_root(cell, |vm| vm.cursor_read_magic(top, rest, incdec))
            }
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
                    typed: &self.typed_refs,
                    scope: self.stack[top].defining_class,
                    called: self.stack[top].called_class,
                    incdec: false,
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
        self.rw_null_keys = 0;
        self.cursor_static = None;
        self.deferred_undef_src = None;
        self.skip_false_dep = 0;
        self.typed_write_result = None;
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
        let mut warnings: Vec<(i64, String)> = Vec::new();
        // The container of the first undefined key, held while its warning
        // runs (zend_undefined_index_write's GC_ADDREF), with the warning's
        // index.
        let mut hold: Option<(usize, RwHold)> = None;
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
                        typed: &self.typed_refs,
                        scope: self.stack[top].defining_class,
                        called: self.stack[top].called_class,
                        incdec: false,
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
                (c @ (None | Some(Value::Null)), CursorStep::Key(k)) => {
                    self.rw_null_key(k, &mut warnings);
                    match ArrayKey::from_value(&deref(k)) {
                        Some(key) => {
                            if i == 0 && c.is_some() && hold.is_none() {
                                hold = Some((warnings.len(), RwHold::RootNull));
                            }
                            warnings.push((builtins::E_WARNING, undefined_key_msg(&key)))
                        }
                        None => {
                            self.flush_rw_warnings(warnings)?;
                            return Err(self.illegal_array_offset(top, k));
                        }
                    }
                    None
                }
                (Some(Value::Array(a)), CursorStep::Key(k)) => {
                    self.rw_null_key(k, &mut warnings);
                    let Some(key) = ArrayKey::from_value(&deref(k)) else {
                        // The fetch for read-write rejects the key before
                        // the operator runs.
                        self.flush_rw_warnings(warnings)?;
                        return Err(self.illegal_array_offset(top, k));
                    };
                    match a.get(&key) {
                        Some(v) => Some(deref(v)),
                        None => {
                            if hold.is_none() {
                                hold = Some((warnings.len(), RwHold::Array(i, a.clone())));
                            }
                            warnings.push((builtins::E_WARNING, undefined_key_msg(&key)));
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
                    let present = h.borrow().props().get(&key).map(deref);
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
                                h.borrow_mut().props_mut().insert(key, Value::Null);
                            }
                            warnings.push((
                                builtins::E_WARNING,
                                format!(
                                    "Undefined property: {}::${}",
                                    String::from_utf8_lossy(self.class(cls).name.as_bytes()),
                                    String::from_utf8_lossy(name.as_bytes())
                                ),
                            ));
                            None
                        }
                    }
                }
                _ => break,
            };
        }
        let Some((at, hold)) = hold else {
            return self.flush_rw_warnings(warnings);
        };
        for (i, (level, w)) in warnings.into_iter().enumerate() {
            self.diag(level, &w)?;
            if i == at {
                let destroyed = match &hold {
                    // php separates the held table on any write to it:
                    // the path no longer reaches it.
                    RwHold::Array(level, a) => {
                        let mut v = deref(&self.cursor_root_value(top));
                        for step in &steps[..*level] {
                            let next = match (&v, step) {
                                (Value::Array(x), CursorStep::Key(k)) => {
                                    ArrayKey::from_value(k).and_then(|key| x.get(&key).map(deref))
                                }
                                _ => None,
                            };
                            v = next.unwrap_or(Value::Null);
                        }
                        !matches!(&v, Value::Array(x) if x.data_ptr() == a.data_ptr())
                    }
                    RwHold::RootNull => !matches!(deref(&self.cursor_root_value(top)), Value::Null),
                };
                if destroyed {
                    self.rw_aborted = true;
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    fn illegal_array_offset(&self, top: usize, k: &Value) -> Thrown {
        let cx = PropCtx {
            classes: &self.classes,
            typed: &self.typed_refs,
            scope: self.stack[top].defining_class,
            called: self.stack[top].called_class,
            incdec: self.incdec_write,
        };
        illegal_offset_on_array(&deref(k), &cx)
    }

    fn flush_rw_warnings(&mut self, warnings: Vec<(i64, String)>) -> Result<(), Thrown> {
        for (level, w) in warnings {
            self.diag(level, &w)?;
        }
        Ok(())
    }

    /// The warning of an undefined variable that is the last key of an
    /// assignment's path, in php's order: ZEND_ASSIGN_DIM reads the key
    /// after checking the container. A false container is converted first
    /// (with its deprecation); a string container is held while the key
    /// is read. When the handler released either, php writes nothing:
    /// `Ok(true)`, with the cursor released and the result `dst` null.
    #[cold]
    #[inline(never)]
    pub(crate) fn assign_undef_key(&mut self, top: usize, dst: Reg) -> Result<bool, Thrown> {
        let key = self.deferred_undef_key.take().expect("a deferred key");
        match self.assign_undef_key_in(top, key) {
            Ok(false) => Ok(false),
            r => {
                self.cursor_steps.clear();
                self.cursor_release();
                if r.is_ok() {
                    self.reg_set(top, dst, Value::Null);
                }
                r
            }
        }
    }

    fn assign_undef_key_in(&mut self, top: usize, key: Reg) -> Result<bool, Thrown> {
        let container = |vm: &Vm| {
            let n = vm.cursor_steps.len();
            deref(&if n <= 1 {
                vm.cursor_root_value(top)
            } else {
                vm.cursor_read_steps(top, &vm.cursor_steps[..n - 1])
            })
        };
        match container(self) {
            Value::Bool(false) => {
                self.diag(
                    builtins::E_DEPRECATED,
                    "Automatic conversion of false to array is deprecated",
                )?;
                if !matches!(container(self), Value::Bool(false)) {
                    return Ok(true);
                }
                self.skip_false_dep += 1;
                self.undef_cv(top, key)?;
                // php's converted array is gone when the handler replaced
                // the variable.
                if !matches!(container(self), Value::Bool(false)) {
                    return Ok(true);
                }
            }
            Value::Str(s) => {
                self.undef_cv(top, key)?;
                if !matches!(container(self), Value::Str(t) if t.ptr_eq(&s)) {
                    self.diag(builtins::E_WARNING, "String offset cast occurred")?;
                    return Ok(true);
                }
            }
            // "Cannot use a scalar value as an array" comes first; the key
            // is not read.
            Value::Int(_) | Value::Float(_) | Value::Bool(true) | Value::Resource(_) => {}
            _ => {
                self.lval_undef_key(top, key)?;
            }
        }
        self.cursor_null_key = true;
        Ok(false)
    }

    /// The null keys of an assignment's path: php's write fetches convert
    /// them (with the deprecation) before the assigned value is read; the
    /// write then converts them quietly.
    #[cold]
    pub(crate) fn w_null_key_diags(&mut self, top: usize) -> Result<(), Thrown> {
        let mut cur = Some(deref(&self.cursor_root_value(top)));
        let mut n = 0;
        for step in self.cursor_steps.iter() {
            let CursorStep::Key(k) = step else { break };
            cur = match cur {
                Some(Value::Array(a)) => {
                    if matches!(k, Value::Null) {
                        n += 1;
                    }
                    ArrayKey::from_value(k).and_then(|key| a.get(&key).map(deref))
                }
                None | Some(Value::Null) => {
                    if matches!(k, Value::Null) {
                        n += 1;
                    }
                    None
                }
                _ => break,
            };
        }
        for _ in 0..n {
            self.diag(builtins::E_DEPRECATED, NULL_OFFSET_DEPRECATION)?;
            self.rw_null_keys += 1;
        }
        Ok(())
    }

    /// A null key on a read-modify-write path: php converts it (with the
    /// deprecation) when it fetches the element, before the undefined-key
    /// warning; the write then converts it quietly.
    fn rw_null_key(&mut self, k: &Value, warnings: &mut Vec<(i64, String)>) {
        if matches!(deref(k), Value::Null) {
            warnings.push((builtins::E_DEPRECATED, NULL_OFFSET_DEPRECATION.into()));
            self.rw_null_keys += 1;
        }
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
                    if self.incomplete_class == Some(h.borrow().class) {
                        return MagicSplit::Incomplete(h);
                    }
                    if self.cursor_hooked(top, &h, name) {
                        return if i + 1 == steps.len() {
                            MagicSplit::Leaf(h, name.clone())
                        } else {
                            MagicSplit::Nested(i, h, name.clone())
                        };
                    }
                    let cls = h.borrow().class;
                    let present = match classes::resolve_prop(&self.classes, cls, scope, name) {
                        PropRes::Slot(k) => h.borrow().props().get(&k).cloned(),
                        PropRes::Dynamic => h.borrow().props().get(name).cloned(),
                        PropRes::Denied(_) => None,
                    };
                    let uninit = match classes::resolve_prop(&self.classes, cls, scope, name) {
                        PropRes::Slot(k) => present.is_none() && self.uninit_typed(&h, &k),
                        _ => false,
                    };
                    if present.is_none() {
                        let lazy = match classes::resolve_prop(&self.classes, cls, scope, name) {
                            PropRes::Slot(k) => self.lazy_slot(&h, Some(&k)),
                            PropRes::Dynamic => {
                                self.lazy_slot(&h, None)
                                    && self.magic(&h, cls, b"__get", name).is_none()
                            }
                            PropRes::Denied(_) => false,
                        };
                        if lazy {
                            return MagicSplit::Lazy(i, h);
                        }
                    }
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
                    // Native dimension handlers (PDORow) count too.
                    return if self.class_implements(cls, b"ArrayAccess")
                        || self.handlers_of(&h).is_some_and(|t| t.dimension.is_some())
                    {
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

    /// Initialize the lazy object `h`: a cell holding the object the
    /// cursor path continues on.
    fn lazy_cursor_cell(&mut self, h: &ObjectHandle) -> Result<Rc<RefCell<Value>>, Thrown> {
        let inst = self.lazy_init(h)?;
        Ok(Rc::new(RefCell::new(Value::Object(inst))))
    }

    /// zend_std_get_property_ptr_ptr's lazy_init: initialize `h` and run `f`
    /// on the instance's cell; a fetch the proxy's `__get` guard let through
    /// holds the same guard on the instance.
    fn lazy_ptr_fetch<T>(
        &mut self,
        h: &ObjectHandle,
        step: &CursorStep,
        f: impl FnOnce(&mut Self, Rc<RefCell<Value>>) -> Result<T, Thrown>,
    ) -> Result<T, Thrown> {
        let guard = match step {
            CursorStep::Prop(name) => {
                let cls = h.borrow().class;
                let held = self.lookup_method(cls, b"__get").is_some()
                    && self.magic(h, cls, b"__get", name).is_none();
                held.then(|| name.clone())
            }
            _ => None,
        };
        let cell = self.lazy_cursor_cell(h)?;
        let inst = match &*cell.borrow() {
            Value::Object(o) if guard.is_some() => Some(o.clone()),
            _ => None,
        };
        match (inst, guard) {
            (Some(inst), Some(name)) => {
                let icls = inst.borrow().class;
                let has_get = self.lookup_method(icls, b"__get").is_some();
                self.with_guard(&inst, has_get, b"__get", &name, |vm| f(vm, cell))
            }
            _ => f(self, cell),
        }
    }

    /// Whether register `r` holds a PHP reference (IS_REFERENCE), not just a
    /// cell rhp shares with the symbol table: a global-scope variable is
    /// one only once made so (`ref_globals`), and an include frame's
    /// variables are bound to its caller's.
    pub(crate) fn reg_is_php_ref(&self, top: usize, r: Reg) -> bool {
        let f = &self.stack[top];
        if !matches!(f.regs[r as usize], Value::Ref(_)) {
            return false;
        }
        match f.proto.cv_var_names.get(r as usize) {
            Some(name) if f.global_scope => self.ref_globals.contains(name.as_bytes()),
            Some(_) => f.include_caller().is_none(),
            None => true,
        }
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
        if self.cursor_hooked(top, &h, &name) {
            return Ok(match self.hook_write_fetch(top, &h, &name)? {
                Value::Ref(rc) => rc,
                other => Rc::new(RefCell::new(other)),
            });
        }
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
        if self.cursor_readonly_check(top, steps, true)? {
            let v = self.cursor_read_steps(top, steps);
            return Ok(Rc::new(RefCell::new(deref(&v))));
        }
        let (i, h, name) = match self.cursor_magic_split(top, steps) {
            MagicSplit::Incomplete(h) => return Err(self.incomplete_error(&h, "modify a property")),
            MagicSplit::Plain => {
                // `&$GLOBALS['x']`: the symbol-table cell is the variable.
                if let (true, CursorRoot::Cell(rc), Some(n)) =
                    (steps.is_empty(), &self.cursor_root, &self.cursor_global)
                {
                    let rc = rc.clone();
                    if !self.ref_globals.contains(&n[..]) {
                        self.ref_globals.insert(n.clone());
                    }
                    crate::dispatch::cell_define(&rc);
                    return Ok(rc);
                }
                self.cursor_dyn_check(top, steps, true)?;
                // ZEND_FETCH_REF on a typed property: the reference gets
                // it as a type source.
                let leaf = if steps.is_empty() {
                    self.cursor_static_prop().map(|st| (None, st.cls, st.name))
                } else if matches!(steps.last(), Some(CursorStep::Prop(_))) {
                    self.cursor_leaf_slot(top, steps)
                        .map(|(h, k)| (Some(h), 0, k))
                } else {
                    None
                };
                let cx = PropCtx {
                    classes: &self.classes,
                    typed: &self.typed_refs,
                    scope: self.stack[top].defining_class,
                    called: self.stack[top].called_class,
                    incdec: false,
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
                if let (Ok(rc), Some((obj, cls, key))) = (&r, leaf) {
                    match obj {
                        Some(h) => self.add_prop_ref_source(rc, &h, &key),
                        None => self.add_static_ref_source(rc, cls, key.as_bytes()),
                    }
                }
                self.flush_path_diags()?;
                return r;
            }
            MagicSplit::Leaf(h, name) => (steps.len() - 1, h, name),
            MagicSplit::Nested(i, h, name) => (i, h, name),
            MagicSplit::Lazy(i, h) => {
                return self.lazy_ptr_fetch(&h, &steps[i], |vm, cell| {
                    vm.with_cursor_root(cell, |vm| vm.cursor_make_ref(top, &steps[i..]))
                });
            }
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
            typed: &self.typed_refs,
            scope: self.stack[top].defining_class,
            called: self.stack[top].called_class,
            incdec: self.incdec_write,
        };
        match &self.cursor_root {
            CursorRoot::Cv(r) => read_path(&self.stack[top].regs[*r as usize], steps, &cx),
            CursorRoot::Cell(rc) => read_path(&rc.borrow(), steps, &cx),
        }
    }

    /// Readonly and asymmetric visibility along a cursor path: writing the
    /// property itself follows the readonly / set visibility rules; writing
    /// *through* one (`$o->ro[] = 1`, `unset($o->ro[0])`) or taking a
    /// reference is php's "Cannot indirectly modify" (zend_std_read_property
    /// for BP_VAR_W/RW/UNSET), ahead of the uninitialized check, unless it
    /// holds an object (then a reference gets a copy: `Ok(true)`).
    fn cursor_readonly_check(
        &mut self,
        top: usize,
        steps: &[CursorStep],
        make_ref: bool,
    ) -> Result<bool, Thrown> {
        self.cursor_through_check(top, steps, make_ref, false)
            .map(|r| r == Through::Copy)
    }

    pub(crate) fn cursor_through_check(
        &mut self,
        top: usize,
        steps: &[CursorStep],
        make_ref: bool,
        unset: bool,
    ) -> Result<Through, Thrown> {
        self.cursor_leaf = None;
        // Only properties along the path are checked.
        if !steps.iter().any(|s| matches!(s, CursorStep::Prop(_))) {
            return Ok(Through::Plain);
        }
        let scope = self.stack[top].defining_class;
        let mut cur = self.cursor_root_value(top);
        for (i, step) in steps.iter().enumerate() {
            let next = match (deref(&cur), step) {
                (Value::Array(a), CursorStep::Key(k)) => {
                    match ArrayKey::from_value(k).and_then(|key| a.get(&key).cloned()) {
                        Some(v) => v,
                        None => return Ok(Through::Plain),
                    }
                }
                (Value::Object(h), CursorStep::Prop(name)) => {
                    let cls = h.borrow().class;
                    let PropRes::Slot(key) = classes::resolve_prop(&self.classes, cls, scope, name)
                    else {
                        return Ok(Through::Plain);
                    };
                    let val = h.borrow().props().get(&key).cloned();
                    let Some(pe) = self.class(cls).props.get(&key) else {
                        return Ok(Through::Plain);
                    };
                    // The typed property a write to the leaf lands in.
                    let typed_leaf = match &pe.ty {
                        Some(ty) if i + 1 == steps.len() => {
                            Some((cls, pe.declaring_class, pe.name.clone(), ty.clone()))
                        }
                        _ => None,
                    };
                    let checked = pe.readonly || pe.set_visibility.is_some();
                    if typed_leaf.is_some() {
                        self.cursor_leaf = typed_leaf;
                    }
                    if checked {
                        let ro = self.class(cls).props[&key].readonly;
                        let last = i + 1 == steps.len();
                        if last && !make_ref {
                            if !unset {
                                let called = self.stack[top].called_class;
                                self.check_readonly_write(
                                    &h,
                                    &key,
                                    val.is_some(),
                                    scope,
                                    called,
                                    "modify",
                                )?;
                                // The write uses up the `__clone` window.
                                if ro && val.is_some() {
                                    self.consume_reinit(&h, &key);
                                }
                            }
                        } else if ro
                            || classes::set_access_ok(&self.classes, cls, scope, &key).is_err()
                        {
                            match val.as_ref().map(deref) {
                                Some(Value::Object(_)) if last => return Ok(Through::Copy),
                                Some(Value::Object(_)) => {}
                                None if unset => return Ok(Through::Stop),
                                _ => return Err(self.indirect_modification_err(top, cls, &key)),
                            }
                        }
                    }
                    match val {
                        Some(v) => v,
                        None => return Ok(Through::Plain),
                    }
                }
                _ => return Ok(Through::Plain),
            };
            cur = next;
        }
        Ok(Through::Plain)
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
        if !steps.iter().any(|s| matches!(s, CursorStep::Prop(_))) {
            return Ok(());
        }
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
                    let val = h.borrow().props().get(&key).cloned();
                    match val {
                        Some(v) => v,
                        None => {
                            let leaf = i + 1 == steps.len();
                            let set =
                                !make_ref && leaf && self.magic(&h, cls, b"__set", name).is_some();
                            if dynamic && !set {
                                // Held here by `h`, and by `cur` unless
                                // that is a reference.
                                let locals = 1 + usize::from(
                                    matches!(&cur, Value::Object(o) if Rc::ptr_eq(o, &h)),
                                );
                                self.check_dynamic_prop_held(top, &h, locals, name)?;
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
        // (`$o->p = &$x` fetches the property for a reference.)
        self.cursor_readonly_check(top, steps, matches!(v, Value::Ref(_)))?;
        let mut v = v;
        let mut bind = None;
        match self.cursor_magic_split(top, steps) {
            MagicSplit::Incomplete(h) => return Err(self.incomplete_error(&h, "modify a property")),
            MagicSplit::Plain => {
                self.flush_deferred_undef(top)?;
                self.cursor_dyn_check(top, steps, false)?;
                // zend_assign_to_typed_prop / zend_binary_assign_op_typed_prop.
                if steps.is_empty() && !matches!(v, Value::Ref(_)) && self.cursor_static.is_some() {
                    v = self.cursor_static_coerce(top, v)?;
                } else if !matches!(v, Value::Ref(_)) {
                    if let Some((cls, decl, name, ty)) = self.cursor_leaf.take() {
                        let strict = self.stack[top].proto.strict_types;
                        v =
                            self.verify_type_for_prop(&ty, decl, cls, name.as_bytes(), v, strict)?;
                        self.typed_write_result = Some(v.clone());
                    }
                }
                if matches!(v, Value::Ref(_)) {
                    bind = self.cursor_bind_ref_check(top, steps, &v)?;
                } else if self.cursor_static.is_some() {
                    self.cursor_static_auto_init(top, steps)?;
                }
            }
            // `$o->p = &$x` through a hook (zend_assign_to_property_reference):
            // the write fetch runs the get hook, then the reference cannot be
            // made.
            MagicSplit::Leaf(h, name)
                if matches!(v, Value::Ref(_)) && self.cursor_hooked(top, &h, &name) =>
            {
                let e = Thrown::error("Error", "Cannot assign by reference to overloaded object");
                return Err(match self.hook_write_fetch(top, &h, &name) {
                    Ok(_) => e,
                    Err(prev) => {
                        use rhp_builtins::Host;
                        self.chain_thrown(e, prev)
                    }
                });
            }
            // ZEND_ASSIGN_OBJ_REF on a property served by `__get`: the
            // write fetch reads it, then the reference cannot be made.
            MagicSplit::Leaf(h, name) if matches!(v, Value::Ref(_)) => {
                self.fetching_as(builtins::MagicFetch::Write, |vm| {
                    vm.prop_fetch(top, Value::Object(h), name)
                })?;
                return Err(Thrown::error(
                    "Error",
                    "Cannot assign by reference to overloaded object",
                ));
            }
            MagicSplit::Leaf(h, name) => {
                self.flush_deferred_undef(top)?;
                return self.prop_store(top, Value::Object(h), name, v);
            }
            // (`$o->p = &$x` is a write fetch: get_property_ptr_ptr.)
            MagicSplit::Lazy(i, h) if matches!(v, Value::Ref(_)) => {
                return self.lazy_ptr_fetch(&h, &steps[i], |vm, cell| {
                    vm.with_cursor_root(cell, |vm| vm.cursor_write(top, &steps[i..], v))
                });
            }
            MagicSplit::Lazy(i, h) => {
                let cell = self.lazy_cursor_cell(&h)?;
                return self.with_cursor_root(cell, |vm| vm.cursor_write(top, &steps[i..], v));
            }
            MagicSplit::Nested(i, h, name) => {
                let cell = self.magic_write_cell(top, h, name)?;
                let rest = &steps[i + 1..];
                return self.with_cursor_root(cell, |vm| vm.cursor_write(top, rest, v));
            }
            // `$obj[$k] = v` / `$obj[] = v` on an ArrayAccess object ->
            // offsetSet; below it, the element is fetched with offsetGet.
            MagicSplit::Offset(i, h) => {
                let mut key = aa_key(&steps[i]).unwrap_or(Value::Null);
                if i + 1 == steps.len() {
                    if let Some(d) = self.handlers_of(&h).and_then(|t| t.dimension) {
                        (d.write)(self, &h, aa_key_opt(&steps[i]).as_ref(), v)?;
                        if !self.destructibles.is_empty() {
                            self.dtor_check = true;
                        }
                        return Ok(());
                    }
                    // spl_array_write_dimension_ex: a reference to null is
                    // an offset (get_hash_key's deprecation), not an append.
                    if i < 64
                        && self.cursor_ref_null & (1 << i) != 0
                        && matches!(steps[i], CursorStep::Key(Value::Null))
                        && self.spl_array_owner(&h, b"offsetset").is_some()
                    {
                        self.diag(builtins::E_DEPRECATED, NULL_OFFSET_DEPRECATION)?;
                        key = Value::str(Vec::new());
                    }
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
            typed: &self.typed_refs,
            scope: self.stack[top].defining_class,
            called: self.stack[top].called_class,
            incdec: self.incdec_write,
        };
        // An object written to a string offset is converted afterwards.
        let obj_val = matches!(v, Value::Object(_) | Value::Closure(_)).then(|| v.clone());
        let bound = bind.is_some().then(|| v.clone());
        let r = match &self.cursor_root {
            CursorRoot::Cv(r) => {
                let r = *r as usize;
                assign_path(&mut self.stack[top].regs[r], steps, v, &cx)
            }
            CursorRoot::Cell(rc) => {
                let rc = rc.clone();
                // A global's cell that is itself a typed reference
                // (`$GLOBALS['x']` bound to a typed property): written
                // through it as a reference.
                if cx.typed.has(&rc) && !(steps.is_empty() && matches!(v, Value::Ref(_))) {
                    assign_path(&mut Value::Ref(rc), steps, v, &cx)
                } else {
                    let r = assign_path(&mut rc.borrow_mut(), steps, v, &cx);
                    r
                }
            }
        };
        flush_deferred_writes();
        flush_replaced();
        let r = self.finish_typed_stop(top, r);
        if let (Some(b), Some(bv), true) = (bind, &bound, r.is_ok()) {
            self.finish_ref_bind(b, bv);
        }
        self.flush_path_diags()?;
        if STRING_OFFSET_OBJECT.with(|f| f.replace(false)) {
            return self.string_offset_object_write(top, steps, obj_val, r);
        }
        // The overwritten element may have held the last reference to an
        // object: its destructor runs before the next statement, as in php.
        if self.release_pending() {
            self.dtor_check = true;
        }
        r
    }

    /// The warning of an undefined assigned variable whose read was put
    /// off until the containers were fetched.
    fn flush_deferred_undef(&mut self, top: usize) -> Result<(), Thrown> {
        match self.deferred_undef_src.take() {
            Some(r) => self.undef_cv(top, r).map(|_| ()),
            None => Ok(()),
        }
    }

    /// Remember the static property a cursor is rooted at, when it has a
    /// type or a set visibility to check.
    pub(crate) fn note_cursor_static(
        &mut self,
        cell: &Rc<RefCell<Value>>,
        cls: ClassId,
        name: &[u8],
    ) {
        if !self.class(cls).checked_statics {
            return;
        }
        let Some(sp) = self.class(cls).static_props.get(name) else {
            return;
        };
        if sp.ty.is_some() || sp.set_visibility.is_some() {
            self.cursor_static = Some(StaticRoot {
                cell: cell.clone(),
                cls,
                name: PhpStr::from_bytes(name),
                decl: sp.declaring_class,
                set_vis: sp.set_visibility,
                ty: sp.ty.clone(),
            });
        }
    }

    /// The static property the open cursor is rooted at, while the root is
    /// still its cell.
    pub(crate) fn cursor_static_prop(&self) -> Option<StaticRoot> {
        let st = self.cursor_static.as_ref()?;
        match &self.cursor_root {
            CursorRoot::Cell(rc) if Rc::ptr_eq(rc, &st.cell) => Some(st.clone()),
            _ => None,
        }
    }

    /// "Typed static property C::$p must not be accessed before
    /// initialization".
    pub(crate) fn uninit_static_err(&self, cls: ClassId, name: &[u8]) -> Thrown {
        let decl = self
            .class(cls)
            .static_props
            .get(name)
            .map_or(cls, |sp| sp.declaring_class);
        Thrown::error(
            "Error",
            format!(
                "Typed static property {}::${} must not be accessed before initialization",
                String::from_utf8_lossy(self.class(decl).name.as_bytes()),
                String::from_utf8_lossy(name)
            ),
        )
    }

    /// php's asymmetric visibility and typed checks for a cursor rooted
    /// at a static property (ZEND_ASSIGN_STATIC_PROP and friends). Returns
    /// `v`.
    pub(crate) fn cursor_static_check(
        &mut self,
        top: usize,
        steps: &[CursorStep],
        op: StaticOp,
        v: Value,
    ) -> Result<Value, Thrown> {
        let Some(StaticRoot {
            cls,
            name,
            decl,
            set_vis,
            ty,
            ..
        }) = self.cursor_static_prop()
        else {
            return Ok(v);
        };
        let cur = self.cursor_root_value(top);
        let undef = match &self.cursor_root {
            CursorRoot::Cell(rc) => rc.borrow().is_undef(),
            CursorRoot::Cv(_) => false,
        };
        let direct = steps.is_empty();
        let op = if direct || matches!(op, StaticOp::Fetch | StaticOp::Unset) {
            op
        } else {
            StaticOp::Fetch
        };
        if let Some(sv) = set_vis {
            let scope = self.stack[top].defining_class;
            let ok = scope == Some(decl)
                || (sv == rhp_values::Visibility::Protected
                    && scope.is_some_and(|s| {
                        self.class(s).ancestors.contains(&decl)
                            || self.class(decl).ancestors.contains(&s)
                    }));
            if !ok {
                let what = match op {
                    StaticOp::Assign => Some("modify"),
                    StaticOp::Ref | StaticOp::Rw => Some("indirectly modify"),
                    // An object below a nested write is used as is; a
                    // reference to the static itself is refused (php's
                    // ZEND_FETCH_REF makes the slot a reference first).
                    StaticOp::Fetch => match deref(&cur) {
                        Value::Object(_) if !direct => None,
                        _ => Some("indirectly modify"),
                    },
                    StaticOp::Unset if undef || matches!(deref(&cur), Value::Object(_)) => None,
                    StaticOp::Unset => Some("indirectly modify"),
                };
                if let Some(what) = what {
                    let vis = if sv == rhp_values::Visibility::Private {
                        "private(set)"
                    } else {
                        "protected(set)"
                    };
                    let from = match self.stack[top].called_class {
                        Some(c) => format!(
                            "scope {}",
                            String::from_utf8_lossy(self.class(c).name.as_bytes())
                        ),
                        None => "global scope".into(),
                    };
                    return Err(Thrown::error(
                        "Error",
                        format!(
                            "Cannot {what} {vis} property {}::${} from {from}",
                            String::from_utf8_lossy(self.class(decl).name.as_bytes()),
                            String::from_utf8_lossy(name.as_bytes())
                        ),
                    ));
                }
            }
        }
        if ty.is_some() && op == StaticOp::Rw && undef {
            return Err(self.uninit_static_err(cls, name.as_bytes()));
        }
        Ok(v)
    }

    /// `v` coerced to the type of the typed static property the cursor is
    /// rooted at (zend_verify_property_type), for a write to the root.
    fn cursor_static_coerce(&mut self, top: usize, v: Value) -> Result<Value, Thrown> {
        let Some(StaticRoot {
            cls,
            name,
            decl,
            ty: Some(ty),
            ..
        }) = self.cursor_static_prop()
        else {
            return Ok(v);
        };
        let strict = self.stack[top].proto.strict_types;
        let v = self.verify_type_for_prop(&ty, decl, cls, name.as_bytes(), v, strict)?;
        self.typed_write_result = Some(v.clone());
        Ok(v)
    }

    /// The readonly / set visibility error a write to the leaf of `steps`
    /// would raise, if any.
    pub(crate) fn cursor_readonly_leaf_err(
        &mut self,
        top: usize,
        steps: &[CursorStep],
    ) -> Option<Thrown> {
        self.cursor_readonly_check(top, steps, false).err()
    }

    /// The declared typed property the leaf of a cursor path is (php's
    /// zend_object_fetch_property_type_info on the fetched slot): its
    /// object's class and storage key.
    pub(crate) fn cursor_leaf_typed(
        &self,
        top: usize,
        steps: &[CursorStep],
    ) -> Option<(ClassId, PhpStr)> {
        self.cursor_leaf_slot(top, steps)
            .map(|(h, k)| (h.borrow().class, k))
    }

    /// [`Vm::cursor_leaf_typed`] with the object.
    pub(crate) fn cursor_leaf_slot(
        &self,
        top: usize,
        steps: &[CursorStep],
    ) -> Option<(ObjectHandle, PhpStr)> {
        let (CursorStep::Prop(name), prefix) = steps.split_last()? else {
            return None;
        };
        let parent = if prefix.is_empty() {
            self.cursor_root_value(top)
        } else {
            self.cursor_read_steps(top, prefix)
        };
        let Value::Object(h) = deref(&parent) else {
            return None;
        };
        let cls = h.borrow().class;
        let PropRes::Slot(key) =
            classes::resolve_prop(&self.classes, cls, self.stack[top].defining_class, name)
        else {
            return None;
        };
        self.class(cls).props.get(&key)?.ty.as_ref()?;
        Some((h, key))
    }

    /// What [`assign_path`] left to the VM (see [`TypedStop`]): the write
    /// through a typed reference, or the auto-initialization error.
    #[inline]
    fn finish_typed_stop(
        &mut self,
        top: usize,
        r: Result<Option<Box<TypedStop>>, Thrown>,
    ) -> Result<(), Thrown> {
        match r? {
            None => Ok(()),
            Some(stop) => self.typed_stop(top, *stop),
        }
    }

    #[cold]
    #[inline(never)]
    fn typed_stop(&mut self, top: usize, stop: TypedStop) -> Result<(), Thrown> {
        match stop {
            TypedStop::Leaf(rc, v) => {
                let strict = self.stack[top].proto.strict_types;
                let (c, old) = self.assign_to_ref(&rc, v, strict)?;
                self.typed_write_result = Some(c);
                drop(old);
                Ok(())
            }
            TypedStop::RefInit(rc) => self.verify_ref_array_assignable(&rc),
            TypedStop::PropInit(cls, key) => match self.prop_src_info(cls, &key) {
                Some(info) => self.check_auto_init(&info),
                None => Ok(()),
            },
        }
    }

    /// The typed reference the leaf of a cursor path holds, if any.
    pub(crate) fn cursor_leaf_ref(
        &mut self,
        top: usize,
        steps: &[CursorStep],
    ) -> Option<Rc<RefCell<Value>>> {
        if self.typed_refs.is_empty() {
            return None;
        }
        let scope = self.stack[top].defining_class;
        let mut cur = match &self.cursor_root {
            // A global's cell may be the typed reference itself.
            CursorRoot::Cell(rc) if self.typed_refs.has(rc) => Value::Ref(rc.clone()),
            _ => self.cursor_root_value(top),
        };
        for step in steps {
            let next = match (deref(&cur), step) {
                (Value::Array(a), CursorStep::Key(k)) => {
                    ArrayKey::from_value(k).and_then(|key| a.get(&key).cloned())
                }
                (Value::Object(h), CursorStep::Prop(name)) => {
                    let cls = h.borrow().class;
                    let key = match classes::resolve_prop(&self.classes, cls, scope, name) {
                        PropRes::Slot(k) => k,
                        PropRes::Dynamic => name.clone(),
                        PropRes::Denied(_) => return None,
                    };
                    let v = h.borrow().props().get(&key).cloned();
                    v
                }
                _ => None,
            };
            cur = next?;
        }
        match cur {
            Value::Ref(rc) if self.typed_refs.has(&rc) => Some(rc),
            _ => None,
        }
    }

    /// A write below a typed static property at the cursor root that would
    /// make it an array (ZEND_FETCH_STATIC_PROP_W with ZEND_FETCH_DIM_WRITE).
    #[cold]
    #[inline(never)]
    fn cursor_static_auto_init(&self, top: usize, steps: &[CursorStep]) -> Result<(), Thrown> {
        if !matches!(steps.first(), Some(CursorStep::Key(_) | CursorStep::Append)) {
            return Ok(());
        }
        let Some(st) = self.cursor_static_prop() else {
            return Ok(());
        };
        let cur = self.cursor_root_value(top);
        if !promotes_to_array(&deref(&cur)) {
            return Ok(());
        }
        match self.static_src_info(st.cls, st.name.as_bytes()) {
            Some(info) => self.check_auto_init(&info),
            None => Ok(()),
        }
    }

    /// zend_throw_auto_init_in_prop_error unless typed property `s` takes
    /// an array.
    fn check_auto_init(&self, s: &crate::refsrc::SrcInfo) -> Result<(), Thrown> {
        use rhp_bytecode::TypeKind as K;
        if s.ty
            .kinds
            .iter()
            .any(|k| matches!(k, K::Array | K::Iterable | K::Mixed))
        {
            return Ok(());
        }
        Err(Thrown::type_error(format!(
            "Cannot auto-initialize an array inside property {}::${} of type {}",
            String::from_utf8_lossy(self.class(s.decl).name.as_bytes()),
            String::from_utf8_lossy(s.name.as_bytes()),
            self.prop_type_string(&s.ty, s.decl, Some(s.cls))
        )))
    }

    /// The typed slot a `lval = &$x` binds (zend_assign_to_typed_property_
    /// reference): checks the reference against it and drops the old
    /// reference's source. What to add as a source after the bind.
    #[cold]
    #[inline(never)]
    fn cursor_bind_ref_check(
        &mut self,
        top: usize,
        steps: &[CursorStep],
        v: &Value,
    ) -> Result<Option<RefBind>, Thrown> {
        let strict = self.stack[top].proto.strict_types;
        if steps.is_empty() {
            let Some(st) = self.cursor_static_prop() else {
                return Ok(None);
            };
            let Some(info) = self.static_src_info(st.cls, st.name.as_bytes()) else {
                return Ok(None);
            };
            self.verify_prop_assignable_by_ref(&info, v, strict)?;
            if let Value::Ref(old) = &*st.cell.borrow() {
                self.del_static_ref_source(old, info.decl, st.name.as_bytes());
            }
            return Ok(Some(RefBind::Static(st.cls, st.name)));
        }
        let Some((h, key)) = self.cursor_leaf_slot(top, steps) else {
            return Ok(None);
        };
        let cls = h.borrow().class;
        let Some(info) = self.prop_src_info(cls, &key) else {
            return Ok(None);
        };
        self.verify_prop_assignable_by_ref(&info, v, strict)?;
        let old = h.borrow().props().get(&key).cloned();
        if let Some(Value::Ref(old)) = old {
            self.del_prop_ref_source(&old, &h, &key);
        }
        Ok(Some(RefBind::Prop(h, key)))
    }

    /// Add the source [`Vm::cursor_bind_ref_check`] found.
    #[cold]
    #[inline(never)]
    fn finish_ref_bind(&mut self, bind: RefBind, v: &Value) {
        let Value::Ref(rc) = v else { return };
        match bind {
            RefBind::Static(cls, name) => self.add_static_ref_source(rc, cls, name.as_bytes()),
            RefBind::Prop(h, key) => self.add_prop_ref_source(rc, &h, &key),
        }
    }

    pub(crate) fn cursor_unset(&mut self, top: usize, steps: &[CursorStep]) -> Result<(), Thrown> {
        if self.cursor_through_check(top, steps, false, true)? == Through::Stop {
            return Ok(());
        }
        match self.cursor_magic_split(top, steps) {
            MagicSplit::Incomplete(h) => return Err(self.incomplete_error(&h, "modify a property")),
            MagicSplit::Plain | MagicSplit::Leaf(..) => {
                // A property leaf is unset by its object (zend_std_unset_property).
                if let Some((CursorStep::Prop(name), prefix)) = steps.split_last() {
                    let parent = if prefix.is_empty() {
                        self.cursor_root_value(top)
                    } else {
                        self.cursor_read_steps(top, prefix)
                    };
                    if let Value::Object(h) = deref(&parent) {
                        return self.prop_unset(top, Value::Object(h), name.clone());
                    }
                }
            }
            MagicSplit::Lazy(i, h) => {
                let cell = self.lazy_cursor_cell(&h)?;
                return self.with_cursor_root(cell, |vm| vm.cursor_unset(top, &steps[i..]));
            }
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
                    if let Some(d) = self.handlers_of(&h).and_then(|t| t.dimension) {
                        (d.unset)(self, &h, &key)?;
                        if !self.destructibles.is_empty() {
                            self.dtor_check = true;
                        }
                        return Ok(());
                    }
                    if let Some(base) = self.spl_array_owner(&h, b"offsetunset") {
                        // ArrayIterator's own offsetUnset keeps its position.
                        if self.class(base).name.as_bytes() == b"ArrayObject" {
                            return self.spl_array_unset_dim(&h, base, &key);
                        }
                        self.spl_array_key_in(&h, &key, SplKeyUse::Unset)?;
                    }
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
            typed: &self.typed_refs,
            scope: self.stack[top].defining_class,
            called: self.stack[top].called_class,
            incdec: self.incdec_write,
        };
        let r = match &self.cursor_root {
            CursorRoot::Cv(r) => unset_path(&mut self.stack[top].regs[*r as usize], steps, &cx),
            CursorRoot::Cell(rc) => {
                let rc = rc.clone();
                let r = unset_path(&mut rc.borrow_mut(), steps, &cx);
                r
            }
        };
        flush_replaced();
        self.flush_path_diags()?;
        // The removed element may have held the last reference to an object.
        if self.release_pending() {
            self.dtor_check = true;
        }
        r
    }

    /// zend_assign_to_string_offset with an object value: after the
    /// offset's checks, the value goes through `__toString`, then the
    /// write runs again with the string (its offset diagnostics already
    /// shown).
    #[cold]
    fn string_offset_object_write(
        &mut self,
        top: usize,
        steps: &[CursorStep],
        v: Option<Value>,
        r: Result<(), Thrown>,
    ) -> Result<(), Thrown> {
        r?;
        let Some(v) = v else {
            return Ok(());
        };
        let s = Value::str(self.to_string_vm(&v)?);
        let cx = PropCtx {
            classes: &self.classes,
            typed: &self.typed_refs,
            scope: self.stack[top].defining_class,
            called: self.stack[top].called_class,
            incdec: false,
        };
        let r = match &self.cursor_root {
            CursorRoot::Cv(r) => assign_path(&mut self.stack[top].regs[*r as usize], steps, s, &cx),
            CursorRoot::Cell(rc) => {
                let rc = rc.clone();
                let r = assign_path(&mut rc.borrow_mut(), steps, s, &cx);
                r
            }
        };
        flush_deferred_writes();
        // Keep only what the conversion's write adds after the offset.
        PATH_DIAGS.with(|d| {
            d.borrow_mut()
                .retain(|(_, m)| m.starts_with(b"Only the first byte"))
        });
        self.flush_path_diags()?;
        r.map(|_| ())
    }

    /// `$str[$k]->…` written: the offset's own checks, then "Cannot use
    /// string offset as an object".
    pub(crate) fn string_offset_as_object(&mut self, top: usize, k: &Value) -> Thrown {
        let cx = PropCtx {
            classes: &self.classes,
            typed: &self.typed_refs,
            scope: self.stack[top].defining_class,
            called: self.stack[top].called_class,
            incdec: self.incdec_write,
        };
        let checked = string_offset(k, false, &cx);
        if let Err(t) = self.flush_path_diags() {
            return t;
        }
        match checked {
            Err(t) => t,
            Ok(_) => Thrown::error("Error", "Cannot use string offset as an object"),
        }
    }

    /// Emit the diagnostics a cursor-path walk queued ([`path_diag`]).
    fn flush_path_diags(&mut self) -> Result<(), Thrown> {
        let diags = PATH_DIAGS.with(|d| std::mem::take(&mut *d.borrow_mut()));
        for (level, msg) in diags {
            if self.rw_null_keys > 0 && msg == NULL_OFFSET_DEPRECATION.as_bytes() {
                self.rw_null_keys -= 1;
                continue;
            }
            if self.skip_false_dep > 0 && msg == FALSE_TO_ARRAY.as_bytes() {
                self.skip_false_dep -= 1;
                continue;
            }
            self.diag(level, &msg)?;
        }
        Ok(())
    }

    /// Whether `$h[k]` goes through the object's dimension handlers: an
    /// ArrayAccess class, or one with native ones (ResourceBundle).
    pub(crate) fn offset_object(&self, h: &ObjectHandle) -> bool {
        self.class_implements(h.borrow().class, b"ArrayAccess")
            || self.handlers_of(h).is_some_and(|t| t.dimension.is_some())
    }

    /// [`Instr::DimAppendW`]: FETCH_DIM_W with no dimension, as the object
    /// of a property write. Only a variable (a CV) keeps the element an
    /// array or null gets; a fetched container is read.
    pub(crate) fn dim_append_w(&mut self, top: usize, arr: Reg) -> Result<Value, Thrown> {
        let a = self.reg_get(top, arr);
        match &a {
            Value::Object(h) if self.offset_object(h) => {
                let cell = self.aa_fetch_w(h, &CursorStep::Append, Value::Null, DimFetch::W)?;
                let v = cell.borrow().clone();
                return Ok(v);
            }
            Value::Str(_) => {
                return Err(Thrown::error(
                    "Error",
                    "[] operator not supported for strings",
                ))
            }
            Value::Object(h) => {
                let name = self.classes[h.borrow().class as usize].name.clone();
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Cannot use object of type {} as array",
                        String::from_utf8_lossy(name.as_bytes())
                    ),
                ));
            }
            Value::Closure(_) => {
                return Err(Thrown::error(
                    "Error",
                    "Cannot use object of type Closure as array",
                ))
            }
            Value::Bool(false) => self.diag(builtins::E_DEPRECATED, FALSE_TO_ARRAY)?,
            Value::Null | Value::Undef | Value::Array(_) => {}
            _ => {
                return Err(Thrown::error(
                    "Error",
                    "Cannot use a scalar value as an array",
                ))
            }
        }
        if arr >= self.stack[top].proto.num_cvs {
            return Ok(Value::Null);
        }
        let append = |slot: &mut Value| -> Result<(), Thrown> {
            if !matches!(slot, Value::Array(_)) {
                *slot = Value::Array(PhpArray::new());
            }
            let Value::Array(a) = slot else {
                unreachable!("made an array above")
            };
            *a.append_mut().ok_or_else(cannot_add_element)? = Value::Null;
            Ok(())
        };
        match &mut self.stack[top].regs[arr as usize] {
            Value::Ref(rc) => append(&mut rc.borrow_mut())?,
            slot => append(slot)?,
        }
        Ok(Value::Null)
    }

    /// `offsetGet($key)` on an ArrayAccess object.
    pub(crate) fn aa_offset_get(&mut self, h: &ObjectHandle, key: Value) -> Result<Value, Thrown> {
        if let Some(d) = self.handlers_of(h).and_then(|t| t.dimension) {
            return Ok((d.read)(self, h, Some(&key), DimType::R)?.unwrap_or(Value::Null));
        }
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
        if let Some(d) = self.handlers_of(h).and_then(|t| t.dimension) {
            return (d.has)(self, h, &key, false);
        }
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
                    self.cursor_steps = CursorSteps::from(&steps[i + 1..]);
                }
                MagicSplit::Lazy(i, h) => {
                    let cell = match self.lazy_cursor_cell(&h) {
                        Ok(c) => c,
                        Err(e) => {
                            self.cursor_steps = steps;
                            return Err(e);
                        }
                    };
                    self.cursor_root = CursorRoot::Cell(cell);
                    self.cursor_steps = CursorSteps::from(&steps[i..]);
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
        if let Some(i) = scope::named_cv(&self.stack[top].proto, name) {
            return Some(Ok(i));
        }
        // The global scope ({main}, top-level includes) is the global
        // symbol table.
        if self.stack[top].global_scope {
            return self.globals.get(name).map(|c| Err(c.clone()));
        }
        // An include/eval frame has no scope of its own: a variable it has no
        // slot for lives in the including scope.
        if let Some(caller) = self.stack[top].include_caller() {
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
        if let Some(i) = scope::named_cv(&self.stack[top].proto, name) {
            self.stack[top].regs[i as usize] = Value::Ref(cell);
        } else if let Some(caller) = self.stack[top].include_caller() {
            self.var_var_bind(caller, name, cell);
        } else {
            self.stack[top].dyn_vars.insert(name.to_vec(), cell);
        }
    }

    /// `unset($$n)`: drop frame `top`'s variable `name` (a compiled slot
    /// becomes undefined, as `unset($x)` does; a by-name variable is removed).
    pub(crate) fn var_var_unset(&mut self, top: usize, name: &[u8]) {
        if self.stack[top].global_scope {
            self.unset_global(name);
            return;
        }
        if let Some(i) = scope::named_cv(&self.stack[top].proto, name) {
            self.unset_cv(top, i);
        } else if let Some(caller) = self.stack[top].include_caller() {
            self.var_var_unset(caller, name);
        } else if let Some(c) = self.stack[top].dyn_vars.shift_remove(name) {
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
        if let Some(i) = scope::named_cv(&self.stack[top].proto, name) {
            return Ok(i);
        }
        if self.stack[top].global_scope {
            return Err(self.global_cell(name));
        }
        if let Some(caller) = self.stack[top].include_caller() {
            return Err(match self.var_var_slot(caller, name) {
                Ok(cv) => self.promote_slot(caller, cv),
                Err(cell) => cell,
            });
        }
        Err(self.stack[top].dyn_vars.cell(name))
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
        if let Some(d) = self.handlers_of(h).and_then(|t| t.dimension) {
            let ty = match mode {
                DimFetch::W => DimType::W,
                DimFetch::Rw => DimType::Rw,
                DimFetch::Unset => DimType::Unset,
            };
            let got = (d.read)(self, h, aa_key_opt(step).as_ref(), ty)?;
            return Ok(match got {
                Some(Value::Ref(rc)) => rc,
                other => {
                    let other = other.unwrap_or(Value::Null);
                    if !matches!(other, Value::Object(_) | Value::Closure(_)) {
                        self.indirect_overloaded_notice(h)?;
                    }
                    Rc::new(RefCell::new(other))
                }
            });
        }
        if matches!(step, CursorStep::Append) && self.spl_array_base(h).is_some() {
            return self.spl_array_append_fetch(h);
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

    /// The ArrayObject / ArrayIterator class `h` is or derives from.
    pub(crate) fn spl_array_base(&self, h: &ObjectHandle) -> Option<ClassId> {
        let cls = h.borrow().class;
        self.class(cls).ancestors.iter().copied().find(|&a| {
            let n = self.class(a).name.as_bytes();
            n == b"ArrayObject" || n == b"ArrayIterator"
        })
    }

    /// spl_array_has_dimension_ex: `isset($ao[$k])` (`check_empty` false)
    /// or `!empty($ao[$k])`. A user offsetExists is asked first; the
    /// storage is read directly unless a user offsetGet must supply the
    /// value.
    pub(crate) fn spl_array_has_dim(
        &mut self,
        h: &ObjectHandle,
        base: ClassId,
        k: Value,
        check_empty: bool,
    ) -> Result<bool, Thrown> {
        let user_has = self.spl_array_owner(h, b"offsetexists").is_none();
        let user_get = self.spl_array_owner(h, b"offsetget").is_none();
        let mut value = None;
        if user_has {
            if !self.aa_offset_exists(h, k.clone())? {
                return Ok(false);
            }
            if !check_empty {
                return Ok(true);
            }
            if user_get {
                value = Some(self.aa_offset_get(h, k.clone())?);
            }
        }
        let value = match value {
            Some(v) => v,
            None => {
                let storage = self.spl_array_storage(h, base)?;
                let akey = self.spl_array_key_in(h, &k, SplKeyUse::Isset)?;
                let found = match &*storage.borrow() {
                    Value::Array(a) => a.get(&akey).map(deref),
                    _ => None,
                };
                let Some(found) = found else {
                    return Ok(false);
                };
                if check_empty && user_get {
                    self.aa_offset_get(h, k)?
                } else {
                    found
                }
            }
        };
        if check_empty {
            self.truthy(&deref(&value))
        } else {
            Ok(!matches!(deref(&value), Value::Null))
        }
    }

    /// spl_array_read_dimension_ex in BP_VAR_IS (`$ao[$k] ?? ...`, a
    /// nested isset): one key conversion, no warning for a missing key.
    fn spl_array_read_is(
        &mut self,
        h: &ObjectHandle,
        base: ClassId,
        k: Value,
    ) -> Result<Value, Thrown> {
        let user_has = self.spl_array_owner(h, b"offsetexists").is_none();
        let user_get = self.spl_array_owner(h, b"offsetget").is_none();
        if user_get || user_has {
            if !self.spl_array_has_dim(h, base, k.clone(), false)? {
                return Ok(Value::Null);
            }
            if user_get {
                return self.aa_offset_get(h, k);
            }
        }
        let storage = self.spl_array_storage(h, base)?;
        let akey = self.spl_array_key_in(h, &k, SplKeyUse::Isset)?;
        let found = match &*storage.borrow() {
            Value::Array(a) => a.get(&akey).map(deref),
            _ => None,
        };
        Ok(found.unwrap_or(Value::Null))
    }

    /// spl_array_unset_dimension_ex for ArrayObject's built-in offsetUnset.
    fn spl_array_unset_dim(
        &mut self,
        h: &ObjectHandle,
        base: ClassId,
        k: &Value,
    ) -> Result<(), Thrown> {
        let akey = self.spl_array_key_in(h, k, SplKeyUse::Unset)?;
        let storage = self.spl_array_storage(h, base)?;
        if let Value::Array(a) = &mut *storage.borrow_mut() {
            a.remove(&akey);
        }
        // Backed by an object: its property table too.
        {
            let cx = PropCtx {
                classes: &self.classes,
                typed: &self.typed_refs,
                scope: Some(base),
                called: Some(base),
                incdec: false,
            };
            let mut obj = Value::Object(h.clone());
            let o = make_ref_path(&mut obj, &[CursorStep::Prop(PhpStr::from("__o"))], &cx)?;
            let backing = o.borrow().clone();
            if let Value::Object(o) = backing {
                let key = match akey {
                    rhp_values::ArrayKey::Int(n) => PhpStr::from_bytes(n.to_string().into_bytes()),
                    rhp_values::ArrayKey::Str(s) => s,
                };
                rhp_builtins::Host::prop_store(self, &o, &key, None);
            }
        }
        self.collect_released()
    }

    /// spl_array_read_dimension_ex for `$ao[]` with a user offsetGet: it is
    /// called with an undefined offset, so a required `$offset` fails ("not
    /// passed") and the result is php's uninitialized_zval (the notice).
    fn spl_array_append_fetch(&mut self, h: &ObjectHandle) -> Result<Rc<RefCell<Value>>, Thrown> {
        let cls = h.borrow().class;
        let Some(m) = self.lookup_method(cls, b"offsetget") else {
            self.indirect_overloaded_notice(h)?;
            return Ok(Rc::new(RefCell::new(Value::Null)));
        };
        // (zend_handle_undef_args: only a RECV_INIT takes its default; a
        // variadic one fails too, without a name.)
        let got = match m.proto.params.first() {
            Some(p) if p.default.is_none() => {
                let pname = if p.variadic {
                    String::new()
                } else {
                    let n = &m.proto.names[p.name as usize];
                    format!(" (${})", String::from_utf8_lossy(n.as_bytes()))
                };
                Err(Thrown::error(
                    "ArgumentCountError",
                    format!(
                        "{}::{}(): Argument #1{pname} not passed",
                        self.class_name_lossy(m.defining_class),
                        String::from_utf8_lossy(m.proto.name.as_bytes()),
                    ),
                ))
            }
            _ => self.fetching_as(builtins::MagicFetch::Write, |vm| {
                vm.call_sync(&m, Some(h.clone()), cls, vec![])
            }),
        };
        match got {
            Ok(Value::Ref(rc)) => Ok(rc),
            Ok(v @ (Value::Object(_) | Value::Closure(_))) => Ok(Rc::new(RefCell::new(v))),
            Err(e) => {
                self.indirect_overloaded_notice(h)?;
                Err(e)
            }
            Ok(v) => {
                self.indirect_overloaded_notice(h)?;
                Ok(Rc::new(RefCell::new(v)))
            }
        }
    }

    pub(crate) fn indirect_overloaded_notice(&mut self, h: &ObjectHandle) -> Result<(), Thrown> {
        let cls = h.borrow().class;
        let msg = format!(
            "Indirect modification of overloaded element of {} has no effect",
            String::from_utf8_lossy(crate::errors::c_name(self.class(cls).name.as_bytes()))
        );
        self.diag(builtins::E_NOTICE, &msg)
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
            typed: &self.typed_refs,
            scope: Some(owner),
            called: Some(owner),
            incdec: false,
        };
        let mut obj = Value::Object(h.clone());
        make_ref_path(&mut obj, &[CursorStep::Prop(PhpStr::from("__s"))], &cx)
    }

    /// The object a built-in ArrayIterator is backed by (its `__o`).
    pub(crate) fn spl_array_backing(
        &mut self,
        h: &ObjectHandle,
        owner: ClassId,
    ) -> Result<Option<ObjectHandle>, Thrown> {
        if self.class(owner).name.as_bytes() != b"ArrayIterator" {
            return Ok(None);
        }
        let cx = PropCtx {
            classes: &self.classes,
            typed: &self.typed_refs,
            scope: Some(owner),
            called: Some(owner),
            incdec: false,
        };
        let mut obj = Value::Object(h.clone());
        let o = make_ref_path(&mut obj, &[CursorStep::Prop(PhpStr::from("__o"))], &cx)?;
        let v = o.borrow().clone();
        Ok(match v {
            Value::Object(o) => Some(o),
            _ => None,
        })
    }

    /// The storage key of offset `key` of the built-in ArrayObject /
    /// ArrayIterator `h` (spl_array_get_dimension_ptr's conversion).
    pub(crate) fn spl_array_key(
        &mut self,
        h: &ObjectHandle,
        key: &Value,
    ) -> Result<rhp_values::ArrayKey, Thrown> {
        self.spl_array_key_in(h, key, SplKeyUse::Access)
    }

    /// spl_array's get_hash_key, failing with zend_illegal_container_offset's
    /// message for the fetch type.
    fn spl_array_key_in(
        &mut self,
        h: &ObjectHandle,
        key: &Value,
        how: SplKeyUse,
    ) -> Result<rhp_values::ArrayKey, Thrown> {
        let akey = self.dim_key(&deref(key), false)?;
        akey.ok_or_else(|| {
            let ty = rhp_builtins::zval_type_name(self, key);
            let on = rhp_builtins::zval_type_name(self, &Value::Object(h.clone()));
            Thrown::type_error(match how {
                SplKeyUse::Access => format!("Cannot access offset of type {ty} on {on}"),
                SplKeyUse::Isset => format!("Cannot access offset of type {ty} in isset or empty"),
                SplKeyUse::Unset => format!("Cannot unset offset of type {ty} on {on}"),
            })
        })
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
        let akey = match mode {
            // uninitialized_zval after the TypeError: the notice follows.
            DimFetch::Unset => match self.spl_array_key_in(h, key, SplKeyUse::Unset) {
                Ok(k) => k,
                Err(e) => {
                    self.indirect_overloaded_notice(h)?;
                    return Err(e);
                }
            },
            _ => self.spl_array_key(h, key)?,
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
                    self.diag(builtins::E_WARNING, &msg)?;
                }
                DimFetch::W => {}
            }
        }
        let cx = PropCtx {
            classes: &self.classes,
            typed: &self.typed_refs,
            scope: Some(owner),
            called: Some(owner),
            incdec: false,
        };
        let mut slot = storage.borrow_mut();
        make_ref_path(&mut slot, &[CursorStep::Key(akey.to_value())], &cx).map(Some)
    }

    /// Promote a CV slot to a shared reference cell, returning the cell. If the
    /// slot already holds a `Ref`, its cell is reused. An undefined variable
    /// becomes null: a PHP reference always holds a value.
    pub(crate) fn promote(&mut self, top: usize, cv: Reg) -> Rc<RefCell<Value>> {
        let rc = self.promote_slot(top, cv);
        crate::dispatch::cell_define(&rc);
        rc
    }

    /// [`Vm::promote`] keeping an undefined variable undefined: the cell is
    /// the variable's storage shared by name (an include's scope), not a PHP
    /// reference.
    pub(crate) fn promote_slot(&mut self, top: usize, cv: Reg) -> Rc<RefCell<Value>> {
        let f = &self.stack[top];
        if f.global_scope {
            if let Some(n) = f.proto.cv_var_names.get(cv as usize) {
                if !self.ref_globals.contains(n.as_bytes()) {
                    self.ref_globals.insert(n.as_bytes().to_vec());
                }
            }
        }
        let slot = &mut self.stack[top].regs[cv as usize];
        match slot {
            Value::Ref(rc) => rc.clone(),
            other => {
                let v = std::mem::replace(other, Value::Undef);
                let rc = Rc::new(RefCell::new(v));
                *other = Value::Ref(rc.clone());
                rc
            }
        }
    }

    /// php's array-offset key conversion for a read (`zend_fetch_dimension_
    /// address_inner` / `slow_index_convert`), with its diagnostics. `None`:
    /// an illegal offset type (TypeError raised by the caller).
    #[inline]
    fn dim_key(&mut self, key: &Value, quiet: bool) -> Result<Option<ArrayKey>, Thrown> {
        let _ = quiet;
        match key {
            Value::Int(n) => return Ok(Some(ArrayKey::Int(*n))),
            Value::Str(s) => return Ok(Some(ArrayKey::from_php_str(s))),
            _ => {}
        }
        self.dim_key_slow(key)
    }

    #[inline(never)]
    fn dim_key_slow(&mut self, key: &Value) -> Result<Option<ArrayKey>, Thrown> {
        match key {
            Value::Null => self.diag(
                builtins::E_DEPRECATED,
                "Using null as an array offset is deprecated, use an empty string instead",
            )?,
            Value::Float(f) => {
                for (level, msg) in builtins::zend::operators::dval_to_lval_safe_msgs(*f) {
                    self.diag(level, &msg)?;
                }
            }
            Value::Resource(r) => self.diag(
                builtins::E_WARNING,
                &format!(
                    "Resource ID#{} used as offset, casting to integer ({})",
                    r.id, r.id
                ),
            )?,
            _ => {}
        }
        Ok(ArrayKey::from_value(key))
    }

    /// The key of an array literal element (ZEND_INIT_ARRAY /
    /// ZEND_ADD_ARRAY_ELEMENT): php's offset diagnostics, and
    /// zend_illegal_container_offset's TypeError for an unusable key.
    pub(crate) fn init_array_key(&mut self, k: &Value) -> Result<ArrayKey, Thrown> {
        match k {
            Value::Int(n) => Ok(ArrayKey::Int(*n)),
            Value::Str(s) => Ok(ArrayKey::from_php_str(s)),
            _ => match self.dim_key(k, false)? {
                Some(key) => Ok(key),
                None => Err(Thrown::type_error(format!(
                    "Cannot access offset of type {} on array",
                    rhp_builtins::zval_type_name(self, k)
                ))),
            },
        }
    }

    /// `isset($c[$k])` / `empty($c[$k])` on anything but an ArrayAccess
    /// object (ZEND_ISSET_ISEMPTY_DIM_OBJ: zend_find_array_dim_slow,
    /// zend_isset_dim_slow, zend_isempty_dim_slow). The result is
    /// `isset` or, with `empty`, `empty`.
    pub(crate) fn isset_isempty_dim(
        &mut self,
        container: &Value,
        key: &Value,
        empty: bool,
    ) -> Result<bool, Thrown> {
        let key = &deref(key);
        match deref(container) {
            Value::Array(a) => {
                let k = match key {
                    Value::Int(n) => ArrayKey::Int(*n),
                    Value::Str(s) => ArrayKey::from_php_str(s),
                    // slow_index_convert: an error handler that drops the
                    // last reference destroys the array (php's GC_DELREF
                    // check); only this fetch's two copies are left then.
                    Value::Float(_) => {
                        let before = a.ref_count();
                        let k = self.dim_key(key, true)?;
                        let after = a.ref_count();
                        if after < before && after <= 2 {
                            return Ok(empty);
                        }
                        k.unwrap_or(ArrayKey::Int(0))
                    }
                    _ => match self.dim_key(key, true)? {
                        Some(k) => k,
                        None => {
                            return Err(Thrown::type_error(format!(
                                "Cannot access offset of type {} in isset or empty",
                                rhp_builtins::zval_type_name(self, key)
                            )))
                        }
                    },
                };
                let v = a.get(&k).map(deref);
                if empty {
                    match v {
                        Some(v) => Ok(!self.truthy(&v)?),
                        None => Ok(true),
                    }
                } else {
                    Ok(!matches!(v, None | Some(Value::Null)))
                }
            }
            Value::Str(s) => {
                let off = match key {
                    Value::Int(n) => Some(*n),
                    Value::Null => Some(0),
                    Value::Bool(b) => Some(*b as i64),
                    // zval_get_long_ex(is_strict): the float warning, then
                    // the deprecation when the value changed.
                    Value::Float(f) => {
                        let l = self.dval_to_lval(*f)?;
                        if l as f64 != *f {
                            let shown = rhp_values::fmt::float_serialize(*f);
                            self.diag(
                                builtins::E_DEPRECATED,
                                &[
                                    b"Implicit conversion from float ".as_slice(),
                                    &shown,
                                    b" to int loses precision",
                                ]
                                .concat(),
                            )?;
                        }
                        Some(l)
                    }
                    Value::Str(ks) => match conv::numeric_string(ks.as_bytes()) {
                        Some(conv::Num::Int(n))
                            if ks.as_bytes().first().is_some_and(|&c| c <= b'9') =>
                        {
                            Some(n)
                        }
                        _ => None,
                    },
                    _ => None,
                };
                let bytes = s.as_bytes();
                let len = bytes.len() as i64;
                let at = off
                    .map(|o| if o < 0 { o + len } else { o })
                    .filter(|&i| i >= 0 && i < len);
                Ok(match at {
                    Some(i) if empty => bytes[i as usize] == b'0',
                    Some(_) => true,
                    None => empty,
                })
            }
            // zend_std_has_dimension without ArrayAccess.
            Value::Object(h) => {
                if let Some(d) = self.handlers_of(&h).and_then(|t| t.dimension) {
                    return (d.has)(self, &h, key, empty);
                }
                let cls = h.borrow().class;
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
            _ => Ok(empty),
        }
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
                        rhp_builtins::zval_type_name(self, key)
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
                            self.diag(builtins::E_WARNING, &msg)?;
                        }
                        Ok(Value::Null)
                    }
                }
            }
            Value::Object(h) => {
                // A native read_dimension (WeakMap, ResourceBundle) answers
                // BP_VAR_IS itself.
                if let Some(d) = self.handlers_of(h).and_then(|t| t.dimension) {
                    let ty = if quiet { DimType::Is } else { DimType::R };
                    return Ok((d.read)(self, h, Some(key), ty)?.unwrap_or(Value::Null));
                }
                let cls = h.borrow().class;
                if self.class_implements(cls, b"ArrayAccess") {
                    if let (true, Some(base)) = (quiet, self.spl_array_base(h)) {
                        return self.spl_array_read_is(h, base, key.clone());
                    }
                    // zend_std_read_dimension(BP_VAR_IS): offsetExists first.
                    if quiet && !self.aa_offset_exists(h, key.clone())? {
                        return Ok(Value::Null);
                    }
                    return self.aa_offset_get(h, key.clone());
                }
                // A class with native dimension handlers (PDORow).
                if let Some(d) = self.handlers_of(h).and_then(|t| t.dimension) {
                    let t = if quiet { DimType::Is } else { DimType::R };
                    return Ok((d.read)(self, h, Some(key), t)?.unwrap_or(Value::Null));
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
                                if trailing {
                                    self.diag(
                                        builtins::E_WARNING,
                                        &[b"Illegal string offset \"".as_slice(), kb, b"\""]
                                            .concat(),
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
                            self.diag(builtins::E_WARNING, "String offset cast occurred")?;
                        }
                        conv::to_int(key)
                    }
                    _ => {
                        return Err(Thrown::type_error(format!(
                            "Cannot access offset of type {} on string",
                            rhp_builtins::zval_type_name(self, key)
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
                    self.diag(
                        builtins::E_WARNING,
                        &format!("Uninitialized string offset {offset}"),
                    )?;
                    Ok(Value::str(""))
                }
            }
            Value::Null => Ok(Value::Null).and_then(|v| {
                if mode == DimMode::Read {
                    self.diag(builtins::E_WARNING, "Trying to access array offset on null")?;
                }
                Ok(v)
            }),
            other => {
                if !quiet {
                    // zend_zval_type_name for list(), else zend_zval_value_name
                    let msg = if mode == DimMode::List {
                        format!(
                            "Cannot use {} as array",
                            rhp_builtins::zval_type_name(self, other)
                        )
                    } else {
                        format!(
                            "Trying to access array offset on {}",
                            rhp_builtins::zval_value_name(self, other)
                        )
                    };
                    self.diag(builtins::E_WARNING, &msg)?;
                }
                Ok(Value::Null)
            }
        }
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
                    .props()
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
            path_diag(builtins::E_DEPRECATED, FALSE_TO_ARRAY);
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

const FALSE_TO_ARRAY: &str = "Automatic conversion of false to array is deprecated";

const NULL_OFFSET_DEPRECATION: &str =
    "Using null as an array offset is deprecated, use an empty string instead";

fn path_diag(level: i64, msg: impl Into<Vec<u8>>) {
    PATH_DIAGS.with(|d| d.borrow_mut().push((level, msg.into())));
}

/// Queue the diagnostics php's slow_index_convert raises for array key
/// `k` (null, a float that isn't a whole int, a resource); `null_dep`:
/// whether null is deprecated here (not for a final unset).
fn key_diags(k: &Value, null_dep: bool) {
    match k {
        Value::Null if null_dep => path_diag(builtins::E_DEPRECATED, NULL_OFFSET_DEPRECATION),
        Value::Float(f) => {
            for (level, msg) in builtins::zend::operators::dval_to_lval_safe_msgs(*f) {
                path_diag(level, msg);
            }
        }
        Value::Resource(r) => path_diag(
            builtins::E_WARNING,
            format!(
                "Resource ID#{} used as offset, casting to integer ({})",
                r.id, r.id
            ),
        ),
        _ => {}
    }
}

/// The array key for a write / fetch-for-write with `k`: php's key
/// diagnostics, the TypeError for an unusable key.
fn write_key_of(k: &Value, cx: &PropCtx) -> Result<ArrayKey, Thrown> {
    match k {
        Value::Int(n) => return Ok(ArrayKey::Int(*n)),
        Value::Str(s) => return Ok(ArrayKey::from_php_str(s)),
        _ => {}
    }
    let k = deref(k);
    let key = ArrayKey::from_value(&k).ok_or_else(|| illegal_offset_on_array(&k, cx))?;
    key_diags(&k, true);
    Ok(key)
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
                            builtins::E_WARNING,
                            [b"Illegal string offset \"".as_slice(), kb, b"\""].concat(),
                        );
                    }
                    Ok(n)
                }
                _ => Err(illegal("string".into())),
            }
        }
        v @ (Value::Float(_) | Value::Null | Value::Bool(_)) => {
            path_diag(rhp_builtins::E_WARNING, "String offset cast occurred");
            Ok(conv::to_int(&v))
        }
        other => Err(illegal(cx.type_name(&other))),
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
        path_diag(
            rhp_builtins::E_WARNING,
            format!("Illegal string offset {offset}"),
        );
        set_string_offset_result(Value::Null);
        return Ok(());
    }
    let at = if offset < 0 { offset + len } else { offset } as usize;
    let bytes = match deref(value) {
        Value::Array(_) => {
            path_diag(rhp_builtins::E_WARNING, "Array to string conversion");
            b"Array".to_vec()
        }
        Value::Object(_) | Value::Closure(_) => {
            STRING_OFFSET_OBJECT.with(|f| f.set(true));
            return Ok(());
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
            builtins::E_WARNING,
            "Only the first byte will be assigned to the string offset",
        );
    }
    set_string_offset_result(Value::str(vec![c]));
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
        let mut out = s.make_mut();
        out.resize(at, b' ');
        out.push(c);
    } else {
        s.make_mut()[at] = c;
    }
    Ok(())
}

thread_local! {
    /// Leaf writes [`assign_path`] could not make while the target cell was
    /// borrowed by the path itself.
    static DEFERRED_WRITE: RefCell<Vec<(Rc<RefCell<Value>>, Value)>> =
        const { RefCell::new(Vec::new()) };
}

/// Store the writes [`assign_path`] deferred.
#[inline]
fn flush_deferred_writes() {
    if DEFERRED_WRITE.with(|d| d.borrow().is_empty()) {
        return;
    }
    let pending = DEFERRED_WRITE.with(|d| std::mem::take(&mut *d.borrow_mut()));
    for (rc, v) in pending {
        let old = std::mem::replace(&mut *rc.borrow_mut(), v);
        rhp_values::gc::possible_root(&old);
        drop(old);
    }
}

thread_local! {
    /// Values a cursor write replaced, released once the path's borrows
    /// are (each may become a possible root of a cycle).
    static REPLACED: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
}

/// Release `old` (overwritten by a path write) after the write.
fn replaced(old: Value) {
    if let Value::Object(_) | Value::Array(_) | Value::Ref(_) | Value::Closure(_) = old {
        REPLACED.with(|r| r.borrow_mut().push(old));
    }
}

/// The values [`replaced`] kept: possible roots, then dropped.
pub(crate) fn flush_replaced() {
    let old = REPLACED.with(|r| {
        let mut r = r.borrow_mut();
        if r.is_empty() {
            Vec::new()
        } else {
            std::mem::take(&mut *r)
        }
    });
    for v in &old {
        rhp_values::gc::possible_root(v);
    }
}

/// Write `value` at a cursor path, autovivifying + COW-bubbling along the way.
fn assign_path(
    slot: &mut Value,
    steps: &[CursorStep],
    value: Value,
    cx: &PropCtx,
) -> Result<Option<Box<TypedStop>>, Thrown> {
    // An undefined variable at the root is created by the write (not by a
    // property write, which fails on it).
    if let Value::Undef = slot {
        if !matches!(steps.first(), Some(CursorStep::Prop(_))) {
            *slot = Value::Null;
        }
    }
    // `lval = &$x` (a `Ref` value at the leaf) rebinds the slot itself: it
    // does not write through a reference the slot already held.
    if steps.is_empty() && matches!(value, Value::Ref(_)) {
        replaced(std::mem::replace(slot, value));
        return Ok(None);
    }
    // Descend through a reference to its underlying storage.
    if let Value::Ref(rc) = slot {
        if cx.typed.has(rc) {
            if steps.is_empty() {
                return Ok(Some(Box::new(TypedStop::Leaf(rc.clone(), value))));
            }
            let nested = matches!(steps[0], CursorStep::Key(_) | CursorStep::Append);
            if nested
                && rc.try_borrow().is_ok_and(|v| promotes_to_array(&v))
                && !cx.typed.accepts_array(cx.classes, rc)
            {
                return Ok(Some(Box::new(TypedStop::RefInit(rc.clone()))));
            }
        }
        let Ok(mut inner) = rc.try_borrow_mut() else {
            // The reference is to a container this write is inside (an array
            // holding a reference to itself): the value is stored once the
            // path is released.
            if steps.is_empty() {
                DEFERRED_WRITE.with(|d| d.borrow_mut().push((rc.clone(), value)));
                return Ok(None);
            }
            return Err(Thrown::error(
                "Error",
                "Cannot write into an array through a reference to itself",
            ));
        };
        return assign_path(&mut inner, steps, value, cx);
    }
    let Some((head, rest)) = steps.split_first() else {
        replaced(std::mem::replace(slot, value));
        return Ok(None);
    };
    if let CursorStep::Prop(name) = head {
        // Descend into an object property (handle semantics: no COW).
        let handle = match slot {
            Value::Object(h) => h.clone(),
            Value::Closure(_) => return Err(crate::props::closure_prop_error(name.as_bytes())),
            // zend_throw_non_object_error: the property itself is assigned;
            // a write further down the path modifies it.
            other => {
                let shown = cx.value_name(other);
                let op = if !rest.is_empty() || matches!(value, Value::Ref(_)) {
                    "modify"
                } else if cx.incdec {
                    "increment/decrement"
                } else {
                    "assign"
                };
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Attempt to {op} property \"{}\" on {shown}",
                        String::from_utf8_lossy(name.as_bytes())
                    ),
                ));
            }
        };
        let mut od = handle.borrow_mut();
        let key = cx.write_key(&od, name, "modify", rest)?;
        // zend_handle_fetch_obj_flags (ZEND_FETCH_DIM_WRITE): a typed
        // property that would become an array.
        if matches!(rest.first(), Some(CursorStep::Key(_) | CursorStep::Append)) {
            let at = od.props().get_index_of(&key);
            let promotes = at.is_none_or(|i| match &od.props()[i] {
                Value::Ref(r) => r.try_borrow().is_ok_and(|v| promotes_to_array(&v)),
                v => promotes_to_array(v),
            });
            if promotes
                && cx.classes[od.class as usize]
                    .props
                    .get(&key)
                    .and_then(|pe| pe.ty.as_ref())
                    .is_some_and(|t| !crate::refsrc::takes_array(t))
            {
                return Ok(Some(Box::new(TypedStop::PropInit(od.class, key))));
            }
            if let Some(i) = at {
                return assign_path(&mut od.props_mut()[i], rest, value, cx);
            }
        }
        let child = od.props_mut().get_or_insert_null(key);
        return assign_path(child, rest, value, cx);
    }
    if let Value::Str(_) = slot {
        let CursorStep::Key(k) = head else {
            return Err(Thrown::error(
                "Error",
                "[] operator not supported for strings",
            ));
        };
        if rest.is_empty() && !matches!(value, Value::Ref(_)) {
            return assign_string_offset(slot, k, &value, cx).map(|_| None);
        }
        string_offset(k, false, cx)?;
        // `$str[k] =& $v`
        if rest.is_empty() {
            return Err(string_offset_misuse(rest, true));
        }
        return Err(string_offset_misuse(rest, false));
    }
    autovivify(slot, cx)?;
    let Value::Array(arr) = slot else {
        unreachable!("autovivify ensures array")
    };
    let child = match head {
        CursorStep::Key(k) => arr.get_or_insert_mut(write_key_of(k, cx)?),
        CursorStep::Append => arr.append_mut().ok_or_else(cannot_add_element)?,
        CursorStep::Prop(_) => unreachable!("handled above"),
    };
    assign_path(child, rest, value, cx)
}

/// zend_cannot_add_element: an append past PHP_INT_MAX.
pub(crate) fn cannot_add_element() -> Thrown {
    Thrown::error(
        "Error",
        "Cannot add element to the array as the next element is already occupied",
    )
}

/// The key value for an ArrayAccess cursor step: `Key(k)` -> k, `Append` -> null
/// (`$obj[] = v` calls offsetSet(null, v)); `Prop` is not array access.
/// The offset of an ArrayAccess step for a native dimension handler:
/// `None` for `[]`.
pub(crate) fn aa_key_opt(step: &CursorStep) -> Option<Value> {
    match step {
        CursorStep::Key(k) => Some(k.clone()),
        _ => None,
    }
}

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
    if let Value::Undef = slot {
        *slot = Value::Null;
    }
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
            Value::Closure(_) => return Err(crate::props::closure_prop_error(name.as_bytes())),
            // zend_throw_non_object_error for a fetch for write
            other => {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Attempt to modify property \"{}\" on {}",
                        String::from_utf8_lossy(name.as_bytes()),
                        cx.value_name(other)
                    ),
                ))
            }
        };
        let mut od = handle.borrow_mut();
        let key = cx.write_key(&od, name, "indirectly modify", rest)?;
        if rest.is_empty() && !od.props().contains_key(&key) {
            // zend_handle_fetch_obj_flags (ZEND_FETCH_REF): an uninitialized
            // typed property that does not accept null.
            if let Some(pe) = cx.classes[od.class as usize].props.get(&key) {
                if let Some(t) = &pe.ty {
                    let null_ok = t.nullable
                        || t.kinds
                            .iter()
                            .any(|k| matches!(k, TypeKind::Mixed | TypeKind::Null));
                    if !null_ok {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Cannot access uninitialized non-nullable property {}::${} by reference",
                                String::from_utf8_lossy(
                                    cx.classes[pe.declaring_class as usize].name.as_bytes()
                                ),
                                String::from_utf8_lossy(pe.name.as_bytes())
                            ),
                        ));
                    }
                }
            }
        }
        let child = od.props_mut().get_or_insert_null(key);
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
        CursorStep::Append => arr.append_mut().ok_or_else(cannot_add_element)?,
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
                if let Some(old) = od.remove_prop(&key) {
                    replaced(old);
                }
            } else if let Some(child) = od.props_mut().get_mut(&key) {
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
                // ZEND_UNSET_DIM / zend_fetch_dimension_address(UNSET) on
                // anything else.
                Value::Bool(false) => {
                    path_diag(
                        builtins::E_DEPRECATED,
                        "Automatic conversion of false to array is deprecated",
                    );
                    return Ok(());
                }
                Value::Bool(true) | Value::Int(_) | Value::Float(_) | Value::Resource(_) => {
                    return Err(Thrown::error(
                        "Error",
                        "Cannot unset offset in a non-array variable",
                    ))
                }
                // An object without ArrayAccess (zend_bad_array_access).
                o @ (Value::Object(_) | Value::Closure(_)) => {
                    let name = match o {
                        Value::Object(h) => cx.class_name(&Value::Object(h.clone())),
                        _ => None,
                    }
                    .unwrap_or_else(|| b"Closure".to_vec());
                    return Err(Thrown::error(
                        "Error",
                        format!(
                            "Cannot use object of type {} as array",
                            String::from_utf8_lossy(&name)
                        ),
                    ));
                }
                _ => return Ok(()),
            };
            let Some(key) = ArrayKey::from_value(k) else {
                if !rest.is_empty() {
                    return Err(illegal_offset_on_array(k, cx));
                }
                return Err(Thrown::type_error(format!(
                    "Cannot unset offset of type {} on array",
                    cx.type_name(k)
                )));
            };
            key_diags(&deref(k), !rest.is_empty());
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

/// For by-reference foreach: promote the first array element at position
/// `pos` or later to a shared cell and return `(its position, key, cell)`.
/// The element in the array becomes a `Value::Ref` to that cell, so a later
/// write through the cell is visible in the array. Returns `None` if `slot`
/// is not an array or no element is left.
pub(crate) fn foreach_promote(
    slot: &mut Value,
    pos: usize,
) -> Option<(usize, ArrayKey, Rc<RefCell<Value>>)> {
    let Value::Array(arr) = slot else { return None };
    let (at, _, _) = arr.fetch_from(pos)?;
    let (key, v) = arr.make_mut().at_mut(at)?;
    Some((at, key, promote(v)))
}

/// The cell behind element `v`, making it a reference first.
fn promote(v: &mut Value) -> Rc<RefCell<Value>> {
    match v {
        Value::Ref(rc) => rc.clone(),
        other => {
            let inner = std::mem::replace(other, Value::Null);
            let rc = Rc::new(RefCell::new(inner));
            *other = Value::Ref(rc.clone());
            rc
        }
    }
}

/// FE_FETCH_RW over the array in `slot`: the element at HashTable iterator
/// `id`'s position (the internal pointer when the array does not carry the
/// iterator: the variable was given another array), promoted to a shared
/// cell; the iterator moves past it. `None` at the end.
pub(crate) fn foreach_next_ref(
    slot: &mut Value,
    id: u32,
) -> Option<(ArrayKey, Rc<RefCell<Value>>)> {
    let Value::Array(arr) = slot else { return None };
    let (key, v) = arr.make_mut().iter_step(id)?;
    Some((key, promote(v)))
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
    Thrown::type_error(format!(
        "Cannot access offset of type {} on array",
        cx.type_name(k)
    ))
}

/// What an ArrayObject offset is used for (the wording of its TypeError).
#[derive(Clone, Copy)]
enum SplKeyUse {
    Access,
    Isset,
    Unset,
}
