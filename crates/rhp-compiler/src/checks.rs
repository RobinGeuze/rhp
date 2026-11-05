//! php's declaration-level compile checks (zend_compile_class_decl and the
//! member compilers it calls, zend_begin_method_decl, zend_compile_params'
//! promotion checks, zend_check_magic_method_implementation): reserved
//! names, member redeclaration, interface/enum/readonly rules, magic method
//! signatures. Each check records an E_COMPILE_ERROR with [`FileCtx::fatal`]
//! (the earliest one wins) or a compile diagnostic.

use std::collections::HashSet;

use crate::parse::cst::{
    AttributeList, ClassLikeMember, FunctionLikeParameterList, Hint, Identifier, Method,
    MethodBody, Modifier, Property, PropertyItem, Sequence, Statement, TraitUseAdaptation,
    TraitUseMethodReference, TraitUseSpecification,
};
use mago_span::HasSpan;
use rhp_bytecode::{TypeHint, TypeKind};
use rhp_values::{PhpStr, Visibility, E_COMPILE_WARNING, E_DEPRECATED, E_WARNING};

use crate::decl::{has_modifier, modifiers_visibility};
use crate::file::FileCtx;
use crate::names::identifier_raw;
use crate::util::strip_dollar;

/// zend_compile.c `reserved_class_names`.
pub(crate) const RESERVED_CLASS_NAMES: [&[u8]; 17] = [
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

/// zend_is_reserved_class_name: the unqualified tail of `name` is reserved.
pub(crate) fn is_reserved_class_name(name: &[u8]) -> bool {
    let tail = name.rsplit(|&b| b == b'\\').next().unwrap_or(name);
    RESERVED_CLASS_NAMES
        .iter()
        .any(|r| r.eq_ignore_ascii_case(tail))
}

/// What kind of class-like a declaration is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeclKind {
    Class,
    Interface,
    Trait,
    Enum,
    Anon,
}

/// The parts of a class-like declaration the checks look at.
pub(crate) struct Decl<'a, 'arena> {
    pub kind: DeclKind,
    /// The class name as php prints it (namespaced; an anonymous class's up
    /// to its NUL byte).
    pub name: &'a PhpStr,
    /// The declared (unqualified) name; none for an anonymous class.
    pub short: Option<&'a [u8]>,
    /// Offset of the class keyword: decl-level errors are reported there.
    pub at: u32,
    pub extends: Option<&'a crate::parse::cst::Extends<'arena>>,
    pub implements: Option<&'a crate::parse::cst::Implements<'arena>>,
    pub attributes: &'a Sequence<'arena, AttributeList<'arena>>,
    pub is_abstract: bool,
    pub is_readonly: bool,
    pub backing: Option<&'a Hint<'arena>>,
    pub members: &'a Sequence<'arena, ClassLikeMember<'arena>>,
}

