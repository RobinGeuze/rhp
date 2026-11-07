// Expected values captured from php 8.5.8.

use super::*;
use crate::test_support::TestHost;
use rhp_values::PhpArray;

fn s(b: &str) -> Value {
    Value::str(b)
}

/// filter_var($value, $filter)
fn fv(value: Value, filter: i64) -> Value {
    fn_filter_var(&mut TestHost::new(), &[value, Value::Int(filter)]).unwrap()
}
/// filter_var($value, $filter, $flags_int)
fn fvf(value: Value, filter: i64, flags: i64) -> Value {
    fn_filter_var(
        &mut TestHost::new(),
        &[value, Value::Int(filter), Value::Int(flags)],
    )
    .unwrap()
}
/// filter_var($value, $filter, ['options'=>[...]])
fn fv_opts(value: Value, filter: i64, opts: &[(&str, Value)]) -> Value {
    let mut inner = PhpArray::new();
    for (k, v) in opts {
        inner.insert(ArrayKey::from_bytes(k.as_bytes()), v.clone());
    }
    let mut outer = PhpArray::new();
    outer.insert(ArrayKey::from_bytes(b"options"), Value::Array(inner));
    fn_filter_var(
        &mut TestHost::new(),
        &[value, Value::Int(filter), Value::Array(outer)],
    )
    .unwrap()
}

fn is_false(v: &Value) -> bool {
    matches!(v, Value::Bool(false))
}
fn is_null(v: &Value) -> bool {
    matches!(v, Value::Null)
}
fn as_str(v: &Value) -> String {
    match v {
        Value::Str(s) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
        other => panic!("expected string, got {}", other.type_name()),
    }
}
fn as_int(v: &Value) -> i64 {
    match v {
        Value::Int(n) => *n,
        other => panic!("expected int, got {}", other.type_name()),
    }
}
fn as_float(v: &Value) -> f64 {
    match v {
        Value::Float(f) => *f,
        other => panic!("expected float, got {}", other.type_name()),
    }
}
fn as_bool(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        other => panic!("expected bool, got {}", other.type_name()),
    }
}

#[test]
fn validate_int_cases() {
    assert_eq!(as_int(&fv(s("123"), FILTER_VALIDATE_INT)), 123);
    assert_eq!(as_int(&fv(s("  123  "), FILTER_VALIDATE_INT)), 123);
    assert_eq!(as_int(&fv(s("+5"), FILTER_VALIDATE_INT)), 5);
    assert_eq!(as_int(&fv(s("-5"), FILTER_VALIDATE_INT)), -5);
    assert_eq!(as_int(&fv(s("0"), FILTER_VALIDATE_INT)), 0);
    assert!(is_false(&fv(s("007"), FILTER_VALIDATE_INT)));
    assert!(is_false(&fv(s("abc"), FILTER_VALIDATE_INT)));
    assert!(is_false(&fv(s("12.5"), FILTER_VALIDATE_INT)));
    assert!(is_false(&fv(s(""), FILTER_VALIDATE_INT)));
    assert!(is_false(&fv(s("1 2"), FILTER_VALIDATE_INT)));
    assert_eq!(
        as_int(&fv(s("9223372036854775807"), FILTER_VALIDATE_INT)),
        i64::MAX
    );
    assert!(is_false(&fv(s("9223372036854775808"), FILTER_VALIDATE_INT)));
    assert_eq!(
        as_int(&fv(s("-9223372036854775808"), FILTER_VALIDATE_INT)),
        i64::MIN
    );
    // non-string inputs
    assert_eq!(as_int(&fv(Value::Int(123), FILTER_VALIDATE_INT)), 123);
    assert_eq!(as_int(&fv(Value::Float(12.0), FILTER_VALIDATE_INT)), 12);
    assert!(is_false(&fv(Value::Float(12.5), FILTER_VALIDATE_INT)));
    assert_eq!(as_int(&fv(Value::Bool(true), FILTER_VALIDATE_INT)), 1);
    assert!(is_false(&fv(Value::Bool(false), FILTER_VALIDATE_INT)));
    assert!(is_false(&fv(Value::Null, FILTER_VALIDATE_INT)));
    // flags
    assert_eq!(
        as_int(&fvf(s("0x1A"), FILTER_VALIDATE_INT, FILTER_FLAG_ALLOW_HEX)),
        26
    );
    assert!(is_false(&fvf(
        s("1A"),
        FILTER_VALIDATE_INT,
        FILTER_FLAG_ALLOW_HEX
    )));
    assert_eq!(
        as_int(&fvf(
            s("0755"),
            FILTER_VALIDATE_INT,
            FILTER_FLAG_ALLOW_OCTAL
        )),
        493
    );
    assert!(is_false(&fvf(
        s("0789"),
        FILTER_VALIDATE_INT,
        FILTER_FLAG_ALLOW_OCTAL
    )));
    // null on failure
    assert!(is_null(&fvf(
        s("abc"),
        FILTER_VALIDATE_INT,
        FILTER_NULL_ON_FAILURE
    )));
    // range
    assert_eq!(
        as_int(&fv_opts(
            s("5"),
            FILTER_VALIDATE_INT,
            &[("min_range", Value::Int(1)), ("max_range", Value::Int(10))]
        )),
        5
    );
    assert!(is_false(&fv_opts(
        s("15"),
        FILTER_VALIDATE_INT,
        &[("min_range", Value::Int(1)), ("max_range", Value::Int(10))]
    )));
    assert_eq!(
        as_int(&fv_opts(
            s("abc"),
            FILTER_VALIDATE_INT,
            &[("default", Value::Int(-1))]
        )),
        -1
    );
}

