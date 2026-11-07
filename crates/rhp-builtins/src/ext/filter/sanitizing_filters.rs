//! ext/filter/sanitizing_filters.c: the sanitizing filters.

use rhp_values::{conv, Value};

use super::*;
use crate::{Host, Thrown};

fn bytes(value: &Value) -> Vec<u8> {
    match value {
        Value::Str(s) => s.as_bytes().to_vec(),
        v => conv::to_string_bytes(v),
    }
}

/// `php_filter_encode_html`: the bytes set in `chars` as `&#N;`.
fn encode_html(s: &[u8], chars: &[bool; 256]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &b in s {
        if chars[b as usize] {
            out.extend_from_slice(b"&#");
            out.extend_from_slice(b.to_string().as_bytes());
            out.push(b';');
        } else {
            out.push(b);
        }
    }
    out
}

/// `php_filter_strip`: FILTER_FLAG_STRIP_LOW/HIGH/BACKTICK.
fn strip(s: Vec<u8>, flags: i64) -> Vec<u8> {
    if flags & (FILTER_FLAG_STRIP_LOW | FILTER_FLAG_STRIP_HIGH | FILTER_FLAG_STRIP_BACKTICK) == 0 {
        return s;
    }
    s.into_iter()
        .filter(|&b| {
            !((b >= 127 && flags & FILTER_FLAG_STRIP_HIGH != 0)
                || (b < 32 && flags & FILTER_FLAG_STRIP_LOW != 0)
                || (b == b'`' && flags & FILTER_FLAG_STRIP_BACKTICK != 0))
        })
        .collect()
}

/// The ENCODE_LOW / ENCODE_HIGH / ENCODE_AMP part of an encode set.
fn encode_set(flags: i64) -> [bool; 256] {
    let mut enc = [false; 256];
    if flags & FILTER_FLAG_ENCODE_AMP != 0 {
        enc[b'&' as usize] = true;
    }
    if flags & FILTER_FLAG_ENCODE_LOW != 0 {
        enc[..32].fill(true);
    }
    if flags & FILTER_FLAG_ENCODE_HIGH != 0 {
        enc[127..].fill(true);
    }
    enc
}

/// `filter_map_apply`: keep the bytes in `allowed`.
fn keep(s: &[u8], allowed: &[u8]) -> Vec<u8> {
    let mut map = [false; 256];
    for &b in allowed {
        map[b as usize] = true;
    }
    s.iter().copied().filter(|&b| map[b as usize]).collect()
}

const ALNUM: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// FILTER_SANITIZE_STRING (deprecated).
pub(super) fn php_filter_string(
    _host: &mut dyn Host,
    _fname: &str,
    value: &mut Value,
    flags: i64,
    _options: Option<&Value>,
) -> Result<bool, Thrown> {
    let s = strip(bytes(value), flags);
    let mut enc = encode_set(flags);
    if flags & FILTER_FLAG_NO_ENCODE_QUOTES == 0 {
        enc[b'\'' as usize] = true;
        enc[b'"' as usize] = true;
    }
    let s = encode_html(&s, &enc);
    // strip tags, which also removes NULs
    let s = crate::ext::standard::string::strip_tags_ex(&s, None, true);
    *value = if s.is_empty() && flags & FILTER_FLAG_EMPTY_STRING_NULL != 0 {
        Value::Null
    } else {
        Value::str(s)
    };
    Ok(true)
}

/// FILTER_SANITIZE_ENCODED: urlencode all but alphanumerics and `-._`.
pub(super) fn php_filter_encoded(
    _host: &mut dyn Host,
    _fname: &str,
    value: &mut Value,
    flags: i64,
    _options: Option<&Value>,
) -> Result<bool, Thrown> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let s = strip(bytes(value), flags);
    let mut out = Vec::with_capacity(s.len());
    for b in s {
        if ALNUM.contains(&b) || b"-._".contains(&b) {
            out.push(b);
        } else {
            out.extend_from_slice(&[b'%', HEX[(b >> 4) as usize], HEX[(b & 15) as usize]]);
        }
    }
    *value = Value::str(out);
    Ok(true)
}

/// FILTER_SANITIZE_SPECIAL_CHARS.
pub(super) fn php_filter_special_chars(
    _host: &mut dyn Host,
    _fname: &str,
    value: &mut Value,
    flags: i64,
    _options: Option<&Value>,
) -> Result<bool, Thrown> {
    let s = strip(bytes(value), flags);
    let mut enc = [false; 256];
    for b in *b"'\"<>&" {
        enc[b as usize] = true;
    }
    enc[..32].fill(true);
    if flags & FILTER_FLAG_ENCODE_HIGH != 0 {
        enc[127..].fill(true);
    }
    *value = Value::str(encode_html(&s, &enc));
    Ok(true)
}

