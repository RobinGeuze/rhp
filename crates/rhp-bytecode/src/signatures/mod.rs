//! php's arginfo for the functions, classes and methods of the extensions
//! rhp provides, generated from php-src's `*.stub.php` into [`table`] by
//! `rhp-difftest gen signatures`.
//!
//! This is metadata only. Whether a function or class exists is up to the
//! registry and the prelude, so a stub row never makes rhp report something
//! it does not implement.

mod table;

use std::cmp::Ordering;

/// One parameter: its name, its type as Reflection prints it (empty when
/// untyped), its default as Reflection prints it (empty when none), and
/// flags ([`R`], [`PR`], [`V`], [`U`], and [`ZV`], [`RS`], [`ST`], [`NUL`]
/// for what a function's C body parses it as).
#[derive(Debug)]
pub struct Param {
    pub name: &'static str,
    pub ty: &'static str,
    pub default: &'static str,
    pub flags: u8,
}

/// By reference (`ZEND_SEND_BY_REF`).
pub const R: u8 = 1;
/// `@prefer-ref` (`ZEND_SEND_PREFER_REF`): by reference when the argument
/// is a variable, by value otherwise.
pub const PR: u8 = 2;
/// Variadic.
pub const V: u8 = 4;
/// Optional without a default value (`= UNKNOWN` in the stub).
pub const U: u8 = 8;
/// php's C body parses it as any value (`Z_PARAM_ZVAL`, `z`).
pub const ZV: u8 = 16;
/// php's C body parses it as a resource (`Z_PARAM_RESOURCE`, `r`).
pub const RS: u8 = 32;
/// php's C body parses it as an open stream (`PHP_Z_PARAM_STREAM`).
pub const ST: u8 = 64;
/// With [`RS`] or [`ST`]: or null.
pub const NUL: u8 = 128;

impl Param {
    pub fn by_ref(&self) -> bool {
        self.flags & (R | PR) != 0
    }
    pub fn prefer_ref(&self) -> bool {
        self.flags & PR != 0
    }
    pub fn variadic(&self) -> bool {
        self.flags & V != 0
    }
    /// The default value's source text, when there is one.
    pub fn default(&self) -> Option<&'static str> {
        (!self.default.is_empty()).then_some(self.default)
    }
    /// Does the type accept null?
    pub fn nullable(&self) -> bool {
        type_nullable(self.ty)
    }
}

/// Does a Reflection type string accept null?
pub fn type_nullable(ty: &str) -> bool {
    ty.is_empty() || ty == "mixed" || ty.starts_with('?') || ty.split('|').any(|t| t == "null")
}

/// Method and function flags: Reflection's modifiers plus two of ours.
pub const PUBLIC: u16 = 1;
pub const PROTECTED: u16 = 2;
pub const PRIVATE: u16 = 4;
pub const STATIC: u16 = 16;
pub const FINAL: u16 = 32;
pub const ABSTRACT: u16 = 64;
/// `@tentative-return-type`.
pub const TENTATIVE: u16 = 1 << 8;
/// `#[\Deprecated]`.
pub const DEPRECATED: u16 = 1 << 9;
/// `@frameless-function` of arity n (0 to 3): `FRAMELESS << n`.
pub const FRAMELESS: u16 = 1 << 10;

#[derive(Debug)]
pub struct Func {
    pub name: &'static str,
    pub ext: &'static str,
    pub flags: u16,
    /// The return type as Reflection prints it (empty when none).
    pub ret: &'static str,
    pub params: &'static [Param],
}

#[derive(Debug)]
pub struct Method {
    pub class: &'static str,
    pub name: &'static str,
    pub flags: u16,
    pub ret: &'static str,
    pub params: &'static [Param],
}

/// Class kinds.
pub const CLASS: u8 = 0;
pub const INTERFACE: u8 = 1;
pub const TRAIT: u8 = 2;
pub const ENUM: u8 = 3;

/// Class flags: ReflectionClass's modifiers, then the `ce_flags` gen_stub
/// derives from `@strict-properties`, `#[\AllowDynamicProperties]` and
/// `@not-serializable` (php's ZEND_ACC_* values).
pub const CLASS_FINAL: u32 = 32;
pub const CLASS_ABSTRACT: u32 = 64;
pub const CLASS_READONLY: u32 = 65536;
pub const CLASS_NO_DYNAMIC_PROPERTIES: u32 = 1 << 13;
pub const CLASS_ALLOW_DYNAMIC_PROPERTIES: u32 = 1 << 15;
pub const CLASS_NOT_SERIALIZABLE: u32 = 1 << 29;
/// The bits ReflectionClass::getModifiers() reports.
pub const CLASS_MODIFIERS: u32 = CLASS_FINAL | CLASS_ABSTRACT | CLASS_READONLY;