#[test]
fn validate_float_cases() {
    assert_eq!(as_float(&fv(s("12.5"), FILTER_VALIDATE_FLOAT)), 12.5);
    assert_eq!(as_float(&fv(s("123"), FILTER_VALIDATE_FLOAT)), 123.0);
    assert_eq!(as_float(&fv(s("  12.5  "), FILTER_VALIDATE_FLOAT)), 12.5);
    assert_eq!(as_float(&fv(s("+5.5"), FILTER_VALIDATE_FLOAT)), 5.5);
    assert_eq!(as_float(&fv(s("-.5"), FILTER_VALIDATE_FLOAT)), -0.5);
    assert_eq!(as_float(&fv(s("5."), FILTER_VALIDATE_FLOAT)), 5.0);
    assert_eq!(as_float(&fv(s("1e3"), FILTER_VALIDATE_FLOAT)), 1000.0);
    assert_eq!(as_float(&fv(s("1.5e-3"), FILTER_VALIDATE_FLOAT)), 0.0015);
    assert_eq!(as_float(&fv(s("+.5e2"), FILTER_VALIDATE_FLOAT)), 50.0);
    assert_eq!(as_float(&fv(s("007"), FILTER_VALIDATE_FLOAT)), 7.0);
    assert!(is_false(&fv(s("abc"), FILTER_VALIDATE_FLOAT)));
    assert!(is_false(&fv(s(""), FILTER_VALIDATE_FLOAT)));
    assert!(is_false(&fv(s("12.5.6"), FILTER_VALIDATE_FLOAT)));
    assert!(is_false(&fv(s("1,000"), FILTER_VALIDATE_FLOAT)));
    assert!(is_false(&fv(s("1e"), FILTER_VALIDATE_FLOAT)));
    assert!(is_false(&fv(s("inf"), FILTER_VALIDATE_FLOAT)));
    assert_eq!(as_float(&fv(Value::Int(123), FILTER_VALIDATE_FLOAT)), 123.0);
    assert_eq!(as_float(&fv(Value::Bool(true), FILTER_VALIDATE_FLOAT)), 1.0);
    assert!(is_false(&fv(Value::Bool(false), FILTER_VALIDATE_FLOAT)));
    // thousand flag
    assert_eq!(
        as_float(&fvf(
            s("1,234.5"),
            FILTER_VALIDATE_FLOAT,
            FILTER_FLAG_ALLOW_THOUSAND
        )),
        1234.5
    );
    assert_eq!(
        as_float(&fvf(
            s("1,234,567"),
            FILTER_VALIDATE_FLOAT,
            FILTER_FLAG_ALLOW_THOUSAND
        )),
        1234567.0
    );
    assert!(is_false(&fvf(
        s("1,23"),
        FILTER_VALIDATE_FLOAT,
        FILTER_FLAG_ALLOW_THOUSAND
    )));
    assert!(is_false(&fvf(
        s("12,34.5"),
        FILTER_VALIDATE_FLOAT,
        FILTER_FLAG_ALLOW_THOUSAND
    )));
    // decimal option
    assert_eq!(
        as_float(&fv_opts(
            s("1,5"),
            FILTER_VALIDATE_FLOAT,
            &[("decimal", s(","))]
        )),
        1.5
    );
    assert!(is_false(&fv_opts(
        s("1.5"),
        FILTER_VALIDATE_FLOAT,
        &[("decimal", s(","))]
    )));
}

