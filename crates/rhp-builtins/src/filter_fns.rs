//! `filter_var` and the common validators / sanitizers.
//!
//! Every function here is verified byte-for-byte against the local php 8.5.8
//! binary (see the `#[cfg(test)]` block — expected values are hardcoded from
//! `php -r '...'` output).
//!
//! Strings are byte strings. Scalars are converted with the standard PHP
//! string cast before validation/sanitization (matching ext/filter), except
//! that the INT and FLOAT validators short-circuit real int/float inputs.

use std::collections::HashMap;

use rhp_values::{conv, ArrayKey, Value};

use crate::{Builtin, Host, Thrown};

// ---------------------------------------------------------------------------
// filter ids (php 8.5.8 — `php -r 'echo FILTER_VALIDATE_INT;'`)
// ---------------------------------------------------------------------------

const FILTER_VALIDATE_INT: i64 = 257;
const FILTER_VALIDATE_BOOLEAN: i64 = 258;
const FILTER_VALIDATE_FLOAT: i64 = 259;
const FILTER_VALIDATE_DOMAIN: i64 = 277;
const FILTER_VALIDATE_URL: i64 = 273;
const FILTER_VALIDATE_EMAIL: i64 = 274;
const FILTER_VALIDATE_IP: i64 = 275;
const FILTER_VALIDATE_MAC: i64 = 276;
const FILTER_DEFAULT: i64 = 516;
const FILTER_SANITIZE_SPECIAL_CHARS: i64 = 515;
const FILTER_SANITIZE_EMAIL: i64 = 517;
const FILTER_SANITIZE_URL: i64 = 518;
const FILTER_SANITIZE_NUMBER_INT: i64 = 519;

// flags
const FLAG_ALLOW_OCTAL: i64 = 1;
const FLAG_ALLOW_HEX: i64 = 2;
const FLAG_ALLOW_THOUSAND: i64 = 8192;
const FLAG_IPV4: i64 = 1 << 20; // 1048576
const FLAG_IPV6: i64 = 1 << 21; // 2097152
const FLAG_PATH_REQUIRED: i64 = 1 << 18; // 262144
const FLAG_QUERY_REQUIRED: i64 = 1 << 19; // 524288
const FLAG_HOSTNAME: i64 = 1 << 20; // 1048576 (shares a bit with IPV4; context-specific)
const FLAG_EMAIL_UNICODE: i64 = 1 << 20; // the same bit again, for EMAIL
const FLAG_NULL_ON_FAILURE: i64 = 1 << 27; // 134217728

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// Deref a (possibly `Ref`) value to an owned value.
fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

/// php's filter whitespace set for the INT/FLOAT/BOOLEAN validators:
/// space, `\t`, `\n`, `\v`, `\r` — note `\f` (0x0c) is NOT included.
fn is_filter_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | b'\r')
}

/// Strip leading/trailing filter-whitespace.
fn trim_filter_ws(s: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = s.len();
    while start < end && is_filter_ws(s[start]) {
        start += 1;
    }
    while end > start && is_filter_ws(s[end - 1]) {
        end -= 1;
    }
    &s[start..end]
}

// ---------------------------------------------------------------------------
// options
// ---------------------------------------------------------------------------

struct Opts {
    flags: i64,
    min_range: Option<Value>,
    max_range: Option<Value>,
    decimal: Option<u8>,
    default: Option<Value>,
}