/// The class name as php's `%s` prints it (C strings stop at NUL).
fn display_name(name: &PhpStr) -> String {
    let b = name.as_bytes();
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn s(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Run every declaration check of zend_compile_class_decl on `d`.
pub(crate) fn check_decl(cx: &FileCtx, d: &Decl) {
    if cx.is_prelude {
        return;
    }
    // Types are lowered in the class's scope (`self`, `static`).
    let parent = match d.kind {
        DeclKind::Class | DeclKind::Anon => d
            .extends
            .and_then(|e| e.types.iter().next())
            .map(|id| cx.resolve_class_name(id)),
        _ => None,
    };
    let scope = crate::file::ClassScope {
        name: d.name.as_bytes().to_vec(),
        parent,
        is_trait: d.kind == DeclKind::Trait,
        is_anon: d.kind == DeclKind::Anon,
    };
    cx.with_class_scope(scope, || check_decl_in_scope(cx, d));
}

fn check_decl_in_scope(cx: &FileCtx, d: &Decl) {
    let cname = display_name(d.name);
    // zend_assert_valid_class_name
    if let Some(short) = d.short {
        let what = match d.kind {
            DeclKind::Enum => "an enum name",
            DeclKind::Interface => "an interface name",
            DeclKind::Trait => "a trait name",
            _ => "a class name",
        };
        if is_reserved_class_name(short) {
            cx.fatal(
                d.at,
                format!("Cannot use \"{}\" as {what} as it is reserved", s(short)),
            );
        }
        if short == b"_" {
            cx.diag(
                E_DEPRECATED,
                d.at,
                format!("Using \"_\" as {what} is deprecated since 8.4"),
            );
        }
    }
    // Interfaces list their parents under `extends`, compiled like an
    // implements list.
    let extends_label = if d.kind == DeclKind::Interface {
        "interface name"
    } else {
        "class name"
    };
    if let Some(e) = d.extends {
        for id in e.types.iter() {
            check_class_ref(cx, id, extends_label);
        }
    }
    check_class_attributes(cx, d, &cname);
    if let Some(i) = d.implements {
        for id in i.types.iter() {
            check_class_ref(cx, id, "interface name");
        }
    }
    let mut backed = false;
    if let Some(h) = d.backing {
        // A type with a compile error of its own fails on that instead.
        if let Some(th) = type_hint(cx, h) {
            let ok = !th.nullable
                && th.groups.is_empty()
                && matches!(th.kinds.as_slice(), [TypeKind::Int] | [TypeKind::String]);
            if !ok {
                cx.fatal(
                    h.span().start.offset,
                    format!(
                        "Enum backing type must be int or string, {} given",
                        type_string(&th)
                    ),
                );
            }
        }
        backed = true;
    }
    check_members(cx, d, &cname, backed);
}

/// zend_resolve_const_class_name_reference: `self`, `parent` and `static`
/// name no class here.
fn check_class_ref(cx: &FileCtx, id: &Identifier, what: &str) {
    if let Identifier::Local(l) = id {
        let n = l.value;
        if [&b"self"[..], b"parent", b"static"]
            .iter()
            .any(|r| r.eq_ignore_ascii_case(n))
        {
            cx.fatal(
                id.span().start.offset,
                format!("Cannot use \"{}\" as {what}, as it is reserved", s(n)),
            );
        }
    }
}

/// The class attributes' validators (zend_attributes.c).
fn check_class_attributes(cx: &FileCtx, d: &Decl, cname: &str) {
    let mut delayed = false;
    let mut dynamic = None;
    for l in d.attributes.iter() {
        for a in l.attributes.iter() {
            let n = cx.resolve_class_name(&a.name).to_ascii_lowercase();
            match n.as_slice() {
                b"delayedtargetvalidation" => delayed = true,
                b"allowdynamicproperties" => dynamic = Some(d.at),
                _ => {}
            }
        }
    }
    let Some(at) = dynamic else { return };
    let what = match d.kind {
        DeclKind::Trait => "trait",
        DeclKind::Interface => "interface",
        _ if d.is_readonly => "readonly class",
        DeclKind::Enum => "enum",
        _ => return,
    };
    if !delayed {
        cx.fatal(
            at,
            format!("Cannot apply #[\\AllowDynamicProperties] to {what} {cname}"),
        );
    }
}

fn check_members(cx: &FileCtx, d: &Decl, cname: &str, backed: bool) {
    let mut consts: HashSet<Vec<u8>> = HashSet::new();
    let mut props: HashSet<Vec<u8>> = HashSet::new();
    let mut methods: HashSet<Vec<u8>> = HashSet::new();
    for m in d.members.iter() {
        match m {
            ClassLikeMember::TraitUse(tu) => {
                if d.kind == DeclKind::Interface {
                    if let Some(t) = tu.trait_names.iter().next() {
                        cx.fatal(
                            tu.span().start.offset,
                            format!(
                                "Cannot use traits inside of interfaces. {} is used in {cname}",
                                s(identifier_raw(t))
                            ),
                        );
                    }
                }
                for t in tu.trait_names.iter() {
                    check_class_ref(cx, t, "trait name");
                }
                if let TraitUseSpecification::Concrete(c) = &tu.specification {
                    for a in c.adaptations.iter() {
                        match a {
                            TraitUseAdaptation::Precedence(p) => {
                                check_class_ref(cx, &p.method_reference.trait_name, "trait name");
                                for t in p.trait_names.iter() {
                                    check_class_ref(cx, t, "trait name");
                                }
                            }
                            TraitUseAdaptation::Alias(al) => {
                                if let TraitUseMethodReference::Absolute(r) = &al.method_reference {
                                    check_class_ref(cx, &r.trait_name, "trait name");
                                }
                            }
                        }
                    }
                }
            }
            ClassLikeMember::Constant(c) => {
                let vis = modifiers_visibility(&c.modifiers);
                for item in c.items.iter() {
                    let at = item.name.span.start.offset;
                    let n = item.name.value;
                    if d.kind == DeclKind::Interface && vis != Visibility::Public {
                        cx.fatal(
                            at,
                            format!(
                                "Access type for interface constant {cname}::{} must be public",
                                s(n)
                            ),
                        );
                    }
                    if n.eq_ignore_ascii_case(b"class") {
                        cx.fatal(
                            at,
                            "A class constant must not be called 'class'; it is reserved for class name fetching".into(),
                        );
                    }
                    if !consts.insert(n.to_vec()) {
                        cx.fatal(
                            at,
                            format!("Cannot redefine class constant {cname}::{}", s(n)),
                        );
                    }
                }
            }
            ClassLikeMember::EnumCase(ec) => {
                use crate::parse::cst::EnumCaseItem;
                let at = ec.span().start.offset;
                if d.kind != DeclKind::Enum {
                    cx.fatal(at, "Case can only be used in enums".into());
                    continue;
                }
                let (n, has_value) = match &ec.item {
                    EnumCaseItem::Unit(u) => (u.name.value, false),
                    EnumCaseItem::Backed(b) => (b.name.value, true),
                };
                if backed && !has_value {
                    cx.fatal(
                        at,
                        format!("Case {} of backed enum {cname} must have a value", s(n)),
                    );
                } else if !backed && has_value {
                    cx.fatal(
                        at,
                        format!(
                            "Case {} of non-backed enum {cname} must not have a value",
                            s(n)
                        ),
                    );
                }
                if !consts.insert(n.to_vec()) {
                    cx.fatal(
                        at,
                        format!("Cannot redefine class constant {cname}::{}", s(n)),
                    );
                }
            }
            ClassLikeMember::Property(p) => check_property(cx, d, cname, p, &mut props),
            ClassLikeMember::Method(m) => {
                check_method(cx, d, cname, m, &mut methods, &mut props);
            }
        }
    }
}

/// zend_compile_prop_decl's checks.
fn check_property(cx: &FileCtx, d: &Decl, cname: &str, p: &Property, props: &mut HashSet<Vec<u8>>) {
    let at = p.span().start.offset;
    if d.kind == DeclKind::Enum {
        cx.fatal(at, format!("Enum {cname} cannot include properties"));
        return;
    }
    let (mods, hint, items, hooked): (_, _, Vec<&PropertyItem>, bool) = match p {
        Property::Plain(pp) => (
            &pp.modifiers,
            pp.hint.as_ref(),
            pp.items.iter().collect(),
            false,
        ),
        Property::Hooked(h) => (&h.modifiers, h.hint.as_ref(), vec![&h.item], true),
    };
    if d.kind == DeclKind::Interface && !hooked {
        cx.fatal(at, "Interfaces may only include hooked properties".into());
        return;
    }
    let is_static = has_modifier(mods, |m| matches!(m, Modifier::Static(_)));
    let readonly = has_modifier(mods, |m| matches!(m, Modifier::Readonly(_))) || d.is_readonly;
    for item in items {
        let (var, has_default) = match item {
            PropertyItem::Concrete(c) => (&c.variable, true),
            PropertyItem::Abstract(a) => (&a.variable, false),
        };
        let name = strip_dollar(var.name);
        let vat = var.span.start.offset;
        if !props.insert(name.to_vec()) {
            cx.fatal(vat, format!("Cannot redeclare {cname}::${}", s(name)));
        }
        if readonly && !hooked {
            if hint.is_none() {
                cx.fatal(
                    vat,
                    format!("Readonly property {cname}::${} must have type", s(name)),
                );
            } else if has_default {
                cx.fatal(
                    vat,
                    format!(
                        "Readonly property {cname}::${} cannot have default value",
                        s(name)
                    ),
                );
            } else if is_static {
                cx.fatal(
                    vat,
                    format!("Static property {cname}::${} cannot be readonly", s(name)),
                );
            }
        }
    }
}

/// zend_begin_method_decl's checks, the promotion checks of
/// zend_compile_params, and zend_check_magic_method_implementation.
fn check_method(
    cx: &FileCtx,
    d: &Decl,
    cname: &str,
    m: &Method,
    methods: &mut HashSet<Vec<u8>>,
    props: &mut HashSet<Vec<u8>>,
) {
    let at = m.function.span.start.offset;
    let name = m.name.value;
    let mname = s(name);
    let lc = name.to_ascii_lowercase();
    let mods = &m.modifiers;
    let vis = modifiers_visibility(mods);
    let is_final = has_modifier(mods, |x| matches!(x, Modifier::Final(_)));
    let mut is_abstract = has_modifier(mods, |x| matches!(x, Modifier::Abstract(_)));
    let has_body = matches!(m.body, MethodBody::Concrete(_));
    let is_ctor = lc == b"__construct";
    if has_modifier(mods, |x| matches!(x, Modifier::Readonly(_))) {
        cx.fatal(at, "Cannot use 'readonly' as method modifier".into());
    }
    if vis == Visibility::Private && is_final && !is_ctor {
        cx.diag(
            E_COMPILE_WARNING,
            at,
            "Private methods cannot be final as they are never overridden by other classes".into(),
        );
    }
    if is_abstract && !(d.is_abstract || d.kind == DeclKind::Trait) {
        let msg = match d.kind {
            DeclKind::Anon => format!("Anonymous class method {mname}() must not be abstract"),
            DeclKind::Enum => format!("Enum method {cname}::{mname}() must not be abstract"),
            DeclKind::Interface => {
                format!("Interface method {cname}::{mname}() must not be abstract")
            }
            _ => format!(
                "Class {cname} declares abstract method {mname}() and must therefore be declared abstract"
            ),
        };
        cx.fatal(at, msg);
    }
    let in_interface = d.kind == DeclKind::Interface;
    if in_interface {
        if vis != Visibility::Public {
            cx.fatal(
                at,
                format!("Access type for interface method {cname}::{mname}() must be public"),
            );
        }
        if is_final {
            cx.fatal(
                at,
                format!("Interface method {cname}::{mname}() must not be final"),
            );
        }
        is_abstract = true;
    }
    let kind_word = if in_interface {
        "Interface"
    } else {
        "Abstract"
    };
    if is_abstract {
        if vis == Visibility::Private && d.kind != DeclKind::Trait {
            cx.fatal(
                at,
                format!("{kind_word} function {cname}::{mname}() cannot be declared private"),
            );
        }
        if has_body {
            cx.fatal(
                at,
                format!("{kind_word} function {cname}::{mname}() cannot contain body"),
            );
        }
    } else if !has_body {
        cx.fatal(
            at,
            format!("Non-abstract method {cname}::{mname}() must contain body"),
        );
    }
    if !methods.insert(lc.clone()) {
        cx.fatal(at, format!("Cannot redeclare {cname}::{mname}()"));
    }
    check_promotion(
        cx,
        d,
        cname,
        Some((is_ctor, is_abstract)),
        &m.parameter_list,
        props,
    );
    // After the body in php: ordered at the method's end, reported at its
    // first line.
    let line = cx.line_of(at);
    for msg in magic_method_errors(cx, cname, m, &lc) {
        match msg {
            Magic::Error(msg) => cx.fatal_line(m.span().end.offset, line, msg),
            Magic::Warning(msg) => cx.compile_diags.borrow_mut().push((E_WARNING, msg, line)),
        }
    }
}

/// Promoted parameters outside a class constructor (`method` is `None` for
/// a free function).
pub(crate) fn check_promotion(
    cx: &FileCtx,
    d: &Decl,
    cname: &str,
    method: Option<(bool, bool)>,
    params: &FunctionLikeParameterList,
    props: &mut HashSet<Vec<u8>>,
) {
    let mut seen: HashSet<&[u8]> = HashSet::new();
    for p in params.parameters.iter() {
        if redefined_param(cx, p, &mut seen) || p.modifiers.is_empty() {
            continue;
        }
        let at = p.span().start.offset;
        let (is_ctor, is_abstract) = method.unwrap_or((false, false));
        if !is_ctor {
            cx.fatal(
                at,
                "Cannot declare promoted property outside a constructor".into(),
            );
            continue;
        }
        if is_abstract || d.kind == DeclKind::Interface {
            cx.fatal(
                at,
                "Cannot declare promoted property in an abstract constructor".into(),
            );
            continue;
        }
        if p.ellipsis.is_some() {
            cx.fatal(at, "Cannot declare variadic promoted property".into());
            continue;
        }
        let name = strip_dollar(p.variable.name);
        if !props.insert(name.to_vec()) {
            cx.fatal(at, format!("Cannot redeclare {cname}::${}", s(name)));
            continue;
        }
        let readonly =
            d.is_readonly || has_modifier(&p.modifiers, |m| matches!(m, Modifier::Readonly(_)));
        if readonly && p.hint.is_none() && p.hooks.is_none() {
            cx.fatal(
                at,
                format!("Readonly property {cname}::${} must have type", s(name)),
            );
        }
    }
}

/// A free function's parameters: none may be promoted.
pub(crate) fn check_function_params(cx: &FileCtx, params: &FunctionLikeParameterList) {
    if cx.is_prelude {
        return;
    }
    let mut seen: HashSet<&[u8]> = HashSet::new();
    for p in params.parameters.iter() {
        if redefined_param(cx, p, &mut seen) {
            continue;
        }
        if !p.modifiers.is_empty() {
            cx.fatal(
                p.span().start.offset,
                "Cannot declare promoted property outside a constructor".into(),
            );
        }
    }
}

/// zend_compile_params: "Redefinition of parameter $x".
fn redefined_param<'a>(
    cx: &FileCtx,
    p: &'a crate::parse::cst::FunctionLikeParameter,
    seen: &mut HashSet<&'a [u8]>,
) -> bool {
    let name = strip_dollar(p.variable.name);
    if seen.insert(name) {
        return false;
    }
    cx.fatal(
        p.span().start.offset,
        format!("Redefinition of parameter ${}", s(name)),
    );
    true
}

enum Magic {
    Error(String),
    Warning(String),
}

/// php's MAY_BE_* bits for the magic method type checks.
const NULL: u32 = 1 << 1;
const FALSE: u32 = 1 << 2;
const TRUE: u32 = 1 << 3;
const LONG: u32 = 1 << 4;
const DOUBLE: u32 = 1 << 5;
const STRING: u32 = 1 << 6;
const ARRAY: u32 = 1 << 7;
const OBJECT: u32 = 1 << 8;
const RESOURCE: u32 = 1 << 9;
const CALLABLE: u32 = 1 << 17;
const VOID: u32 = 1 << 18;
const STATIC: u32 = 1 << 20;
const NEVER: u32 = 1 << 21;
const BOOL: u32 = FALSE | TRUE;
const ANY: u32 = NULL | BOOL | LONG | DOUBLE | STRING | ARRAY | OBJECT | RESOURCE;

/// ZEND_TYPE_PURE_MASK of a declared type, and whether it names classes
/// (ZEND_TYPE_IS_COMPLEX).
/// The lowered type of `h`, without raising its compile errors (the type
/// itself is compiled, with its errors, later). `None` if it has one.
fn type_hint(cx: &FileCtx, h: &Hint) -> Option<TypeHint> {
    cx.quietly(|| crate::hints::compile_type(cx, h, false))
        .map(|t| t.hint)
}

fn type_mask(t: &TypeHint) -> (u32, bool) {
    let mut mask = if t.nullable { NULL } else { 0 };
    let mut complex = !t.groups.is_empty();
    for k in &t.kinds {
        mask |= match k {
            TypeKind::Int => LONG,
            TypeKind::Float => DOUBLE,
            TypeKind::String => STRING,
            TypeKind::Bool => BOOL,
            TypeKind::True => TRUE,
            TypeKind::False => FALSE,
            TypeKind::Array => ARRAY,
            TypeKind::Object => OBJECT,
            TypeKind::Callable => CALLABLE,
            TypeKind::Iterable => {
                complex = true;
                ARRAY
            }
            TypeKind::Mixed => ANY,
            TypeKind::Void => VOID,
            TypeKind::Never => NEVER,
            TypeKind::Null => NULL,
            TypeKind::StaticType => STATIC,
            _ => {
                complex = true;
                0
            }
        };
    }
    (mask, complex)
}

/// zend_check_magic_method_implementation for the method `m` (`lc` is its
/// lowercase name) of class `cname`.
fn magic_method_errors(cx: &FileCtx, cname: &str, m: &Method, lc: &[u8]) -> Vec<Magic> {
    if !lc.starts_with(b"__") {
        return Vec::new();
    }
    let mname = s(m.name.value);
    let is_static = has_modifier(&m.modifiers, |x| matches!(x, Modifier::Static(_)));
    let public = modifiers_visibility(&m.modifiers) == Visibility::Public;
    let params: Vec<_> = m.parameter_list.parameters.iter().collect();
    let ret = m
        .return_type_hint
        .as_ref()
        .and_then(|r| type_hint(cx, &r.hint));

    let args = |n: usize| -> Option<Magic> {
        if params.len() != n {
            return Some(Magic::Error(match n {
                0 => format!("Method {cname}::{mname}() cannot take arguments"),
                1 => format!("Method {cname}::{mname}() must take exactly 1 argument"),
                _ => format!("Method {cname}::{mname}() must take exactly {n} arguments"),
            }));
        }
        params.iter().any(|p| p.ampersand.is_some()).then(|| {
            Magic::Error(format!(
                "Method {cname}::{mname}() cannot take arguments by reference"
            ))
        })
    };
    let non_static =
        || is_static.then(|| Magic::Error(format!("Method {cname}::{mname}() cannot be static")));
    let must_static =
        || (!is_static).then(|| Magic::Error(format!("Method {cname}::{mname}() must be static")));
    let public_ = || {
        (!public).then(|| {
            Magic::Warning(format!(
                "The magic method {cname}::{mname}() must have public visibility"
            ))
        })
    };
    let arg_type = |i: usize, want: u32, name: &str| -> Option<Magic> {
        let p = params.get(i)?;
        let h = p.hint.as_ref()?;
        let (mask, _) = type_mask(&type_hint(cx, h)?);
        (mask & want == 0).then(|| {
            Magic::Error(format!(
                "{cname}::{mname}(): Parameter #{} (${}) must be of type {name} when declared",
                i + 1,
                s(strip_dollar(p.variable.name))
            ))
        })
    };
    let ret_type = |want: u32, name: &str| -> Option<Magic> {
        let t = ret.as_ref()?;
        let (mask, mut complex) = type_mask(t);
        if mask & NEVER != 0 {
            return None;
        }
        let mut extra = mask & !want;
        if extra & STATIC != 0 {
            extra &= !STATIC;
            complex = true;
        }
        (extra != 0 || (complex && want != OBJECT)).then(|| {
            Magic::Error(format!(
                "{cname}::{mname}(): Return type must be {name} when declared"
            ))
        })
    };
    let no_ret = || {
        ret.is_some().then(|| {
            Magic::Error(format!(
                "Method {cname}::{mname}() cannot declare a return type"
            ))
        })
    };
    // Each check raises in order; warnings do not stop the later checks.
    let mut warning = None;
    let mut run = |r: Option<Magic>| -> Option<Magic> {
        match r {
            Some(Magic::Warning(w)) => {
                warning = Some(Magic::Warning(w));
                None
            }
            e => e,
        }
    };
    let err = match lc {
        b"__construct" => run(non_static()).or_else(|| run(no_ret())),
        b"__destruct" => run(args(0))
            .or_else(|| run(non_static()))
            .or_else(|| run(no_ret())),
        b"__clone" => run(args(0))
            .or_else(|| run(non_static()))
            .or_else(|| run(ret_type(VOID, "void"))),
        b"__get" => run(args(1))
            .or_else(|| run(non_static()))
            .or_else(|| run(public_()))
            .or_else(|| run(arg_type(0, STRING, "string"))),
        b"__set" => run(args(2))
            .or_else(|| run(non_static()))
            .or_else(|| run(public_()))
            .or_else(|| run(arg_type(0, STRING, "string")))
            .or_else(|| run(ret_type(VOID, "void"))),
        b"__unset" => run(args(1))
            .or_else(|| run(non_static()))
            .or_else(|| run(public_()))
            .or_else(|| run(arg_type(0, STRING, "string")))
            .or_else(|| run(ret_type(VOID, "void"))),
        b"__isset" => run(args(1))
            .or_else(|| run(non_static()))
            .or_else(|| run(public_()))
            .or_else(|| run(arg_type(0, STRING, "string")))
            .or_else(|| run(ret_type(BOOL, "bool"))),
        b"__call" => run(args(2))
            .or_else(|| run(non_static()))
            .or_else(|| run(public_()))
            .or_else(|| run(arg_type(0, STRING, "string")))
            .or_else(|| run(arg_type(1, ARRAY, "array"))),
        b"__callstatic" => run(args(2))
            .or_else(|| run(must_static()))
            .or_else(|| run(public_()))
            .or_else(|| run(arg_type(0, STRING, "string")))
            .or_else(|| run(arg_type(1, ARRAY, "array"))),
        b"__tostring" => run(args(0))
            .or_else(|| run(non_static()))
            .or_else(|| run(public_()))
            .or_else(|| run(ret_type(STRING, "string"))),
        b"__debuginfo" => run(args(0))
            .or_else(|| run(non_static()))
            .or_else(|| run(public_()))
            .or_else(|| run(ret_type(ARRAY | NULL, "?array"))),
        b"__serialize" => run(args(0))
            .or_else(|| run(non_static()))
            .or_else(|| run(public_()))
            .or_else(|| run(ret_type(ARRAY, "array"))),
        b"__unserialize" => run(args(1))
            .or_else(|| run(non_static()))
            .or_else(|| run(public_()))
            .or_else(|| run(arg_type(0, ARRAY, "array")))
            .or_else(|| run(ret_type(VOID, "void"))),
        b"__set_state" => run(args(1))
            .or_else(|| run(must_static()))
            .or_else(|| run(public_()))
            .or_else(|| run(arg_type(0, ARRAY, "array")))
            .or_else(|| run(ret_type(OBJECT, "object"))),
        b"__invoke" => run(non_static()).or_else(|| run(public_())),
        b"__sleep" => run(args(0))
            .or_else(|| run(non_static()))
            .or_else(|| run(public_()))
            .or_else(|| run(ret_type(ARRAY, "array"))),
        b"__wakeup" => run(args(0))
            .or_else(|| run(non_static()))
            .or_else(|| run(public_()))
            .or_else(|| run(ret_type(VOID, "void"))),
        _ => None,
    };
    warning.into_iter().chain(err).collect()
}

/// zend_type_to_string for a type declared in the file being compiled
/// (class names as resolved, `self`/`parent`/`static` as written).
pub(crate) fn type_string(ty: &TypeHint) -> String {
    let has = |k: &TypeKind| ty.kinds.contains(k);
    if has(&TypeKind::Mixed) {
        return "mixed".into();
    }
    let mut parts: Vec<String> = ty
        .groups
        .iter()
        .map(|g| {
            let names: Vec<String> = g.iter().map(|n| s(n.as_bytes())).collect();
            format!("({})", names.join("&"))
        })
        .collect();
    for k in &ty.kinds {
        match k {
            TypeKind::Named(n) => parts.push(s(n.as_bytes())),
            TypeKind::SelfType => parts.push("self".into()),
            TypeKind::Parent => parts.push("parent".into()),
            TypeKind::Iterable => parts.push("Traversable".into()),
            _ => {}
        }
    }
    if has(&TypeKind::StaticType) {
        parts.push("static".into());
    }
    let array = has(&TypeKind::Array) || has(&TypeKind::Iterable);
    let bool_ = has(&TypeKind::Bool) || (has(&TypeKind::True) && has(&TypeKind::False));
    for (on, name) in [
        (has(&TypeKind::Callable), "callable"),
        (has(&TypeKind::Object), "object"),
        (array, "array"),
        (has(&TypeKind::String), "string"),
        (has(&TypeKind::Int), "int"),
        (has(&TypeKind::Float), "float"),
        (bool_, "bool"),
        (!bool_ && has(&TypeKind::False), "false"),
        (!bool_ && has(&TypeKind::True), "true"),
        (has(&TypeKind::Void), "void"),
        (has(&TypeKind::Never), "never"),
    ] {
        if on {
            parts.push(name.into());
        }
    }
    let sep = if ty.intersection { "&" } else { "|" };
    if ty.nullable || has(&TypeKind::Null) {
        if parts.is_empty() {
            return "null".into();
        }
        if parts.len() == 1 && ty.groups.is_empty() {
            return format!("?{}", parts[0]);
        }
        return format!("{}|null", parts.join(sep));
    }
    parts.join(sep)
}

/// zend_compile_use: a class import may not be named like a reserved class
/// name. Walks the file's `use` statements (namespace bodies included).
pub(crate) fn check_uses(cx: &FileCtx, stmts: &Sequence<Statement>) {
    use crate::parse::cst::UseItems;
    for st in stmts.iter() {
        match st {
            Statement::Namespace(ns) => check_uses(cx, ns.statements()),
            Statement::Use(u) => {
                let mut items: Vec<(Vec<u8>, &crate::parse::cst::UseItem)> = Vec::new();
                match &u.items {
                    UseItems::Sequence(seq) => {
                        items.extend(seq.items.iter().map(|it| (Vec::new(), it)))
                    }
                    UseItems::TypedSequence(_) | UseItems::TypedList(_) => {}
                    UseItems::MixedList(ml) => {
                        let pfx = identifier_raw(&ml.namespace).to_vec();
                        for mit in ml.items.iter() {
                            if mit.r#type.is_none() {
                                items.push((pfx.clone(), &mit.item));
                            }
                        }
                    }
                }
                for (pfx, it) in items {
                    let raw = identifier_raw(&it.name);
                    let mut old = pfx;
                    if !old.is_empty() {
                        old.push(b'\\');
                    }
                    old.extend_from_slice(raw);
                    let old = old.strip_prefix(b"\\").unwrap_or(&old).to_vec();
                    let new: Vec<u8> = match &it.alias {
                        Some(a) => a.identifier.value.to_vec(),
                        None => old.rsplit(|&b| b == b'\\').next().unwrap_or(&old).to_vec(),
                    };
                    if is_reserved_class_name(&new) {
                        cx.fatal(
                            it.span().start.offset,
                            format!(
                                "Cannot use {} as {} because '{}' is a special class name",
                                s(&old),
                                s(&new),
                                s(&new)
                            ),
                        );
                    }
                }
            }
            _ => {}
        }
    }
}
