//! The part of libzip's API ext/zip calls (zip.h), declared by hand.
//! `build/native.rs` links the libzip php links; `shim.c` compiles the
//! constants against the same zip.h and reports the layouts mirrored here.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_double, c_int, c_longlong, c_uint, c_void};

pub type zip_flags_t = u32;

#[repr(C)]
pub struct zip_t {
    _p: [u8; 0],
}
#[repr(C)]
pub struct zip_file_t {
    _p: [u8; 0],
}
#[repr(C)]
pub struct zip_source_t {
    _p: [u8; 0],
}

/// `struct zip_stat`.
#[repr(C)]
pub struct zip_stat_t {
    pub valid: u64,
    pub name: *const c_char,
    pub index: u64,
    pub size: u64,
    pub comp_size: u64,
    pub mtime: libc::time_t,
    pub crc: u32,
    pub comp_method: u16,
    pub encryption_method: u16,
    pub flags: u32,
}

impl zip_stat_t {
    pub fn zeroed() -> zip_stat_t {
        zip_stat_t {
            valid: 0,
            name: std::ptr::null(),
            index: 0,
            size: 0,
            comp_size: 0,
            mtime: 0,
            crc: 0,
            comp_method: 0,
            encryption_method: 0,
            flags: 0,
        }
    }
}

/// `struct zip_error`.
#[repr(C)]
pub struct zip_error_t {
    pub zip_err: c_int,
    pub sys_err: c_int,
    pub str_: *mut c_char,
}

pub type zip_progress_callback = unsafe extern "C" fn(*mut zip_t, c_double, *mut c_void);
pub type zip_cancel_callback = unsafe extern "C" fn(*mut zip_t, *mut c_void) -> c_int;
pub type zip_state_free = unsafe extern "C" fn(*mut c_void);

pub const ZIP_CREATE: c_int = 1;
pub const ZIP_TRUNCATE: c_int = 8;
pub const ZIP_RDONLY: c_int = 16;
pub const ZIP_FL_NOCASE: zip_flags_t = 1;
pub const ZIP_FL_UNCHANGED: zip_flags_t = 8;
pub const ZIP_FL_ENC_RAW: zip_flags_t = 64;
pub const ZIP_EM_NONE: u16 = 0;
pub const ZIP_ER_OK: c_int = 0;

