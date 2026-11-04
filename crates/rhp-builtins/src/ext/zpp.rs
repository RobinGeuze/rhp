//! Small `zend_parse_parameters`-style argument helpers shared by the
//! extension modules: deref + coerce one positional argument the way php's
//! non-strict-mode internal-function parameter parsing does, raising the same
//! `TypeError` text for values that cannot be coerced.

use rhp_values::{conv, Value};

use crate::{Host, Thrown};

pub use rhp_values::ops::deref;

fn type_err(host: &dyn Host, fname: &str, n: usize, pname: &str, want: &str, v: &Value) -> Thrown {
    Thrown::type_error(format!(
        "{fname}(): Argument #{n} (${pname}) must be of type {want}, {} given",
        crate::argparse::zval_value_name(host, v)
    ))
}

/// A `string` parameter (`Z_PARAM_STR`). Scalars and null coerce; arrays,
/// resources and objects without a string cast raise a `TypeError`.
pub fn str_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Vec<u8>, Thrown> {
    let v = args.get(i).map(deref).unwrap_or(Value::Null);
    match &v {
        Value::Str(s) => Ok(s.as_bytes().to_vec()),
        Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) => {
            Ok(conv::to_string_bytes(&v))
        }
        Value::Object(_) => match crate::object::to_string(host, &v)? {
            Some(s) => Ok(s),
            None => Err(type_err(host, fname, i + 1, pname, "string", &v)),
        },
        _ => Err(type_err(host, fname, i + 1, pname, "string", &v)),
    }
}

/// An optional `?string` parameter: `None` when absent or null.
pub fn opt_str_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Option<Vec<u8>>, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => str_arg(host, args, i, fname, pname).map(Some),
    }
}

/// An `int` parameter (`Z_PARAM_LONG`): ints, bools, null, integral floats and
/// numeric strings coerce; anything else raises a `TypeError`.
pub fn int_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<i64, Thrown> {
    let v = args.get(i).map(deref).unwrap_or(Value::Null);
    match &v {
        Value::Int(n) => Ok(*n),
        Value::Bool(b) => Ok(*b as i64),
        Value::Null => Ok(0),
        Value::Float(f) => {
            if f.is_finite() && *f >= -9.223_372_036_854_776e18 && *f < 9.223_372_036_854_776e18 {
                Ok(*f as i64)
            } else {
                Err(type_err(host, fname, i + 1, pname, "int", &v))
            }
        }
        Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
            Some(conv::Num::Int(n)) => Ok(n),
            Some(conv::Num::Float(f)) if f.is_finite() => Ok(f as i64),
            _ => Err(type_err(host, fname, i + 1, pname, "int", &v)),
        },
        _ => Err(type_err(host, fname, i + 1, pname, "int", &v)),
    }
}

/// zpp `l` (`zend_parse_arg_long`) in the caller's typing mode, for a
/// builtin that a prelude wrapper passes the raw argument: an int; in
/// coercive mode also a bool, null (deprecated), or a float or numeric
/// string that fits, with php's deprecation when a fraction is dropped.
pub fn long_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<i64, Thrown> {
    long_arg_as(host, args, i, fname, pname, "int")
}

/// [`long_arg`] for a `?int` parameter (`Z_PARAM_LONG_OR_NULL`): `None`
/// when absent or null.
pub fn opt_long_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Option<i64>, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => long_arg_as(host, args, i, fname, pname, "?int").map(Some),
    }
}

fn long_arg_as(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
    ty: &str,
) -> Result<i64, Thrown> {
    // ZEND_DOUBLE_FITS_LONG
    const TWO_63: f64 = 9_223_372_036_854_775_808.0;
    let fits = |f: f64| (-TWO_63..TWO_63).contains(&f);
    let v = args.get(i).map(deref).unwrap_or(Value::Null);
    if let Value::Int(n) = v {
        return Ok(n);
    }
    // zend_wrong_parameter_type_error names the value (`true given`).
    let wrong = |host: &dyn Host, v: &Value| {
        Thrown::type_error(format!(
            "{fname}(): Argument #{} (${pname}) must be of type {ty}, {} given",
            i + 1,
            crate::argparse::zval_value_name(host, v)
        ))
    };
    if host.caller_strict_types() {
        return Err(wrong(host, &v));
    }
    match &v {
        Value::Bool(b) => Ok(*b as i64),
        Value::Null => {
            host.emit_diagnostic(
                crate::E_DEPRECATED,
                &format!(
                    "{fname}(): Passing null to parameter #{} (${pname}) of type int is deprecated",
                    i + 1
                ),
            )?;
            Ok(0)
        }
        Value::Float(f) if fits(*f) => {
            if f.fract() != 0.0 {
                let shown =
                    String::from_utf8_lossy(&rhp_values::fmt::float_serialize(*f)).into_owned();
                host.emit_diagnostic(
                    crate::E_DEPRECATED,
                    &format!("Implicit conversion from float {shown} to int loses precision"),
                )?;
            }
            Ok(*f as i64)
        }
        Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
            Some(conv::Num::Int(n)) => Ok(n),
            Some(conv::Num::Float(f)) if fits(f) => {
                if f.fract() != 0.0 {
                    host.emit_diagnostic(
                        crate::E_DEPRECATED,
                        &format!(
                            "Implicit conversion from float-string \"{}\" to int loses precision",
                            String::from_utf8_lossy(s.as_bytes())
                        ),
                    )?;
                }
                Ok(f as i64)
            }
            _ => Err(wrong(host, &v)),
        },
        _ => Err(wrong(host, &v)),
    }
}

