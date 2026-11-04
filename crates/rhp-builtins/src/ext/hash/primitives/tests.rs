// tests: expected values from php

use super::*;
use crate::test_support::TestHost;

fn s(b: &str) -> Value {
    Value::str(b)
}
fn call(f: crate::NativeFn, args: &[Value]) -> Value {
    f(&mut TestHost::new(), args).expect("no throw")
}
fn calls(f: crate::NativeFn, args: &[Value]) -> String {
    String::from_utf8(bytes_of(&call(f, args))).unwrap()
}
/// True when the result is php `false` (used for failure returns).
fn is_false(f: crate::NativeFn, args: &[Value]) -> bool {
    matches!(call(f, args), Value::Bool(false))
}

#[test]
fn base64_encode_vectors() {
    assert_eq!(calls(fn_base64_encode, &[s("")]), "");
    assert_eq!(calls(fn_base64_encode, &[s("f")]), "Zg==");
    assert_eq!(calls(fn_base64_encode, &[s("fo")]), "Zm8=");
    assert_eq!(calls(fn_base64_encode, &[s("foo")]), "Zm9v");
    assert_eq!(calls(fn_base64_encode, &[s("foob")]), "Zm9vYg==");
    assert_eq!(calls(fn_base64_encode, &[s("fooba")]), "Zm9vYmE=");
    assert_eq!(calls(fn_base64_encode, &[s("foobar")]), "Zm9vYmFy");
    assert_eq!(
        calls(fn_base64_encode, &[s("Hello, World!")]),
        "SGVsbG8sIFdvcmxkIQ=="
    );
    assert_eq!(
        calls(
            fn_base64_encode,
            &[Value::str(vec![0x00, 0xff, 0x80, 0x7f])]
        ),
        "AP+Afw=="
    );
}

#[test]
fn base64_decode_vectors() {
    assert_eq!(calls(fn_base64_decode, &[s("Zm9vYmFy")]), "foobar");
    assert_eq!(
        calls(fn_base64_decode, &[s("SGVsbG8sIFdvcmxkIQ==")]),
        "Hello, World!"
    );
    assert_eq!(calls(fn_base64_decode, &[s("")]), "");
    assert_eq!(calls(fn_base64_decode, &[s("====")]), "");
    // non-strict: skip invalid chars / whitespace
    assert_eq!(calls(fn_base64_decode, &[s("Zm9v!!")]), "foo");
    assert_eq!(calls(fn_base64_decode, &[s("Zm 9v")]), "foo");
    assert_eq!(calls(fn_base64_decode, &[s("QQ")]), "A");
    // strict: invalid chars fail, whitespace ok
    assert!(is_false(
        fn_base64_decode,
        &[s("Zm9v!!"), Value::Bool(true)]
    ));
    assert_eq!(
        calls(fn_base64_decode, &[s("Zm 9v"), Value::Bool(true)]),
        "foo"
    );
    assert_eq!(
        calls(fn_base64_decode, &[s("Zm9vYg=="), Value::Bool(true)]),
        "foob"
    );
    assert!(is_false(
        fn_base64_decode,
        &[s("Zm9vYg="), Value::Bool(true)]
    ));
    assert_eq!(
        calls(fn_base64_decode, &[s("Zm9vYg"), Value::Bool(true)]),
        "foob"
    );
    assert!(is_false(fn_base64_decode, &[s("Z==="), Value::Bool(true)]));
    assert!(is_false(fn_base64_decode, &[s("A"), Value::Bool(true)]));
    assert_eq!(calls(fn_base64_decode, &[s("QQ"), Value::Bool(true)]), "A");
}

#[test]
fn base64_roundtrip_binary() {
    // decode non-strict of "ab=c" -> bytes 69 b7
    let v = call(fn_base64_decode, &[s("ab=c")]);
    assert_eq!(bytes_of(&v), vec![0x69, 0xb7]);
    let v = call(fn_base64_decode, &[s("YQ==YQ==")]);
    assert_eq!(bytes_of(&v), vec![0x61, 0x06, 0x10]);
}

#[test]
fn bin2hex_hex2bin_vectors() {
    assert_eq!(calls(fn_bin2hex, &[s("abc")]), "616263");
    assert_eq!(calls(fn_bin2hex, &[s("")]), "");
    assert_eq!(
        calls(fn_bin2hex, &[Value::str(vec![0xff, 0x00, 0x10])]),
        "ff0010"
    );
    assert_eq!(calls(fn_hex2bin, &[s("616263")]), "abc");
    assert_eq!(calls(fn_hex2bin, &[s("")]), "");
    assert_eq!(
        bytes_of(&call(fn_hex2bin, &[s("ff0010")])),
        vec![0xff, 0x00, 0x10]
    );
    // odd length -> false
    assert!(is_false(fn_hex2bin, &[s("abc")]));
    // non-hex -> false
    assert!(is_false(fn_hex2bin, &[s("xy")]));
}
