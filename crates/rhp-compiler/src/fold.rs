//! Compile-time constants: folding constant expressions and php's operand kinds.

use crate::parse::cst::{
    ArrayElement, Construct, Expression, Literal, MagicConstant, UnaryPrefixOperator, Variable,
};
use rhp_values::{ArrayKey, PhpArray, Thrown, Value};

use crate::file::FileCtx;
use crate::names::identifier_name;
use crate::util::{dirname, is_superglobal, strip_dollar};

/// Fold a compile-time-constant expression (literals, const arrays, true/false/
/// null, negated numeric literals) to a `Value`. Errors on anything requiring
/// runtime evaluation (named constants, class constants, arithmetic).
pub(crate) fn fold_const(cx: &FileCtx, e: &Expression) -> Result<Value, Thrown> {
    let v = match e {
        Expression::Literal(l) => literal_value(l),
        // A heredoc / nowdoc / "..." without interpolation is a plain string.
        Expression::CompositeString(_) => match crate::ast_export::plain_string(e) {
            Some(s) => Value::str(s),
            None => return Err(non_const_err()),
        },
        Expression::Parenthesized(p) => return fold_const(cx, p.expression),
        // Context-free magic constants fold in const / default-value positions
        // (`const ROOT = __DIR__;`, `function f($p = __FILE__)`). The scope-
        // dependent ones (__CLASS__/__FUNCTION__/__METHOD__) are handled by the
        // ProtoBuilder path, not here.
        Expression::MagicConstant(mc) => {
            use mago_span::HasSpan;
            match mc {
                MagicConstant::Line(_) => Value::Int(cx.line_of(mc.span().start.offset) as i64),
                MagicConstant::File(_) => Value::str(cx.file_name()),
                MagicConstant::Directory(_) => Value::str(dirname(cx.builtins, cx.file.as_bytes())),
                MagicConstant::Namespace(_) => Value::str(cx.cur_ns()),
                _ => return Err(non_const_err()),
            }
        }
        Expression::Array(arr) => return fold_const_array(cx, arr.elements.iter()),
        Expression::LegacyArray(arr) => return fold_const_array(cx, arr.elements.iter()),
        Expression::ConstantAccess(ca) => match identifier_name(&ca.name) {
            b"true" | b"TRUE" | b"True" => Value::Bool(true),
            b"false" | b"FALSE" | b"False" => Value::Bool(false),
            b"null" | b"NULL" | b"Null" => Value::Null,
            _ => return Err(non_const_err()),
        },
        Expression::UnaryPrefix(u) => {
            use UnaryPrefixOperator as Op;
            let inner = fold_const(cx, u.operand)?;
            match (&u.operator, inner) {
                (Op::Negation(_), Value::Int(n)) => Value::Int(-n),
                (Op::Negation(_), Value::Float(f)) => Value::Float(-f),
                (Op::Plus(_), v @ (Value::Int(_) | Value::Float(_))) => v,
                (Op::BitwiseNot(_), v) => rhp_values::ops::bitwise_not(&v)?,
                (Op::Not(_), v) => Value::Bool(!rhp_values::conv::to_bool(&v)),
                _ => return Err(non_const_err()),
            }
        }
        Expression::Binary(b) => {
            use crate::parse::cst::BinaryOperator as Bop;
            use rhp_values::ops::{self, BitOp};
            let l = fold_const(cx, b.lhs)?;
            let r = fold_const(cx, b.rhs)?;
            match &b.operator {
                Bop::Addition(_) => ops::add(&l, &r)?,
                Bop::Subtraction(_) => ops::sub(&l, &r)?,
                Bop::Multiplication(_) => ops::mul(&l, &r)?,
                Bop::Division(_) => ops::div(&l, &r)?,
                Bop::Modulo(_) => ops::modulo(&l, &r)?,
                Bop::Exponentiation(_) => ops::pow(&l, &r)?,
                Bop::StringConcat(_) => {
                    // Arrays warn (Array to string conversion): left to run time.
                    if matches!(l, Value::Array(_)) || matches!(r, Value::Array(_)) {
                        return Err(non_const_err());
                    }
                    ops::concat(&l, &r)
                }
                Bop::BitwiseAnd(_) => ops::bitwise(&l, &r, BitOp::And)?,
                Bop::BitwiseOr(_) => ops::bitwise(&l, &r, BitOp::Or)?,
                Bop::BitwiseXor(_) => ops::bitwise(&l, &r, BitOp::Xor)?,
                Bop::LeftShift(_) => ops::bitwise(&l, &r, BitOp::Shl)?,
                Bop::RightShift(_) => ops::bitwise(&l, &r, BitOp::Shr)?,
                Bop::Equal(_) => Value::Bool(ops::loose_eq(&l, &r)),
                Bop::NotEqual(_) | Bop::AngledNotEqual(_) => Value::Bool(!ops::loose_eq(&l, &r)),
                Bop::Identical(_) => Value::Bool(ops::strict_eq(&l, &r)),
                Bop::NotIdentical(_) => Value::Bool(!ops::strict_eq(&l, &r)),
                Bop::LessThan(_) => Value::Bool(ops::loose_compare(&l, &r).is_lt()),
                Bop::LessThanOrEqual(_) => Value::Bool(ops::loose_compare(&l, &r).is_le()),
                Bop::GreaterThan(_) => Value::Bool(ops::loose_compare(&r, &l).is_lt()),
                Bop::GreaterThanOrEqual(_) => Value::Bool(ops::loose_compare(&r, &l).is_le()),
                Bop::Spaceship(_) => ops::spaceship(&l, &r),
                Bop::And(_) | Bop::LowAnd(_) => {
                    Value::Bool(rhp_values::conv::to_bool(&l) && rhp_values::conv::to_bool(&r))
                }
                Bop::Or(_) | Bop::LowOr(_) => {
                    Value::Bool(rhp_values::conv::to_bool(&l) || rhp_values::conv::to_bool(&r))
                }
                Bop::NullCoalesce(_) => {
                    if matches!(l, Value::Null) {
                        r
                    } else {
                        l
                    }
                }
                _ => return Err(non_const_err()),
            }
        }
        _ => return Err(non_const_err()),
    };
    Ok(v)
}

