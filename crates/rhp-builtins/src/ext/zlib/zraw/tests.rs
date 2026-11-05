use std::ffi::{c_void, CStr};

use super::*;

/// rhp calls the system zlib php links, not a copy built into the binary
/// (libz-sys's bundled fallback, which build.rs refuses).
#[test]
fn links_the_library_php_links() {
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: dladdr fills `info` for an address inside a loaded image.
    let ok = unsafe { libc::dladdr(z::zlibVersion as *const c_void, &mut info) };
    assert!(
        ok != 0 && !info.dli_fname.is_null(),
        "dladdr finds zlibVersion"
    );
    // SAFETY: dladdr returned a NUL-terminated path.
    let path = unsafe { CStr::from_ptr(info.dli_fname) }.to_string_lossy();
    if cfg!(target_os = "macos") {
        assert_eq!(path, "/usr/lib/libz.1.dylib");
    } else {
        assert!(path.contains("libz.so"), "{path}");
    }
}
