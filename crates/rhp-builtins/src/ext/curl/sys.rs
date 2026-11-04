//! The libcurl API rhp calls (no bindgen), and its constants.
//!
//! build/native.rs links libcurl. Option, info and error numbers come from
//! consts.c, compiled against the linked curl.h: [`php`] for php's constants,
//! [`internal`] for the C names php does not export.

#![allow(non_camel_case_types, clippy::upper_case_acronyms)]

use std::collections::HashMap;
use std::ffi::{c_char, c_double, c_int, c_long, c_uint, c_void};
use std::sync::OnceLock;

use super::consts;

pub enum CURL {}
pub enum CURLM {}
pub enum CURLSH {}
pub enum curl_mime {}
pub enum curl_mimepart {}
pub enum curl_pushheaders {}

pub type CURLcode = c_int;
pub type CURLMcode = c_int;
pub type CURLSHcode = c_int;
pub type curl_off_t = i64;

#[repr(C)]
pub struct curl_slist {
    pub data: *mut c_char,
    pub next: *mut curl_slist,
}

#[repr(C)]
pub struct curl_certinfo {
    pub num_of_certs: c_int,
    pub certinfo: *mut *mut curl_slist,
}

#[repr(C)]
pub struct curl_blob {
    pub data: *mut c_void,
    pub len: usize,
    pub flags: c_uint,
}

/// `CURLMsg`; `data` is a union of a pointer and a `CURLcode`.
#[repr(C)]
pub struct CURLMsg {
    pub msg: c_int,
    pub easy_handle: *mut CURL,
    pub data: *mut c_void,
}

impl CURLMsg {
    /// `data.result`.
    pub fn result(&self) -> CURLcode {
        // SAFETY: the union's first bytes hold the CURLcode.
        unsafe { *(std::ptr::addr_of!(self.data) as *const CURLcode) }
    }
}

/// `curl_version_info_data` up to CURLVERSION_FIFTH (all php reads).
#[repr(C)]
pub struct curl_version_info_data {
    pub age: c_int,
    pub version: *const c_char,
    pub version_num: c_uint,
    pub host: *const c_char,
    pub features: c_int,
    pub ssl_version: *const c_char,
    pub ssl_version_num: c_long,
    pub libz_version: *const c_char,
    pub protocols: *const *const c_char,
    pub ares: *const c_char,
    pub ares_num: c_int,
    pub libidn: *const c_char,
    pub iconv_ver_num: c_int,
    pub libssh_version: *const c_char,
    pub brotli_ver_num: c_uint,
    pub brotli_version: *const c_char,
}

pub type WriteFn = unsafe extern "C" fn(*mut c_char, usize, usize, *mut c_void) -> usize;
pub type ReadFn = unsafe extern "C" fn(*mut c_char, usize, usize, *mut c_void) -> usize;
pub type ProgressFn =
    unsafe extern "C" fn(*mut c_void, c_double, c_double, c_double, c_double) -> c_int;
pub type XferinfoFn =
    unsafe extern "C" fn(*mut c_void, curl_off_t, curl_off_t, curl_off_t, curl_off_t) -> c_int;
pub type FnmatchFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
pub type DebugFn = unsafe extern "C" fn(*mut CURL, c_int, *mut c_char, usize, *mut c_void) -> c_int;
pub type PrereqFn =
    unsafe extern "C" fn(*mut c_void, *mut c_char, *mut c_char, c_int, c_int) -> c_int;
pub type SshHostkeyFn = unsafe extern "C" fn(*mut c_void, c_int, *const c_char, usize) -> c_int;
pub type PushFn =
    unsafe extern "C" fn(*mut CURL, *mut CURL, usize, *mut curl_pushheaders, *mut c_void) -> c_int;
pub type SeekFn = unsafe extern "C" fn(*mut c_void, curl_off_t, c_int) -> c_int;
pub type FreeFn = unsafe extern "C" fn(*mut c_void);

