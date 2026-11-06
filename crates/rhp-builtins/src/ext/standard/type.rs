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
                    matches!(
                        parts[0],
                        Value::Str(_) | Value::Object(_) | Value::Closure(_)
                    ) && matches!(parts[1], Value::Str(_)),
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
                Value::Closure(_) => b"Closure".to_vec(),
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
        return Ok(Value::Int(zend::operators::zval_get_long(host, &v)?));
    }
    let Value::Str(s) = &v else {
        return Ok(Value::Int(zend::operators::zval_get_long(host, &v)?));
    };
    Ok(Value::Int(intval_base(s.as_bytes(), base)))
}

fn c_isspace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// php's `intval($s, $base)` for a string: a "0b" prefix (after
/// whitespace and a sign) for base 0 and 2, else C `strtol`.
fn intval_base(bytes: &[u8], base: i64) -> i64 {
    if base == 0 || base == 2 {
        let start = bytes
            .iter()
            .position(|&c| !c_isspace(c))
            .unwrap_or(bytes.len());
        let s = &bytes[start..];
        if s.len() > 2 {
            let off = usize::from(s[0] == b'-' || s[0] == b'+');
            if s[off] == b'0' && matches!(s.get(off + 1), Some(b'b' | b'B')) {
                let mut tmp = s[..off].to_vec();
                tmp.extend_from_slice(&s[off + 2..]);
                return strtol(&tmp, 2);
            }
        }
    }
    strtol(bytes, base)
}

/// C `strtol`: whitespace, sign, a 0x prefix for base 16 (and 0), octal
/// for a leading 0 in base 0; saturates on overflow; 0 for a bad base.
fn strtol(bytes: &[u8], mut base: i64) -> i64 {
    if base != 0 && !(2..=36).contains(&base) {
        return 0;
    }
    let at = |i: usize| bytes.get(i).copied().unwrap_or(0);
    let mut i = 0;
    while c_isspace(at(i)) {
        i += 1;
    }
    let neg = at(i) == b'-';
    if matches!(at(i), b'-' | b'+') {
        i += 1;
    }
    if (base == 0 || base == 16)
        && at(i) == b'0'
        && matches!(at(i + 1), b'x' | b'X')
        && at(i + 2).is_ascii_hexdigit()
    {
        i += 2;
        base = 16;
    }
    if base == 0 {
        base = if at(i) == b'0' { 8 } else { 10 };
    }
    let mut acc: u64 = 0;
    let mut overflow = false;
    while let Some(d) = (at(i) as char).to_digit(36) {
        if d as i64 >= base {
            break;
        }
        match acc
            .checked_mul(base as u64)
            .and_then(|a| a.checked_add(d as u64))
        {
            Some(a) => acc = a,
            None => overflow = true,
        }
        i += 1;
    }
    if neg {
        if overflow || acc > i64::MIN.unsigned_abs() {
            i64::MIN
        } else {
            (acc as i64).wrapping_neg()
        }
    } else if overflow || acc > i64::MAX as u64 {
        i64::MAX
    } else {
        acc as i64
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

/// An array, or an object whose class is `iface` or implements it.
fn array_or_instance(host: &mut dyn Host, v: &Value, iface: &[u8]) -> bool {
    match deref(v) {
        Value::Array(_) => true,
        o @ Value::Object(_) => host
            .object_class_name(&o)
            .is_some_and(|c| host.reflect_is_a(&c, iface)),
        Value::Closure(_) => false,
        _ => false,
    }
}

/// `is_iterable`: an array or a Traversable.
fn fn_is_iterable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(array_or_instance(
        host,
        &args[0],
        b"Traversable",
    )))
}

/// `is_countable`: an array or a Countable.
fn fn_is_countable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(array_or_instance(host, &args[0], b"Countable")))
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
    // php 8.5: NAN warns when coerced; array and object read the variable
    // again after the warning (a handler may have changed it).
    let is_nan = matches!(cur, Value::Float(f) if f.is_nan());
    let nan_warn = zend::operators::nan_coerced;
    let cur = match ty.as_slice() {
        b"array" | b"object" if is_nan => {
            nan_warn(host, std::str::from_utf8(&ty).unwrap_or(""))?;
            deref(&args[0])
        }
        b"bool" | b"boolean" | b"string" | b"null" if is_nan => {
            let to = match ty.as_slice() {
                b"boolean" => "bool",
                t => std::str::from_utf8(t).unwrap_or(""),
            };
            nan_warn(host, to)?;
            cur.clone()
        }
        _ => cur.clone(),
    };
    let new = match ty.as_slice() {
        b"integer" | b"int" => {
            object_warn(host, "int")?;
            match cur {
                Value::Float(f) => {
                    let n = crate::zend::operators::dval_to_lval(host, f)?;
                    Value::Int(n)
                }
                _ => Value::Int(conv::to_int(&cur)),
            }
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
                        // convert_to_string leaves an empty string behind.
                        if let Value::Ref(rc) = &args[0] {
                            *rc.borrow_mut() = Value::str("");
                        }
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "Object of class {} could not be converted to string",
                                zval_value_name(host, &cur)
                            ),
                        ));
                    }
                }
            }
            _ => Value::str(conv::to_string_bytes(&cur)),
        },
        b"array" => match &cur {
            _ if is_nan => {
                let mut a = PhpArray::new();
                a.push(cur.clone());
                Value::Array(a)
            }
            Value::Array(_) => cur.clone(),
            Value::Null => Value::Array(PhpArray::new()),
            Value::Object(h) => {
                let h = rhp_values::lazy_target(h);
                Value::Array(host.object_props_mangled(&h))
            }
            other => {
                let mut a = PhpArray::new();
                a.push(other.clone());
                Value::Array(a)
            }
        },
        b"object" => match &cur {
            _ if is_nan => host.make_object(vec![(b"scalar".to_vec(), cur.clone())]),
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
        b"bool" | b"boolean" => Value::Bool(is_nan || conv::to_bool(&cur)),
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
