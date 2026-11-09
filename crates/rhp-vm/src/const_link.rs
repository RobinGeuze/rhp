//! Typed class constants at link time: php's do_inherit_constant_check and
//! do_trait_constant_check type rules (zend_inheritance.c).
//!
//! The covariance test ports zend_perform_covariant_type_check with its
//! three outcomes; the method LSP check should share it once it exists.
//! Class lookups do not autoload, so an unresolved check is an error.

use rhp_bytecode::{ClassProto, TypeHint, TypeKind as K};
use rhp_values::{ClassId, Thrown};

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
        let cx = Cx::new(proto, parent);
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
            for (k, c) in self.classes[*src as usize].constants.iter() {
                if c.visibility == rhp_values::Visibility::Private {
                    continue;
                }
                if let Some(t) = &c.ty {
                    inherited.push((k.clone(), c.declaring_class, (**t).clone()));
                }
            }
        }
        // Trait constants against the class's own ones; a trait constant
        // the class does not redeclare then counts as its own below.
        let mut from_traits: Vec<(Vec<u8>, Option<TypeHint>)> = Vec::new();
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
                // php binds traits before inheriting: only the class's own
                // constants (and earlier traits') can collide here.
                // (a trait constant takes the composing class as its owner)
                let existing = own(&k)
                    .map(|c| c.type_hint.clone())
                    .or_else(|| {
                        from_traits
                            .iter()
                            .find(|(n, _)| *n == k)
                            .map(|(_, t)| t.clone())
                    })
                    .map(|t| (cname.clone(), t));
                let Some((owner, ety)) = existing else {
                    from_traits.push((k.clone(), tty.clone()));
                    continue;
                };
                let ok = match (&ety, &tty) {
                    (None, None) => true,
                    (Some(e), Some(t)) => {
                        let tscope = self.classes[tid as usize].name.clone();
                        let (cs, ts) = (cname.as_bytes(), tscope.as_bytes());
                        self.const_type_status(&cx, e, cs, t, ts) != St::Err
                            && self.const_type_status(&cx, t, ts, e, cs) != St::Err
                    }
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
            let child: Option<TypeHint> = match own(&k) {
                Some(c) => c.type_hint.clone(),
                None => match from_traits.iter().find(|(n, _)| *n == k) {
                    Some((_, t)) => t.clone(),
                    None => continue,
                },
            };
            let pscope = self.classes[pdecl as usize].name.clone();
            let ok = child.as_ref().is_some_and(|ct| {
                self.const_type_status(&cx, ct, cname.as_bytes(), &pty, pscope.as_bytes()) == St::Ok
            });
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

    /// Can php early-bind `proto` as far as its typed constants go
    /// (zend_can_early_bind)? Not while a check needs a class that is not
    /// loaded; php then declares the class when its declaration runs.
    pub(crate) fn const_types_resolvable(&self, proto: &ClassProto) -> bool {
        let Some(&parent) = proto
            .parent
            .as_ref()
            .and_then(|p| self.class_ids.get(&lc(p.as_bytes())))
        else {
            return true;
        };
        let cx = Cx::new(proto, Some(parent));
        proto.constants.iter().all(|c| {
            let Some(pc) = self.classes[parent as usize]
                .constants
                .get(c.name.as_bytes())
            else {
                return true;
            };
            if pc.visibility == rhp_values::Visibility::Private {
                return true;
            }
            match (&c.type_hint, pc.ty.as_deref()) {
                (Some(ct), Some(pt)) => {
                    let child = self.norm(ct, &cx, cx.name);
                    let par = self.norm(
                        pt,
                        &cx,
                        self.classes[pc.declaring_class as usize].name.as_bytes(),
                    );
                    self.covariant(&cx, &child, &par) != St::Unresolved
                }
                _ => true,
            }
        })
    }

    /// Does every value of `child` (declared in `cscope`) belong to
    /// `parent` (declared in `pscope`)? For `proto` being linked.
    fn const_type_status(
        &self,
        cx: &Cx,
        child: &TypeHint,
        cscope: &[u8],
        parent: &TypeHint,
        pscope: &[u8],
    ) -> St {
        let c = self.norm(child, cx, cscope);
        let p = self.norm(parent, cx, pscope);
        self.covariant(cx, &c, &p)
    }

    /// A type as php's zend_type: builtin bits plus class members, with
    /// self/parent resolved against `scope`.
    fn norm(&self, t: &TypeHint, cx: &Cx, scope: &[u8]) -> Ty {
        let mut ty = Ty {
            mask: if t.nullable { M_NULL } else { 0 },
            members: Vec::new(),
            intersection: t.intersection,
        };
        let parent_of = |scope: &[u8]| -> Vec<u8> {
            if scope.eq_ignore_ascii_case(cx.name) {
                return cx
                    .proto
                    .parent
                    .as_ref()
                    .map_or(b"parent".to_vec(), |p| p.as_bytes().to_vec());
            }
            self.link_lookup(&lc(scope))
                .and_then(|id| self.classes[id as usize].parent)
                .map_or(b"parent".to_vec(), |p| {
                    self.classes[p as usize].name.as_bytes().to_vec()
                })
        };
        for k in &t.kinds {
            match k {
                K::Mixed => ty.mask |= M_ANY,
                K::Null => ty.mask |= M_NULL,
                K::Int => ty.mask |= M_INT,
                K::Float => ty.mask |= M_FLOAT,
                K::String => ty.mask |= M_STRING,
                K::Bool => ty.mask |= M_TRUE | M_FALSE,
                K::True => ty.mask |= M_TRUE,
                K::False => ty.mask |= M_FALSE,
                K::Array => ty.mask |= M_ARRAY,
                K::Object => ty.mask |= M_OBJECT,
                K::Callable => ty.mask |= M_CALLABLE,
                K::Void => ty.mask |= M_VOID,
                K::Never => ty.mask |= M_NEVER,
                K::StaticType => ty.mask |= M_STATIC,
                K::Iterable => {
                    ty.mask |= M_ARRAY;
                    ty.members.push(Member::Class(b"Traversable".to_vec()));
                }
                K::SelfType => ty.members.push(Member::Class(scope.to_vec())),
                K::Parent => ty.members.push(Member::Class(parent_of(scope))),
                K::Named(n) => ty.members.push(Member::Class(n.as_bytes().to_vec())),
            }
        }
        for g in &t.groups {
            ty.members.push(Member::Group(
                g.iter().map(|n| n.as_bytes().to_vec()).collect(),
            ));
        }
        ty
    }

    /// lookup_class: a loaded class, or the class being linked.
    fn lookup(&self, cx: &Cx, name: &[u8]) -> Option<Cls> {
        let n = name.strip_prefix(b"\\").unwrap_or(name);
        if let Some(id) = self.link_lookup(&lc(n)) {
            return Some(Cls::Loaded(id));
        }
        n.eq_ignore_ascii_case(cx.name).then_some(Cls::Linking)
    }

    /// unlinked_instanceof
    fn cls_is_a(&self, cx: &Cx, a: Cls, b: Cls) -> bool {
        let anc = |a: ClassId, b: ClassId| self.classes[a as usize].ancestors.contains(&b);
        match (a, b) {
            (Cls::Loaded(a), Cls::Loaded(b)) => a == b || anc(a, b),
            (Cls::Linking, Cls::Loaded(b)) => {
                cx.parent.is_some_and(|p| anc(p, b))
                    || cx.proto.interfaces.iter().any(|i| {
                        self.class_ids
                            .get(&lc(i.as_bytes()))
                            .is_some_and(|&i| anc(i, b))
                    })
            }
            (Cls::Linking, Cls::Linking) => true,
            (Cls::Loaded(_), Cls::Linking) => false,
        }
    }

    /// zend_type_permits_self
    fn permits_self(&self, cx: &Cx, t: &Ty) -> bool {
        t.mask & M_OBJECT != 0
            || t.members.iter().any(|m| match m {
                Member::Class(n) => self
                    .lookup(cx, n)
                    .is_some_and(|c| self.cls_is_a(cx, Cls::Linking, c)),
                Member::Group(_) => false,
            })
    }

    /// zend_perform_covariant_type_check
    fn covariant(&self, cx: &Cx, fe: &Ty, proto: &Ty) -> St {
        if proto.mask & M_ANY == M_ANY && fe.mask & M_VOID == 0 {
            return St::Ok;
        }
        let mut added = fe.mask & !proto.mask;
        if added & M_STATIC != 0 && self.permits_self(cx, proto) {
            added &= !M_STATIC;
        }
        if added == M_NEVER {
            return St::Ok;
        }
        if added != 0 {
            return St::Err;
        }
        if fe.intersection {
            return self.inter_subtype(cx, &fe.class_names(), proto);
        }
        let mut unresolved = false;
        for m in &fe.members {
            let st = match m {
                Member::Group(g) => {
                    let names: Vec<&[u8]> = g.iter().map(|n| n.as_slice()).collect();
                    self.inter_subtype(cx, &names, proto)
                }
                Member::Class(n) => self.class_subtype(cx, n, proto),
            };
            if st == St::Err {
                return St::Err;
            }
            unresolved |= st == St::Unresolved;
        }
        if unresolved {
            St::Unresolved
        } else {
            St::Ok
        }
    }

    /// zend_is_class_subtype_of_type
    fn class_subtype(&self, cx: &Cx, fe_name: &[u8], proto: &Ty) -> St {
        let mut unresolved = false;
        let fe = self.lookup(cx, fe_name);
        if proto.mask & M_OBJECT != 0 {
            if fe.is_some() {
                return St::Ok;
            }
            unresolved = true;
        }
        if proto.mask & M_CALLABLE != 0 {
            match fe {
                None => unresolved = true,
                Some(Cls::Loaded(id))
                    if self.classes[id as usize]
                        .name
                        .as_bytes()
                        .eq_ignore_ascii_case(b"Closure") =>
                {
                    return St::Ok
                }
                _ => {}
            }
        }
        // a final class may replace `static` with itself
        if proto.mask & M_STATIC != 0 && cx.proto.flags.is_final {
            match fe {
                None => unresolved = true,
                Some(Cls::Linking) => return St::Ok,
                _ => {}
            }
        }
        let inter = proto.intersection;
        for m in &proto.members {
            let pname = match m {
                Member::Group(g) => {
                    let sub = Ty {
                        mask: 0,
                        members: g.iter().map(|n| Member::Class(n.clone())).collect(),
                        intersection: true,
                    };
                    match self.class_subtype(cx, fe_name, &sub) {
                        St::Err if inter => return St::Err,
                        St::Unresolved => unresolved = true,
                        St::Ok if !inter => return St::Ok,
                        _ => {}
                    }
                    continue;
                }
                Member::Class(n) => n,
            };
            if fe_name.eq_ignore_ascii_case(pname) {
                if !inter {
                    return St::Ok;
                }
                continue;
            }
            let (Some(a), Some(b)) = (fe, self.lookup(cx, pname)) else {
                unresolved = true;
                continue;
            };
            if self.cls_is_a(cx, a, b) {
                if !inter {
                    return St::Ok;
                }
            } else if inter {
                return St::Err;
            }
        }
        if unresolved {
            St::Unresolved
        } else if inter {
            St::Ok
        } else {
            St::Err
        }
    }

    /// zend_is_intersection_subtype_of_type
    fn inter_subtype(&self, cx: &Cx, fe: &[&[u8]], proto: &Ty) -> St {
        let mut unresolved = false;
        if proto.mask & M_OBJECT != 0 {
            for n in fe {
                if self.lookup(cx, n).is_some() {
                    return St::Ok;
                }
                unresolved = true;
            }
        }
        let early = if proto.intersection { St::Err } else { St::Ok };
        for m in &proto.members {
            let st = match m {
                Member::Group(g) => {
                    let sub = Ty {
                        mask: 0,
                        members: g.iter().map(|n| Member::Class(n.clone())).collect(),
                        intersection: true,
                    };
                    self.inter_subtype(cx, fe, &sub)
                }
                Member::Class(p) => self.inter_subtype_of_class(cx, fe, p),
            };
            if st == early {
                return st;
            }
            unresolved |= st == St::Unresolved;
        }
        if unresolved {
            St::Unresolved
        } else if early == St::Err {
            St::Ok
        } else {
            St::Err
        }
    }

    /// zend_is_intersection_subtype_of_class
    fn inter_subtype_of_class(&self, cx: &Cx, fe: &[&[u8]], pname: &[u8]) -> St {
        let mut unresolved = false;
        let p = self.lookup(cx, pname);
        for n in fe {
            if n.eq_ignore_ascii_case(pname) {
                return St::Ok;
            }
            let (Some(a), Some(b)) = (self.lookup(cx, n), p) else {
                unresolved = true;
                continue;
            };
            if self.cls_is_a(cx, a, b) {
                return St::Ok;
            }
        }
        if unresolved {
            St::Unresolved
        } else {
            St::Err
        }
    }
    /// do_inherit_constant_check's final, ambiguity and visibility rules
    /// for the constants `proto` (not linked yet) inherits from its parent,
    /// then from the interfaces it lists.
    pub(crate) fn check_const_inheritance(
        &mut self,
        proto: &ClassProto,
        parent: Option<ClassId>,
    ) -> Result<(), Thrown> {
        use rhp_values::Visibility as V;
        // (declaring class, visibility, final); `None` is the class itself.
        type Entry = (Option<ClassId>, V, bool);
        let mut table: indexmap::IndexMap<Vec<u8>, Entry> = indexmap::IndexMap::new();
        for c in &proto.constants {
            table.insert(c.name.as_bytes().to_vec(), (None, c.visibility, c.is_final));
        }
        for c in &proto.enum_cases {
            table.insert(c.name.as_bytes().to_vec(), (None, V::Public, false));
        }
        for t in &proto.traits {
            if let Some(&tid) = self.class_ids.get(&lc(t.as_bytes())) {
                for (k, c) in self.classes[tid as usize].constants.iter() {
                    table
                        .entry(k.clone())
                        .or_insert((None, c.visibility, c.is_final));
                }
            }
        }
        let rank = |v: V| match v {
            V::Public => 0,
            V::Protected => 1,
            V::Private => 2,
        };
        let cname = String::from_utf8_lossy(proto.name.as_bytes());
        let cname = cname.split('\0').next().unwrap_or_default().to_string();
        let shown = |vm: &Vm, c: Option<ClassId>| match c {
            None => cname.clone(),
            Some(id) => {
                let n = String::from_utf8_lossy(vm.classes[id as usize].name.as_bytes());
                n.split('\0').next().unwrap_or_default().to_string()
            }
        };
        let check = |vm: &Vm, k: &[u8], child: Entry, p: Entry| -> Option<String> {
            let kn = String::from_utf8_lossy(k);
            if p.0 != child.0 && p.2 {
                return Some(format!(
                    "{}::{kn} cannot override final constant {}::{kn}",
                    shown(vm, child.0),
                    shown(vm, p.0)
                ));
            }
            if child.0 != p.0 && child.0.is_some() {
                return Some(format!(
                    "{} {cname} inherits both {}::{kn} and {}::{kn}, which is ambiguous",
                    crate::link_checks::kind_uc(proto.kind),
                    shown(vm, child.0),
                    shown(vm, p.0)
                ));
            }
            if rank(child.1) > rank(p.1) {
                let pk = p.0.map_or(proto.kind, |id| vm.classes[id as usize].kind);
                let word = match p.1 {
                    V::Public => "public",
                    V::Protected => "protected",
                    V::Private => "private",
                };
                return Some(format!(
                    "Access level to {cname}::{kn} must be {word} (as in {} {}){}",
                    crate::link_checks::kind_uc(pk).to_ascii_lowercase(),
                    shown(vm, p.0),
                    if p.1 == V::Public { "" } else { " or weaker" }
                ));
            }
            None
        };
        let entries = |vm: &Vm, id: ClassId| -> Vec<(Vec<u8>, Entry)> {
            vm.classes[id as usize]
                .constants
                .iter()
                .map(|(k, c)| {
                    (
                        k.clone(),
                        (Some(c.declaring_class), c.visibility, c.is_final),
                    )
                })
                .collect()
        };
        let mut err = None;
        if let Some(p) = parent {
            for (k, pc) in entries(self, p) {
                match table.get(&k) {
                    Some(&child) => {
                        err = err.or_else(|| check(self, &k, child, pc));
                    }
                    None if pc.1 != V::Private => {
                        table.insert(k, pc);
                    }
                    None => {}
                }
            }
        }
        let parent_ifaces: Vec<ClassId> = parent
            .map(|p| self.classes[p as usize].interfaces.clone())
            .unwrap_or_default();
        let ifaces: Vec<ClassId> = proto
            .interfaces
            .iter()
            .filter_map(|i| self.class_ids.get(&lc(i.as_bytes())).copied())
            .collect();
        // Interfaces the parent implements are only checked; then the new
        // ones add theirs.
        for pass in [true, false] {
            for &iid in &ifaces {
                if parent_ifaces.contains(&iid) != pass {
                    continue;
                }
                for (k, ic) in entries(self, iid) {
                    match table.get(&k) {
                        Some(&child) => err = err.or_else(|| check(self, &k, child, ic)),
                        None if !pass => {
                            table.insert(k, ic);
                        }
                        None => {}
                    }
                }
            }
        }
        match err {
            Some(msg) => Err(self.link_fatal(proto, 64, &msg)),
            None => Ok(()),
        }
    }
}

