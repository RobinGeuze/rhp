//! Type declarations: lowering hints to `TypeHint` and php's checks on type names.

use std::rc::Rc;

use crate::parse::cst::{Hint, Identifier};
use rhp_values::{PhpStr, Thrown, Value};

use crate::file::{ClassScope, FileCtx};
use crate::names::{identifier_raw, starts_with_ci, strip_lead_bs};

// php's MAY_BE_* bits of a declared type (zend_type_info.h), as far as
// type declarations use them.
pub(crate) const MAY_NULL: u32 = 1 << 1;
pub(crate) const MAY_FALSE: u32 = 1 << 2;
pub(crate) const MAY_TRUE: u32 = 1 << 3;
pub(crate) const MAY_LONG: u32 = 1 << 4;
pub(crate) const MAY_DOUBLE: u32 = 1 << 5;
pub(crate) const MAY_STRING: u32 = 1 << 6;
pub(crate) const MAY_ARRAY: u32 = 1 << 7;
pub(crate) const MAY_OBJECT: u32 = 1 << 8;
const MAY_RESOURCE: u32 = 1 << 9;
pub(crate) const MAY_CALLABLE: u32 = 1 << 12;
pub(crate) const MAY_VOID: u32 = 1 << 14;
pub(crate) const MAY_STATIC: u32 = 1 << 15;
pub(crate) const MAY_NEVER: u32 = 1 << 17;
const MAY_BOOL: u32 = MAY_FALSE | MAY_TRUE;
const MAY_ANY: u32 = MAY_NULL
    | MAY_BOOL
    | MAY_LONG
    | MAY_DOUBLE
    | MAY_STRING
    | MAY_ARRAY
    | MAY_OBJECT
    | MAY_RESOURCE;

/// One class member of a declared type: the name php compares and prints
/// (self/parent already substituted where php does it at compile time) and
/// the kind it lowers to.
#[derive(Clone)]
struct ClassMember {
    name: Vec<u8>,
    kind: rhp_bytecode::TypeKind,
}

#[derive(Clone)]
enum Member {
    Class(ClassMember),
    Intersection(Vec<ClassMember>),
}

/// A declared type as php's compiler builds it (`zend_type`): builtin type
/// bits plus the class members in declaration order.
#[derive(Clone)]
pub(crate) struct PhpType {
    pub(crate) mask: u32,
    members: Vec<Member>,
    /// A pure intersection `A&B` (`members` are its classes).
    intersection: bool,
    /// `iterable` (Traversable|array) is part of the type.
    iterable: bool,
    /// `null` written as a union member.
    explicit_null: bool,
}

impl PhpType {
    fn of_mask(mask: u32) -> Self {
        PhpType {
            mask,
            members: Vec::new(),
            intersection: false,
            iterable: false,
            explicit_null: false,
        }
    }

    fn is_complex(&self) -> bool {
        !self.members.is_empty()
    }

