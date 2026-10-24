//! PHP operator semantics (arithmetic, comparison, bitwise, concat).
//!
//! These can raise `Throwable`s (`DivisionByZeroError`, `TypeError`), so they
//! live here rather than in `rhp-values`. All rules verified against
//! php 8.5.8 (see `docs/design/php-semantics.md`).

use std::cmp::Ordering;

use rhp_values::Thrown;
use rhp_values::conv::{self, Num};
use rhp_values::{PhpStr, Value};

/// Coerce an operand to a number for arithmetic, raising `TypeError` for
/// unsupported types and non-numeric strings (PHP 8 behavior).
fn arith_num(v: &Value, op: &str) -> Result<Num, Thrown> {
    match v {
        Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) => Ok(conv::to_number(v).0),
        Value::Str(s) => match conv::scan_number(s.as_bytes()) {
            // leading-numeric strings are allowed (PHP warns; we accept)
            Some((num, _)) => Ok(num),
            None => Err(Thrown::type_error(format!(
                "Unsupported operand types: string {op} int"
            ))),
        },
        Value::Array(_) => Err(Thrown::type_error(format!("Unsupported operand types: array {op} int"))),
        Value::Object(_) | Value::Closure(_) => {
            Err(Thrown::type_error(format!("Unsupported operand types: object {op} int")))
        }
        Value::Ref(r) => arith_num(&r.borrow(), op),
    }
}

/// `+` `-` `*` on two numbers, with int-overflow → float promotion.
fn arith_add_sub_mul(a: Num, b: Num, f_int: fn(i64, i64) -> Option<i64>, f_flt: fn(f64, f64) -> f64) -> Value {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => match f_int(x, y) {
            Some(n) => Value::Int(n),
            None => Value::Float(f_flt(x as f64, y as f64)), // overflow → float
        },
        _ => Value::Float(f_flt(a.as_f64(), b.as_f64())),
    }
}

pub fn add(a: &Value, b: &Value) -> Result<Value, Thrown> {
    // array + array = union (left wins); only defined for arrays.
    if let (Value::Array(x), Value::Array(y)) = (&crate::deref(a), &crate::deref(b)) {
        let mut out = x.clone();
        for (k, v) in y.iter() {
            if out.get(k).is_none() {
                out.insert(k.clone(), v.clone());
            }
        }
        return Ok(Value::Array(out));
    }
    let (x, y) = (arith_num(a, "+")?, arith_num(b, "+")?);
    Ok(arith_add_sub_mul(x, y, i64::checked_add, |p, q| p + q))
}

pub fn sub(a: &Value, b: &Value) -> Result<Value, Thrown> {
    let (x, y) = (arith_num(a, "-")?, arith_num(b, "-")?);
    Ok(arith_add_sub_mul(x, y, i64::checked_sub, |p, q| p - q))
}

pub fn mul(a: &Value, b: &Value) -> Result<Value, Thrown> {
    let (x, y) = (arith_num(a, "*")?, arith_num(b, "*")?);
    Ok(arith_add_sub_mul(x, y, i64::checked_mul, |p, q| p * q))
}

pub fn div(a: &Value, b: &Value) -> Result<Value, Thrown> {
    let (x, y) = (arith_num(a, "/")?, arith_num(b, "/")?);
    if y.as_f64() == 0.0 {
        return Err(Thrown::error("DivisionByZeroError", "Division by zero"));
    }
    match (x, y) {
        (Num::Int(p), Num::Int(q)) if q != 0 && p % q == 0 => Ok(Value::Int(p / q)),
        _ => Ok(Value::Float(x.as_f64() / y.as_f64())),
    }
}

pub fn modulo(a: &Value, b: &Value) -> Result<Value, Thrown> {
    let p = conv::to_int(&crate::deref(a));
    let q = conv::to_int(&crate::deref(b));
    if q == 0 {
        return Err(Thrown::error("DivisionByZeroError", "Modulo by zero"));
    }
    if q == -1 {
        return Ok(Value::Int(0)); // avoid i64::MIN % -1 overflow panic
    }
    Ok(Value::Int(p % q)) // Rust % has sign of dividend, like PHP
}

pub fn pow(a: &Value, b: &Value) -> Result<Value, Thrown> {
    let x = arith_num(a, "**")?;
    let y = arith_num(b, "**")?;
    if let (Num::Int(base), Num::Int(exp)) = (x, y) {
        if exp >= 0 {
            // integer power with overflow → float
            let mut acc: i64 = 1;
            let mut overflow = false;
            for _ in 0..exp {
                match acc.checked_mul(base) {
                    Some(v) => acc = v,
                    None => {
                        overflow = true;
                        break;
                    }
                }
            }
            if !overflow {
                return Ok(Value::Int(acc));
            }
            return Ok(Value::Float((base as f64).powf(exp as f64)));
        }
    }
    Ok(Value::Float(x.as_f64().powf(y.as_f64())))
}

pub fn bitwise(a: &Value, b: &Value, op: BitOp) -> Value {
    let x = conv::to_int(&crate::deref(a));
    let y = conv::to_int(&crate::deref(b));
    Value::Int(match op {
        BitOp::And => x & y,
        BitOp::Or => x | y,
        BitOp::Xor => x ^ y,
        BitOp::Shl => x.wrapping_shl(y as u32),
        BitOp::Shr => x.wrapping_shr(y as u32),
    })
}