fn fold_const_array<'a>(
    cx: &FileCtx,
    els: impl Iterator<Item = &'a ArrayElement<'a>>,
) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    for el in els {
        match el {
            ArrayElement::KeyValue(kv) => {
                let k = fold_const(cx, kv.key)?;
                let v = fold_const(cx, kv.value)?;
                // Keys php converts with a diagnostic (null, a float that
                // isn't a whole int, a resource) or rejects: left to run time.
                let key = match &k {
                    Value::Int(n) => ArrayKey::Int(*n),
                    Value::Str(s) => ArrayKey::from_bytes(s.as_bytes()),
                    Value::Bool(b) => ArrayKey::Int(*b as i64),
                    Value::Float(f)
                        if f.fract() == 0.0
                            && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0)
                                .contains(f) =>
                    {
                        ArrayKey::Int(*f as i64)
                    }
                    _ => return Err(non_const_err()),
                };
                a.insert(key, v);
            }
            ArrayElement::Value(v) => a.push(fold_const(cx, v.value)?),
            ArrayElement::Variadic(_) => return Err(non_const_err()),
            ArrayElement::Missing(_) => {}
        }
    }
    Ok(Value::Array(a))
}

/// An array literal built only from int/string-keyed scalar literals,
/// `true`/`false`/`null` and nested such arrays, as its value (`None` when
/// building it could raise a diagnostic or needs runtime evaluation).
pub(crate) fn literal_array<'a>(els: impl Iterator<Item = &'a ArrayElement<'a>>) -> Option<Value> {
    fn scalar(e: &Expression) -> Option<Value> {
        match e {
            Expression::Literal(l) => Some(literal_value(l)),
            Expression::Array(arr) => literal_array(arr.elements.iter()),
            Expression::LegacyArray(arr) => literal_array(arr.elements.iter()),
            Expression::ConstantAccess(ca) => match identifier_name(&ca.name) {
                b"true" | b"TRUE" | b"True" => Some(Value::Bool(true)),
                b"false" | b"FALSE" | b"False" => Some(Value::Bool(false)),
                b"null" | b"NULL" | b"Null" => Some(Value::Null),
                _ => None,
            },
            Expression::UnaryPrefix(u) => match (&u.operator, scalar(u.operand)?) {
                (UnaryPrefixOperator::Negation(_), Value::Int(n)) => {
                    n.checked_neg().map(Value::Int)
                }
                (UnaryPrefixOperator::Negation(_), Value::Float(f)) => Some(Value::Float(-f)),
                _ => None,
            },
            _ => None,
        }
    }
    let mut a = PhpArray::new();
    for el in els {
        match el {
            ArrayElement::KeyValue(kv) => {
                // Only int / string keys: float, bool and null keys convert
                // with diagnostics or php-specific rules at run time.
                let key = match scalar(kv.key)? {
                    Value::Int(n) => ArrayKey::Int(n),
                    k @ Value::Str(_) => ArrayKey::from_value(&k)?,
                    _ => return None,
                };
                // A PHP_INT_MAX key makes a later append fail at run time.
                if matches!(key, ArrayKey::Int(i64::MAX)) {
                    return None;
                }
                a.insert(key, scalar(kv.value)?);
            }
            ArrayElement::Value(v) => a.push(scalar(v.value)?),
            _ => return None,
        }
    }
    Some(Value::Array(a))
}

