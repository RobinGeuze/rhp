//! Small zpp-style parameter helpers (php's `Z_PARAM_*` coercion rules) for
//! builtins that must raise php's exact `TypeError` text on a wrong-typed
//! argument instead of silently coercing.

use rhp_semantics::deref;
use rhp_values::{conv, PhpArray, Value};

use crate::{Host, Thrown};

/// php's `zend_zval_value_name`: the type word used in "..., X given".
pub(crate) fn value_name(host: &dyn Host, v: &Value) -> String {
    match deref(v) {
        Value::Null => "null".into(),
        Value::Bool(true) => "true".into(),
        Value::Bool(false) => "false".into(),
        Value::Int(_) => "int".into(),
        Value::Float(_) => "float".into(),
        Value::Str(_) => "string".into(),
        Value::Array(_) => "array".into(),
        Value::Closure(_) => "Closure".into(),
        Value::Resource(_) => "resource".into(),
        o @ Value::Object(_) => host
            .object_class_name(&o)
            .map(|n| String::from_utf8_lossy(&n).into_owned())
            .unwrap_or_else(|| "object".into()),
        Value::Ref(_) => unreachable!(),
    }
}

fn type_error(host: &dyn Host, func: &str, n: usize, pname: &str, want: &str, v: &Value) -> Thrown {
    Thrown::type_error(format!(
        "{func}(): Argument #{n} (${pname}) must be of type {want}, {} given",
        value_name(host, v)
    ))
}

/// `Z_PARAM_STR`: scalars coerce, `Stringable` objects call `__toString`,
/// anything else is a `TypeError`. `i` is 0-based; messages are 1-based.
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
        Value::Object(_) => {
            let mut cb = PhpArray::new();
            cb.push(v.clone());
            cb.push(Value::str("__toString"));
            let cb = Value::Array(cb);
            if host.is_callable_value(&cb) {
                Ok(conv::to_string_bytes(&host.call_callable(&cb, &[])?))
            } else {
                Err(type_error(host, func, i + 1, pname, "string", &v))
            }
        }
        _ => Ok(conv::to_string_bytes(&v)),
    }
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
