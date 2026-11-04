// tests — expected values captured from php 8.5.8

use super::*;

struct NoopHost;
impl Host for NoopHost {
    fn write_out(&mut self, _: &[u8]) {}
}

fn s(b: &str) -> Value {
    Value::str(b)
}

fn enc(v: &Value, flags: i64) -> String {
    let r = fn_json_encode(&mut NoopHost, &[v.clone(), Value::Int(flags)]).unwrap();
    match r {
        Value::Str(s) => String::from_utf8(s.as_bytes().to_vec()).unwrap(),
        Value::Bool(false) => "<false>".to_string(),
        _ => unreachable!(),
    }
}

fn dec(json: &str) -> Value {
    fn_json_decode(&mut NoopHost, &[s(json)]).unwrap()
}

fn dec_f(json: &str, assoc: Value, depth: i64, flags: i64) -> Value {
    fn_json_decode(
        &mut NoopHost,
        &[s(json), assoc, Value::Int(depth), Value::Int(flags)],
    )
    .unwrap()
}

fn arr(items: &[Value]) -> Value {
    let mut a = PhpArray::new();
    for v in items {
        a.push(v.clone());
    }
    Value::Array(a)
}

fn map(pairs: &[(&str, Value)]) -> Value {
    let mut a = PhpArray::new();
    for (k, v) in pairs {
        a.insert(ArrayKey::from_bytes(k.as_bytes()), v.clone());
    }
    Value::Array(a)
}

// --- encode ----------------------------------------------------------

#[test]
fn encode_scalars_and_strings() {
    assert_eq!(enc(&Value::Null, 0), "null");
    assert_eq!(enc(&Value::Bool(true), 0), "true");
    assert_eq!(enc(&Value::Int(42), 0), "42");
    assert_eq!(enc(&Value::Float(1.0), 0), "1");
    assert_eq!(enc(&Value::Float(1.5), 0), "1.5");
    assert_eq!(enc(&Value::Float(1e20), 0), "1.0e+20");
    assert_eq!(enc(&Value::Float(f64::NAN), 0), "<false>");
    assert_eq!(enc(&s("a/b"), 0), "\"a\\/b\"");
    assert_eq!(enc(&s("café"), 0), "\"caf\\u00e9\"");
    assert_eq!(enc(&s("日本"), 0), "\"\\u65e5\\u672c\"");
    assert_eq!(enc(&s("emoji😀"), 0), "\"emoji\\ud83d\\ude00\"");
    assert_eq!(enc(&Value::str(vec![0xC3, 0x28]), 0), "<false>");
}

#[test]
fn encode_flags() {
    assert_eq!(enc(&s("a/b"), JSON_UNESCAPED_SLASHES), "\"a/b\"");
    assert_eq!(enc(&s("café"), JSON_UNESCAPED_UNICODE), "\"café\"");
    assert_eq!(
        enc(&arr(&[s("1"), s("2.5"), s("x")]), JSON_NUMERIC_CHECK),
        "[1,2.5,\"x\"]"
    );
    assert_eq!(
        enc(&arr(&[Value::Int(1), Value::Int(2)]), JSON_FORCE_OBJECT),
        "{\"0\":1,\"1\":2}"
    );
}

#[test]
fn encode_pretty_print() {
    assert_eq!(enc(&arr(&[]), JSON_PRETTY_PRINT), "[]");
    assert_eq!(
        enc(&map(&[("a", arr(&[]))]), JSON_PRETTY_PRINT),
        "{\n    \"a\": []\n}"
    );
    assert_eq!(
        enc(
            &arr(&[Value::Int(1), arr(&[Value::Int(2), Value::Int(3)])]),
            JSON_PRETTY_PRINT
        ),
        "[\n    1,\n    [\n        2,\n        3\n    ]\n]"
    );
    assert_eq!(
        enc(
            &map(&[
                ("a", Value::Int(1)),
                ("b", arr(&[Value::Int(1), Value::Int(2), Value::Int(3)])),
                ("c", map(&[("x", Value::Bool(true)), ("y", Value::Null)])),
                ("d", arr(&[])),
            ]),
            JSON_PRETTY_PRINT
        ),
        "{\n    \"a\": 1,\n    \"b\": [\n        1,\n        2,\n        3\n    ],\n    \"c\": {\n        \"x\": true,\n        \"y\": null\n    },\n    \"d\": []\n}"
    );
}

// --- decode ----------------------------------------------------------

fn is_int(v: &Value, n: i64) -> bool {
    matches!(v, Value::Int(x) if *x == n)
}

