use super::*;

#[test]
fn normalizes_like_php() {
    assert_eq!(normalize_protected(b" a b.c[ x ][y]z"), b"a_b_c[x ][y]");
    assert_eq!(normalize_protected(b"f[tmp_name]"), b"f[tmp_name]");
    assert_eq!(normalize_protected(b"plain"), b"plain");
}

#[test]
fn getword_conf_unquotes() {
    assert_eq!(getword_conf(b" \"a\\\"b\" "), b"a\"b");
    assert_eq!(getword_conf(b"abc def"), b"abc");
}

#[test]
fn content_type_dup_lowercases_the_type_only() {
    assert_eq!(
        content_type_dup(b"Multipart/Form-Data; Boundary=AbC"),
        b"multipart/form-data; Boundary=AbC"
    );
}

#[test]
fn quantity_errors() {
    assert!(quantity_error(b"128M").is_none());
    // Messages captured from php 8.5.12 (ini_set('memory_hard_limit_percent', …)).
    let range = |v: &str| {
        format!("Invalid quantity \"{v}\": value is out of range, using overflow result for backwards compatibility")
    };
    for v in [
        "99999999999999999999",
        "9223372036854775808",
        "-9223372036854775809",
        "9007199254740992G",
        "0x8000000000000000",
        "-9007199254740993G",
    ] {
        assert_eq!(quantity_error(v.as_bytes()), Some(range(v)), "{v}");
    }
    for v in [
        "9223372036854775807",
        "-9223372036854775808",
        "8589934591G",
        "-8589934592G",
        "0x7fffffffffffffff",
    ] {
        assert_eq!(quantity_error(v.as_bytes()), None, "{v}");
    }
    assert!(quantity_error(b"1GB")
        .unwrap()
        .contains("unknown multiplier \"B\""));
}
