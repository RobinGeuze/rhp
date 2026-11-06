//! The declaration errors php raises while linking a class
//! (zend_do_link_class): enum rules (zend_enum.c), what a class may extend,
//! readonly class inheritance and trait use. Each is a fatal error at the
//! class declaration's line.

use rhp_bytecode::{AttributeProto, ClassProto, HookKind};
use rhp_values::{ClassId, ClassKind, PhpStr, Thrown, Visibility};

use crate::classes::PropEntry;
use crate::Vm;

/// Whether `attrs` holds `#[\Override]`.
fn has_override(attrs: &[AttributeProto]) -> bool {
    attrs
        .iter()
        .any(|a| a.name.as_bytes().eq_ignore_ascii_case(b"Override"))
}

/// A class name as php prints it (an anonymous class's name stops at NUL).
fn shown(name: &PhpStr) -> String {
    let b = name.as_bytes();
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// zend_get_object_type_uc.
pub(crate) fn kind_uc(k: ClassKind) -> &'static str {
    match k {
        ClassKind::Class => "Class",
        ClassKind::Interface => "Interface",
        ClassKind::Trait => "Trait",
        ClassKind::Enum => "Enum",
    }
}

/// php's property_has_operation.
fn has_operation(p: &PropEntry, kind: usize) -> bool {
    (!p.virt && (kind == HookKind::Get as usize || !p.readonly))
        || p.hooks.as_ref().is_some_and(|h| h[kind].is_some())
}

/// zend_is_reserved_class_name for an unqualified name.
pub(crate) fn is_reserved_class_name(name: &[u8]) -> bool {
    const RESERVED: [&[u8]; 17] = [
        b"bool",
        b"false",
        b"float",
        b"int",
        b"null",
        b"parent",
        b"self",
        b"static",
        b"string",
        b"true",
        b"void",
        b"never",
        b"iterable",
        b"object",
        b"mixed",
        b"array",
        b"callable",
    ];
    RESERVED.iter().any(|r| r.eq_ignore_ascii_case(name))
}

fn lc(b: &[u8]) -> Vec<u8> {
    b.to_ascii_lowercase()
}

/// zend_verify_enum_magic_methods' list, in php's order.
const ENUM_FORBIDDEN_METHODS: [&str; 14] = [
    "__construct",
    "__destruct",
    "__clone",
    "__get",
    "__set",
    "__unset",
    "__isset",
    "__toString",
    "__debugInfo",
    "__serialize",
    "__unserialize",
    "__sleep",
    "__wakeup",
    "__set_state",
];

