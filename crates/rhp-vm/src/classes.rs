//! OOP class table: the linked `ClassEntry`s + `Vm` methods to install
//! (flatten + link) `ClassProto`s, resolve `ClassRef`s, allocate objects, and
//! look up methods/static props. See `docs/design/oop.md`.
//!
//! Inheritance is flattened at link time (parent merged first, own members
//! override) for O(1) dispatch.

use std::cell::RefCell;
use std::rc::Rc;

use indexmap::IndexMap;
use rustc_hash::{FxBuildHasher, FxHashMap};

use rhp_bytecode::{
    ClassProto, ClassRef, ConstInitProto, FunctionProto, PropDefaultProto, Reg, TypeHint,
};
use rhp_values::{
    ClassFlags, ClassId, ClassKind, ObjectData, ObjectHandle, PhpStr, Thrown, Value, Visibility,
};

use rhp_builtins::zend::object_handlers::{CloneObj, ObjectHandlers};

use crate::calls::Args;
use crate::{DeclSlot, Frame, UnitFn, UnitId, Vm};

/// A linked class: all tables flattened from the inheritance chain.
pub struct ClassEntry {
    pub name: PhpStr,
    pub kind: ClassKind,
    pub flags: ClassFlags,
    pub parent: Option<ClassId>,
    /// self + all transitive parents + interfaces (the instanceof set).
    pub ancestors: Vec<ClassId>,
    /// Implemented interfaces in php's `ce->interfaces` order (parent's first,
    /// then own, then interfaces inherited through own interfaces) — what
    /// `class_implements` / `getInterfaceNames` report.
    pub interfaces: Vec<ClassId>,
    /// Traits used directly by this class (`class_uses`), in `use` order.
    pub traits: Vec<ClassId>,
    /// Flattened vtable, keyed by lowercase method name, in php's member
    /// order (see [`php_member_order`]).
    pub methods: IndexMap<Vec<u8>, ResolvedMethod, FxBuildHasher>,
    /// Flattened instance-property slots in php's object layout order, keyed
    /// by *storage key*: the plain name for public/protected properties, the
    /// php-mangled `"\0Class\0name"` for private ones — so a parent's private
    /// `$x` and a child's `$x` are two distinct slots (see [`prop_storage_key`]).
    pub props: IndexMap<PhpStr, PropEntry>,
    /// php's `properties_info`: unmangled name -> storage key of the most-derived
    /// declaration of that name (which may be an inherited parent private).
    pub prop_visible: FxHashMap<PhpStr, PhpStr>,
    /// Private instance properties declared by THIS class (incl. trait-imported):
    /// unmangled name -> storage key. Consulted first when this class is the
    /// calling scope, so its methods see their own private slot.
    pub own_private: FxHashMap<PhpStr, PhpStr>,
    /// Flattened class constants (case-sensitive), in php's member order.
    pub constants: IndexMap<Vec<u8>, ClassConst>,
    /// Flattened static-property slots (shared cells; inherited ones alias).
    pub static_props: IndexMap<Vec<u8>, StaticProp>,
    /// Enum case singletons in declaration order (empty for non-enums).
    pub enum_cases: Vec<ObjectHandle>,
    /// Backing type of a backed enum (`None` for pure enums / non-enums).
    pub enum_backing: Option<rhp_bytecode::EnumBacking>,
    /// Whether the deferred initializers (constants, property defaults,
    /// static properties) have all been evaluated — php's
    /// `ZEND_ACC_CONSTANTS_UPDATED`. See [`Vm::update_class_constants`].
    pub updated: bool,
    /// The declaration this entry was linked from (Reflection metadata:
    /// attributes, doc comments, lines, declared member order).
    pub proto: Rc<ClassProto>,
    /// The unit `proto` was installed with.
    pub unit: UnitId,
    /// Declaring file and line (empty file: a built-in class).
    pub decl_file: PhpStr,
    pub decl_line: u32,
    /// The object handlers: the built-in class's own registration
    /// ([`rhp_builtins::zend::object_handlers::registered`]), else the parent's (php's
    /// inherited `create_object`). `None` for plain classes, so the engine
    /// pays a field test only.
    pub handlers: Option<&'static ObjectHandlers>,
    /// php's `ce->create_object` is set (see
    /// [`rhp_builtins::zend::object_handlers::ClassHandlers::create_object`]).
    pub create_object: bool,
}

#[derive(Clone)]
pub struct ResolvedMethod {
    pub proto: Rc<FunctionProto>,
    /// The unit `proto` was installed with (a trait method keeps the
    /// trait's).
    pub unit: UnitId,
    pub defining_class: ClassId,
    pub visibility: Visibility,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_final: bool,
    /// php's `common.prototype` for a constructor: the class whose abstract
    /// or interface constructor this one implements (see `inherit`).
    pub prototype: Option<ClassId>,
}

#[derive(Clone)]
pub struct PropEntry {
    /// Unmangled property name (no `$`).
    pub name: PhpStr,
    pub default: Value,
    /// A not-yet-evaluated non-constant default (evaluated in the declaring
    /// class's scope by [`Vm::update_class_constants`]).
    pub init: Option<UnitFn>,
    pub visibility: Visibility,
    /// Asymmetric write visibility (`private(set)` / `protected(set)`).
    pub set_visibility: Option<Visibility>,
    pub declaring_class: ClassId,
    /// php's `property_info->prototype->ce`: the class of the root declaration
    /// of a redeclared public/protected property (protected access is checked
    /// against it, so sibling classes may read a redeclared property).
    pub prototype: ClassId,
    /// Declared type (`None` = untyped).
    pub ty: Option<Rc<TypeHint>>,
    /// `readonly` (or a member of a readonly class).
    pub readonly: bool,
    /// Declared without a default: a typed one starts uninitialized (its
    /// slot is absent from a new object).
    pub no_default: bool,
}

impl PropEntry {
    /// A typed property without a default starts uninitialized.
    pub fn starts_uninit(&self) -> bool {
        self.ty.is_some() && self.no_default && self.init.is_none()
    }
}

#[derive(Clone)]
pub struct StaticProp {
    pub cell: Rc<RefCell<Value>>,
    /// The declared default (php's `default_static_members_table`), shared
    /// like `cell` by inheriting classes; what Reflection reports.
    pub default: Rc<RefCell<Value>>,
    /// A not-yet-evaluated non-constant initializer for `cell`.
    pub init: Option<UnitFn>,
    pub visibility: Visibility,
    pub declaring_class: ClassId,
    /// The root declaration's class of a redeclared public/protected static
    /// (php's `property_info->prototype->ce`), for the protected check.
    pub prototype: ClassId,
    /// Declared type (`None` = untyped).
    pub ty: Option<Rc<TypeHint>>,
}

#[derive(Clone)]
pub struct ClassConst {
    pub value: Value,
    pub visibility: Visibility,
    pub declaring_class: ClassId,
    /// A not-yet-evaluated constant expression (php's `IS_CONSTANT_AST`):
    /// evaluated on first access, in the declaring class's scope.
    pub init: Option<ConstInit>,
    /// Set while the initializer is being evaluated from inside another
    /// constant expression (php's `IS_CONSTANT_VISITED`), to detect cycles.
    pub visited: bool,
    /// Declared type of a typed constant (`None` = untyped).
    pub ty: Option<Rc<TypeHint>>,
}

/// A deferred class-constant initializer.
#[derive(Clone)]
pub enum ConstInit {
    /// `const X = <expr>;` — the thunk yields the constant's value.
    Expr(UnitFn),
    /// `case C = <expr>;` of a backed enum — the constant's value is already
    /// the case object; the thunk yields its backing `value` property.
    EnumCase(UnitFn),
}

/// A linked class's flags: the declaration's, the ones an internal class's
/// stub gives it (gen_stub's `ce_flags`), the inherited ones
/// (zend_do_inheritance), and `not_cloneable` from the handlers.
fn link_flags(
    proto: &ClassProto,
    parent: Option<ClassFlags>,
    handlers: Option<&'static ObjectHandlers>,
    internal: bool,
) -> ClassFlags {
    use rhp_bytecode::signatures as sigs;
    let mut f = proto.flags;
    let stub = if internal {
        sigs::class(proto.name.as_bytes()).map_or(0, |c| c.flags)
    } else {
        0
    };
    f.allow_dynamic |= stub & sigs::CLASS_ALLOW_DYNAMIC_PROPERTIES != 0;
    f.no_dynamic |= stub & sigs::CLASS_NO_DYNAMIC_PROPERTIES != 0;
    f.not_serializable |= stub & sigs::CLASS_NOT_SERIALIZABLE != 0;
    if let Some(p) = parent {
        f.allow_dynamic |= p.allow_dynamic;
        f.not_serializable |= p.not_serializable;
    }
    f.not_cloneable = proto.kind == ClassKind::Enum
        || handlers.is_some_and(|h| matches!(h.clone_obj, CloneObj::None));
    f
}

/// The key a declared instance property is stored under in
/// `ObjectData::properties`: the plain name for public/protected, php's mangled
/// `"\0Class\0name"` for private (so shadowed privates are distinct slots).
pub(crate) fn prop_storage_key(class_name: &PhpStr, name: &PhpStr, vis: Visibility) -> PhpStr {
    if vis == Visibility::Private {
        mangle_private(class_name.as_bytes(), name.as_bytes())
    } else {
        name.clone()
    }
}

