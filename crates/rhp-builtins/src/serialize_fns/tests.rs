// tests (expected values hardcoded from php 8.5.8)
#![allow(clippy::approx_constant)] // deliberate literal test values, not PI

use super::*;

struct NullHost;
impl Host for NullHost {
    fn write_out(&mut self, _bytes: &[u8]) {}
}

fn ser(v: Value) -> Vec<u8> {
    let mut h = NullHost;
    match fn_serialize(&mut h, &[v]).unwrap() {
        Value::Str(s) => s.as_bytes().to_vec(),
        _ => panic!("serialize did not return a string"),
    }
}

fn unser(s: &[u8]) -> Value {
    let mut h = NullHost;
    fn_unserialize(&mut h, &[Value::str(s.to_vec())]).unwrap()
}

fn arr(items: Vec<(ArrayKey, Value)>) -> Value {
    let mut a = PhpArray::new();
    for (k, v) in items {
        a.insert(k, v);
    }
    Value::Array(a)
}

// --- serialize: scalars -------------------------------------------------

#[test]
fn serialize_scalars() {
    // php -r 'echo serialize(null);'            => N;
    assert_eq!(ser(Value::Null), b"N;");
    // php -r 'echo serialize(false);'           => b:0;
    assert_eq!(ser(Value::Bool(false)), b"b:0;");
    // php -r 'echo serialize(true);'            => b:1;
    assert_eq!(ser(Value::Bool(true)), b"b:1;");
    // php -r 'echo serialize(42);'              => i:42;
    assert_eq!(ser(Value::Int(42)), b"i:42;");
    // php -r 'echo serialize(-7);'              => i:-7;
    assert_eq!(ser(Value::Int(-7)), b"i:-7;");
    // php -r 'echo serialize(3.14);'            => d:3.14;
    assert_eq!(ser(Value::Float(3.14)), b"d:3.14;");
    // php -r 'echo serialize("hello");'         => s:5:"hello";
    assert_eq!(ser(Value::str("hello")), b"s:5:\"hello\";");
}

#[test]
fn serialize_float_precision() {
    // php -r 'echo serialize(0.1);'             => d:0.1;
    assert_eq!(ser(Value::Float(0.1)), b"d:0.1;");
    // php -r 'echo serialize(0.1+0.2);'         => d:0.30000000000000004;
    assert_eq!(ser(Value::Float(0.1 + 0.2)), b"d:0.30000000000000004;");
    // php -r 'echo serialize(100.0);'           => d:100;
    assert_eq!(ser(Value::Float(100.0)), b"d:100;");
    // php -r 'echo serialize(1.0e20);'          => d:1.0E+20;
    assert_eq!(ser(Value::Float(1.0e20)), b"d:1.0E+20;");
    // php -r 'echo serialize(INF);'             => d:INF;
    assert_eq!(ser(Value::Float(f64::INFINITY)), b"d:INF;");
    // php -r 'echo serialize(-INF);'            => d:-INF;
    assert_eq!(ser(Value::Float(f64::NEG_INFINITY)), b"d:-INF;");
    // php -r 'echo serialize(NAN);'             => d:NAN;
    assert_eq!(ser(Value::Float(f64::NAN)), b"d:NAN;");
}

#[test]
fn serialize_byte_length_is_bytes() {
    // php -r 'echo bin2hex(serialize("\x00\x01ab"));' => 733a343a2200016162223b
    // i.e. s:4:"\x00\x01ab"; — 4 BYTES, quotes unescaped
    assert_eq!(
        ser(Value::str(vec![0u8, 1, b'a', b'b'])),
        b"s:4:\"\x00\x01ab\";"
    );
    // php -r 'echo serialize("he\"ll\\o");'     => s:7:"he"ll\o";
    assert_eq!(ser(Value::str("he\"ll\\o")), b"s:7:\"he\"ll\\o\";");
}

#[test]
fn serialize_arrays() {
    // php -r 'echo serialize([1,"two",3.5,true,null]);'
    //   => a:5:{i:0;i:1;i:1;s:3:"two";i:2;d:3.5;i:3;b:1;i:4;N;}
    let v = arr(vec![
        (ArrayKey::Int(0), Value::Int(1)),
        (ArrayKey::Int(1), Value::str("two")),
        (ArrayKey::Int(2), Value::Float(3.5)),
        (ArrayKey::Int(3), Value::Bool(true)),
        (ArrayKey::Int(4), Value::Null),
    ]);
    assert_eq!(
        ser(v),
        &b"a:5:{i:0;i:1;i:1;s:3:\"two\";i:2;d:3.5;i:3;b:1;i:4;N;}"[..]
    );
}

#[test]
fn serialize_nested_and_string_keys() {
    // php -r 'echo serialize(["a"=>1,"b"=>[2,3]]);'
    //   => a:2:{s:1:"a";i:1;s:1:"b";a:2:{i:0;i:2;i:1;i:3;}}
    let inner = arr(vec![
        (ArrayKey::Int(0), Value::Int(2)),
        (ArrayKey::Int(1), Value::Int(3)),
    ]);
    let v = arr(vec![
        (
            ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"a".to_vec())),
            Value::Int(1),
        ),
        (
            ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"b".to_vec())),
            inner,
        ),
    ]);
    assert_eq!(
        ser(v),
        &b"a:2:{s:1:\"a\";i:1;s:1:\"b\";a:2:{i:0;i:2;i:1;i:3;}}"[..]
    );
}

