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
fn call_int(f: crate::NativeFn, args: &[Value]) -> i64 {
    match call(f, args) {
        Value::Int(n) => n,
        other => panic!("expected int, got {}", other.type_name()),
    }
}
/// True when the result is php `false` (used for failure returns).
fn is_false(f: crate::NativeFn, args: &[Value]) -> bool {
    matches!(call(f, args), Value::Bool(false))
}

#[test]
fn md5_vectors() {
    assert_eq!(calls(fn_md5, &[s("")]), "d41d8cd98f00b204e9800998ecf8427e");
    assert_eq!(
        calls(fn_md5, &[s("abc")]),
        "900150983cd24fb0d6963f7d28e17f72"
    );
    assert_eq!(
        calls(fn_md5, &[s("message digest")]),
        "f96b697d7cb7938d525a2f31aaf161d0"
    );
    assert_eq!(
        calls(fn_md5, &[s("The quick brown fox jumps over the lazy dog")]),
        "9e107d9d372bb6826bd81d3542a419d6"
    );
    // binary form: bin2hex(md5($s, true)) == md5($s)
    let bin = call(fn_md5, &[s("abc"), Value::Bool(true)]);
    assert_eq!(bytes_of(&bin).len(), 16);
    assert_eq!(
        String::from_utf8(to_hex(&bytes_of(&bin))).unwrap(),
        "900150983cd24fb0d6963f7d28e17f72"
    );
}

#[test]
fn sha1_vectors() {
    assert_eq!(
        calls(fn_sha1, &[s("")]),
        "da39a3ee5e6b4b0d3255bfef95601890afd80709"
    );
    assert_eq!(
        calls(fn_sha1, &[s("abc")]),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
    assert_eq!(
        calls(fn_sha1, &[s("The quick brown fox jumps over the lazy dog")]),
        "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"
    );
    assert_eq!(
        calls(
            fn_sha1,
            &[s(
                "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )]
        ),
        "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
    );
    let bin = call(fn_sha1, &[s("abc"), Value::Bool(true)]);
    assert_eq!(bytes_of(&bin).len(), 20);
}

#[test]
fn crc32_vectors() {
    assert_eq!(call_int(fn_crc32, &[s("")]), 0);
    assert_eq!(call_int(fn_crc32, &[s("abc")]), 891568578);
    assert_eq!(
        call_int(
            fn_crc32,
            &[s("The quick brown fox jumps over the lazy dog")]
        ),
        1095738169
    );
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