/// `"\0Class\0name"`.
pub(crate) fn mangle_private(class_name: &[u8], name: &[u8]) -> PhpStr {
    let mut k = Vec::with_capacity(class_name.len() + name.len() + 2);
    k.push(0);
    k.extend_from_slice(class_name);
    k.push(0);
    k.extend_from_slice(name);
    PhpStr::from_bytes(k)
}

/// Outcome of resolving `$obj->name` against the object's class from a scope.
#[derive(Debug)]
pub(crate) enum PropRes {
    /// A declared property slot, by storage key.
    Slot(PhpStr),
    /// Not a declared property visible here: a dynamic property (plain name).
    Dynamic,
    /// Declared but inaccessible from the scope (or a name starting with NUL).
    Denied(Deny),
}

/// Why a property access was refused.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Deny {
    /// A private / protected declaration the scope cannot see.
    Vis(Visibility),
    /// An undeclared name starting with `"\0"` (php's mangled-name guard).
    BadName,
}

/// php's `zend_get_property_offset`: resolve property `name` on an instance of
/// `cls` as seen from `scope`. The scope's own private property wins (so a
/// parent method sees the parent's private slot even when a child redeclares
/// the name); otherwise the most-derived declaration applies, where a parent's
/// private is invisible (treated as dynamic) and an own private / protected is
/// an access error. Inlined into the property paths, so that its map lookups
/// stay inlined whichever codegen unit this module lands in.
#[inline]
pub(crate) fn resolve_prop(
    classes: &[ClassEntry],
    cls: ClassId,
    scope: Option<ClassId>,
    name: &PhpStr,
) -> PropRes {
    let entry = &classes[cls as usize];
    if let Some(s) = scope {
        if let Some(k) = classes[s as usize].own_private.get(name) {
            if entry.props.contains_key(k) {
                return PropRes::Slot(k.clone());
            }
        }
    }
    let Some(key) = entry.prop_visible.get(name) else {
        return dynamic_or_bad(name);
    };
    let pe = &entry.props[key];
    match pe.visibility {
        Visibility::Public => PropRes::Slot(key.clone()),
        Visibility::Protected => {
            if protected_ok(classes, scope, pe.prototype) {
                PropRes::Slot(key.clone())
            } else {
                PropRes::Denied(Deny::Vis(Visibility::Protected))
            }
        }
        // (scope == declaring class was handled by `own_private` above.)
        Visibility::Private if pe.declaring_class != cls => dynamic_or_bad(name),
        Visibility::Private => PropRes::Denied(Deny::Vis(Visibility::Private)),
    }
}

/// An undeclared (dynamic) property name; one starting with NUL is refused
/// (`Cannot access property starting with "\0"`), so mangled private keys
/// can never be reached by name.
fn dynamic_or_bad(name: &PhpStr) -> PropRes {
    if name.as_bytes().first() == Some(&0) {
        PropRes::Denied(Deny::BadName)
    } else {
        PropRes::Dynamic
    }
}

/// php's `zend_asymmetric_property_has_set_access`: may `scope` write the
/// declared slot `key` of an instance of `cls`? (Read access is assumed.)
pub(crate) fn set_access_ok(
    classes: &[ClassEntry],
    cls: ClassId,
    scope: Option<ClassId>,
    key: &PhpStr,
) -> Result<(), (Visibility, ClassId, PhpStr)> {
    let Some(pe) = classes[cls as usize].props.get(key) else {
        return Ok(());
    };
    let Some(sv) = pe.set_visibility else {
        return Ok(());
    };
    if scope == Some(pe.declaring_class)
        || (sv == Visibility::Protected && protected_ok(classes, scope, pe.prototype))
    {
        return Ok(());
    }
    // php prints `prop_info->name` with %s: for a non-public property that is
    // the mangled `"\0C\0name"` / `"\0*\0name"`, which renders as empty.
    let shown = if pe.visibility == Visibility::Public {
        pe.name.clone()
    } else {
        PhpStr::from("")
    };
    Err((sv, pe.declaring_class, shown))
}

/// `Cannot {op} private(set) property A::$x from {global scope|scope C}`.
pub(crate) fn set_access_err(
    classes: &[ClassEntry],
    called: Option<ClassId>,
    op: &str,
    (sv, decl, name): (Visibility, ClassId, PhpStr),
) -> Thrown {
    // php names a protected(set) readonly property "protected(set) readonly".
    let readonly = classes[decl as usize]
        .props
        .values()
        .any(|pe| pe.declaring_class == decl && pe.name == name && pe.readonly);
    let v = match sv {
        Visibility::Private => "private(set)",
        _ if readonly => "protected(set) readonly",
        _ => "protected(set)",
    };
    let from = match called {
        Some(c) => format!(
            "scope {}",
            String::from_utf8_lossy(classes[c as usize].name.as_bytes())
        ),
        None => "global scope".to_string(),
    };
    Thrown::error(
        "Error",
        format!(
            "Cannot {op} {v} property {}::${} from {from}",
            String::from_utf8_lossy(classes[decl as usize].name.as_bytes()),
            String::from_utf8_lossy(name.as_bytes()),
        ),
    )
}

fn protected_ok(classes: &[ClassEntry], scope: Option<ClassId>, declaring: ClassId) -> bool {
    match scope {
        Some(c) => {
            classes[c as usize].ancestors.contains(&declaring)
                || classes[declaring as usize].ancestors.contains(&c)
        }
        None => false,
    }
}

/// `Cannot access {private|protected} property C::$name` (C = object's class).
pub(crate) fn prop_access_err(
    classes: &[ClassEntry],
    cls: ClassId,
    name: &[u8],
    deny: Deny,
) -> Thrown {
    let v = match deny {
        Deny::BadName => {
            return Thrown::error("Error", "Cannot access property starting with \"\\0\"")
        }
        Deny::Vis(Visibility::Private) => "private",
        Deny::Vis(Visibility::Protected) => "protected",
        Deny::Vis(Visibility::Public) => "public",
    };
    Thrown::error(
        "Error",
        format!(
            "Cannot access {v} property {}::${}",
            String::from_utf8_lossy(classes[cls as usize].name.as_bytes()),
            String::from_utf8_lossy(name),
        ),
    )
}

/// Resolve `name` on `cls` to the storage key to use, or the access error.
pub(crate) fn prop_key(
    classes: &[ClassEntry],
    cls: ClassId,
    scope: Option<ClassId>,
    name: &PhpStr,
) -> Result<PhpStr, Thrown> {
    match resolve_prop(classes, cls, scope, name) {
        PropRes::Slot(k) => Ok(k),
        PropRes::Dynamic => Ok(name.clone()),
        PropRes::Denied(vis) => Err(prop_access_err(classes, cls, name.as_bytes(), vis)),
    }
}

/// The value slot for storage key `key` on `od`, created (as null) when absent.
/// A declared property that was `unset()` comes back at its declared position
/// (php keeps the slot, it was only UNDEF), ahead of any dynamic properties.
pub(crate) fn prop_slot_mut<'o>(
    classes: &[ClassEntry],
    od: &'o mut ObjectData,
    key: PhpStr,
) -> &'o mut Value {
    if let Some(i) = od.properties.get_index_of(&key) {
        return &mut od.properties[i];
    }
    let entry = &classes[od.class as usize];
    let pos = match entry.props.get_index_of(&key) {
        Some(slot) => od
            .properties
            .keys()
            .take_while(|k| entry.props.get_index_of(*k).is_some_and(|j| j < slot))
            .count(),
        None => od.properties.len(),
    };
    od.properties.shift_insert(pos, key, Value::Null);
    &mut od.properties[pos]
}

/// php's `zend_check_property_access`: is the object slot stored under `key`
/// the one `$obj->name` reaches from `scope`? Returns its unmangled name if so
/// (drives `get_object_vars`, `foreach` over an object, and friends).
pub(crate) fn slot_accessible(
    classes: &[ClassEntry],
    cls: ClassId,
    scope: Option<ClassId>,
    key: &PhpStr,
) -> Option<PhpStr> {
    let entry = &classes[cls as usize];
    let name = match entry.props.get(key) {
        Some(pe) => pe.name.clone(),
        None => key.clone(), // dynamic property
    };
    let ok = match resolve_prop(classes, cls, scope, &name) {
        PropRes::Slot(k) => &k == key,
        PropRes::Dynamic => *key == name && !entry.props.contains_key(key),
        PropRes::Denied(_) => false,
    };
    ok.then_some(name)
}

/// php's visibility-mangled form of an object's storage key (what `(array)`,
/// `serialize` and the debug dumpers see): private keys are already mangled,
/// a declared protected property becomes `"\0*\0name"`, everything else
/// (public / dynamic) stays plain.
pub(crate) fn mangled_prop_key(entry: &ClassEntry, key: &PhpStr) -> PhpStr {
    match entry.props.get(key) {
        Some(pe) if pe.visibility == Visibility::Protected => {
            let mut k = Vec::with_capacity(key.len() + 3);
            k.extend_from_slice(b"\0*\0");
            k.extend_from_slice(key.as_bytes());
            PhpStr::from_bytes(k)
        }
        _ => key.clone(),
    }
}

