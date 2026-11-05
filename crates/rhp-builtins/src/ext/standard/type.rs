//! ext/standard/type.c: `gettype`, `settype`, the `is_*` type checks and
//! the `intval` family.

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::zend::zpp::{array_param, str_param, zval_value_name};
use crate::{zend, Host, Thrown};

fn fn_gettype(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    if host.resource_closed(&v) {
        return Ok(Value::str("resource (closed)"));
    }
    Ok(Value::str(v.type_name()))
}

fn fn_get_debug_type(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = match deref(&args[0]) {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        Value::Closure(_) => "Closure",
        o @ Value::Object(_) => {
            // php: the class name. An anonymous class is named
            // `Prefix@anonymous\0file:line$n`; only the part before the NUL shows.
            let cname = host.object_class_name(&o).unwrap_or_default();
            let end = cname.iter().position(|&b| b == 0).unwrap_or(cname.len());
            return Ok(Value::str(cname[..end].to_vec()));
        }
        ref v @ Value::Resource(ref r) => {
            if host.resource_closed(v) {
                return Ok(Value::str("resource (closed)"));
            }
            return Ok(Value::str(format!("resource ({})", r.type_name)));
        }
        Value::Ref(_) => unreachable!(),
    };
    Ok(Value::str(name))
}

fn fn_is_numeric(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(conv::is_numeric(&args[0])))
}

macro_rules! is_type {
    ($name:ident, $pat:pat) => {
        fn $name(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            Ok(Value::Bool(matches!(deref(&args[0]), $pat)))
        }
    };
}

is_type!(fn_is_int, Value::Int(_));

is_type!(fn_is_float, Value::Float(_));

is_type!(fn_is_string, Value::Str(_));

is_type!(fn_is_bool, Value::Bool(_));

is_type!(fn_is_null, Value::Null);

is_type!(fn_is_array, Value::Array(_));

is_type!(fn_is_object, Value::Object(_) | Value::Closure(_));

fn fn_is_callable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    if let Some(Value::Ref(rc)) = args.get(2) {
        *rc.borrow_mut() = Value::str(callable_name(host, &v));
    }
    // `$syntax_only`: only the callable's shape is checked.
    if args.get(1).is_some_and(|f| conv::to_bool(&deref(f))) {
        match &v {
            Value::Str(_) | Value::Closure(_) => return Ok(Value::Bool(true)),
            Value::Array(a) if a.len() == 2 => {
                let parts: Vec<Value> = a.iter().map(|(_, x)| deref(x)).collect();
                return Ok(Value::Bool(
                    matches!(parts[0], Value::Str(_) | Value::Object(_))
                        && matches!(parts[1], Value::Str(_)),
                ));
            }
            Value::Object(_) => {}
            _ => return Ok(Value::Bool(false)),
        }
    }
    Ok(Value::Bool(host.check_callable(&v)?.is_none()))
}

/// zend_get_callable_name_ex: `is_callable`'s `$callable_name`.
fn callable_name(host: &dyn Host, v: &Value) -> Vec<u8> {
    let class_of = |h: &rhp_values::ObjectHandle| {
        let cls = h.borrow().class;
        host.class_name(cls).unwrap_or_default()
    };
    match v {
        Value::Array(a) if a.len() == 2 => {
            let parts: Vec<Value> = a.iter().map(|(_, x)| deref(x)).collect();
            let method = match &parts[1] {
                Value::Str(m) => m.as_bytes().to_vec(),
                _ => return b"Array".to_vec(),
            };
            let class = match &parts[0] {
                Value::Str(c) => c.as_bytes().to_vec(),
                Value::Object(h) => class_of(h),
                _ => return b"Array".to_vec(),
            };
            [&class[..], b"::", &method[..]].concat()
        }
        Value::Array(_) => b"Array".to_vec(),
        Value::Closure(cd) => match &cd.wrapped {
            // A first-class callable names what it wraps (`strlen`, `A::m`).
            Some(w) => callable_name(host, &deref(w)),
            None => host
                .closure_name(v)
                .unwrap_or_else(|| b"Closure::__invoke".to_vec()),
        },
        Value::Object(h) => [&class_of(h)[..], b"::__invoke"].concat(),
        other => conv::to_string_bytes(other),
    }
}

