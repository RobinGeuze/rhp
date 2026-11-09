use super::*;

/// A host whose ini has the default prompt.
fn valid(code: &str) -> (bool, String) {
    let mut h = crate::test_support::TestHost::new();
    let (ok, p) = is_valid_code(&mut h, code.as_bytes());
    (ok, String::from_utf8(p).unwrap())
}

#[test]
fn statement_ends() {
    assert_eq!(valid("echo 1;\n"), (true, "php > ".into()));
    assert_eq!(valid("echo 'a\n"), (false, "php ' ".into()));
    assert_eq!(valid("echo \"a\n"), (false, "php \" ".into()));
    assert_eq!(valid("if (1) {\n"), (false, "php { ".into()));
    assert_eq!(valid("f(\n"), (false, "php ( ".into()));
    assert_eq!(valid("/* x\n"), (false, "/*  > ".into()));
    assert_eq!(valid("echo <<<FOO\nbar\n"), (false, "<<< > ".into()));
    assert_eq!(valid("echo <<<FOO\nbar\nFOO;\n"), (true, "php > ".into()));
    assert_eq!(valid("echo <<<FOO\n  FOOL\n"), (false, "<<< > ".into()));
    assert_eq!(valid("?>\n"), (true, "    > ".into()));
    assert_eq!(valid("#[Attr]\n"), (false, "php > ".into()));
}