/// FILTER_SANITIZE_FULL_SPECIAL_CHARS: htmlentities without double
/// encoding, in the default charset.
pub(super) fn php_filter_full_special_chars(
    host: &mut dyn Host,
    fname: &str,
    value: &mut Value,
    flags: i64,
    _options: Option<&Value>,
) -> Result<bool, Thrown> {
    let quotes = flags & FILTER_FLAG_NO_ENCODE_QUOTES == 0;
    let s = bytes(value);
    let out = crate::ext::standard::html::escape_entities(host, fname, &s, quotes)?;
    *value = Value::str(out);
    Ok(true)
}

/// FILTER_UNSAFE_RAW (FILTER_DEFAULT).
pub(super) fn php_filter_unsafe_raw(
    _host: &mut dyn Host,
    _fname: &str,
    value: &mut Value,
    flags: i64,
    _options: Option<&Value>,
) -> Result<bool, Thrown> {
    let s = bytes(value);
    // Only when flags are set
    if flags != 0 && !s.is_empty() {
        let s = strip(s, flags);
        *value = Value::str(encode_html(&s, &encode_set(flags)));
    } else if flags & FILTER_FLAG_EMPTY_STRING_NULL != 0 && s.is_empty() {
        *value = Value::Null;
    }
    Ok(true)
}

/// FILTER_SANITIZE_EMAIL: the characters of RFC 822 section 6.
pub(super) fn php_filter_email(
    _host: &mut dyn Host,
    _fname: &str,
    value: &mut Value,
    _flags: i64,
    _options: Option<&Value>,
) -> Result<bool, Thrown> {
    let allowed = [ALNUM, b"!#$%&'*+-=?^_`{|}~@.[]"].concat();
    *value = Value::str(keep(&bytes(value), &allowed));
    Ok(true)
}

/// FILTER_SANITIZE_URL: the characters of RFC 1738 section 5.
pub(super) fn php_filter_url(
    _host: &mut dyn Host,
    _fname: &str,
    value: &mut Value,
    _flags: i64,
    _options: Option<&Value>,
) -> Result<bool, Thrown> {
    // SAFE EXTRA NATIONAL PUNCTUATION RESERVED
    let allowed = [
        ALNUM,
        b"$-_.+",
        b"!*'(),",
        b"{}|\\^~[]`",
        b"<>#%\"",
        b";/?:@&=",
    ]
    .concat();
    *value = Value::str(keep(&bytes(value), &allowed));
    Ok(true)
}

/// FILTER_SANITIZE_NUMBER_INT.
pub(super) fn php_filter_number_int(
    _host: &mut dyn Host,
    _fname: &str,
    value: &mut Value,
    _flags: i64,
    _options: Option<&Value>,
) -> Result<bool, Thrown> {
    *value = Value::str(keep(&bytes(value), b"+-0123456789"));
    Ok(true)
}

/// FILTER_SANITIZE_NUMBER_FLOAT.
pub(super) fn php_filter_number_float(
    _host: &mut dyn Host,
    _fname: &str,
    value: &mut Value,
    flags: i64,
    _options: Option<&Value>,
) -> Result<bool, Thrown> {
    let mut allowed = b"+-0123456789".to_vec();
    if flags & FILTER_FLAG_ALLOW_FRACTION != 0 {
        allowed.push(b'.');
    }
    if flags & FILTER_FLAG_ALLOW_THOUSAND != 0 {
        allowed.push(b',');
    }
    if flags & FILTER_FLAG_ALLOW_SCIENTIFIC != 0 {
        allowed.extend_from_slice(b"eE");
    }
    *value = Value::str(keep(&bytes(value), &allowed));
    Ok(true)
}

/// FILTER_SANITIZE_ADD_SLASHES: `php_addslashes`.
pub(super) fn php_filter_add_slashes(
    _host: &mut dyn Host,
    _fname: &str,
    value: &mut Value,
    _flags: i64,
    _options: Option<&Value>,
) -> Result<bool, Thrown> {
    let s = bytes(value);
    let mut out = Vec::with_capacity(s.len());
    for b in s {
        match b {
            b'\'' | b'"' | b'\\' => out.extend_from_slice(&[b'\\', b]),
            0 => out.extend_from_slice(b"\\0"),
            b => out.push(b),
        }
    }
    *value = Value::str(out);
    Ok(true)
}

/// callback_filter.c: FILTER_CALLBACK calls the `options` callable with
/// the string.
pub(super) fn php_filter_callback(
    host: &mut dyn Host,
    fname: &str,
    value: &mut Value,
    _flags: i64,
    options: Option<&Value>,
) -> Result<bool, Thrown> {
    let cb = match options {
        Some(cb) if host.is_callable_value(cb) => cb.clone(),
        _ => {
            *value = Value::Null;
            return Err(Thrown::type_error(format!(
                "{fname}(): Option must be a valid callback"
            )));
        }
    };
    let arg = std::mem::replace(value, Value::Null);
    *value = host.call_callable(&cb, &[arg])?;
    Ok(true)
}
