//! Small zpp-style parameter helpers (php's `Z_PARAM_*` coercion rules) for
//! builtins that must raise php's exact `TypeError` text on a wrong-typed
//! argument instead of silently coercing.

use rhp_values::ops::deref;
use rhp_values::{conv, PhpArray, Value};

use crate::{Host, Thrown};

/// php's `zend_zval_value_name`, the word zpp and most "…, X given"
/// messages use: the class for an object (cut at a NUL, as `%s` prints an
/// anonymous class), `true`/`false` for a bool, else the type.
pub fn zval_value_name(host: &dyn Host, v: &Value) -> String {
    match deref(v) {
        Value::Bool(true) => "true".into(),
        Value::Bool(false) => "false".into(),
        other => zval_type_name(host, &other),
    }
}

/// php's `zend_zval_type_name`: [`zval_value_name`], but `bool` for both
/// bools (operator errors, typed properties, a few extension messages).
pub fn zval_type_name(host: &dyn Host, v: &Value) -> String {
    match deref(v) {
        Value::Null => "null".into(),
        Value::Bool(_) => "bool".into(),
        Value::Int(_) => "int".into(),
        Value::Float(_) => "float".into(),
        Value::Str(_) => "string".into(),
        Value::Array(_) => "array".into(),
        Value::Closure(_) => "Closure".into(),
        Value::Resource(_) => "resource".into(),
        o @ Value::Object(_) => host
            .object_class_name(&o)
            .map(|n| String::from_utf8_lossy(&crate::dump::c_str(n)).into_owned())
            .unwrap_or_else(|| "object".into()),
        Value::Ref(_) => unreachable!(),
    }
}

/// php's `zval_get_string()`: arrays warn "Array to string conversion",
/// objects go through their string cast (`cast_object`, `__toString`) or
/// throw.
pub(crate) fn zval_get_string(host: &mut dyn Host, v: &Value) -> Result<Vec<u8>, Thrown> {
    match deref(v) {
        Value::Array(_) => {
            host.emit_diagnostic(crate::E_WARNING, "Array to string conversion")?;
            Ok(b"Array".to_vec())
        }
        o @ Value::Object(_) => {
            if let Some(s) = crate::object::to_string(host, &o)? {
                return Ok(s);
            }
            Err(Thrown::error(
                "Error",
                format!(
                    "Object of class {} could not be converted to string",
                    zval_value_name(host, &o)
                ),
            ))
        }
        Value::Closure(_) => Err(Thrown::error(
            "Error",
            "Object of class Closure could not be converted to string",
        )),
        other => Ok(conv::to_string_bytes(&other)),
    }
}

/// php's `zval_get_long()`: a float outside the int range warns and wraps
/// (0 for INF and NAN), a numeric string converts silently (a float one
/// saturates, or is 0 when infinite), an object warns and gives 1.
pub(crate) fn zval_get_long(host: &mut dyn Host, v: &Value) -> Result<i64, Thrown> {
    // ZEND_DOUBLE_FITS_LONG
    const TWO_63: f64 = 9_223_372_036_854_775_808.0;
    match deref(v) {
        Value::Float(f) => {
            if !(-TWO_63..TWO_63).contains(&f) {
                let shown =
                    String::from_utf8_lossy(&rhp_values::fmt::float_serialize(f)).into_owned();
                host.emit_diagnostic(
                    crate::E_WARNING,
                    &format!("The float {shown} is not representable as an int, cast occurred"),
                )?;
            }
            Ok(rhp_values::float_to_int(f))
        }
        Value::Str(s) => Ok(match conv::scan_number(s.as_bytes()) {
            Some((conv::Num::Int(n), _)) => n,
            // zend_dval_to_lval_cap
            Some((conv::Num::Float(f), _)) if !f.is_finite() => 0,
            Some((conv::Num::Float(f), _)) if f >= TWO_63 => i64::MAX,
            Some((conv::Num::Float(f), _)) if f < -TWO_63 => i64::MIN,
            Some((conv::Num::Float(f), _)) => f as i64,
            None => 0,
        }),
        o @ (Value::Object(_) | Value::Closure(_)) => {
            host.emit_diagnostic(
                crate::E_WARNING,
                &format!(
                    "Object of class {} could not be converted to int",
                    zval_value_name(host, &o)
                ),
            )?;
            Ok(1)
        }
        other => Ok(conv::to_int(&other)),
    }
}

fn type_error(host: &dyn Host, func: &str, n: usize, pname: &str, want: &str, v: &Value) -> Thrown {
    Thrown::type_error(format!(
        "{func}(): Argument #{n} (${pname}) must be of type {want}, {} given",
        zval_value_name(host, v)
    ))
}

