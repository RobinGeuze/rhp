use super::*;
use std::mem::{offset_of, size_of};

/// libzip has no static archive of php's release (libraries.md), so it
/// is the shared library the build found: php's.
#[test]
fn libzip_is_the_library_php_links() {
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::dladdr(zip_open as *const c_void, &mut info) };
    assert!(
        ok != 0 && !info.dli_fname.is_null(),
        "dladdr finds zip_open"
    );
    let path = unsafe { std::ffi::CStr::from_ptr(info.dli_fname) }
        .to_string_lossy()
        .into_owned();
    let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.into());
    // Exported for a dynamic link (RHP_LIBZIP_LINK=static has none).
    if let Some(libdir) = option_env!("RHP_LIBZIP_LIBDIR") {
        let dir = canon(std::path::Path::new(&path))
            .parent()
            .map(std::path::Path::to_path_buf);
        assert_eq!(
            dir,
            Some(canon(std::path::Path::new(libdir))),
            "zip_open comes from {path}"
        );
    }
    assert_eq!(
        unsafe { cstr(zip_libzip_version()) },
        env!("RHP_LIBZIP_VERSION").as_bytes(),
        "the library is the release of the headers"
    );
}

#[test]
fn layouts_match_zip_h() {
    let want = [
        size_of::<zip_stat_t>(),
        offset_of!(zip_stat_t, valid),
        offset_of!(zip_stat_t, name),
        offset_of!(zip_stat_t, index),
        offset_of!(zip_stat_t, size),
        offset_of!(zip_stat_t, comp_size),
        offset_of!(zip_stat_t, mtime),
        offset_of!(zip_stat_t, crc),
        offset_of!(zip_stat_t, comp_method),
        offset_of!(zip_stat_t, encryption_method),
        size_of::<zip_error_t>(),
        offset_of!(zip_error_t, zip_err),
        offset_of!(zip_error_t, sys_err),
        offset_of!(zip_error_t, str_),
    ];
    assert_eq!(c_layout(), want);
}