#[derive(Debug)]
pub struct Class {
    pub name: &'static str,
    pub ext: &'static str,
    pub kind: u8,
    pub flags: u32,
    /// The declared parent class (empty when none).
    pub parent: &'static str,
    /// The declared interfaces (`implements`, or an interface's `extends`).
    pub interfaces: &'static [&'static str],
}

pub(crate) const fn p(
    name: &'static str,
    ty: &'static str,
    default: &'static str,
    flags: u8,
) -> Param {
    Param {
        name,
        ty,
        default,
        flags,
    }
}

pub(crate) const fn f(
    name: &'static str,
    ext: &'static str,
    flags: u16,
    ret: &'static str,
    params: &'static [Param],
) -> Func {
    Func {
        name,
        ext,
        flags,
        ret,
        params,
    }
}

pub(crate) const fn m(
    class: &'static str,
    name: &'static str,
    flags: u16,
    ret: &'static str,
    params: &'static [Param],
) -> Method {
    Method {
        class,
        name,
        flags,
        ret,
        params,
    }
}

pub(crate) const fn c(
    name: &'static str,
    ext: &'static str,
    kind: u8,
    flags: u32,
    parent: &'static str,
    interfaces: &'static [&'static str],
) -> Class {
    Class {
        name,
        ext,
        kind,
        flags,
        parent,
        interfaces,
    }
}

/// Arity and by-ref queries shared by functions and methods.
pub trait Signature {
    fn params(&self) -> &'static [Param];
    fn flags(&self) -> u16;
    fn ret(&self) -> &'static str;

    /// Required parameters (zend's `required_num_args`).
    fn min_args(&self) -> usize {
        self.params()
            .iter()
            .rposition(|p| p.default.is_empty() && p.flags & (V | U) == 0)
            .map_or(0, |i| i + 1)
    }
    /// Declared parameters, or `None` when the last one is variadic.
    fn max_args(&self) -> Option<usize> {
        let ps = self.params();
        match ps.last() {
            Some(p) if p.variadic() => None,
            _ => Some(ps.len()),
        }
    }
    /// The parameter argument `i` binds to (a variadic one takes the rest).
    fn param(&self, i: usize) -> Option<&'static Param> {
        let ps = self.params();
        ps.get(i).or_else(|| ps.last().filter(|p| p.variadic()))
    }
    fn by_ref(&self, i: usize) -> bool {
        self.param(i).is_some_and(Param::by_ref)
    }
    fn prefer_ref(&self, i: usize) -> bool {
        self.param(i).is_some_and(Param::prefer_ref)
    }
    fn has_by_ref(&self) -> bool {
        self.params().iter().any(Param::by_ref)
    }
    fn deprecated(&self) -> bool {
        self.flags() & DEPRECATED != 0
    }
}

impl Signature for Func {
    fn params(&self) -> &'static [Param] {
        self.params
    }
    fn flags(&self) -> u16 {
        self.flags
    }
    fn ret(&self) -> &'static str {
        self.ret
    }
}

impl Func {
    /// Whether php compiles a call with `argc` arguments to a frameless
    /// handler (find_frameless_function_info's arity check).
    pub fn frameless(&self, argc: usize) -> bool {
        argc <= 3
            && (argc..=3).any(|n| {
                self.flags & (FRAMELESS << n) != 0
                    && self.min_args() <= argc
                    && (self.max_args().is_some() || n == argc)
            })
    }
}

impl Signature for Method {
    fn params(&self) -> &'static [Param] {
        self.params
    }
    fn flags(&self) -> u16 {
        self.flags
    }
    fn ret(&self) -> &'static str {
        self.ret
    }
}

impl Method {
    pub fn tentative(&self) -> bool {
        self.flags & TENTATIVE != 0
    }
}

/// ASCII case-insensitive order (the tables are sorted by lowercase name).
fn cmp_ci(a: &[u8], b: &[u8]) -> Ordering {
    a.iter()
        .map(u8::to_ascii_lowercase)
        .cmp(b.iter().map(u8::to_ascii_lowercase))
}

fn strip_ns(name: &[u8]) -> &[u8] {
    name.strip_prefix(b"\\").unwrap_or(name)
}

/// The stub signature of function `name` (any case, leading `\` allowed).
pub fn function(name: &[u8]) -> Option<&'static Func> {
    let name = strip_ns(name);
    let t = table::FUNCTIONS;
    t.binary_search_by(|r| cmp_ci(r.name.as_bytes(), name))
        .ok()
        .map(|i| &t[i])
}