#[test]
fn decode_scalars_and_numbers() {
    assert!(is_int(&dec("123"), 123));
    assert!(matches!(dec("123.45"), Value::Float(f) if (f - 123.45).abs() < 1e-9));
    assert!(matches!(dec("1e3"), Value::Float(f) if f == 1000.0));
    assert!(is_int(&dec("-0"), 0));
    assert!(matches!(dec("0.0"), Value::Float(f) if f == 0.0));
    assert!(matches!(dec("true"), Value::Bool(true)));
    assert!(matches!(dec("false"), Value::Bool(false)));
    assert!(matches!(dec("null"), Value::Null));
    // overflow -> float by default
    assert!(matches!(dec("12345678901234567890"), Value::Float(_)));
    // BIGINT_AS_STRING -> string
    let big = dec_f(
        "12345678901234567890",
        Value::Null,
        512,
        JSON_BIGINT_AS_STRING,
    );
    assert!(matches!(&big, Value::Str(s) if s.as_bytes() == b"12345678901234567890"));
}

#[test]
fn decode_strings_and_unicode() {
    assert!(matches!(dec("\"hi\\u0041\""), Value::Str(s) if s.as_bytes() == b"hiA"));
    // surrogate pair -> 😀 (U+1F600)
    assert!(matches!(dec("\"\\ud83d\\ude00\""), Value::Str(s) if s.as_bytes() == "😀".as_bytes()));
    // lone surrogate -> error -> null
    assert!(matches!(dec("\"\\ud83d\""), Value::Null));
    assert_eq!(get_last_error(), JSON_ERROR_UTF16);
}

#[test]
fn decode_containers() {
    assert!(matches!(dec("[1,2,3]"), Value::Array(a) if a.len() == 3));
    assert!(matches!(dec("  [1,2]  "), Value::Array(a) if a.len() == 2));
    let obj = dec("{\"a\":1,\"b\":2}");
    assert!(matches!(&obj, Value::Array(a) if a.len() == 2));
    // duplicate key -> last wins
    let dup = dec("{\"a\":1,\"a\":2}");
    if let Value::Array(a) = dup {
        assert_eq!(a.len(), 1);
        assert!(is_int(a.get(&ArrayKey::from_bytes(b"a")).unwrap(), 2));
    } else {
        panic!("expected array");
    }
}

#[test]
fn decode_errors() {
    assert!(matches!(dec("[1,2] x"), Value::Null));
    assert_eq!(get_last_error(), JSON_ERROR_SYNTAX);
    assert!(matches!(dec("[1,2"), Value::Null));
    assert!(matches!(dec("{bad}"), Value::Null));
    assert!(matches!(dec(""), Value::Null));
    assert_eq!(get_last_error(), JSON_ERROR_SYNTAX);
    assert!(matches!(dec("01"), Value::Null));
    assert!(matches!(dec("[1,]"), Value::Null));
    // control char in string
    assert!(matches!(dec("\"a\nb\""), Value::Null));
    assert_eq!(get_last_error(), JSON_ERROR_CTRL_CHAR);
}

#[test]
fn decode_depth_limit() {
    // [1] is 1 level deep: needs depth >= 2.
    assert!(matches!(dec_f("[1]", Value::Null, 1, 0), Value::Null));
    assert_eq!(get_last_error(), JSON_ERROR_DEPTH);
    assert!(matches!(dec_f("[1]", Value::Null, 2, 0), Value::Array(_)));
    assert!(matches!(dec_f("[[1]]", Value::Null, 2, 0), Value::Null));
    assert!(matches!(dec_f("[[1]]", Value::Null, 3, 0), Value::Array(_)));
    assert!(matches!(dec_f("5", Value::Null, 1, 0), Value::Int(5)));
}

#[test]
fn decode_round_trip_with_encode() {
    let original = map(&[
        ("name", s("café")),
        ("nums", arr(&[Value::Int(1), Value::Int(2), Value::Int(3)])),
        ("nested", map(&[("ok", Value::Bool(true))])),
    ]);
    let encoded = enc(&original, 0);
    let decoded = dec(&encoded);
    // re-encode the decoded value; should match the first encoding.
    assert_eq!(enc(&decoded, 0), encoded);
}

#[test]
fn last_error_msg_roundtrip() {
    dec("{bad}");
    assert!(matches!(
        fn_json_last_error(&mut NoopHost, &[]).unwrap(),
        Value::Int(x) if x == JSON_ERROR_SYNTAX
    ));
    let msg = fn_json_last_error_msg(&mut NoopHost, &[]).unwrap();
    assert!(matches!(msg, Value::Str(s) if s.as_bytes() == b"Syntax error"));
    dec("[1]");
    assert!(matches!(
        fn_json_last_error(&mut NoopHost, &[]).unwrap(),
        Value::Int(0)
    ));
}
