use super::*;

#[test]
fn strip_headers() {
    let mut bag = b"Content-Type: a\r\nX: y\r\ncontent-length: 3".to_vec();
    let mut lc = bag.to_ascii_lowercase();
    strip_header(&mut bag, &mut lc, b"content-length");
    strip_header(&mut bag, &mut lc, b"content-type");
    assert_eq!(bag, b"X: y");
}

#[test]
fn passwd() {
    assert_eq!(strip_url_passwd(b"http://u:p@h/x"), b"http://...@h/x");
    assert_eq!(strip_url_passwd(b"http://h/x"), b"http://h/x");
}

#[test]
fn has_header() {
    assert!(check_has_header(b"host: x", b"host:"));
    assert!(check_has_header(b"a: b\r\nhost: x", b"host:"));
    assert!(!check_has_header(b"a: host: x", b"host:"));
}