#[test]
fn serialize_empty_array() {
    // php -r 'echo serialize([]);'              => a:0:{}
    assert_eq!(ser(arr(vec![])), b"a:0:{}");
}

// --- unserialize: happy path -------------------------------------------

#[test]
fn unserialize_scalars() {
    assert!(matches!(unser(b"N;"), Value::Null));
    assert!(matches!(unser(b"b:1;"), Value::Bool(true)));
    assert!(matches!(unser(b"b:0;"), Value::Bool(false)));
    assert!(matches!(unser(b"i:42;"), Value::Int(42)));
    assert!(matches!(unser(b"i:-7;"), Value::Int(-7)));
    match unser(b"d:3.14;") {
        Value::Float(f) => assert_eq!(f, 3.14),
        _ => panic!(),
    }
    match unser(b"s:5:\"hello\";") {
        Value::Str(s) => assert_eq!(s.as_bytes(), b"hello"),
        _ => panic!(),
    }
}

#[test]
fn unserialize_float_specials() {
    match unser(b"d:0.30000000000000004;") {
        Value::Float(f) => assert_eq!(f, 0.1 + 0.2),
        _ => panic!(),
    }
    match unser(b"d:1.0E+20;") {
        Value::Float(f) => assert_eq!(f, 1.0e20),
        _ => panic!(),
    }
    assert!(matches!(unser(b"d:INF;"), Value::Float(f) if f == f64::INFINITY));
    assert!(matches!(unser(b"d:-INF;"), Value::Float(f) if f == f64::NEG_INFINITY));
    assert!(matches!(unser(b"d:NAN;"), Value::Float(f) if f.is_nan()));
}

#[test]
fn unserialize_array() {
    // php -r 'var_dump(unserialize("a:2:{i:0;i:1;i:1;i:2;}"));'
    //   => array(2){ [0]=>int(1) [1]=>int(2) }
    match unser(b"a:2:{i:0;i:1;i:1;i:2;}") {
        Value::Array(a) => {
            assert_eq!(a.len(), 2);
            assert!(matches!(a.get(&ArrayKey::Int(0)), Some(Value::Int(1))));
            assert!(matches!(a.get(&ArrayKey::Int(1)), Some(Value::Int(2))));
        }
        _ => panic!(),
    }
    assert!(matches!(unser(b"a:0:{}"), Value::Array(a) if a.is_empty()));
}

#[test]
fn unserialize_int_overflow_saturates() {
    // php -r 'var_dump(unserialize("i:9999999999999999999999;"));'
    //   => int(9223372036854775807)
    assert!(matches!(
        unser(b"i:9999999999999999999999;"),
        Value::Int(i64::MAX)
    ));
}

// --- round trip ---------------------------------------------------------

#[test]
fn round_trip() {
    let v = arr(vec![
        (ArrayKey::Int(0), Value::Int(1)),
        (ArrayKey::Int(1), Value::str("two")),
        (ArrayKey::Int(2), Value::Float(3.5)),
        (ArrayKey::Int(3), Value::Bool(true)),
        (ArrayKey::Int(4), Value::Null),
    ]);
    let s = ser(v);
    assert_eq!(
        s,
        &b"a:5:{i:0;i:1;i:1;s:3:\"two\";i:2;d:3.5;i:3;b:1;i:4;N;}"[..]
    );
    match unser(&s) {
        Value::Array(a) => {
            assert_eq!(a.len(), 5);
            assert!(matches!(a.get(&ArrayKey::Int(0)), Some(Value::Int(1))));
            assert!(
                matches!(a.get(&ArrayKey::Int(1)), Some(Value::Str(x)) if x.as_bytes() == b"two")
            );
            assert!(matches!(a.get(&ArrayKey::Int(2)), Some(Value::Float(f)) if *f == 3.5));
            assert!(matches!(a.get(&ArrayKey::Int(3)), Some(Value::Bool(true))));
            assert!(matches!(a.get(&ArrayKey::Int(4)), Some(Value::Null)));
        }
        _ => panic!(),
    }
}

// --- unserialize: malformed => false -----------------------------------

#[test]
fn unserialize_malformed_returns_false() {
    // All of these are `bool(false)` under php 8.5.8.
    assert!(matches!(unser(b"x"), Value::Bool(false)));
    assert!(matches!(unser(b""), Value::Bool(false)));
    assert!(matches!(unser(b"i:42"), Value::Bool(false))); // missing ';'
    assert!(matches!(unser(b"s:5:\"hel\";"), Value::Bool(false))); // short string
    assert!(matches!(unser(b"s:3:\"ab\";"), Value::Bool(false))); // len mismatch
    assert!(matches!(unser(b"b:2;"), Value::Bool(false))); // bad bool
}

#[test]
fn unserialize_trailing_data_tolerated() {
    // php -r 'var_dump(unserialize("s:3:\"abc\";extra"));' => string(3) "abc"
    match unser(b"s:3:\"abc\";extra") {
        Value::Str(s) => assert_eq!(s.as_bytes(), b"abc"),
        _ => panic!(),
    }
}
