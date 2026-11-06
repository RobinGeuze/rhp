//! Inheritance checks run while a user class links (php's
//! zend_inheritance.c): method signature compatibility (LSP) against parent,
//! trait-abstract and interface methods; the final / static / abstract /
//! visibility rules for overrides; property type invariance; tentative
//! return types of internal methods (the `#[\ReturnTypeWillChange]`
//! deprecation); and the delayed variance checks, which autoload the classes
//! a check needs and then report "Could not check compatibility".
//!
//! The checks run once the class's entry is in the class table, so the class
//! and its parent resolve like any linked class. A class still loading its
//! dependencies (`LinkState::linking`) is visible to them as php's unlinked
//! class is: by its declared parent and interface names.

use std::rc::Rc;

use rhp_bytecode::{
    signatures as sigs, ClassProto, FunctionProto, ParamDefault, TypeHint, TypeKind,
};
use rhp_values::{conv, ClassId, ClassKind, PhpStr, Thrown, Value, Visibility};

use crate::classes::ResolvedMethod;
use crate::Vm;

/// php's `inheritance_status`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Status {
    Success,
    Error,
    Warning,
    Unresolved,
}

/// Linking state shared across nested class declarations.
#[derive(Default)]
pub(crate) struct LinkState {
    /// Classes declared at run time that are still loading their parent,
    /// interfaces and traits (php registers them unlinked first), innermost
    /// last, with whether a variance check looked at them.
    linking: Vec<(Vec<u8>, Rc<ClassProto>, bool)>,
    /// php's `CG(delayed_autoloads)`: classes a variance check could not find.
    delayed: Vec<PhpStr>,
    /// Hoisting (php's early binding at compile time): no unlinked classes
    /// are visible and nothing is registered for autoloading.
    compiling: bool,
    /// Diagnostics raised while linking, emitted once linking is done
    /// (php's `zend_begin_record_errors`): message, file, line.
    recorded: Vec<(Vec<u8>, Vec<u8>, i64)>,
}

// php's MAY_BE_* bits, as far as declared types use them.
const M_NULL: u32 = 1;
const M_FALSE: u32 = 1 << 1;
const M_TRUE: u32 = 1 << 2;
const M_LONG: u32 = 1 << 3;
const M_DOUBLE: u32 = 1 << 4;
const M_STRING: u32 = 1 << 5;
const M_ARRAY: u32 = 1 << 6;
const M_OBJECT: u32 = 1 << 7;
const M_RESOURCE: u32 = 1 << 8;
const M_CALLABLE: u32 = 1 << 9;
const M_VOID: u32 = 1 << 10;
const M_NEVER: u32 = 1 << 11;
const M_STATIC: u32 = 1 << 12;
const M_BOOL: u32 = M_FALSE | M_TRUE;
const M_ANY: u32 = M_NULL | M_BOOL | M_LONG | M_DOUBLE | M_STRING | M_ARRAY | M_OBJECT | M_RESOURCE;

/// A declared type as php's `zend_type` holds it: the builtin-type mask
/// plus the class names (a union's names and DNF groups, or a pure
/// intersection's names). `self` and `parent` stay names.
#[derive(Clone, Debug)]
pub(crate) struct ZType {
    mask: u32,
    singles: Vec<Single>,
    intersection: bool,
}

#[derive(Clone, Debug)]
enum Single {
    Name(PhpStr),
    Inter(Vec<PhpStr>),
}

impl ZType {
    pub(crate) fn from_hint(h: &TypeHint) -> ZType {
        let mut mask = if h.nullable { M_NULL } else { 0 };
        let mut singles: Vec<Single> = h.groups.iter().map(|g| Single::Inter(g.clone())).collect();
        for k in &h.kinds {
            match k {
                TypeKind::Int => mask |= M_LONG,
                TypeKind::Float => mask |= M_DOUBLE,
                TypeKind::String => mask |= M_STRING,
                TypeKind::Bool => mask |= M_BOOL,
                TypeKind::Array => mask |= M_ARRAY,
                TypeKind::Object => mask |= M_OBJECT,
                TypeKind::Callable => mask |= M_CALLABLE,
                TypeKind::Iterable => {
                    mask |= M_ARRAY;
                    singles.push(Single::Name(PhpStr::from("Traversable")));
                }
                TypeKind::Mixed => mask |= M_ANY,
                TypeKind::Void => mask |= M_VOID,
                TypeKind::Never => mask |= M_NEVER,
                TypeKind::Null => mask |= M_NULL,
                TypeKind::True => mask |= M_TRUE,
                TypeKind::False => mask |= M_FALSE,
                TypeKind::StaticType => mask |= M_STATIC,
                TypeKind::SelfType => singles.push(Single::Name(PhpStr::from("self"))),
                TypeKind::Parent => singles.push(Single::Name(PhpStr::from("parent"))),
                TypeKind::Named(n) => singles.push(Single::Name(n.clone())),
            }
        }
        ZType {
            mask,
            singles,
            intersection: h.intersection,
        }
    }

    /// A type as a stub (and Reflection) spells it: `?Foo`, `A|false`,
    /// `(A&B)|null`, `A&B`.
    fn from_stub(s: &str) -> ZType {
        let (mut mask, s) = match s.strip_prefix('?') {
            Some(r) => (M_NULL, r),
            None => (0, s),
        };
        let mut singles = Vec::new();
        let intersection = !s.contains('|') && s.contains('&');
        let parts: Vec<&str> = if intersection {
            s.split('&').collect()
        } else {
            s.split('|').collect()
        };
        for p in parts {
            if let Some(g) = p.strip_prefix('(').and_then(|g| g.strip_suffix(')')) {
                singles.push(Single::Inter(g.split('&').map(PhpStr::from).collect()));
                continue;
            }
            let bit = match p.to_ascii_lowercase().as_str() {
                "int" => M_LONG,
                "float" => M_DOUBLE,
                "string" => M_STRING,
                "bool" => M_BOOL,
                "array" => M_ARRAY,
                "object" => M_OBJECT,
                "callable" => M_CALLABLE,
                "mixed" => M_ANY,
                "void" => M_VOID,
                "never" => M_NEVER,
                "null" => M_NULL,
                "true" => M_TRUE,
                "false" => M_FALSE,
                "static" => M_STATIC,
                "resource" => M_RESOURCE,
                "iterable" => {
                    singles.push(Single::Name(PhpStr::from("Traversable")));
                    M_ARRAY
                }
                _ => {
                    singles.push(Single::Name(PhpStr::from(p)));
                    0
                }
            };
            mask |= bit;
        }
        ZType {
            mask,
            singles,
            intersection,
        }
    }

    fn group(names: &[PhpStr]) -> ZType {
        ZType {
            mask: 0,
            singles: names.iter().cloned().map(Single::Name).collect(),
            intersection: true,
        }
    }

    fn contains_void(&self) -> bool {
        self.mask & M_VOID != 0
    }
}

/// A class a check looked up: linked, or still loading its dependencies.
#[derive(Clone)]
enum CRef {
    Linked(ClassId),
    Pending(Rc<ClassProto>),
}

