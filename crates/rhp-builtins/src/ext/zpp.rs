//! Small `zend_parse_parameters`-style argument helpers shared by the
//! extension modules: deref + coerce one positional argument the way php's
//! non-strict-mode internal-function parameter parsing does, raising the same
//! `TypeError` text for values that cannot be coerced.

use rhp_values::{conv, Value};

use crate::{Host, Thrown};

/// Deref a (possibly `Ref`) value to an owned value.
pub fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

/// The type name php's ZPP uses in "must be of type X, Y given" messages.
pub fn given_type(host: &dyn Host, v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(_) => "bool".into(),
        Value::Int(_) => "int".into(),
        Value::Float(_) => "float".into(),
        Value::Str(_) => "string".into(),
        Value::Array(_) => "array".into(),
        Value::Closure(_) => "Closure".into(),
        Value::Object(_) => host
            .object_class_name(v)
            .map(|n| String::from_utf8_lossy(&n).into_owned())
            .unwrap_or_else(|| "object".into()),
        Value::Resource(_) => "resource".into(),
        Value::Ref(r) => given_type(host, &r.borrow()),
    }
}

fn type_err(host: &dyn Host, fname: &str, n: usize, pname: &str, want: &str, v: &Value) -> Thrown {
    Thrown::type_error(format!(
        "{fname}(): Argument #{n} (${pname}) must be of type {want}, {} given",
        given_type(host, v)
    ))
}

/// A `string` parameter (`Z_PARAM_STR`). Scalars and null coerce; arrays,
/// resources and objects without `__toString` raise a `TypeError`.
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
        Value::Object(_) => {
            let cls = host.object_class_name(&v);
            let has = cls
                .as_deref()
                .and_then(|c| host.resolve_class(c))
                .is_some_and(|id| host.class_has_method(id, b"__tostring"));
            if has {
                let r = host.call_callable(
                    &Value::Array({
                        let mut a = rhp_values::PhpArray::new();
                        a.push(v.clone());
                        a.push(Value::str("__toString"));
                        a
                    }),
                    &[],
                )?;
                Ok(conv::to_string_bytes(&r))
            } else {
                Err(type_err(host, fname, i + 1, pname, "string", &v))
            }
        }
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
            "{fname}(): Argument #{} (${pname}) must be of type int, {} given",
            i + 1,
            crate::argparse::value_name(host, v)
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
