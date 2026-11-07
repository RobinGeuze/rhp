use super::*;

fn encrypt(plain: &[u8], key: &[u8], iv: &[u8; 16]) -> String {
    use base64::Engine as _;
    use cbc::cipher::{block_padding::Pkcs7, BlockModeEncrypt, KeyIvInit};
    use hmac::{digest::KeyInit, Mac};
    let b = base64::engine::general_purpose::STANDARD;
    let mut buf = plain.to_vec();
    buf.resize(plain.len() + 16, 0);
    let ct = cbc::Encryptor::<aes::Aes256>::new_from_slices(key, iv)
        .unwrap()
        .encrypt_padded::<Pkcs7>(&mut buf, plain.len())
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
    let json =
        format!(r#"{{"iv":"{iv}","value":"{value}","mac":"{mac}","tag":""}}"#).replace('/', "\\/");
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
        pass_status: BTreeMap::from([("debug".to_string(), 500)]),
        in_warmup: true,
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
    let bad = reference_failures(&reqs.iter().collect::<Vec<_>>(), "default", &resps);
    let ids: Vec<&str> = bad.iter().map(|b| b.split(' ').next().unwrap()).collect();
    assert_eq!(ids, ["crashed", "wrong-status", "unsent", "error-page"]);
    assert!(
        bad[1].ends_with("status 200 (the corpus expects 419)"),
        "{bad:?}"
    );
    // `pass_status = { debug = 500 }` overrides the status in that pass.
    let bad = reference_failures(&reqs.iter().collect::<Vec<_>>(), "debug", &resps);
    let ids: Vec<&str> = bad.iter().map(|b| b.split(' ').next().unwrap()).collect();
    assert_eq!(
        ids,
        ["fine", "fine-404", "crashed", "wrong-status", "unsent"]
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
        warmup_clear: vec![],
        warmup_drop: vec![],
        ini: vec![],
        tokens: vec![regex(r"sid=([A-Za-z0-9]+)").unwrap()],
        masks: vec![],
        passes: vec![],
        laravel_key: None,
        requests: vec![],
    };
    let mut n = Normalizer::new(&corpus, "default", Some(("8.5.9".into(), "8.5.8".into())));
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
    fs::create_dir_all(dir.join("config")).unwrap();
    fs::write(dir.join("config/ref.php"), "R").unwrap();
    let state = vec![
        "var/cache".to_string(),
        "var/missing".to_string(),
        "config/ref.php".to_string(),
    ];
    let snap = take_snapshot(&dir, &state);
    fs::write(dir.join("var/cache/x/a.php"), "changed").unwrap();
    fs::write(dir.join("var/cache/new.php"), "N").unwrap();
    fs::write(dir.join("config/ref.php"), "R2").unwrap();
    let after = take_snapshot(&dir, &state);
    let w = written_files(&snap, &after);
    assert_eq!(w.len(), 3);
    assert!(w.contains_key(Path::new("config/ref.php")));
    restore_snapshot(&dir, &state, &snap).unwrap();
    assert_eq!(fs::read_to_string(dir.join("config/ref.php")).unwrap(), "R");
    assert_eq!(
        fs::read_to_string(dir.join("var/cache/x/a.php")).unwrap(),
        "A"
    );
    assert!(!dir.join("var/cache/new.php").exists());
    assert!(!dir.join("var/missing").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_php_path_runs_that_script() {
    let dir = std::env::temp_dir().join(format!("rhp-app-script-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("web")).unwrap();
    fs::write(dir.join("web/index.php"), "").unwrap();
    fs::write(dir.join("web/login.php"), "").unwrap();
    fs::write(dir.join("secret.php"), "").unwrap();
    let app = App {
        name: "t".into(),
        root: dir.clone(),
        docroot: dir.join("web"),
        front: dir.join("web/index.php"),
        script_name: "/index.php".into(),
    };
    let front = (dir.join("web/index.php"), "/index.php".to_string());
    assert_eq!(
        script_for(&app, "/login.php?x=1"),
        (dir.join("web/login.php"), "/login.php".to_string())
    );
    assert_eq!(script_for(&app, "/hello/"), front);
    assert_eq!(script_for(&app, "/missing.php"), front);
    assert_eq!(script_for(&app, "/../secret.php"), front);
    assert_eq!(script_for(&app, "/x?next=/login.php"), front);
    let _ = fs::remove_dir_all(&dir);
}

fn corpus_from(toml: &str) -> Result<Corpus, String> {
    let dir = std::env::temp_dir().join(format!("rhp-app-corpus-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    // Tests run in parallel: each call gets its own file.
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = dir.join(format!("{n}.toml"));
    fs::write(&path, toml).unwrap();
    let app = App {
        name: "t".into(),
        root: dir.clone(),
        docroot: dir.clone(),
        front: dir.join("index.php"),
        script_name: "/index.php".into(),
    };
    let c = load_corpus(&app, &path);
    let _ = fs::remove_file(&path);
    c
}

#[test]
fn passes_parse_and_select() {
    let c = corpus_from(
        r#"
warmup = "corpus"
[[pass]]
name = "debug"
env = { APP_DEBUG = "1" }
only = ["b"]
[[pass]]
name = "cold"
warmup = "none"
clear = ["var/cache"]
[[mask]]
name = "ids"
pattern = "id-\\d+"
pass = ["debug"]
[[request]]
id = "a"
path = "/a"
flow = "f"
[[request]]
id = "b"
path = "/b"
flow = "f"
[[request]]
id = "c"
path = "/c"
"#,
    )
    .unwrap();
    let names: Vec<&str> = c.passes.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["default", "debug", "cold"]);
    let (default, debug, cold) = (&c.passes[0], &c.passes[1], &c.passes[2]);
    assert_eq!(default.warmup, Warmup::Corpus);
    assert_eq!(debug.warmup, Warmup::Corpus);
    assert_eq!(cold.warmup, Warmup::None);
    assert_eq!(cold.clear, ["var/cache"]);
    assert_eq!(debug.env, [("APP_DEBUG".to_string(), "1".to_string())]);
    assert_eq!(default.key("a"), "a");
    assert_eq!(debug.key("@files"), "debug/@files");
    // `only = ["b"]` brings in the rest of b's flow, in corpus order.
    let ids =
        |p: &Pass| -> Vec<String> { p.select(&c.requests).iter().map(|r| r.id.clone()).collect() };
    assert_eq!(ids(debug), ["a", "b"]);
    assert_eq!(ids(cold), ["a", "b", "c"]);
    assert_eq!(c.masks[0].passes, ["debug"]);

    let mut n = Normalizer::new(&c, "debug", None);
    assert_eq!(n.masks("body", b"x id-12".to_vec()), b"x ");
    let mut n = Normalizer::new(&c, "cold", None);
    assert_eq!(n.masks("body", b"x id-12".to_vec()), b"x id-12");

    for bad in [
        "[[pass]]\nname = \"default\"",
        "[[pass]]\nname = \"Debug\"",
        "[[pass]]\nname = \"x\"\nwarmup = \"later\"",
        "[[mask]]\nname = \"m\"\npattern = \"x\"\npass = [\"nope\"]",
    ] {
        assert!(corpus_from(bad).is_err(), "{bad}");
    }
}

#[test]
fn cookie_max_age_is_masked_but_deletion_is_kept() {
    let corpus = corpus_from("").unwrap();
    let mut n = Normalizer::new(&corpus, "default", None);
    let r = Response {
        headers: vec![
            (
                "Set-Cookie".into(),
                "a=b; expires=Wed, 18 May 2033 03:33:20 GMT; Max-Age=205027315; path=/".into(),
            ),
            (
                "Set-Cookie".into(),
                "gone=deleted; expires=Thu, 01 Jan 1970 00:00:01 GMT; Max-Age=0".into(),
            ),
        ],
        ..Response::default()
    };
    let out = n.response(&r);
    assert_eq!(
        out.headers[0].1,
        "a=b; expires=<DATE>; Max-Age=<SECS>; path=/"
    );
    assert_eq!(out.headers[1].1, "gone=deleted; expires=<DATE>; Max-Age=0");
}

#[test]
fn sqlite_files_are_compared_as_dumps() {
    assert_eq!(sqlite_dump(b"not a database"), None);
    let dir = std::env::temp_dir().join(format!("rhp-app-sqlite-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let db = dir.join("t.sqlite");
    let made = Command::new("sqlite3")
        .arg(&db)
        .arg("CREATE TABLE t(k TEXT, v TEXT); INSERT INTO t VALUES('when', '2026-01-01 12:00:00');")
        .status();
    if !made.is_ok_and(|s| s.success()) {
        eprintln!("skipped: no sqlite3");
        let _ = fs::remove_dir_all(&dir);
        return;
    }
    let data = fs::read(&db).unwrap();
    let dump = String::from_utf8(sqlite_dump(&data).unwrap()).unwrap();
    assert!(dump.contains("CREATE TABLE t(k TEXT, v TEXT);"), "{dump}");
    let corpus = corpus_from("").unwrap();
    let mut n = Normalizer::new(&corpus, "default", None);
    let (_, out) = n.file(Path::new("var/t.sqlite"), &Some(data));
    let out = String::from_utf8(out.unwrap()).unwrap();
    assert!(
        out.contains("INSERT INTO t VALUES('when','<TIMESTAMP>');"),
        "{out}"
    );
    assert!(n.fired.contains("sqlite-dump"));
    let _ = fs::remove_dir_all(&dir);
}
