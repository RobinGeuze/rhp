//! ext/filter/logical_filters.c: the validating filters. Each one gets the
//! value as a string and leaves the result in it (see [`super::FilterFn`]).

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, Value};

use super::sanitizing_filters::php_filter_url;
use super::*;
use crate::{Host, Thrown};

/// `RETURN_VALIDATION_FAILED`: false, null under FILTER_NULL_ON_FAILURE, or
/// null and FAILURE (the caller throws) under FILTER_THROW_ON_FAILURE.
pub(super) fn failed(value: &mut Value, flags: i64) -> Result<bool, Thrown> {
    if flags & FILTER_THROW_ON_FAILURE != 0 {
        *value = Value::Null;
        return Ok(false);
    }
    *value = if flags & FILTER_NULL_ON_FAILURE != 0 {
        Value::Null
    } else {
        Value::Bool(false)
    };
    Ok(true)
}

/// The option `name` of the options array (`zend_hash_str_find`).
fn option<'a>(options: Option<&'a Value>, name: &[u8]) -> Option<&'a Value> {
    match options {
        Some(Value::Array(a)) => a.get(&ArrayKey::from_bytes(name)),
        _ => None,
    }
}

/// `FETCH_LONG_OPTION`.
fn long_option(
    host: &mut dyn Host,
    options: Option<&Value>,
    name: &[u8],
) -> Result<Option<i64>, Thrown> {
    match option(options, name) {
        Some(v) => crate::zend::zpp::zval_get_long(host, v).map(Some),
        None => Ok(None),
    }
}

/// `FETCH_DOUBLE_OPTION`.
fn double_option(
    host: &mut dyn Host,
    options: Option<&Value>,
    name: &[u8],
) -> Result<Option<f64>, Thrown> {
    let Some(v) = option(options, name) else {
        return Ok(None);
    };
    let v = deref(v);
    let n = crate::zend::object_handlers::object_to_number(
        host,
        &v,
        crate::zend::object_handlers::CastTarget::Float,
    )?;
    Ok(Some(match n {
        Some(n) => conv::to_float(&n),
        None => conv::to_float(&v),
    }))
}

/// `FETCH_STRING_OPTION` / `FETCH_STR_OPTION`: only a string counts.
fn string_option(options: Option<&Value>, name: &[u8]) -> Option<Vec<u8>> {
    match option(options, name).map(deref) {
        Some(Value::Str(s)) => Some(s.as_bytes().to_vec()),
        _ => None,
    }
}

/// The value's bytes (it is a string when a filter runs).
fn bytes(value: &Value) -> Vec<u8> {
    match value {
        Value::Str(s) => s.as_bytes().to_vec(),
        v => conv::to_string_bytes(v),
    }
}

/// `PHP_FILTER_TRIM_DEFAULT_EX`: the bounds of `s` without the white space
/// php trims (no `\f`).
fn trim(s: &[u8]) -> (usize, usize) {
    let ws = |b: u8| matches!(b, b' ' | b'\t' | b'\r' | 0x0b | b'\n');
    let mut start = 0;
    let mut end = s.len();
    while start < end && ws(s[start]) {
        start += 1;
    }
    while end > start && ws(s[end - 1]) {
        end -= 1;
    }
    (start, end)
}