    /// zend_type_to_string: class members in order, then the builtin types
    /// in php's fixed order; a lone type with null prints as `?T`.
    pub(crate) fn to_php_string(&self) -> String {
        fn add(s: &mut Option<String>, t: &str, sep: char) {
            match s {
                None => *s = Some(t.to_string()),
                Some(s) => {
                    s.push(sep);
                    s.push_str(t);
                }
            }
        }
        let lossy = |n: &[u8]| String::from_utf8_lossy(n).into_owned();
        let inter = |g: &[ClassMember]| {
            g.iter()
                .map(|c| lossy(&c.name))
                .collect::<Vec<_>>()
                .join("&")
        };
        let mut s: Option<String> = None;
        if self.intersection {
            let all: Vec<ClassMember> = self
                .members
                .iter()
                .filter_map(|m| match m {
                    Member::Class(c) => Some(c.clone()),
                    Member::Intersection(_) => None,
                })
                .collect();
            s = Some(inter(&all));
        } else {
            for m in &self.members {
                match m {
                    Member::Class(c) => add(&mut s, &lossy(&c.name), '|'),
                    Member::Intersection(g) => add(&mut s, &format!("({})", inter(g)), '|'),
                }
            }
        }
        let mask = self.mask;
        if mask == MAY_ANY || mask == MAY_ANY | MAY_NULL {
            add(&mut s, "mixed", '|');
            return s.unwrap_or_default();
        }
        for (bit, name) in [
            (MAY_STATIC, "static"),
            (MAY_CALLABLE, "callable"),
            (MAY_OBJECT, "object"),
            (MAY_ARRAY, "array"),
            (MAY_STRING, "string"),
            (MAY_LONG, "int"),
            (MAY_DOUBLE, "float"),
        ] {
            if mask & bit != 0 {
                add(&mut s, name, '|');
            }
        }
        if mask & MAY_BOOL == MAY_BOOL {
            add(&mut s, "bool", '|');
        } else if mask & MAY_FALSE != 0 {
            add(&mut s, "false", '|');
        } else if mask & MAY_TRUE != 0 {
            add(&mut s, "true", '|');
        }
        if mask & MAY_VOID != 0 {
            add(&mut s, "void", '|');
        }
        if mask & MAY_NEVER != 0 {
            add(&mut s, "never", '|');
        }
        if mask & MAY_NULL != 0 {
            match &s {
                Some(t) if !t.contains('|') && !t.contains('&') => return format!("?{t}"),
                _ => add(&mut s, "null", '|'),
            }
        }
        s.unwrap_or_default()
    }

    /// zend_is_valid_default_value: does a constant default fit the type's
    /// builtin bits? An int for a float type becomes a float.
    pub(crate) fn accepts_default(&self, v: &mut Value) -> bool {
        let bit = match v {
            Value::Null => MAY_NULL,
            Value::Bool(false) => MAY_FALSE,
            Value::Bool(true) => MAY_TRUE,
            Value::Int(_) => MAY_LONG,
            Value::Float(_) => MAY_DOUBLE,
            Value::Str(_) => MAY_STRING,
            Value::Array(_) => MAY_ARRAY,
            _ => MAY_OBJECT,
        };
        if self.mask & bit != 0 {
            return true;
        }
        if let Value::Int(n) = v {
            if self.mask & MAY_DOUBLE != 0 {
                *v = Value::Float(*n as f64);
                return true;
            }
        }
        false
    }

    pub(crate) fn is_mixed(&self) -> bool {
        self.mask & MAY_ANY == MAY_ANY
    }

    pub(crate) fn is_intersection(&self) -> bool {
        self.intersection
    }

    /// The type with null added (for messages suggesting `?T`).
    pub(crate) fn with_null(&self) -> PhpType {
        let mut t = self.clone();
        t.mask |= MAY_NULL;
        t
    }

    pub(crate) fn has(&self, bits: u32) -> bool {
        self.mask & bits != 0
    }

