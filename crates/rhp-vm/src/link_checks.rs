//! The declaration errors php raises while linking a class
//! (zend_do_link_class): enum rules (zend_enum.c), what a class may extend,
//! readonly class inheritance and trait use. Each is a fatal error at the
//! class declaration's line.

use rhp_bytecode::ClassProto;
use rhp_values::{ClassKind, Thrown};

use crate::Vm;

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
                    .map(|p| String::from_utf8_lossy(p.name.as_bytes()).into_owned());
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
            let pname = String::from_utf8_lossy(parent.name.as_bytes()).into_owned();
            let (pkind, pflags) = (parent.kind, parent.flags);
            if proto.kind == ClassKind::Interface {
                if pkind != ClassKind::Interface {
                    let msg = format!("Interface {name} cannot extend class {pname}");
                    return fail(self, 64, msg);
                }
            } else if pkind == ClassKind::Enum {
                let msg = format!("Class {name} cannot extend enum {pname}");
                return fail(self, 64, msg);
            } else if pflags.is_final {
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
}
