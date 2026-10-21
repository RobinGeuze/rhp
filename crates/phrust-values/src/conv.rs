//! Pure PHP value conversions and numeric-string parsing.
//!
//! These are infallible, AST-free, and depend only on `Value` + `fmt`, so both
//! the VM / oracle (echo, operators, casts) and `phrust-builtins`
//! (`intval`/`strval`/`is_numeric`/…) can share them. Conversions that can raise
//! a `Throwable` (e.g. arithmetic on arrays) live in `phrust-semantics`, which can
//! depend on the error type.
//!
//! All rules verified against php 8.5.8 (see `docs/design/php-semantics.md`).

use crate::fmt;
use crate::Value;

/// A PHP numeric: the result of interpreting a value/string as a number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    pub fn to_value(self) -> Value {
        match self {
            Num::Int(n) => Value::Int(n),
            Num::Float(f) => Value::Float(f),
        }
    }
    pub fn as_f64(self) -> f64 {
        match self {
            Num::Int(n) => n as f64,
            Num::Float(f) => f,
        }
    }
}

const WS: &[u8] = b" \t\n\r\x0b\x0c";

fn is_ws(b: u8) -> bool {
    WS.contains(&b)
}

/// `(bool)` / truthiness.
pub fn to_bool(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Int(n) => *n != 0,
        Value::Float(f) => *f != 0.0, // NaN is truthy; -0.0 is falsy
        Value::Str(s) => {
            let b = s.as_bytes();
            !(b.is_empty() || b == b"0")
        }
        Value::Array(a) => !a.is_empty(),
        Value::Object(_) => true,
        Value::Ref(r) => to_bool(&r.borrow()),
    }
}

/// `echo` / `.` / `(string)` / interpolation form (precision 14).
///
/// Object `__toString` is not handled here (no interpreter access); objects are
/// a phase-7 concern and render as a placeholder until then.
pub fn to_string_bytes(v: &Value) -> Vec<u8> {
    match v {
        Value::Null => Vec::new(),
        Value::Bool(true) => b"1".to_vec(),
        Value::Bool(false) => Vec::new(),
        Value::Int(n) => n.to_string().into_bytes(),
        Value::Float(f) => fmt::float_echo(*f),
        Value::Str(s) => s.as_bytes().to_vec(),
        Value::Array(_) => b"Array".to_vec(), // PHP also emits an E_WARNING
        Value::Object(_) => b"Object".to_vec(),
        Value::Ref(r) => to_string_bytes(&r.borrow()),
    }
}

/// `(int)` cast.
pub fn to_int(v: &Value) -> i64 {
    match v {
        Value::Null => 0,
        Value::Bool(b) => *b as i64,
        Value::Int(n) => *n,
        Value::Float(f) => crate::float_to_int(*f),
        Value::Str(s) => match scan_number(s.as_bytes()) {
            Some((Num::Int(n), _)) => n,
            Some((Num::Float(f), _)) => crate::float_to_int(f),
            None => 0,
        },
        Value::Array(a) => !a.is_empty() as i64,
        Value::Object(_) => 1,
        Value::Ref(r) => to_int(&r.borrow()),
    }
}

/// `(float)` cast.
pub fn to_float(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::Bool(b) => *b as i64 as f64,
        Value::Int(n) => *n as f64,
        Value::Float(f) => *f,
        Value::Str(s) => match scan_number(s.as_bytes()) {
            Some((num, _)) => num.as_f64(),
            None => 0.0,
        },
        Value::Array(a) => !a.is_empty() as i64 as f64,
        Value::Object(_) => 1.0,
        Value::Ref(r) => to_float(&r.borrow()),
    }
}

/// Interpret a scalar as a number for arithmetic. Strings use their leading
/// numeric prefix (PHP's "non well formed numeric string" behavior). Arrays and
/// objects are rejected by the caller before this is reached.
///
/// Returns `(num, well_formed)`: `well_formed` is false when a string had
/// trailing non-numeric bytes (PHP would emit a warning) or was non-numeric.
pub fn to_number(v: &Value) -> (Num, bool) {
    match v {
        Value::Null => (Num::Int(0), true),
        Value::Bool(b) => (Num::Int(*b as i64), true),
        Value::Int(n) => (Num::Int(*n), true),
        Value::Float(f) => (Num::Float(*f), true),
        Value::Str(s) => {
            let bytes = s.as_bytes();
            match scan_number(bytes) {
                Some((num, end)) => (num, bytes[end..].iter().all(|&b| is_ws(b))),
                None => (Num::Int(0), false),
            }
        }
        Value::Ref(r) => to_number(&r.borrow()),
        Value::Array(_) | Value::Object(_) => (Num::Int(0), false),
    }
}

/// Strict numeric-string test (`is_numeric`): the whole string, after trimming
/// surrounding whitespace, is a valid int or float. Leading AND trailing
/// whitespace are allowed (PHP 8). Hex/binary/octal prefixes are NOT numeric.
pub fn numeric_string(s: &[u8]) -> Option<Num> {
    let (num, end) = scan_number(s)?;
    if s[end..].iter().all(|&b| is_ws(b)) {
        Some(num)
    } else {
        None
    }
}

