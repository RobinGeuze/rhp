//! OOP class table: the linked `ClassEntry`s + `Vm` methods to install
//! (flatten + link) `ClassProto`s, resolve `ClassRef`s, allocate objects, and
//! look up methods/static props. See `docs/design/oop.md`.
//!
//! Inheritance is flattened at link time (parent merged first, own members
//! override) for O(1), inline-cacheable dispatch.

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

use crate::{Frame, Vm};

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
    pub methods: IndexMap<Vec<u8>, ResolvedMethod>,
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
}

#[derive(Clone)]
pub struct ResolvedMethod {
    pub proto: Rc<FunctionProto>,
    pub defining_class: ClassId,
    pub visibility: Visibility,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_final: bool,
}

#[derive(Clone)]
pub struct PropEntry {
    /// Unmangled property name (no `$`).
    pub name: PhpStr,
    pub default: Value,
    /// A not-yet-evaluated non-constant default (evaluated in the declaring
    /// class's scope by [`Vm::update_class_constants`]).
    pub init: Option<Rc<FunctionProto>>,
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
}

#[derive(Clone)]
pub struct StaticProp {
    pub cell: Rc<RefCell<Value>>,
    /// A not-yet-evaluated non-constant initializer for `cell`.
    pub init: Option<Rc<FunctionProto>>,
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
    Expr(Rc<FunctionProto>),
    /// `case C = <expr>;` of a backed enum — the constant's value is already
    /// the case object; the thunk yields its backing `value` property.
    EnumCase(Rc<FunctionProto>),
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
/// an access error.
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
    let v = if sv == Visibility::Private {
        "private(set)"
    } else {
        "protected(set)"
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

/// Reorder a resolved member table into php's order: the class's own members
/// in declaration order, then trait members, then everything inherited (the
/// parent's table, itself already in this order, followed by interface
/// members). Only the order changes — which member won is decided by the
/// caller. This is what reflection and `get_class_*` enumerate.
fn php_member_order<V>(
    mut map: IndexMap<Vec<u8>, V>,
    first: impl IntoIterator<Item = Vec<u8>>,
) -> IndexMap<Vec<u8>, V> {
    let mut out = IndexMap::with_capacity(map.len());
    for k in first {
        if let Some(v) = map.shift_remove(&k) {
            out.insert(k, v);
        }
    }
    out.extend(map);
    out
}

impl Vm {
    /// Link every `ClassProto` into the class table, in dependency order so that
    /// `extends`/`implements`/`use` (which may be forward references) resolve.
    pub(crate) fn install_classes(&mut self, protos: &[ClassProto]) -> Result<(), Thrown> {
        let mut pending: Vec<&ClassProto> = protos.iter().collect();
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
                    self.link_class(proto)?;
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

    /// Append a program's user class declarations (top-level, then nested)
    /// to the flat declaration table, stamped with the file's bases.
    /// Returns the table range of the top-level ones.
    pub(crate) fn register_decl_classes(
        &mut self,
        prog: &rhp_bytecode::Program,
        base: u32,
    ) -> std::ops::Range<usize> {
        let cbase = self.decl_classes.len() as u32;
        let user = prog.classes.iter().skip(prog.num_prelude_classes);
        for c in user.chain(prog.cond_classes.iter()) {
            let c = crate::rebase_class(c, base, cbase);
            self.decl_classes.push((c, false));
        }
        let start = cbase as usize;
        start..start + prog.classes.len() - prog.num_prelude_classes
    }

    /// Hoist (early-bind) the top-level classes in `range`
    /// whose parent/interfaces/traits are available — possibly from each
    /// other. The others stay undeclared until their `DeclareClass` runs.
    pub(crate) fn hoist_decl_classes(
        &mut self,
        range: std::ops::Range<usize>,
    ) -> Result<(), Thrown> {
        loop {
            let mut progressed = false;
            for i in range.clone() {
                if self.decl_classes[i].1 {
                    continue;
                }
                let proto = self.decl_classes[i].0.clone();
                if self.class_ids.contains_key(&lc(proto.name.as_bytes()))
                    || !self.deps_ready(&proto)
                {
                    continue;
                }
                self.link_class(&proto)?;
                self.decl_classes[i].1 = true;
                progressed = true;
            }
            if !progressed {
                break;
            }
        }
        Ok(())
    }

    /// Run-time class declaration (`DeclareClass`): link the class now,
    /// autoloading missing dependencies. A no-op if it was hoisted.
    pub(crate) fn declare_class_at(&mut self, slot: usize) -> Result<(), Thrown> {
        let Some((proto, done)) = self.decl_classes.get(slot) else {
            return Ok(());
        };
        if *done {
            return Ok(());
        }
        let proto = proto.clone();
        if self.class_ids.contains_key(&lc(proto.name.as_bytes())) {
            return Err(Thrown::error(
                "Error",
                format!(
                    "Cannot declare {} {}, because the name is already in use",
                    kind_word(proto.kind),
                    String::from_utf8_lossy(proto.name.as_bytes())
                ),
            ));
        }
        let deps = proto
            .parent
            .iter()
            .map(|n| ("Class", n))
            .chain(proto.interfaces.iter().map(|n| ("Interface", n)))
            .chain(proto.traits.iter().map(|n| ("Trait", n)));
        for (what, n) in deps.collect::<Vec<_>>() {
            if !self.class_ids.contains_key(&lc(n.as_bytes()))
                && !self.try_autoload(n.as_bytes())?
            {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "{what} \"{}\" not found",
                        String::from_utf8_lossy(n.as_bytes())
                    ),
                ));
            }
        }
        self.link_class(&proto)?;
        self.decl_classes[slot].1 = true;
        Ok(())
    }