is_type!(
    fn_is_scalar,
    Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_)
);

fn fn_intval(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    let base = args.get(1).map(|b| conv::to_int(&deref(b))).unwrap_or(10);
    // zval_get_long: the object's cast_object.
    if let Some(n) =
        zend::object_handlers::object_to_number(host, &v, zend::object_handlers::CastTarget::Int)?
    {
        return Ok(Value::Int(conv::to_int(&n)));
    }
    // `base` applies only to a string argument; base 10 is plain integer conversion.
    if base == 10 {
        return Ok(Value::Int(conv::to_int(&v)));
    }
    let Value::Str(s) = &v else {
        return Ok(Value::Int(conv::to_int(&v)));
    };
    Ok(Value::Int(intval_base(s.as_bytes(), base)))
}

/// Parse a string in an explicit base, matching PHP's `intval($s, $base)`:
/// skips leading whitespace, an optional sign, and (for base 0/16) a `0x` / (0/8)
/// `0o`/`0` / (0/2) `0b` prefix; stops at the first invalid digit.
fn intval_base(bytes: &[u8], mut base: i64) -> i64 {
    let mut i = 0;
    while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
        i += 1;
    }
    let neg = match bytes.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    // Base 0: auto-detect from the prefix.
    let rest = &bytes[i..];
    let (digits_start, detected) = detect_prefix(rest, base);
    i += digits_start;
    if base == 0 {
        base = detected;
    }
    if !(2..=36).contains(&base) {
        return 0;
    }
    let mut acc: i64 = 0;
    while i < bytes.len() {
        let d = match bytes[i] {
            b'0'..=b'9' => (bytes[i] - b'0') as i64,
            b'a'..=b'z' => (bytes[i] - b'a' + 10) as i64,
            b'A'..=b'Z' => (bytes[i] - b'A' + 10) as i64,
            _ => break,
        };
        if d >= base {
            break;
        }
        acc = acc.saturating_mul(base).saturating_add(d);
        i += 1;
    }
    if neg {
        -acc
    } else {
        acc
    }
}

/// Returns (bytes-consumed-by-prefix, detected-base) for base 0/16/8/2 prefixes.
fn detect_prefix(rest: &[u8], base: i64) -> (usize, i64) {
    match rest {
        [b'0', b'x' | b'X', ..] if base == 0 || base == 16 => (2, 16),
        [b'0', b'b' | b'B', ..] if base == 0 || base == 2 => (2, 2),
        [b'0', b'o' | b'O', ..] if base == 0 || base == 8 => (2, 8),
        [b'0', ..] if base == 0 => (1, 8),
        _ => (0, if base == 0 { 10 } else { base }),
    }
}

fn fn_floatval(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // zval_get_double: the object's cast_object.
    if let Some(f) = zend::object_handlers::object_to_number(
        host,
        &args[0],
        zend::object_handlers::CastTarget::Float,
    )? {
        return Ok(Value::Float(conv::to_float(&f)));
    }
    Ok(Value::Float(conv::to_float(&args[0])))
}

fn fn_strval(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(zend::zpp::zval_get_string(host, &args[0])?))
}

fn fn_boolval(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(zend::object_handlers::is_true(host, &args[0])?))
}

// Only arrays qualify until Traversable/Countable objects land (phase 7); php
// also returns true for objects implementing those interfaces.
fn fn_is_iterable(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(matches!(deref(&args[0]), Value::Array(_))))
}

fn fn_is_countable(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(matches!(deref(&args[0]), Value::Array(_))))
}

pub(crate) fn fn_array_is_list(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = array_param(host, args, 0, "array_is_list")?;
    let mut expected: i64 = 0;
    for (k, _) in a.iter() {
        match k {
            ArrayKey::Int(n) if *n == expected => expected += 1,
            _ => return Ok(Value::Bool(false)),
        }
    }
    Ok(Value::Bool(true))
}

