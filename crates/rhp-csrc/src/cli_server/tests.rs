use super::*;

#[test]
fn parses_a_request() {
    let mut p = HttpParser::new();
    assert!(p.at_request_start());
    let mut seen = Vec::new();
    let req = b"POST /a/b?x=1#f HTTP/1.1\r\nHost: h\r\nContent-Length: 3\r\n\r\nabc";
    let n = p.execute(req, &mut |e| {
        seen.push(format!("{e:?}"));
        true
    });
    assert_eq!(n, req.len());
    assert_eq!(p.method(), method::POST);
    assert_eq!(p.protocol_version(), 101);
    assert_eq!(method_str(method::NOT_IMPLEMENTED), "NOTIMPLEMENTED");
    assert_eq!(method_str(method::PATCH), "PATCH");
    let s = seen.join(" ");
    assert!(s.contains("Path([47, 97, 47, 98])"), "{s}");
    assert!(
        s.ends_with("Body([97, 98, 99]) MessageComplete(101)"),
        "{s}"
    );
    assert!(
        s.contains("Url([47, 97, 47, 98, 63, 120, 61, 49, 35, 102], 3)"),
        "{s}"
    );
}

#[test]
fn stops_on_a_malformed_request() {
    let mut p = HttpParser::new();
    let req = b"GET / XTTP/1.1\r\n\r\n";
    assert!(p.execute(req, &mut |_| true) < req.len());
}

#[test]
fn mime_table() {
    let m: Vec<_> = mime_types().collect();
    assert!(m.len() > 1000);
    assert!(m.contains(&(&b"css"[..], &b"text/css"[..])));
}