#[test]
fn validate_bool_cases() {
    for t in ["1", "true", "on", "yes", "TRUE", "On", "  true  ", "yes "] {
        assert!(as_bool(&fv(s(t), FILTER_VALIDATE_BOOL)), "{t}");
    }
    for f in ["0", "false", "off", "no", "FALSE", ""] {
        assert!(!as_bool(&fv(s(f), FILTER_VALIDATE_BOOL)), "{f}");
    }
    for n in ["2", "abc", "00", "1.0", "t", "y", "-1"] {
        assert!(is_false(&fv(s(n), FILTER_VALIDATE_BOOL)), "{n}");
    }
    // null on failure
    assert!(is_null(&fvf(
        s("abc"),
        FILTER_VALIDATE_BOOL,
        FILTER_NULL_ON_FAILURE
    )));
    assert!(!as_bool(&fvf(
        s(""),
        FILTER_VALIDATE_BOOL,
        FILTER_NULL_ON_FAILURE
    )));
    // non-string
    assert!(as_bool(&fv(Value::Bool(true), FILTER_VALIDATE_BOOL)));
    assert!(as_bool(&fv(Value::Int(1), FILTER_VALIDATE_BOOL)));
    assert!(!as_bool(&fv(Value::Int(0), FILTER_VALIDATE_BOOL)));
    assert!(is_false(&fv(Value::Int(2), FILTER_VALIDATE_BOOL)));
    assert!(as_bool(&fv(Value::Float(1.0), FILTER_VALIDATE_BOOL)));
    assert!(!as_bool(&fv(Value::Null, FILTER_VALIDATE_BOOL)));
}

#[test]
fn validate_email_cases() {
    for ok in [
        "a@b.com",
        "user@example.com",
        "user.name@example.co.uk",
        "user+tag@example.com",
        "a@b.c",
        "John.Doe@example.com",
        "user@[127.0.0.1]",
        "user@sub.example.com",
        "x@y.z",
        "a!#$%&'*+-/=?^_`{|}~@example.com",
        "\"quoted\"@example.com",
        "user@[IPv6:2001:db8::1]",
        "user.name+tag@sub.domain.com",
        "a@b.co",
        "a@b.c1",
        "a@1b.bc",
        "a@[1.2.3.4]",
    ] {
        assert_eq!(as_str(&fv(s(ok), FILTER_VALIDATE_EMAIL)), ok, "{ok}");
    }
    for bad in [
        "a@b",
        "@b.com",
        "a@",
        "a b@c.com",
        "a@@b.com",
        "a@b@c.com",
        "",
        "user@localhost",
        "foo@bar.",
        "foo@-bar.com",
        "a@b.com ",
        "  a@b.com  ",
        "a@b..com",
        "test@example",
        "unicode@exämple.com",
        "a@b.com\n",
        "user@[::1]",
        // non-bracket domain whose TLD begins with a digit is rejected
        "a@1.2.3.4",
        "a@b.2",
        "a@b.1c",
        "a@12.34",
        "a@99",
        "a@0.0.0.0",
        "a.@b.com",
        ".a@b.com",
        "a@b-.com",
    ] {
        assert!(is_false(&fv(s(bad), FILTER_VALIDATE_EMAIL)), "{bad}");
    }
    // 12 (int) -> "12" has no '@'
    assert!(is_false(&fv(Value::Int(12), FILTER_VALIDATE_EMAIL)));
    // regexp0 allows letters outside ASCII; invalid UTF-8 fails under /u.
    let uni = FILTER_FLAG_EMAIL_UNICODE;
    assert!(is_false(&fv(s("é@b.de"), FILTER_VALIDATE_EMAIL)));
    assert_eq!(
        as_str(&fvf(s("é@b.de"), FILTER_VALIDATE_EMAIL, uni)),
        "é@b.de"
    );
    let bad_utf8 = Value::str(b"\xff@b.de");
    assert!(is_false(&fvf(bad_utf8, FILTER_VALIDATE_EMAIL, uni)));
    // more than 320 octets
    let long = format!("{}@b.com", "a".repeat(315));
    assert!(is_false(&fv(s(&long), FILTER_VALIDATE_EMAIL)));
}