pub enum BitOp {
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

pub fn bitwise_not(a: &Value) -> Value {
    Value::Int(!conv::to_int(&crate::deref(a)))
}

pub fn negate(a: &Value) -> Result<Value, Thrown> {
    match arith_num(a, "-")? {
        Num::Int(n) => match n.checked_neg() {
            Some(v) => Ok(Value::Int(v)),
            None => Ok(Value::Float(-(n as f64))), // -i64::MIN → float
        },
        Num::Float(f) => Ok(Value::Float(-f)),
    }
}

pub fn concat(a: &Value, b: &Value) -> Value {
    let mut bytes = conv::to_string_bytes(&crate::deref(a));
    bytes.extend_from_slice(&conv::to_string_bytes(&crate::deref(b)));
    Value::Str(PhpStr::from_bytes(bytes))
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

/// PHP loose comparison (`<`, `<=`, `>`, `>=`, `<=>`, and `==` via `== Equal`).
/// Verified against php 8.5.8 (comparison_php8 / spaceship / string_ordering).
pub fn loose_compare(a: &Value, b: &Value) -> Ordering {
    let a = crate::deref(a);
    let b = crate::deref(b);
    use Value::*;
    match (&a, &b) {
        (Null, Null) => Ordering::Equal,
        // bool on either side: compare as bools.
        (Bool(_), _) | (_, Bool(_)) => conv::to_bool(&a).cmp(&conv::to_bool(&b)),
        // null vs X: treat null as the "zero" of X's type.
        (Null, _) => loose_compare(&null_as(&b), &b),
        (_, Null) => loose_compare(&a, &null_as(&a)),
        // numbers
        (Int(_) | Float(_), Int(_) | Float(_)) => cmp_num(conv::to_number(&a).0, conv::to_number(&b).0),
        // number vs string
        (Int(_) | Float(_), Str(s)) => match conv::numeric_string(s.as_bytes()) {
            Some(n) => cmp_num(conv::to_number(&a).0, n),
            None => cmp_bytes(&conv::to_string_bytes(&a), s.as_bytes()),
        },
        (Str(s), Int(_) | Float(_)) => match conv::numeric_string(s.as_bytes()) {
            Some(n) => cmp_num(n, conv::to_number(&b).0),
            None => cmp_bytes(s.as_bytes(), &conv::to_string_bytes(&b)),
        },
        // string vs string
        (Str(x), Str(y)) => {
            match (conv::numeric_string(x.as_bytes()), conv::numeric_string(y.as_bytes())) {
                (Some(nx), Some(ny)) => cmp_num(nx, ny),
                _ => cmp_bytes(x.as_bytes(), y.as_bytes()),
            }
        }
        // arrays
        (Array(x), Array(y)) => match x.len().cmp(&y.len()) {
            Ordering::Equal => {
                for (k, xv) in x.iter() {
                    match y.get(k) {
                        Some(yv) => match loose_compare(xv, yv) {
                            Ordering::Equal => continue,
                            other => return other,
                        },
                        None => return Ordering::Greater, // uncomparable; PHP: not equal
                    }
                }
                Ordering::Equal
            }
            other => other,
        },
        (Array(_), _) => Ordering::Greater, // array > scalar
        (_, Array(_)) => Ordering::Less,
        // objects: identity/placeholder until phase 7
        _ => Ordering::Equal,
    }
}

/// The value `null` is compared *as* when the other side has a concrete type.
fn null_as(other: &Value) -> Value {
    match other {
        Value::Int(_) => Value::Int(0),
        Value::Float(_) => Value::Float(0.0),
        Value::Str(_) => Value::str(""),
        Value::Array(_) => Value::Array(rhp_values::PhpArray::new()),
        _ => Value::Null,
    }
}

fn cmp_num(a: Num, b: Num) -> Ordering {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => x.cmp(&y),
        _ => a.as_f64().partial_cmp(&b.as_f64()).unwrap_or(Ordering::Equal),
    }
}

fn cmp_bytes(a: &[u8], b: &[u8]) -> Ordering {
    a.cmp(b)
}

pub fn loose_eq(a: &Value, b: &Value) -> bool {
    loose_compare(a, b) == Ordering::Equal
}

/// PHP strict comparison (`===`): same type and value; identity for objects.
pub fn strict_eq(a: &Value, b: &Value) -> bool {
    let a = crate::deref(a);
    let b = crate::deref(b);
    use Value::*;
    match (&a, &b) {
        (Null, Null) => true,
        (Bool(x), Bool(y)) => x == y,
        (Int(x), Int(y)) => x == y,
        (Float(x), Float(y)) => x == y, // NaN !== NaN, as PHP
        (Str(x), Str(y)) => x == y,
        (Array(x), Array(y)) => {
            x.len() == y.len()
                && x.iter().zip(y.iter()).all(|((xk, xv), (yk, yv))| xk == yk && strict_eq(xv, yv))
        }
        (Object(x), Object(y)) => std::rc::Rc::ptr_eq(x, y),
        _ => false,
    }
}

/// The `<=>` result as a PHP int.
pub fn spaceship(a: &Value, b: &Value) -> Value {
    Value::Int(match loose_compare(a, b) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    })
}