    fn deps_ready(&self, p: &ClassProto) -> bool {
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

    fn link_class(&mut self, proto: &ClassProto) -> Result<(), Thrown> {
        let id = self.classes.len() as ClassId;
        let parent = match &proto.parent {
            Some(n) => Some(self.class_ids[&lc(n.as_bytes())]),
            None => None,
        };

        // Methods: parent's table, then traits, then own (override by lc name).
        // An *abstract* method is only a requirement: it must never clobber an
        // already-present concrete method (e.g. a trait's `abstract foo()`
        // alongside a concrete `foo()` inherited from the parent class).
        let mut methods: IndexMap<Vec<u8>, ResolvedMethod> = parent
            .map(|p| self.classes[p as usize].methods.clone())
            .unwrap_or_default();
        let insert_method =
            |methods: &mut IndexMap<Vec<u8>, ResolvedMethod>, k: Vec<u8>, m: ResolvedMethod| {
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
                        ..m
                    },
                );
            }
        }
        for m in &proto.methods {
            if let Some(existing) = methods.get(&lc(m.name.as_bytes())) {
                // Private methods are not inherited (except constructors).
                let private = existing.visibility == Visibility::Private
                    && !m.name.as_bytes().eq_ignore_ascii_case(b"__construct");
                if existing.is_final && existing.defining_class != id && !private {
                    return Err(Thrown::error(
                        "Error",
                        format!(
                            "Cannot override final method {}::{}()",
                            String::from_utf8_lossy(
                                self.classes[existing.defining_class as usize]
                                    .name
                                    .as_bytes()
                            ),
                            String::from_utf8_lossy(existing.proto.name.as_bytes())
                        ),
                    ));
                }
            }
            insert_method(
                &mut methods,
                lc(m.name.as_bytes()),
                ResolvedMethod {
                    proto: m.proto.clone(),
                    defining_class: id,
                    visibility: m.visibility,
                    is_static: m.is_static,
                    is_abstract: m.is_abstract,
                    is_final: m.is_final,
                },
            );
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
                PropDefaultProto::Init(thunk) => (Value::Null, Some(thunk.clone())),
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
                let v = sp.cell.borrow().clone();
                let prototype = static_proto(&static_props, &k);
                static_props.insert(
                    k,
                    StaticProp {
                        cell: Rc::new(RefCell::new(v)),
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
                ConstInitProto::Thunk(thunk) => Some(thunk.clone()),
            };
            let prototype = static_proto(&static_props, sp.name.as_bytes());
            static_props.insert(
                sp.name.as_bytes().to_vec(),
                StaticProp {
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
                ConstInitProto::Thunk(thunk) => (Value::Null, Some(ConstInit::Expr(thunk.clone()))),
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
        let mut interfaces: Vec<ClassId> = parent
            .map(|p| self.classes[p as usize].interfaces.clone())
            .unwrap_or_default();
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
        let traits: Vec<ClassId> = proto
            .traits
            .iter()
            .map(|t| self.class_ids[&lc(t.as_bytes())])
            .collect();

        self.class_ids.insert(lc(proto.name.as_bytes()), id);
        self.classes.push(ClassEntry {
            name: proto.name.clone(),
            kind: proto.kind,
            flags: proto.flags,
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
        });

        // Enum cases: materialize one singleton object per case and expose it as
        // a public class constant, so `E::Case` resolves through `FetchClassConst`.
        if proto.kind == ClassKind::Enum {
            for case in &proto.enum_cases {
                let (value, init) = match &case.value {
                    None => (None, None),
                    Some(ConstInitProto::Const(v)) => (Some(v.clone()), None),
                    // The backing value is filled in on first access.
                    Some(ConstInitProto::Thunk(t)) => {
                        (Some(Value::Null), Some(ConstInit::EnumCase(t.clone())))
                    }
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
        Ok(())
    }

    /// Build an enum case's singleton object: an instance of the enum class with
    /// a read-only `name` property (and `value` for a backed enum).
    fn new_enum_case(
        &mut self,
        class_id: ClassId,
        name: &PhpStr,
        value: Option<Value>,
    ) -> ObjectHandle {
        let oid = self.next_object_id;
        self.next_object_id += 1;
        let mut properties: IndexMap<PhpStr, Value, FxBuildHasher> =
            IndexMap::with_hasher(FxBuildHasher);
        properties.insert(PhpStr::from("name"), Value::Str(name.clone()));
        if let Some(v) = value {
            properties.insert(PhpStr::from("value"), v);
        }
        Rc::new(RefCell::new(ObjectData {
            class: class_id,
            id: oid,
            properties,
        }))
    }

    /// Resolve a `ClassRef` (in the context of frame `top`) to a `ClassId`.
    pub(crate) fn resolve_class(&self, top: usize, class: ClassRef) -> Result<ClassId, Thrown> {
        match class {
            ClassRef::Name(nidx) => {
                let proto = self.stack[top].proto.clone();
                self.resolve_class_by_name(proto.names[nidx as usize].as_bytes())
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

    /// Like [`resolve_class`], but on a miss for a by-name reference it runs the
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
        // autoloaders (zend_is_valid_class_name).
        if !fqcn
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
        self.class_ids.get(&lc(n)).copied().ok_or_else(|| {
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
            properties.insert(name.clone(), pe.default.clone());
        }
        let obj = Rc::new(RefCell::new(ObjectData {
            class: class_id,
            id,
            properties,
        }));
        // Throwables record their creation site (file/line/trace) at creation,
        // before any constructor runs (zend_default_exception_new).
        if self.class_implements(class_id, b"Throwable") {
            self.init_throwable(&obj);
        }
        obj
    }

    /// Look up a method (flattened table) by name on `class_id`.
    pub(crate) fn lookup_method(&self, class_id: ClassId, name: &[u8]) -> Option<ResolvedMethod> {
        self.classes[class_id as usize]
            .methods
            .get(&lc(name))
            .cloned()
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
        let args: Vec<Value> = (0..argc)
            .map(|i| self.stack[top].regs[(base + i) as usize].clone())
            .collect();
        self.push_method_frame(method, this, called_class, args, dst)
    }

    /// Push a method frame with pre-collected args (for spread / magic dispatch).
    pub(crate) fn push_method_frame(
        &mut self,
        method: &ResolvedMethod,
        this: Option<ObjectHandle>,
        called_class: ClassId,
        args: Vec<Value>,
        dst: Reg,
    ) -> Result<(), Thrown> {
        if self.stack.len() >= self.max_depth {
            return Err(Thrown::error("Error", "Maximum call stack depth exceeded"));
        }
        let callee = method.proto.clone();
        let mut frame = Frame::new(callee.clone(), dst);
        if let (Some(this_cv), Some(obj)) = (callee.this_cv, this) {
            frame.regs[this_cv as usize] = Value::Object(obj);
        }
        frame.defining_class = Some(method.defining_class);
        frame.called_class = Some(called_class);
        self.bind_frame(&mut frame, &args, Some(self.caller_strict()))?;
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
    fn run_init_thunk(
        &mut self,
        thunk: &Rc<FunctionProto>,
        class_id: ClassId,
    ) -> Result<Value, Thrown> {
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
                self.zval_value_name(&v),
                String::from_utf8_lossy(self.classes[decl as usize].name.as_bytes()),
                String::from_utf8_lossy(name),
                crate::type_string(ty),
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
                // zend_zval_type_name: like the value name, but bools are "bool".
                let tn = match &v {
                    Value::Bool(_) => "bool".to_string(),
                    other => self.zval_value_name(other),
                };
                Err(Thrown::type_error(format!(
                    "Cannot assign {} to class constant {}::{} of type {}",
                    tn,
                    String::from_utf8_lossy(self.classes[decl as usize].name.as_bytes()),
                    String::from_utf8_lossy(name),
                    crate::type_string(ty),
                )))
            }
        }
    }

    /// php's `zend_zval_value_name`.
    pub(crate) fn zval_value_name(&self, v: &Value) -> String {
        match v {
            Value::Bool(true) => "true".into(),
            Value::Bool(false) => "false".into(),
            other => self.value_debug_type(other),
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
            let ty = sp.ty.clone();
            // An inherited slot aliases the parent's cell (already updated).
            if decl == cls {
                if let Some(t) = thunk {
                    let mut v = self.run_init_thunk(&t, cls)?;
                    if let Some(ty) = ty {
                        v = self.verify_prop_init_type(v, &ty, cls, &name)?;
                    }
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