/// The stub declaration of class `name`.
pub fn class(name: &[u8]) -> Option<&'static Class> {
    let name = strip_ns(name);
    let t = table::CLASSES;
    t.binary_search_by(|r| cmp_ci(r.name.as_bytes(), name))
        .ok()
        .map(|i| &t[i])
}

/// The stub signature of method `class::name` as declared on `class`
/// (inherited methods are not repeated).
pub fn method(class: &[u8], name: &[u8]) -> Option<&'static Method> {
    let class = strip_ns(class);
    let t = table::METHODS;
    t.binary_search_by(|r| {
        cmp_ci(r.class.as_bytes(), class).then_with(|| cmp_ci(r.name.as_bytes(), name))
    })
    .ok()
    .map(|i| &t[i])
}

/// A `#[\Deprecated]` function's `since` and `message` (each empty when
/// the attribute leaves it out).
pub fn deprecation(f: &Func) -> Option<(&'static str, &'static str)> {
    if !f.deprecated() {
        return None;
    }
    let t = table::DEPRECATIONS;
    t.binary_search_by(|r| cmp_ci(r.0.as_bytes(), f.name.as_bytes()))
        .ok()
        .map(|i| (t[i].1, t[i].2))
}

/// Every function row.
pub fn functions() -> &'static [Func] {
    table::FUNCTIONS
}

/// Every class row.
pub fn classes() -> &'static [Class] {
    table::CLASSES
}

/// Every method row.
pub fn methods() -> &'static [Method] {
    table::METHODS
}

/// A parameter whose type is made of scalar and array types only, so zpp's
/// checks on it are generic: the `Z_PARAM_*` its type maps to ([`Zpp`]),
/// in the caller's typing mode.
#[derive(Debug, Clone)]
pub struct ScalarParam {
    pub pos: usize,
    pub name: &'static str,
    pub ty: &'static str,
    pub kind: Zpp,
    pub variadic: bool,
    pub nullable: bool,
    pub has_str: bool,
    pub has_arr: bool,
    pub by_ref: bool,
}

/// The `Z_PARAM_*` a scalar stub type parses with (`null` aside, which
/// [`ScalarParam::nullable`] covers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zpp {
    /// `string`: `Z_PARAM_STR`.
    Str,
    /// `int`: `Z_PARAM_LONG`.
    Long,
    /// `float`: `Z_PARAM_DOUBLE`.
    Double,
    /// `bool`: `Z_PARAM_BOOL`.
    Bool,
    /// `int|float`: `Z_PARAM_NUMBER`.
    Number,
    /// `string|int`: `Z_PARAM_STR_OR_LONG`.
    StrOrLong,
    /// `string|int|float`: `Z_PARAM_NUMBER_OR_STR`.
    NumberOrStr,
    /// `array`: `Z_PARAM_ARRAY`.
    Array,
    /// `array|string`: `Z_PARAM_ARRAY_HT_OR_STR`.
    ArrayOrStr,
    /// `array|int`: `Z_PARAM_ARRAY_HT_OR_LONG`.
    ArrayOrLong,
    /// A class name (zpp's `C`, zend_parse_arg_class): converted to a
    /// string in either mode, then a class that must exist and derive from
    /// [`class_param_base`] when it names one.
    Class,
    /// An untyped parameter php parses with `Z_PARAM_RESOURCE` ([`RS`]).
    Resource,
    /// An untyped parameter php parses with `PHP_Z_PARAM_STREAM` ([`ST`]):
    /// a resource that is an open stream.
    Stream,
    /// Another mix of scalar types: only the object, array and null checks.
    Other,
}

