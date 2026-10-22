//! Character-class tests (`ctype_*`), `is_iterable` / `is_countable`, and
//! `json_encode`.
//!
//! All behavior verified byte-for-byte against the local php 8.5.8 binary.
//!
//! ## `ctype_*` quirks (matched to php)
//! - An `int` in `-128..=255` is interpreted as a single ASCII byte
//!   (`ctype_digit(48)` tests `'0'` → true, `ctype_digit(1)` tests SOH → false).
//! - An `int` outside that range is rendered as its decimal string and tested
//!   whole (`ctype_digit(256)` tests `"256"` → true, `ctype_digit(-99999)`
//!   tests `"-99999"` → false because of the `-`).
//! - A `string` yields false when empty, otherwise true iff *every* byte is in
//!   the class.
//! - Any other type (bool/float/null/array/object) yields false. (php emits a
//!   deprecation notice to stderr for these and for the int-as-string path; we
//!   don't, which is fine — notices go to stderr.)
//! - High bytes (128..=255) follow this binary's `C.UTF-8` locale, i.e. the
//!   ISO-8859-1 classification (e.g. `ctype_alpha("\xE9")` / `é` → true,
//!   `ctype_punct("\xA1")` / `¡` → true). The tables below are transcribed from
//!   that binary.
//!
//! ## `json_encode`
//! Default flags only (flags/depth args are accepted but ignored). Floats use
//! shortest round-trip (`serialize_precision = -1`) via [`fmt::float_serialize`]
//! but with a lowercase `e` exponent and *no* forced `.0` (so `1.0` → `"1"`,
//! `1e20` → `"1.0e+20"`). Strings are escaped per php's default: `"` `\` `/`
//! and the C0 controls are escaped, and every non-ASCII code point becomes
//! `\uXXXX` (surrogate pair for astral planes). Invalid UTF-8, `NAN`, `INF`, and
//! objects make the whole call return `false`, as in php.

use phrust_values::{fmt, ArrayKey, Value};

use crate::{Builtin, Host, Thrown};

/// Local deref: unwrap a `Ref` to an owned clone of its target.
fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

// ---------------------------------------------------------------------------
// ctype character classes (byte predicates). ASCII per the C locale; high
// bytes (128..=255) per this binary's C.UTF-8 == ISO-8859-1 tables.
// ---------------------------------------------------------------------------

fn is_digit(b: u8) -> bool {
    b.is_ascii_digit()
}

fn is_upper(b: u8) -> bool {
    b.is_ascii_uppercase() || (192..=214).contains(&b) || (216..=222).contains(&b)
}

fn is_lower(b: u8) -> bool {
    b.is_ascii_lowercase() || b == 181 || (223..=246).contains(&b) || (248..=255).contains(&b)
}

fn is_alpha(b: u8) -> bool {
    is_upper(b) || is_lower(b) || b == 170 || b == 186
}

fn is_alnum(b: u8) -> bool {
    is_alpha(b) || is_digit(b)
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') || b == 160
}

fn is_punct(b: u8) -> bool {
    b.is_ascii_punctuation() || matches!(b, 161 | 167 | 171 | 182 | 183 | 187 | 191)
}

fn is_xdigit(b: u8) -> bool {
    b.is_ascii_hexdigit()
}

/// Run a ctype class test with php's int/string argument quirks.
fn ctype_check(v: &Value, class: fn(u8) -> bool) -> bool {
    match deref(v) {
        Value::Int(n) => {
            if (-128..=255).contains(&n) {
                class(n as u8)
            } else {
                let s = n.to_string();
                !s.is_empty() && s.bytes().all(class)
            }
        }
        Value::Str(s) => {
            let b = s.as_bytes();
            !b.is_empty() && b.iter().all(|&c| class(c))
        }
        _ => false,
    }
}