/// inheritance_status
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum St {
    Ok,
    Err,
    Unresolved,
}

/// A class found by a lookup: loaded, or the one being linked.
#[derive(Clone, Copy)]
enum Cls {
    Loaded(ClassId),
    Linking,
}

/// The class being linked.
struct Cx<'a> {
    proto: &'a ClassProto,
    name: &'a [u8],
    parent: Option<ClassId>,
}

impl<'a> Cx<'a> {
    fn new(proto: &'a ClassProto, parent: Option<ClassId>) -> Self {
        Cx {
            proto,
            name: proto.name.as_bytes(),
            parent,
        }
    }
}

enum Member {
    Class(Vec<u8>),
    Group(Vec<Vec<u8>>),
}

struct Ty {
    mask: u32,
    members: Vec<Member>,
    intersection: bool,
}

impl Ty {
    fn class_names(&self) -> Vec<&[u8]> {
        self.members
            .iter()
            .filter_map(|m| match m {
                Member::Class(n) => Some(n.as_slice()),
                Member::Group(_) => None,
            })
            .collect()
    }
}

const M_NULL: u32 = 1 << 1;
const M_FALSE: u32 = 1 << 2;
const M_TRUE: u32 = 1 << 3;
const M_INT: u32 = 1 << 4;
const M_FLOAT: u32 = 1 << 5;
const M_STRING: u32 = 1 << 6;
const M_ARRAY: u32 = 1 << 7;
const M_OBJECT: u32 = 1 << 8;
const M_RESOURCE: u32 = 1 << 9;
const M_CALLABLE: u32 = 1 << 12;
const M_VOID: u32 = 1 << 14;
const M_STATIC: u32 = 1 << 15;
const M_NEVER: u32 = 1 << 17;
const M_ANY: u32 =
    M_NULL | M_FALSE | M_TRUE | M_INT | M_FLOAT | M_STRING | M_ARRAY | M_OBJECT | M_RESOURCE;
