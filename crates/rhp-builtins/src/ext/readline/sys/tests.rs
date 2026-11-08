use std::ffi::{c_void, CStr};

use super::*;

/// The file the `readline` rhp calls was loaded from.
fn library_path() -> Option<String> {
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: dladdr fills `info` for an address inside a loaded image.
    let ok = unsafe { libc::dladdr(readline as *const c_void, &mut info) };
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
    let path = library_path().expect("dladdr finds readline");
    if cfg!(target_os = "macos") {
        assert_eq!(path, "/usr/lib/libedit.3.dylib");
    } else if cfg!(target_os = "linux") {
        assert!(path.contains("libedit.so"), "{path}");
    }
    // SAFETY: libedit's constant string.
    let v = unsafe { CStr::from_ptr(rl_library_version) };
    assert_eq!(v.to_bytes(), b"EditLine wrapper");
}
