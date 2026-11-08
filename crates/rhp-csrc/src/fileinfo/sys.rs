//! libmagic's API (magic.h) under its rhp_ names, and the hook table
//! glue/fileinfo.c calls.

use std::ffi::{c_char, c_int, c_void};

#[repr(C)]
pub struct MagicSet {
    _private: [u8; 0],
}

/// glue/fileinfo.c's `struct rhp_fi_stat`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Stat {
    pub mode: u32,
    pub size: i64,
    pub dev: i64,
    pub ino: i64,
    pub nlink: i64,
    pub uid: i64,
    pub gid: i64,
    pub rdev: i64,
    pub atime: i64,
    pub mtime: i64,
    pub ctime: i64,
    pub blksize: i64,
    pub blocks: i64,
}

/// glue/fileinfo.c's `struct rhp_fi_hooks`; the comments there say what
/// each returns.
#[repr(C)]
pub struct Hooks {
    pub open: unsafe extern "C" fn(*const c_char, *const c_char, c_int) -> *mut c_void,
    pub opendir: unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void,
    pub close: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub read: unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> isize,
    pub write: unsafe extern "C" fn(*mut c_void, *const c_char, usize) -> isize,
    pub get_line: unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> isize,
    pub readdir: unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> isize,
    pub stat: unsafe extern "C" fn(*mut c_void, *mut Stat) -> c_int,
    pub stat_path: unsafe extern "C" fn(*const c_char, *mut Stat) -> c_int,
    pub fd: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub error: unsafe extern "C" fn(c_int, *const c_char, usize),
    pub pcre_compile: unsafe extern "C" fn(*const c_char, usize) -> *mut c_void,
    pub pcre_match:
        unsafe extern "C" fn(*mut c_void, *const c_char, usize, *mut usize, *mut usize) -> c_int,
    pub pcre_test: unsafe extern "C" fn(*mut c_void, *const c_char, usize, usize) -> c_int,
    #[allow(clippy::type_complexity)]
    pub pcre_replace: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        usize,
        *const c_char,
        usize,
        usize,
        *mut usize,
        *mut usize,
    ) -> *mut c_char,
}

extern "C" {
    pub fn rhp_fi_set_hooks(h: *const Hooks);
    pub fn rhp_magic_open(flags: c_int) -> *mut MagicSet;
    pub fn rhp_magic_close(ms: *mut MagicSet);
    pub fn rhp_magic_load(ms: *mut MagicSet, file: *const c_char) -> c_int;
    pub fn rhp_magic_buffer(ms: *mut MagicSet, buf: *const c_void, nb: usize) -> *const c_char;
    pub fn rhp_magic_stream(ms: *mut MagicSet, stream: *mut c_void) -> *const c_char;
    pub fn rhp_magic_getflags(ms: *mut MagicSet) -> c_int;
    pub fn rhp_magic_setflags(ms: *mut MagicSet, flags: c_int) -> c_int;
    pub fn rhp_magic_errno(ms: *mut MagicSet) -> c_int;
    pub fn rhp_magic_error(ms: *mut MagicSet) -> *const c_char;
    pub fn rhp_magic_version() -> c_int;
}