impl Vm {
    /// Check `proto` (dependencies all declared) before linking it, in the
    /// order zend_do_link_class finds the errors.
    pub(crate) fn check_link(&mut self, proto: &ClassProto) -> Result<(), Thrown> {
        if proto.decl_file.is_empty() {
            return Ok(()); // built-in classes are known good
        }
        let name = String::from_utf8_lossy(proto.name.as_bytes()).into_owned();
        let name = name.split('\0').next().unwrap_or_default().to_string();
        let fail = |vm: &mut Vm, ty: i64, msg: String| -> Result<(), Thrown> {
            Err(vm.fatal_error_at(
                ty,
                msg.as_bytes(),
                proto.decl_file.as_bytes().to_vec(),
                proto.decl_line as i64,
            ))
        };
        let class_of = |vm: &Vm, n: &rhp_values::PhpStr| {
            vm.class_ids.get(&lc(n.as_bytes())).map(|&id| id as usize)
        };
        let is_enum = proto.kind == ClassKind::Enum;
        // zend_do_link_class fetching the traits.
        for t in &proto.traits {
            if let Some(tid) = class_of(self, t) {
                if self.classes[tid].kind != ClassKind::Trait {
                    return Err(Thrown::error(
                        "Error",
                        format!(
                            "{name} cannot use {} - it is not a trait",
                            shown(&self.classes[tid].name)
                        ),
                    ));
                }
            }
        }
        // zend_enum_register_funcs
        if is_enum {
            let mut builtin = vec!["cases"];
            if proto.enum_backing.is_some() {
                builtin.extend(["from", "tryFrom"]);
            }
            for b in builtin {
                if proto
                    .methods
                    .iter()
                    .any(|m| m.name.as_bytes().eq_ignore_ascii_case(b.as_bytes()))
                {
                    return fail(self, 64, format!("Cannot redeclare {name}::{b}()"));
                }
            }
        }
        // Trait binding: constants against enum cases, readonly classes'
        // properties.
        for t in &proto.traits {
            let Some(tid) = class_of(self, t) else {
                continue;
            };
            let tname = String::from_utf8_lossy(self.classes[tid].name.as_bytes()).into_owned();
            if is_enum {
                for k in self.classes[tid].constants.keys() {
                    if proto
                        .enum_cases
                        .iter()
                        .any(|c| c.name.as_bytes() == k.as_slice())
                    {
                        let k = String::from_utf8_lossy(k);
                        let msg = format!(
                            "Cannot use trait {tname}, because {tname}::{k} conflicts with enum case {name}::{k}"
                        );
                        return fail(self, 64, msg);
                    }
                }
            }
            if proto.flags.is_readonly {
                let bad = self.classes[tid]
                    .props
                    .values()
                    .find(|p| !p.readonly && !proto.props.iter().any(|pp| pp.name == p.name))
                    .map(|p| String::from_utf8_lossy(p.name.as_bytes()).into_owned())
                    .or_else(|| {
                        // A static property is never readonly.
                        self.classes[tid]
                            .static_props
                            .keys()
                            .find(|k| {
                                !proto
                                    .static_props
                                    .iter()
                                    .any(|sp| sp.name.as_bytes() == k.as_slice())
                            })
                            .map(|k| String::from_utf8_lossy(k).into_owned())
                    });
                if let Some(p) = bad {
                    let msg = format!(
                        "Readonly class {name} cannot use trait with a non-readonly property {tname}::${p}"
                    );
                    return fail(self, 64, msg);
                }
            }
        }
        // zend_do_inheritance_ex
        if let Some(pid) = proto.parent.as_ref().and_then(|p| class_of(self, p)) {
            let parent = &self.classes[pid];
            let pname_raw = parent.name.as_bytes().to_vec();
            let pname = String::from_utf8_lossy(&pname_raw).into_owned();
            let (pkind, pflags) = (parent.kind, parent.flags);
            if proto.kind == ClassKind::Interface {
                if pkind != ClassKind::Interface {
                    let msg = format!("Interface {name} cannot extend class {pname}");
                    return fail(self, 64, msg);
                }
            } else if pkind == ClassKind::Enum {
                let msg = format!("Class {name} cannot extend enum {pname}");
                return fail(self, 64, msg);
            } else if pflags.is_final && !pname_raw.windows(11).any(|w| w == b"@anonymous\0") {
                // (rhp marks anonymous classes final; php does not.)
                let msg = format!("Class {name} cannot extend final class {pname}");
                return fail(self, 64, msg);
            } else if matches!(pkind, ClassKind::Interface | ClassKind::Trait) {
                let what = if pkind == ClassKind::Interface {
                    "interface"
                } else {
                    "trait"
                };
                let msg = format!("Class {name} cannot extend {what} {pname}");
                return fail(self, 64, msg);
            }
            if proto.flags.is_readonly != pflags.is_readonly {
                let msg = format!(
                    "{} class {name} cannot extend {} class {pname}",
                    if proto.flags.is_readonly {
                        "Readonly"
                    } else {
                        "Non-readonly"
                    },
                    if pflags.is_readonly {
                        "readonly"
                    } else {
                        "non-readonly"
                    },
                );
                return fail(self, 64, msg);
            }
        }
        // zend_do_implement_interfaces: each declared interface must be one,
        // and not one listed before (an inherited one is skipped).
        let parent_ifaces: Vec<ClassId> = proto
            .parent
            .as_ref()
            .and_then(|p| class_of(self, p))
            .map(|p| self.classes[p].interfaces.clone())
            .unwrap_or_default();
        let mut listed: Vec<ClassId> = Vec::new();
        for i in &proto.interfaces {
            let Some(iid) = class_of(self, i) else {
                continue;
            };
            let iname = shown(&self.classes[iid].name);
            if self.classes[iid].kind != ClassKind::Interface {
                return fail(
                    self,
                    1,
                    format!("{name} cannot implement {iname} - it is not an interface"),
                );
            }
            let iid = iid as ClassId;
            if listed.contains(&iid) {
                let msg = format!(
                    "{} {name} cannot implement previously implemented interface {iname}",
                    kind_uc(proto.kind)
                );
                return fail(self, 64, msg);
            }
            if !parent_ifaces.contains(&iid) {
                listed.push(iid);
            }
        }
        // interface_gets_implemented of UnitEnum / BackedEnum, for every
        // interface the class ends up implementing.
        if proto.kind != ClassKind::Interface {
            let mut ifaces: Vec<usize> = Vec::new();
            for i in &proto.interfaces {
                if let Some(iid) = class_of(self, i) {
                    ifaces.push(iid);
                    ifaces.extend(self.classes[iid].ancestors.iter().map(|&a| a as usize));
                }
            }
            for iid in ifaces {
                let iname = self.classes[iid].name.as_bytes();
                let unit = iname.eq_ignore_ascii_case(b"UnitEnum");
                let backed = iname.eq_ignore_ascii_case(b"BackedEnum");
                if !(unit || backed) {
                    continue;
                }
                let iname = String::from_utf8_lossy(iname).into_owned();
                if !is_enum {
                    let msg = format!("Non-enum class {name} cannot implement interface {iname}");
                    return fail(self, 1, msg);
                }
                if backed && proto.enum_backing.is_none() {
                    let msg = format!("Non-backed enum {name} cannot implement interface {iname}");
                    return fail(self, 1, msg);
                }
            }
        }
        // zend_verify_enum: properties (a trait's), then magic methods.
        if is_enum {
            let trait_props = proto.traits.iter().any(|t| {
                class_of(self, t).is_some_and(|tid| {
                    !self.classes[tid].props.is_empty()
                        || !self.classes[tid].static_props.is_empty()
                })
            });
            if trait_props {
                return fail(self, 64, format!("Enum {name} cannot include properties"));
            }
            let mut names: Vec<Vec<u8>> = proto
                .methods
                .iter()
                .map(|m| lc(m.name.as_bytes()))
                .collect();
            for t in &proto.traits {
                if let Some(tid) = class_of(self, t) {
                    names.extend(self.classes[tid].methods.keys().cloned());
                }
            }
            for m in ENUM_FORBIDDEN_METHODS {
                if names.contains(&lc(m.as_bytes())) {
                    let msg = format!("Enum {name} cannot include magic method {m}");
                    return fail(self, 64, msg);
                }
            }
        }
        Ok(())
    }