/// `Z_PARAM_STR`: scalars coerce, objects go through their string cast
/// (`cast_object`, `__toString`), anything else is a `TypeError`. `i` is
/// 0-based; messages are 1-based.
pub(crate) fn str_param(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    func: &str,
    pname: &str,
) -> Result<Vec<u8>, Thrown> {
    let v = deref(&args[i]);
    match &v {
        Value::Array(_) | Value::Resource(_) | Value::Closure(_) => {
            Err(type_error(host, func, i + 1, pname, "string", &v))
        }
        Value::Object(_) => match crate::object::to_string(host, &v)? {
            Some(s) => Ok(s),
            None => Err(type_error(host, func, i + 1, pname, "string", &v)),
        },
        _ => Ok(conv::to_string_bytes(&v)),
    }
}

/// An `array` parameter. The VM's zpp has checked it for a call from user
/// code; a prelude caller passing anything else gets php's TypeError.
pub(crate) fn array_param(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    func: &str,
) -> Result<PhpArray, Thrown> {
    match args.get(i).map(deref) {
        Some(Value::Array(a)) => Ok(a),
        v => {
            let label = crate::signatures::function(func.as_bytes())
                .and_then(|f| f.params.get(i))
                .filter(|p| !p.variadic())
                .map(|p| format!(" (${})", p.name))
                .unwrap_or_default();
            Err(Thrown::type_error(format!(
                "{func}(): Argument #{}{label} must be of type array, {} given",
                i + 1,
                zval_value_name(host, &v.unwrap_or(Value::Null))
            )))
        }
    }
}

/// `Z_PARAM_PATH`: [`str_param`], plus a `ValueError` for a NUL byte.
pub(crate) fn path_param(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    func: &str,
    pname: &str,
) -> Result<Vec<u8>, Thrown> {
    let s = str_param(host, args, i, func, pname)?;
    if s.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{func}(): Argument #{} (${pname}) must not contain any null bytes",
                i + 1
            ),
        ));
    }
    Ok(s)
}

/// `Z_PARAM_STR_OR_NULL`: absent or `null` → `None`.
pub(crate) fn str_or_null_param(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    func: &str,
    pname: &str,
) -> Result<Option<Vec<u8>>, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => str_param(host, args, i, func, pname).map(Some),
    }
}

/// `Z_PARAM_BOOL` (absent → `default`).
pub(crate) fn bool_param(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    func: &str,
    pname: &str,
    default: bool,
) -> Result<bool, Thrown> {
    match args.get(i).map(deref) {
        None => Ok(default),
        Some(v @ (Value::Array(_) | Value::Object(_) | Value::Resource(_) | Value::Closure(_))) => {
            Err(type_error(host, func, i + 1, pname, "bool", &v))
        }
        Some(v) => Ok(conv::to_bool(&v)),
    }
}

/// `Z_PARAM_LONG_OR_NULL`: absent or `null` → `None`.
pub(crate) fn int_or_null_param(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    func: &str,
    pname: &str,
) -> Result<Option<i64>, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(None),
        Some(v @ (Value::Array(_) | Value::Object(_) | Value::Resource(_) | Value::Closure(_))) => {
            Err(type_error(host, func, i + 1, pname, "?int", &v))
        }
        Some(Value::Str(s)) if conv::scan_number(s.as_bytes()).is_none() => {
            Err(type_error(host, func, i + 1, pname, "?int", &Value::Str(s)))
        }
        Some(v) => Ok(Some(conv::to_int(&v))),
    }
}

/// `Z_PARAM_LONG` (absent → `default`). Non-numeric strings are a TypeError.
pub(crate) fn int_param(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    func: &str,
    pname: &str,
    default: i64,
) -> Result<i64, Thrown> {
    match args.get(i).map(deref) {
        None => Ok(default),
        Some(v @ (Value::Array(_) | Value::Object(_) | Value::Resource(_) | Value::Closure(_))) => {
            Err(type_error(host, func, i + 1, pname, "int", &v))
        }
        Some(Value::Str(s)) => {
            // Leading-numeric strings are accepted (php warns); wholly
            // non-numeric ones are a TypeError.
            if conv::scan_number(s.as_bytes()).is_none() {
                return Err(type_error(host, func, i + 1, pname, "int", &Value::Str(s)));
            }
            Ok(conv::to_int(&Value::Str(s)))
        }
        Some(v) => Ok(conv::to_int(&v)),
    }
}
