use super::*;

#[test]
fn wildcard_names() {
    assert!(matches_wildcard_name(b"a.example.com", b"*.example.com"));
    assert!(!matches_wildcard_name(b"a.b.example.com", b"*.example.com"));
    assert!(matches_wildcard_name(b"Example.COM", b"example.com"));
    assert!(matches_wildcard_name(
        b"foo-bar.example.com",
        b"foo*.example.com"
    ));
    assert!(!matches_wildcard_name(b"example.com", b"*.example.com"));
    assert!(!matches_wildcard_name(b"a.example.com", b"a.*.com"));
}

#[test]
fn alpn_parse() {
    assert_eq!(
        alpn_protos_parse(b"h2,http/1.1").unwrap(),
        b"\x02h2\x08http/1.1"
    );
    assert_eq!(alpn_protos_parse(b"").unwrap(), b"\x00");
}

#[test]
fn url_names() {
    assert_eq!(
        url_name(b"127.0.0.1:443").as_deref(),
        Some(&b"127.0.0.1"[..])
    );
    assert_eq!(
        url_name(b"example.com.:443").as_deref(),
        Some(&b"example.com"[..])
    );
    assert_eq!(url_name(b"[::1]:443"), None);
}
