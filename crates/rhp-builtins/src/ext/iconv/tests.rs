use super::*;
use crate::diag_test::run;

fn s(b: &[u8]) -> Value {
    Value::str(b.to_vec())
}

/// `(result-as-debug, diagnostics)`.
fn go(f: crate::NativeFn, args: &[Value]) -> (String, Vec<String>) {
    let (r, d) = run(f, args);
    let r = match r {
        Ok(Value::Bool(b)) => format!("bool({b})"),
        Ok(Value::Int(i)) => format!("int({i})"),
        Ok(Value::Str(b)) => format!("string({:?})", String::from_utf8_lossy(b.as_bytes())),
        Ok(_) => "other".into(),
        Err(Thrown::Error { class, message }) => format!("{class}: {message}"),
        Err(_) => "thrown".into(),
    };
    (r, d)
}

#[test]
fn links_the_library_php_links() {
    let path = sys::library_path().expect("dladdr finds iconv_open");
    if cfg!(target_os = "macos") {
        assert_eq!(path, "/usr/lib/libiconv.2.dylib");
        assert_eq!(sys::implementation(), "libiconv");
    } else if cfg!(all(target_os = "linux", target_env = "gnu")) {
        assert!(path.contains("libc.so"), "{path}");
        assert_eq!(sys::implementation(), "glibc");
    }
    assert!(sys::version().contains('.'), "{}", sys::version());
}

#[test]
fn strlen_diagnostics() {
    let ill = "Notice: iconv_strlen(): Detected an illegal character in input string";
    assert_eq!(
        go(fn_iconv_strlen, &[s(b"h\xe9llo")]),
        ("bool(false)".into(), vec![ill.into()])
    );
    assert_eq!(
        go(fn_iconv_strlen, &[s(b"h\xc3")]).1,
        vec!["Notice: iconv_strlen(): Detected an incomplete multibyte character in input string"]
    );
    assert_eq!(
        go(fn_iconv_strlen, &[s(b"abc"), s(b"bogus")]).1,
        vec!["Warning: iconv_strlen(): Wrong encoding, conversion from \"bogus\" to \"UCS-4LE\" is not allowed"]
    );
    assert_eq!(
        go(fn_iconv_strlen, &[s(b"abc"), s(&[b'x'; 64])]).1,
        vec!["Warning: iconv_strlen(): Encoding parameter exceeds the maximum allowed length of 64 characters"]
    );
    assert_eq!(
        go(fn_iconv_strlen, &[s("aé".as_bytes())]),
        ("int(2)".into(), vec![])
    );
}

#[test]
fn strpos_and_substr() {
    assert_eq!(
        go(fn_iconv_strpos, &[s("aéb".as_bytes()), s(b"b")]).0,
        "int(2)"
    );
    assert_eq!(go(fn_iconv_strrpos, &[s(b"abab"), s(b"b")]).0, "int(3)");
    assert_eq!(
        go(
            fn_iconv_substr,
            &[s("aéb".as_bytes()), Value::Int(1), Value::Int(1)]
        )
        .0,
        "string(\"é\")"
    );
    assert_eq!(
        go(fn_iconv_strpos, &[s(b"\xff"), s(b"a"), Value::Int(3)]).0,
        "ValueError: iconv_strpos(): Argument #3 ($offset) must be contained in argument #1 ($haystack)"
    );
}

#[test]
fn mime_round_trip() {
    let mut opts = PhpArray::new();
    opts.insert(ArrayKey::from_bytes(b"scheme"), Value::str("Q"));
    let (r, d) = go(
        fn_iconv_mime_encode,
        &[s(b"Subject"), s("Prüfung".as_bytes()), Value::Array(opts)],
    );
    assert_eq!(
        (r.as_str(), d.len()),
        ("string(\"Subject: =?UTF-8?Q?Pr=C3=BCfung?=\")", 0)
    );
    assert_eq!(
        go(
            fn_iconv_mime_decode,
            &[s(b"Subject: =?UTF-8?Q?Pr=C3=BCfung?=")]
        )
        .0,
        "string(\"Subject: Prüfung\")"
    );
    let (r, _) = run(
        fn_iconv_mime_decode_headers,
        &[s(b"A: 1\r\nB: 2\r\nA: 3\r\n")],
    );
    let Ok(Value::Array(a)) = r else {
        panic!("not an array")
    };
    assert_eq!(a.len(), 2);
}

#[test]
fn unknown_error_prints_php_errno() {
    reset_state();
    assert_eq!(
        go(fn_iconv_mime_decode, &[s(b"=?UTF-8?Q?=ZZ?=")]).1,
        vec!["Notice: iconv_mime_decode(): Unknown error (0)"]
    );
    let _ = go(fn_iconv, &[s(b"UTF-8"), s(b"ASCII"), s(b"\xc3")]);
    assert_eq!(
        go(fn_iconv_mime_decode, &[s(b"=?UTF-8?Q?=ZZ?=")]).1,
        vec![format!(
            "Notice: iconv_mime_decode(): Unknown error ({EINVAL})"
        )]
    );
    reset_state();
}