fn parse_opts(arg: Option<&Value>) -> Opts {
    let mut o = Opts {
        flags: 0,
        min_range: None,
        max_range: None,
        decimal: None,
        default: None,
    };
    match arg.map(deref) {
        None | Some(Value::Null) => {}
        Some(Value::Array(a)) => {
            if let Some(f) = a.get(&ArrayKey::from_bytes(b"flags")) {
                o.flags = conv::to_int(&deref(f));
            }
            if let Some(opts) = a.get(&ArrayKey::from_bytes(b"options")) {
                if let Value::Array(oa) = deref(opts) {
                    if let Some(v) = oa.get(&ArrayKey::from_bytes(b"min_range")) {
                        o.min_range = Some(deref(v));
                    }
                    if let Some(v) = oa.get(&ArrayKey::from_bytes(b"max_range")) {
                        o.max_range = Some(deref(v));
                    }
                    if let Some(v) = oa.get(&ArrayKey::from_bytes(b"default")) {
                        o.default = Some(deref(v));
                    }
                    if let Some(v) = oa.get(&ArrayKey::from_bytes(b"decimal")) {
                        let d = conv::to_string_bytes(&deref(v));
                        if d.len() == 1 {
                            o.decimal = Some(d[0]);
                        }
                    }
                }
            }
        }
        // a bare scalar third argument is the flags bitmask
        Some(v) => o.flags = conv::to_int(&v),
    }
    o
}

// ---------------------------------------------------------------------------
// filter_var
// ---------------------------------------------------------------------------

fn fn_filter_var(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let value = deref(&args[0]);
    let filter = args
        .get(1)
        .map(|v| conv::to_int(&deref(v)))
        .unwrap_or(FILTER_DEFAULT);
    let opts = parse_opts(args.get(2));

    let result = run_filter(host, &value, filter, &opts)?;
    Ok(match result {
        Some(v) => v,
        None => {
            if let Some(d) = &opts.default {
                d.clone()
            } else if opts.flags & FLAG_NULL_ON_FAILURE != 0 {
                Value::Null
            } else {
                Value::Bool(false)
            }
        }
    })
}

