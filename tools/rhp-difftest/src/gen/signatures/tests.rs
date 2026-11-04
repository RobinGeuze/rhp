use super::*;

#[test]
fn floats_print_like_php_parser() {
    assert_eq!(print_float(1.0), "1.0");
    assert_eq!(print_float(0.5), "0.5");
    assert_eq!(print_float(1e100), "1E+100");
}

#[test]
fn strings_print_like_php_parser() {
    assert_eq!(double_quoted(b" \n\r\t\x0b\0"), "\" \\n\\r\\t\\v\\x00\"");
    assert_eq!(single_quoted(b"a'b\\"), "'a\\'b\\\\'");
}
