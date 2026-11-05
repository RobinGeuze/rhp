//! Constant-expression validation: zend_eval_const_expr's errors, then
//! zend_compile_const_expr's (zend_is_allowed_in_const_expr and the per-kind
//! checks). Constant expressions are class constants, property defaults,
//! enum case values (no `new`), parameter defaults, attribute arguments and
//! `const` declarations.

use crate::parse::cst::{
    Access, Argument, ArrayElement, BinaryOperator, ClassLikeConstantSelector,
    ClassLikeMemberSelector, Expression, PartialApplication, UnaryPrefixOperator,
};
use mago_span::HasSpan;

use crate::file::FileCtx;

type Fail = (u32, String);

fn fail<T>(e: &Expression, msg: impl Into<String>) -> Result<T, Fail> {
    Err((e.span().start.offset, msg.into()))
}

/// Check `e`; on the first error record it with [`FileCtx::fatal`] and
/// return false. `allow_dynamic` permits `new` and object casts;
/// `scope_known` is zend_is_scope_known for `self`/`parent` checks.
pub(crate) fn check_const_expr(
    cx: &FileCtx,
    e: &Expression,
    allow_dynamic: bool,
    scope_known: bool,
) -> bool {
    if cx.is_prelude {
        return true;
    }
    let r = eval_pass(cx, e, scope_known).and_then(|()| compile_pass(e, allow_dynamic));
    match r {
        Ok(()) => true,
        Err((at, msg)) => {
            cx.fatal(at, msg);
            false
        }
    }
}

/// The nearest expressions below `e`.
fn children<'a>(e: &'a Expression<'a>) -> Vec<&'a Expression<'a>> {
    use crate::parse::cst::Node;
    fn walk<'a>(n: Node<'a, 'a>, out: &mut Vec<&'a Expression<'a>>) {
        for c in n.children() {
            match c {
                Node::Expression(x) => out.push(x),
                other => walk(other, out),
            }
        }
    }
    let mut out = Vec::new();
    walk(Node::Expression(e), &mut out);
    out
}