    /// Lower to the VM's [`rhp_bytecode::TypeHint`].
    pub(crate) fn lower(&self) -> rhp_bytecode::TypeHint {
        use rhp_bytecode::TypeKind as K;
        let mut out = rhp_bytecode::TypeHint {
            nullable: false,
            intersection: self.intersection,
            kinds: Vec::new(),
            groups: Vec::new(),
            group_pos: Vec::new(),
            explicit_null: self.explicit_null,
        };
        let mut classes = 0u16;
        for m in &self.members {
            match m {
                Member::Class(c) => {
                    out.kinds.push(c.kind.clone());
                    classes += 1;
                }
                Member::Intersection(g) => {
                    out.groups.push(
                        g.iter()
                            .map(|c| PhpStr::from_bytes(c.name.clone()))
                            .collect(),
                    );
                    out.group_pos.push(classes);
                }
            }
        }
        let mask = self.mask;
        if mask & MAY_ANY == MAY_ANY {
            out.kinds.push(K::Mixed);
            return out;
        }
        out.nullable = mask & MAY_NULL != 0;
        for (bit, k) in [
            (MAY_STATIC, K::StaticType),
            (MAY_CALLABLE, K::Callable),
            (MAY_OBJECT, K::Object),
            (MAY_STRING, K::String),
            (MAY_LONG, K::Int),
            (MAY_DOUBLE, K::Float),
            (MAY_VOID, K::Void),
            (MAY_NEVER, K::Never),
        ] {
            if mask & bit != 0 {
                out.kinds.push(k);
            }
        }
        // `iterable`'s array part is its Iterable kind.
        if mask & MAY_ARRAY != 0 && !self.iterable {
            out.kinds.push(K::Array);
        }
        if mask & MAY_BOOL == MAY_BOOL {
            out.kinds.push(K::Bool);
        } else if mask & MAY_FALSE != 0 {
            out.kinds.push(K::False);
        } else if mask & MAY_TRUE != 0 {
            out.kinds.push(K::True);
        }
        if out.kinds.is_empty() && out.groups.is_empty() && out.nullable {
            out.kinds.push(K::Null);
        }
        out
    }
}

/// Where a type declaration sits, for php's self/parent/static rules.
struct Scope {
    class: Option<Rc<ClassScope>>,
    /// zend_is_scope_known: not in a closure or a trait.
    known: bool,
}

/// A compiled type declaration: what the VM checks, and php's view of it.
pub(crate) struct CompiledType {
    pub(crate) hint: rhp_bytecode::TypeHint,
    pub(crate) ty: PhpType,
}

/// zend_compile_typename: compile a type declaration, raising php's compile
/// errors. `closure`: the declaration belongs to a closure (its scope is not
/// known at compile time). `None` once a compile error was recorded.
pub(crate) fn compile_type(cx: &FileCtx, h: &Hint, closure: bool) -> Option<CompiledType> {
    compile_type_ex(cx, h, closure, false).map(|(t, _)| t)
}

/// [`compile_type`] with php's `force_allow_null` (a parameter defaulting
/// to null); the flag is whether that made the type nullable.
pub(crate) fn compile_type_ex(
    cx: &FileCtx,
    h: &Hint,
    closure: bool,
    force_null: bool,
) -> Option<(CompiledType, bool)> {
    let class = cx.class_scope.borrow().clone();
    let known = !closure && class.as_ref().is_none_or(|c| !c.is_trait);
    let sc = Scope { class, known };
    let (ty, forced) = typename(cx, &sc, h, force_null).ok()?;
    Some((
        CompiledType {
            hint: ty.lower(),
            ty,
        },
        forced,
    ))
}

type R<T> = Result<T, ()>;

fn fail<T>(cx: &FileCtx, h: &Hint, msg: String) -> R<T> {
    use mago_span::HasSpan;
    cx.fatal(h.span().start.offset, msg);
    Err(())
}

fn flatten_union<'a>(h: &'a Hint<'a>, out: &mut Vec<&'a Hint<'a>>) {
    match h {
        Hint::Union(u) => {
            flatten_union(u.left, out);
            flatten_union(u.right, out);
        }
        other => out.push(other),
    }
}

fn flatten_intersection<'a>(h: &'a Hint<'a>, out: &mut Vec<&'a Hint<'a>>) {
    match h {
        Hint::Intersection(i) => {
            flatten_intersection(i.left, out);
            flatten_intersection(i.right, out);
        }
        Hint::Parenthesized(p) => flatten_intersection(p.hint, out),
        other => out.push(other),
    }
}

fn is_intersection(h: &Hint) -> bool {
    match h {
        Hint::Intersection(_) => true,
        Hint::Parenthesized(p) => is_intersection(p.hint),
        _ => false,
    }
}

