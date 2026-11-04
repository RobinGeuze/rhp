//! zend_parse_parameters-style argument helpers (Zend/zend_API.c): coerce
//! one positional argument the way php's non-strict internal-function
//! parameter parsing does (`Z_PARAM_*`), raising php's exact `TypeError`
//! text for values that cannot be coerced, plus `zend_zval_type_name` and
//! `zend_zval_value_name`.

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
            .map(|n| String::from_utf8_lossy(&crate::ext::standard::var::c_str(n)).into_owned())
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
            if let Some(s) = crate::zend::object_handlers::to_string(host, &o)? {
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
        Value::Object(_) => match crate::zend::object_handlers::to_string(host, &v)? {
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
            let label = rhp_bytecode::signatures::function(func.as_bytes())
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

pub use rhp_values::ops::deref;

fn type_err(host: &dyn Host, fname: &str, n: usize, pname: &str, want: &str, v: &Value) -> Thrown {
    Thrown::type_error(format!(
        "{fname}(): Argument #{n} (${pname}) must be of type {want}, {} given",
        zval_value_name(host, v)
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
        Value::Object(_) => match crate::zend::object_handlers::to_string(host, &v)? {
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
            zval_value_name(host, v)
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

/// zpp's `f` check: "F(): Argument #N ($callback) must be a valid callback, …".
pub fn require_callback(
    host: &mut dyn Host,
    func: &str,
    n: usize,
    callable: &Value,
) -> Result<(), Thrown> {
    match host.check_callable(callable)? {
        None => Ok(()),
        Some(why) => Err(Thrown::type_error(format!(
            "{func}(): Argument #{n} ($callback) must be a valid callback, {why}"
        ))),
    }
}

pub(crate) fn arg_bytes(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&deref(&args[i]))
}

/// `__zpp_arg($fn, $n, $name, $type, $value)`: coerce one argument of a
/// prelude-implemented internal method the way php's zpp does in coercive
/// mode (`string`, `int`, `?string`, `?int`, `?Throwable`), or raise its
/// TypeError ("Fn(): Argument #n ($name) must be of type T, X given").
fn fn_zpp_arg(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let func = arg_bytes(args, 0);
    let n = conv::to_int(&deref(&args[1]));
    let name = arg_bytes(args, 2);
    let ty = arg_bytes(args, 3);
    let v = deref(&args[4]);
    let (nullable, base) = match ty.strip_prefix(b"?") {
        Some(b) => (true, b.to_vec()),
        None => (false, ty.clone()),
    };
    let coerced: Option<Value> = match (&v, base.as_slice()) {
        (Value::Null, _) if nullable => Some(Value::Null),
        // Coercive mode: null to a scalar internal param is the scalar's zero.
        (Value::Null, b"string") => Some(Value::str("")),
        (Value::Null, b"int") => Some(Value::Int(0)),
        (Value::Str(_), b"string") => Some(v.clone()),
        (Value::Int(_) | Value::Float(_) | Value::Bool(_), b"string") => {
            Some(Value::str(conv::to_string_bytes(&v)))
        }
        (Value::Object(_), b"string") => {
            crate::zend::object_handlers::to_string(host, &v)?.map(Value::str)
        }
        (Value::Int(_), b"int") => Some(v.clone()),
        (Value::Bool(b), b"int") => Some(Value::Int(*b as i64)),
        (Value::Float(f), b"int") if f.is_finite() => Some(Value::Int(*f as i64)),
        (Value::Str(s), b"int") => conv::numeric_string(s.as_bytes()).map(|num| {
            Value::Int(match num {
                conv::Num::Int(i) => i,
                conv::Num::Float(f) => f as i64,
            })
        }),
        (Value::Object(_), b"Throwable") => {
            let cls = host.object_class_name(&v).unwrap_or_default();
            host.reflect_is_a(&cls, b"Throwable").then(|| v.clone())
        }
        _ => None,
    };
    match coerced {
        Some(c) => Ok(c),
        None => {
            let given = zval_value_name(host, &v);
            Err(Thrown::type_error(format!(
                "{}(): Argument #{} (${}) must be of type {}, {} given",
                String::from_utf8_lossy(&func),
                n,
                String::from_utf8_lossy(&name),
                String::from_utf8_lossy(&ty),
                given
            )))
        }
    }
}

/// `Z_PARAM_DOUBLE_OR_NULL` (coercive): `None` for null / absent.
pub(crate) fn zpp_double_or_null(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Option<f64>, Thrown> {
    let v = match args.get(i).map(deref) {
        None | Some(Value::Null) => return Ok(None),
        Some(v) => v,
    };
    let f = match &v {
        Value::Int(n) => Some(*n as f64),
        Value::Float(f) => Some(*f),
        Value::Bool(b) => Some(*b as i64 as f64),
        Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
            Some(conv::Num::Int(n)) => Some(n as f64),
            Some(conv::Num::Float(f)) => Some(f),
            None => None,
        },
        _ => None,
    };
    f.map(Some).ok_or_else(|| {
        Thrown::type_error(format!(
            "{fname}(): Argument #{} (${pname}) must be of type ?float, {} given",
            i + 1,
            zval_value_name(host, &v)
        ))
    })
}

pub fn register(m: &mut crate::Registry) {
    crate::register_helpers(m, &[("__zpp_arg", fn_zpp_arg, 5, Some(5))]);
}