/// zend_eval_const_expr (children first).
fn eval_pass(cx: &FileCtx, e: &Expression, scope_known: bool) -> Result<(), Fail> {
    match e {
        Expression::Closure(_) | Expression::ArrowFunction(_) | Expression::AnonymousClass(_) => {
            return Ok(())
        }
        Expression::Array(a) => {
            for el in a.elements.iter() {
                if let ArrayElement::Missing(_) = el {
                    return fail(e, "Cannot use empty array elements in arrays");
                }
            }
        }
        Expression::LegacyArray(a) => {
            for el in a.elements.iter() {
                if let ArrayElement::Missing(_) = el {
                    return fail(e, "Cannot use empty array elements in arrays");
                }
            }
        }
        Expression::List(_) => return fail(e, "Cannot use list() as standalone expression"),
        _ => {}
    }
    for c in children(e) {
        eval_pass(cx, c, scope_known)?;
    }
    match e {
        Expression::ArrayAppend(_) => fail(e, "Cannot use [] for reading"),
        Expression::UnaryPrefix(u) if matches!(u.operator, UnaryPrefixOperator::UnsetCast(..)) => {
            fail(e, "The (unset) cast is no longer supported")
        }
        Expression::Access(Access::ClassConstant(cca))
            if is_class_name(&cca.constant) && scope_known =>
        {
            let kind = match cca.class {
                Expression::Self_(_) => "self",
                Expression::Parent(_) => "parent",
                Expression::Static(_) => "static",
                _ => return Ok(()),
            };
            match cx.class_scope.borrow().as_ref() {
                None => fail(
                    e,
                    format!("Cannot use \"{kind}\" when no class scope is active"),
                ),
                Some(cs) if kind == "parent" && cs.parent.is_none() => fail(
                    e,
                    "Cannot use \"parent\" when current class scope has no parent",
                ),
                Some(_) => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

fn is_class_name(sel: &ClassLikeConstantSelector) -> bool {
    matches!(sel, ClassLikeConstantSelector::Identifier(li) if li.value.eq_ignore_ascii_case(b"class"))
}

fn invalid<T>(e: &Expression) -> Result<T, Fail> {
    fail(e, "Constant expression contains invalid operations")
}

/// zend_compile_const_expr (the node, then its children).
fn compile_pass(e: &Expression, allow_dynamic: bool) -> Result<(), Fail> {
    use UnaryPrefixOperator as Op;
    match e {
        Expression::Literal(_)
        | Expression::Parenthesized(_)
        | Expression::Conditional(_)
        | Expression::ArrayAccess(_)
        | Expression::Array(_)
        | Expression::LegacyArray(_)
        | Expression::ConstantAccess(_)
        | Expression::Identifier(_)
        | Expression::MagicConstant(_)
        | Expression::Self_(_)
        | Expression::Parent(_)
        | Expression::Static(_)
        | Expression::Access(Access::Property(_) | Access::NullSafeProperty(_)) => {}
        Expression::Binary(b) => {
            if matches!(b.operator, BinaryOperator::Instanceof(_)) {
                return invalid(e);
            }
        }
        Expression::UnaryPrefix(u) => match &u.operator {
            Op::Reference(_) | Op::ErrorControl(_) | Op::PreIncrement(_) | Op::PreDecrement(_) => {
                return invalid(e)
            }
            Op::ObjectCast(..) if !allow_dynamic => {
                return fail(e, "Object casts are not supported in this context")
            }
            _ => {}
        },
        Expression::Access(Access::ClassConstant(cca)) => {
            if is_class_name(&cca.constant) {
                match cca.class {
                    Expression::Identifier(_) | Expression::Self_(_) | Expression::Parent(_) => {}
                    Expression::Static(_) => {
                        return fail(
                            e,
                            "static::class cannot be used for compile-time class name resolution",
                        )
                    }
                    _ => {
                        return fail(
                            e,
                            "(expression)::class cannot be used in constant expressions",
                        )
                    }
                }
            } else {
                match cca.class {
                    Expression::Identifier(_) | Expression::Self_(_) | Expression::Parent(_) => {}
                    Expression::Static(_) => {
                        return fail(e, "\"static::\" is not allowed in compile-time constants")
                    }
                    _ => {
                        return fail(
                            e,
                            "Dynamic class names are not allowed in compile-time class constant references",
                        )
                    }
                }
            }
        }
        Expression::Instantiation(inst) => {
            if !allow_dynamic {
                return fail(e, "New expressions are not supported in this context");
            }
            match inst.class {
                Expression::Identifier(_) | Expression::Self_(_) | Expression::Parent(_) => {}
                Expression::Static(_) => {
                    return fail(e, "\"static\" is not allowed in compile-time constants")
                }
                _ => return fail(e, "Cannot use dynamic class name in constant expression"),
            }
            if let Some(args) = &inst.argument_list {
                check_args(e, args.arguments.iter())?;
            }
        }
        Expression::AnonymousClass(_) => {
            if !allow_dynamic {
                return fail(e, "New expressions are not supported in this context");
            }
            return fail(e, "Cannot use anonymous class in constant expression");
        }
        Expression::Closure(cl) => {
            if cl.r#static.is_none() {
                return fail(e, "Closures in constant expressions must be static");
            }
            if cl.use_clause.is_some() {
                return fail(e, "Cannot use(...) variables in constant expression");
            }
            return Ok(());
        }
        Expression::PartialApplication(pa) => {
            if !pa.is_first_class_callable() {
                return invalid(e);
            }
            match pa {
                PartialApplication::Function(f) => {
                    if !matches!(f.function, Expression::Identifier(_)) {
                        return fail(e, "Cannot use dynamic function name in constant expression");
                    }
                }
                PartialApplication::StaticMethod(s) => {
                    match s.class {
                        Expression::Identifier(_)
                        | Expression::Self_(_)
                        | Expression::Parent(_) => {}
                        Expression::Static(_) => {
                            return fail(e, "\"static\" is not allowed in compile-time constants")
                        }
                        _ => {
                            return fail(e, "Cannot use dynamic class name in constant expression")
                        }
                    }
                    if !matches!(s.method, ClassLikeMemberSelector::Identifier(_)) {
                        return fail(e, "Cannot use dynamic method name in constant expression");
                    }
                }
                PartialApplication::Method(_) => return invalid(e),
            }
            return Ok(());
        }
        _ => return invalid(e),
    }
    for c in children(e) {
        compile_pass(c, allow_dynamic)?;
    }
    Ok(())
}

/// zend_compile_const_expr_args.
fn check_args<'a>(
    e: &Expression,
    args: impl Iterator<Item = &'a Argument<'a>>,
) -> Result<(), Fail> {
    let mut named = false;
    for a in args {
        match a {
            Argument::Positional(p) if p.ellipsis.is_some() => {
                return fail(
                    e,
                    "Argument unpacking in constant expressions is not supported",
                )
            }
            Argument::Named(_) => named = true,
            Argument::Positional(_) if named => {
                return fail(e, "Cannot use positional argument after named argument")
            }
            _ => {}
        }
    }
    Ok(())
}