/// `Some(value)` on success, `None` on validation failure (array/object input
/// always fails). The caller maps `None` onto false / null / default.
fn run_filter(
    host: &mut dyn Host,
    value: &Value,
    filter: i64,
    opts: &Opts,
) -> Result<Option<Value>, Thrown> {
    if matches!(value, Value::Array(_) | Value::Object(_)) {
        return Ok(None);
    }
    Ok(match filter {
        FILTER_VALIDATE_INT => validate_int(value, opts),
        FILTER_VALIDATE_FLOAT => validate_float(value, opts),
        FILTER_VALIDATE_BOOLEAN => validate_bool(value),
        FILTER_VALIDATE_EMAIL => validate_email(host, value, opts.flags)?,
        FILTER_VALIDATE_URL => validate_url(value, opts),
        FILTER_VALIDATE_IP => validate_ip(value, opts),
        FILTER_VALIDATE_MAC => validate_mac(value),
        FILTER_VALIDATE_DOMAIN => validate_domain(value, opts),
        FILTER_SANITIZE_NUMBER_INT => Some(Value::str(sanitize_number_int(value))),
        FILTER_SANITIZE_EMAIL => Some(Value::str(sanitize_set(value, &email_sanitize_mask()))),
        FILTER_SANITIZE_URL => Some(Value::str(sanitize_url(value))),
        FILTER_SANITIZE_SPECIAL_CHARS => Some(Value::str(sanitize_special_chars(value))),
        FILTER_DEFAULT => Some(Value::str(conv::to_string_bytes(value))),
        _ => None,
    })
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_MAC
// ---------------------------------------------------------------------------

/// Validate a MAC address: six hex pairs separated uniformly by `:` or `-`
/// (`AA:BB:CC:DD:EE:FF` / `AA-BB-...`), or the Cisco form of three dotted hex
/// quads (`AABB.CCDD.EEFF`). Returns the original string on success.
fn validate_mac(value: &Value) -> Option<Value> {
    let bytes = conv::to_string_bytes(value);
    let s = &bytes[..];
    let hex = |b: u8| b.is_ascii_hexdigit();
    let ok = if s.len() == 17 && (s[2] == b':' || s[2] == b'-') {
        let sep = s[2];
        s.iter()
            .enumerate()
            .all(|(i, &b)| if i % 3 == 2 { b == sep } else { hex(b) })
    } else if s.len() == 14 {
        // Cisco: xxxx.xxxx.xxxx
        s.iter()
            .enumerate()
            .all(|(i, &b)| if i == 4 || i == 9 { b == b'.' } else { hex(b) })
    } else {
        false
    };
    ok.then(|| Value::str(bytes))
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_INT
// ---------------------------------------------------------------------------

fn validate_int(value: &Value, opts: &Opts) -> Option<Value> {
    let n: i64 = match value {
        Value::Int(n) => *n,
        Value::Float(f) => {
            // a real float is accepted only if it is integral and in range
            if f.is_finite() && f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 {
                *f as i64
            } else {
                return None;
            }
        }
        other => {
            let s = conv::to_string_bytes(other);
            parse_filter_int(&s, opts.flags)?
        }
    };
    if let Some(mn) = &opts.min_range {
        if n < conv::to_int(mn) {
            return None;
        }
    }
    if let Some(mx) = &opts.max_range {
        if n > conv::to_int(mx) {
            return None;
        }
    }
    Some(Value::Int(n))
}

/// Parse a php-filter integer string. `None` == invalid.
fn parse_filter_int(s: &[u8], flags: i64) -> Option<i64> {
    let t = trim_filter_ws(s);
    if t.is_empty() {
        return None;
    }
    // hex: requires an explicit 0x / 0X prefix.
    if flags & FLAG_ALLOW_HEX != 0 && t.len() > 2 && t[0] == b'0' && (t[1] == b'x' || t[1] == b'X')
    {
        return parse_radix(&t[2..], 16);
    }
    // octal: a leading 0 (with more digits) selects base 8.
    if flags & FLAG_ALLOW_OCTAL != 0 && t.len() > 1 && t[0] == b'0' {
        return parse_radix(&t[1..], 8);
    }
    parse_decimal_int(t)
}

/// Parse `digits` in the given radix (8 or 16) into an i64, rejecting any
/// invalid digit or overflow. No sign is permitted.
fn parse_radix(digits: &[u8], radix: u32) -> Option<i64> {
    if digits.is_empty() {
        return None;
    }
    let mut val: i128 = 0;
    for &c in digits {
        let d = (c as char).to_digit(radix)?;
        val = val * radix as i128 + d as i128;
        if val > i64::MAX as i128 {
            return None;
        }
    }
    Some(val as i64)
}

/// Strict base-10 parse: optional sign, then `0` or a non-zero-leading digit
/// run, whole string consumed, i64 range enforced.
fn parse_decimal_int(t: &[u8]) -> Option<i64> {
    let (neg, digits) = match t.first() {
        Some(b'+') => (false, &t[1..]),
        Some(b'-') => (true, &t[1..]),
        _ => (false, t),
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    // reject leading zeros (but a lone "0" is fine)
    if digits.len() > 1 && digits[0] == b'0' {
        return None;
    }
    let mut val: i128 = 0;
    for &c in digits {
        val = val * 10 + (c - b'0') as i128;
        if val > (i64::MAX as i128) + 1 {
            return None;
        }
    }
    let signed = if neg { -val } else { val };
    if signed < i64::MIN as i128 || signed > i64::MAX as i128 {
        return None;
    }
    Some(signed as i64)
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_FLOAT
// ---------------------------------------------------------------------------

fn validate_float(value: &Value, opts: &Opts) -> Option<Value> {
    let f: f64 = match value {
        Value::Int(n) => *n as f64,
        Value::Float(f) => *f,
        other => {
            let s = conv::to_string_bytes(other);
            let dec = opts.decimal.unwrap_or(b'.');
            parse_filter_float(&s, dec, opts.flags & FLAG_ALLOW_THOUSAND != 0)?
        }
    };
    if let Some(mn) = &opts.min_range {
        if f < conv::to_float(mn) {
            return None;
        }
    }
    if let Some(mx) = &opts.max_range {
        if f > conv::to_float(mx) {
            return None;
        }
    }
    Some(Value::Float(f))
}

/// Parse a php-filter float string. `decimal` is the decimal-point byte;
/// `thousand` enables grouped integer parts (groups of 3). `None` == invalid.
fn parse_filter_float(s: &[u8], decimal: u8, thousand: bool) -> Option<f64> {
    let t = trim_filter_ws(s);
    if t.is_empty() {
        return None;
    }
    let n = t.len();
    let mut i = 0;
    let mut neg = false;
    match t.first() {
        Some(b'+') => i = 1,
        Some(b'-') => {
            neg = true;
            i = 1;
        }
        _ => {}
    }

    // integer part (optionally comma-grouped)
    let mut int_raw: Vec<u8> = Vec::new();
    let mut had_thousand = false;
    while i < n {
        let c = t[i];
        if c.is_ascii_digit() {
            int_raw.push(c);
            i += 1;
        } else if thousand && c == b',' {
            had_thousand = true;
            int_raw.push(b',');
            i += 1;
        } else {
            break;
        }
    }
    if had_thousand && !valid_thousand_groups(&int_raw) {
        return None;
    }
    let int_digits: Vec<u8> = int_raw.into_iter().filter(|&c| c != b',').collect();

    // fractional part
    let mut frac_digits: Vec<u8> = Vec::new();
    if i < n && t[i] == decimal {
        i += 1;
        while i < n && t[i].is_ascii_digit() {
            frac_digits.push(t[i]);
            i += 1;
        }
    }

    if int_digits.is_empty() && frac_digits.is_empty() {
        return None;
    }

    // build a Rust-parseable mantissa (always a digit before '.')
    let mut norm: Vec<u8> = Vec::new();
    if neg {
        norm.push(b'-');
    }
    if int_digits.is_empty() {
        norm.push(b'0');
    } else {
        norm.extend_from_slice(&int_digits);
    }
    if !frac_digits.is_empty() {
        norm.push(b'.');
        norm.extend_from_slice(&frac_digits);
    }

    // exponent
    if i < n && (t[i] == b'e' || t[i] == b'E') {
        norm.push(b'e');
        i += 1;
        if i < n && (t[i] == b'+' || t[i] == b'-') {
            norm.push(t[i]);
            i += 1;
        }
        let mut any = false;
        while i < n && t[i].is_ascii_digit() {
            norm.push(t[i]);
            any = true;
            i += 1;
        }
        if !any {
            return None;
        }
    }

    if i != n {
        return None; // trailing garbage
    }
    let text = std::str::from_utf8(&norm).ok()?;
    let f: f64 = text.parse().ok()?;
    if f.is_finite() {
        Some(f)
    } else {
        None
    }
}

/// Validate comma grouping: first group 1..=3 digits, each later group exactly
/// 3 digits, none empty.
fn valid_thousand_groups(int_raw: &[u8]) -> bool {
    let groups: Vec<&[u8]> = int_raw.split(|&c| c == b',').collect();
    if groups.len() < 2 {
        return false;
    }
    if groups[0].is_empty() || groups[0].len() > 3 {
        return false;
    }
    groups[1..].iter().all(|g| g.len() == 3)
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_BOOLEAN
// ---------------------------------------------------------------------------

fn validate_bool(value: &Value) -> Option<Value> {
    let s = conv::to_string_bytes(value);
    let mut t = trim_filter_ws(&s).to_vec();
    t.make_ascii_lowercase();
    match t.as_slice() {
        b"1" | b"true" | b"on" | b"yes" => Some(Value::Bool(true)),
        b"0" | b"false" | b"off" | b"no" | b"" => Some(Value::Bool(false)),
        _ => None,
    }
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

fn validate_email(host: &mut dyn Host, value: &Value, flags: i64) -> Result<Option<Value>, Thrown> {
    let s = conv::to_string_bytes(value);
    // RFC 2821's 320-octet maximum, checked before the regex as php does.
    if s.len() > 320 {
        return Ok(None);
    }
    let re = if flags & FLAG_EMAIL_UNICODE != 0 {
        EMAIL_REGEXP0
    } else {
        EMAIL_REGEXP1
    };
    let ok = crate::preg_fns::fixed_pattern_matches(host, "filter_var", re.as_bytes(), &s)?;
    Ok(ok.then(|| Value::str(s)))
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_IP
// ---------------------------------------------------------------------------

fn validate_ip(value: &Value, opts: &Opts) -> Option<Value> {
    let s = conv::to_string_bytes(value);
    let want_v4 = opts.flags & FLAG_IPV4 != 0;
    let want_v6 = opts.flags & FLAG_IPV6 != 0;
    let allow_v4 = want_v4 || !want_v6;
    let allow_v6 = want_v6 || !want_v4;
    if (allow_v4 && valid_ipv4(&s)) || (allow_v6 && valid_ipv6(&s)) {
        Some(Value::str(s))
    } else {
        None
    }
}

/// Dotted-quad IPv4: four octets 0..=255, no leading zeros, nothing else.
fn valid_ipv4(s: &[u8]) -> bool {
    let octets: Vec<&[u8]> = s.split(|&c| c == b'.').collect();
    if octets.len() != 4 {
        return false;
    }
    octets.iter().all(|o| {
        if o.is_empty() || o.len() > 3 || !o.iter().all(u8::is_ascii_digit) {
            return false;
        }
        if o.len() > 1 && o[0] == b'0' {
            return false; // no leading zeros
        }
        let v: u32 = o.iter().fold(0, |a, &c| a * 10 + (c - b'0') as u32);
        v <= 255
    })
}

/// RFC-4291 IPv6, including `::` compression and a trailing embedded IPv4.
fn valid_ipv6(s: &[u8]) -> bool {
    if !s.contains(&b':') {
        return false;
    }
    // embedded IPv4 in the final 32 bits -> rewrite to two hex groups.
    let rebuilt: Vec<u8>;
    let text: &[u8] = if s.contains(&b'.') {
        let colon = match s.iter().rposition(|&c| c == b':') {
            Some(i) => i,
            None => return false,
        };
        let v4 = &s[colon + 1..];
        let mut parts = [0u16; 2];
        if !ipv4_to_groups(v4, &mut parts) {
            return false;
        }
        let mut r = s[..=colon].to_vec();
        r.extend_from_slice(format!("{:x}:{:x}", parts[0], parts[1]).as_bytes());
        rebuilt = r;
        &rebuilt
    } else {
        s
    };
    valid_ipv6_pure(text)
}

/// Validate an IPv4 and pack it into two 16-bit groups (high, low).
fn ipv4_to_groups(s: &[u8], out: &mut [u16; 2]) -> bool {
    if !valid_ipv4(s) {
        return false;
    }
    let mut bytes = [0u8; 4];
    for (i, o) in s.split(|&c| c == b'.').enumerate() {
        bytes[i] = o.iter().fold(0u16, |a, &c| a * 10 + (c - b'0') as u16) as u8;
    }
    out[0] = ((bytes[0] as u16) << 8) | bytes[1] as u16;
    out[1] = ((bytes[2] as u16) << 8) | bytes[3] as u16;
    true
}

/// Pure (no embedded IPv4) IPv6 validation.
fn valid_ipv6_pure(s: &[u8]) -> bool {
    let is_hex_group =
        |g: &[u8]| -> bool { !g.is_empty() && g.len() <= 4 && g.iter().all(u8::is_ascii_hexdigit) };

    // find a single "::"
    let mut dc = None;
    let mut i = 0;
    while i + 1 < s.len() {
        if s[i] == b':' && s[i + 1] == b':' {
            dc = Some(i);
            break;
        }
        i += 1;
    }

    if let Some(pos) = dc {
        let left = &s[..pos];
        let right = &s[pos + 2..];
        // a second "::" anywhere is invalid
        if right.windows(2).any(|w| w == b"::") {
            return false;
        }
        let lg: Vec<&[u8]> = if left.is_empty() {
            Vec::new()
        } else {
            left.split(|&c| c == b':').collect()
        };
        let rg: Vec<&[u8]> = if right.is_empty() {
            Vec::new()
        } else {
            right.split(|&c| c == b':').collect()
        };
        if !lg.iter().all(|g| is_hex_group(g)) || !rg.iter().all(|g| is_hex_group(g)) {
            return false;
        }
        // compression must stand for at least one omitted group
        lg.len() + rg.len() <= 7
    } else {
        let groups: Vec<&[u8]> = s.split(|&c| c == b':').collect();
        groups.len() == 8 && groups.iter().all(|g| is_hex_group(g))
    }
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_DOMAIN
// ---------------------------------------------------------------------------

fn validate_domain(value: &Value, opts: &Opts) -> Option<Value> {
    let s = conv::to_string_bytes(value);
    let hostname = opts.flags & FLAG_HOSTNAME != 0;

    // length excluding a single trailing dot must be <= 253
    let eff_len = if s.last() == Some(&b'.') {
        s.len() - 1
    } else {
        s.len()
    };
    if eff_len > 253 {
        return None;
    }

    let labels: Vec<&[u8]> = s.split(|&c| c == b'.').collect();
    let last = labels.len() - 1;
    for (i, label) in labels.iter().enumerate() {
        let trailing_empty = i == last && label.is_empty();
        if trailing_empty {
            continue; // trailing dot is ignored
        }
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        if hostname && !valid_host_label(label) {
            return None;
        }
    }
    Some(Value::str(s))
}

/// A DNS-style label: alnum, internal hyphens allowed, no leading/trailing
/// hyphen, length 1..=63.
fn valid_host_label(l: &[u8]) -> bool {
    if l.is_empty() || l.len() > 63 {
        return false;
    }
    if l[0] == b'-' || l[l.len() - 1] == b'-' {
        return false;
    }
    l.iter().all(|&b| b.is_ascii_alphanumeric() || b == b'-')
}

// ---------------------------------------------------------------------------
// FILTER_VALIDATE_URL
// ---------------------------------------------------------------------------

struct UrlParts {
    scheme: Option<Vec<u8>>,
    host: Option<Vec<u8>>,
    path_present: bool,
    query_present: bool,
}

fn validate_url(value: &Value, opts: &Opts) -> Option<Value> {
    let s = conv::to_string_bytes(value);
    let parts = parse_url(&s);
    let scheme = parts.scheme.as_ref()?;
    let sl = scheme.to_ascii_lowercase();

    if sl == b"http" || sl == b"https" {
        match &parts.host {
            Some(h) if valid_http_host(h) => {}
            _ => return None,
        }
    }
    if parts.host.is_none() && sl != b"mailto" && sl != b"news" && sl != b"file" {
        return None;
    }
    if opts.flags & FLAG_PATH_REQUIRED != 0 && !parts.path_present {
        return None;
    }
    if opts.flags & FLAG_QUERY_REQUIRED != 0 && !parts.query_present {
        return None;
    }
    Some(Value::str(s))
}

/// http/https host: a bracketed IPv6 literal, or dot-separated DNS labels
/// (alnum + internal hyphens, no empty labels except a single trailing dot).
fn valid_http_host(host: &[u8]) -> bool {
    if host.first() == Some(&b'[') && host.last() == Some(&b']') {
        return valid_ipv6(&host[1..host.len() - 1]);
    }
    if host.is_empty() {
        return false;
    }
    let labels: Vec<&[u8]> = host.split(|&c| c == b'.').collect();
    let last = labels.len() - 1;
    labels
        .iter()
        .enumerate()
        .all(|(i, label)| (i == last && label.is_empty()) || valid_host_label(label))
}

/// A pragmatic parse_url: extracts scheme, host presence, and path/query
/// presence — enough for FILTER_VALIDATE_URL.
fn parse_url(s: &[u8]) -> UrlParts {
    let mut parts = UrlParts {
        scheme: None,
        host: None,
        path_present: false,
        query_present: false,
    };
    let Some(colon) = s.iter().position(|&c| c == b':') else {
        return parts;
    };
    let cand = &s[..colon];
    let valid_scheme = !cand.is_empty()
        && cand
            .iter()
            .all(|&c| c.is_ascii_alphanumeric() || c == b'+' || c == b'-' || c == b'.');
    if !valid_scheme {
        return parts;
    }
    parts.scheme = Some(cand.to_vec());
    let rest = &s[colon + 1..];

    if rest.starts_with(b"//") {
        let after = &rest[2..];
        let end = after
            .iter()
            .position(|&c| c == b'/' || c == b'?' || c == b'#')
            .unwrap_or(after.len());
        let authority = &after[..end];
        let remainder = &after[end..];
        let hostport = match authority.iter().rposition(|&c| c == b'@') {
            Some(p) => &authority[p + 1..],
            None => authority,
        };
        let host = strip_port(hostport);
        parts.host = if host.is_empty() {
            None
        } else {
            Some(host.to_vec())
        };
        parse_path_query(remainder, &mut parts);
    } else {
        // scheme:path form (no authority)
        parse_path_query(rest, &mut parts);
    }
    parts
}

/// Strip a `:port` suffix, honoring `[ipv6]` host literals.
fn strip_port(hostport: &[u8]) -> &[u8] {
    if hostport.first() == Some(&b'[') {
        match hostport.iter().position(|&c| c == b']') {
            Some(close) => &hostport[..=close],
            None => hostport,
        }
    } else {
        match hostport.iter().position(|&c| c == b':') {
            Some(colon) => &hostport[..colon],
            None => hostport,
        }
    }
}

fn parse_path_query(remainder: &[u8], parts: &mut UrlParts) {
    let before_frag = match remainder.iter().position(|&c| c == b'#') {
        Some(h) => &remainder[..h],
        None => remainder,
    };
    match before_frag.iter().position(|&c| c == b'?') {
        Some(q) => {
            parts.path_present = !before_frag[..q].is_empty();
            parts.query_present = true;
        }
        None => parts.path_present = !before_frag.is_empty(),
    }
}

// ---------------------------------------------------------------------------
// sanitizers
// ---------------------------------------------------------------------------

/// FILTER_SANITIZE_NUMBER_INT: keep digits and `+` / `-`.
fn sanitize_number_int(value: &Value) -> Vec<u8> {
    conv::to_string_bytes(value)
        .into_iter()
        .filter(|&b| b.is_ascii_digit() || b == b'+' || b == b'-')
        .collect()
}

/// FILTER_SANITIZE_URL: keep printable non-space ASCII (0x21..=0x7e).
fn sanitize_url(value: &Value) -> Vec<u8> {
    conv::to_string_bytes(value)
        .into_iter()
        .filter(|&b| (0x21..=0x7e).contains(&b))
        .collect()
}

/// Keep only the bytes allowed by `mask`.
fn sanitize_set(value: &Value, mask: &[bool; 256]) -> Vec<u8> {
    conv::to_string_bytes(value)
        .into_iter()
        .filter(|&b| mask[b as usize])
        .collect()
}

/// FILTER_SANITIZE_EMAIL character set.
fn email_sanitize_mask() -> [bool; 256] {
    let mut mask = [false; 256];
    for &b in b"!#$%&'*+-=?^_`{|}~@.[]" {
        mask[b as usize] = true;
    }
    for b in b'0'..=b'9' {
        mask[b as usize] = true;
    }
    for b in b'A'..=b'Z' {
        mask[b as usize] = true;
    }
    for b in b'a'..=b'z' {
        mask[b as usize] = true;
    }
    mask
}

/// FILTER_SANITIZE_SPECIAL_CHARS: encode `"&'<>` and all bytes < 0x20 as
/// `&#NN;`; everything else (including bytes >= 0x7f) is kept verbatim.
fn sanitize_special_chars(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    for b in conv::to_string_bytes(value) {
        if b < 0x20 || matches!(b, b'"' | b'&' | b'\'' | b'<' | b'>') {
            out.extend_from_slice(format!("&#{};", b).as_bytes());
        } else {
            out.push(b);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register the filter builtins into the shared table.
pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
    put(m, "filter_var", fn_filter_var, 1, Some(3));
}

#[cfg(test)]
mod tests;