/// Whether php's VM swaps the operands of a commutative opcode: op1's
/// kind is lower than op2's, in IS_CONST < IS_TMP_VAR < IS_VAR < IS_CV.
/// Reads of elements and properties are TMP (zend_adjust_for_fetch_type);
/// calls, `new`, `yield` and `include`/`eval` are VAR. (php's frameless
/// and special-cased function calls give TMP; they count as VAR here.)
pub(crate) fn php_swaps_operands(cx: &FileCtx, lhs: &Expression, rhs: &Expression) -> bool {
    match (php_operand_kind(lhs), php_operand_kind(rhs)) {
        (l, r) if l != r => l < r,
        // Both CONST-or-TMP: only a CONST op1 against a TMP op2 swaps.
        (Kind::ConstOrTmp, Kind::ConstOrTmp) => !is_ct_const(cx, rhs) && is_ct_const(cx, lhs),
        _ => false,
    }
}

/// zend_try_ct_eval_const: `true`/`false`/`null` in any case, or a
/// global builtin constant that is not deprecated.
pub(crate) fn ct_constant(cx: &FileCtx, id: &crate::parse::cst::Identifier) -> Option<Value> {
    let namespaced_fq = matches!(id, crate::parse::cst::Identifier::FullyQualified(f) if f.value[1..].contains(&b'\\'));
    if !namespaced_fq {
        match identifier_name(id).to_ascii_lowercase().as_slice() {
            b"true" => return Some(Value::Bool(true)),
            b"false" => return Some(Value::Bool(false)),
            b"null" => return Some(Value::Null),
            _ => {}
        }
    }
    let (name, fallback) = cx.resolve_name(id, crate::names::NameKind::Const);
    // php-cli's stream resources are per-request constants, never
    // substituted (can_ct_eval_const).
    if fallback.is_some() || matches!(name.as_slice(), b"STDIN" | b"STDOUT" | b"STDERR") {
        return None;
    }
    (cx.builtins.constant)(&name)
}

