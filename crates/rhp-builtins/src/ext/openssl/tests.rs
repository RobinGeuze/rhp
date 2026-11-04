use super::*;
use crate::test_support::TestHost;

#[test]
fn openssl_is_linked_statically() {
    if env!("RHP_OPENSSL_LINK_MODE") != "static" {
        return;
    }
    let exe = std::env::current_exe().unwrap();
    // Linux: the binary's own NEEDED entries. ldd would also list the
    // libssl and libcrypto that libcurl's shared libssh2 loads.
    let (tool, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("otool", &["-L"])
    } else {
        ("readelf", &["-d"])
    };
    let Ok(out) = std::process::Command::new(tool)
        .args(args)
        .arg(&exe)
        .output()
    else {
        return;
    };
    let deps = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{tool} failed: {deps}");
    assert!(
        !deps.contains("libcrypto") && !deps.contains("libssl"),
        "{deps}"
    );
}

#[test]
fn header_and_library_agree() {
    // "OpenSSL 4.1.0 23 Oct 2026": the release the headers name is the
    // one linked (the date may differ for a patched distro build).
    let lib = library_version();
    let release = |s: &str| s.split_whitespace().nth(1).map(str::to_string);
    assert!(lib.starts_with("OpenSSL "), "{lib}");
    assert_eq!(
        release(&lib),
        release(VERSION_TEXT),
        "{lib} vs {VERSION_TEXT}"
    );
    let n = version_number();
    let v = release(VERSION_TEXT).unwrap_or_default();
    let parts: Vec<i64> = v.split('.').filter_map(|p| p.parse().ok()).collect();
    assert_eq!(n >> 28, parts[0], "{n:x}");
    assert_eq!((n >> 20) & 0xff, parts[1], "{n:x}");
}

#[test]
fn error_ring_holds_the_last_fifteen() {
    ERRORS.with(|c| c.set(None));
    // OpenSSL's own queue holds fewer than 16: move it over each time
    for _ in 0..20 {
        let bio = mem_buf(b"garbage");
        let k = backend::pem_read_bio_public_key(bio.p);
        assert!(k.is_null());
        store_errors();
    }
    let mut n = 0;
    let mut host = TestHost::new();
    while let Ok(Value::Str(_)) = fn_error_string(&mut host, &[]) {
        n += 1;
    }
    assert_eq!(n, ERR_BUFFER_SIZE - 1);
}

#[test]
fn a_callback_panic_resumes_after_the_c_call() {
    let r = std::panic::catch_unwind(|| {
        with_callbacks(|| {
            assert_eq!(guard(1, || -> i32 { panic!("in a callback") }), 1);
            // Later callbacks of the same C call do nothing.
            assert_eq!(guard(2, || 3), 2);
        })
    });
    assert!(r.is_err());
    // The slot is clear again.
    assert_eq!(guard(0, || 5), 5);
}