/// Where a program's class declarations go in the declaration table.
pub(crate) struct DeclRanges {
    pub(crate) top: std::ops::Range<usize>,
    pub(crate) anon: std::ops::Range<usize>,
}

/// A program's user class declarations: (top-level, nested, anonymous).
/// The compiler appends the anonymous ones after the top-level ones.
pub(crate) fn decl_layout(prog: &rhp_bytecode::Program) -> (usize, usize, usize) {
    let user = &prog.classes[prog.num_prelude_classes..];
    let anon = user
        .iter()
        .rev()
        .take_while(|c| is_anon_name(c.name.as_bytes()))
        .count();
    (user.len() - anon, prog.cond_classes.len(), anon)
}

/// A generated anonymous-class name (`Prefix@anonymous\0file:line$n`).
pub(crate) fn is_anon_name(name: &[u8]) -> bool {
    name.windows(10).any(|w| w == b"@anonymous")
}

fn kind_word(k: ClassKind) -> &'static str {
    match k {
        ClassKind::Interface => "interface",
        ClassKind::Trait => "trait",
        ClassKind::Enum => "enum",
        _ => "class",
    }
}

fn lc(bytes: &[u8]) -> Vec<u8> {
    bytes.to_ascii_lowercase()
}

/// `f` of `name` lowercased, without allocating when `name` is lowercase
/// already or short (the run-time lookups by name).
pub(crate) fn with_lc<R>(name: &[u8], f: impl FnOnce(&[u8]) -> R) -> R {
    if !name.iter().any(u8::is_ascii_uppercase) {
        return f(name);
    }
    let mut buf = [0u8; 64];
    match buf.get_mut(..name.len()) {
        Some(b) => {
            b.copy_from_slice(name);
            b.make_ascii_lowercase();
            f(b)
        }
        None => f(&name.to_ascii_lowercase()),
    }
}

fn unit_fn(proto: &Rc<FunctionProto>, unit: UnitId) -> UnitFn {
    UnitFn {
        proto: proto.clone(),
        unit,
    }
}

/// Reorder a resolved member table into php's order: the class's own members
/// in declaration order, then trait members, then everything inherited (the
/// parent's table, itself already in this order, followed by interface
/// members). Only the order changes — which member won is decided by the
/// caller. This is what reflection and `get_class_*` enumerate.
fn php_member_order<V, S: std::hash::BuildHasher + Default>(
    mut map: IndexMap<Vec<u8>, V, S>,
    first: impl IntoIterator<Item = Vec<u8>>,
) -> IndexMap<Vec<u8>, V, S> {
    let mut out = IndexMap::with_capacity_and_hasher(map.len(), S::default());
    for k in first {
        if let Some(v) = map.shift_remove(&k) {
            out.insert(k, v);
        }
    }
    out.extend(map);
    out
}

