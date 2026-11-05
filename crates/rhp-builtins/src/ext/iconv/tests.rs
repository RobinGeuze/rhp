use super::*;
use crate::test_support::run;

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