fn typename(cx: &FileCtx, sc: &Scope, h: &Hint, force_null: bool) -> R<(PhpType, bool)> {
    let (marked, inner) = match h {
        Hint::Nullable(n) => (true, n.hint),
        other => (false, other),
    };
    let inner = match inner {
        Hint::Parenthesized(p) if !is_intersection(p.hint) => p.hint,
        other => other,
    };
    let mut ty = if matches!(inner, Hint::Union(_)) {
        union_type(cx, sc, inner)?
    } else if is_intersection(inner) {
        let classes = intersection_type(cx, sc, inner)?;
        let members: Vec<Member> = if force_null {
            // An implicitly nullable intersection becomes the DNF (A&B)|null.
            vec![Member::Intersection(classes)]
        } else {
            classes.into_iter().map(Member::Class).collect()
        };
        PhpType {
            intersection: !force_null,
            members,
            ..PhpType::of_mask(0)
        }
    } else {
        let s = single_type(cx, sc, inner)?;
        let mut t = PhpType::of_mask(s.mask);
        t.iterable = s.iterable;
        if let Some(c) = s.class {
            t.members.push(Member::Class(c));
        }
        t
    };
    let mask = ty.mask;
    if mask == MAY_ANY && marked {
        return fail(
            cx,
            h,
            "Type mixed cannot be marked as nullable since mixed already includes null".into(),
        );
    }
    if mask & MAY_NULL != 0 && marked {
        return fail(cx, h, "null cannot be marked as nullable".into());
    }
    let forced = force_null && !marked && mask & MAY_NULL == 0;
    if marked || force_null {
        ty.mask |= MAY_NULL;
    }
    let mask = ty.mask;
    if mask & MAY_VOID != 0 && (ty.is_complex() || mask != MAY_VOID) {
        return fail(cx, h, "Void can only be used as a standalone type".into());
    }
    if mask & MAY_NEVER != 0 && (ty.is_complex() || mask != MAY_NEVER) {
        return fail(cx, h, "never can only be used as a standalone type".into());
    }
    Ok((ty, forced))
}

fn class_str(c: &ClassMember) -> String {
    String::from_utf8_lossy(&c.name).into_owned()
}

fn inter_str(g: &[ClassMember]) -> String {
    g.iter().map(class_str).collect::<Vec<_>>().join("&")
}

