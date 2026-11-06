//! php's internal attributes at compile time (zend_attributes.c and the
//! second loop of zend_compile_attributes): each internal attribute's
//! allowed targets, whether it may repeat, and its validator. User
//! attributes are only checked by `ReflectionAttribute::newInstance()`.

use rhp_bytecode::AttributeProto;
use rhp_values::PhpStr;

use crate::checks::DeclKind;
use crate::file::FileCtx;

pub(crate) const TARGET_CLASS: u32 = 1;
pub(crate) const TARGET_FUNCTION: u32 = 2;
pub(crate) const TARGET_METHOD: u32 = 4;
pub(crate) const TARGET_PROPERTY: u32 = 8;
pub(crate) const TARGET_CLASS_CONST: u32 = 16;
pub(crate) const TARGET_PARAMETER: u32 = 32;
pub(crate) const TARGET_CONST: u32 = 64;
const TARGET_ALL: u32 = 127;
const IS_REPEATABLE: u32 = 128;

/// The class a class-target attribute sits on (its validators look at it).
#[derive(Clone, Copy)]
pub(crate) struct ClassInfo<'a> {
    pub kind: DeclKind,
    pub name: &'a [u8],
    pub is_abstract: bool,
    pub is_readonly: bool,
}

/// Where an attribute list sits.
#[derive(Clone, Copy)]
pub(crate) struct AttrTarget<'a> {
    /// One `TARGET_*` bit.
    pub target: u32,
    /// For a promoted parameter, the other target (php's `promoted`
    /// argument): internal attributes that only fit that one are skipped.
    pub promoted: u32,
    pub class: Option<ClassInfo<'a>>,
    /// A property hook (`#[\NoDiscard]` refuses it).
    pub hook: bool,
    /// Where target and repetition errors are reported.
    pub at: u32,
}

impl AttrTarget<'_> {
    pub(crate) fn new(target: u32, at: u32) -> AttrTarget<'static> {
        AttrTarget {
            target,
            promoted: 0,
            class: None,
            hook: false,
            at,
        }
    }
}

#[derive(Clone, Copy)]
enum Validator {
    None,
    Attribute,
    AllowDynamicProperties,
    Deprecated,
    NoDiscard,
}

/// zend_internal_attribute_get: the flags and validator of an internal
/// attribute, by lowercase name.
fn internal(lc: &[u8]) -> Option<(u32, Validator)> {
    Some(match lc {
        b"attribute" => (TARGET_CLASS, Validator::Attribute),
        b"returntypewillchange" => (TARGET_METHOD, Validator::None),
        b"allowdynamicproperties" => (TARGET_CLASS, Validator::AllowDynamicProperties),
        b"sensitiveparameter" => (TARGET_PARAMETER, Validator::None),
        b"override" => (TARGET_METHOD | TARGET_PROPERTY, Validator::None),
        b"deprecated" => (
            TARGET_METHOD | TARGET_FUNCTION | TARGET_CLASS_CONST | TARGET_CONST | TARGET_CLASS,
            Validator::Deprecated,
        ),
        b"nodiscard" => (TARGET_METHOD | TARGET_FUNCTION, Validator::NoDiscard),
        b"delayedtargetvalidation" => (TARGET_ALL, Validator::None),
        _ => return None,
    })
}

/// Whether internal attribute `lc` is dropped from a promoted parameter's
/// list (it fits the other target but not this one).
pub(crate) fn skip_promoted(lc: &[u8], t: &AttrTarget) -> bool {
    match internal(lc) {
        Some((flags, _)) => t.target & flags == 0 && t.promoted & flags != 0,
        None => false,
    }
}

/// zend_get_attribute_target_names.
pub(crate) fn target_names(flags: u32) -> String {
    const NAMES: [&str; 7] = [
        "class",
        "function",
        "method",
        "property",
        "class constant",
        "parameter",
        "constant",
    ];
    let mut out = Vec::new();
    for (i, n) in NAMES.iter().enumerate() {
        if flags & (1 << i) != 0 {
            out.push(*n);
        }
    }
    out.join(", ")
}

fn lc(a: &AttributeProto) -> Vec<u8> {
    a.name.as_bytes().to_ascii_lowercase()
}