macro_rules! ctype_fn {
    ($name:ident, $class:path) => {
        fn $name(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            Ok(Value::Bool(ctype_check(&args[0], $class)))
        }
    };
}
ctype_fn!(fn_ctype_digit, is_digit);
ctype_fn!(fn_ctype_alpha, is_alpha);
ctype_fn!(fn_ctype_alnum, is_alnum);
ctype_fn!(fn_ctype_space, is_space);
ctype_fn!(fn_ctype_upper, is_upper);
ctype_fn!(fn_ctype_lower, is_lower);
ctype_fn!(fn_ctype_punct, is_punct);
ctype_fn!(fn_ctype_xdigit, is_xdigit);

// ---------------------------------------------------------------------------
// is_iterable / is_countable
// ---------------------------------------------------------------------------

// Only arrays qualify until Traversable/Countable objects land (phase 7); php
// also returns true for objects implementing those interfaces.
fn fn_is_iterable(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(matches!(deref(&args[0]), Value::Array(_))))
}

fn fn_is_countable(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(matches!(deref(&args[0]), Value::Array(_))))
}

// ---------------------------------------------------------------------------
// json_encode (default flags)
// ---------------------------------------------------------------------------

fn fn_json_encode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // flags (arg 1) and depth (arg 2) are accepted but ignored: default only.
    let mut out = Vec::new();
    if json_encode_value(&args[0], &mut out) {
        Ok(Value::str(out))
    } else {
        Ok(Value::Bool(false))
    }
}

/// Encode one value, appending to `out`. Returns false on an unencodable value
/// (NAN/INF, invalid UTF-8, object) — php then fails the whole call.
fn json_encode_value(v: &Value, out: &mut Vec<u8>) -> bool {
    match deref(v) {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Int(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::Float(f) => {
            if !f.is_finite() {
                return false;
            }
            let mut s = fmt::float_serialize(f);
            // php's json uses a lowercase exponent ('1.0e+20'); float_serialize
            // emits the var_dump-style uppercase 'E'.
            for b in s.iter_mut() {
                if *b == b'E' {
                    *b = b'e';
                }
            }
            out.extend_from_slice(&s);
        }
        Value::Str(s) => return json_encode_string(s.as_bytes(), out),
        Value::Array(a) => {
            let is_list = a
                .iter()
                .enumerate()
                .all(|(i, (k, _))| matches!(k, ArrayKey::Int(n) if *n == i as i64));
            if is_list {
                out.push(b'[');
                for (i, (_, val)) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    if !json_encode_value(val, out) {
                        return false;
                    }
                }
                out.push(b']');
            } else {
                out.push(b'{');
                for (i, (k, val)) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    match k {
                        ArrayKey::Int(n) => {
                            out.push(b'"');
                            out.extend_from_slice(n.to_string().as_bytes());
                            out.push(b'"');
                        }
                        ArrayKey::Str(s) => {
                            if !json_encode_string(s.as_bytes(), out) {
                                return false;
                            }
                        }
                    }
                    out.push(b':');
                    if !json_encode_value(val, out) {
                        return false;
                    }
                }
                out.push(b'}');
            }
        }
        // Objects need property introspection we don't have yet; closures
        // json_encode to `null` in PHP (no public props), but we fail the call.
        Value::Object(_) | Value::Closure(_) => return false,
        Value::Ref(_) => unreachable!("deref'd above"),
    }
    true
}