fn union_type(cx: &FileCtx, sc: &Scope, h: &Hint) -> R<PhpType> {
    let mut parts = Vec::new();
    flatten_union(h, &mut parts);
    let mut ty = PhpType::of_mask(0);
    let mut only_iterable_class = true;
    for part in parts {
        if is_intersection(part) {
            only_iterable_class = false;
            let g = intersection_type(cx, sc, part)?;
            // zend_are_intersection_types_redundant /
            // zend_is_intersection_type_redundant_by_single_type
            for m in &ty.members {
                match m {
                    Member::Intersection(prev) => {
                        let (small, large, flipped) = if prev.len() < g.len() {
                            (prev, &g, true)
                        } else {
                            (&g, prev, false)
                        };
                        let shared = small
                            .iter()
                            .filter(|a| large.iter().any(|b| a.name.eq_ignore_ascii_case(&b.name)))
                            .count();
                        if shared == small.len() {
                            let (small_s, large_s) = if flipped {
                                (inter_str(prev), inter_str(&g))
                            } else {
                                (inter_str(&g), inter_str(prev))
                            };
                            let msg = if small.len() == large.len() {
                                format!("Type {small_s} is redundant with type {large_s}")
                            } else {
                                format!(
                                    "Type {large_s} is redundant as it is more restrictive than type {small_s}"
                                )
                            };
                            return fail(cx, part, msg);
                        }
                    }
                    Member::Class(c) => {
                        if g.iter().any(|x| x.name.eq_ignore_ascii_case(&c.name)) {
                            let msg = format!(
                                "Type {} is redundant as it is more restrictive than type {}",
                                inter_str(&g),
                                class_str(c)
                            );
                            return fail(cx, part, msg);
                        }
                    }
                }
            }
            ty.members.push(Member::Intersection(g));
            continue;
        }
        let s = single_type(cx, sc, part)?;
        if s.mask == MAY_ANY {
            return fail(
                cx,
                part,
                "Type mixed can only be used as a standalone type".into(),
            );
        }
        if s.class.is_some() && !s.iterable {
            only_iterable_class = false;
        }
        let overlap = ty.mask & s.mask;
        if overlap != 0 {
            let msg = format!(
                "Duplicate type {} is redundant",
                PhpType::of_mask(overlap).to_php_string()
            );
            return fail(cx, part, msg);
        }
        if (ty.mask & MAY_TRUE != 0 && s.mask == MAY_FALSE)
            || (ty.mask & MAY_FALSE != 0 && s.mask == MAY_TRUE)
        {
            return fail(
                cx,
                part,
                "Type contains both true and false, bool must be used instead".into(),
            );
        }
        ty.mask |= s.mask;
        if matches!(part, Hint::Null(_)) {
            ty.explicit_null = true;
        }
        ty.iterable |= s.iterable;
        if let Some(c) = s.class {
            // zend_is_type_list_redundant_by_single_type
            for m in &ty.members {
                match m {
                    Member::Intersection(g) => {
                        if g.iter().any(|x| x.name.eq_ignore_ascii_case(&c.name)) {
                            let msg = format!(
                                "Type {} is redundant as it is more restrictive than type {}",
                                inter_str(g),
                                class_str(&c)
                            );
                            return fail(cx, part, msg);
                        }
                    }
                    Member::Class(prev) => {
                        if prev.name.eq_ignore_ascii_case(&c.name) {
                            let msg = format!("Duplicate type {} is redundant", class_str(&c));
                            return fail(cx, part, msg);
                        }
                    }
                }
            }
            ty.members.push(Member::Class(c));
        }
    }
    if ty.mask & MAY_OBJECT != 0
        && ((!only_iterable_class && ty.is_complex()) || ty.mask & MAY_STATIC != 0)
    {
        let msg = format!(
            "Type {} contains both object and a class type, which is redundant",
            ty.to_php_string()
        );
        return fail(cx, h, msg);
    }
    Ok(ty)
}

fn intersection_type(cx: &FileCtx, sc: &Scope, h: &Hint) -> R<Vec<ClassMember>> {
    let mut parts = Vec::new();
    flatten_intersection(h, &mut parts);
    let mut out: Vec<ClassMember> = Vec::new();
    for part in parts {
        let s = single_type(cx, sc, part)?;
        if s.iterable {
            return fail(
                cx,
                part,
                "Type Traversable|array cannot be part of an intersection type".into(),
            );
        }
        let Some(c) = s.class else {
            let msg = format!(
                "Type {} cannot be part of an intersection type",
                PhpType::of_mask(s.mask).to_php_string()
            );
            return fail(cx, part, msg);
        };
        if c.name.eq_ignore_ascii_case(b"self") || c.name.eq_ignore_ascii_case(b"parent") {
            let msg = format!(
                "Type {} cannot be part of an intersection type",
                class_str(&c)
            );
            return fail(cx, part, msg);
        }
        if out.iter().any(|p| p.name.eq_ignore_ascii_case(&c.name)) {
            let msg = format!("Duplicate type {} is redundant", class_str(&c));
            return fail(cx, part, msg);
        }
        out.push(c);
    }
    Ok(out)
}

struct Single {
    mask: u32,
    class: Option<ClassMember>,
    iterable: bool,
}

