//! The `ctype` extension: character-class tests (`ctype_*`).
//!
//! Reimplemented in Rust and verified byte-for-byte against php 8.5.8
//! (`crates/rhp-difftest` sweep `ctype`).
//!
//! ## Quirks (matched to php)
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

use std::collections::HashMap;

use rhp_values::Value;

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

// High-byte behavior per this binary's C.UTF-8 == ISO-8859-1 (byte 173, the
// soft hyphen, classifies as control — not printable — like php).
fn is_cntrl(b: u8) -> bool {
    b <= 31 || (127..=159).contains(&b) || b == 173
}

fn is_graph(b: u8) -> bool {
    (33..=126).contains(&b) || (161..=172).contains(&b) || (174..=255).contains(&b)
}

fn is_print(b: u8) -> bool {
    b == 32 || b == 160 || is_graph(b)
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
ctype_fn!(fn_ctype_cntrl, is_cntrl);
ctype_fn!(fn_ctype_graph, is_graph);
ctype_fn!(fn_ctype_print, is_print);

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    macro_rules! reg {
        ($name:literal, $func:path, $min:literal, $max:expr) => {
            m.insert($name, Builtin { name: $name, func: $func, min_args: $min, max_args: $max });
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
    reg!("ctype_cntrl", fn_ctype_cntrl, 1, Some(1));
    reg!("ctype_graph", fn_ctype_graph, 1, Some(1));
    reg!("ctype_print", fn_ctype_print, 1, Some(1));
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
}