/// Whether php's compiler folds `e` to a constant. A binary operation
/// checks its right operand first, so the left spine of a long chain is
/// not walked at every level.
fn is_ct_const(cx: &FileCtx, e: &Expression) -> bool {
    match e {
        Expression::Parenthesized(p) => is_ct_const(cx, p.expression),
        Expression::MagicConstant(_) => true,
        // `PHP_INT_MAX` is a CONST operand.
        Expression::ConstantAccess(ca) => ct_constant(cx, &ca.name).is_some(),
        Expression::Binary(b) => {
            is_ct_const(cx, b.rhs) && is_ct_const(cx, b.lhs) && fold_const(cx, e).is_ok()
        }
        _ => fold_const(cx, e).is_ok(),
    }
}

#[derive(PartialEq, PartialOrd)]
enum Kind {
    ConstOrTmp,
    Var,
    Cv,
}

fn php_operand_kind(e: &Expression) -> Kind {
    match e {
        Expression::Parenthesized(p) => php_operand_kind(p.expression),
        Expression::Variable(Variable::Direct(d)) => {
            let n = strip_dollar(d.name);
            if n == b"this" || is_superglobal(n) {
                Kind::ConstOrTmp
            } else {
                Kind::Cv
            }
        }
        Expression::Call(_)
        | Expression::Instantiation(_)
        | Expression::AnonymousClass(_)
        | Expression::Yield(_) => Kind::Var,
        Expression::Construct(
            Construct::Eval(_)
            | Construct::Include(_)
            | Construct::IncludeOnce(_)
            | Construct::Require(_)
            | Construct::RequireOnce(_),
        ) => Kind::Var,
        _ => Kind::ConstOrTmp,
    }
}

fn non_const_err() -> Thrown {
    Thrown::unsupported(
        "non-constant initializer",
        "only literal / array / true/false/null constant initializers for now",
    )
}

/// Fold a literal AST node to its runtime value (php's scanner rules: an
/// integer literal past `i64::MAX` becomes a float).
pub(crate) fn literal_value(lit: &Literal) -> Value {
    match lit {
        Literal::Integer(i) => match i.value {
            Some(u) if u <= i64::MAX as u64 => Value::Int(u as i64),
            // The parser's value saturates: php reads the digits as a double
            // (zend_strtod, or zend_hex/oct/bin_strtod).
            _ => Value::Float(big_int_literal(i.raw)),
        },
        Literal::Float(f) => Value::Float(f.value.0),
        Literal::String(s) => Value::str(s.value.unwrap_or(s.raw).to_vec()),
        Literal::True(_) => Value::Bool(true),
        Literal::False(_) => Value::Bool(false),
        Literal::Null(_) => Value::Null,
    }
}

/// An integer literal too big for an int, as php's scanner makes it a float.
fn big_int_literal(raw: &[u8]) -> f64 {
    let digits: Vec<u8> = raw.iter().copied().filter(|&b| b != b'_').collect();
    let (base, body) = match digits.as_slice() {
        [b'0', b'x' | b'X', rest @ ..] => (16, rest),
        [b'0', b'b' | b'B', rest @ ..] => (2, rest),
        [b'0', b'o' | b'O', rest @ ..] => (8, rest),
        [b'0', rest @ ..] if !rest.is_empty() => (8, rest),
        d => {
            return std::str::from_utf8(d)
                .ok()
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0)
        }
    };
    body.iter().fold(0.0, |acc, &c| {
        if base == 16 {
            acc * 16.0 + (c as char).to_digit(16).unwrap_or(0) as f64
        } else {
            // zend_oct/bin_strtod: `value * base + c - '0'`, two roundings.
            acc * base as f64 + c as f64 - b'0' as f64
        }
    })
}

/// Structural equality for constant-pool deduplication.
pub(crate) fn const_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
        (Value::Str(x), Value::Str(y)) => x == y,
        _ => false,
    }
}