/// `get_active_function_name()` + a ValueError message.
fn value_error(fname: &str, msg: &str) -> Thrown {
    Thrown::error("ValueError", format!("{fname}(): {msg}"))
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_INT
// ---------------------------------------------------------------------------

/// `php_filter_parse_int`: an optional sign, then 1-9 and digits, in range.
fn parse_int(s: &[u8]) -> Option<i64> {
    let mut i = 0;
    let neg = s.first() == Some(&b'-');
    if matches!(s.first(), Some(b'-' | b'+')) {
        i = 1;
    }
    // +0 and -0
    if s.get(i) == Some(&b'0') && i + 1 == s.len() {
        return Some(0);
    }
    let mut v: i64 = match s.get(i) {
        Some(c @ b'1'..=b'9') => {
            i += 1;
            let d = (c - b'0') as i64;
            if neg {
                -d
            } else {
                d
            }
        }
        _ => return None,
    };
    // MAX_LENGTH_OF_LONG - 1
    if s.len() - i > 19 {
        return None;
    }
    while i < s.len() {
        let c = s[i];
        if !c.is_ascii_digit() {
            return None;
        }
        let d = (c - b'0') as i64;
        i += 1;
        if !neg && v <= (i64::MAX - d) / 10 {
            v = v * 10 + d;
        } else if neg && v >= (i64::MIN + d) / 10 {
            v = v * 10 - d;
        } else {
            return None;
        }
    }
    Some(v)
}

/// `php_filter_parse_octal` / `php_filter_parse_hex`: unsigned digits in
/// `radix`, wrapping into a signed long like php's cast.
pub(super) fn parse_radix(s: &[u8], radix: u64) -> Option<i64> {
    let mut v: u64 = 0;
    for &c in s {
        let n = match c {
            b'0'..=b'9' => (c - b'0') as u64,
            b'a'..=b'f' if radix == 16 => (c - b'a' + 10) as u64,
            b'A'..=b'F' if radix == 16 => (c - b'A' + 10) as u64,
            _ => return None,
        };
        if n >= radix {
            return None;
        }
        v = v.checked_mul(radix)?.checked_add(n)?;
    }
    Some(v as i64)
}

pub(super) fn php_filter_int(
    host: &mut dyn Host,
    _fname: &str,
    value: &mut Value,
    flags: i64,
    options: Option<&Value>,
) -> Result<bool, Thrown> {
    let min_range = long_option(host, options, b"min_range")?;
    let max_range = long_option(host, options, b"max_range")?;
    let s = bytes(value);
    if s.is_empty() {
        return failed(value, flags);
    }
    let (start, end) = trim(&s);
    if start >= end {
        return failed(value, flags);
    }
    let p = &s[start..end];
    // Past the trimmed end the C reads white space or the NUL.
    let at = |i: usize| p.get(i).copied().unwrap_or(0);
    let parsed = if p[0] == b'0' {
        let rest = &p[1..];
        if flags & FILTER_FLAG_ALLOW_HEX != 0 && matches!(at(1), b'x' | b'X') {
            if rest.len() == 1 {
                return failed(value, flags);
            }
            parse_radix(&rest[1..], 16)
        } else if flags & FILTER_FLAG_ALLOW_OCTAL != 0 {
            let mut digits = rest;
            if matches!(at(1), b'o' | b'O') {
                if rest.len() == 1 {
                    return failed(value, flags);
                }
                digits = &rest[1..];
            }
            parse_radix(digits, 8)
        } else if !rest.is_empty() {
            None
        } else {
            Some(0)
        }
    } else {
        parse_int(p)
    };
    match parsed {
        Some(n) if !min_range.is_some_and(|m| n < m) && !max_range.is_some_and(|m| n > m) => {
            *value = Value::Int(n);
            Ok(true)
        }
        _ => failed(value, flags),
    }
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_BOOL
// ---------------------------------------------------------------------------

pub(super) fn php_filter_boolean(
    _host: &mut dyn Host,
    _fname: &str,
    value: &mut Value,
    flags: i64,
    _options: Option<&Value>,
) -> Result<bool, Thrown> {
    let s = bytes(value);
    let (start, end) = trim(&s);
    let t = s[start..end].to_ascii_lowercase();
    let ret = match &t[..] {
        b"" | b"0" | b"no" | b"off" | b"false" => Some(false),
        b"1" | b"on" | b"yes" | b"true" => Some(true),
        _ => None,
    };
    match ret {
        Some(b) => {
            *value = Value::Bool(b);
            Ok(true)
        }
        None => failed(value, flags),
    }
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_FLOAT
// ---------------------------------------------------------------------------

pub(super) fn php_filter_float(
    host: &mut dyn Host,
    fname: &str,
    value: &mut Value,
    flags: i64,
    options: Option<&Value>,
) -> Result<bool, Thrown> {
    let s = bytes(value);
    let (start, end) = trim(&s);
    if start >= end {
        return failed(value, flags);
    }
    let mut dec_sep = b'.';
    if let Some(d) = string_option(options, b"decimal") {
        if d.len() != 1 {
            return Err(value_error(
                fname,
                "\"decimal\" option must be one character long",
            ));
        }
        dec_sep = d[0];
    }
    let tsd: Vec<u8> = match string_option(options, b"thousand") {
        Some(t) if t.is_empty() => {
            return Err(value_error(fname, "\"thousand\" option must not be empty"));
        }
        // a C string: it ends at its first NUL
        Some(t) => t.split(|&b| b == 0).next().unwrap_or_default().to_vec(),
        None => b"',.".to_vec(),
    };
    let min_range = double_option(host, options, b"min_range")?;
    let max_range = double_option(host, options, b"max_range")?;

    // The C reads the byte at `end`: trailing white space or the NUL.
    let at = |i: usize| {
        if i < end {
            s[i]
        } else {
            s.get(i).copied().unwrap_or(0)
        }
    };
    let mut i = start;
    let mut num: Vec<u8> = Vec::with_capacity(end - start);
    if i < end && matches!(s[i], b'+' | b'-') {
        num.push(s[i]);
        i += 1;
    }
    let mut first = true;
    let ok = loop {
        let mut n = 0;
        while i < end && s[i].is_ascii_digit() {
            n += 1;
            num.push(s[i]);
            i += 1;
        }
        if i == end || at(i) == dec_sep || at(i) == b'e' || at(i) == b'E' {
            if !first && n != 3 {
                break false;
            }
            if at(i) == dec_sep {
                num.push(b'.');
                i += 1;
                while i < end && s[i].is_ascii_digit() {
                    num.push(s[i]);
                    i += 1;
                }
            }
            if matches!(at(i), b'e' | b'E') {
                num.push(at(i));
                i += 1;
                if i < end && matches!(s[i], b'+' | b'-') {
                    num.push(s[i]);
                    i += 1;
                }
                while i < end && s[i].is_ascii_digit() {
                    num.push(s[i]);
                    i += 1;
                }
            }
            break true;
        }
        // strchr(tsd_sep, c) also finds the NUL terminator
        let c = s[i];
        if flags & FILTER_FLAG_ALLOW_THOUSAND != 0 && (c == 0 || tsd.contains(&c)) {
            if if first { !(1..=3).contains(&n) } else { n != 3 } {
                break false;
            }
            first = false;
            i += 1;
        } else {
            break false;
        }
    };
    if !ok || i != end {
        return failed(value, flags);
    }
    let out_of_range =
        |d: f64| min_range.is_some_and(|m| d < m) || max_range.is_some_and(|m| d > m);
    match conv::numeric_string(&num) {
        Some(conv::Num::Int(l)) => {
            if out_of_range(l as f64) {
                return failed(value, flags);
            }
            *value = Value::Float(l as f64);
            Ok(true)
        }
        Some(conv::Num::Float(d)) => {
            // an underflow to zero, or an overflow
            let underflow =
                d == 0.0 && num.len() > 1 && num.iter().any(|b| (b'1'..=b'9').contains(b));
            if underflow || !d.is_finite() || out_of_range(d) {
                return failed(value, flags);
            }
            *value = Value::Float(d);
            Ok(true)
        }
        None => failed(value, flags),
    }
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_REGEXP
// ---------------------------------------------------------------------------

pub(super) fn php_filter_validate_regexp(
    host: &mut dyn Host,
    fname: &str,
    value: &mut Value,
    flags: i64,
    options: Option<&Value>,
) -> Result<bool, Thrown> {
    let Some(regexp) = string_option(options, b"regexp") else {
        return Err(value_error(fname, "\"regexp\" option is missing"));
    };
    let Some(re) = crate::ext::pcre::php_pcre::compile_warn(host, fname, &regexp)? else {
        return failed(value, flags);
    };
    if matches!(re.find_at(&bytes(value), 0), Ok(Some(_))) {
        Ok(true)
    } else {
        failed(value, flags)
    }
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_DOMAIN
// ---------------------------------------------------------------------------

/// `php_filter_validate_domain_ex`.
pub(super) fn validate_domain_ex(d: &[u8], flags: i64) -> bool {
    let hostname = flags & FILTER_FLAG_HOSTNAME != 0;
    // The C string: past the end is the NUL.
    let at = |i: usize| d.get(i).copied().unwrap_or(0);
    let alnum = |b: u8| b.is_ascii_alphanumeric();
    let mut l = d.len();
    let mut e = d.len();
    // Ignore a trailing dot
    if l > 0 && d[l - 1] == b'.' {
        e -= 1;
        l -= 1;
    }
    // at most 253 characters, the final dot not included
    if l > 253 {
        return false;
    }
    // The first character must be alphanumeric
    if at(0) == b'.' || (hostname && !alnum(at(0))) {
        return false;
    }
    let mut i = 1u32;
    for s in 0..e {
        if d[s] == b'.' {
            // a label's first and last characters must be alphanumeric
            if at(s + 1) == b'.' || (hostname && (!alnum(d[s - 1]) || !alnum(at(s + 1)))) {
                return false;
            }
            i = 1;
        } else {
            if i > 63 || (hostname && (d[s] != b'-' || at(s + 1) == 0) && !alnum(d[s])) {
                return false;
            }
            i += 1;
        }
    }
    true
}

pub(super) fn php_filter_validate_domain(
    _host: &mut dyn Host,
    _fname: &str,
    value: &mut Value,
    flags: i64,
    _options: Option<&Value>,
) -> Result<bool, Thrown> {
    if validate_domain_ex(&bytes(value), flags) {
        Ok(true)
    } else {
        failed(value, flags)
    }
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_URL
// ---------------------------------------------------------------------------

/// `is_userinfo_valid`.
fn is_userinfo_valid(s: &[u8]) -> bool {
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let mut p = 0;
    while p < s.len() {
        let c = s[p];
        if c.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:".contains(&c) {
            p += 1;
        } else if c == b'%'
            && p as isize <= s.len() as isize - 3
            && at(p + 1).is_ascii_digit()
            && at(p + 2).is_ascii_hexdigit()
        {
            p += 3;
        } else {
            return false;
        }
    }
    true
}

/// `php_filter_is_valid_ipv6_hostname`: `[` an IPv6 address `]`.
fn is_valid_ipv6_hostname(s: &[u8]) -> bool {
    s.len() >= 2
        && s[0] == b'['
        && s[s.len() - 1] == b']'
        && validate_ipv6(&s[1..s.len() - 1], None)
}

pub(super) fn php_filter_validate_url(
    host: &mut dyn Host,
    fname: &str,
    value: &mut Value,
    flags: i64,
    options: Option<&Value>,
) -> Result<bool, Thrown> {
    let old_len = bytes(value).len();
    php_filter_url(host, fname, value, flags, options)?;
    let s = match &*value {
        Value::Str(s) if s.len() == old_len => s.as_bytes().to_vec(),
        _ => return failed(value, flags),
    };
    // php_uri_get_parser: only php's parse_url parser is built in.
    if let Some(parser) = string_option(options, b"uri_parser_class") {
        if parser != b"parse_url" {
            return Err(value_error(
                fname,
                "\"uri_parser_class\" option has invalid value",
            ));
        }
    }
    let Some(uri) = crate::ext::standard::url::parse(&s) else {
        return failed(value, flags);
    };
    if let Some(scheme) = &uri.scheme {
        if scheme.eq_ignore_ascii_case(b"http") || scheme.eq_ignore_ascii_case(b"https") {
            let Some(h) = &uri.host else {
                return failed(value, flags);
            };
            // a bracketed IPv6 address, or a host name (a loose IPv4 check)
            if !is_valid_ipv6_hostname(h) && !validate_domain_ex(h, FILTER_FLAG_HOSTNAME) {
                return failed(value, flags);
            }
        }
    }
    let Some(scheme) = &uri.scheme else {
        return failed(value, flags);
    };
    // some schemes allow the host to be empty
    if (uri.host.is_none() && !matches!(&scheme[..], b"mailto" | b"news" | b"file"))
        || (flags & FILTER_FLAG_PATH_REQUIRED != 0 && uri.path.is_none())
        || (flags & FILTER_FLAG_QUERY_REQUIRED != 0 && uri.query.is_none())
    {
        return failed(value, flags);
    }
    if uri.user.as_deref().is_some_and(|u| !is_userinfo_valid(u))
        || uri.pass.as_deref().is_some_and(|p| !is_userinfo_valid(p))
    {
        return failed(value, flags);
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_EMAIL
// ---------------------------------------------------------------------------

// php's own regexes (logical_filters.c), regexp0 for FILTER_FLAG_EMAIL_UNICODE
// and regexp1 otherwise. php's comment on them:
//
//   The regex below is based on a regex by Michael Rushton.
//   However, it is not identical.  I changed it to only consider routeable
//   addresses as valid.  Michael's regex considers a@b a valid address
//   which conflicts with section 2.3.5 of RFC 5321 [...]
//
//   Michael's regex carries this copyright:
//
//   Copyright © Michael Rushton 2009-10
//   http://squiloople.com/
//   Feel free to use and redistribute this code. But please keep this copyright notice.
const EMAIL_REGEXP0: &str = concat!(
    r"/^(?!(?:(?:\x22?\x5C[\x00-\x7E]\x22?)|(?:\x22?[^\x5C\x22]\x22?)){255,})(?!(?:(?:\x22?\x5",
    r"C[\x00-\x7E]\x22?)|(?:\x22?[^\x5C\x22]\x22?)){65,}@)(?:(?:[\x21\x23-\x27\x2A\x2B\x2D\x2F",
    r"-\x39\x3D\x3F\x5E-\x7E\pL\pN]+)|(?:\x22(?:[\x01-\x08\x0B\x0C\x0E-\x1F\x21\x23-\x5B\x5D-\",
    r"x7F\pL\pN]|(?:\x5C[\x00-\x7F]))*\x22))(?:\.(?:(?:[\x21\x23-\x27\x2A\x2B\x2D\x2F-\x39\x3D",
    r"\x3F\x5E-\x7E\pL\pN]+)|(?:\x22(?:[\x01-\x08\x0B\x0C\x0E-\x1F\x21\x23-\x5B\x5D-\x7F\pL\pN",
    r"]|(?:\x5C[\x00-\x7F]))*\x22)))*@(?:(?:(?!.*[^.]{64,})(?:(?:(?:xn--)?[a-z0-9]+(?:-+[a-z0-",
    r"9]+)*\.){1,126}){1,}(?:(?:[a-z][a-z0-9]*)|(?:(?:xn--)[a-z0-9]+))(?:-+[a-z0-9]+)*)|(?:\[(",
    r"?:(?:IPv6:(?:(?:[a-f0-9]{1,4}(?::[a-f0-9]{1,4}){7})|(?:(?!(?:.*[a-f0-9][:\]]){7,})(?:[a-",
    r"f0-9]{1,4}(?::[a-f0-9]{1,4}){0,5})?::(?:[a-f0-9]{1,4}(?::[a-f0-9]{1,4}){0,5})?)))|(?:(?:",
    r"IPv6:(?:(?:[a-f0-9]{1,4}(?::[a-f0-9]{1,4}){5}:)|(?:(?!(?:.*[a-f0-9]:){5,})(?:[a-f0-9]{1,",
    r"4}(?::[a-f0-9]{1,4}){0,3})?::(?:[a-f0-9]{1,4}(?::[a-f0-9]{1,4}){0,3}:)?)))?(?:(?:25[0-5]",
    r")|(?:2[0-4][0-9])|(?:1[0-9]{2})|(?:[1-9]?[0-9]))(?:\.(?:(?:25[0-5])|(?:2[0-4][0-9])|(?:1",
    r"[0-9]{2})|(?:[1-9]?[0-9]))){3}))\]))$/iDu",
);
const EMAIL_REGEXP1: &str = concat!(
    r"/^(?!(?:(?:\x22?\x5C[\x00-\x7E]\x22?)|(?:\x22?[^\x5C\x22]\x22?)){255,})(?!(?:(?:\x22?\x5",
    r"C[\x00-\x7E]\x22?)|(?:\x22?[^\x5C\x22]\x22?)){65,}@)(?:(?:[\x21\x23-\x27\x2A\x2B\x2D\x2F",
    r"-\x39\x3D\x3F\x5E-\x7E]+)|(?:\x22(?:[\x01-\x08\x0B\x0C\x0E-\x1F\x21\x23-\x5B\x5D-\x7F]|(",
    r"?:\x5C[\x00-\x7F]))*\x22))(?:\.(?:(?:[\x21\x23-\x27\x2A\x2B\x2D\x2F-\x39\x3D\x3F\x5E-\x7",
    r"E]+)|(?:\x22(?:[\x01-\x08\x0B\x0C\x0E-\x1F\x21\x23-\x5B\x5D-\x7F]|(?:\x5C[\x00-\x7F]))*\",
    r"x22)))*@(?:(?:(?!.*[^.]{64,})(?:(?:(?:xn--)?[a-z0-9]+(?:-+[a-z0-9]+)*\.){1,126}){1,}(?:(",
    r"?:[a-z][a-z0-9]*)|(?:(?:xn--)[a-z0-9]+))(?:-+[a-z0-9]+)*)|(?:\[(?:(?:IPv6:(?:(?:[a-f0-9]",
    r"{1,4}(?::[a-f0-9]{1,4}){7})|(?:(?!(?:.*[a-f0-9][:\]]){7,})(?:[a-f0-9]{1,4}(?::[a-f0-9]{1",
    r",4}){0,5})?::(?:[a-f0-9]{1,4}(?::[a-f0-9]{1,4}){0,5})?)))|(?:(?:IPv6:(?:(?:[a-f0-9]{1,4}",
    r"(?::[a-f0-9]{1,4}){5}:)|(?:(?!(?:.*[a-f0-9]:){5,})(?:[a-f0-9]{1,4}(?::[a-f0-9]{1,4}){0,3",
    r"})?::(?:[a-f0-9]{1,4}(?::[a-f0-9]{1,4}){0,3}:)?)))?(?:(?:25[0-5])|(?:2[0-4][0-9])|(?:1[0",
    r"-9]{2})|(?:[1-9]?[0-9]))(?:\.(?:(?:25[0-5])|(?:2[0-4][0-9])|(?:1[0-9]{2})|(?:[1-9]?[0-9]",
    r"))){3}))\]))$/iD",
);

pub(super) fn php_filter_validate_email(
    host: &mut dyn Host,
    fname: &str,
    value: &mut Value,
    flags: i64,
    _options: Option<&Value>,
) -> Result<bool, Thrown> {
    let s = bytes(value);
    // at most 320 octets (RFC 2821)
    if s.len() > 320 {
        return failed(value, flags);
    }
    let re = if flags & FILTER_FLAG_EMAIL_UNICODE != 0 {
        EMAIL_REGEXP0
    } else {
        EMAIL_REGEXP1
    };
    if crate::ext::pcre::php_pcre::fixed_pattern_matches(host, fname, re.as_bytes(), &s)? {
        Ok(true)
    } else {
        failed(value, flags)
    }
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_IP
// ---------------------------------------------------------------------------

/// `_php_filter_validate_ipv4`: four decimal octets, no leading zeros.
fn validate_ipv4(s: &[u8], ip: &mut [i32]) -> bool {
    let mut i = 0;
    let mut n = 0;
    while i < s.len() {
        if !s[i].is_ascii_digit() {
            return false;
        }
        let leading_zero = s[i] == b'0';
        let mut m = 1;
        let mut num = (s[i] - b'0') as i32;
        i += 1;
        while i < s.len() && s[i].is_ascii_digit() {
            num = num * 10 + (s[i] - b'0') as i32;
            i += 1;
            m += 1;
            if num > 255 || m > 3 {
                return false;
            }
        }
        // a leading 0 would be octal, which is not supported
        if leading_zero && (num != 0 || m > 1) {
            return false;
        }
        ip[n] = num;
        n += 1;
        if n == 4 {
            return i == s.len();
        }
        if i >= s.len() || s[i] != b'.' {
            return false;
        }
        i += 1;
    }
    false
}

/// `_php_filter_validate_ipv6`; fills `ip` with the eight blocks.
fn validate_ipv6(s: &[u8], mut ip: Option<&mut [i32; 8]>) -> bool {
    if !s.contains(&b':') {
        return false;
    }
    let mut compressed_pos: i32 = -1;
    let mut blocks: i32 = 0;
    let mut ip4 = [0i32; 4];
    let mut end = s.len();
    // a bundled IPv4 address
    let ipv4 = s.iter().position(|&c| c == b'.');
    if let Some(mut p) = ipv4 {
        while p > 0 && s[p - 1] != b':' {
            p -= 1;
        }
        if !validate_ipv4(&s[p..], &mut ip4) {
            return false;
        }
        end = p;
        if end < 2 {
            return false;
        }
        // the ':' before the IPv4 part only counts in a '::'
        if s[p - 2] != b':' {
            end -= 1;
        }
        blocks = 2;
    }
    let mut i = 0;
    'scan: while i < end {
        if s[i] == b':' {
            i += 1;
            if i >= end {
                // cannot end in ':' without a ':' before it
                return false;
            }
            if s[i] == b':' {
                if compressed_pos >= 0 {
                    return false;
                }
                if let Some(ip) = ip.as_deref_mut() {
                    if blocks < 8 {
                        ip[blocks as usize] = -1;
                    }
                }
                // '::' stands for one or more 16-bit zero blocks
                compressed_pos = blocks;
                blocks += 1;
                i += 1;
                if i == end {
                    if blocks > 8 {
                        return false;
                    }
                    break 'scan;
                }
            } else if i - 1 == 0 {
                // a leading ':' needs another ':'
                return false;
            }
        }
        let mut num: u32 = 0;
        let mut n = 0;
        while i < end {
            let d = match s[i] {
                c @ b'0'..=b'9' => c - b'0',
                c @ b'a'..=b'f' => c - b'a' + 10,
                c @ b'A'..=b'F' => c - b'A' + 10,
                _ => break,
            };
            num = num.wrapping_mul(16).wrapping_add(d as u32);
            n += 1;
            i += 1;
        }
        if let Some(ip) = ip.as_deref_mut() {
            if blocks < 8 {
                ip[blocks as usize] = num as i32;
            }
        }
        if !(1..=4).contains(&n) {
            return false;
        }
        blocks += 1;
        if blocks > 8 {
            return false;
        }
    }
    if let Some(ip) = ip {
        if ipv4.is_some() {
            ip[..5].fill(0);
            ip[5] = 0xffff;
            ip[6] = 256 * ip4[0] + ip4[1];
            ip[7] = 256 * ip4[2] + ip4[3];
        } else if compressed_pos >= 0 && blocks <= 8 {
            let offset = 8 - blocks;
            let mut j = 7;
            while j > compressed_pos + offset {
                ip[j as usize] = ip[(j - offset) as usize];
                j -= 1;
            }
            let mut j = compressed_pos + offset;
            while j >= compressed_pos {
                ip[j as usize] = 0;
                j -= 1;
            }
        }
    }
    (compressed_pos >= 0 && blocks <= 8) || blocks == 8
}

/// (global, reserved, private) for a special IPv4 block of RFC 6890.
// One branch per RFC 6890 block, as php lists them.
#[allow(clippy::if_same_then_else)]
fn ipv4_status(ip: &[i32; 8]) -> Option<(bool, bool, bool)> {
    let (g, r, p) = (true, true, true);
    let none = (false, false, false);
    Some(if ip[0] == 0 {
        // RFC 0791 - This network
        (false, r, false)
    } else if ip[0] == 10 {
        // RFC 1918 - Private Use
        (false, false, p)
    } else if ip[0] == 100 && (64..=127).contains(&ip[1]) {
        // RFC 6598 - Shared Address Space
        none
    } else if ip[0] == 127 {
        // RFC 1122 - Loopback
        (false, r, false)
    } else if ip[0] == 169 && ip[1] == 254 {
        // RFC 3927 - Link Local
        (false, r, false)
    } else if ip[0] == 172 && (16..=31).contains(&ip[1]) {
        // RFC 1918 - Private Use
        (false, false, p)
    } else if ip[0] == 192 && ip[1] == 0 && ip[2] == 0 {
        // RFC 6890 - IETF Protocol Assignments
        none
    } else if ip[0] == 192 && ip[1] == 0 && ip[2] == 2 {
        // RFC 5737 - Documentation
        none
    } else if ip[0] == 192 && ip[1] == 88 && ip[2] == 99 {
        // RFC 3068 - 6to4 Relay Anycast
        (g, false, false)
    } else if ip[0] == 192 && ip[1] == 168 {
        // RFC 1918 - Private Use
        (false, false, p)
    } else if ip[0] == 198 && (18..=19).contains(&ip[1]) {
        // RFC 2544 - Benchmarking
        none
    } else if ip[0] == 198 && ip[1] == 51 && ip[2] == 100 {
        // RFC 5737 - Documentation
        none
    } else if ip[0] == 203 && ip[1] == 0 && ip[2] == 113 {
        // RFC 5737 - Documentation
        none
    } else if ip[0] >= 240 && ip[1] <= 255 {
        // RFC 1122 - Reserved (and RFC 0919's limited broadcast)
        (false, r, false)
    } else {
        return None;
    })
}

/// (global, reserved, private) for a special IPv6 block of RFC 6890.
// One branch per RFC 6890 block, as php lists them.
#[allow(clippy::if_same_then_else)]
fn ipv6_status(ip: &[i32; 8]) -> Option<(bool, bool, bool)> {
    let none = (false, false, false);
    let zeros = |n: usize| ip[..n].iter().all(|&b| b == 0);
    Some(if zeros(8) {
        // RFC 4291 - Unspecified Address
        (false, true, false)
    } else if zeros(7) && ip[7] == 1 {
        // RFC 4291 - Loopback Address
        (false, true, false)
    } else if ip[0] == 0x0064 && ip[1] == 0xff9b {
        // RFC 6052 - IPv4-IPv6 Translation
        (true, false, false)
    } else if zeros(5) && ip[5] == 0xffff {
        // RFC 4291 - IPv4-mapped Address
        (false, true, false)
    } else if ip[0] == 0x0100 && zeros_from(ip, 1, 4) {
        // RFC 6666 - Discard-Only Address Block
        none
    } else if ip[0] == 0x2001 && ip[1] <= 0x01ff {
        // RFC 4380 TEREDO, RFC 2928 IETF Protocol Assignments, ...
        none
    } else if ip[0] == 0x2001 && ip[1] == 0x0db8 {
        // RFC 3849 - Documentation
        none
    } else if ip[0] == 0x2002 {
        // RFC 3056 - 6to4
        none
    } else if (0xfc00..=0xfdff).contains(&ip[0]) {
        // RFC 4193 - Unique-Local
        (false, false, true)
    } else if (0xfe80..=0xfebf).contains(&ip[0]) {
        // RFC 4291 - Linked-Scoped Unicast
        (false, true, false)
    } else {
        return None;
    })
}

fn zeros_from(ip: &[i32; 8], from: usize, to: usize) -> bool {
    ip[from..to].iter().all(|&b| b == 0)
}

pub(super) fn php_filter_validate_ip(
    _host: &mut dyn Host,
    _fname: &str,
    value: &mut Value,
    flags: i64,
    _options: Option<&Value>,
) -> Result<bool, Thrown> {
    let s = bytes(value);
    let v6 = if s.contains(&b':') {
        true
    } else if s.contains(&b'.') {
        false
    } else {
        return failed(value, flags);
    };
    let (want4, want6) = (flags & FILTER_FLAG_IPV4 != 0, flags & FILTER_FLAG_IPV6 != 0);
    if !(want4 && want6) && ((want4 && v6) || (want6 && !v6)) {
        return failed(value, flags);
    }
    let mut ip = [0i32; 8];
    let status = if v6 {
        if !validate_ipv6(&s, Some(&mut ip)) {
            return failed(value, flags);
        }
        ipv6_status(&ip)
    } else {
        if !validate_ipv4(&s, &mut ip) {
            return failed(value, flags);
        }
        ipv4_status(&ip)
    };
    // not a special block
    let Some((global, reserved, private)) = status else {
        return Ok(true);
    };
    if (flags & FILTER_FLAG_GLOBAL_RANGE != 0 && !global)
        || (flags & FILTER_FLAG_NO_PRIV_RANGE != 0 && private)
        || (flags & FILTER_FLAG_NO_RES_RANGE != 0 && reserved)
    {
        return failed(value, flags);
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_MAC
// ---------------------------------------------------------------------------

pub(super) fn php_filter_validate_mac(
    _host: &mut dyn Host,
    fname: &str,
    value: &mut Value,
    flags: i64,
    options: Option<&Value>,
) -> Result<bool, Thrown> {
    let s = bytes(value);
    let exp = string_option(options, b"separator");
    if exp.as_ref().is_some_and(|e| e.len() != 1) {
        return Err(value_error(
            fname,
            "\"separator\" option must be one character long",
        ));
    }
    let (tokens, length, separator) = if s.len() == 14 {
        // EUI-64: four hex digits separated by dots
        (3, 4, b'.')
    } else if s.len() == 17 && s[2] == b'-' {
        // IEEE 802: two hex digits separated by hyphens
        (6, 2, b'-')
    } else if s.len() == 17 && s[2] == b':' {
        // IEEE 802: two hex digits separated by colons
        (6, 2, b':')
    } else {
        return failed(value, flags);
    };
    if exp.is_some_and(|e| e[0] != separator) {
        return failed(value, flags);
    }
    for i in 0..tokens {
        let offset = i * (length + 1);
        if i < tokens - 1 && s[offset + length] != separator {
            return failed(value, flags);
        }
        if parse_radix(&s[offset..offset + length], 16).is_none() {
            return failed(value, flags);
        }
    }
    Ok(true)
}