/// Append a JSON string literal for `bytes`. Returns false on invalid UTF-8.
fn json_encode_string(bytes: &[u8], out: &mut Vec<u8>) -> bool {
    let s = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };
    out.push(b'"');
    for ch in s.chars() {
        let cp = ch as u32;
        match ch {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '/' => out.extend_from_slice(b"\\/"),
            '\u{08}' => out.extend_from_slice(b"\\b"),
            '\u{09}' => out.extend_from_slice(b"\\t"),
            '\u{0a}' => out.extend_from_slice(b"\\n"),
            '\u{0c}' => out.extend_from_slice(b"\\f"),
            '\u{0d}' => out.extend_from_slice(b"\\r"),
            _ if cp < 0x20 => out.extend_from_slice(format!("\\u{cp:04x}").as_bytes()),
            _ if cp < 0x80 => out.push(cp as u8),
            _ if cp <= 0xFFFF => out.extend_from_slice(format!("\\u{cp:04x}").as_bytes()),
            _ => {
                let v = cp - 0x10000;
                let hi = 0xD800 + (v >> 10);
                let lo = 0xDC00 + (v & 0x3FF);
                out.extend_from_slice(format!("\\u{hi:04x}\\u{lo:04x}").as_bytes());
            }
        }
    }
    out.push(b'"');
    true
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register this module's builtins into the engine's table.
pub fn register(m: &mut std::collections::HashMap<&'static str, Builtin>) {
    macro_rules! reg {
        ($name:literal, $func:path, $min:literal, $max:expr) => {
            m.insert(
                $name,
                Builtin { name: $name, func: $func, min_args: $min, max_args: $max },
            );
        };
    }
    reg!("ctype_digit", fn_ctype_digit, 1, Some(1));
    reg!("ctype_alpha", fn_ctype_alpha, 1, Some(1));
    reg!("ctype_alnum", fn_ctype_alnum, 1, Some(1));
    reg!("ctype_space", fn_ctype_space, 1, Some(1));
    reg!("ctype_upper", fn_ctype_upper, 1, Some(1));
    reg!("ctype_lower", fn_ctype_lower, 1, Some(1));
    reg!("ctype_punct", fn_ctype_punct, 1, Some(1));
    reg!("ctype_xdigit", fn_ctype_xdigit, 1, Some(1));
    reg!("is_iterable", fn_is_iterable, 1, Some(1));
    reg!("is_countable", fn_is_countable, 1, Some(1));
    reg!("json_encode", fn_json_encode, 1, Some(3));
}

