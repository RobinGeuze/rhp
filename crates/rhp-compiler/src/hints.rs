//! Type declarations: lowering hints to `TypeHint` and php's checks on type names.

use crate::parse::cst::{Hint, Identifier};
use rhp_values::{PhpStr, Thrown};

use crate::file::FileCtx;
use crate::names::{identifier_raw, starts_with_ci, strip_lead_bs};

/// Lower a mago type hint to the bytecode [`rhp_bytecode::TypeHint`] (resolving
/// class names against the current namespace).
pub(crate) fn type_hint(cx: &FileCtx, h: &Hint) -> rhp_bytecode::TypeHint {
    let mut out = rhp_bytecode::TypeHint {
        nullable: false,
        intersection: false,
        kinds: Vec::new(),
        groups: Vec::new(),
        explicit_null: false,
    };
    collect_hint(cx, h, &mut out);
    out
}

/// The class names of a (parenthesized) intersection hint `A&B&C`.
fn intersection_names(cx: &FileCtx, h: &Hint, out: &mut Vec<PhpStr>) {
    match h {
        Hint::Parenthesized(p) => intersection_names(cx, p.hint, out),
        Hint::Intersection(i) => {
            intersection_names(cx, i.left, out);
            intersection_names(cx, i.right, out);
        }
        Hint::Identifier(id) => out.push(PhpStr::from_bytes(cx.resolve_class_name(id))),
        _ => {}
    }
}

fn collect_hint(cx: &FileCtx, h: &Hint, out: &mut rhp_bytecode::TypeHint) {
    use rhp_bytecode::TypeKind as K;
    match h {
        Hint::Nullable(n) => {
            out.nullable = true;
            collect_hint(cx, n.hint, out);
        }
        Hint::Parenthesized(p) => collect_hint(cx, p.hint, out),
        Hint::Union(u) => {
            // A DNF member `(A&B)` of a union is an intersection group.
            for side in [u.left, u.right] {
                match side {
                    Hint::Parenthesized(p) if matches!(p.hint, Hint::Intersection(_)) => {
                        let mut g = Vec::new();
                        intersection_names(cx, p.hint, &mut g);
                        out.groups.push(g);
                    }
                    Hint::Null(_) => {
                        out.nullable = true;
                        out.explicit_null = true;
                    }
                    other => collect_hint(cx, other, out),
                }
            }
        }
        Hint::Intersection(i) => {
            out.intersection = true;
            collect_hint(cx, i.left, out);
            collect_hint(cx, i.right, out);
        }
        Hint::Null(_) => out.nullable = true,
        Hint::True(_) => out.kinds.push(K::True),
        Hint::False(_) => out.kinds.push(K::False),
        Hint::Array(_) => out.kinds.push(K::Array),
        Hint::Callable(_) => out.kinds.push(K::Callable),
        Hint::Static(_) => out.kinds.push(K::StaticType),
        Hint::Self_(_) => out.kinds.push(K::SelfType),
        Hint::Parent(_) => out.kinds.push(K::Parent),
        Hint::Void(_) => out.kinds.push(K::Void),
        Hint::Never(_) => out.kinds.push(K::Never),
        Hint::Float(_) => out.kinds.push(K::Float),
        Hint::Bool(_) => out.kinds.push(K::Bool),
        Hint::Integer(_) => out.kinds.push(K::Int),
        Hint::String(_) => out.kinds.push(K::String),
        Hint::Object(_) => out.kinds.push(K::Object),
        Hint::Mixed(_) => out.kinds.push(K::Mixed),
        Hint::Iterable(_) => out.kinds.push(K::Iterable),
        Hint::Identifier(id) => {
            check_type_name(cx, id);
            out.kinds
                .push(K::Named(PhpStr::from_bytes(cx.resolve_class_name(id))))
        }
    }
}

/// zend_compile_single_typename's compile errors for a class-name type: a
/// builtin type name written fully qualified (`\int`) or namespace-relative
/// (`namespace\static`) is "Type declaration '%s' must be unqualified" (php
/// 8.5.12 added `static` to the builtin list); `\self` is an invalid class
/// name; and a class name ending in a reserved word (`Foo\int`) is refused.
fn check_type_name(cx: &FileCtx, id: &Identifier) {
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
        return cx.fatal(at, msg);
    }
    // zend_resolve_class_name: `\self` / `namespace\parent` name no class.
    if !prefix.is_empty() && (lc == b"self" || lc == b"parent") {
        let msg = format!(
            "'{prefix}{}' is an invalid class name",
            String::from_utf8_lossy(&name)
        );
        return cx.fatal(at, msg);
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
    }
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
