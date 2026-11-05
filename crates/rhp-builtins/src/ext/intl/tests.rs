use super::*;

#[test]
fn uidna_info_layout_matches_the_headers() {
    // SAFETY: a plain getter.
    let size = unsafe { sys::rhp_intl_uidna_info_size() };
    assert_eq!(size, std::mem::size_of::<sys::UIDNAInfo>());
}

#[test]
fn constants_come_from_the_linked_icu() {
    let c = constants();
    let get = |n: &str| {
        c.iter()
            .find(|(k, _)| *k == n)
            .map(|(_, v)| rhp_values::conv::to_string_bytes(v))
    };
    assert_eq!(
        get("INTL_ICU_VERSION").as_deref(),
        Some(env!("RHP_U_ICU_VERSION").as_bytes())
    );
    assert_eq!(get("U_ZERO_ERROR").as_deref(), Some(&b"0"[..]));
    assert_eq!(get("INTL_MAX_LOCALE_LEN").as_deref(), Some(&b"156"[..]));
    assert_eq!(c.len(), 170);
    assert_eq!(u_error_name(1), "U_ILLEGAL_ARGUMENT_ERROR");
}

#[test]
fn icu_is_linked_statically() {
    if env!("RHP_ICU_UC_LINK_MODE") != "static" {
        return;
    }
    // Make sure the test binary pulls in ICU.
    assert!(!u_error_name(0).is_empty());
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
    // Not "libicu": Homebrew's libcurl uses macOS's own libicucore, as
    // php's does.
    for lib in ["libicuuc", "libicui18n", "libicudata"] {
        assert!(!deps.contains(lib), "{deps}");
    }
}
