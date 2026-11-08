use super::*;

#[test]
fn normalizes_paths() {
    let n = |s: &str| String::from_utf8(normalize_vpath(s.as_bytes())).unwrap();
    assert_eq!(n("/a/b"), "/a/b");
    assert_eq!(n("//a//b/"), "/a/b/");
    assert_eq!(n("/a/./b"), "/a/b");
    assert_eq!(n("/a/../../b"), "/b");
    assert_eq!(n("/a/%2e%2e/b"), "/b");
    assert_eq!(n("/x%20y"), "/x y");
}

#[test]
fn parses_addresses() {
    assert_eq!(parse_addr("localhost:0"), Some(("localhost".into(), 0)));
    assert_eq!(parse_addr("[::1]:8080"), Some(("::1".into(), 8080)));
    assert_eq!(parse_addr("localhost"), None);
    assert_eq!(parse_addr("h:99999"), None);
    assert_eq!(parse_addr("h:1x"), None);
}

#[test]
fn escapes_uris() {
    assert_eq!(escape_html(b"/<a>&amp;'\""), b"/&lt;a&gt;&amp;&#039;&quot;");
    assert_eq!(escape_html(b"/&x"), b"/&amp;x");
    assert_eq!(escape_html(b"/\xff"), b"");
}