/// zend_compile_single_typename.
fn single_type(cx: &FileCtx, sc: &Scope, h: &Hint) -> R<Single> {
    use rhp_bytecode::TypeKind as K;
    let builtin = |mask| {
        Ok(Single {
            mask,
            class: None,
            iterable: false,
        })
    };
    match h {
        Hint::Null(_) => builtin(MAY_NULL),
        Hint::True(_) => builtin(MAY_TRUE),
        Hint::False(_) => builtin(MAY_FALSE),
        Hint::Array(_) => builtin(MAY_ARRAY),
        Hint::Callable(_) => builtin(MAY_CALLABLE),
        Hint::Void(_) => builtin(MAY_VOID),
        Hint::Never(_) => builtin(MAY_NEVER),
        Hint::Float(_) => builtin(MAY_DOUBLE),
        Hint::Bool(_) => builtin(MAY_BOOL),
        Hint::Integer(_) => builtin(MAY_LONG),
        Hint::String(_) => builtin(MAY_STRING),
        Hint::Object(_) => builtin(MAY_OBJECT),
        Hint::Mixed(_) => builtin(MAY_ANY),
        Hint::Iterable(_) => Ok(Single {
            mask: MAY_ARRAY,
            class: Some(ClassMember {
                name: b"Traversable".to_vec(),
                kind: K::Iterable,
            }),
            iterable: true,
        }),
        Hint::Static(_) => {
            if sc.class.is_none() && sc.known {
                return fail(
                    cx,
                    h,
                    "Cannot use \"static\" when no class scope is active".into(),
                );
            }
            builtin(MAY_STATIC)
        }
        Hint::Self_(k) | Hint::Parent(k) => {
            let is_self = matches!(h, Hint::Self_(_));
            let word = if is_self { "self" } else { "parent" };
            // zend_ensure_valid_class_fetch_type
            if sc.known {
                match &sc.class {
                    None => {
                        return fail(
                            cx,
                            h,
                            format!("Cannot use \"{word}\" when no class scope is active"),
                        )
                    }
                    Some(c) if !is_self && c.parent.is_none() => {
                        return fail(
                            cx,
                            h,
                            "Cannot use \"parent\" when current class scope has no parent".into(),
                        )
                    }
                    _ => {}
                }
            }
            let substituted = match &sc.class {
                Some(c) if sc.known && !c.is_anon => {
                    if is_self {
                        Some(c.name.clone())
                    } else {
                        c.parent.clone()
                    }
                }
                _ => None,
            };
            Ok(Single {
                mask: 0,
                class: Some(ClassMember {
                    name: substituted.unwrap_or_else(|| k.value.to_vec()),
                    kind: if is_self { K::SelfType } else { K::Parent },
                }),
                iterable: false,
            })
        }
        Hint::Identifier(id) => {
            if check_type_name(cx, id) {
                return Err(());
            }
            let resolved = cx.resolve_class_name(id);
            confusable_warning(cx, id, &resolved);
            Ok(Single {
                mask: 0,
                class: Some(ClassMember {
                    name: resolved.clone(),
                    kind: K::Named(PhpStr::from_bytes(resolved)),
                }),
                iterable: false,
            })
        }
        Hint::Nullable(_) | Hint::Union(_) | Hint::Intersection(_) | Hint::Parenthesized(_) => {
            // The grammar does not nest these here.
            builtin(0)
        }
    }
}

