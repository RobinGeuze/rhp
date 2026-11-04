use super::*;

fn encrypt(plain: &[u8], key: &[u8], iv: &[u8; 16]) -> String {
    use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
    use base64::Engine as _;
    use hmac::{digest::KeyInit, Mac};
    let b = base64::engine::general_purpose::STANDARD;
    let mut buf = plain.to_vec();
    buf.resize(plain.len() + 16, 0);
    let ct = cbc::Encryptor::<aes::Aes256>::new_from_slices(key, iv)
        .unwrap()
        .encrypt_padded_mut::<Pkcs7>(&mut buf, plain.len())
        .unwrap()
        .to_vec();
    let (iv, value) = (b.encode(iv), b.encode(ct));
    let mut h = hmac::Hmac::<sha2::Sha256>::new_from_slice(key).unwrap();
    h.update(iv.as_bytes());
    h.update(value.as_bytes());
    let mac: String = h
        .finalize()
        .into_bytes()
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect();
    let json = format!(r#"{{"iv":"{iv}","value":"{value}","mac":"{mac}","tag":""}}"#)
        .replace('/', "\\/");
    b.encode(json)
}

#[test]
fn laravel_cookies_decrypt_and_verify() {
    let key = [7u8; 32];
    let prefix = "0123456789abcdef0123456789abcdef01234567|";
    let payload = encrypt(format!("{prefix}sessionid123").as_bytes(), &key, &[1; 16]);
    let plain = laravel_decrypt(payload.as_bytes(), &key).unwrap();
    assert_eq!(strip_cookie_prefix(&plain), b"sessionid123");
    assert_eq!(
        laravel_decrypt(payload.as_bytes(), &[8u8; 32]),
        Err("bad mac")
    );
    assert_eq!(laravel_decrypt(b"plain", &key), Err("not base64"));
}

#[test]
fn parses_cgi_responses() {
    let r = parse_response(b"Status: 404 Not Found\r\nX-Powered-By: PHP/8.5.8\r\nContent-type: text/html; charset=UTF-8\r\n\r\nnope");
    assert_eq!(r.status, 404);
    assert_eq!(r.headers.len(), 3);
    assert_eq!(
        r.headers[2],
        ("Content-type".into(), "text/html; charset=UTF-8".into())
    );
    assert_eq!(r.body, b"nope");
    assert_eq!(parse_response(b"Content-type: x\r\n\r\n").status, 200);
    assert!(parse_response(b"garbage").error.is_some());
}

#[test]
fn a_broken_reference_is_a_harness_error() {
    let req = |id: &str, status: Option<u16>| Request {
        id: id.into(),
        method: "GET".into(),
        uri: "/".into(),
        headers: Vec::new(),
        body: Vec::new(),
        content_type: None,
        flow: None,
        status,
    };
    let ok = |status| {
        Ok(Response {
            status,
            ..Response::default()
        })
    };
    let reqs = [
        req("fine", None),
        req("fine-404", Some(404)),
        req("crashed", None),
        req("wrong-status", Some(419)),
        req("unsent", None),
        req("error-page", None),
    ];
    let resps = [
        ok(200),
        ok(404),
        Ok(Response {
            error: Some("no CGI header block".into()),
            ..Response::default()
        }),
        ok(200),
        Err("no cookie \"XSRF-TOKEN\" in the jar".into()),
        ok(500),
    ];
    let bad = reference_failures(&reqs, &resps);
    let ids: Vec<&str> = bad.iter().map(|b| b.split(' ').next().unwrap()).collect();
    assert_eq!(ids, ["crashed", "wrong-status", "unsent", "error-page"]);
    assert!(
        bad[1].ends_with("status 200 (the corpus expects 419)"),
        "{bad:?}"
    );
}

#[test]
fn cookie_jar_sets_and_expires() {
    let mut jar = Jar::default();
    let set = |v: &str| vec![("Set-Cookie".to_string(), v.to_string())];
    jar.update(&set("a=1; path=/"));
    jar.update(&set(
        "b=x%3D; expires=Sun, 01 Nov 2026 12:00:00 GMT; Max-Age=7200",
    ));
    assert_eq!(jar.header(), "a=1; b=x%3D");
    jar.update(&set(
        "a=deleted; expires=Thu, 01 Jan 1970 00:00:01 GMT; Max-Age=0",
    ));
    assert_eq!(jar.header(), "b=x%3D");
    assert_eq!(expand("t={cookie:b}", &jar, None).unwrap(), b"t=x=");
    assert!(expand("{cookie:zz}", &jar, None).is_err());
}

#[test]
fn tokens_rename_consistently_and_rules_are_named() {
    let corpus = Corpus {
        state: vec![],
        warmup: Warmup::None,
        warmup_clear: vec![],
        warmup_drop: vec![],
        ini: vec![],
        tokens: vec![regex(r"sid=([A-Za-z0-9]+)").unwrap()],
        masks: vec![],
        laravel_key: None,
        requests: vec![],
    };
    let mut n = Normalizer::new(&corpus, Some(("8.5.9".into(), "8.5.8".into())));
    let r = Response {
        status: 200,
        headers: vec![
            ("X-Powered-By".into(), "PHP/8.5.9".into()),
            (
                "Set-Cookie".into(),
                "sid=abcdef123456; expires=Sun, 01 Nov 2026 12:00:00 GMT; path=/".into(),
            ),
            ("Date".into(), "Sun, 01 Nov 2026 12:00:00 GMT".into()),
        ],
        body: b"token abcdef123456 here".to_vec(),
        stderr: b"[2026-11-01T12:00:00.123+00:00] app.ERROR: x".to_vec(),
        error: None,
    };
    let out = n.response(&r);
    assert_eq!(out.headers[0].1, "PHP/8.5.8");
    assert_eq!(out.headers[1].1, "sid=<TOK1>; expires=<DATE>; path=/");
    assert_eq!(out.headers[2].1, "<HTTPDATE>");
    assert_eq!(out.body, b"token <TOK1> here");
    assert_eq!(out.stderr, b"[<TIMESTAMP>] app.ERROR: x");
    let fired: Vec<&str> = n.fired.iter().map(String::as_str).collect();
    assert_eq!(
        fired,
        [
            "cookie-expires",
            "date-header",
            "log-timestamp",
            "php-version",
            "token-rename"
        ]
    );
}

#[test]
fn snapshot_restore_round_trips() {
    let dir = std::env::temp_dir().join(format!("rhp-app-snap-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("var/cache/x")).unwrap();
    fs::write(dir.join("var/cache/x/a.php"), "A").unwrap();
    let state = vec!["var/cache".to_string(), "var/missing".to_string()];
    let snap = take_snapshot(&dir, &state);
    fs::write(dir.join("var/cache/x/a.php"), "changed").unwrap();
    fs::write(dir.join("var/cache/new.php"), "N").unwrap();
    let after = take_snapshot(&dir, &state);
    let w = written_files(&snap, &after);
    assert_eq!(w.len(), 2);
    restore_snapshot(&dir, &state, &snap).unwrap();
    assert_eq!(
        fs::read_to_string(dir.join("var/cache/x/a.php")).unwrap(),
        "A"
    );
    assert!(!dir.join("var/cache/new.php").exists());
    assert!(!dir.join("var/missing").exists());
    let _ = fs::remove_dir_all(&dir);
}