extern "C" {
    pub fn curl_global_init(flags: c_long) -> CURLcode;
    pub fn curl_version_info(age: c_int) -> *mut curl_version_info_data;

    pub fn curl_easy_init() -> *mut CURL;
    pub fn curl_easy_setopt(curl: *mut CURL, option: c_int, ...) -> CURLcode;
    pub fn curl_easy_perform(curl: *mut CURL) -> CURLcode;
    pub fn curl_easy_cleanup(curl: *mut CURL);
    pub fn curl_easy_getinfo(curl: *mut CURL, info: c_int, ...) -> CURLcode;
    pub fn curl_easy_duphandle(curl: *mut CURL) -> *mut CURL;
    pub fn curl_easy_reset(curl: *mut CURL);
    pub fn curl_easy_strerror(code: CURLcode) -> *const c_char;
    pub fn curl_easy_escape(curl: *mut CURL, s: *const c_char, len: c_int) -> *mut c_char;
    pub fn curl_easy_unescape(
        curl: *mut CURL,
        s: *const c_char,
        len: c_int,
        outlen: *mut c_int,
    ) -> *mut c_char;
    pub fn curl_easy_pause(curl: *mut CURL, bitmask: c_int) -> CURLcode;
    pub fn curl_easy_upkeep(curl: *mut CURL) -> CURLcode;
    pub fn curl_free(p: *mut c_void);

    pub fn curl_slist_append(list: *mut curl_slist, s: *const c_char) -> *mut curl_slist;
    pub fn curl_slist_free_all(list: *mut curl_slist);

    pub fn curl_multi_init() -> *mut CURLM;
    pub fn curl_multi_add_handle(multi: *mut CURLM, curl: *mut CURL) -> CURLMcode;
    pub fn curl_multi_remove_handle(multi: *mut CURLM, curl: *mut CURL) -> CURLMcode;
    pub fn curl_multi_perform(multi: *mut CURLM, running: *mut c_int) -> CURLMcode;
    pub fn curl_multi_wait(
        multi: *mut CURLM,
        extra: *mut c_void,
        extra_nfds: c_uint,
        timeout_ms: c_int,
        numfds: *mut c_int,
    ) -> CURLMcode;
    pub fn curl_multi_info_read(multi: *mut CURLM, msgs_in_queue: *mut c_int) -> *mut CURLMsg;
    pub fn curl_multi_cleanup(multi: *mut CURLM) -> CURLMcode;
    pub fn curl_multi_strerror(code: CURLMcode) -> *const c_char;
    pub fn curl_multi_setopt(multi: *mut CURLM, option: c_int, ...) -> CURLMcode;
    pub fn curl_pushheader_bynum(h: *mut curl_pushheaders, num: usize) -> *mut c_char;

    pub fn curl_share_init() -> *mut CURLSH;
    pub fn curl_share_setopt(share: *mut CURLSH, option: c_int, ...) -> CURLSHcode;
    pub fn curl_share_cleanup(share: *mut CURLSH) -> CURLSHcode;
    pub fn curl_share_strerror(code: CURLSHcode) -> *const c_char;

    pub fn curl_mime_init(curl: *mut CURL) -> *mut curl_mime;
    pub fn curl_mime_free(mime: *mut curl_mime);
    pub fn curl_mime_addpart(mime: *mut curl_mime) -> *mut curl_mimepart;
    pub fn curl_mime_name(part: *mut curl_mimepart, name: *const c_char) -> CURLcode;
    pub fn curl_mime_data(part: *mut curl_mimepart, data: *const c_char, len: usize) -> CURLcode;
    pub fn curl_mime_data_cb(
        part: *mut curl_mimepart,
        datasize: curl_off_t,
        readfunc: Option<ReadFn>,
        seekfunc: Option<SeekFn>,
        freefunc: Option<FreeFn>,
        arg: *mut c_void,
    ) -> CURLcode;
    pub fn curl_mime_filename(part: *mut curl_mimepart, name: *const c_char) -> CURLcode;
    pub fn curl_mime_type(part: *mut curl_mimepart, mimetype: *const c_char) -> CURLcode;
}