#[test]
fn validate_ip_cases() {
    for ok in [
        "127.0.0.1",
        "0.0.0.0",
        "255.255.255.255",
        "::1",
        "2001:db8::1",
        "fe80::1",
        "::ffff:127.0.0.1",
        "2001:0db8:0000:0000:0000:ff00:0042:8329",
        "::",
        "0:0:0:0:0:0:0:1",
        "1:2:3:4:5:6:7:8",
        "::ffff:0:0",
        "1:2:3:4:5:6:1.2.3.4",
        "::1.2.3.4",
    ] {
        assert_eq!(as_str(&fv(s(ok), FILTER_VALIDATE_IP)), ok, "{ok}");
    }
    for bad in [
        "256.1.1.1",
        "1.2.3",
        "1.2.3.4.5",
        "01.2.3.4",
        "1.2.3.04",
        "abc",
        "",
        "  127.0.0.1",
        "127.0.0.1 ",
        "99999",
        "1.2.3.-4",
        "2001:db8:::1",
        "12345::",
        "1:2:3:4:5:6:7:8:9",
        "g::1",
        "1::2::3",
        "1:2:3:4:5:6:7:1.2.3.4",
    ] {
        assert!(is_false(&fv(s(bad), FILTER_VALIDATE_IP)), "{bad}");
    }
    // flags
    assert!(is_false(&fvf(
        s("::1"),
        FILTER_VALIDATE_IP,
        FILTER_FLAG_IPV4
    )));
    assert_eq!(
        as_str(&fvf(s("127.0.0.1"), FILTER_VALIDATE_IP, FILTER_FLAG_IPV4)),
        "127.0.0.1"
    );
    assert!(is_false(&fvf(
        s("127.0.0.1"),
        FILTER_VALIDATE_IP,
        FILTER_FLAG_IPV6
    )));
    assert_eq!(
        as_str(&fvf(s("::1"), FILTER_VALIDATE_IP, FILTER_FLAG_IPV6)),
        "::1"
    );
}

#[test]
fn validate_url_cases() {
    for ok in [
        "http://example.com",
        "https://example.com/path?q=1#frag",
        "ftp://a.b/c",
        "http://example.com:8080/a",
        "mailto:a@b.com",
        "http://localhost",
        "http://127.0.0.1",
        "http://user:pass@host.com/p",
        "HTTP://Example.COM",
        "custom://foo/bar",
        "a://b",
        "http://[::1]/x",
        "file:///etc/passwd",
        "news:comp.lang.php",
        "http://a./x",
        "http://a.b./x",
        "ftp://a..b",
    ] {
        assert_eq!(as_str(&fv(s(ok), FILTER_VALIDATE_URL)), ok, "{ok}");
    }
    for bad in [
        "http://",
        "example.com",
        "www.example.com",
        "//example.com/path",
        "",
        "http:///path",
        "http://ex ample.com",
        "  http://a.com",
        "http://a.com\n",
        "urn:isbn:096",
        "http://a_b.com",
        "http://a..b/x",
        "http://.a/x",
        "http://-a.b/x",
        "http://a-.b/x",
        "http://a.b..c",
    ] {
        assert!(is_false(&fv(s(bad), FILTER_VALIDATE_URL)), "{bad}");
    }
    // path/query required
    assert!(is_false(&fvf(
        s("http://example.com"),
        FILTER_VALIDATE_URL,
        FILTER_FLAG_PATH_REQUIRED
    )));
    assert_eq!(
        as_str(&fvf(
            s("http://example.com/path"),
            FILTER_VALIDATE_URL,
            FILTER_FLAG_PATH_REQUIRED
        )),
        "http://example.com/path"
    );
    assert_eq!(
        as_str(&fvf(
            s("http://example.com/p?x=1"),
            FILTER_VALIDATE_URL,
            FILTER_FLAG_QUERY_REQUIRED
        )),
        "http://example.com/p?x=1"
    );
    assert!(is_false(&fvf(
        s("http://example.com/p"),
        FILTER_VALIDATE_URL,
        FILTER_FLAG_QUERY_REQUIRED
    )));
}

#[test]
fn validate_domain_cases() {
    for ok in [
        "example.com",
        "localhost",
        "a.b.c",
        "-bad.com",
        "bad-.com",
        "ex_ample.com",
        "www.example.com",
        "123.com",
        "",
        "a",
        "exämple.com",
        "a-b.c-d.com",
        "sub.domain.co.uk",
        "a.",
        "a.b.",
    ] {
        assert_eq!(as_str(&fv(s(ok), FILTER_VALIDATE_DOMAIN)), ok, "{ok}");
    }
    for bad in ["a..b", ".a", "..", "a...b", "."] {
        assert!(is_false(&fv(s(bad), FILTER_VALIDATE_DOMAIN)), "{bad}");
    }
    // length
    assert_eq!(
        as_str(&fv(s(&"a".repeat(63)), FILTER_VALIDATE_DOMAIN)),
        "a".repeat(63)
    );
    assert!(is_false(&fv(s(&"a".repeat(64)), FILTER_VALIDATE_DOMAIN)));
    // hostname flag
    assert_eq!(
        as_str(&fvf(
            s("localhost"),
            FILTER_VALIDATE_DOMAIN,
            FILTER_FLAG_HOSTNAME
        )),
        "localhost"
    );
    assert!(is_false(&fvf(
        s("-bad.com"),
        FILTER_VALIDATE_DOMAIN,
        FILTER_FLAG_HOSTNAME
    )));
    assert!(is_false(&fvf(
        s("ex_ample.com"),
        FILTER_VALIDATE_DOMAIN,
        FILTER_FLAG_HOSTNAME
    )));
    assert_eq!(
        as_str(&fvf(s("a-b"), FILTER_VALIDATE_DOMAIN, FILTER_FLAG_HOSTNAME)),
        "a-b"
    );
}