    /// zend_inheritance_check_override: a method, property or hook marked
    /// `#[\Override]` must override (or implement) a parent or interface
    /// member. Traits are checked where they are used.
    pub(crate) fn check_override(&mut self, ce: ClassId) -> Result<(), Thrown> {
        let e = &self.classes[ce as usize];
        if e.kind == ClassKind::Trait || e.decl_file.is_empty() {
            return Ok(());
        }
        let parent = e.parent.map(|p| &self.classes[p as usize]);
        let new_ifaces: Vec<ClassId> = e
            .interfaces
            .iter()
            .copied()
            .filter(|i| parent.is_none_or(|p| !p.interfaces.contains(i)))
            .collect();
        // Methods: do_inheritance_check_on_method resets the flag unless the
        // parent method is private or a constructor without an abstract
        // prototype.
        for (k, m) in &e.methods {
            if m.defining_class != ce || !has_override(&m.proto.meta.attributes) {
                continue;
            }
            let by_parent = parent.and_then(|p| p.methods.get(k)).is_some_and(|pm| {
                if k == b"__construct" {
                    let pc = pm.prototype.unwrap_or(pm.defining_class);
                    self.classes[pc as usize]
                        .methods
                        .get(k)
                        .is_some_and(|m| m.is_abstract)
                } else {
                    pm.visibility != Visibility::Private || pm.is_abstract
                }
            });
            let by_iface = || {
                new_ifaces
                    .iter()
                    .any(|&i| self.classes[i as usize].methods.contains_key(k))
            };
            let by_trait = || {
                e.traits.iter().any(|&t| {
                    self.classes[t as usize]
                        .methods
                        .get(k)
                        .is_some_and(|tm| tm.is_abstract && !std::rc::Rc::ptr_eq(&tm.proto, &m.proto))
                })
            };
            if by_parent || by_iface() || by_trait() {
                continue;
            }
            let msg = format!(
                "{}::{}() has #[\\Override] attribute, but no matching parent method exists",
                shown(&e.name),
                String::from_utf8_lossy(m.proto.name.as_bytes())
            );
            let file = m.proto.debug.file.as_bytes().to_vec();
            let line = m.proto.meta.start_line as i64;
            return Err(self.fatal_compile_error_at(msg.as_bytes(), file, line));
        }
        // The non-private parent or interface property of that name.
        let inherited = |name: &PhpStr| -> Option<&PropEntry> {
            parent
                .and_then(|p| p.prop_visible.get(name).and_then(|k| p.props.get(k)))
                .filter(|pp| pp.visibility != Visibility::Private)
                .or_else(|| {
                    new_ifaces.iter().find_map(|&i| {
                        let ie = &self.classes[i as usize];
                        ie.prop_visible.get(name).and_then(|k| ie.props.get(k))
                    })
                })
        };
        // The property declarations the class owns: its own, then its
        // traits'.
        let decl_attrs = |name: &[u8], is_static: bool| -> Option<bool> {
            let own = |p: &ClassProto| {
                if is_static {
                    p.static_props
                        .iter()
                        .find(|sp| sp.name.as_bytes() == name)
                        .map(|sp| has_override(&sp.meta.attributes))
                } else {
                    p.props
                        .iter()
                        .find(|pp| pp.name.as_bytes() == name)
                        .map(|pp| has_override(&pp.meta.attributes))
                }
            };
            own(&e.proto).or_else(|| {
                e.traits
                    .iter()
                    .find_map(|&t| own(&self.classes[t as usize].proto))
            })
        };
        for pe in e.props.values() {
            if pe.declaring_class != ce {
                continue;
            }
            let inh = inherited(&pe.name);
            if decl_attrs(pe.name.as_bytes(), false) == Some(true) && inh.is_none() {
                return Err(self.override_prop_error(ce, &pe.name.clone()));
            }
            let Some(hooks) = &pe.hooks else { continue };
            for (kind, h) in hooks.iter().enumerate() {
                let Some(h) = h else { continue };
                if h.defining_class != ce
                    || !has_override(&h.proto.meta.attributes)
                    || inh.is_some_and(|p| has_operation(p, kind))
                {
                    continue;
                }
                let msg = format!(
                    "{}::{}() has #[\\Override] attribute, but no matching parent method exists",
                    shown(&e.name),
                    String::from_utf8_lossy(h.proto.name.as_bytes())
                );
                let file = h.proto.debug.file.as_bytes().to_vec();
                let line = h.proto.meta.start_line as i64;
                return Err(self.fatal_compile_error_at(msg.as_bytes(), file, line));
            }
        }
        for (k, sp) in &e.static_props {
            if sp.declaring_class != ce || decl_attrs(k, true) != Some(true) {
                continue;
            }
            let ok = parent
                .and_then(|p| p.static_props.get(k))
                .is_some_and(|ps| ps.visibility != Visibility::Private);
            if !ok {
                return Err(self.override_prop_error(ce, &PhpStr::from(k.as_slice())));
            }
        }
        Ok(())
    }

