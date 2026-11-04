use super::*;

fn p(s: &str) -> Option<Url> {
    parse(s.as_bytes())
}

#[test]
fn shapes() {
    let u = p("http://user:pw@example.com:8080/a/b?x=1#frag").unwrap();
    assert_eq!(u.scheme.as_deref(), Some(&b"http"[..]));
    assert_eq!(u.user.as_deref(), Some(&b"user"[..]));
    assert_eq!(u.pass.as_deref(), Some(&b"pw"[..]));
    assert_eq!(u.host.as_deref(), Some(&b"example.com"[..]));
    assert_eq!(u.port, 8080);
    assert_eq!(u.path.as_deref(), Some(&b"/a/b"[..]));
    assert_eq!(u.query.as_deref(), Some(&b"x=1"[..]));
    assert_eq!(u.fragment.as_deref(), Some(&b"frag"[..]));
    let u = p("127.0.0.1:443").unwrap();
    assert_eq!(u.host.as_deref(), Some(&b"127.0.0.1"[..]));
    assert_eq!(u.port, 443);
    // no scheme: php takes a bracketed host as a path
    let u = p("[::1]:443").unwrap();
    assert_eq!(u.path.as_deref(), Some(&b"[::1]:443"[..]));
    let u = p("http://[::1]:443/x").unwrap();
    assert_eq!(u.host.as_deref(), Some(&b"[::1]"[..]));
    assert_eq!(u.port, 443);
    assert!(p(":80").is_none());
    assert!(p("http://host:99999").is_none());
    let u = p("http://h?").unwrap();
    assert_eq!(u.query.as_deref(), Some(&b""[..]));
    assert!(u.path.is_none());
    let u = p("/p").unwrap();
    assert_eq!(u.path.as_deref(), Some(&b"/p"[..]));
    let u = p("").unwrap();
    assert_eq!(u.path.as_deref(), Some(&b""[..]));
    // ZEND_STRTOL: white space and a sign before the port
    assert_eq!(p("http://h: 80/").unwrap().port, 80);
    assert_eq!(p("http://h:+80/").unwrap().port, 80);
    assert_eq!(p("http://h:-0/").unwrap().port, 0);
    assert!(p("http://h:-1/").is_none());
    assert!(p("http://h: /").is_none());
}