/// An optional `int` parameter with a default.
pub fn int_arg_or(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
    default: i64,
) -> Result<i64, Thrown> {
    if args.len() <= i {
        return Ok(default);
    }
    int_arg(host, args, i, fname, pname)
}

/// An optional `?int` parameter: `None` when absent or null.
pub fn opt_int_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Option<i64>, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => int_arg(host, args, i, fname, pname).map(Some),
    }
}

/// A `bool` parameter (`Z_PARAM_BOOL`) with a default.
pub fn bool_arg_or(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
    default: bool,
) -> Result<bool, Thrown> {
    let Some(v) = args.get(i).map(deref) else {
        return Ok(default);
    };
    match &v {
        Value::Array(_) | Value::Object(_) | Value::Closure(_) | Value::Resource(_) => {
            Err(type_err(host, fname, i + 1, pname, "bool", &v))
        }
        _ => Ok(conv::to_bool(&v)),
    }
}

/// php's `zend_argument_value_error`: a `ValueError` naming the argument.
pub fn value_error(fname: &str, n: usize, pname: &str, msg: &str) -> Thrown {
    Thrown::error(
        "ValueError",
        format!("{fname}(): Argument #{n} (${pname}) {msg}"),
    )
}

/// Assign through a by-reference argument slot (no-op when not a reference).
pub fn set_ref(args: &[Value], i: usize, v: Value) {
    if let Some(Value::Ref(r)) = args.get(i) {
        *r.borrow_mut() = v;
    }
}

/// `ZEND_DOUBLE_FITS_LONG`, true for NAN as in C.
fn double_fits_long(d: f64) -> bool {
    const TWO_63: f64 = 9_223_372_036_854_775_808.0;
    !(d >= TWO_63 || d < -TWO_63)
}

/// php 8.5's `zend_dval_to_lval`: a float that is not finite or out of the
/// int range warns; out-of-range values wrap modulo 2^64, others are 0.
fn dval_to_lval(host: &mut dyn Host, d: f64) -> Result<i64, Thrown> {
    if d.is_finite() && double_fits_long(d) {
        return Ok(d as i64);
    }
    let shown = String::from_utf8_lossy(&rhp_values::fmt::float_serialize(d)).into_owned();
    host.emit_diagnostic(
        crate::E_WARNING,
        &format!("The float {shown} is not representable as an int, cast occurred"),
    )?;
    Ok(if d.is_finite() {
        rhp_values::float_to_int(d)
    } else {
        0
    })
}

/// php's `zval_try_get_long` (`zendi_try_get_long`): `None` when the value
/// cannot be an int (non-numeric string, array, …). A float goes through
/// `zend_dval_to_lval_safe` (a warning out of range, the "loses precision"
/// deprecation for a fraction or NAN), a float string through
/// `zend_dval_to_lval_cap` (saturating; 0 when not finite); a
/// leading-numeric string raises the "non-numeric value" warning.
pub fn try_get_long(host: &mut dyn Host, v: &Value) -> Result<Option<i64>, Thrown> {
    Ok(Some(match v {
        Value::Null | Value::Bool(false) => 0,
        Value::Bool(true) => 1,
        Value::Int(n) => *n,
        Value::Float(f) => {
            let l = dval_to_lval(host, *f)?;
            if l as f64 != *f && double_fits_long(*f) {
                let shown =
                    String::from_utf8_lossy(&rhp_values::fmt::float_serialize(*f)).into_owned();
                host.emit_diagnostic(
                    crate::E_DEPRECATED,
                    &format!("Implicit conversion from float {shown} to int loses precision"),
                )?;
            }
            l
        }
        Value::Str(s) => {
            let b = s.as_bytes();
            let Some((num, end)) = conv::scan_number(b) else {
                return Ok(None);
            };
            if !b[end..]
                .iter()
                .all(|&c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
            {
                host.emit_diagnostic(crate::E_WARNING, "A non-numeric value encountered")?;
            }
            match num {
                conv::Num::Int(n) => n,
                conv::Num::Float(f) => {
                    let l = if !f.is_finite() {
                        0
                    } else if !double_fits_long(f) {
                        if f > 0.0 {
                            i64::MAX
                        } else {
                            i64::MIN
                        }
                    } else {
                        f as i64
                    };
                    if l as f64 != f {
                        host.emit_diagnostic(
                            crate::E_DEPRECATED,
                            &format!(
                                "Implicit conversion from float-string \"{}\" to int loses precision",
                                String::from_utf8_lossy(b)
                            ),
                        )?;
                    }
                    l
                }
            }
        }
        _ => return Ok(None),
    }))
}