// consts.c
#[repr(C)]
struct ConstSlot {
    value: i64,
    defined: c_int,
}

extern "C" {
    static rhp_curl_php_consts: [ConstSlot; 0];
    static rhp_curl_php_count: std::ffi::c_ulong;
    static rhp_curl_internal_consts: [i64; 0];
    static rhp_curl_internal_count: std::ffi::c_ulong;
}

/// php's constants that libcurl defines, in registration order.
pub fn php_constants() -> Vec<(&'static str, i64)> {
    // SAFETY: consts.c defines `rhp_curl_php_count` slots, one per name.
    let slots = unsafe {
        let n = rhp_curl_php_count as usize;
        assert_eq!(n, consts::PHP.len(), "consts.c and consts.rs disagree");
        std::slice::from_raw_parts(rhp_curl_php_consts.as_ptr(), n)
    };
    consts::PHP
        .iter()
        .zip(slots)
        .filter(|(_, s)| s.defined != 0)
        .map(|(n, s)| (*n, s.value))
        .collect()
}

/// Process-wide (not thread-local: handles are freed during thread-local
/// teardown too).
fn by_name() -> &'static HashMap<&'static str, i64> {
    static MAP: OnceLock<HashMap<&'static str, i64>> = OnceLock::new();
    MAP.get_or_init(|| rhp_values::mem::uncounted(|| php_constants().into_iter().collect()))
}

/// The value of php constant `name`. Panics on a name php does not define
/// against this libcurl: the port names only constants it does.
pub fn php(name: &str) -> i64 {
    *by_name()
        .get(name)
        .unwrap_or_else(|| panic!("curl constant {name} is not defined"))
}

/// Whether php defines constant `name` against this libcurl.
pub fn php_defined(name: &str) -> bool {
    by_name().contains_key(name)
}

/// The value of a libcurl name in [`consts::INTERNAL`].
pub fn internal(name: &str) -> i64 {
    let i = consts::INTERNAL
        .iter()
        .position(|n| *n == name)
        .unwrap_or_else(|| panic!("{name} is not in consts::INTERNAL"));
    // SAFETY: consts.c defines one value per INTERNAL name.
    unsafe {
        assert_eq!(rhp_curl_internal_count as usize, consts::INTERNAL.len());
        *rhp_curl_internal_consts.as_ptr().add(i)
    }
}

// shim.c
pub enum FILE {}

extern "C" {
    /// A write-only stdio stream whose writes go to
    /// [`super::easy::rhp_curl_stderr_write`] with `cookie`.
    pub fn rhp_curl_cookie_file(cookie: *mut c_void) -> *mut FILE;
    pub fn rhp_curl_stderr() -> *mut FILE;
    pub fn fclose(f: *mut FILE) -> c_int;
}

#[cfg(test)]
extern "C" {
    static rhp_curl_layout: [usize; 0];
    static rhp_curl_layout_count: std::ffi::c_ulong;
}

/// The sizes and offsets shim.c reports, in its order, for the layout test.
#[cfg(test)]
pub fn c_layout() -> &'static [usize] {
    // SAFETY: shim.c defines `rhp_curl_layout_count` entries.
    unsafe { std::slice::from_raw_parts(rhp_curl_layout.as_ptr(), rhp_curl_layout_count as usize) }
}

/// The same, from this file's declarations.
#[cfg(test)]
pub fn rust_layout() -> Vec<usize> {
    use std::mem::{offset_of, size_of};
    vec![
        offset_of!(curl_version_info_data, ssl_version_num),
        offset_of!(curl_version_info_data, protocols),
        offset_of!(curl_version_info_data, iconv_ver_num),
        offset_of!(curl_version_info_data, brotli_version),
        size_of::<CURLMsg>(),
        offset_of!(CURLMsg, data),
        size_of::<curl_blob>(),
        size_of::<curl_certinfo>(),
        size_of::<curl_slist>(),
    ]
}
