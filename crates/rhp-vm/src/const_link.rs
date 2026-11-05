//! Typed class constants at link time: php's do_inherit_constant_check and
//! do_trait_constant_check type rules (zend_inheritance.c).
//!
//! The covariance test here is a small one for constant types (no
//! `static`, `callable`, `void` or `never` can occur); the method LSP check
//! should take it over once it exists.

use rhp_bytecode::{ClassProto, TypeHint, TypeKind as K};
use rhp_values::{ClassId, PhpStr, Thrown};

use crate::Vm;

fn lc(b: &[u8]) -> Vec<u8> {
    b.to_ascii_lowercase()
}

impl Vm {
    /// The type checks a class's constants must pass against the ones it
    /// inherits (parent, then traits, then interfaces), before linking.
    pub(crate) fn check_const_types(
        &mut self,
        proto: &ClassProto,
        parent: Option<ClassId>,
    ) -> Result<(), Thrown> {
        let own = |name: &[u8]| proto.constants.iter().find(|c| c.name.as_bytes() == name);
        let cname = proto.name.clone();
        // Inherited from the parent class and interfaces.
        let mut sources: Vec<ClassId> = parent.into_iter().collect();
        for i in &proto.interfaces {
            if let Some(&iid) = self.class_ids.get(&lc(i.as_bytes())) {
                sources.push(iid);
            }
        }
        // (constant name, parent declaring class, parent type)
        let mut inherited: Vec<(Vec<u8>, ClassId, TypeHint)> = Vec::new();
        for src in &sources {
            for (k, c) in &self.classes[*src as usize].constants {
                if c.visibility == rhp_values::Visibility::Private {
                    continue;
                }
                if let Some(t) = &c.ty {
                    inherited.push((k.clone(), c.declaring_class, (**t).clone()));
                }
            }
        }
        // Trait constants against the class's own (or inherited) ones.
        for (ti, tname) in proto.traits.iter().enumerate() {
            let Some(&tid) = self.class_ids.get(&lc(tname.as_bytes())) else {
                continue;
            };
            let consts: Vec<(Vec<u8>, Option<TypeHint>)> = self.classes[tid as usize]
                .constants
                .iter()
                .map(|(k, c)| (k.clone(), c.ty.as_deref().cloned()))
                .collect();
            for (k, tty) in consts {
                let existing: Option<(PhpStr, Option<TypeHint>)> = match own(&k) {
                    Some(c) => Some((cname.clone(), c.type_hint.clone())),
                    None => parent.and_then(|p| {
                        let c = self.classes[p as usize].constants.get(&k)?;
                        let decl = self.classes[c.declaring_class as usize].name.clone();
                        Some((decl, c.ty.as_deref().cloned()))
                    }),
                };
                let Some((owner, ety)) = existing else {
                    continue;
                };
                let ok = match (&ety, &tty) {
                    (None, None) => true,
                    (Some(e), Some(t)) => self.type_subsumes(e, t) && self.type_subsumes(t, e),
                    _ => false,
                };
                if !ok {
                    // find_first_constant_definition
                    let first = if owner.as_bytes() == cname.as_bytes() {
                        proto.traits[..ti]
                            .iter()
                            .find(|t| {
                                self.class_ids.get(&lc(t.as_bytes())).is_some_and(|&id| {
                                    self.classes[id as usize].constants.contains_key(&k)
                                })
                            })
                            .cloned()
                            .unwrap_or(owner)
                    } else {
                        owner
                    };
                    let tdecl = self.classes[tid as usize].name.clone();
                    let msg = format!(
                        "{} and {} define the same constant ({}) in the composition of {}. However, the definition differs and is considered incompatible. Class was composed",
                        String::from_utf8_lossy(first.as_bytes()),
                        String::from_utf8_lossy(tdecl.as_bytes()),
                        String::from_utf8_lossy(&k),
                        String::from_utf8_lossy(cname.as_bytes()),
                    );
                    return Err(self.fatal_link_error(proto, &msg));
                }
            }
        }
        for (k, pdecl, pty) in inherited {
            let Some(child) = own(&k) else {
                continue;
            };
            let ok = child
                .type_hint
                .as_ref()
                .is_some_and(|ct| self.type_subsumes(ct, &pty));
            if !ok {
                let msg = format!(
                    "Type of {}::{} must be compatible with {}::{} of type {}",
                    String::from_utf8_lossy(cname.as_bytes()),
                    String::from_utf8_lossy(&k),
                    String::from_utf8_lossy(self.classes[pdecl as usize].name.as_bytes()),
                    String::from_utf8_lossy(&k),
                    self.type_string_resolved(&pty, Some(pdecl), None),
                );
                return Err(self.fatal_link_error(proto, &msg));
            }
        }
        Ok(())
    }

