use super::ini_define_value as v;

#[test]
fn ini_define_values_match_php() {
    assert_eq!(v("On"), "1");
    assert_eq!(v("on "), "1");
    assert_eq!(v("TRUE"), "1");
    assert_eq!(v("true;x"), "1");
    assert_eq!(v("off"), "");
    assert_eq!(v("NULL"), "");
    assert_eq!(v("none"), "");
    assert_eq!(v("\"On\""), "On");
    assert_eq!(v("'On'"), "On");
    assert_eq!(v(r#""kill \$\$""#), "kill $$");
    assert_eq!(v(r#""a\"b\\c\nd""#), r#"a"b\c\nd"#);
    assert_eq!(v(""), "");
    assert_eq!(v("-1"), "-1");
    assert_eq!(v("128M"), "128M");
    assert_eq!(v("/a;b"), "/a;b");
    // Constants and the ini expression operators.
    assert_eq!(v("E_ALL"), "30719");
    assert_eq!(v("E_ALL & ~E_DEPRECATED"), "22527");
    assert_eq!(v("E_ALL^E_NOTICE"), "30711");
    assert_eq!(v("E_ALL & ~(E_NOTICE | E_WARNING)"), "30709");
    assert_eq!(v("PHP_INT_SIZE"), "8");
    // Extension constants don't exist yet when php parses `-d`.
    assert_eq!(v("SORT_DESC"), "SORT_DESC");
    assert_eq!(v("M_PI"), "M_PI");
    // A value starting with whitespace is literal.
    assert_eq!(v(" E_ALL"), " E_ALL");
    assert_eq!(v("files"), "files");
    assert_eq!(v("Europe/London"), "Europe/London");
    assert_eq!(v("UTF-8"), "UTF-8");
}