extern "C" {
    pub fn zip_open(path: *const c_char, flags: c_int, errorp: *mut c_int) -> *mut zip_t;
    pub fn zip_close(za: *mut zip_t) -> c_int;
    pub fn zip_discard(za: *mut zip_t);
    pub fn zip_get_error(za: *mut zip_t) -> *mut zip_error_t;
    pub fn zip_error_clear(za: *mut zip_t);
    pub fn zip_error_code_zip(e: *const zip_error_t) -> c_int;
    pub fn zip_error_code_system(e: *const zip_error_t) -> c_int;
    pub fn zip_error_fini(e: *mut zip_error_t);
    pub fn zip_error_init(e: *mut zip_error_t);
    pub fn zip_error_set(e: *mut zip_error_t, ze: c_int, se: c_int);
    pub fn zip_error_strerror(e: *mut zip_error_t) -> *const c_char;
    pub fn zip_strerror(za: *mut zip_t) -> *const c_char;
    pub fn zip_libzip_version() -> *const c_char;

    pub fn zip_get_num_entries(za: *mut zip_t, flags: zip_flags_t) -> i64;
    pub fn zip_get_name(za: *mut zip_t, index: u64, flags: zip_flags_t) -> *const c_char;
    pub fn zip_name_locate(za: *mut zip_t, name: *const c_char, flags: zip_flags_t) -> i64;
    pub fn zip_stat(
        za: *mut zip_t,
        name: *const c_char,
        flags: zip_flags_t,
        sb: *mut zip_stat_t,
    ) -> c_int;
    pub fn zip_stat_index(
        za: *mut zip_t,
        index: u64,
        flags: zip_flags_t,
        sb: *mut zip_stat_t,
    ) -> c_int;

    pub fn zip_get_archive_comment(
        za: *mut zip_t,
        lenp: *mut c_int,
        flags: zip_flags_t,
    ) -> *const c_char;
    pub fn zip_set_archive_comment(za: *mut zip_t, comment: *const c_char, len: u16) -> c_int;
    pub fn zip_get_archive_flag(za: *mut zip_t, flag: zip_flags_t, flags: zip_flags_t) -> c_int;
    pub fn zip_set_archive_flag(za: *mut zip_t, flag: zip_flags_t, value: c_int) -> c_int;
    pub fn zip_file_get_comment(
        za: *mut zip_t,
        index: u64,
        lenp: *mut u32,
        flags: zip_flags_t,
    ) -> *const c_char;
    pub fn zip_file_set_comment(
        za: *mut zip_t,
        index: u64,
        comment: *const c_char,
        len: u16,
        flags: zip_flags_t,
    ) -> c_int;
    pub fn zip_file_get_external_attributes(
        za: *mut zip_t,
        index: u64,
        flags: zip_flags_t,
        opsys: *mut u8,
        attributes: *mut u32,
    ) -> c_int;
    pub fn zip_file_set_external_attributes(
        za: *mut zip_t,
        index: u64,
        flags: zip_flags_t,
        opsys: u8,
        attributes: u32,
    ) -> c_int;
    pub fn zip_file_set_encryption(
        za: *mut zip_t,
        index: u64,
        method: u16,
        password: *const c_char,
    ) -> c_int;
    pub fn zip_file_set_mtime(
        za: *mut zip_t,
        index: u64,
        mtime: libc::time_t,
        flags: zip_flags_t,
    ) -> c_int;
    pub fn zip_set_file_compression(za: *mut zip_t, index: u64, method: i32, flags: u32) -> c_int;
    pub fn zip_set_default_password(za: *mut zip_t, password: *const c_char) -> c_int;

    pub fn zip_file_add(
        za: *mut zip_t,
        name: *const c_char,
        source: *mut zip_source_t,
        flags: zip_flags_t,
    ) -> i64;
    pub fn zip_file_replace(
        za: *mut zip_t,
        index: u64,
        source: *mut zip_source_t,
        flags: zip_flags_t,
    ) -> c_int;
    pub fn zip_dir_add(za: *mut zip_t, name: *const c_char, flags: zip_flags_t) -> i64;
    pub fn zip_delete(za: *mut zip_t, index: u64) -> c_int;
    pub fn zip_file_rename(
        za: *mut zip_t,
        index: u64,
        name: *const c_char,
        flags: zip_flags_t,
    ) -> c_int;
    pub fn zip_unchange(za: *mut zip_t, index: u64) -> c_int;
    pub fn zip_unchange_all(za: *mut zip_t) -> c_int;
    pub fn zip_unchange_archive(za: *mut zip_t) -> c_int;

    pub fn zip_source_buffer(
        za: *mut zip_t,
        data: *const c_void,
        len: u64,
        freep: c_int,
    ) -> *mut zip_source_t;
    pub fn zip_source_file(
        za: *mut zip_t,
        fname: *const c_char,
        start: u64,
        len: i64,
    ) -> *mut zip_source_t;
    pub fn zip_source_filep(
        za: *mut zip_t,
        file: *mut libc::FILE,
        start: u64,
        len: i64,
    ) -> *mut zip_source_t;
    pub fn zip_source_free(source: *mut zip_source_t);

    pub fn zip_fopen(za: *mut zip_t, name: *const c_char, flags: zip_flags_t) -> *mut zip_file_t;
    pub fn zip_fopen_index(za: *mut zip_t, index: u64, flags: zip_flags_t) -> *mut zip_file_t;
    pub fn zip_fread(zf: *mut zip_file_t, buf: *mut c_void, nbytes: u64) -> i64;
    pub fn zip_fseek(zf: *mut zip_file_t, offset: i64, whence: c_int) -> i8;
    pub fn zip_ftell(zf: *mut zip_file_t) -> i64;
    pub fn zip_fclose(zf: *mut zip_file_t) -> c_int;
    pub fn zip_file_get_error(zf: *mut zip_file_t) -> *mut zip_error_t;

    pub fn zip_register_progress_callback_with_state(
        za: *mut zip_t,
        precision: c_double,
        callback: Option<zip_progress_callback>,
        free: Option<zip_state_free>,
        ud: *mut c_void,
    ) -> c_int;
    pub fn zip_register_cancel_callback_with_state(
        za: *mut zip_t,
        callback: Option<zip_cancel_callback>,
        free: Option<zip_state_free>,
        ud: *mut c_void,
    ) -> c_int;

    pub fn zip_compression_method_supported(method: i32, compress: c_int) -> c_int;
    pub fn zip_encryption_method_supported(method: u16, encode: c_int) -> c_int;

    // shim.c
    pub fn rhp_zip_constants(
        cb: unsafe extern "C" fn(
            *mut c_void,
            *const c_char,
            c_longlong,
            *const c_char,
            *const c_char,
        ),
        ctx: *mut c_void,
    );
    pub fn rhp_zip_fl_open_file_now() -> c_uint;
    pub fn rhp_zip_file_is_seekable(zf: *mut zip_file_t) -> c_int;
    #[cfg(test)]
    fn rhp_zip_layout(out: *mut usize);
}

/// The C string at `p` as bytes (empty for null).
///
/// # Safety
/// `p` is null or a NUL-terminated string that outlives the borrow.
pub unsafe fn cstr<'a>(p: *const c_char) -> &'a [u8] {
    if p.is_null() {
        b""
    } else {
        unsafe { std::ffi::CStr::from_ptr(p) }.to_bytes()
    }
}

/// shim.c's `rhp_zip_layout`: zip.h's sizes and offsets.
#[cfg(test)]
fn c_layout() -> [usize; 14] {
    let mut out = [0usize; 14];
    unsafe { rhp_zip_layout(out.as_mut_ptr()) };
    out
}

#[cfg(test)]
mod tests;