/// The validation loop of zend_compile_attributes: target and repetition
/// checks for internal attributes, then their validators. With
/// `#[\DelayedTargetValidation]` a target error is left to
/// `newInstance()` and a validator error is stored on the attribute.
pub(crate) fn validate(cx: &FileCtx, attrs: &mut [AttributeProto], t: &AttrTarget) {
    let delayed = attrs.iter().any(|a| lc(a) == b"delayedtargetvalidation");
    for i in 0..attrs.len() {
        let name = lc(&attrs[i]);
        let Some((flags, validator)) = internal(&name) else {
            continue;
        };
        let shown = String::from_utf8_lossy(attrs[i].name.as_bytes()).into_owned();
        let mut run_validator = true;
        if t.target & flags == 0 {
            if !delayed {
                cx.fatal(
                    t.at,
                    format!(
                        "Attribute \"{shown}\" cannot target {} (allowed targets: {})",
                        target_names(t.target),
                        target_names(flags)
                    ),
                );
                return;
            }
            run_validator = false;
        }
        if flags & IS_REPEATABLE == 0
            && attrs
                .iter()
                .enumerate()
                .any(|(j, o)| j != i && lc(o) == name)
        {
            cx.fatal(t.at, format!("Attribute \"{shown}\" must not be repeated"));
            return;
        }
        if !run_validator {
            continue;
        }
        if let Some(err) = run(validator, t) {
            if delayed {
                attrs[i].validation_error = Some(PhpStr::from_bytes(err.into_bytes()));
            } else {
                cx.fatal(t.at, err);
                return;
            }
        }
    }
}

fn run(v: Validator, t: &AttrTarget) -> Option<String> {
    let class = |c: &ClassInfo| String::from_utf8_lossy(c.name).into_owned();
    match v {
        Validator::None => None,
        Validator::Attribute => {
            let c = t.class?;
            let what = match c.kind {
                DeclKind::Trait => "trait",
                DeclKind::Interface => "interface",
                DeclKind::Enum => "enum",
                _ if c.is_abstract => "abstract class",
                _ => return None,
            };
            Some(format!(
                "Cannot apply #[\\Attribute] to {what} {}",
                class(&c)
            ))
        }
        Validator::AllowDynamicProperties => {
            let c = t.class?;
            let what = match c.kind {
                DeclKind::Trait => "trait",
                DeclKind::Interface => "interface",
                _ if c.is_readonly => "readonly class",
                DeclKind::Enum => "enum",
                _ => return None,
            };
            Some(format!(
                "Cannot apply #[\\AllowDynamicProperties] to {what} {}",
                class(&c)
            ))
        }
        Validator::Deprecated => {
            if t.target != TARGET_CLASS {
                return None;
            }
            let c = t.class?;
            let what = match c.kind {
                DeclKind::Trait => return None,
                DeclKind::Interface => "interface",
                DeclKind::Enum => "enum",
                _ => "class",
            };
            Some(format!(
                "Cannot apply #[\\Deprecated] to {what} {}",
                class(&c)
            ))
        }
        Validator::NoDiscard => t
            .hook
            .then(|| "#[\\NoDiscard] is not supported for property hooks".to_string()),
    }
}

/// Whether the list has attribute `lc` (any case).
pub(crate) fn has(attrs: &[AttributeProto], lc: &[u8]) -> bool {
    attrs
        .iter()
        .any(|a| a.name.as_bytes().eq_ignore_ascii_case(lc))
}

/// The function flag the `#[\NoDiscard]` validator sets (not for a hook,
/// nor when its error was delayed).
pub(crate) fn nodiscard_flag(attrs: &[AttributeProto]) -> bool {
    attrs.iter().any(|a| {
        a.name.as_bytes().eq_ignore_ascii_case(b"NoDiscard") && a.validation_error.is_none()
    })
}

/// The function flags zend_compile_func_decl derives from its attributes,
/// and the `#[\NoDiscard]` return type check. `in_class`: php's
/// `CG(active_class_entry)` (a method, or a closure inside one); `hook`: a
/// property hook; `at`: the `function` keyword.
pub(crate) fn apply_fn_flags(
    cx: &FileCtx,
    proto: &mut rhp_bytecode::FunctionProto,
    in_class: bool,
    hook: bool,
    at: u32,
) {
    use rhp_bytecode::{ProtoFlags, TypeKind};
    let attrs = &proto.meta.attributes;
    if attrs.is_empty() {
        return;
    }
    if has(attrs, b"deprecated") {
        proto.flags.insert(ProtoFlags::DEPRECATED);
    }
    // The validator refuses a hook (an error, or one delayed).
    if hook || !nodiscard_flag(attrs) {
        return;
    }
    proto.flags.insert(ProtoFlags::NODISCARD);
    let what = if in_class { "method" } else { "function" };
    let Some(rt) = &proto.return_type else {
        return;
    };
    if rt.kinds.contains(&TypeKind::Void) {
        cx.fatal(
            at,
            format!(
                "A void {what} does not return a value, but #[\\NoDiscard] requires a return value"
            ),
        );
    } else if rt.kinds.contains(&TypeKind::Never) {
        cx.fatal(
            at,
            format!("A never returning {what} does not return a value, but #[\\NoDiscard] requires a return value"),
        );
    }
}
