use super::*;

// shim.c: the header's version.
extern "C" {
    fn rhp_gmp_header_version() -> *const c_char;
}

/// The version of the gmp.h rhp was compiled against.
fn header_version() -> &'static str {
    // SAFETY: a string literal in shim.c.
    unsafe { CStr::from_ptr(rhp_gmp_header_version()) }
        .to_str()
        .unwrap_or("")
}

#[test]
fn runtime_library_matches_the_header() {
    assert_eq!(version(), header_version());
}

/// Dynamically, from the library php links (LGPL: never statically).
#[test]
fn links_libgmp_dynamically() {
    assert_eq!(env!("RHP_GMP_LINK_MODE"), "dynamic");
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: dladdr fills `info` for an address inside a loaded image.
    let ok = unsafe { libc::dladdr(mpz_init as *const c_void, &mut info) };
    assert!(ok != 0 && !info.dli_fname.is_null());
    // SAFETY: dladdr returned a NUL-terminated path.
    let path = unsafe { CStr::from_ptr(info.dli_fname) }.to_string_lossy();
    assert!(path.contains("libgmp."), "{path}");
    if cfg!(target_os = "macos") {
        // Homebrew's gmp (opt/gmp or its Cellar keg).
        assert!(
            path.contains("/gmp/") && path.ends_with("/libgmp.10.dylib"),
            "{path}"
        );
    }
}

#[test]
fn round_trips_and_macros() {
    let mut z = Mpz::new();
    assert!(z.set_str(b"-123456789012345678901234567890", 10));
    assert_eq!(z.sgn(), -1);
    assert_eq!(z.to_str(10), b"-123456789012345678901234567890");
    assert_eq!(z.to_str(-16), b"-18EE90FF6C373E0EE4E3F0AD2");
    assert!(z.cmp_si(0) < 0);
    assert!(!z.set_str(b"12x", 10));
    assert_eq!(Mpz::from_i64(i64::MIN).get_si(), i64::MIN);
    assert_eq!(Mpz::new().to_str(10), b"0");
    assert_eq!(Mpz::from_i64(0).sgn(), 0);
}