#[test]
fn sanitize_cases() {
    // NUMBER_INT
    assert_eq!(
        as_str(&fv(s("abc123def"), FILTER_SANITIZE_NUMBER_INT)),
        "123"
    );
    assert_eq!(
        as_str(&fv(s("+12-34"), FILTER_SANITIZE_NUMBER_INT)),
        "+12-34"
    );
    assert_eq!(as_str(&fv(s("  -5  "), FILTER_SANITIZE_NUMBER_INT)), "-5");
    assert_eq!(
        as_str(&fv(s("$1,234.50"), FILTER_SANITIZE_NUMBER_INT)),
        "123450"
    );
    assert_eq!(
        as_str(&fv(Value::Int(123), FILTER_SANITIZE_NUMBER_INT)),
        "123"
    );
    // EMAIL
    assert_eq!(as_str(&fv(s("a@b.com"), FILTER_SANITIZE_EMAIL)), "a@b.com");
    assert_eq!(
        as_str(&fv(s("john doe@ex ample.com"), FILTER_SANITIZE_EMAIL)),
        "johndoe@example.com"
    );
    // URL
    assert_eq!(
        as_str(&fv(s("http://ex ample.com/a b?x=<1>"), FILTER_SANITIZE_URL)),
        "http://example.com/ab?x=<1>"
    );
    // SPECIAL_CHARS
    assert_eq!(
        as_str(&fv(
            s("<a href=\"x\">&amp; '"),
            FILTER_SANITIZE_SPECIAL_CHARS
        )),
        "&#60;a href=&#34;x&#34;&#62;&#38;amp; &#39;"
    );
    assert_eq!(
        as_str(&fv(s("line\nbreak\ttab"), FILTER_SANITIZE_SPECIAL_CHARS)),
        "line&#10;break&#9;tab"
    );
}

#[test]
fn special_chars_byte_ranges() {
    // byte 127 and high bytes are kept verbatim; 0x1f is encoded.
    fn sc_bytes(byte: u8) -> Vec<u8> {
        let r = fn_filter_var(
            &mut TestHost::new(),
            &[
                Value::str(vec![byte]),
                Value::Int(FILTER_SANITIZE_SPECIAL_CHARS),
            ],
        )
        .unwrap();
        match r {
            Value::Str(s) => s.as_bytes().to_vec(),
            _ => panic!("expected string"),
        }
    }
    assert_eq!(sc_bytes(0x7f), vec![0x7f]);
    assert_eq!(sc_bytes(0xff), vec![0xff]);
    assert_eq!(sc_bytes(0x1f), b"&#31;".to_vec());
}

#[test]
fn default_passthrough() {
    assert_eq!(as_str(&fv(s("abc"), FILTER_DEFAULT)), "abc");
    assert_eq!(as_str(&fv(Value::Int(123), FILTER_DEFAULT)), "123");
    assert_eq!(as_str(&fv(Value::Float(1.5), FILTER_DEFAULT)), "1.5");
    assert_eq!(as_str(&fv(Value::Bool(true), FILTER_DEFAULT)), "1");
    assert_eq!(as_str(&fv(Value::Null, FILTER_DEFAULT)), "");
    assert_eq!(as_str(&fv(s("a<b>c"), FILTER_DEFAULT)), "a<b>c");
    // default filter when omitted
    assert_eq!(
        as_str(&fn_filter_var(&mut TestHost::new(), &[s("hi")]).unwrap()),
        "hi"
    );
}

#[test]
fn array_input_fails() {
    let arr = Value::Array(PhpArray::new());
    assert!(is_false(&fv(arr.clone(), FILTER_VALIDATE_INT)));
    assert!(is_false(&fv(arr.clone(), FILTER_SANITIZE_NUMBER_INT)));
    assert!(is_false(&fv(arr, FILTER_DEFAULT)));
}