    fn override_prop_error(&mut self, ce: ClassId, name: &PhpStr) -> Thrown {
        let msg = format!(
            "{}::${} has #[\\Override] attribute, but no matching parent property exists",
            shown(&self.classes[ce as usize].name),
            String::from_utf8_lossy(name.as_bytes())
        );
        let proto = self.classes[ce as usize].proto.clone();
        self.link_fatal(&proto, 64, &msg)
    }

    /// The interface_gets_implemented handlers (Traversable, Iterator,
    /// IteratorAggregate, Throwable, DateTimeInterface) for each interface
    /// the just-linked user class `ce` implements, in its interface order.
    pub(crate) fn interface_handlers(&mut self, ce: ClassId) -> Result<(), Thrown> {
        let e = &self.classes[ce as usize];
        if e.kind == ClassKind::Interface || e.decl_file.is_empty() {
            return Ok(());
        }
        let name = shown(&e.name);
        let has = |n: &[u8]| {
            e.interfaces
                .iter()
                .any(|&i| self.classes[i as usize].name.as_bytes().eq_ignore_ascii_case(n))
        };
        let extends = |n: &[u8]| {
            let mut c = Some(ce);
            while let Some(id) = c {
                if self.classes[id as usize].name.as_bytes().eq_ignore_ascii_case(n) {
                    return true;
                }
                c = self.classes[id as usize].parent;
            }
            false
        };
        let mut err: Option<(i64, String)> = None;
        for &i in &e.interfaces {
            let iname = self.classes[i as usize].name.as_bytes().to_ascii_lowercase();
            match iname.as_slice() {
                b"traversable" => {
                    if !e.flags.is_abstract && !has(b"Iterator") && !has(b"IteratorAggregate") {
                        err = Some((
                            16,
                            format!(
                                "{} {name} must implement interface Traversable as part of either Iterator or IteratorAggregate",
                                kind_uc(e.kind)
                            ),
                        ));
                    }
                }
                b"iterator" | b"iteratoraggregate" => {
                    let other: &[u8] = if iname == b"iterator" {
                        b"IteratorAggregate"
                    } else {
                        b"Iterator"
                    };
                    if has(other) {
                        err = Some((
                            1,
                            format!("Class {name} cannot implement both Iterator and IteratorAggregate at the same time"),
                        ));
                    }
                }
                b"throwable" => {
                    let mut root = ce;
                    while let Some(p) = self.classes[root as usize].parent {
                        root = p;
                    }
                    let rn = self.classes[root as usize].name.as_bytes();
                    if rn != b"Exception" && rn != b"Error" {
                        let tail = if e.kind == ClassKind::Enum {
                            ""
                        } else {
                            ", extend Exception or Error instead"
                        };
                        err = Some((
                            1,
                            format!(
                                "{} {name} cannot implement interface Throwable{tail}",
                                kind_uc(e.kind)
                            ),
                        ));
                    }
                }
                b"datetimeinterface" => {
                    if !extends(b"DateTime") && !extends(b"DateTimeImmutable") {
                        err = Some((
                            1,
                            "DateTimeInterface can't be implemented by user classes".to_string(),
                        ));
                    }
                }
                _ => {}
            }
            if err.is_some() {
                break;
            }
        }
        let Some((level, msg)) = err else {
            return Ok(());
        };
        if level == 16 {
            // E_CORE_ERROR: no file or line.
            return Err(self.fatal_error_at(16, msg.as_bytes(), b"Unknown".to_vec(), 0));
        }
        let proto = e.proto.clone();
        Err(self.link_fatal(&proto, level, &msg))
    }
}