impl CRef {
    fn same(&self, o: &CRef) -> bool {
        match (self, o) {
            (CRef::Linked(a), CRef::Linked(b)) => a == b,
            (CRef::Pending(a), CRef::Pending(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// One parameter of a [`Sig`].
#[derive(Clone)]
struct SParam {
    name: Vec<u8>,
    ty: Option<ZType>,
    by_ref: bool,
    variadic: bool,
    /// The default as zend_get_function_declaration prints it.
    default: Option<String>,
}

/// A method signature with what the override rules need.
#[derive(Clone)]
pub(crate) struct Sig {
    name: Vec<u8>,
    /// The method's class (`fptr->common.scope`).
    class: ClassId,
    returns_ref: bool,
    required: usize,
    params: Vec<SParam>,
    variadic: bool,
    ret: Option<ZType>,
    tentative: bool,
    vis: Visibility,
    is_static: bool,
    is_abstract: bool,
    is_final: bool,
    is_ctor: bool,
    return_type_will_change: bool,
    /// Declaring file and line (empty / 0 for an internal method).
    file: Vec<u8>,
    line: u32,
    /// php's `common.prototype`, kept for constructors: the class whose
    /// abstract (or interface) constructor this one implements.
    prototype: Option<ClassId>,
}

/// A property for the type invariance check.
#[derive(Clone)]
struct PInfo {
    class: ClassId,
    name: PhpStr,
    ty: Option<ZType>,
    /// prop_get_variance of this (parent) property.
    variance: PropVariance,
}

/// php's prop_variance: a virtual property with only a get hook may narrow
/// its type, one with only a set hook may widen it.
#[derive(Clone, Copy, PartialEq)]
enum PropVariance {
    Invariant,
    Covariant,
    Contravariant,
}

fn prop_variance(pe: &crate::classes::PropEntry) -> PropVariance {
    match &pe.hooks {
        Some(h) if pe.virt => {
            if h[rhp_bytecode::HookKind::Set as usize].is_none() {
                PropVariance::Covariant
            } else if h[rhp_bytecode::HookKind::Get as usize].is_none() {
                PropVariance::Contravariant
            } else {
                PropVariance::Invariant
            }
        }
        _ => PropVariance::Invariant,
    }
}

enum Obligation {
    Method {
        child: Sig,
        child_scope: ClassId,
        parent: Sig,
        parent_scope: ClassId,
    },
    Prop {
        child: PInfo,
        parent: PInfo,
    },
}

// do_inheritance_check_on_method's flags.
const CHECK_SILENT: u8 = 1;
const CHECK_PROTO: u8 = 2;
const CHECK_VISIBILITY: u8 = 4;
const SET_CHILD_PROTO: u8 = 8;

fn vis_rank(v: Visibility) -> u8 {
    match v {
        Visibility::Public => 1,
        Visibility::Protected => 2,
        Visibility::Private => 4,
    }
}

fn vis_word(v: Visibility) -> &'static str {
    match v {
        Visibility::Public => "public",
        Visibility::Protected => "protected",
        Visibility::Private => "private",
    }
}

fn lossy(b: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(b)
}

/// A class name as messages print it (an anonymous class's name stops at
/// its NUL byte).
fn cut_nul(b: &[u8]) -> &[u8] {
    match b.iter().position(|&c| c == 0) {
        Some(i) => &b[..i],
        None => b,
    }
}

/// A user parameter's default as zend_get_function_declaration prints it.
/// `self_name`: the class `self::class` folds to at compile time (none in a
/// trait).
fn user_default(
    proto: &FunctionProto,
    p: &rhp_bytecode::ParamInfo,
    self_name: Option<&[u8]>,
) -> String {
    let mut folded = None;
    if let Some(c) = &p.default_const {
        // `X::class` is folded to its string when the class is known at
        // compile time, else it is an expression.
        let lc = c.as_bytes().to_ascii_lowercase();
        if !lc.ends_with(b"::class") {
            return lossy(c.as_bytes()).into_owned();
        }
        if !matches!(p.default, Some(ParamDefault::Const(_))) {
            match self_name {
                Some(n) if lc == b"self::class" => folded = Some(Value::str(n.to_vec())),
                _ => return "<expression>".into(),
            }
        }
    }
    let v = match (folded, p.default) {
        (Some(v), _) => Some(v),
        (None, Some(ParamDefault::Const(ci))) => Some(proto.consts[ci as usize].clone()),
        _ => None,
    };
    match v {
        Some(v) => match &v {
            Value::Bool(false) => "false".into(),
            Value::Bool(true) => "true".into(),
            Value::Null => "null".into(),
            Value::Str(s) => {
                let b = s.as_bytes();
                let mut out = String::from("'");
                out.push_str(&lossy(&b[..b.len().min(10)]));
                if b.len() > 10 {
                    out.push_str("...");
                }
                out.push('\'');
                out
            }
            Value::Array(a) => if a.is_empty() { "[]" } else { "[...]" }.into(),
            v => lossy(&conv::to_string_bytes(v)).into_owned(),
        },
        None => "<expression>".into(),
    }
}

impl Vm {
    /// The signature of user method `proto` declared in class `class`.
    #[allow(clippy::too_many_arguments)]
    fn user_sig(
        &self,
        proto: &FunctionProto,
        class: ClassId,
        vis: Visibility,
        is_static: bool,
        is_abstract: bool,
        is_final: bool,
        prototype: Option<ClassId>,
    ) -> Sig {
        let ps = &proto.params;
        // A method compiled in the class itself (not copied from a trait).
        let e = self.class(class);
        let self_name = e
            .proto
            .methods
            .iter()
            .any(|m| std::ptr::eq(&*m.proto, proto))
            .then(|| e.name.as_bytes().to_vec());
        let required = ps
            .iter()
            .rposition(|p| p.default.is_none() && !p.variadic)
            .map_or(0, |i| i + 1);
        let params = ps
            .iter()
            .enumerate()
            .map(|(i, p)| SParam {
                name: proto.names[p.name as usize].as_bytes().to_vec(),
                ty: p.type_hint.as_ref().map(|h| {
                    let mut t = ZType::from_hint(h);
                    // A `null` default makes the type implicitly nullable.
                    if let Some(ParamDefault::Const(ci)) = p.default {
                        if matches!(proto.consts[ci as usize], Value::Null)
                            && p.default_const.is_none()
                        {
                            t.mask |= M_NULL;
                        }
                    }
                    t
                }),
                by_ref: p.by_ref,
                variadic: p.variadic,
                default: (i >= required && !p.variadic)
                    .then(|| user_default(proto, p, self_name.as_deref())),
            })
            .collect();
        let name = proto.name.as_bytes().to_vec();
        let mut ret = proto.return_type.as_ref().map(ZType::from_hint);
        // __toString() has an implicit `string` return type.
        if ret.is_none() && name.eq_ignore_ascii_case(b"__tostring") {
            ret = Some(ZType {
                mask: M_STRING,
                singles: Vec::new(),
                intersection: false,
            });
        }
        Sig {
            is_ctor: name.eq_ignore_ascii_case(b"__construct"),
            name,
            class,
            returns_ref: proto.returns_ref,
            required,
            params,
            variadic: ps.last().is_some_and(|p| p.variadic),
            ret,
            tentative: false,
            vis,
            is_static,
            is_abstract,
            is_final,
            return_type_will_change: proto.meta.attributes.iter().any(|a| {
                a.name
                    .as_bytes()
                    .eq_ignore_ascii_case(b"ReturnTypeWillChange")
            }),
            file: proto.debug.file.as_bytes().to_vec(),
            line: proto.meta.start_line,
            prototype,
        }
    }

    /// The signature of an internal method, from its stub.
    fn stub_sig(&self, class: ClassId, name: &[u8]) -> Option<Sig> {
        use sigs::Signature;
        let m = sigs::method(self.class(class).name.as_bytes(), name)?;
        let required = m.min_args();
        let params = m
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| SParam {
                name: p.name.as_bytes().to_vec(),
                ty: (!p.ty.is_empty()).then(|| ZType::from_stub(p.ty)),
                by_ref: p.by_ref(),
                variadic: p.variadic(),
                default: (i >= required && !p.variadic())
                    .then(|| p.default().unwrap_or("<default>").to_string()),
            })
            .collect();
        let vis = if m.flags & sigs::PRIVATE != 0 {
            Visibility::Private
        } else if m.flags & sigs::PROTECTED != 0 {
            Visibility::Protected
        } else {
            Visibility::Public
        };
        Some(Sig {
            name: m.name.as_bytes().to_vec(),
            class,
            returns_ref: false,
            required,
            params,
            variadic: m.params.last().is_some_and(|p| p.variadic()),
            ret: (!m.ret.is_empty()).then(|| ZType::from_stub(m.ret)),
            tentative: m.tentative(),
            vis,
            is_static: m.flags & sigs::STATIC != 0,
            is_abstract: m.flags & sigs::ABSTRACT != 0,
            is_final: m.flags & sigs::FINAL != 0,
            is_ctor: m.name.eq_ignore_ascii_case("__construct"),
            return_type_will_change: false,
            file: Vec::new(),
            line: 0,
            prototype: None,
        })
    }

    /// The signature of a linked class's method; `None` for an internal
    /// method without a stub row (it is not checked).
    fn method_sig(&self, m: &ResolvedMethod) -> Option<Sig> {
        if m.proto.is_internal() || self.class(m.defining_class).decl_file.is_empty() {
            return self.stub_sig(m.defining_class, m.proto.name.as_bytes());
        }
        Some(self.user_sig(
            &m.proto,
            m.defining_class,
            m.visibility,
            m.is_static,
            m.is_abstract,
            m.is_final,
            m.prototype,
        ))
    }

    // ----- class lookups ------------------------------------------------

    /// lookup_class: a linked class, or (outside hoisting) one still
    /// loading its dependencies. Never autoloads.
    fn lsp_lookup(&mut self, name: &[u8]) -> Option<CRef> {
        let key = name.to_ascii_lowercase();
        if let Some(&id) = self.class_ids.get(&key) {
            return Some(CRef::Linked(id));
        }
        if self.inherit.compiling {
            return None;
        }
        self.lookup_unlinked(&key)
    }

    fn lookup_unlinked(&mut self, key: &[u8]) -> Option<CRef> {
        let e = self.inherit.linking.iter_mut().rev().find(|e| e.0 == key)?;
        e.2 = true;
        Some(CRef::Pending(e.1.clone()))
    }

    /// resolve_class_name: `self` and `parent` against `scope`.
    fn resolve_name(&self, scope: ClassId, name: &PhpStr) -> PhpStr {
        let b = name.as_bytes();
        if b.eq_ignore_ascii_case(b"parent") {
            if let Some(p) = self.class(scope).parent {
                return self.class(p).name.clone();
            }
        } else if b.eq_ignore_ascii_case(b"self") {
            return self.class(scope).name.clone();
        }
        name.clone()
    }

    /// unlinked_instanceof.
    fn lsp_instanceof(&mut self, a: &CRef, b: &CRef) -> bool {
        if a.same(b) {
            return true;
        }
        match a {
            CRef::Linked(x) => match b {
                CRef::Linked(y) => self.class(*x).ancestors.binary_search(y).is_ok(),
                CRef::Pending(_) => false,
            },
            CRef::Pending(p) => {
                let p = p.clone();
                let find = |vm: &mut Vm, n: &PhpStr| {
                    let key = n.as_bytes().to_ascii_lowercase();
                    match vm.class_ids.get(&key) {
                        Some(&id) => Some(CRef::Linked(id)),
                        None => vm.lookup_unlinked(&key),
                    }
                };
                if let Some(pn) = &p.parent {
                    if let Some(pc) = find(self, pn) {
                        if self.lsp_instanceof(&pc, b) {
                            return true;
                        }
                    }
                }
                for i in &p.interfaces {
                    if let Some(ic) = find(self, i) {
                        if !ic.same(a) && self.lsp_instanceof(&ic, b) {
                            return true;
                        }
                    }
                }
                false
            }
        }
    }

    fn register_unresolved(&mut self, scope: ClassId, ty: &ZType) {
        for s in &ty.singles {
            match s {
                Single::Inter(names) => {
                    for n in names {
                        self.register_unresolved_name(scope, n);
                    }
                }
                Single::Name(n) => self.register_unresolved_name(scope, n),
            }
        }
    }

    fn register_unresolved_name(&mut self, scope: ClassId, n: &PhpStr) {
        let n = self.resolve_name(scope, n);
        if self.inherit.compiling || self.lsp_lookup(n.as_bytes()).is_some() {
            return;
        }
        if !self.inherit.delayed.iter().any(|d| d == &n) {
            self.inherit.delayed.push(n);
        }
    }

    // ----- type variance (zend_perform_covariant_type_check) -------------

    fn type_permits_self(&mut self, ty: &ZType, scope: ClassId, self_: ClassId) -> bool {
        if ty.mask & M_OBJECT != 0 {
            return true;
        }
        for s in ty.singles.clone() {
            if let Single::Name(n) = s {
                let n = self.resolve_name(scope, &n);
                if let Some(c) = self.lsp_lookup(n.as_bytes()) {
                    if self.lsp_instanceof(&CRef::Linked(self_), &c) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn intersection_subtype_of_class(
        &mut self,
        fe_scope: ClassId,
        fe: &ZType,
        proto_scope: ClassId,
        proto_name: &PhpStr,
    ) -> Status {
        let _ = proto_scope;
        let mut unresolved = false;
        let mut proto_ce: Option<CRef> = None;
        for s in fe.singles.clone() {
            let Single::Name(n) = s else { continue };
            let fe_name = self.resolve_name(fe_scope, &n);
            if fe_name
                .as_bytes()
                .eq_ignore_ascii_case(proto_name.as_bytes())
            {
                return Status::Success;
            }
            if proto_ce.is_none() {
                proto_ce = self.lsp_lookup(proto_name.as_bytes());
            }
            let fe_ce = self.lsp_lookup(fe_name.as_bytes());
            let (Some(f), Some(p)) = (fe_ce, proto_ce.clone()) else {
                unresolved = true;
                continue;
            };
            if self.lsp_instanceof(&f, &p) {
                return Status::Success;
            }
        }
        if unresolved {
            Status::Unresolved
        } else {
            Status::Error
        }
    }

    fn class_subtype_of_type(
        &mut self,
        fe_scope: ClassId,
        fe_name: &PhpStr,
        proto_scope: ClassId,
        proto: &ZType,
    ) -> Status {
        let mut fe_ce: Option<CRef> = None;
        let mut unresolved = false;
        if proto.mask & M_OBJECT != 0 {
            fe_ce = self.lsp_lookup(fe_name.as_bytes());
            if fe_ce.is_none() {
                unresolved = true;
            } else {
                return Status::Success;
            }
        }
        if proto.mask & M_CALLABLE != 0 {
            if fe_ce.is_none() {
                fe_ce = self.lsp_lookup(fe_name.as_bytes());
            }
            match &fe_ce {
                None => unresolved = true,
                Some(CRef::Linked(id))
                    if self
                        .class(*id)
                        .name
                        .as_bytes()
                        .eq_ignore_ascii_case(b"Closure")
                        && self.class(*id).decl_file.is_empty() =>
                {
                    return Status::Success
                }
                _ => {}
            }
        }
        if proto.mask & M_STATIC != 0 && self.class(fe_scope).flags.is_final {
            if fe_ce.is_none() {
                fe_ce = self.lsp_lookup(fe_name.as_bytes());
            }
            match &fe_ce {
                None => unresolved = true,
                Some(c) if c.same(&CRef::Linked(fe_scope)) => return Status::Success,
                _ => {}
            }
        }
        let is_inter = proto.intersection;
        for s in proto.singles.clone() {
            match s {
                Single::Inter(g) => {
                    match self.class_subtype_of_type(
                        fe_scope,
                        fe_name,
                        proto_scope,
                        &ZType::group(&g),
                    ) {
                        Status::Error => {
                            if is_inter {
                                return Status::Error;
                            }
                        }
                        Status::Unresolved => unresolved = true,
                        Status::Success => {
                            if !is_inter {
                                return Status::Success;
                            }
                        }
                        Status::Warning => {}
                    }
                }
                Single::Name(n) => {
                    let proto_name = self.resolve_name(proto_scope, &n);
                    if fe_name
                        .as_bytes()
                        .eq_ignore_ascii_case(proto_name.as_bytes())
                    {
                        if !is_inter {
                            return Status::Success;
                        }
                        continue;
                    }
                    if fe_ce.is_none() {
                        fe_ce = self.lsp_lookup(fe_name.as_bytes());
                    }
                    let proto_ce = self.lsp_lookup(proto_name.as_bytes());
                    let (Some(f), Some(p)) = (fe_ce.clone(), proto_ce) else {
                        unresolved = true;
                        continue;
                    };
                    if self.lsp_instanceof(&f, &p) {
                        if !is_inter {
                            return Status::Success;
                        }
                    } else if is_inter {
                        return Status::Error;
                    }
                }
            }
        }
        if unresolved {
            Status::Unresolved
        } else if is_inter {
            Status::Success
        } else {
            Status::Error
        }
    }

    fn intersection_subtype_of_type(
        &mut self,
        fe_scope: ClassId,
        fe: &ZType,
        proto_scope: ClassId,
        proto: &ZType,
    ) -> Status {
        let mut unresolved = false;
        if proto.mask & M_OBJECT != 0 {
            for s in fe.singles.clone() {
                let Single::Name(n) = s else { continue };
                let n = self.resolve_name(fe_scope, &n);
                if self.lsp_lookup(n.as_bytes()).is_some() {
                    return Status::Success;
                }
                unresolved = true;
            }
        }
        let early = if proto.intersection {
            Status::Error
        } else {
            Status::Success
        };
        for s in proto.singles.clone() {
            let st = match s {
                Single::Inter(g) => {
                    self.intersection_subtype_of_type(fe_scope, fe, proto_scope, &ZType::group(&g))
                }
                Single::Name(n) => {
                    let pn = self.resolve_name(proto_scope, &n);
                    self.intersection_subtype_of_class(fe_scope, fe, proto_scope, &pn)
                }
            };
            if st == early {
                return st;
            }
            if st == Status::Unresolved {
                unresolved = true;
            }
        }
        if unresolved {
            Status::Unresolved
        } else if early == Status::Error {
            Status::Success
        } else {
            Status::Error
        }
    }

    /// zend_perform_covariant_type_check: is `fe` (declared in `fe_scope`)
    /// a subtype of `proto` (declared in `proto_scope`)? Classes it cannot
    /// find make it `Unresolved` (and, outside hoisting, are registered for
    /// autoloading).
    pub(crate) fn covariant_type_check(
        &mut self,
        fe_scope: ClassId,
        fe: &ZType,
        proto_scope: ClassId,
        proto: &ZType,
    ) -> Status {
        if proto.mask == M_ANY && !fe.contains_void() {
            return Status::Success;
        }
        let mut added = fe.mask & !proto.mask;
        if added != 0 {
            if added & M_STATIC != 0 && self.type_permits_self(proto, proto_scope, fe_scope) {
                added &= !M_STATIC;
            }
            if added == M_NEVER {
                return Status::Success;
            }
            if added != 0 {
                return Status::Error;
            }
        }
        let mut unresolved = false;
        let early;
        if fe.intersection {
            early = if proto.intersection {
                Status::Error
            } else {
                Status::Success
            };
            let st = self.intersection_subtype_of_type(fe_scope, fe, proto_scope, proto);
            if st == early {
                return st;
            }
            if st == Status::Unresolved {
                unresolved = true;
            }
        } else {
            early = Status::Error;
            for s in fe.singles.clone() {
                let st = match s {
                    Single::Inter(g) => self.intersection_subtype_of_type(
                        fe_scope,
                        &ZType::group(&g),
                        proto_scope,
                        proto,
                    ),
                    Single::Name(n) => {
                        let n = self.resolve_name(fe_scope, &n);
                        self.class_subtype_of_type(fe_scope, &n, proto_scope, proto)
                    }
                };
                if st == early {
                    return st;
                }
                if st == Status::Unresolved {
                    unresolved = true;
                }
            }
        }
        if !unresolved {
            return if early == Status::Error {
                Status::Success
            } else {
                Status::Error
            };
        }
        self.register_unresolved(fe_scope, fe);
        self.register_unresolved(proto_scope, proto);
        Status::Unresolved
    }

    /// zend_do_perform_arg_type_hint_check.
    fn arg_type_check(
        &mut self,
        fe_scope: ClassId,
        fe: &SParam,
        proto_scope: ClassId,
        proto: &SParam,
    ) -> Status {
        let Some(fe_ty) = &fe.ty else {
            return Status::Success;
        };
        if fe_ty.mask == M_ANY && fe_ty.singles.is_empty() {
            return Status::Success;
        }
        let Some(proto_ty) = &proto.ty else {
            return Status::Error;
        };
        self.covariant_type_check(proto_scope, proto_ty, fe_scope, fe_ty)
    }

    /// zend_do_perform_implementation_check.
    fn implementation_check(
        &mut self,
        fe: &Sig,
        fe_scope: ClassId,
        proto: &Sig,
        proto_scope: ClassId,
    ) -> Status {
        if proto.required < fe.required {
            return Status::Error;
        }
        if proto.returns_ref && !fe.returns_ref {
            return Status::Error;
        }
        if proto.variadic && !fe.variadic {
            return Status::Error;
        }
        let pn = proto.params.len();
        let fn_ = fe.params.len();
        let mut status = Status::Success;
        for i in 0..pn.max(fn_) {
            let p = if i < pn {
                Some(&proto.params[i])
            } else if proto.variadic {
                proto.params.last()
            } else {
                None
            };
            let f = if i < fn_ {
                Some(&fe.params[i])
            } else if fe.variadic {
                fe.params.last()
            } else {
                None
            };
            let Some(p) = p else { continue };
            let Some(f) = f else {
                return Status::Error;
            };
            match self.arg_type_check(fe_scope, f, proto_scope, p) {
                Status::Success => {}
                Status::Error => return Status::Error,
                _ => status = Status::Unresolved,
            }
            if f.by_ref != p.by_ref {
                return Status::Error;
            }
        }
        if let Some(pr) = &proto.ret {
            let Some(fr) = &fe.ret else {
                if !proto.tentative {
                    return Status::Error;
                }
                return if status == Status::Success {
                    Status::Warning
                } else {
                    status
                };
            };
            let st = self.covariant_type_check(fe_scope, fr, proto_scope, pr);
            if st != Status::Success {
                if st == Status::Error && proto.tentative {
                    return Status::Warning;
                }
                return st;
            }
        }
        status
    }

    // ----- messages -------------------------------------------------------

    /// zend_type_to_string_resolved.
    fn ztype_string(&self, ty: &ZType, scope: ClassId) -> String {
        let resolve =
            |n: &PhpStr| lossy(cut_nul(self.resolve_name(scope, n).as_bytes())).into_owned();
        let mut parts: Vec<String> = Vec::new();
        if ty.intersection {
            let names: Vec<String> = ty
                .singles
                .iter()
                .filter_map(|s| match s {
                    Single::Name(n) => Some(lossy(n.as_bytes()).into_owned()),
                    Single::Inter(_) => None,
                })
                .collect();
            // An implicitly nullable intersection is the DNF `(A&B)|null`.
            if ty.mask & M_NULL != 0 {
                parts.push(format!("({})", names.join("&")));
            } else {
                parts.push(names.join("&"));
            }
        } else {
            for s in &ty.singles {
                match s {
                    Single::Inter(g) => {
                        let names: Vec<String> =
                            g.iter().map(|n| lossy(n.as_bytes()).into_owned()).collect();
                        parts.push(format!("({})", names.join("&")));
                    }
                    Single::Name(n) => parts.push(resolve(n)),
                }
            }
        }
        let m = ty.mask;
        if m == M_ANY {
            parts.push("mixed".into());
            return parts.join("|");
        }
        if m & M_STATIC != 0 {
            let called = (!self.inherit.compiling)
                .then(|| self.stack.last().and_then(|f| f.called_class))
                .flatten();
            parts.push(match called {
                Some(c) => lossy(self.class(c).name.as_bytes()).into_owned(),
                None => "static".into(),
            });
        }
        for (bit, word) in [
            (M_CALLABLE, "callable"),
            (M_OBJECT, "object"),
            (M_ARRAY, "array"),
            (M_STRING, "string"),
            (M_LONG, "int"),
            (M_DOUBLE, "float"),
        ] {
            if m & bit != 0 {
                parts.push(word.into());
            }
        }
        if m & M_BOOL == M_BOOL {
            parts.push("bool".into());
        } else if m & M_FALSE != 0 {
            parts.push("false".into());
        } else if m & M_TRUE != 0 {
            parts.push("true".into());
        }
        if m & M_VOID != 0 {
            parts.push("void".into());
        }
        if m & M_NEVER != 0 {
            parts.push("never".into());
        }
        let s = parts.join("|");
        if m & M_NULL != 0 {
            if s.is_empty() {
                return "null".into();
            }
            if !s.contains('|') && !s.contains('&') {
                return format!("?{s}");
            }
            return format!("{s}|null");
        }
        s
    }

    /// zend_get_function_declaration.
    fn declaration(&self, f: &Sig, scope: ClassId) -> String {
        let mut s = String::new();
        if f.returns_ref {
            s.push_str("& ");
        }
        s.push_str(&lossy(cut_nul(self.class(f.class).name.as_bytes())));
        s.push_str("::");
        s.push_str(&lossy(&f.name));
        s.push('(');
        for (i, p) in f.params.iter().enumerate() {
            if let Some(t) = &p.ty {
                s.push_str(&self.ztype_string(t, scope));
                s.push(' ');
            }
            if p.by_ref {
                s.push('&');
            }
            if p.variadic {
                s.push_str("...");
            }
            s.push('$');
            s.push_str(&lossy(&p.name));
            if let Some(d) = &p.default {
                s.push_str(" = ");
                s.push_str(d);
            }
            if i + 1 < f.params.len() {
                s.push_str(", ");
            }
        }
        s.push(')');
        if let Some(r) = &f.ret {
            s.push_str(": ");
            s.push_str(&self.ztype_string(r, scope));
        }
        s
    }

    /// A fatal E_COMPILE_ERROR at `file`:`line`, after the diagnostics
    /// recorded so far (shown without the user error handler).
    fn lsp_fatal(&mut self, msg: &str, file: &[u8], line: u32) -> Thrown {
        self.flush_recorded(false);
        self.fatal_compile_error_at(msg.as_bytes(), file.to_vec(), line as i64)
    }

    /// Emit the recorded diagnostics; `handler`: whether the user error
    /// handler sees them (not before a fatal error).
    pub(crate) fn flush_recorded(&mut self, handler: bool) {
        let rec = std::mem::take(&mut self.inherit.recorded);
        if rec.is_empty() {
            return;
        }
        let saved = (!handler).then(|| std::mem::take(&mut self.error_handlers));
        for (msg, file, line) in rec {
            self.raise_error_at(rhp_builtins::E_DEPRECATED, &msg, file, line);
        }
        if let Some(h) = saved {
            self.error_handlers = h;
        }
    }

    /// emit_incompatible_method_error.
    fn emit_method_error(
        &mut self,
        child: &Sig,
        child_scope: ClassId,
        parent: &Sig,
        parent_scope: ClassId,
        status: Status,
    ) -> Result<(), Thrown> {
        let pp = self.declaration(parent, parent_scope);
        let cp = self.declaration(child, child_scope);
        match status {
            Status::Unresolved => {
                let missing = self
                    .inherit
                    .delayed
                    .first()
                    .map(|n| lossy(n.as_bytes()).into_owned())
                    .unwrap_or_default();
                let msg = format!(
                    "Could not check compatibility between {cp} and {pp}, because class {missing} is not available"
                );
                Err(self.lsp_fatal(&msg, &child.file.clone(), child.line))
            }
            Status::Warning => {
                if !child.return_type_will_change {
                    let msg = format!(
                        "Return type of {cp} should either be compatible with {pp}, or the #[\\ReturnTypeWillChange] attribute should be used to temporarily suppress the notice"
                    );
                    self.inherit.recorded.push((
                        msg.into_bytes(),
                        child.file.clone(),
                        child.line as i64,
                    ));
                }
                Ok(())
            }
            _ => {
                let msg = format!("Declaration of {cp} must be compatible with {pp}");
                Err(self.lsp_fatal(&msg, &child.file.clone(), child.line))
            }
        }
    }

    /// do_inheritance_check_on_method. Returns the status (silent mode) and
    /// the child's new constructor prototype, if it gets one.
    #[allow(clippy::too_many_arguments)]
    fn check_method(
        &mut self,
        child: &Sig,
        child_scope: ClassId,
        parent: &Sig,
        parent_scope: ClassId,
        flags: u8,
        obligations: &mut Vec<Obligation>,
    ) -> Result<(Status, Option<ClassId>), Thrown> {
        let silent = flags & CHECK_SILENT != 0;
        if parent.vis == Visibility::Private && !parent.is_abstract && !parent.is_ctor {
            return Ok((Status::Success, None));
        }
        let child_class = lossy(cut_nul(self.class(child.class).name.as_bytes())).into_owned();
        let parent_class = lossy(cut_nul(self.class(parent.class).name.as_bytes())).into_owned();
        let fname = lossy(&child.name).into_owned();
        if flags & CHECK_PROTO != 0 {
            let msg = if parent.is_final {
                Some(format!(
                    "Cannot override final method {parent_class}::{fname}()"
                ))
            } else if child.is_static != parent.is_static {
                Some(if child.is_static {
                    format!("Cannot make non static method {parent_class}::{fname}() static in class {child_class}")
                } else {
                    format!("Cannot make static method {parent_class}::{fname}() non static in class {child_class}")
                })
            } else if child.is_abstract && !parent.is_abstract {
                Some(format!(
                    "Cannot make non abstract method {parent_class}::{fname}() abstract in class {child_class}"
                ))
            } else {
                None
            };
            if let Some(msg) = msg {
                if silent {
                    return Ok((Status::Error, None));
                }
                return Err(self.lsp_fatal(&msg, &child.file.clone(), child.line));
            }
        }
        // The prototype: the parent's own one, or the parent.
        let mut parent = parent.clone();
        let mut parent_scope = parent_scope;
        let proto_class = parent.prototype.unwrap_or(parent.class);
        let mut new_proto = None;
        if parent.is_ctor {
            let proto_sig = if proto_class == parent.class {
                Some(parent.clone())
            } else {
                self.class(proto_class)
                    .methods
                    .get(&b"__construct"[..])
                    .cloned()
                    .and_then(|m| self.method_sig(&m))
            };
            match proto_sig {
                Some(p) if p.is_abstract => {
                    if proto_class != parent.class {
                        parent_scope = p.class;
                    }
                    parent = p;
                }
                _ => return Ok((Status::Success, None)),
            }
        }
        if flags & SET_CHILD_PROTO != 0 {
            new_proto = Some(proto_class);
        }
        if flags & CHECK_VISIBILITY != 0 && vis_rank(child.vis) > vis_rank(parent.vis) {
            if silent {
                return Ok((Status::Error, None));
            }
            let msg = format!(
                "Access level to {child_class}::{fname}() must be {} (as in class {}){}",
                vis_word(parent.vis),
                lossy(cut_nul(self.class(parent.class).name.as_bytes())),
                if parent.vis == Visibility::Public {
                    ""
                } else {
                    " or weaker"
                }
            );
            return Err(self.lsp_fatal(&msg, &child.file.clone(), child.line));
        }
        if flags & CHECK_PROTO != 0 {
            let st = self.implementation_check(child, child_scope, &parent, parent_scope);
            if silent {
                return Ok((st, new_proto));
            }
            match st {
                Status::Success => {}
                Status::Unresolved => obligations.push(Obligation::Method {
                    child: child.clone(),
                    child_scope,
                    parent,
                    parent_scope,
                }),
                _ => self.emit_method_error(child, child_scope, &parent, parent_scope, st)?,
            }
        }
        Ok((Status::Success, new_proto))
    }

    // ----- properties ------------------------------------------------------

    fn prop_types_compatible(&mut self, parent: &PInfo, child: &PInfo) -> Status {
        match (&parent.ty, &child.ty) {
            (None, None) => return Status::Success,
            (Some(_), None) | (None, Some(_)) => return Status::Error,
            _ => {}
        }
        let (pt, ct) = (parent.ty.clone().unwrap(), child.ty.clone().unwrap());
        if pt.mask == ct.mask
            && pt.singles.len() == 1
            && ct.singles.len() == 1
            && matches!((&pt.singles[0], &ct.singles[0]), (Single::Name(a), Single::Name(b)) if a == b)
        {
            return Status::Success;
        }
        let s1 = if parent.variance == PropVariance::Contravariant {
            Status::Success
        } else {
            self.covariant_type_check(child.class, &ct, parent.class, &pt)
        };
        let s2 = if parent.variance == PropVariance::Covariant {
            Status::Success
        } else {
            self.covariant_type_check(parent.class, &pt, child.class, &ct)
        };
        if s1 == Status::Success && s2 == Status::Success {
            Status::Success
        } else if s1 == Status::Error || s2 == Status::Error {
            Status::Error
        } else {
            Status::Unresolved
        }
    }

    fn prop_error(&mut self, child: &PInfo, parent: &PInfo, ce: ClassId) -> Thrown {
        let ty = parent
            .ty
            .as_ref()
            .map(|t| self.ztype_string(t, parent.class))
            .unwrap_or_default();
        let rel = match parent.variance {
            PropVariance::Invariant => "",
            PropVariance::Covariant => "subtype of ",
            PropVariance::Contravariant => "supertype of ",
        };
        let msg = format!(
            "Type of {}::${} must be {rel}{ty} (as in class {})",
            lossy(cut_nul(self.class(child.class).name.as_bytes())),
            lossy(child.name.as_bytes()),
            lossy(cut_nul(self.class(parent.class).name.as_bytes())),
        );
        let (file, line) = (self.class(ce).decl_file.clone(), self.class(ce).decl_line);
        self.lsp_fatal(&msg, file.as_bytes(), line)
    }

    /// The typed-property part of do_inherit_property for one redeclared
    /// property.
    fn check_prop(
        &mut self,
        ce: ClassId,
        child: PInfo,
        parent: PInfo,
        silent: bool,
        obligations: &mut Vec<Obligation>,
    ) -> Result<Status, Thrown> {
        if parent.ty.is_some() {
            if silent && child.ty.is_none() {
                return Ok(Status::Success);
            }
            let st = self.prop_types_compatible(&parent, &child);
            if silent {
                return Ok(st);
            }
            match st {
                Status::Error => return Err(self.prop_error(&child, &parent, ce)),
                Status::Unresolved => obligations.push(Obligation::Prop { child, parent }),
                _ => {}
            }
        } else if child.ty.is_some() && !silent {
            let msg = format!(
                "Type of {}::${} must be omitted to match the parent definition in class {}",
                lossy(cut_nul(self.class(ce).name.as_bytes())),
                lossy(child.name.as_bytes()),
                lossy(cut_nul(self.class(parent.class).name.as_bytes())),
            );
            let (file, line) = (self.class(ce).decl_file.clone(), self.class(ce).decl_line);
            return Err(self.lsp_fatal(&msg, file.as_bytes(), line));
        }
        Ok(Status::Success)
    }

    /// The redeclared parent properties: (child, parent) pairs in the
    /// parent's order, instance properties first.
    fn redeclared_props(&self, ce: ClassId) -> Vec<(PInfo, PInfo)> {
        let mut out = Vec::new();
        let Some(p) = self.class(ce).parent else {
            return out;
        };
        let (c, pe) = (self.class(ce), self.class(p));
        let ty = |t: &Option<Rc<TypeHint>>| t.as_deref().map(ZType::from_hint);
        for pp in pe.props.values() {
            if pp.visibility == Visibility::Private {
                continue;
            }
            let Some(cp) = c
                .prop_visible
                .get(&pp.name)
                .and_then(|k| c.props.get(k))
                .filter(|cp| cp.declaring_class == ce)
            else {
                continue;
            };
            out.push((
                PInfo {
                    class: ce,
                    name: cp.name.clone(),
                    ty: ty(&cp.ty),
                    variance: PropVariance::Invariant,
                },
                PInfo {
                    class: pp.declaring_class,
                    name: pp.name.clone(),
                    ty: ty(&pp.ty),
                    variance: prop_variance(pp),
                },
            ));
        }
        for (k, ps) in &pe.static_props {
            if ps.visibility == Visibility::Private {
                continue;
            }
            let Some(cs) = c.static_props.get(k).filter(|cs| cs.declaring_class == ce) else {
                continue;
            };
            out.push((
                PInfo {
                    class: ce,
                    name: PhpStr::from(k.as_slice()),
                    ty: ty(&cs.ty),
                    variance: PropVariance::Invariant,
                },
                PInfo {
                    class: ps.declaring_class,
                    name: PhpStr::from(k.as_slice()),
                    ty: ty(&ps.ty),
                    variance: PropVariance::Invariant,
                },
            ));
        }
        out
    }

    // ----- the linking pass --------------------------------------------------

    /// The class's own declaration of method `key`, or a concrete trait
    /// method copied into it.
    fn own_method_sig(&self, ce: ClassId, key: &[u8]) -> Option<Sig> {
        let e = self.class(ce);
        if let Some(m) = e
            .proto
            .methods
            .iter()
            .find(|m| m.name.as_bytes().eq_ignore_ascii_case(key))
        {
            let proto = e.methods.get(key).and_then(|r| r.prototype);
            return Some(self.user_sig(
                &m.proto,
                ce,
                m.visibility,
                m.is_static,
                m.is_abstract,
                m.is_final,
                proto,
            ));
        }
        let m = e.methods.get(key)?;
        (m.defining_class == ce && !m.is_abstract)
            .then(|| self.method_sig(m))
            .flatten()
    }

    fn set_prototype(&mut self, ce: ClassId, key: &[u8], proto: Option<ClassId>, is_ctor: bool) {
        if let (Some(p), true) = (proto, is_ctor) {
            if let Some(m) = self.classes[ce as usize].methods.get_mut(key) {
                if m.defining_class == ce {
                    m.prototype = Some(p);
                }
            }
        }
    }

    /// zend_can_early_bind: the checks a hoisted class would fail, silently.
    fn can_early_bind(&mut self, ce: ClassId) -> Result<Status, Thrown> {
        let Some(parent) = self.class(ce).parent else {
            return Ok(Status::Success);
        };
        let mut overall = Status::Success;
        let mut none = Vec::new();
        let keys: Vec<Vec<u8>> = self.class(parent).methods.keys().cloned().collect();
        for k in keys {
            let Some(child) = self.own_method_sig(ce, &k) else {
                continue;
            };
            let pm = self.class(parent).methods[&k].clone();
            let Some(psig) = self.method_sig(&pm) else {
                continue;
            };
            let (st, _) = self.check_method(
                &child,
                ce,
                &psig,
                pm.defining_class,
                CHECK_SILENT | CHECK_PROTO | CHECK_VISIBILITY,
                &mut none,
            )?;
            match st {
                Status::Success => {}
                Status::Warning => overall = Status::Warning,
                st => return Ok(st),
            }
        }
        for (c, p) in self.redeclared_props(ce) {
            let st = self.check_prop(ce, c, p, true, &mut none)?;
            if st != Status::Success {
                return Ok(st);
            }
        }
        Ok(overall)
    }

    /// Run the inheritance checks for the just-linked user class `ce`.
    /// `early`: hoisting; returns `Ok(false)` when the class cannot be
    /// early-bound because a check needs a class that is not declared yet
    /// (the caller unlinks it again).
    pub(crate) fn inheritance_checks(&mut self, ce: ClassId, early: bool) -> Result<bool, Thrown> {
        let outer = std::mem::take(&mut self.inherit.recorded);
        let r = self.inheritance_checks_inner(ce, early);
        let rec = std::mem::replace(&mut self.inherit.recorded, outer);
        match r {
            Ok(true) => {
                let outer = std::mem::replace(&mut self.inherit.recorded, rec);
                self.flush_recorded(true);
                self.inherit.recorded = outer;
                if let Some(t) = self.pending_throw.take() {
                    return Err(t);
                }
                Ok(true)
            }
            other => other,
        }
    }

    fn inheritance_checks_inner(&mut self, ce: ClassId, early: bool) -> Result<bool, Thrown> {
        if early {
            let prev = std::mem::replace(&mut self.inherit.compiling, true);
            let st = self.can_early_bind(ce);
            self.inherit.compiling = prev;
            if st? == Status::Unresolved {
                return Ok(false);
            }
        }
        let prev = std::mem::replace(&mut self.inherit.compiling, early);
        let mut obligations = Vec::new();
        let r = self.link_pass(ce, &mut obligations);
        self.inherit.compiling = prev;
        r?;
        if !obligations.is_empty() {
            self.load_delayed_classes(ce)?;
            for o in obligations {
                match o {
                    Obligation::Method {
                        child,
                        child_scope,
                        parent,
                        parent_scope,
                    } => {
                        let st =
                            self.implementation_check(&child, child_scope, &parent, parent_scope);
                        if st != Status::Success {
                            self.emit_method_error(&child, child_scope, &parent, parent_scope, st)?;
                        }
                    }
                    Obligation::Prop { child, parent } => {
                        if self.prop_types_compatible(&parent, &child) != Status::Success {
                            return Err(self.prop_error(&child, &parent, ce));
                        }
                    }
                }
            }
        }
        self.implement_serializable(ce)?;
        Ok(true)
    }

    /// zend_implement_serializable: a class implementing Serializable
    /// without both __serialize and __unserialize is deprecated; an enum
    /// may not implement it at all (zend_verify_enum_interfaces).
    fn implement_serializable(&mut self, ce: ClassId) -> Result<(), Thrown> {
        let e = self.class(ce);
        if e.kind == ClassKind::Interface
            || !e
                .interfaces
                .iter()
                .any(|&i| self.class(i).name.as_bytes() == b"Serializable")
        {
            return Ok(());
        }
        let (file, line) = (e.decl_file.as_bytes().to_vec(), e.decl_line);
        let name = lossy(cut_nul(e.name.as_bytes())).into_owned();
        let is_enum = e.kind == ClassKind::Enum;
        if !e.flags.is_abstract
            && !(e.methods.contains_key(&b"__serialize"[..])
                && e.methods.contains_key(&b"__unserialize"[..]))
        {
            let msg = format!(
                "{name} implements the Serializable interface, which is deprecated. Implement __serialize() and __unserialize() instead (or in addition, if support for old PHP versions is necessary)"
            );
            self.inherit
                .recorded
                .push((msg.into_bytes(), file.clone(), line as i64));
        }
        if is_enum {
            let msg = format!("Enum {name} cannot implement the Serializable interface");
            return Err(self.lsp_fatal(&msg, &file, line));
        }
        Ok(())
    }

    /// load_delayed_classes: autoload what the variance checks could not
    /// find; an exception from an autoloader is fatal.
    fn load_delayed_classes(&mut self, ce: ClassId) -> Result<(), Thrown> {
        while !self.inherit.delayed.is_empty() {
            let name = self.inherit.delayed.remove(0);
            if let Err(t) = self.try_autoload(name.as_bytes()) {
                let prefix = format!(
                    "During inheritance of {}, while autoloading {}",
                    lossy(cut_nul(self.class(ce).name.as_bytes())),
                    lossy(name.as_bytes())
                );
                return Err(self.uncaught_fatal(&prefix, t));
            }
        }
        Ok(())
    }

    /// The method checks of zend_do_link_class, in php's order: properties
    /// and methods against the parent, trait abstract methods, then the
    /// interfaces the parent does not already implement.
    fn link_pass(&mut self, ce: ClassId, obligations: &mut Vec<Obligation>) -> Result<(), Thrown> {
        let parent = self.class(ce).parent;
        if let Some(pid) = parent {
            for (c, p) in self.redeclared_props(ce) {
                self.check_hooks(ce, pid, &c.name, obligations)?;
                self.check_prop(ce, c, p, false, obligations)?;
            }
        }
        if let Some(parent) = parent {
            let keys: Vec<Vec<u8>> = self.class(parent).methods.keys().cloned().collect();
            for k in keys {
                let Some(child) = self.own_method_sig(ce, &k) else {
                    continue;
                };
                let pm = self.class(parent).methods[&k].clone();
                let Some(psig) = self.method_sig(&pm) else {
                    continue;
                };
                let (_, np) = self.check_method(
                    &child,
                    ce,
                    &psig,
                    pm.defining_class,
                    CHECK_PROTO | CHECK_VISIBILITY | SET_CHILD_PROTO,
                    obligations,
                )?;
                self.set_prototype(ce, &k, np, child.is_ctor);
            }
        }
        // Abstract trait methods: what the class (or its parent) has must
        // satisfy them; visibility is not checked.
        let traits = self.class(ce).traits.clone();
        for t in traits {
            let tm: Vec<(Vec<u8>, ResolvedMethod)> = self
                .class(t)
                .methods
                .iter()
                .filter(|(_, m)| m.is_abstract)
                .map(|(k, m)| (k.clone(), m.clone()))
                .collect();
            for (k, m) in tm {
                let existing = match self.own_method_sig(ce, &k) {
                    Some(s) => Some((s, ce)),
                    None => parent
                        .and_then(|p| self.class(p).methods.get(&k).cloned())
                        .and_then(|pm| self.method_sig(&pm).map(|s| (s, pm.defining_class))),
                };
                let Some((child, child_scope)) = existing else {
                    continue;
                };
                let Some(tsig) = self.method_sig(&m) else {
                    continue;
                };
                self.check_method(&child, child_scope, &tsig, ce, CHECK_PROTO, obligations)?;
            }
        }
        // Interfaces new to this class.
        let inherited: Vec<ClassId> = parent
            .map(|p| self.class(p).interfaces.clone())
            .unwrap_or_default();
        let is_iface = self.class(ce).kind == ClassKind::Interface;
        let ifaces: Vec<ClassId> = self
            .class(ce)
            .interfaces
            .iter()
            .copied()
            .filter(|i| !inherited.contains(i))
            .collect();
        for iface in ifaces.iter().copied() {
            let names: Vec<PhpStr> = self
                .class(iface)
                .props
                .values()
                .map(|p| p.name.clone())
                .collect();
            for n in names {
                self.check_hooks(ce, iface, &n, obligations)?;
            }
        }
        for iface in ifaces {
            let ims: Vec<(Vec<u8>, ResolvedMethod)> = self
                .class(iface)
                .methods
                .iter()
                .map(|(k, m)| (k.clone(), m.clone()))
                .collect();
            for (k, im) in ims {
                let (child, child_scope) = match self.own_method_sig(ce, &k) {
                    Some(s) => (s, ce),
                    None => {
                        let Some(m) = self.class(ce).methods.get(&k).cloned() else {
                            continue;
                        };
                        if Rc::ptr_eq(&m.proto, &im.proto) && m.defining_class == im.defining_class
                        {
                            continue;
                        }
                        let Some(s) = self.method_sig(&m) else {
                            continue;
                        };
                        (s, m.defining_class)
                    }
                };
                let Some(isig) = self.method_sig(&im) else {
                    continue;
                };
                let mut flags = CHECK_PROTO | CHECK_VISIBILITY;
                if !is_iface {
                    flags |= SET_CHILD_PROTO;
                }
                let (_, np) = self.check_method(
                    &child,
                    child_scope,
                    &isig,
                    im.defining_class,
                    flags,
                    obligations,
                )?;
                if child_scope == ce {
                    self.set_prototype(ce, &k, np, child.is_ctor);
                }
            }
        }
        Ok(())
    }

    /// inherit_property_hook's signature check: each hook `ce` declares
    /// for property `name` against the same hook of `from` (the parent or
    /// an interface).
    fn check_hooks(
        &mut self,
        ce: ClassId,
        from: ClassId,
        name: &PhpStr,
        obligations: &mut Vec<Obligation>,
    ) -> Result<(), Thrown> {
        let prop = |vm: &Vm, c: ClassId| {
            let e = vm.class(c);
            e.prop_visible
                .get(name)
                .and_then(|k| e.props.get(k))
                .and_then(|p| p.hooks.clone().map(|h| (p.visibility, h)))
        };
        let (Some((_, ch)), Some((pvis, ph))) = (prop(self, ce), prop(self, from)) else {
            return Ok(());
        };
        if pvis == Visibility::Private {
            return Ok(());
        }
        for kind in 0..2 {
            let (Some(c), Some(p)) = (&ch[kind], &ph[kind]) else {
                continue;
            };
            if c.defining_class != ce
                || p.visibility == Visibility::Private
                || p.is_final
                || Rc::ptr_eq(&c.proto, &p.proto)
            {
                continue;
            }
            let (Some(csig), Some(psig)) = (self.method_sig(c), self.method_sig(p)) else {
                continue;
            };
            self.check_method(
                &csig,
                ce,
                &psig,
                p.defining_class,
                CHECK_PROTO | CHECK_VISIBILITY,
                obligations,
            )?;
        }
        Ok(())
    }

    /// Track a run-time class declaration while it loads its dependencies.
    pub(crate) fn push_linking(&mut self, proto: &Rc<ClassProto>) {
        let key = proto.name.as_bytes().to_ascii_lowercase();
        self.inherit.linking.push((key, proto.clone(), false));
    }

    /// Stop tracking it; returns whether a variance check used it.
    pub(crate) fn pop_linking(&mut self) -> bool {
        self.inherit.linking.pop().is_some_and(|e| e.2)
    }

    /// zend_exception_uncaught_error: a fatal E_ERROR
    /// "<prefix>: Uncaught <exception as a string>".
    pub(crate) fn uncaught_fatal(&mut self, prefix: &str, t: Thrown) -> Thrown {
        if !t.is_catchable() {
            return t;
        }
        let s = self.throwable_string(t);
        let mut msg = format!("{prefix}: Uncaught ").into_bytes();
        msg.extend_from_slice(&s);
        self.fatal_error_here(1, &msg)
    }
}