/// Does this value count as numeric for `is_numeric()`? (int/float always;
/// strings per [`numeric_string`]; everything else no.)
pub fn is_numeric(v: &Value) -> bool {
    match v {
        Value::Int(_) | Value::Float(_) => true,
        Value::Str(s) => numeric_string(s.as_bytes()).is_some(),
        Value::Ref(r) => is_numeric(&r.borrow()),
        _ => false,
    }
}

/// Scan a leading number from `s` (after skipping leading ASCII whitespace).
/// Returns the parsed [`Num`] and the byte index immediately after the number
/// (so the caller can check for trailing garbage). `None` if no number is found.
///
/// Integer forms that overflow `i64` become `Num::Float`, matching PHP.
pub fn scan_number(s: &[u8]) -> Option<(Num, usize)> {
    let mut i = 0;
    while i < s.len() && is_ws(s[i]) {
        i += 1;
    }
    let num_start = i;

    // optional sign
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        i += 1;
    }

    let int_digits_start = i;
    while i < s.len() && s[i].is_ascii_digit() {
        i += 1;
    }
    let has_int_digits = i > int_digits_start;

    let mut is_float = false;
    // fractional part
    if i < s.len() && s[i] == b'.' {
        is_float = true;
        i += 1;
        let frac_start = i;
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
        }
        let has_frac_digits = i > frac_start;
        if !has_int_digits && !has_frac_digits {
            // lone "." (or "-.") — not a number
            return None;
        }
    } else if !has_int_digits {
        return None;
    }

    // exponent (only if followed by valid digits)
    if i < s.len() && (s[i] == b'e' || s[i] == b'E') {
        let mut j = i + 1;
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            j += 1;
        }
        let exp_digits_start = j;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_digits_start {
            is_float = true;
            i = j;
        }
        // else: the 'e' is not part of the number; stop before it.
    }

    let text = std::str::from_utf8(&s[num_start..i]).ok()?;
    if is_float {
        let f: f64 = text.parse().ok()?;
        Some((Num::Float(f), i))
    } else {
        match text.parse::<i64>() {
            Ok(n) => Some((Num::Int(n), i)),
            // integer literal too large for i64 => float (as PHP does)
            Err(_) => {
                let f: f64 = text.parse().ok()?;
                Some((Num::Float(f), i))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PhpArray;

    fn s(b: &str) -> Value {
        Value::str(b)
    }

    #[test]
    fn bool_casts() {
        assert!(!to_bool(&s("0")));
        assert!(!to_bool(&s("")));
        assert!(to_bool(&s("0.0")));
        assert!(to_bool(&s("false")));
        assert!(!to_bool(&Value::Float(0.0)));
        assert!(!to_bool(&Value::Array(PhpArray::new())));
        assert!(to_bool(&Value::Float(f64::NAN)));
    }

    #[test]
    fn int_casts() {
        assert_eq!(to_int(&s("123abc")), 123);
        assert_eq!(to_int(&s("abc")), 0);
        assert_eq!(to_int(&s("12.9")), 12);
        assert_eq!(to_int(&Value::Float(12.9)), 12);
        assert_eq!(to_int(&s("  42  ")), 42);
        assert_eq!(to_int(&s("0x1A")), 0);
        assert_eq!(to_int(&s("010")), 10);
    }

    #[test]
    fn float_casts() {
        assert_eq!(to_float(&s("1.5e3")), 1500.0);
        assert_eq!(to_float(&s("abc")), 0.0);
        assert_eq!(to_float(&s("5 apples")), 5.0);
    }

    #[test]
    fn numeric_string_cases() {
        assert!(numeric_string(b"123").is_some());
        assert!(numeric_string(b"1.5e3").is_some());
        assert!(numeric_string(b" 123").is_some());
        assert!(numeric_string(b"123 ").is_some());
        assert!(numeric_string(b"0x1A").is_none());
        assert!(numeric_string(b"0b11").is_none());
        assert!(numeric_string(b".5").is_some());
        assert!(numeric_string(b"9223372036854775808").is_some()); // -> float
    }

    #[test]
    fn number_for_arith() {
        assert_eq!(to_number(&s("5")), (Num::Int(5), true));
        assert_eq!(to_number(&s("5.5")), (Num::Float(5.5), true));
        assert_eq!(to_number(&s("  5  ")), (Num::Int(5), true));
        assert_eq!(to_number(&s("1e2")), (Num::Float(100.0), true));
        assert_eq!(to_number(&s("5 apples")).0, Num::Int(5));
        assert!(!to_number(&s("5 apples")).1); // not well-formed
    }

    #[test]
    fn string_form() {
        assert_eq!(to_string_bytes(&Value::Int(42)), b"42");
        assert_eq!(to_string_bytes(&Value::Bool(true)), b"1");
        assert_eq!(to_string_bytes(&Value::Bool(false)), b"");
        assert_eq!(to_string_bytes(&Value::Null), b"");
        assert_eq!(to_string_bytes(&Value::Float(3.0)), b"3");
    }
}
