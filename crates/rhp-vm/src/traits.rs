//! Trait method binding (zend_do_bind_traits): resolving the `use T { ... }`
//! rules (zend_traits_init_trait_structures), then copying each trait's
//! methods with their aliases and exclusions (zend_traits_copy_functions,
//! zend_add_trait_method).

use std::rc::Rc;

use indexmap::IndexMap;
use rhp_bytecode::{
    ClassProto, ConstInitProto, FunctionProto, PropDefaultProto, TraitRules, TypeHint,
};
use rhp_values::{ClassId, ClassKind, PhpStr, Thrown, Value, Visibility};
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};

use crate::classes::ResolvedMethod;
use crate::Vm;

/// The trait methods a class gets, before the parent's are inherited.
#[derive(Default)]
pub(crate) struct TraitMethods {
    /// Concrete methods (lc name => copy), in php's order. The copies keep
    /// the trait as their defining class.
    pub concrete: IndexMap<Vec<u8>, ResolvedMethod, FxBuildHasher>,
    /// Abstract methods with their aliases, in order: added (or checked
    /// against what is there) after the parent's methods are inherited.
    pub abstracts: Vec<(Vec<u8>, ResolvedMethod)>,
}

/// The resolved rules: per used trait the excluded method names, per
/// alias the trait it applies to.
struct Resolved {
    excludes: Vec<FxHashSet<Vec<u8>>>,
    alias_traits: Vec<ClassId>,
}

fn lc(b: &[u8]) -> Vec<u8> {
    b.to_ascii_lowercase()
}

