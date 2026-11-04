use super::*;

#[test]
fn c_layouts_match() {
    assert_eq!(sys::c_layout(), &sys::rust_layout()[..]);
}

#[test]
fn constants_come_from_curl_h() {
    let c = sys::php_constants();
    let get = |n: &str| c.iter().find(|(k, _)| *k == n).map(|(_, v)| *v);
    assert_eq!(get("CURLOPT_URL"), Some(10002));
    assert_eq!(get("CURLOPT_RETURNTRANSFER"), Some(19913));
    assert_eq!(get("CURLOPT_SAFE_UPLOAD"), Some(-1));
    assert_eq!(get("CURLE_OK"), Some(0));
    assert_eq!(sys::internal("CURLOPT_ERRORBUFFER"), 10010);
    assert_eq!(sys::internal("CURL_ERROR_SIZE"), 256);
    assert_eq!(constants().len(), c.len());
}

#[test]
fn libcurl_is_linked_statically() {
    if env!("RHP_CURL_LINK_MODE") != "static" {
        return;
    }
    global_init();
    let d = unsafe { &*sys::curl_version_info(sys::php("CURLVERSION_NOW") as _) };
    assert_eq!(
        cstr_bytes(d.version),
        env!("RHP_LIBCURL_VERSION").as_bytes(),
        "the linked libcurl is not the one whose headers rhp compiled against"
    );
    let exe = std::env::current_exe().unwrap();
    let (tool, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("otool", &["-L"])
    } else {
        ("ldd", &[])
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
    assert!(!deps.contains("libcurl"), "{deps}");
}
