//! Native URL and string-escaping builtins (urlencode, nl2br, addslashes,
//! strip_tags, parse_url, ...). The HTML entity functions are in `html`.
//!
//! The unit tests' expected values were captured from php.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::{Host, Thrown};

// ---------------------------------------------------------------------------
// small shared helpers
// ---------------------------------------------------------------------------

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

// ---------------------------------------------------------------------------
// urlencode / rawurlencode / urldecode / rawurldecode
// ---------------------------------------------------------------------------

fn fn_urlencode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let mut out = Vec::with_capacity(s.len());
    for &b in &s {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(b),
            b' ' => out.push(b'+'),
            _ => {
                out.push(b'%');
                out.push(HEX_UPPER[(b >> 4) as usize]);
                out.push(HEX_UPPER[(b & 0xf) as usize]);
            }
        }
    }
    Ok(Value::str(out))
}

fn fn_rawurlencode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let mut out = Vec::with_capacity(s.len());
    for &b in &s {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b),
            _ => {
                out.push(b'%');
                out.push(HEX_UPPER[(b >> 4) as usize]);
                out.push(HEX_UPPER[(b & 0xf) as usize]);
            }
        }
    }
    Ok(Value::str(out))
}

fn fn_urldecode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(crate::main::php_variables::url_decode(
        &conv::to_string_bytes(&args[0]),
    )))
}

fn fn_rawurldecode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(crate::main::php_variables::raw_url_decode(
        &conv::to_string_bytes(&args[0]),
    )))
}

// ---------------------------------------------------------------------------
// parse_url
// ---------------------------------------------------------------------------

/// php `parse_url($url, $component = -1)`.
fn fn_parse_url(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let url = conv::to_string_bytes(&args[0]);
    let component = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(-1);
    let Some(u) = crate::url::parse(&url) else {
        return Ok(Value::Bool(false));
    };
    let port = u.has_port.then_some(u.port as i64);
    if component > -1 {
        let part = match component {
            0 => u.scheme,
            1 => u.host,
            2 => return Ok(port.map_or(Value::Null, Value::Int)),
            3 => u.user,
            4 => u.pass,
            5 => u.path,
            6 => u.query,
            7 => u.fragment,
            _ => {
                return Err(Thrown::error(
                    "ValueError",
                    format!("parse_url(): Argument #2 ($component) must be a valid URL component identifier, {component} given"),
                ))
            }
        };
        return Ok(part.map_or(Value::Null, Value::str));
    }
    let mut arr = rhp_values::PhpArray::new();
    let mut put = |k: &[u8], v: Option<Value>| {
        if let Some(v) = v {
            arr.insert(rhp_values::ArrayKey::from_bytes(k), v);
        }
    };
    put(b"scheme", u.scheme.map(Value::str));
    put(b"host", u.host.map(Value::str));
    put(b"port", port.map(Value::Int));
    put(b"user", u.user.map(Value::str));
    put(b"pass", u.pass.map(Value::str));
    put(b"path", u.path.map(Value::str));
    put(b"query", u.query.map(Value::str));
    put(b"fragment", u.fragment.map(Value::str));
    Ok(Value::Array(arr))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register all URL/HTML encoding builtins into the shared table.
pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("urlencode", fn_urlencode),
            ("urldecode", fn_urldecode),
            ("rawurlencode", fn_rawurlencode),
            ("rawurldecode", fn_rawurldecode),
            ("parse_url", fn_parse_url),
        ],
    );
}

#[cfg(test)]
mod tests;
