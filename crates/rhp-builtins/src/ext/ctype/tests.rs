// Expected values captured from php 8.5.8.

use super::*;

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