    /// A compile error raised while linking `proto`: at its declaration.
    fn fatal_link_error(&mut self, proto: &ClassProto, msg: &str) -> Thrown {
        self.fatal_compile_error_at(
            msg.as_bytes(),
            proto.decl_file.as_bytes().to_vec(),
            proto.decl_line as i64,
        )
    }

    /// Can php early-bind `proto` as far as its typed constants go? Not
    /// while a class their compatibility depends on is not loaded (php
    /// then declares the class when its declaration runs).
    pub(crate) fn const_types_resolvable(&self, proto: &ClassProto) -> bool {
        let Some(parent) = proto
            .parent
            .as_ref()
            .and_then(|p| self.class_ids.get(&lc(p.as_bytes())))
        else {
            return true;
        };
        let loaded = |t: &TypeHint| {
            t.kinds
                .iter()
                .filter_map(named)
                .chain(t.groups.iter().flatten().cloned())
                .all(|n| self.class_ids.contains_key(&lc(n.as_bytes())))
        };
        proto.constants.iter().all(|c| {
            let Some(pc) = self.classes[*parent as usize]
                .constants
                .get(c.name.as_bytes())
            else {
                return true;
            };
            pc.ty.as_deref().is_none_or(loaded) && c.type_hint.as_ref().is_none_or(loaded)
        })
    }

    /// Is every value of type `child` a value of type `parent`?
    fn type_subsumes(&self, child: &TypeHint, parent: &TypeHint) -> bool {
        let phas = |k: &K| parent.kinds.contains(k);
        if phas(&K::Mixed) {
            return true;
        }
        if child.kinds.contains(&K::Mixed) {
            return false;
        }
        if child.nullable && !parent.nullable {
            return false;
        }
        if child.intersection {
            let names: Vec<PhpStr> = child.kinds.iter().filter_map(named).collect();
            return self.group_subsumed(&names, parent);
        }
        if !child.groups.iter().all(|g| self.group_subsumed(g, parent)) {
            return false;
        }
        child.kinds.iter().all(|k| match k {
            K::Int | K::Float | K::String | K::Object | K::Callable | K::Void | K::Never => phas(k),
            K::Array => phas(&K::Array) || phas(&K::Iterable),
            K::Bool => phas(&K::Bool) || (phas(&K::True) && phas(&K::False)),
            K::True | K::False => phas(k) || phas(&K::Bool),
            K::Null => parent.nullable,
            K::Iterable => {
                phas(&K::Iterable)
                    || ((phas(&K::Array)) && self.class_subsumed(b"Traversable", parent))
            }
            K::StaticType => phas(&K::StaticType) || phas(&K::Object),
            K::Named(n) => self.class_subsumed(n.as_bytes(), parent),
            K::Mixed | K::SelfType | K::Parent => false,
        })
    }

    /// Is the class `name` a subtype of `parent`?
    fn class_subsumed(&self, name: &[u8], parent: &TypeHint) -> bool {
        if parent.kinds.contains(&K::Object) {
            return true;
        }
        if parent.intersection {
            return parent
                .kinds
                .iter()
                .filter_map(named)
                .all(|p| self.class_is_a(name, p.as_bytes()));
        }
        if parent.kinds.contains(&K::Iterable) && self.class_is_a(name, b"Traversable") {
            return true;
        }
        parent
            .kinds
            .iter()
            .filter_map(named)
            .any(|p| self.class_is_a(name, p.as_bytes()))
            || parent
                .groups
                .iter()
                .any(|g| g.iter().all(|p| self.class_is_a(name, p.as_bytes())))
    }

    /// Is an intersection of `names` a subtype of `parent`?
    fn group_subsumed(&self, names: &[PhpStr], parent: &TypeHint) -> bool {
        if parent.kinds.contains(&K::Object) {
            return true;
        }
        let one_is = |p: &PhpStr| {
            names
                .iter()
                .any(|n| self.class_is_a(n.as_bytes(), p.as_bytes()))
        };
        if parent.intersection {
            return parent.kinds.iter().filter_map(named).all(|p| one_is(&p));
        }
        parent.kinds.iter().filter_map(named).any(|p| one_is(&p))
            || parent.groups.iter().any(|g| g.iter().all(one_is))
    }

    /// `name` is `other` or a (loaded) descendant of it.
    fn class_is_a(&self, name: &[u8], other: &[u8]) -> bool {
        if name.eq_ignore_ascii_case(other) {
            return true;
        }
        match (
            self.resolve_class_by_name(name),
            self.resolve_class_by_name(other),
        ) {
            (Ok(a), Ok(b)) => self.classes[a as usize].ancestors.contains(&b),
            _ => false,
        }
    }
}

fn named(k: &K) -> Option<PhpStr> {
    match k {
        K::Named(n) => Some(n.clone()),
        _ => None,
    }
}
