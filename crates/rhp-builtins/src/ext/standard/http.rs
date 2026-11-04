//! ext/standard/http.c: `http_build_query`.

use rhp_values::ops::deref;
use rhp_values::{conv, fmt, ArrayKey, Value};

use crate::zend::zpp::{int_param, str_or_null_param, str_param, zval_value_name};
use crate::{Host, Thrown};

const HEX: &[u8; 16] = b"0123456789ABCDEF";
const PHP_QUERY_RFC3986: i64 = 2;

fn url_encode_into(out: &mut Vec<u8>, s: &[u8], raw: bool) {
    for &b in s {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(b),
            b'~' if raw => out.push(b),
            b' ' if !raw => out.push(b'+'),
            _ => {
                out.push(b'%');
                out.push(HEX[(b >> 4) as usize]);
                out.push(HEX[(b & 0xf) as usize]);
            }
        }
    }
}

pub(crate) fn url_encode(s: &[u8], raw: bool) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len());
    url_encode_into(&mut v, s, raw);
    v
}

struct Enc<'a> {
    out: Vec<u8>,
    sep: &'a [u8],
    raw: bool,
}

/// Is this value an enum case object?
fn enum_class(host: &dyn Host, v: &Value) -> bool {
    match v {
        Value::Object(h) => host.is_enum_class(h.borrow().class),
        _ => false,
    }
}

/// One hash level of `php_url_encode_hash_ex`. `entries` is the (key, value)
/// list of an array or an object's visible properties.
fn encode_hash(
    host: &mut dyn Host,
    e: &mut Enc,
    entries: Vec<(ArrayKey, Value)>,
    num_prefix: Option<&[u8]>,
    key_prefix: Option<&[u8]>,
    depth: usize,
) -> Result<(), Thrown> {
    if depth > 256 {
        // php guards self-recursive arrays (GC_IS_RECURSIVE); stop descending.
        return Ok(());
    }
    for (key, v) in entries {
        let v = deref(&v);
        let nested = match &v {
            Value::Array(_) => true,
            Value::Object(_) => !enum_class(host, &v),
            _ => false,
        };
        if nested {
            let new_prefix = match &key {
                ArrayKey::Str(k) => {
                    let ek = url_encode(k.as_bytes(), e.raw);
                    match key_prefix {
                        Some(kp) => [kp, &ek, b"%5D%5B"].concat(),
                        None => [&ek[..], b"%5B"].concat(),
                    }
                }
                ArrayKey::Int(i) => {
                    let is = i.to_string().into_bytes();
                    match (key_prefix, num_prefix) {
                        (Some(kp), Some(np)) => [kp, np, &is, b"%5D%5B"].concat(),
                        (Some(kp), None) => [kp, &is, b"%5D%5B"].concat(),
                        (None, Some(np)) => [np, &is, b"%5B"].concat(),
                        (None, None) => [&is[..], b"%5B"].concat(),
                    }
                }
            };
            let sub = entries_of(host, &v);
            encode_hash(host, e, sub, None, Some(&new_prefix), depth + 1)?;
            continue;
        }
        if matches!(v, Value::Null | Value::Resource(_)) {
            continue;
        }
        // php_url_encode_scalar
        let mut part = Vec::new();
        if let Some(kp) = key_prefix {
            part.extend_from_slice(kp);
        }
        match &key {
            ArrayKey::Str(k) => url_encode_into(&mut part, k.as_bytes(), e.raw),
            ArrayKey::Int(i) => {
                if let Some(np) = num_prefix {
                    part.extend_from_slice(np);
                }
                part.extend_from_slice(i.to_string().as_bytes());
            }
        }
        if key_prefix.is_some() {
            part.extend_from_slice(b"%5D");
        }
        part.push(b'=');
        let mut val = v.clone();
        if enum_class(host, &val) {
            match host.object_prop(&val, b"value") {
                Some(bv) => val = bv,
                None => {
                    let cname = zval_value_name(host, &val);
                    return Err(Thrown::error(
                        "ValueError",
                        format!("Unbacked enum {cname} cannot be converted to a string"),
                    ));
                }
            }
        }
        match &val {
            Value::Str(s) => url_encode_into(&mut part, s.as_bytes(), e.raw),
            Value::Int(i) => part.extend_from_slice(i.to_string().as_bytes()),
            Value::Float(f) => url_encode_into(&mut part, &fmt::float_echo(*f), e.raw),
            Value::Bool(b) => part.push(if *b { b'1' } else { b'0' }),
            other => url_encode_into(&mut part, &conv::to_string_bytes(other), e.raw),
        }
        if !e.out.is_empty() {
            e.out.extend_from_slice(e.sep);
        }
        e.out.extend_from_slice(&part);
    }
    Ok(())
}

fn entries_of(host: &dyn Host, v: &Value) -> Vec<(ArrayKey, Value)> {
    match v {
        Value::Array(a) => a.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Value::Object(_) => host
            .object_scoped_props(v)
            .into_iter()
            .map(|(k, v)| (ArrayKey::Str(k), v))
            .collect(),
        _ => Vec::new(),
    }
}

fn fn_http_build_query(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = deref(&args[0]);
    match &data {
        Value::Array(_) => {}
        Value::Object(_) if enum_class(host, &data) => {
            return Err(Thrown::type_error(format!(
                "http_build_query(): Argument #1 ($data) must not be an enum, {} given",
                zval_value_name(host, &data)
            )));
        }
        Value::Object(_) => {}
        other => {
            return Err(Thrown::type_error(format!(
                "http_build_query(): Argument #1 ($data) must be of type array, {} given",
                zval_value_name(host, other)
            )));
        }
    }
    let prefix = match args.get(1) {
        Some(_) => str_param(host, args, 1, "http_build_query", "numeric_prefix")?,
        None => Vec::new(),
    };
    // A null separator falls back to `arg_separator.output` (never empty).
    let sep = str_or_null_param(host, args, 2, "http_build_query", "arg_separator")?
        .unwrap_or_else(|| {
            crate::main::ini::ini_value(b"arg_separator.output")
                .flatten()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| b"&".to_vec())
        });
    let enc = int_param(host, args, 3, "http_build_query", "encoding_type", 1)?;
    let mut e = Enc {
        out: Vec::new(),
        sep: &sep,
        raw: enc == PHP_QUERY_RFC3986,
    };
    let entries = entries_of(host, &data);
    let np = (!prefix.is_empty()).then_some(&prefix[..]);
    encode_hash(host, &mut e, entries, np, None, 0)?;
    Ok(Value::str(e.out))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("http_build_query", fn_http_build_query)]);
}
