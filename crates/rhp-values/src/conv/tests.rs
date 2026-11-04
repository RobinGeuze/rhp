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