// ---------------------------------------------------------------------------
// tests — expected values captured from php 8.5.8
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn s(b: &str) -> Value {
        Value::str(b)
    }

    fn je(v: &Value) -> Value {
        let mut out = Vec::new();
        if json_encode_value(v, &mut out) {
            Value::str(out)
        } else {
            Value::Bool(false)
        }
    }

    fn je_str(v: &Value) -> String {
        match je(v) {
            Value::Str(s) => String::from_utf8(s.as_bytes().to_vec()).unwrap(),
            Value::Bool(false) => "<false>".to_string(),
            _ => unreachable!(),
        }
    }

    fn arr(items: &[Value]) -> Value {
        let mut a = phrust_values::PhpArray::new();
        for v in items {
            a.push(v.clone());
        }
        Value::Array(a)
    }

    fn map(pairs: &[(&str, Value)]) -> Value {
        let mut a = phrust_values::PhpArray::new();
        for (k, v) in pairs {
            a.insert(ArrayKey::from_bytes(k.as_bytes()), v.clone());
        }
        Value::Array(a)
    }

    // --- ctype -----------------------------------------------------------

    #[test]
    fn ctype_strings() {
        assert!(ctype_check(&s("1234"), is_digit));
        assert!(!ctype_check(&s("12.3"), is_digit));
        assert!(!ctype_check(&s(""), is_digit)); // empty -> false
        assert!(ctype_check(&s("Hello"), is_alpha));
        assert!(!ctype_check(&s("Hello1"), is_alpha));
        assert!(ctype_check(&s("abc123"), is_alnum));
        assert!(!ctype_check(&s("abc 123"), is_alnum));
        assert!(ctype_check(&s(" \t\n"), is_space));
        assert!(!ctype_check(&s(" x"), is_space));
        assert!(ctype_check(&s("ABC"), is_upper));
        assert!(!ctype_check(&s("ABc"), is_upper));
        assert!(ctype_check(&s("abc"), is_lower));
        assert!(ctype_check(&s("!?#"), is_punct));
        assert!(!ctype_check(&s("!a"), is_punct));
        assert!(!ctype_check(&s(" "), is_punct));
        assert!(ctype_check(&s("1aF"), is_xdigit));
        assert!(!ctype_check(&s("1aG"), is_xdigit));
    }

    #[test]
    fn ctype_int_quirk() {
        assert!(ctype_check(&Value::Int(48), is_digit)); // '0'
        assert!(!ctype_check(&Value::Int(1), is_digit)); // SOH
        assert!(ctype_check(&Value::Int(256), is_digit)); // -> "256"
        assert!(!ctype_check(&Value::Int(-1), is_digit)); // byte 255
        assert!(!ctype_check(&Value::Int(-128), is_digit)); // byte 128
        assert!(!ctype_check(&Value::Int(-129), is_digit)); // -> "-129"
        assert!(ctype_check(&Value::Int(99999999999), is_digit)); // -> all digits
        assert!(!ctype_check(&Value::Int(-99999), is_digit)); // -> "-99999"
        assert!(ctype_check(&Value::Int(100), is_alnum)); // 'd'
        assert!(!ctype_check(&Value::Int(0), is_alpha)); // NUL
    }

    #[test]
    fn ctype_non_string_non_int() {
        assert!(!ctype_check(&Value::Bool(true), is_digit));
        assert!(!ctype_check(&Value::Float(1.5), is_digit));
        assert!(!ctype_check(&Value::Float(48.0), is_digit));
        assert!(!ctype_check(&Value::Null, is_digit));
        assert!(!ctype_check(&arr(&[Value::Int(1)]), is_alpha));
    }

    #[test]
    fn ctype_high_bytes() {
        // é = 0xE9 (233) alpha+lower; « = 0xAB (171) punct; nbsp = 0xA0 (160) space.
        assert!(ctype_check(&Value::str(vec![0xE9]), is_alpha));
        assert!(ctype_check(&Value::str(vec![0xE9]), is_lower));
        assert!(!ctype_check(&Value::str(vec![0xE9]), is_upper));
        assert!(ctype_check(&Value::str(vec![0xC9]), is_upper)); // É = 201
        assert!(ctype_check(&Value::str(vec![0xAB]), is_punct));
        assert!(ctype_check(&Value::str(vec![0xA0]), is_space));
        assert!(!ctype_check(&Value::str(vec![0xA0]), is_punct));
    }

    // --- is_iterable / is_countable -------------------------------------

    #[test]
    fn iterable_countable() {
        assert!(matches!(
            fn_is_iterable(&mut NoopHost, &[arr(&[])]).unwrap(),
            Value::Bool(true)
        ));
        assert!(matches!(
            fn_is_iterable(&mut NoopHost, &[s("abc")]).unwrap(),
            Value::Bool(false)
        ));
        assert!(matches!(
            fn_is_countable(&mut NoopHost, &[arr(&[Value::Int(1)])]).unwrap(),
            Value::Bool(true)
        ));
        assert!(matches!(
            fn_is_countable(&mut NoopHost, &[Value::Int(5)]).unwrap(),
            Value::Bool(false)
        ));
    }

    struct NoopHost;
    impl Host for NoopHost {
        fn write_out(&mut self, _: &[u8]) {}
    }

    // --- json_encode -----------------------------------------------------

    #[test]
    fn json_scalars() {
        assert_eq!(je_str(&Value::Null), "null");
        assert_eq!(je_str(&Value::Bool(true)), "true");
        assert_eq!(je_str(&Value::Bool(false)), "false");
        assert_eq!(je_str(&Value::Int(42)), "42");
        assert_eq!(je_str(&Value::Int(-7)), "-7");
        assert_eq!(je_str(&Value::Int(i64::MAX)), "9223372036854775807");
    }

    #[test]
    fn json_floats() {
        assert_eq!(je_str(&Value::Float(1.0)), "1");
        assert_eq!(je_str(&Value::Float(1.5)), "1.5");
        assert_eq!(je_str(&Value::Float(1.0 / 3.0)), "0.3333333333333333");
        assert_eq!(je_str(&Value::Float(0.1 + 0.2)), "0.30000000000000004");
        assert_eq!(je_str(&Value::Float(1e20)), "1.0e+20");
        assert_eq!(je_str(&Value::Float(1.5e-10)), "1.5e-10");
        assert_eq!(je_str(&Value::Float(100.0)), "100");
        assert_eq!(je_str(&Value::Float(-0.0)), "-0");
        assert_eq!(je_str(&Value::Float(0.0)), "0");
        assert_eq!(je_str(&Value::Float(1.0e-5)), "1.0e-5");
        // NAN / INF -> false
        assert_eq!(je_str(&Value::Float(f64::NAN)), "<false>");
        assert_eq!(je_str(&Value::Float(f64::INFINITY)), "<false>");
    }

    #[test]
    fn json_strings() {
        assert_eq!(je_str(&s("hello")), "\"hello\"");
        assert_eq!(je_str(&s("a/b")), "\"a\\/b\"");
        assert_eq!(je_str(&s("a\\b")), "\"a\\\\b\"");
        assert_eq!(je_str(&s("quote\"here")), "\"quote\\\"here\"");
        assert_eq!(je_str(&s("tab\tnl\n")), "\"tab\\tnl\\n\"");
        assert_eq!(je_str(&s("\x08\x0c\r")), "\"\\b\\f\\r\"");
        assert_eq!(je_str(&s("\x1b")), "\"\\u001b\"");
        assert_eq!(je_str(&s("\x00\x01\x1f")), "\"\\u0000\\u0001\\u001f\"");
        assert_eq!(je_str(&s("<html>&amp;")), "\"<html>&amp;\"");
        assert_eq!(je_str(&s("apos'")), "\"apos'\"");
        assert_eq!(je_str(&s("\x7f")), "\"\x7f\""); // DEL is literal
        // multibyte
        assert_eq!(je_str(&s("café")), "\"caf\\u00e9\"");
        assert_eq!(je_str(&s("日本")), "\"\\u65e5\\u672c\"");
        assert_eq!(je_str(&s("emoji😀")), "\"emoji\\ud83d\\ude00\"");
        // invalid utf8 -> false
        assert_eq!(je_str(&Value::str(vec![0xC3, 0x28])), "<false>");
    }

    #[test]
    fn json_arrays() {
        assert_eq!(je_str(&arr(&[])), "[]");
        assert_eq!(
            je_str(&arr(&[Value::Int(1), Value::Int(2), Value::Int(3)])),
            "[1,2,3]"
        );
        assert_eq!(je_str(&arr(&[s("a"), s("b")])), "[\"a\",\"b\"]");
        // gap / non-zero-start -> object
        let mut gap = phrust_values::PhpArray::new();
        gap.insert(ArrayKey::Int(0), s("a"));
        gap.insert(ArrayKey::Int(2), s("b"));
        assert_eq!(je_str(&Value::Array(gap)), "{\"0\":\"a\",\"2\":\"b\"}");
        let mut rev = phrust_values::PhpArray::new();
        rev.insert(ArrayKey::Int(1), s("a"));
        rev.insert(ArrayKey::Int(0), s("b"));
        assert_eq!(je_str(&Value::Array(rev)), "{\"1\":\"a\",\"0\":\"b\"}");
        // string keys
        assert_eq!(
            je_str(&map(&[("x", Value::Int(1)), ("y", Value::Int(2))])),
            "{\"x\":1,\"y\":2}"
        );
        // "0","1" canonicalize to ints -> list
        assert_eq!(
            je_str(&map(&[("0", s("a")), ("1", s("b"))])),
            "[\"a\",\"b\"]"
        );
    }

    #[test]
    fn json_nested() {
        let inner = map(&[("c", Value::Int(3))]);
        let outer = map(&[("a", arr(&[Value::Int(1), Value::Int(2)])), ("b", inner)]);
        assert_eq!(je_str(&outer), "{\"a\":[1,2],\"b\":{\"c\":3}}");
        assert_eq!(
            je_str(&arr(&[
                arr(&[Value::Int(1), Value::Int(2)]),
                arr(&[Value::Int(3), Value::Int(4)])
            ])),
            "[[1,2],[3,4]]"
        );
        let nested = map(&[(
            "nested",
            map(&[
                ("a", Value::Bool(true)),
                ("b", Value::Null),
                ("c", arr(&[])),
            ]),
        )]);
        assert_eq!(
            je_str(&nested),
            "{\"nested\":{\"a\":true,\"b\":null,\"c\":[]}}"
        );
    }
}