impl Zpp {
    /// The type a TypeError names (php's `Z_EXPECTED_*`), or `None` for
    /// [`Zpp::Other`].
    pub fn expected(self, nullable: bool) -> Option<&'static str> {
        use Zpp::*;
        Some(match (self, nullable) {
            (Str, false) => "string",
            (Str, true) => "?string",
            (Long, false) => "int",
            (Long, true) => "?int",
            (Double, false) => "float",
            (Double, true) => "?float",
            (Bool, false) => "bool",
            (Bool, true) => "?bool",
            (Number, false) => "int|float",
            (Number, true) => "int|float|null",
            (StrOrLong, false) => "string|int",
            (StrOrLong, true) => "string|int|null",
            (NumberOrStr, false) => "string|int|float",
            (NumberOrStr, true) => "string|int|float|null",
            (Array, false) => "array",
            (Array, true) => "?array",
            (ArrayOrStr, false) => "array|string",
            (ArrayOrStr, true) => "array|string|null",
            (ArrayOrLong, false) => "array|int",
            (ArrayOrLong, true) => "array|int|null",
            (Resource | Stream, false) => "resource",
            (Resource | Stream, true) => "resource or null",
            (Class | Other, _) => return None,
        })
    }

    fn of(ty: &str) -> Zpp {
        let mut parts: Vec<&str> = ty
            .trim_start_matches('?')
            .split('|')
            .filter(|t| *t != "null")
            .collect();
        parts.sort_unstable();
        match parts.as_slice() {
            ["string"] => Zpp::Str,
            ["int"] => Zpp::Long,
            ["float"] => Zpp::Double,
            ["bool"] => Zpp::Bool,
            ["float", "int"] => Zpp::Number,
            ["int", "string"] => Zpp::StrOrLong,
            ["float", "int", "string"] => Zpp::NumberOrStr,
            ["array"] => Zpp::Array,
            ["array", "string"] => Zpp::ArrayOrStr,
            ["array", "int"] => Zpp::ArrayOrLong,
            _ => Zpp::Other,
        }
    }
}

/// Parameters whose zpp is looser than their stub type: fputcsv's `$eol` is
/// `Z_PARAM_STR_OR_NULL`. (Typed parameters php parses as any value, such
/// as hash_equals' and ini_set's, carry [`ZV`].)
const ZPP_OWN: &[(&str, &str)] = &[("fputcsv", "eol")];

/// Parameters whose zpp differs from what their stub type maps to:
/// number_format's `float $num` is `Z_PARAM_NUMBER`, and the `string`
/// parameters php parses as a class name (`C`).
const ZPP_KIND: &[(&str, &str, Zpp)] = &[
    ("get_class_vars", "class", Zpp::Class),
    ("number_format", "num", Zpp::Number),
    ("simplexml_import_dom", "class_name", Zpp::Class),
    ("simplexml_load_file", "class_name", Zpp::Class),
    ("simplexml_load_string", "class_name", Zpp::Class),
];

/// The class a [`Zpp::Class`] parameter's class must derive from, if any.
pub fn class_param_base(func: &str, param: &str) -> Option<&'static str> {
    match (func, param) {
        (
            "simplexml_import_dom" | "simplexml_load_file" | "simplexml_load_string",
            "class_name",
        ) => Some("SimpleXMLElement"),
        _ => None,
    }
}

/// Extensions whose builtins parse every parameter themselves, in php's
/// order: a `GMP|int|string` argument's error comes before a later `int`'s.
const ZPP_OWN_EXT: &[&str] = &["gmp"];

fn scalar_param(f: &Func, pos: usize, p: &'static Param) -> Option<ScalarParam> {
    if p.flags & ZV != 0 || ZPP_OWN_EXT.contains(&f.ext) {
        return None;
    }
    if p.flags & (RS | ST) != 0 {
        return Some(ScalarParam {
            pos,
            name: p.name,
            ty: p.ty,
            kind: if p.flags & ST != 0 {
                Zpp::Stream
            } else {
                Zpp::Resource
            },
            variadic: p.variadic(),
            nullable: p.flags & NUL != 0,
            has_str: false,
            has_arr: false,
            by_ref: false,
        });
    }
    if p.ty.is_empty() || ZPP_OWN.contains(&(f.name, p.name)) {
        return None;
    }
    let parts = || p.ty.trim_start_matches('?').split('|');
    let scalar = parts().all(|t| {
        matches!(
            t,
            "int" | "float" | "string" | "bool" | "false" | "true" | "array" | "null"
        )
    });
    let nullable = p.nullable();
    // A by-ref `?array` (stream_select) may hold null.
    if !scalar || (p.by_ref() && nullable) {
        return None;
    }
    Some(ScalarParam {
        pos,
        name: p.name,
        ty: p.ty,
        kind: ZPP_KIND
            .iter()
            .find(|(func, param, _)| *func == f.name && *param == p.name)
            .map_or_else(|| Zpp::of(p.ty), |&(_, _, kind)| kind),
        variadic: p.variadic(),
        nullable,
        has_str: parts().any(|t| t == "string"),
        has_arr: parts().any(|t| t == "array"),
        by_ref: p.by_ref(),
    })
}

/// A function row's [`ScalarParam`]s, and how many of its parameters come
/// before the first one whose checks are the body's (`prefix`: a callable,
/// a class type, a resource php does not parse as one, …; `mixed` and
/// parameters php parses as any value do not count): zpp reports an
/// earlier parameter's error first, so the checks of a later scalar
/// parameter that the body's error would beat stay with the body.
#[derive(Debug)]
pub struct ScalarParams {
    pub params: Box<[ScalarParam]>,
    pub prefix: usize,
    /// zpp depends on the call ([`scalar_params_for`]).
    by_call: bool,
}

