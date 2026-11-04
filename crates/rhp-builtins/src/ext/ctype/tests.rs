// tests — expected values captured from php 8.5.8

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
fn ctype_classes_locale_independent_part() {
    // High bytes classify per the host locale (as in php), so only assert
    // what holds in every locale: the ASCII classes, and that no high byte
    // is a digit / hex digit. The live `ctype` difftest sweep checks the
    // high bytes against the php on the same machine.
    for b in 0u8..128 {
        let c = b as char;
        assert_eq!(is_digit(b), c.is_ascii_digit(), "digit {b}");
        assert_eq!(is_xdigit(b), c.is_ascii_hexdigit(), "xdigit {b}");
        assert_eq!(is_upper(b), c.is_ascii_uppercase(), "upper {b}");
        assert_eq!(is_lower(b), c.is_ascii_lowercase(), "lower {b}");
        assert_eq!(is_alpha(b), c.is_ascii_alphabetic(), "alpha {b}");
        assert_eq!(is_alnum(b), c.is_ascii_alphanumeric(), "alnum {b}");
        assert_eq!(is_punct(b), c.is_ascii_punctuation(), "punct {b}");
        assert_eq!(is_cntrl(b), c.is_ascii_control(), "cntrl {b}");
        assert_eq!(is_graph(b), c.is_ascii_graphic(), "graph {b}");
        assert_eq!(is_print(b), c.is_ascii_graphic() || b == b' ', "print {b}");
        assert_eq!(
            is_space(b),
            matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'),
            "space {b}"
        );
    }
    for b in 128u8..=255 {
        assert!(!is_digit(b) && !is_xdigit(b), "high byte {b} is a digit");
    }
}