fn lossy(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// A copy of `m` under another name (php's copy with a new
/// `function_name`); the copy keeps its own static variables.
fn renamed(m: &ResolvedMethod, name: &PhpStr) -> ResolvedMethod {
    if m.proto.name.as_bytes() == name.as_bytes() {
        return m.clone();
    }
    let proto = FunctionProto {
        name: name.clone(),
        ..(*m.proto).clone()
    };
    ResolvedMethod {
        proto: Rc::new(proto),
        ..m.clone()
    }
}

impl Vm {
    /// The used traits, a repeated one left out (`None`), as php's
    /// `traits_and_interfaces`.
    pub(crate) fn used_traits(&self, proto: &ClassProto) -> Vec<Option<ClassId>> {
        let mut out: Vec<Option<ClassId>> = Vec::with_capacity(proto.traits.len());
        for t in &proto.traits {
            let id = self.class_ids[&lc(t.as_bytes())];
            out.push((!out.contains(&Some(id))).then_some(id));
        }
        out
    }

    /// zend_check_trait_usage: the index of `t` among the used traits.
    fn trait_usage(
        &mut self,
        proto: &ClassProto,
        traits: &[Option<ClassId>],
        t: ClassId,
    ) -> Result<usize, Thrown> {
        let e = &self.classes[t as usize];
        if e.kind != ClassKind::Trait {
            let msg = format!(
                "Class {} is not a trait, Only traits may be used in 'as' and 'insteadof' statements",
                lossy(e.name.as_bytes())
            );
            return Err(self.link_fatal(proto, 64, &msg));
        }
        if let Some(i) = traits.iter().position(|&x| x == Some(t)) {
            return Ok(i);
        }
        let msg = format!(
            "Required Trait {} wasn't added to {}",
            lossy(e.name.as_bytes()),
            lossy(proto.name.as_bytes())
        );
        Err(self.link_fatal(proto, 64, &msg))
    }

    /// A trait named in a rule; php does not autoload it here.
    fn rule_trait(&mut self, proto: &ClassProto, name: &PhpStr) -> Result<ClassId, Thrown> {
        match self.class_ids.get(&lc(name.as_bytes())) {
            Some(&id) => Ok(id),
            None => {
                let msg = format!("Could not find trait {}", lossy(name.as_bytes()));
                Err(self.link_fatal(proto, 64, &msg))
            }
        }
    }

    /// zend_traits_init_trait_structures.
    fn resolve_trait_rules(
        &mut self,
        proto: &ClassProto,
        rules: &TraitRules,
        traits: &[Option<ClassId>],
    ) -> Result<Resolved, Thrown> {
        let mut excludes = vec![FxHashSet::default(); traits.len()];
        for p in &rules.precedences {
            let t = self.rule_trait(proto, &p.trait_name)?;
            self.trait_usage(proto, traits, t)?;
            let key = lc(p.method.as_bytes());
            if !self.classes[t as usize].methods.contains_key(&key) {
                let msg = format!(
                    "A precedence rule was defined for {}::{} but this method does not exist",
                    lossy(self.classes[t as usize].name.as_bytes()),
                    lossy(p.method.as_bytes())
                );
                return Err(self.link_fatal(proto, 64, &msg));
            }
            for x in &p.excludes {
                let xt = self.rule_trait(proto, x)?;
                let n = self.trait_usage(proto, traits, xt)?;
                if !excludes[n].insert(key.clone()) {
                    let msg = format!(
                        "Failed to evaluate a trait precedence ({}). Method of trait {} was defined to be excluded multiple times",
                        lossy(p.method.as_bytes()),
                        lossy(self.classes[xt as usize].name.as_bytes())
                    );
                    return Err(self.link_fatal(proto, 64, &msg));
                }
                if t == xt {
                    let tn = lossy(self.classes[t as usize].name.as_bytes());
                    let msg = format!(
                        "Inconsistent insteadof definition. The method {} is to be used from {tn}, but {tn} is also on the exclude list",
                        lossy(p.method.as_bytes())
                    );
                    return Err(self.link_fatal(proto, 64, &msg));
                }
            }
        }
        let mut alias_traits = Vec::with_capacity(rules.aliases.len());
        for al in &rules.aliases {
            let key = lc(al.method.as_bytes());
            let t = match &al.trait_name {
                Some(n) => {
                    let t = self.rule_trait(proto, n)?;
                    self.trait_usage(proto, traits, t)?;
                    if !self.classes[t as usize].methods.contains_key(&key) {
                        let msg = format!(
                            "An alias was defined for {}::{} but this method does not exist",
                            lossy(self.classes[t as usize].name.as_bytes()),
                            lossy(al.method.as_bytes())
                        );
                        return Err(self.link_fatal(proto, 64, &msg));
                    }
                    t
                }
                None => {
                    let mut found: Option<ClassId> = None;
                    for &t in traits.iter().flatten() {
                        if !self.classes[t as usize].methods.contains_key(&key) {
                            continue;
                        }
                        if let Some(f) = found {
                            let (a, b) = (
                                lossy(self.classes[f as usize].name.as_bytes()),
                                lossy(self.classes[t as usize].name.as_bytes()),
                            );
                            let m = lossy(al.method.as_bytes());
                            let msg = format!(
                                "An alias was defined for method {m}(), which exists in both {a} and {b}. Use {a}::{m} or {b}::{m} to resolve the ambiguity"
                            );
                            return Err(self.link_fatal(proto, 64, &msg));
                        }
                        found = Some(t);
                    }
                    match found {
                        Some(t) => t,
                        None => {
                            let m = lossy(al.method.as_bytes());
                            let msg = match &al.alias {
                                Some(a) => format!(
                                    "An alias ({}) was defined for method {m}(), but this method does not exist",
                                    lossy(a.as_bytes())
                                ),
                                None => format!(
                                    "The modifiers of the trait method {m}() are changed, but this method does not exist. Error"
                                ),
                            };
                            return Err(self.link_fatal(proto, 64, &msg));
                        }
                    }
                }
            };
            alias_traits.push(t);
        }
        Ok(Resolved {
            excludes,
            alias_traits,
        })
    }

    /// The trait methods of `proto` (its used traits `traits`): php's
    /// zend_do_traits_method_binding for the concrete methods, with the
    /// collision errors, and the list of abstract ones for later.
    pub(crate) fn trait_methods(
        &mut self,
        proto: &ClassProto,
        traits: &[Option<ClassId>],
    ) -> Result<TraitMethods, Thrown> {
        let mut out = TraitMethods::default();
        if traits.is_empty() {
            return Ok(out);
        }
        let empty = TraitRules::default();
        let rules = proto.trait_rules.as_deref().unwrap_or(&empty);
        let resolved = self.resolve_trait_rules(proto, rules, traits)?;
        let own: FxHashSet<Vec<u8>> = proto
            .methods
            .iter()
            .map(|m| lc(m.name.as_bytes()))
            .collect();
        for (i, &t) in traits.iter().enumerate() {
            let Some(t) = t else { continue };
            let methods: Vec<(Vec<u8>, ResolvedMethod)> = self.classes[t as usize]
                .methods
                .iter()
                .map(|(k, m)| (k.clone(), m.clone()))
                .collect();
            for (key, m) in methods {
                // An enum's engine methods come first: the class's own
                // members win over a trait's.
                let engine = matches!(key.as_slice(), b"cases")
                    || (proto.enum_backing.is_some()
                        && matches!(key.as_slice(), b"from" | b"tryfrom"));
                if proto.kind == ClassKind::Enum && engine {
                    continue;
                }
                for (name, k, copy) in
                    self.copy_trait_function(proto, rules, &resolved, i, &key, &m)?
                {
                    if copy.is_abstract {
                        out.abstracts.push((k, renamed(&copy, &name)));
                    } else {
                        self.add_trait_method(proto, &own, &mut out.concrete, name, k, copy)?;
                    }
                }
            }
        }
        Ok(out)
    }

    /// zend_traits_copy_functions: the (name, lc name, copy) entries trait
    /// method `m` (lc `key`, of used trait number `i`) adds: one per alias,
    /// then itself unless excluded. Modifiers from the rules are applied.
    fn copy_trait_function(
        &mut self,
        proto: &ClassProto,
        rules: &TraitRules,
        resolved: &Resolved,
        i: usize,
        key: &[u8],
        m: &ResolvedMethod,
    ) -> Result<Vec<(PhpStr, Vec<u8>, ResolvedMethod)>, Thrown> {
        let mut out = Vec::new();
        let applies = |j: usize| {
            m.defining_class == resolved.alias_traits[j]
                && rules.aliases[j].method.as_bytes().eq_ignore_ascii_case(key)
        };
        let modify = |copy: &mut ResolvedMethod, j: usize| {
            let al = &rules.aliases[j];
            if let Some(v) = al.visibility {
                copy.visibility = v;
            }
            copy.is_final |= al.is_final;
        };
        for (j, al) in rules.aliases.iter().enumerate() {
            let Some(alias) = &al.alias else { continue };
            if !applies(j) {
                continue;
            }
            let mut copy = m.clone();
            modify(&mut copy, j);
            self.private_final_warning(proto, m, &copy, alias.as_bytes());
            out.push((alias.clone(), lc(alias.as_bytes()), copy));
        }
        if !resolved.excludes[i].contains(key) {
            let mut copy = m.clone();
            for (j, al) in rules.aliases.iter().enumerate() {
                if al.alias.is_none() && (al.visibility.is_some() || al.is_final) && applies(j) {
                    modify(&mut copy, j);
                }
            }
            self.private_final_warning(proto, m, &copy, m.proto.name.as_bytes());
            out.push((m.proto.name.clone(), key.to_vec(), copy));
        }
        Ok(out)
    }

    /// zend_traits_check_private_final_inheritance.
    fn private_final_warning(
        &mut self,
        proto: &ClassProto,
        orig: &ResolvedMethod,
        copy: &ResolvedMethod,
        name: &[u8],
    ) {
        if !orig.is_final
            && copy.is_final
            && copy.visibility == Visibility::Private
            && !name.eq_ignore_ascii_case(b"__construct")
        {
            self.raise_error_at(
                rhp_values::E_COMPILE_WARNING,
                b"Private methods cannot be final as they are never overridden by other classes",
                proto.decl_file.as_bytes().to_vec(),
                proto.decl_line as i64,
            );
        }
    }

    /// zend_add_trait_method for a concrete method: the class's own method
    /// wins; two traits may not both bring a method of that name (unless it
    /// is the same one).
    fn add_trait_method(
        &mut self,
        proto: &ClassProto,
        own: &FxHashSet<Vec<u8>>,
        table: &mut IndexMap<Vec<u8>, ResolvedMethod, FxBuildHasher>,
        name: PhpStr,
        key: Vec<u8>,
        m: ResolvedMethod,
    ) -> Result<(), Thrown> {
        if let Some(ex) = table.get(&key) {
            if Rc::ptr_eq(&ex.proto, &m.proto) && ex.visibility == m.visibility {
                return Ok(());
            }
        }
        if own.contains(&key) {
            return Ok(());
        }
        if let Some(ex) = table.get(&key) {
            let msg = format!(
                "Trait method {}::{} has not been applied as {}::{}, because of collision with {}::{}",
                lossy(self.classes[m.defining_class as usize].name.as_bytes()),
                lossy(m.proto.name.as_bytes()),
                lossy(proto.name.as_bytes()),
                lossy(name.as_bytes()),
                lossy(self.classes[ex.defining_class as usize].name.as_bytes()),
                lossy(ex.proto.name.as_bytes()),
            );
            return Err(self.link_fatal(proto, 64, &msg));
        }
        table.insert(key, renamed(&m, &name));
        Ok(())
    }
}

/// What do_trait_constant_check compares of one constant: visibility,
/// final, type and value (`None` when it is a constant expression).
type ConstDesc = (Visibility, bool, Option<TypeHint>, Option<Value>);

/// What zend_do_traits_property_binding compares of one property.
struct PropDesc {
    visibility: Visibility,
    is_static: bool,
    readonly: bool,
    ty: Option<TypeHint>,
    /// The default; `None` when it is a constant expression (not compared).
    default: Option<Option<Value>>,
    hooked: bool,
}

/// check_trait_property_or_constant_value_compatibility. A constant
/// expression is not evaluated here; it counts as compatible.
fn same_value(a: &Option<Option<Value>>, b: &Option<Option<Value>>) -> bool {
    match (a, b) {
        (Some(None), Some(None)) => true,
        (Some(Some(x)), Some(Some(y))) => rhp_values::ops::strict_eq(x, y),
        (Some(_), Some(_)) => false,
        _ => true,
    }
}

impl Vm {
    /// zend_do_traits_constant_binding's checks: a trait constant the class
    /// (or an earlier trait) already has must be declared the same way.
    pub(crate) fn check_trait_constants(
        &mut self,
        proto: &ClassProto,
        traits: &[Option<ClassId>],
    ) -> Result<(), Thrown> {
        // name => (visibility, final, type, value), the class's own first.
        let mut seen: FxHashMap<Vec<u8>, ConstDesc> = FxHashMap::default();
        for c in &proto.constants {
            let v = match &c.value {
                ConstInitProto::Const(v) => Some(v.clone()),
                ConstInitProto::Thunk(_) => None,
            };
            seen.insert(
                c.name.as_bytes().to_vec(),
                (c.visibility, c.is_final, c.type_hint.clone(), v),
            );
        }
        for (i, &t) in traits.iter().enumerate() {
            let Some(t) = t else { continue };
            let consts: Vec<(Vec<u8>, ConstDesc)> = self.classes[t as usize]
                .constants
                .iter()
                .map(|(k, c)| {
                    let v = c.init.is_none().then(|| c.value.clone());
                    (
                        k.clone(),
                        (c.visibility, c.is_final, c.ty.as_deref().cloned(), v),
                    )
                })
                .collect();
            for (k, d) in consts {
                let Some(ex) = seen.get(&k) else {
                    seen.insert(k, d);
                    continue;
                };
                let same_val = match (&ex.3, &d.3) {
                    (Some(a), Some(b)) => rhp_values::ops::strict_eq(a, b),
                    _ => true,
                };
                if ex.0 == d.0 && ex.1 == d.1 && ex.2 == d.2 && same_val {
                    continue;
                }
                let first = self.first_trait_with(traits, i, |e| e.constants.contains_key(&k));
                let msg = format!(
                    "{} and {} define the same constant ({}) in the composition of {}. However, the definition differs and is considered incompatible. Class was composed",
                    first.unwrap_or_else(|| lossy(proto.name.as_bytes())),
                    lossy(self.classes[t as usize].name.as_bytes()),
                    lossy(&k),
                    lossy(proto.name.as_bytes()),
                );
                return Err(self.link_fatal(proto, 64, &msg));
            }
        }
        Ok(())
    }

    /// find_first_constant_definition / find_first_property_definition:
    /// the first of the traits before number `i` that has the member.
    fn first_trait_with(
        &self,
        traits: &[Option<ClassId>],
        i: usize,
        has: impl Fn(&crate::classes::ClassEntry) -> bool,
    ) -> Option<String> {
        traits[..i]
            .iter()
            .flatten()
            .find(|&&t| has(&self.classes[t as usize]))
            .map(|&t| lossy(self.classes[t as usize].name.as_bytes()))
    }

    /// zend_do_traits_property_binding's checks: a trait property the class
    /// (or an earlier trait) already declares must be declared the same way.
    pub(crate) fn check_trait_props(
        &mut self,
        proto: &ClassProto,
        traits: &[Option<ClassId>],
    ) -> Result<(), Thrown> {
        let mut seen: FxHashMap<Vec<u8>, PropDesc> = FxHashMap::default();
        for pp in &proto.props {
            let default = match &pp.default {
                PropDefaultProto::Const(v) => Some(Some(v.clone())),
                PropDefaultProto::None => Some(pp.type_hint.is_none().then_some(Value::Null)),
                PropDefaultProto::Init(_) => None,
            };
            seen.insert(
                pp.name.as_bytes().to_vec(),
                PropDesc {
                    visibility: pp.visibility,
                    is_static: false,
                    readonly: pp.is_readonly || proto.flags.is_readonly,
                    ty: pp.type_hint.clone(),
                    default,
                    hooked: pp.has_hooks(),
                },
            );
        }
        for sp in &proto.static_props {
            let default = match &sp.init {
                ConstInitProto::Const(v) if sp.has_default || sp.type_hint.is_none() => {
                    Some(Some(v.clone()))
                }
                ConstInitProto::Const(_) => Some(None),
                ConstInitProto::Thunk(_) => None,
            };
            seen.insert(
                sp.name.as_bytes().to_vec(),
                PropDesc {
                    visibility: sp.visibility,
                    is_static: true,
                    readonly: false,
                    ty: sp.type_hint.clone(),
                    default,
                    hooked: false,
                },
            );
        }
        for (i, &t) in traits.iter().enumerate() {
            let Some(t) = t else { continue };
            let e = &self.classes[t as usize];
            let mut descs: Vec<(Vec<u8>, PropDesc)> = e
                .props
                .values()
                .map(|pe| {
                    let default = if pe.init.is_some() {
                        None
                    } else if pe.no_default && pe.ty.is_some() {
                        Some(None)
                    } else {
                        Some(Some(pe.default.clone()))
                    };
                    (
                        pe.name.as_bytes().to_vec(),
                        PropDesc {
                            visibility: pe.visibility,
                            is_static: false,
                            readonly: pe.readonly,
                            ty: pe.ty.as_deref().cloned(),
                            default,
                            hooked: pe.hooks.is_some(),
                        },
                    )
                })
                .collect();
            for (k, sp) in &e.static_props {
                let typed_undef = sp.ty.is_some()
                    && e.proto
                        .static_props
                        .iter()
                        .find(|p| p.name.as_bytes() == k.as_slice())
                        .is_some_and(|p| !p.has_default);
                let default = if sp.init.is_some() {
                    None
                } else if typed_undef {
                    Some(None)
                } else {
                    Some(Some(sp.default.borrow().clone()))
                };
                descs.push((
                    k.clone(),
                    PropDesc {
                        visibility: sp.visibility,
                        is_static: true,
                        readonly: false,
                        ty: sp.ty.as_deref().cloned(),
                        default,
                        hooked: false,
                    },
                ));
            }
            for (k, d) in descs {
                let Some(ex) = seen.get(&k) else {
                    seen.insert(k, d);
                    continue;
                };
                let first = || {
                    self.first_trait_with(traits, i, |e| {
                        e.prop_visible.contains_key(&PhpStr::from(k.as_slice()))
                            || e.static_props.contains_key(&k)
                    })
                    .unwrap_or_else(|| lossy(proto.name.as_bytes()))
                };
                let (pn, tn, cn) = (
                    lossy(&k),
                    lossy(self.classes[t as usize].name.as_bytes()),
                    lossy(proto.name.as_bytes()),
                );
                if ex.hooked || d.hooked {
                    let msg = format!(
                        "{} and {tn} define the same hooked property (${pn}) in the composition of {cn}. Conflict resolution between hooked properties is currently not supported. Class was composed",
                        first()
                    );
                    return Err(self.link_fatal(proto, 64, &msg));
                }
                let compatible = ex.visibility == d.visibility
                    && ex.is_static == d.is_static
                    && ex.readonly == d.readonly
                    && ex.ty == d.ty
                    && same_value(&ex.default, &d.default);
                if !compatible {
                    let msg = format!(
                        "{} and {tn} define the same property (${pn}) in the composition of {cn}. However, the definition differs and is considered incompatible. Class was composed",
                        first()
                    );
                    return Err(self.link_fatal(proto, 64, &msg));
                }
            }
        }
        Ok(())
    }
}
