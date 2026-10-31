//! The callback-keyed/valued members of the `array_(u)diff` /
//! `array_(u)intersect` family not covered elsewhere:
//! `array_diff_uassoc`, `array_udiff_assoc`, `array_udiff_uassoc`,
//! `array_intersect_ukey`, `array_intersect_uassoc`, `array_uintersect_assoc`,
//! `array_uintersect_uassoc`.
//!
//! php implements the user-comparator variants with a sort-based algorithm;
//! for any consistent comparator the result is the same as this direct
//! membership test: an entry of the first array "matches" another array when
//! that array holds an entry whose key and value both compare equal.

use std::collections::HashMap;

use rhp_semantics::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::argparse::value_name;
use crate::{Builtin, Host, Thrown};

#[derive(Clone, Copy)]
enum Cmp {
    /// Values compared as strings (`(string)$a === (string)$b`).
    StrValue,
    /// Keys must be identical (hash lookup).
    SameKey,
    /// The user callback at this argument index (from the end).
    User(usize),
}

fn key_value(k: &ArrayKey) -> Value {
    match k {
        ArrayKey::Int(n) => Value::Int(*n),
        ArrayKey::Str(s) => Value::Str(s.clone()),
    }
}

fn user_eq(host: &mut dyn Host, cb: &Value, a: Value, b: Value) -> Result<bool, Thrown> {
    Ok(conv::to_int(&host.call_callable(cb, &[a, b])?) == 0)
}

/// Generic `(u)diff/(u)intersect` with separate key and value comparators.
fn set_op(
    host: &mut dyn Host,
    args: &[Value],
    func: &str,
    ncb: usize,
    key_cmp: Cmp,
    val_cmp: Cmp,
    intersect: bool,
) -> Result<Value, Thrown> {
    if args.len() < ncb + 1 {
        return Err(Thrown::error(
            "ArgumentCountError",
            format!(
                "At least {} arguments are required, {} given",
                ncb + 1,
                args.len()
            ),
        ));
    }
    let narr = args.len() - ncb;
    let cbs: Vec<Value> = args[narr..].iter().map(deref).collect();
    for (i, cb) in cbs.iter().enumerate() {
        if !host.is_callable_value(cb) {
            return Err(Thrown::type_error(format!(
                "{func}(): Argument #{} must be a valid callback, {}",
                narr + i + 1,
                match cb {
                    Value::Str(s) => format!(
                        "function \"{}\" not found or invalid function name",
                        String::from_utf8_lossy(s.as_bytes())
                    ),
                    other => format!(
                        "no array or string given ({} given)",
                        value_name(host, other)
                    ),
                }
            )));
        }
    }
    let mut arrays = Vec::with_capacity(narr);
    for (i, a) in args[..narr].iter().enumerate() {
        match deref(a) {
            Value::Array(arr) => arrays.push(arr),
            other => {
                return Err(Thrown::type_error(format!(
                    "{func}(): Argument #{} must be of type array, {} given",
                    i + 1,
                    value_name(host, &other)
                )))
            }
        }
    }
    let cb_of = |c: Cmp| match c {
        Cmp::User(i) => Some(cbs[i].clone()),
        _ => None,
    };
    let kcb = cb_of(key_cmp);
    let vcb = cb_of(val_cmp);
    let base = &arrays[0];
    let mut out = PhpArray::new();
    for (k, v) in base.iter() {
        let mut found_all = true;
        let mut found_any = false;
        for other in &arrays[1..] {
            let mut here = false;
            match key_cmp {
                Cmp::SameKey => {
                    if let Some(ov) = other.get(k) {
                        here = vals_eq(host, val_cmp, &vcb, v, ov)?;
                    }
                }
                _ => {
                    for (ok, ov) in other.iter() {
                        let keq =
                            user_eq(host, kcb.as_ref().unwrap(), key_value(k), key_value(ok))?;
                        if keq && vals_eq(host, val_cmp, &vcb, v, ov)? {
                            here = true;
                            break;
                        }
                    }
                }
            }
            if here {
                found_any = true;
            } else {
                found_all = false;
            }
        }
        let keep = if intersect { found_all } else { !found_any };
        if keep {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Array(out))
}

fn vals_eq(
    host: &mut dyn Host,
    cmp: Cmp,
    cb: &Option<Value>,
    a: &Value,
    b: &Value,
) -> Result<bool, Thrown> {
    match cmp {
        Cmp::StrValue => Ok(conv::to_string_bytes(a) == conv::to_string_bytes(b)),
        Cmp::User(_) => user_eq(host, cb.as_ref().unwrap(), a.clone(), b.clone()),
        Cmp::SameKey => Ok(true),
    }
}

/// "Key only" ops (`array_intersect_ukey`) ignore values entirely.
const NO_VALUE: Cmp = Cmp::SameKey;

fn fn_array_diff_uassoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    set_op(
        host,
        args,
        "array_diff_uassoc",
        1,
        Cmp::User(0),
        Cmp::StrValue,
        false,
    )
}

fn fn_array_udiff_assoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    set_op(
        host,
        args,
        "array_udiff_assoc",
        1,
        Cmp::SameKey,
        Cmp::User(0),
        false,
    )
}

fn fn_array_udiff_uassoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    set_op(
        host,
        args,
        "array_udiff_uassoc",
        2,
        Cmp::User(1),
        Cmp::User(0),
        false,
    )
}

fn fn_array_intersect_ukey(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    set_op(
        host,
        args,
        "array_intersect_ukey",
        1,
        Cmp::User(0),
        NO_VALUE,
        true,
    )
}

fn fn_array_intersect_uassoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    set_op(
        host,
        args,
        "array_intersect_uassoc",
        1,
        Cmp::User(0),
        Cmp::StrValue,
        true,
    )
}

fn fn_array_uintersect_assoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    set_op(
        host,
        args,
        "array_uintersect_assoc",
        1,
        Cmp::SameKey,
        Cmp::User(0),
        true,
    )
}

fn fn_array_uintersect_uassoc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    set_op(
        host,
        args,
        "array_uintersect_uassoc",
        2,
        Cmp::User(1),
        Cmp::User(0),
        true,
    )
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    let mut put = |name: &'static str, func: crate::NativeFn, min: usize| {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: None,
            },
        );
    };
    put("array_diff_uassoc", fn_array_diff_uassoc, 2);
    put("array_udiff_assoc", fn_array_udiff_assoc, 2);
    put("array_udiff_uassoc", fn_array_udiff_uassoc, 3);
    put("array_intersect_ukey", fn_array_intersect_ukey, 2);
    put("array_intersect_uassoc", fn_array_intersect_uassoc, 2);
    put("array_uintersect_assoc", fn_array_uintersect_assoc, 2);
    put("array_uintersect_uassoc", fn_array_uintersect_uassoc, 3);
}
