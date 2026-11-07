use super::*;
use crate::test_support::TestHost;

fn p(s: &str) -> Option<Url> {
    parse(s.as_bytes())
}

#[test]
fn shapes() {
    let u = p("http://user:pw@example.com:8080/a/b?x=1#frag").unwrap();
    assert_eq!(u.scheme.as_deref(), Some(&b"http"[..]));
    assert_eq!(u.user.as_deref(), Some(&b"user"[..]));
    assert_eq!(u.pass.as_deref(), Some(&b"pw"[..]));
    assert_eq!(u.host.as_deref(), Some(&b"example.com"[..]));
    assert_eq!(u.port, 8080);
    assert_eq!(u.path.as_deref(), Some(&b"/a/b"[..]));
    assert_eq!(u.query.as_deref(), Some(&b"x=1"[..]));
    assert_eq!(u.fragment.as_deref(), Some(&b"frag"[..]));
    let u = p("127.0.0.1:443").unwrap();
    assert_eq!(u.host.as_deref(), Some(&b"127.0.0.1"[..]));
    assert_eq!(u.port, 443);
    // no scheme: php takes a bracketed host as a path
    let u = p("[::1]:443").unwrap();
    assert_eq!(u.path.as_deref(), Some(&b"[::1]:443"[..]));
    let u = p("http://[::1]:443/x").unwrap();
    assert_eq!(u.host.as_deref(), Some(&b"[::1]"[..]));
    assert_eq!(u.port, 443);
    assert!(p(":80").is_none());
    assert!(p("http://host:99999").is_none());
    let u = p("http://h?").unwrap();
    assert_eq!(u.query.as_deref(), Some(&b""[..]));
    assert!(u.path.is_none());
    let u = p("/p").unwrap();
    assert_eq!(u.path.as_deref(), Some(&b"/p"[..]));
    let u = p("").unwrap();
    assert_eq!(u.path.as_deref(), Some(&b""[..]));
    // ZEND_STRTOL: white space and a sign before the port
    assert_eq!(p("http://h: 80/").unwrap().port, 80);
    assert_eq!(p("http://h:+80/").unwrap().port, 80);
    assert_eq!(p("http://h:-0/").unwrap().port, 0);
    assert!(p("http://h:-1/").is_none());
    assert!(p("http://h: /").is_none());
}

fn s(b: &str) -> Value {
    Value::str(b)
}

fn sb(b: &[u8]) -> Value {
    Value::str(b)
}

fn call_bytes(f: crate::NativeFn, args: &[Value]) -> Vec<u8> {
    match f(&mut TestHost::new(), args).expect("no throw") {
        Value::Str(b) => b.as_bytes().to_vec(),
        other => panic!("expected string, got {}", other.type_name()),
    }
}

fn call_str(f: crate::NativeFn, args: &[Value]) -> String {
    String::from_utf8(call_bytes(f, args)).unwrap()
}

#[test]
fn urlencode_cases() {
    assert_eq!(
        call_str(fn_urlencode, &[s("a b+c&d=e/f?g~h.i_j-k")]),
        "a+b%2Bc%26d%3De%2Ff%3Fg%7Eh.i_j-k"
    );
    assert_eq!(
        call_bytes(fn_urlencode, &[sb(b"\xff\x00A~")]),
        b"%FF%00A%7E"
    );
}

#[test]
fn rawurlencode_cases() {
    assert_eq!(
        call_str(fn_rawurlencode, &[s("a b+c&d=e/f?g~h.i_j-k")]),
        "a%20b%2Bc%26d%3De%2Ff%3Fg~h.i_j-k"
    );
    assert_eq!(
        call_bytes(fn_rawurlencode, &[sb(b"\xff\x00A~")]),
        b"%FF%00A~"
    );
}

#[test]
fn urldecode_cases() {
    assert_eq!(
        call_str(fn_urldecode, &[s("a+b%20c%26d%ZZ%4")]),
        "a b c&d%ZZ%4"
    );
    assert_eq!(call_str(fn_urldecode, &[s("abc%")]), "abc%");
    assert_eq!(call_str(fn_rawurldecode, &[s("a+b%20c%26d")]), "a+b c&d");
    assert_eq!(call_str(fn_rawurldecode, &[s("ab%2")]), "ab%2");
}