thread_local! {
    /// The prelude's classes as linked on this thread (php links its
    /// internal classes once, at startup): requests start from a copy
    /// ([`Vm::install_prelude`]). Taken at the thread's second link, so a
    /// thread that runs one request (the CLI) doesn't pay for it.
    static LINKED_PRELUDE: RefCell<Option<Rc<LinkedPrelude>>> = const { RefCell::new(None) };
    /// Whether this thread has linked the prelude before.
    static PRELUDE_LINKED_ONCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A linked prelude, as no request has used it yet.
struct LinkedPrelude {
    /// The declarations it was linked from. The compiler caches the prelude
    /// per thread, so every program compiled on the thread shares them.
    protos: Vec<Rc<ClassProto>>,
    classes: Vec<ClassEntry>,
    class_ids: FxHashMap<Vec<u8>, ClassId>,
    class_order: Vec<usize>,
    /// The capacities linking left `classes` and `class_order` with: the
    /// copies keep them, so the request's memory accounting is the same.
    capacity: (usize, usize),
}

/// A request's own copy of linked classes: the tables are shared values
/// (their strings and arrays are copy-on-write, their protos immutable),
/// and each static property gets a fresh cell, aliased between classes as
/// in `classes`. Enum case objects are not copied: the caller makes sure
/// there are none.
fn copy_classes(classes: &[ClassEntry], capacity: usize) -> Vec<ClassEntry> {
    let mut cells: FxHashMap<usize, Rc<RefCell<Value>>> = FxHashMap::default();
    let mut fresh = |c: &Rc<RefCell<Value>>| {
        cells
            .entry(Rc::as_ptr(c) as usize)
            .or_insert_with(|| Rc::new(RefCell::new(c.borrow().clone())))
            .clone()
    };
    let mut out = Vec::with_capacity(capacity.max(classes.len()));
    for e in classes {
        debug_assert!(e.enum_cases.is_empty());
        let mut static_props = IndexMap::with_capacity(e.static_props.len());
        for (k, sp) in &e.static_props {
            let sp = StaticProp {
                cell: fresh(&sp.cell),
                default: fresh(&sp.default),
                ..sp.clone()
            };
            static_props.insert(k.clone(), sp);
        }
        out.push(ClassEntry {
            name: e.name.clone(),
            kind: e.kind,
            flags: e.flags,
            parent: e.parent,
            ancestors: e.ancestors.clone(),
            interfaces: e.interfaces.clone(),
            traits: e.traits.clone(),
            methods: e.methods.clone(),
            props: e.props.clone(),
            prop_visible: e.prop_visible.clone(),
            own_private: e.own_private.clone(),
            constants: e.constants.clone(),
            static_props,
            enum_cases: Vec::new(),
            enum_backing: e.enum_backing,
            updated: e.updated,
            proto: e.proto.clone(),
            unit: e.unit,
            decl_file: e.decl_file.clone(),
            decl_line: e.decl_line,
            handlers: e.handlers,
            create_object: e.create_object,
        });
    }
    out
}

impl Vm {
    /// Link the prelude's classes (the main script's unit 0) into the empty
    /// class table, or copy them from this thread's earlier link when the
    /// program carries the same prelude. A prelude with enum cases (it has
    /// none: the compiler takes only its classes and interfaces) is linked
    /// every time, since each request needs its own case objects.
    pub(crate) fn install_prelude(&mut self, protos: &[Rc<ClassProto>]) -> Result<(), Thrown> {
        debug_assert!(self.classes.is_empty() && self.class_ids.is_empty());
        let linked = LINKED_PRELUDE.with(|c| c.borrow().clone()).filter(|p| {
            p.protos.len() == protos.len()
                && p.protos.iter().zip(protos).all(|(a, b)| Rc::ptr_eq(a, b))
        });
        if let Some(p) = linked {
            self.classes = copy_classes(&p.classes, p.capacity.0);
            self.class_ids = p.class_ids.clone();
            self.class_order = Vec::with_capacity(p.capacity.1);
            self.class_order.extend_from_slice(&p.class_order);
            return Ok(());
        }
        self.install_classes(protos, 0)?;
        if PRELUDE_LINKED_ONCE.with(|l| l.replace(true))
            && self.classes.iter().all(|e| e.enum_cases.is_empty())
        {
            let p = LinkedPrelude {
                protos: protos.to_vec(),
                classes: copy_classes(&self.classes, 0),
                class_ids: self.class_ids.clone(),
                class_order: self.class_order.clone(),
                capacity: (self.classes.capacity(), self.class_order.capacity()),
            };
            LINKED_PRELUDE.with(|c| *c.borrow_mut() = Some(Rc::new(p)));
        }
        Ok(())
    }

    /// Link every `ClassProto` of `unit` into the class table, in dependency
    /// order so that `extends`/`implements`/`use` (which may be forward
    /// references) resolve.
    pub(crate) fn install_classes(
        &mut self,
        protos: &[Rc<ClassProto>],
        unit: UnitId,
    ) -> Result<(), Thrown> {
        let mut pending: Vec<&Rc<ClassProto>> = protos.iter().collect();
        loop {
            if pending.is_empty() {
                // All classes linked. Deferred initializers (non-constant
                // class consts / static props / property defaults / enum case
                // values) are evaluated lazily, as php does: a constant on
                // first access, the rest on first use of the class (see
                // `update_class_constants`).
                return Ok(());
            }
            let mut progressed = false;
            let mut i = 0;
            while i < pending.len() {
                if self.deps_ready(pending[i]) {
                    let proto = pending.remove(i);
                    self.link_class(proto, unit, false)?;
                    progressed = true;
                } else {
                    i += 1;
                }
            }
            if !progressed {
                // A remaining class names an unlinked dependency. Before failing,
                // try to autoload it (php autoloads a missing parent/interface/
                // trait while declaring a class), then retry the link loop.
                let missing = self.first_missing_dep(pending[0]);
                if let Some(dep) = &missing {
                    if self.try_autoload(dep)? {
                        continue;
                    }
                }
                let p = pending[0];
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Class \"{}\" not found",
                        String::from_utf8_lossy(
                            &missing.unwrap_or_else(|| p.name.as_bytes().to_vec())
                        )
                    ),
                ));
            }
        }
    }

    /// Append the user class declarations of `unit`'s program to the flat
    /// declaration table, at the unit's base: the top-level ones, the
    /// nested ones, then the anonymous ones (`DeclareClass` numbers the
    /// first two in that order; anonymous classes are found by name).
    /// Returns the table ranges of the top-level and the anonymous ones.
    pub(crate) fn register_decl_classes(
        &mut self,
        prog: &rhp_bytecode::Program,
        unit: UnitId,
    ) -> DeclRanges {
        let start = self.decl_classes.len();
        debug_assert_eq!(start, self.units[unit as usize].classes as usize);
        let (top, nested, anon) = decl_layout(prog);
        let user = &prog.classes[prog.num_prelude_classes..];
        let ordered = user[..top]
            .iter()
            .chain(prog.cond_classes.iter())
            .chain(user[top..].iter());
        for c in ordered {
            self.decl_classes.push(DeclSlot {
                proto: c.clone(),
                unit,
                done: false,
            });
        }
        let anon_start = start + top + nested;
        DeclRanges {
            top: start..start + top,
            anon: anon_start..anon_start + anon,
        }
    }

    /// Hoist (early-bind) the top-level classes in `range`, in declaration
    /// order, as php's compiler does (zend_compile_class_decl): only a class
    /// with no interfaces (an implicit `Stringable` from `__toString`
    /// counts, as do an enum's and an interface's `extends` list) and no
    /// traits, whose parent — if any — is already declared. The others stay
    /// undeclared until their `DeclareClass` runs.
    pub(crate) fn hoist_decl_classes(&mut self, ranges: DeclRanges) -> Result<(), Thrown> {
        for i in ranges.top.clone() {
            if self.decl_classes[i].done {
                continue;
            }
            let proto = self.decl_classes[i].proto.clone();
            if is_anon_name(proto.name.as_bytes()) {
                continue;
            }
            let early = proto.interfaces.is_empty()
                && proto.traits.is_empty()
                && proto.kind != ClassKind::Enum
                && !(proto.kind == ClassKind::Interface && proto.parent.is_some())
                && !proto
                    .methods
                    .iter()
                    .any(|m| m.name.as_bytes().eq_ignore_ascii_case(b"__tostring"))
                && proto
                    .parent
                    .as_ref()
                    .is_none_or(|p| self.class_ids.contains_key(&lc(p.as_bytes())));
            if !early || self.class_ids.contains_key(&lc(proto.name.as_bytes())) {
                continue;
            }
            if self.link_slot_ex(i, &proto, proto.parent.is_some())? {
                self.decl_classes[i].done = true;
            }
        }
        // Anonymous classes (registered with the file's classes under their
        // generated names) without a parent, interfaces or traits are linked
        // up front; the others when `new class` runs (php links those at
        // run time).
        loop {
            let mut progressed = false;
            for i in ranges.anon.clone() {
                let proto = &self.decl_classes[i].proto;
                if self.decl_classes[i].done
                    || !is_anon_name(proto.name.as_bytes())
                    || proto.parent.is_some()
                    || !proto.interfaces.is_empty()
                    || !proto.traits.is_empty()
                    || self.class_ids.contains_key(&lc(proto.name.as_bytes()))
                    || !self.deps_ready(proto)
                {
                    continue;
                }
                let proto = proto.clone();
                self.link_slot(i, &proto)?;
                self.decl_classes[i].done = true;
                progressed = true;
            }
            if !progressed {
                break;
            }
        }
        Ok(())
    }

    /// Run-time class declaration (`DeclareClass`): link the class now,
    /// autoloading missing dependencies. A no-op if it was hoisted; running
    /// an executed declaration again (a nested class in a function called
    /// twice, a loop) is php's redeclaration error.
    pub(crate) fn declare_class_at(&mut self, slot: usize) -> Result<(), Thrown> {
        let Some(decl) = self.decl_classes.get(slot) else {
            return Ok(());
        };
        if decl.done && !self.decl_runtime.contains(&slot) {
            return Ok(());
        }
        let proto = decl.proto.clone();
        if let Some(&old) = self.class_ids.get(&lc(proto.name.as_bytes())) {
            // zend_class_redeclaration_error: a fatal compile error.
            let old = &self.classes[old as usize];
            let name = String::from_utf8_lossy(old.name.as_bytes());
            let msg = if old.decl_file.is_empty() {
                format!("Cannot redeclare {} {name}", kind_word(old.kind))
            } else {
                format!(
                    "Cannot redeclare {} {name} (previously declared in {}:{})",
                    kind_word(old.kind),
                    String::from_utf8_lossy(old.decl_file.as_bytes()),
                    old.decl_line
                )
            };
            return Err(self.fatal_compile_error(msg.as_bytes()));
        }
        let deps = proto
            .parent
            .iter()
            .map(|n| ("Class", n))
            .chain(proto.interfaces.iter().map(|n| ("Interface", n)))
            .chain(proto.traits.iter().map(|n| ("Trait", n)));
        // php registers the class (unlinked) before loading its dependencies,
        // so a lookup of it from an autoloader finds it and does not recurse
        // into autoloading it again (it just is not usable yet).
        let self_key = lc(proto.name.as_bytes());
        let marked = self.in_autoload.insert(self_key.clone());
        self.push_linking(&proto);
        let mut failed = None;
        for (what, n) in deps.collect::<Vec<_>>() {
            let loaded = self.class_ids.contains_key(&lc(n.as_bytes()))
                || match self.try_autoload(n.as_bytes()) {
                    Ok(b) => b,
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                };
            if !loaded {
                failed = Some(Thrown::error(
                    "Error",
                    format!(
                        "{what} \"{}\" not found",
                        String::from_utf8_lossy(n.as_bytes())
                    ),
                ));
                break;
            }
        }
        let used = self.pop_linking();
        if marked {
            self.in_autoload.remove(&self_key);
        }
        if let Some(e) = failed {
            // check_unrecoverable_load_failure: a variance check already
            // relied on this class, so it cannot just go away.
            if used {
                let prefix = format!(
                    "During inheritance of {} with variance dependencies",
                    String::from_utf8_lossy(proto.name.as_bytes())
                );
                return Err(self.uncaught_fatal(&prefix, e));
            }
            return Err(e);
        }
        self.link_slot(slot, &proto)?;
        self.decl_classes[slot].done = true;
        self.decl_runtime.insert(slot);
        Ok(())
    }

    /// Link the class compiled into `decl_classes` slot `slot`, recording
    /// that slot as its class-table position.
    pub(crate) fn link_slot(&mut self, slot: usize, proto: &Rc<ClassProto>) -> Result<(), Thrown> {
        self.link_slot_ex(slot, proto, false).map(|_| ())
    }

    /// [`Self::link_slot`]; `early`: see [`Self::link_class`].
    fn link_slot_ex(
        &mut self,
        slot: usize,
        proto: &Rc<ClassProto>,
        early: bool,
    ) -> Result<bool, Thrown> {
        let id = self.classes.len();
        if !self.link_class(proto, self.decl_classes[slot].unit, early)? {
            return Ok(false);
        }
        if let Some(o) = self.class_order.get_mut(id) {
            *o = slot;
        }
        Ok(true)
    }

    pub(crate) fn deps_ready(&self, p: &ClassProto) -> bool {
        let known = |n: &PhpStr| self.class_ids.contains_key(&lc(n.as_bytes()));
        p.parent.as_ref().is_none_or(known)
            && p.interfaces.iter().all(known)
            && p.traits.iter().all(known)
    }

    fn first_missing_dep(&self, p: &ClassProto) -> Option<Vec<u8>> {
        let missing = |n: &PhpStr| {
            if self.class_ids.contains_key(&lc(n.as_bytes())) {
                None
            } else {
                Some(n.as_bytes().to_vec())
            }
        };
        p.parent
            .as_ref()
            .and_then(missing)
            .or_else(|| p.interfaces.iter().find_map(missing))
            .or_else(|| p.traits.iter().find_map(missing))
    }

    /// Link `proto` as the next class. `early`: hoisting (php's early
    /// binding); returns `Ok(false)`, linking nothing, when an inheritance
    /// check needs a class that is not declared yet.
    fn link_class(
        &mut self,
        proto: &Rc<ClassProto>,
        unit: UnitId,
        early: bool,
    ) -> Result<bool, Thrown> {
        let id = self.classes.len() as ClassId;
        let parent = match &proto.parent {
            Some(n) => Some(self.class_ids[&lc(n.as_bytes())]),
            None => None,
        };

        // Methods: parent's table, then traits, then own (override by lc name).
        // An *abstract* method is only a requirement: it must never clobber an
        // already-present concrete method (e.g. a trait's `abstract foo()`
        // alongside a concrete `foo()` inherited from the parent class).
        let mut methods: IndexMap<Vec<u8>, ResolvedMethod, FxBuildHasher> = parent
            .map(|p| self.classes[p as usize].methods.clone())
            .unwrap_or_default();
        let insert_method = |methods: &mut IndexMap<Vec<u8>, ResolvedMethod, FxBuildHasher>,
                             k: Vec<u8>,
                             m: ResolvedMethod| {
            if m.is_abstract {
                if let Some(existing) = methods.get(&k) {
                    if !existing.is_abstract {
                        return; // keep the concrete method
                    }
                }
            }
            methods.insert(k, m);
        };
        for tname in &proto.traits {
            let tid = self.class_ids[&lc(tname.as_bytes())];
            for (k, m) in self.classes[tid as usize].methods.clone() {
                // Trait methods adopt the using class as their defining class.
                insert_method(
                    &mut methods,
                    k,
                    ResolvedMethod {
                        defining_class: id,
                        prototype: None,
                        ..m
                    },
                );
            }
        }
        // Final, static, visibility and signature rules: `inherit`.
        for m in &proto.methods {
            insert_method(
                &mut methods,
                lc(m.name.as_bytes()),
                ResolvedMethod {
                    proto: m.proto.clone(),
                    unit,
                    defining_class: id,
                    visibility: m.visibility,
                    is_static: m.is_static,
                    is_abstract: m.is_abstract,
                    is_final: m.is_final,
                    prototype: None,
                },
            );
        }
        // php copies interface methods into the implementing class's function
        // table (do_inherit_iface_method): an abstract class / interface lists
        // the abstract methods it inherits from its interfaces.
        // (Built-in prelude classes implement their interfaces natively.)
        let ifaces_for_methods: &[PhpStr] =
            if self.internal_classes > 0 || proto.kind == ClassKind::Interface {
                &proto.interfaces
            } else {
                &[]
            };
        for iname in ifaces_for_methods {
            let iid = self.class_ids[&lc(iname.as_bytes())];
            for (k, m) in &self.classes[iid as usize].methods {
                if !methods.contains_key(k) {
                    methods.insert(k.clone(), m.clone());
                }
            }
        }
        let own_methods = proto.methods.iter().map(|m| lc(m.name.as_bytes()));
        let trait_methods: Vec<Vec<u8>> = proto
            .traits
            .iter()
            .flat_map(|t| {
                let tid = self.class_ids[&lc(t.as_bytes())];
                self.classes[tid as usize]
                    .methods
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .collect();
        let methods = php_member_order(methods, own_methods.chain(trait_methods));

        // Instance properties, flattened. A redeclared public/protected property
        // keeps its inherited slot (same plain key, same position); a property
        // whose inherited declaration is a parent *private* gets a fresh slot
        // appended, leaving the parent's private slot in place (php shadowing).
        let mut props: IndexMap<PhpStr, PropEntry> = parent
            .map(|p| self.classes[p as usize].props.clone())
            .unwrap_or_default();
        let mut prop_visible: FxHashMap<PhpStr, PhpStr> = parent
            .map(|p| self.classes[p as usize].prop_visible.clone())
            .unwrap_or_default();
        let mut own_private: FxHashMap<PhpStr, PhpStr> = FxHashMap::default();
        let mut declare = |props: &mut IndexMap<PhpStr, PropEntry>, mut pe: PropEntry| {
            // Redeclaring an inherited public/protected property keeps its
            // root declaration as the prototype.
            if let Some(inh) = prop_visible.get(&pe.name).and_then(|k| props.get(k)) {
                if inh.visibility != Visibility::Private {
                    pe.prototype = inh.prototype;
                }
            }
            let key = prop_storage_key(&proto.name, &pe.name, pe.visibility);
            prop_visible.insert(pe.name.clone(), key.clone());
            if pe.visibility == Visibility::Private {
                own_private.insert(pe.name.clone(), key.clone());
            }
            props.insert(key, pe);
        };
        for pp in &proto.props {
            let (default, init) = match &pp.default {
                PropDefaultProto::Const(v) => (v.clone(), None),
                PropDefaultProto::None => (Value::Null, None),
                // Placeholder until update_class_constants evaluates it.
                PropDefaultProto::Init(thunk) => (Value::Null, Some(unit_fn(thunk, unit))),
            };
            declare(
                &mut props,
                PropEntry {
                    name: pp.name.clone(),
                    default,
                    init,
                    visibility: pp.visibility,
                    set_visibility: pp.set_visibility,
                    declaring_class: id,
                    prototype: id,
                    ty: pp.type_hint.clone().map(Rc::new),
                    readonly: pp.is_readonly || proto.flags.is_readonly,
                    no_default: matches!(pp.default, PropDefaultProto::None),
                },
            );
        }
        // Trait properties follow the class's own (php binds traits after
        // compiling the class body); one the class declares itself is skipped.
        for tname in &proto.traits {
            let tid = self.class_ids[&lc(tname.as_bytes())];
            for pe in self.classes[tid as usize].props.values().cloned() {
                if proto.props.iter().any(|pp| pp.name == pe.name) {
                    continue;
                }
                declare(
                    &mut props,
                    PropEntry {
                        declaring_class: id,
                        prototype: id,
                        ..pe
                    },
                );
            }
        }

        // Static properties (inherited slots alias; redeclared get a fresh cell).
        let mut static_props: IndexMap<Vec<u8>, StaticProp> = parent
            .map(|p| self.classes[p as usize].static_props.clone())
            .unwrap_or_default();
        // A redeclared inherited public/protected static keeps the root
        // declaration as its prototype.
        let static_proto = |props: &IndexMap<Vec<u8>, StaticProp>, name: &[u8]| {
            props
                .get(name)
                .filter(|inh| inh.visibility != Visibility::Private)
                .map_or(id, |inh| inh.prototype)
        };
        for tname in &proto.traits {
            let tid = self.class_ids[&lc(tname.as_bytes())];
            for (k, sp) in self.classes[tid as usize].static_props.clone() {
                // Each using class gets its own cell, seeded from the trait's
                // (an unevaluated initializer is re-run in the using class).
                let v = sp.default.borrow().clone();
                let prototype = static_proto(&static_props, &k);
                static_props.insert(
                    k,
                    StaticProp {
                        cell: Rc::new(RefCell::new(v.clone())),
                        default: Rc::new(RefCell::new(v)),
                        init: sp.init.clone(),
                        visibility: sp.visibility,
                        declaring_class: id,
                        prototype,
                        ty: sp.ty.clone(),
                    },
                );
            }
        }
        for sp in &proto.static_props {
            let cell = Rc::new(RefCell::new(Value::Null));
            let init = match &sp.init {
                ConstInitProto::Const(v) => {
                    *cell.borrow_mut() = v.clone();
                    None
                }
                ConstInitProto::Thunk(thunk) => Some(unit_fn(thunk, unit)),
            };
            let prototype = static_proto(&static_props, sp.name.as_bytes());
            let default = cell.borrow().clone();
            static_props.insert(
                sp.name.as_bytes().to_vec(),
                StaticProp {
                    default: Rc::new(RefCell::new(default)),
                    cell,
                    init,
                    visibility: sp.visibility,
                    declaring_class: id,
                    prototype,
                    ty: sp.type_hint.clone().map(Rc::new),
                },
            );
        }

        // Class constants.
        let mut constants: IndexMap<Vec<u8>, ClassConst> = parent
            .map(|p| self.classes[p as usize].constants.clone())
            .unwrap_or_default();
        for tname in &proto.traits {
            let tid = self.class_ids[&lc(tname.as_bytes())];
            for (k, c) in self.classes[tid as usize].constants.clone() {
                constants.insert(
                    k,
                    ClassConst {
                        declaring_class: id,
                        ..c
                    },
                );
            }
        }
        // Constants are inherited from implemented interfaces too (their flattened
        // maps already include their own parent interfaces'); a parent class's
        // constant takes precedence, so don't overwrite.
        for iname in &proto.interfaces {
            let iid = self.class_ids[&lc(iname.as_bytes())];
            for (k, cc) in self.classes[iid as usize].constants.clone() {
                constants.entry(k).or_insert(cc);
            }
        }
        for c in &proto.constants {
            let (v, init) = match &c.value {
                ConstInitProto::Const(v) => (v.clone(), None),
                // Placeholder until first access evaluates it.
                ConstInitProto::Thunk(thunk) => {
                    (Value::Null, Some(ConstInit::Expr(unit_fn(thunk, unit))))
                }
            };
            constants.insert(
                c.name.as_bytes().to_vec(),
                ClassConst {
                    value: v,
                    visibility: c.visibility,
                    declaring_class: id,
                    init,
                    visited: false,
                    ty: c.type_hint.clone().map(Rc::new),
                },
            );
        }
        let own_consts = proto.constants.iter().map(|c| c.name.as_bytes().to_vec());
        let trait_consts: Vec<Vec<u8>> = proto
            .traits
            .iter()
            .flat_map(|t| {
                let tid = self.class_ids[&lc(t.as_bytes())];
                self.classes[tid as usize]
                    .constants
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .collect();
        let constants = php_member_order(constants, own_consts.chain(trait_consts));

        // Ancestors = self + parent's + each interface's closure.
        let mut ancestors = vec![id];
        if let Some(p) = parent {
            ancestors.extend(self.classes[p as usize].ancestors.iter().copied());
        }
        for iname in &proto.interfaces {
            let iid = self.class_ids[&lc(iname.as_bytes())];
            ancestors.push(iid);
            ancestors.extend(self.classes[iid as usize].ancestors.iter().copied());
        }
        ancestors.sort_unstable();
        ancestors.dedup();

        // php's `zend_do_implement_interfaces`: parent's list, then own direct
        // interfaces (deduped), then for each own interface its inherited
        // interfaces appended in reverse (`zend_do_inherit_interfaces`).
        // Without own interfaces php inherits the parent's list through
        // `zend_do_inherit_interfaces`, which walks it backwards.
        let mut interfaces: Vec<ClassId> = Vec::new();
        if self.internal_classes == 0 {
            // A built-in (prelude) class, registered the way php registers
            // internal ones: its own `__toString` implements Stringable when
            // its methods are registered, then the parent's interfaces are
            // inherited (backwards), then each declared interface is
            // implemented (`zend_class_implements`) with its own parents
            // appended backwards.
            let stringable = self.class_ids.get(&b"stringable"[..]).copied();
            let declares_to_string = proto.kind != ClassKind::Interface
                && proto
                    .methods
                    .iter()
                    .any(|m| m.name.as_bytes().eq_ignore_ascii_case(b"__toString"));
            if let (true, Some(sid)) = (declares_to_string, stringable) {
                interfaces.push(sid);
            }
            let add_with_parents = |interfaces: &mut Vec<ClassId>, iid: ClassId| {
                let ce_num = interfaces.len();
                for &inh in self.classes[iid as usize].interfaces.iter().rev() {
                    if !interfaces[..ce_num].contains(&inh) {
                        interfaces.push(inh);
                    }
                }
            };
            if let Some(p) = parent {
                for &inh in self.classes[p as usize].interfaces.iter().rev() {
                    if !interfaces.contains(&inh) {
                        interfaces.push(inh);
                    }
                }
            }
            for iname in &proto.interfaces {
                let iid = self.class_ids[&lc(iname.as_bytes())];
                if !interfaces.contains(&iid) {
                    interfaces.push(iid);
                    add_with_parents(&mut interfaces, iid);
                }
            }
        } else {
            // php's `zend_do_link_class`: with own interfaces the parent's
            // list is copied as is; without, `zend_do_inherit_interfaces`
            // inherits it walking backwards.
            if let Some(p) = parent {
                interfaces = self.classes[p as usize].interfaces.clone();
                if proto.interfaces.is_empty() {
                    interfaces.reverse();
                }
            }
            let num_parent = interfaces.len();
            for iname in &proto.interfaces {
                let iid = self.class_ids[&lc(iname.as_bytes())];
                if !interfaces.contains(&iid) {
                    interfaces.push(iid);
                }
            }
            let num_direct = interfaces.len();
            for i in num_parent..num_direct {
                let iid = interfaces[i];
                let ce_num = interfaces.len();
                for &inh in self.classes[iid as usize].interfaces.iter().rev() {
                    if !interfaces[..ce_num].contains(&inh) {
                        interfaces.push(inh);
                    }
                }
            }
        }
        // A `__toString` imported from a trait: php adds Stringable at link
        // time (the compiler only sees the class's own methods).
        if proto.kind != ClassKind::Trait && methods.contains_key(&b"__tostring"[..]) {
            if let Some(&sid) = self.class_ids.get(&b"stringable"[..]) {
                if !ancestors.contains(&sid) {
                    interfaces.push(sid);
                    ancestors.push(sid);
                    ancestors.sort_unstable();
                }
            }
        }
        let traits: Vec<ClassId> = proto
            .traits
            .iter()
            .map(|t| self.class_ids[&lc(t.as_bytes())])
            .collect();

        let parent_entry = parent.map(|p| &self.classes[p as usize]);
        let internal = proto.decl_file.is_empty();
        let reg = internal
            .then(|| rhp_builtins::zend::object_handlers::registered(proto.name.as_bytes()))
            .flatten();
        let handlers = reg
            .map(|r| r.handlers)
            .or_else(|| parent_entry.and_then(|p| p.handlers));
        let create_object = reg.map_or_else(
            || parent_entry.is_some_and(|p| p.create_object),
            |r| r.create_object,
        );
        let flags = link_flags(proto, parent_entry.map(|p| p.flags), handlers, internal);

        self.class_ids.insert(lc(proto.name.as_bytes()), id);
        // Not compiled into a slot (prelude): after everything compiled so far.
        self.class_order.push(self.decl_classes.len());
        self.classes.push(ClassEntry {
            name: proto.name.clone(),
            kind: proto.kind,
            flags,
            parent,
            ancestors,
            interfaces,
            traits,
            methods,
            props,
            prop_visible,
            own_private,
            constants,
            static_props,
            enum_cases: Vec::new(),
            enum_backing: proto.enum_backing,
            updated: false,
            proto: proto.clone(),
            unit,
            decl_file: proto.decl_file.clone(),
            decl_line: proto.decl_line,
            handlers,
            create_object,
        });

        // Enum cases: materialize one singleton object per case and expose it as
        // a public class constant, so `E::Case` resolves through `FetchClassConst`.
        if proto.kind == ClassKind::Enum {
            for case in &proto.enum_cases {
                let (value, init) = match &case.value {
                    None => (None, None),
                    Some(ConstInitProto::Const(v)) => (Some(v.clone()), None),
                    // The backing value is filled in on first access.
                    Some(ConstInitProto::Thunk(t)) => (
                        Some(Value::Null),
                        Some(ConstInit::EnumCase(unit_fn(t, unit))),
                    ),
                };
                let obj = self.new_enum_case(id, &case.name, value);
                self.classes[id as usize].constants.insert(
                    case.name.as_bytes().to_vec(),
                    ClassConst {
                        value: Value::Object(obj.clone()),
                        visibility: Visibility::Public,
                        declaring_class: id,
                        init,
                        visited: false,
                        ty: None,
                    },
                );
                self.classes[id as usize].enum_cases.push(obj);
            }
        }
        if self.internal_classes > 0
            && !proto.decl_file.is_empty()
            && !self.inheritance_checks(id, early)?
        {
            self.classes.pop();
            self.class_ids.remove(&lc(proto.name.as_bytes()));
            self.class_order.pop();
            return Ok(false);
        }
        Ok(true)
    }

    /// Build an enum case's singleton object: an instance of the enum class with
    /// a read-only `name` property (and `value` for a backed enum).
    fn new_enum_case(
        &mut self,
        class_id: ClassId,
        name: &PhpStr,
        value: Option<Value>,
    ) -> ObjectHandle {
        // The id is assigned on first access (see `touch_enum_case`).
        let oid = 0;
        let mut properties: IndexMap<PhpStr, Value, FxBuildHasher> =
            IndexMap::with_hasher(FxBuildHasher);
        properties.insert(PhpStr::from("name"), Value::Str(name.clone()));
        if let Some(v) = value {
            properties.insert(PhpStr::from("value"), v);
        }
        Rc::new(RefCell::new(ObjectData::new(class_id, oid, properties)))
    }

    /// Resolve a `ClassRef` (in the context of frame `top`) to a `ClassId`.
    pub(crate) fn resolve_class(&self, top: usize, class: ClassRef) -> Result<ClassId, Thrown> {
        match class {
            ClassRef::Name(nidx) => {
                self.resolve_class_by_name(self.stack[top].proto.names[nidx as usize].as_bytes())
            }
            ClassRef::SelfClass => self.stack[top]
                .defining_class
                .ok_or_else(|| Thrown::error("Error", "Cannot use \"self\" outside class context")),
            ClassRef::ParentClass => {
                let d = self.stack[top]
                    .defining_class
                    .ok_or_else(|| Thrown::error("Error", "Cannot use \"parent\" outside class"))?;
                self.classes[d as usize]
                    .parent
                    .ok_or_else(|| Thrown::error("Error", "Cannot use \"parent\" when no parent"))
            }
            ClassRef::StaticClass => self.stack[top].called_class.ok_or_else(|| {
                Thrown::error("Error", "Cannot use \"static\" outside class context")
            }),
            ClassRef::Dynamic(reg) => {
                let v = self.reg_get(top, reg);
                match v {
                    Value::Str(s) => self.resolve_class_by_name(s.as_bytes()),
                    Value::Object(h) => Ok(h.borrow().class),
                    _ => Err(Thrown::type_error("Class name must be a string or object")),
                }
            }
        }
    }

    /// Like [`Self::resolve_class`], but on a miss for a by-name reference it runs the
    /// registered autoloaders and retries (php's autoloading). `self`/`parent`/
    /// `static` and object refs never autoload.
    pub(crate) fn resolve_class_autoload(
        &mut self,
        top: usize,
        class: ClassRef,
    ) -> Result<ClassId, Thrown> {
        match self.resolve_class(top, class) {
            Ok(id) => Ok(id),
            Err(e) => {
                let name: Option<Vec<u8>> = match class {
                    ClassRef::Name(nidx) => Some(
                        self.stack[top].proto.names[nidx as usize]
                            .as_bytes()
                            .to_vec(),
                    ),
                    ClassRef::Dynamic(reg) => match self.reg_get(top, reg) {
                        crate::Value::Str(s) => Some(s.as_bytes().to_vec()),
                        _ => None,
                    },
                    _ => None,
                };
                // `new class …` whose dependencies were not available at
                // load: declare it now (php's DECLARE_ANON_CLASS).
                if let Some(n) = name.as_ref().filter(|n| is_anon_name(n)) {
                    let slot = self
                        .decl_classes
                        .iter()
                        .position(|d| !d.done && d.proto.name.as_bytes() == &n[..]);
                    if let Some(slot) = slot {
                        self.declare_class_at(slot)?;
                        return self.resolve_class_by_name(n);
                    }
                }
                match name {
                    Some(n) if self.try_autoload(&n)? => self.resolve_class_by_name(&n),
                    _ => Err(e),
                }
            }
        }
    }

    /// Run the registered autoloaders for `name` (php passes the FQCN without a
    /// leading `\`) until the class resolves. Returns whether it became defined.
    pub(crate) fn try_autoload(&mut self, name: &[u8]) -> Result<bool, Thrown> {
        let fqcn = name.strip_prefix(b"\\").unwrap_or(name).to_vec();
        if self.resolve_class_by_name(&fqcn).is_ok() {
            return Ok(true);
        }
        // zend_lookup_class_ex: an invalid class name never reaches the
        // autoloaders (zend_is_valid_class_name). Neither does an empty name
        // nor a lone "\" (GH-23232, php 8.5.11).
        if fqcn.is_empty()
            || !fqcn
                .iter()
                .all(|&b| b.is_ascii_alphanumeric() || b == b'_' || b == b'\\' || b >= 0x80)
        {
            return Ok(false);
        }
        let key = lc(&fqcn);
        if !self.in_autoload.insert(key.clone()) {
            return Ok(false); // already being autoloaded further up the stack
        }
        let r = (|| {
            for loader in self.autoloaders.clone() {
                self.call_callable_value(&loader, vec![crate::Value::str(fqcn.clone())])?;
                if self.resolve_class_by_name(&fqcn).is_ok() {
                    return Ok(true);
                }
            }
            Ok(false)
        })();
        self.in_autoload.remove(&key);
        r
    }

    pub(crate) fn resolve_class_by_name(&self, name: &[u8]) -> Result<ClassId, Thrown> {
        // A leading namespace separator is insignificant; otherwise match the
        // full (namespaced) name — class names resolve with no global fallback.
        let n = name.strip_prefix(b"\\").unwrap_or(name);
        with_lc(n, |k| self.class_ids.get(k).copied()).ok_or_else(|| {
            Thrown::error(
                "Error",
                format!("Class \"{}\" not found", String::from_utf8_lossy(n)),
            )
        })
    }

    pub(crate) fn class(&self, id: ClassId) -> &ClassEntry {
        &self.classes[id as usize]
    }

    /// Is a member with `vis` declared in `declaring` accessible from `caller`?
    pub(crate) fn check_access(
        &self,
        caller: Option<ClassId>,
        declaring: ClassId,
        vis: Visibility,
    ) -> bool {
        match vis {
            Visibility::Public => true,
            Visibility::Private => caller == Some(declaring),
            Visibility::Protected => match caller {
                // Accessible within the class hierarchy (either direction).
                Some(c) => {
                    self.classes[c as usize].ancestors.contains(&declaring)
                        || self.classes[declaring as usize].ancestors.contains(&c)
                }
                None => false,
            },
        }
    }

    /// php's static-property visibility check (`zend_std_get_static_property_with_info`):
    /// a private is reachable only from its declaring class; a protected from
    /// any class related to the root declaration's class.
    pub(crate) fn static_prop_accessible(&self, caller: Option<ClassId>, sp: &StaticProp) -> bool {
        match sp.visibility {
            Visibility::Public => true,
            Visibility::Private => caller == Some(sp.declaring_class),
            Visibility::Protected => {
                caller == Some(sp.declaring_class)
                    || protected_ok(&self.classes, caller, sp.prototype)
            }
        }
    }

    pub(crate) fn access_err(
        &self,
        what: &str,
        class: ClassId,
        member: &[u8],
        vis: Visibility,
    ) -> Thrown {
        let v = match vis {
            Visibility::Private => "private",
            Visibility::Protected => "protected",
            Visibility::Public => "public",
        };
        Thrown::error(
            "Error",
            format!(
                "Cannot access {v} {what} {}::{}{}",
                String::from_utf8_lossy(self.classes[class as usize].name.as_bytes()),
                if what == "property" { "$" } else { "" },
                String::from_utf8_lossy(member),
            ),
        )
    }

    pub(crate) fn static_prop_cell(
        &mut self,
        class_id: ClassId,
        name: &[u8],
    ) -> Result<Rc<RefCell<Value>>, Thrown> {
        if self.classes[class_id as usize]
            .static_props
            .contains_key(name)
        {
            self.update_class_constants(class_id)?;
        }
        self.classes[class_id as usize]
            .static_props
            .get(name)
            .map(|sp| sp.cell.clone())
            .ok_or_else(|| {
                Thrown::error(
                    "Error",
                    format!(
                        "Access to undeclared static property {}::${}",
                        String::from_utf8_lossy(self.classes[class_id as usize].name.as_bytes()),
                        String::from_utf8_lossy(name)
                    ),
                )
            })
    }

    /// Allocate a fresh instance of `class_id`, materializing property defaults.
    pub(crate) fn new_object(&mut self, class_id: ClassId) -> ObjectHandle {
        let id = self.next_object_id;
        self.next_object_id += 1;
        let mut properties: IndexMap<PhpStr, Value, FxBuildHasher> =
            IndexMap::with_hasher(FxBuildHasher);
        for (name, pe) in &self.classes[class_id as usize].props {
            if pe.starts_uninit() {
                continue; // uninitialized typed property: no slot yet
            }
            properties.insert(name.clone(), pe.default.clone());
        }
        let obj = Rc::new(RefCell::new(ObjectData::new(class_id, id, properties)));
        // Throwables record their creation site (file/line/trace) at creation,
        // before any constructor runs (zend_default_exception_new).
        if self.class_implements(class_id, b"Throwable") {
            self.init_throwable(&obj);
        }
        self.track_destructible(&obj);
        obj
    }

    /// Look up a method (flattened table) by name on `class_id`.
    pub(crate) fn lookup_method(&self, class_id: ClassId, name: &[u8]) -> Option<ResolvedMethod> {
        let methods = &self.classes[class_id as usize].methods;
        with_lc(name, |k| methods.get(k).cloned())
    }

    /// Push a frame for a method call, binding `$this` and the LSB classes.
    #[allow(clippy::too_many_arguments)] // VM call-dispatch frame setup
    pub(crate) fn enter_method(
        &mut self,
        top: usize,
        method: &ResolvedMethod,
        this: Option<ObjectHandle>,
        called_class: ClassId,
        dst: Reg,
        base: Reg,
        argc: u16,
    ) -> Result<(), Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let args = Args::Window { top, base, argc };
        self.push_method_frame(method, this, called_class, args, dst)
    }

    /// Push a method frame.
    pub(crate) fn push_method_frame(
        &mut self,
        method: &ResolvedMethod,
        this: Option<ObjectHandle>,
        called_class: ClassId,
        args: Args<'_>,
        dst: Reg,
    ) -> Result<(), Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let callee = method.proto.clone();
        let mut frame = Frame::new(callee.clone(), method.unit, dst);
        if let (Some(this_cv), Some(obj)) = (callee.this_cv, this) {
            frame.regs[this_cv as usize] = Value::Object(obj);
        }
        frame.defining_class = Some(method.defining_class);
        frame.called_class = Some(called_class);
        self.bind_frame(&mut frame, args, Some(self.caller_strict()))?;
        self.bind_superglobals(&mut frame);
        if callee.is_generator {
            let gen = self.make_generator(frame);
            let caller = self.stack.len() - 1;
            if dst != Reg::MAX {
                self.reg_set(caller, dst, gen);
            }
            return Ok(());
        }
        self.stack.push(frame);
        Ok(())
    }

    /// Run a deferred-initializer thunk in `class_id`'s scope.
    fn run_init_thunk(&mut self, thunk: &UnitFn, class_id: ClassId) -> Result<Value, Thrown> {
        self.run_proto_sync(
            thunk.clone(),
            None,
            Some(class_id),
            Some(class_id),
            Vec::new(),
            None,
        )
    }

    /// php's `update_property`: a property initializer is checked against the
    /// declared type in strict mode (only int -> float widening).
    fn verify_prop_init_type(
        &self,
        v: Value,
        ty: &TypeHint,
        decl: ClassId,
        name: &[u8],
    ) -> Result<Value, Thrown> {
        match self.check_one(&v, ty, true, Some(decl)) {
            Ok(None) => Ok(v),
            Ok(Some(c)) => Ok(c),
            Err(()) => Err(Thrown::type_error(format!(
                "Cannot assign {} to property {}::${} of type {}",
                rhp_builtins::zval_value_name(self, &v),
                String::from_utf8_lossy(self.classes[decl as usize].name.as_bytes()),
                String::from_utf8_lossy(name),
                self.type_string_resolved(ty, Some(decl), None),
            ))),
        }
    }

    /// php's `zend_verify_class_constant_type` (strict; int -> float widens).
    fn verify_class_const_type(
        &self,
        v: Value,
        ty: &TypeHint,
        decl: ClassId,
        name: &[u8],
    ) -> Result<Value, Thrown> {
        match self.check_one(&v, ty, true, Some(decl)) {
            Ok(None) => Ok(v),
            Ok(Some(c)) => Ok(c),
            Err(()) => {
                let tn = rhp_builtins::zval_type_name(self, &v);
                Err(Thrown::type_error(format!(
                    "Cannot assign {} to class constant {}::{} of type {}",
                    tn,
                    String::from_utf8_lossy(self.classes[decl as usize].name.as_bytes()),
                    String::from_utf8_lossy(name),
                    self.type_string_resolved(ty, Some(decl), None),
                )))
            }
        }
    }

    /// Read class constant `name` of `cls`, evaluating its initializer first if
    /// it is still pending (php's `zend_update_class_constant`). `display` is
    /// the class as written at the access site (`self`, `parent`, a name), used
    /// in the cycle error; `in_const_expr` is true when the access itself
    /// comes from a constant expression (only those mark the constant as
    /// visited, exactly like php's `zend_get_class_constant_ex`).
    pub(crate) fn class_const_value(
        &mut self,
        cls: ClassId,
        name: &[u8],
        display: &[u8],
        in_const_expr: bool,
    ) -> Result<Option<Value>, Thrown> {
        let r = self.class_const_value_inner(cls, name, display, in_const_expr)?;
        if let Some(Value::Object(h)) = &r {
            self.touch_enum_case(h);
        }
        Ok(r)
    }

    /// An enum case object gets its object id when its constant is first
    /// evaluated (php creates case objects lazily).
    pub(crate) fn touch_enum_case(&mut self, h: &ObjectHandle) {
        if h.borrow().id == 0 {
            h.borrow_mut().id = self.next_object_id;
            self.next_object_id += 1;
        }
    }

    /// [`Vm::touch_enum_case`] for every case of enum `cls`, in order.
    pub(crate) fn touch_enum_cases(&mut self, cls: ClassId) {
        let cases = self.classes[cls as usize].enum_cases.clone();
        for h in &cases {
            self.touch_enum_case(h);
        }
    }

    fn class_const_value_inner(
        &mut self,
        cls: ClassId,
        name: &[u8],
        display: &[u8],
        in_const_expr: bool,
    ) -> Result<Option<Value>, Thrown> {
        let Some(c) = self.classes[cls as usize].constants.get(name) else {
            return Ok(None);
        };
        let Some(init) = c.init.clone() else {
            return Ok(Some(c.value.clone()));
        };
        let decl = c.declaring_class;
        let c_ty = c.ty.clone();
        // An inherited constant is evaluated (once) in its declaring class.
        let owner = if decl != cls && self.classes[decl as usize].constants.contains_key(name) {
            decl
        } else {
            cls
        };
        if owner != cls && self.classes[owner as usize].constants[name].init.is_none() {
            let v = self.classes[owner as usize].constants[name].value.clone();
            let e = self.classes[cls as usize].constants.get_mut(name).unwrap();
            e.value = v.clone();
            e.init = None;
            return Ok(Some(v));
        }
        if in_const_expr {
            if self.classes[owner as usize].constants[name].visited {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Cannot declare self-referencing constant {}::{}",
                        String::from_utf8_lossy(display),
                        String::from_utf8_lossy(name)
                    ),
                ));
            }
            self.classes[owner as usize]
                .constants
                .get_mut(name)
                .unwrap()
                .visited = true;
        }
        let r = match &init {
            ConstInit::Expr(t) | ConstInit::EnumCase(t) => self.run_init_thunk(t, decl),
        };
        if in_const_expr {
            if let Some(e) = self.classes[owner as usize].constants.get_mut(name) {
                e.visited = false;
            }
        }
        let mut v = r?;
        if let (ConstInit::Expr(_), Some(ty)) = (&init, &c_ty) {
            v = self.verify_class_const_type(v, ty, decl, name)?;
        }
        // Evaluation may have completed it re-entrantly; finish it only once.
        let e = self.classes[owner as usize]
            .constants
            .get_mut(name)
            .unwrap();
        if e.init.is_some() {
            e.init = None;
            match init {
                ConstInit::Expr(_) => e.value = v,
                ConstInit::EnumCase(_) => {
                    if let Value::Object(h) = &e.value {
                        h.borrow_mut().properties.insert(PhpStr::from("value"), v);
                    }
                }
            }
        }
        let value = e.value.clone();
        if owner != cls {
            let e = self.classes[cls as usize].constants.get_mut(name).unwrap();
            e.value = value.clone();
            e.init = None;
        }
        Ok(Some(value))
    }

    /// php's `zend_update_class_constants`: on first use of a class (object
    /// creation, static-property access, enum case listing, reflection),
    /// evaluate every pending initializer — parent first, then the class's
    /// constants, instance-property defaults and static properties.
    pub(crate) fn update_class_constants(&mut self, cls: ClassId) -> Result<(), Thrown> {
        if self.classes[cls as usize].updated {
            return Ok(());
        }
        self.touch_enum_cases(cls);
        if let Some(p) = self.classes[cls as usize].parent {
            self.update_class_constants(p)?;
        }
        let pending_consts: Vec<Vec<u8>> = self.classes[cls as usize]
            .constants
            .iter()
            .filter(|(_, c)| c.init.is_some())
            .map(|(k, _)| k.clone())
            .collect();
        for name in pending_consts {
            let display = self.classes[cls as usize].name.as_bytes().to_vec();
            self.class_const_value(cls, &name, &display, false)?;
        }
        let pending_props: Vec<PhpStr> = self.classes[cls as usize]
            .props
            .iter()
            .filter(|(_, p)| p.init.is_some())
            .map(|(k, _)| k.clone())
            .collect();
        for name in pending_props {
            let pe = &self.classes[cls as usize].props[&name];
            let decl = pe.declaring_class;
            let thunk = pe.init.clone();
            // An inherited default was already evaluated by the parent.
            let inherited = if decl != cls {
                self.classes[decl as usize]
                    .props
                    .get(&name)
                    .filter(|dp| dp.init.is_none())
                    .map(|dp| dp.default.clone())
            } else {
                None
            };
            let ty = pe.ty.clone();
            let shown = pe.name.clone();
            let v = match (inherited, thunk) {
                (Some(v), _) => v,
                (None, Some(t)) => {
                    let v = self.run_init_thunk(&t, decl)?;
                    match ty {
                        Some(ty) => self.verify_prop_init_type(v, &ty, decl, shown.as_bytes())?,
                        None => v,
                    }
                }
                (None, None) => continue,
            };
            if let Some(pe) = self.classes[cls as usize].props.get_mut(&name) {
                pe.default = v;
                pe.init = None;
            }
        }
        let pending_statics: Vec<Vec<u8>> = self.classes[cls as usize]
            .static_props
            .iter()
            .filter(|(_, p)| p.init.is_some())
            .map(|(k, _)| k.clone())
            .collect();
        for name in pending_statics {
            let sp = &self.classes[cls as usize].static_props[&name];
            let (decl, cell, thunk) = (sp.declaring_class, sp.cell.clone(), sp.init.clone());
            let default = sp.default.clone();
            let ty = sp.ty.clone();
            // An inherited slot aliases the parent's cell (already updated).
            if decl == cls {
                if let Some(t) = thunk {
                    let mut v = self.run_init_thunk(&t, cls)?;
                    if let Some(ty) = ty {
                        v = self.verify_prop_init_type(v, &ty, cls, &name)?;
                    }
                    *default.borrow_mut() = v.clone();
                    *cell.borrow_mut() = v;
                }
            }
            if let Some(sp) = self.classes[cls as usize].static_props.get_mut(&name) {
                sp.init = None;
            }
        }
        if let Some(backing) = self.classes[cls as usize].enum_backing {
            self.build_backed_enum_table(cls, backing)?;
        }
        self.classes[cls as usize].updated = true;
        Ok(())
    }

    /// php's `zend_enum_build_backed_enum_table`: every case value must have
    /// the enum's backing type, and no two cases may share a value. On
    /// failure the class stays un-updated, so the next use raises again.
    fn build_backed_enum_table(
        &self,
        cls: ClassId,
        backing: rhp_bytecode::EnumBacking,
    ) -> Result<(), Thrown> {
        use rhp_bytecode::EnumBacking;
        let entry = &self.classes[cls as usize];
        let mut seen: Vec<(Value, PhpStr)> = Vec::with_capacity(entry.enum_cases.len());
        for case in &entry.enum_cases {
            let o = case.borrow();
            let name = match o.properties.get(&PhpStr::from("name")) {
                Some(Value::Str(s)) => s.clone(),
                _ => PhpStr::from(""),
            };
            let v = o
                .properties
                .get(&PhpStr::from("value"))
                .cloned()
                .unwrap_or(Value::Null);
            let ok = matches!(
                (&v, backing),
                (Value::Int(_), EnumBacking::Int) | (Value::Str(_), EnumBacking::Str)
            );
            if !ok {
                let tn = match &v {
                    Value::Bool(_) => "bool",
                    Value::Int(_) => "int",
                    Value::Float(_) => "float",
                    Value::Str(_) => "string",
                    Value::Array(_) => "array",
                    Value::Null => "null",
                    Value::Resource(_) => "resource",
                    _ => "object",
                };
                return Err(Thrown::type_error(format!(
                    "Enum case type {} does not match enum backing type {}",
                    tn,
                    match backing {
                        EnumBacking::Int => "int",
                        EnumBacking::Str => "string",
                    }
                )));
            }
            let dup = seen.iter().find(|(sv, _)| match (sv, &v) {
                (Value::Int(a), Value::Int(b)) => a == b,
                (Value::Str(a), Value::Str(b)) => a.as_bytes() == b.as_bytes(),
                _ => false,
            });
            if let Some((_, first)) = dup {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "Duplicate value in enum {} for cases {} and {}",
                        String::from_utf8_lossy(entry.name.as_bytes()),
                        String::from_utf8_lossy(first.as_bytes()),
                        String::from_utf8_lossy(name.as_bytes()),
                    ),
                ));
            }
            seen.push((v, name));
        }
        Ok(())
    }
}