/// `settype(mixed &$var, string $type)`.
fn fn_settype(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ty = str_param(host, args, 1, "settype", "type")?.to_ascii_lowercase();
    let cur = deref(&args[0]);
    // convert_to_long / convert_to_double: an object warns and becomes 1.
    let object_warn = |host: &mut dyn Host, to: &str| -> Result<(), Thrown> {
        if matches!(cur, Value::Object(_) | Value::Closure(_)) {
            host.emit_diagnostic(
                crate::E_WARNING,
                format!(
                    "Object of class {} could not be converted to {to}",
                    zval_value_name(host, &cur)
                )
                .as_bytes(),
            )?;
        }
        Ok(())
    };
    let new = match ty.as_slice() {
        b"integer" | b"int" => {
            object_warn(host, "int")?;
            Value::Int(conv::to_int(&cur))
        }
        b"float" | b"double" => {
            object_warn(host, "float")?;
            Value::Float(conv::to_float(&cur))
        }
        b"string" => match &cur {
            Value::Array(_) => {
                host.emit_diagnostic(crate::E_WARNING, b"Array to string conversion")?;
                Value::str("Array")
            }
            // convert_to_string: the object's string cast; a failure is an
            // Error, not a parameter TypeError.
            Value::Object(_) | Value::Closure(_) => {
                match crate::zend::object_handlers::to_string(host, &cur)? {
                    Some(s) => Value::str(s),
                    None => {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Object of class {} could not be converted to string",
                                zval_value_name(host, &cur)
                            ),
                        ))
                    }
                }
            }
            _ => Value::str(conv::to_string_bytes(&cur)),
        },
        b"array" => match &cur {
            Value::Array(_) => cur.clone(),
            Value::Null => Value::Array(PhpArray::new()),
            Value::Object(h) => Value::Array(host.object_props_mangled(h)),
            other => {
                let mut a = PhpArray::new();
                a.push(other.clone());
                Value::Array(a)
            }
        },
        b"object" => match &cur {
            Value::Object(_) | Value::Closure(_) => cur.clone(),
            Value::Array(a) => {
                let props = a
                    .iter()
                    .map(|(k, v)| {
                        let name = match k {
                            ArrayKey::Int(n) => n.to_string().into_bytes(),
                            ArrayKey::Str(s) => s.as_bytes().to_vec(),
                        };
                        (name, v.clone())
                    })
                    .collect();
                host.make_object(props)
            }
            Value::Null => host.make_object(Vec::new()),
            other => host.make_object(vec![(b"scalar".to_vec(), other.clone())]),
        },
        b"bool" | b"boolean" => Value::Bool(conv::to_bool(&cur)),
        b"null" => Value::Null,
        b"resource" => {
            return Err(Thrown::error(
                "ValueError",
                "Cannot convert to resource type",
            ));
        }
        _ => {
            return Err(Thrown::error(
                "ValueError",
                "settype(): Argument #2 ($type) must be a valid type",
            ));
        }
    };
    if let Value::Ref(r) = &args[0] {
        *r.borrow_mut() = new;
    }
    Ok(Value::Bool(true))
}

fn fn_is_resource(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    Ok(Value::Bool(
        matches!(v, Value::Resource(_)) && !host.resource_closed(&v),
    ))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("gettype", fn_gettype),
            ("get_debug_type", fn_get_debug_type),
            ("is_numeric", fn_is_numeric),
            ("is_int", fn_is_int),
            ("is_integer", fn_is_int),
            ("is_long", fn_is_int),
            ("is_float", fn_is_float),
            ("is_double", fn_is_float),
            ("is_string", fn_is_string),
            ("is_bool", fn_is_bool),
            ("is_null", fn_is_null),
            ("is_array", fn_is_array),
            ("is_scalar", fn_is_scalar),
            ("is_object", fn_is_object),
            ("is_callable", fn_is_callable),
            ("intval", fn_intval),
            ("floatval", fn_floatval),
            ("doubleval", fn_floatval),
            ("strval", fn_strval),
            ("boolval", fn_boolval),
            ("is_iterable", fn_is_iterable),
            ("is_countable", fn_is_countable),
            ("array_is_list", fn_array_is_list),
            ("settype", fn_settype),
            ("is_resource", fn_is_resource),
        ],
    );
}

#[cfg(test)]
mod tests;