/// The functions [`scalar_params_for`] has variants for (`call_variant`
/// tells them apart by their first letter).
const ZPP_BY_CALL: &[&str] = &["implode", "strtr"];

/// The [`ScalarParams`] of a function row.
pub fn scalar_params(f: &'static Func) -> Option<&'static ScalarParams> {
    static CACHE: std::sync::OnceLock<Vec<ScalarParams>> = std::sync::OnceLock::new();
    let rows = table::FUNCTIONS;
    let cache = CACHE.get_or_init(|| {
        rows.iter()
            .map(|f| {
                let params: Box<[ScalarParam]> = f
                    .params
                    .iter()
                    .enumerate()
                    .filter_map(|(i, p)| scalar_param(f, i, p))
                    .collect();
                // `mixed`, untyped by-reference parameters and those php
                // parses as any value accept anything.
                let prefix = (0..f.params.len())
                    .find(|&i| {
                        let p = &f.params[i];
                        !params.iter().any(|sp| sp.pos == i)
                            && p.ty != "mixed"
                            && p.flags & ZV == 0
                            && !(p.ty.is_empty() && p.by_ref())
                    })
                    .unwrap_or(usize::MAX);
                // Past the prefix a class name or a resource is the body's:
                // it checks them after the earlier checks.
                let params = params
                    .into_vec()
                    .into_iter()
                    .filter(|sp| {
                        !matches!(sp.kind, Zpp::Class | Zpp::Resource | Zpp::Stream)
                            || sp.pos < prefix
                    })
                    .collect();
                ScalarParams {
                    params,
                    prefix,
                    by_call: ZPP_BY_CALL.contains(&f.name),
                }
            })
            .collect()
    });
    let at = (f as *const Func as usize).wrapping_sub(rows.as_ptr() as usize)
        / std::mem::size_of::<Func>();
    cache.get(at)
}

/// The [`ScalarParams`] for a call with `argc` arguments (`frameless`:
/// one php compiles to ZEND_FRAMELESS_ICALL_n): the row's, except where
/// php's zpp depends on the count or the variant. strtr parses `$from` as
/// Z_PARAM_ARRAY_HT with two arguments, and `$from` and `$to` as
/// Z_PARAM_STR with three. implode's frameless forms parse nothing with
/// one argument (the body raises the TypeError) and `$separator` as
/// Z_FLF_PARAM_STR with two.
#[inline]
pub fn scalar_params_for(
    f: &'static Func,
    argc: usize,
    frameless: bool,
) -> Option<&'static ScalarParams> {
    let row = scalar_params(f)?;
    if !row.by_call {
        return Some(row);
    }
    Some(call_variant(f, argc, frameless).unwrap_or(row))
}

#[inline]
fn call_variant(f: &Func, argc: usize, frameless: bool) -> Option<&'static ScalarParams> {
    static VARIANTS: std::sync::OnceLock<[ScalarParams; 4]> = std::sync::OnceLock::new();
    let at = match (f.name.as_bytes()[0], argc, frameless) {
        (b's', 2, _) => 0,
        (b's', 3, _) => 1,
        (b'i', 1, true) => 2,
        (b'i', 2, true) => 3,
        _ => return None,
    };
    let variants = VARIANTS.get_or_init(|| {
        // The row's parameters with each named one's kind replaced; the
        // type php's null deprecation prints stays the stub's.
        let with = |name: &str, kinds: &[(&str, Zpp)]| {
            let row = function(name.as_bytes()).and_then(scalar_params);
            let params = row
                .expect("cached stub row")
                .params
                .iter()
                .map(|sp| {
                    let mut sp = sp.clone();
                    if let Some(&(_, kind)) = kinds.iter().find(|(p, _)| *p == sp.name) {
                        sp.kind = kind;
                        sp.nullable = false;
                        sp.has_str = kind == Zpp::Str;
                        sp.has_arr = kind == Zpp::Array;
                    }
                    sp
                })
                .collect();
            ScalarParams {
                params,
                prefix: usize::MAX,
                by_call: true,
            }
        };
        [
            with("strtr", &[("from", Zpp::Array)]),
            with("strtr", &[("from", Zpp::Str), ("to", Zpp::Str)]),
            ScalarParams {
                params: Box::new([]),
                prefix: usize::MAX,
                by_call: true,
            },
            with("implode", &[("separator", Zpp::Str)]),
        ]
    });
    Some(&variants[at])
}

#[cfg(test)]
mod tests;
