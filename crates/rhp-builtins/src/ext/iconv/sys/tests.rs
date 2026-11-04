use super::*;

/// The file the `iconv_open` rhp calls was loaded from.
fn library_path() -> Option<String> {
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: dladdr fills `info` for an address inside a loaded image.
    let ok = unsafe { libc::dladdr(iconv_open as *const c_void, &mut info) };
    if ok == 0 || info.dli_fname.is_null() {
        return None;
    }
    // SAFETY: dladdr returned a NUL-terminated path.
    Some(
        unsafe { CStr::from_ptr(info.dli_fname) }
            .to_string_lossy()
            .into_owned(),
    )
}

#[test]
fn links_the_library_php_links() {
    let path = library_path().expect("dladdr finds iconv_open");
    if cfg!(target_os = "macos") {
        assert_eq!(path, "/usr/lib/libiconv.2.dylib");
        assert_eq!(implementation(), "libiconv");
    } else if cfg!(all(target_os = "linux", target_env = "gnu")) {
        assert!(path.contains("libc.so"), "{path}");
        assert_eq!(implementation(), "glibc");
    }
    assert!(version().contains('.'), "{}", version());
}