/// zend_compile_single_typename's warning for an unqualified class type
/// that looks like a scalar type name (`integer`, `double`, ...).
fn confusable_warning(cx: &FileCtx, id: &Identifier, resolved: &[u8]) {
    use mago_span::HasSpan;
    let Identifier::Local(l) = id else {
        return;
    };
    let correct = match l.value {
        b"boolean" => Some("bool"),
        b"integer" => Some("int"),
        b"double" => Some("float"),
        b"resource" => None,
        _ => return,
    };
    if cx
        .ns()
        .use_class
        .contains_key(&l.value.to_ascii_lowercase())
    {
        return;
    }
    let name = String::from_utf8_lossy(l.value);
    let class = String::from_utf8_lossy(resolved);
    let extra = if cx.cur_ns().is_empty() {
        ""
    } else {
        " or import the class with \"use\""
    };
    let msg = match correct {
        Some(c) => format!(
            "\"{name}\" will be interpreted as a class name. Did you mean \"{c}\"? Write \"\\{class}\"{extra} to suppress this warning"
        ),
        None => format!(
            "\"{name}\" is not a supported builtin type and will be interpreted as a class name. Write \"\\{class}\"{extra} to suppress this warning"
        ),
    };
    let line = cx.line_of(id.span().start.offset);
    cx.compile_diags
        .borrow_mut()
        .push((rhp_values::E_COMPILE_WARNING, msg, line));
}
/// zend_compile_single_typename's compile errors for a class-name type: a
/// builtin type name written fully qualified (`\int`) or namespace-relative
/// (`namespace\static`) is "Type declaration '%s' must be unqualified" (php
/// 8.5.12 added `static` to the builtin list); `\self` is an invalid class
/// name; and a class name ending in a reserved word (`Foo\int`) is refused.
fn check_type_name(cx: &FileCtx, id: &Identifier) -> bool {
    use mago_span::HasSpan;
    const BUILTIN: [&[u8]; 13] = [
        b"null",
        b"true",
        b"false",
        b"int",
        b"float",
        b"string",
        b"bool",
        b"void",
        b"never",
        b"iterable",
        b"object",
        b"mixed",
        b"static",
    ];
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
    let at = id.span().start.offset;
    let (name, prefix): (Vec<u8>, &str) = match id {
        Identifier::FullyQualified(f) => (strip_lead_bs(f.value), "\\"),
        Identifier::Qualified(q) if starts_with_ci(q.value, b"namespace\\") => {
            (q.value[b"namespace\\".len()..].to_vec(), "namespace\\")
        }
        _ => (identifier_raw(id).to_vec(), ""),
    };
    let lc = name.to_ascii_lowercase();
    if !prefix.is_empty() && BUILTIN.contains(&lc.as_slice()) {
        let msg = format!(
            "Type declaration '{}' must be unqualified",
            String::from_utf8_lossy(&lc)
        );
        cx.fatal(at, msg);
        return true;
    }
    // zend_resolve_class_name: `\self` / `namespace\parent` name no class.
    if !prefix.is_empty() && (lc == b"self" || lc == b"parent") {
        let msg = format!(
            "'{prefix}{}' is an invalid class name",
            String::from_utf8_lossy(&name)
        );
        cx.fatal(at, msg);
        return true;
    }
    // zend_assert_valid_class_name(…, "a type name") on the resolved name,
    // whose unqualified tail must not be a reserved name (`Foo\int`).
    let resolved = cx.resolve_class_name(id);
    let tail = resolved.rsplit(|&b| b == b'\\').next().unwrap_or(&resolved);
    if RESERVED.contains(&tail.to_ascii_lowercase().as_slice()) {
        let msg = format!(
            "Cannot use \"{}\" as a type name as it is reserved",
            String::from_utf8_lossy(&resolved)
        );
        cx.fatal(at, msg);
        return true;
    }
    false
}

/// Extract the class name(s) a `catch` clause's type hint names (`A` or `A|B`).
pub(crate) fn hint_class_names(cx: &FileCtx, hint: &Hint) -> Result<Vec<PhpStr>, Thrown> {
    fn collect(cx: &FileCtx, hint: &Hint, out: &mut Vec<PhpStr>) -> Result<(), Thrown> {
        match hint {
            Hint::Identifier(id) => {
                // Resolve against the current namespace + `use` imports (php
                // treats `catch (Foo)` in namespace N as `N\Foo`), matching how
                // `instanceof`/`new` resolve class names.
                out.push(PhpStr::from_bytes(cx.resolve_class_name(id)));
                Ok(())
            }
            Hint::Union(u) => {
                collect(cx, u.left, out)?;
                collect(cx, u.right, out)
            }
            Hint::Parenthesized(p) => collect(cx, p.hint, out),
            _ => Err(Thrown::unsupported(
                "catch type",
                "only class-name catch types",
            )),
        }
    }
    let mut out = Vec::new();
    collect(cx, hint, &mut out)?;
    Ok(out)
}
