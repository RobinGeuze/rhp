//! The system iconv(3), linked as php links it: the macOS SDK's libiconv
//! (/usr/lib/libiconv.2.dylib, an OS library) and glibc's iconv (part of
//! libc) on Linux. docs/design/libraries.md lists both as dynamic.
//!
//! php's C sets and reads the thread's `errno` around these calls. Here that
//! `errno` is a per-thread value of this module's ([`errno`], [`set_errno`]):
//! it goes into libc's `errno` before each call and is read back after, so
//! Rust code between two calls cannot change what the next check sees, and
//! it carries over from one builtin to the next as php's does (php prints it
//! in "Unknown error (%d)").
//!
//! Conversions run under [`Locale`]: php sets LC_CTYPE to C.UTF-8 at startup
//! (zend_reset_lc_ctype_locale) and glibc's //TRANSLIT reads it. rhp never
//! calls setlocale; the guard installs a per-thread C.UTF-8 locale with
//! uselocale while it lives.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::ptr::NonNull;

// libc's own declarations are deprecated on Apple, so rhp declares the three
// functions it calls.
#[cfg_attr(target_os = "macos", link(name = "iconv", kind = "dylib"))]
extern "C" {
    fn iconv_open(tocode: *const c_char, fromcode: *const c_char) -> *mut c_void;
    fn iconv(
        cd: *mut c_void,
        inbuf: *mut *mut c_char,
        inbytesleft: *mut usize,
        outbuf: *mut *mut c_char,
        outbytesleft: *mut usize,
    ) -> usize;
    fn iconv_close(cd: *mut c_void) -> c_int;
}

#[cfg(target_os = "macos")]
#[link(name = "iconv", kind = "dylib")]
extern "C" {
    static _libiconv_version: c_int;
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
extern "C" {
    fn gnu_get_libc_version() -> *const c_char;
}

/// `ICONV_IMPL`, as php's configure names the iconv it finds
/// (ext/iconv/config.m4): glibc's, else one that exports
/// `_libiconv_version`.
pub fn implementation() -> &'static str {
    if cfg!(all(target_os = "linux", target_env = "gnu")) {
        "glibc"
    } else if cfg!(target_os = "macos") {
        "libiconv"
    } else {
        "unknown"
    }
}

/// `ICONV_VERSION` (get_iconv_version): `_libiconv_version` as major.minor,
/// or `gnu_get_libc_version()`.
pub fn version() -> String {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: an int the library exports and never changes.
        let v = unsafe { _libiconv_version };
        format!("{}.{}", v >> 8, v & 0xff)
    }
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // SAFETY: glibc returns a static NUL-terminated string.
        unsafe { CStr::from_ptr(gnu_get_libc_version()) }
            .to_string_lossy()
            .into_owned()
    }
    #[cfg(not(any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))))]
    {
        "unknown".to_string()
    }
}

thread_local! {
    static ERRNO: Cell<c_int> = const { Cell::new(0) };
}

/// php's `errno` as the iconv calls left it.
pub fn errno() -> c_int {
    ERRNO.with(Cell::get)
}

/// php's `errno = v`.
pub fn set_errno(v: c_int) {
    ERRNO.with(|e| e.set(v));
}

#[cfg(target_os = "macos")]
fn errno_location() -> *mut c_int {
    // SAFETY: returns this thread's errno slot; no preconditions.
    unsafe { libc::__error() }
}

#[cfg(not(target_os = "macos"))]
fn errno_location() -> *mut c_int {
    // SAFETY: returns this thread's errno slot; no preconditions.
    unsafe { libc::__errno_location() }
}

/// Run one libc call with [`errno`] as the thread's errno, and keep what the
/// call leaves there.
fn with_errno<T>(f: impl FnOnce() -> T) -> T {
    let p = errno_location();
    // SAFETY: `p` is this thread's errno slot.
    unsafe { *p = errno() };
    let r = f();
    // SAFETY: as above.
    set_errno(unsafe { *p });
    r
}

/// The per-thread C.UTF-8 locale (C when the system has none, like
/// zend_reset_lc_ctype_locale's fallback).
struct ThreadLocale(libc::locale_t);

impl Drop for ThreadLocale {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: created by newlocale below and no longer installed:
            // every Locale guard restores the previous locale when dropped.
            unsafe { libc::freelocale(self.0) };
        }
    }
}

thread_local! {
    static THREAD_LOCALE: ThreadLocale = ThreadLocale(c_utf8_locale());
}

fn c_utf8_locale() -> libc::locale_t {
    for name in [c"C.UTF-8", c"C"] {
        // SAFETY: a valid category mask and NUL-terminated name; no base.
        let l =
            unsafe { libc::newlocale(libc::LC_CTYPE_MASK, name.as_ptr(), std::ptr::null_mut()) };
        if !l.is_null() {
            return l;
        }
    }
    std::ptr::null_mut()
}

/// While alive, this thread runs under the C.UTF-8 locale php's iconv calls
/// see.
pub struct Locale(libc::locale_t);

impl Locale {
    pub fn enter() -> Locale {
        let loc = THREAD_LOCALE.with(|l| l.0);
        if loc.is_null() {
            return Locale(std::ptr::null_mut());
        }
        // SAFETY: `loc` is a valid locale owned by this thread.
        Locale(unsafe { libc::uselocale(loc) })
    }
}

impl Drop for Locale {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the locale uselocale returned on entry (possibly
            // LC_GLOBAL_LOCALE), still valid.
            unsafe { libc::uselocale(self.0) };
        }
    }
}

/// An open conversion descriptor (`iconv_t`), closed when dropped.
pub struct Cd(*mut c_void);

/// iconv's failure result, `(size_t)-1`.
pub const FAILED: usize = usize::MAX;

impl Cd {
    /// `iconv_open(to, from)`; `None` is `(iconv_t)-1`, with [`errno`]
    /// saying why.
    pub fn open(to: &CStr, from: &CStr) -> Option<Cd> {
        // SAFETY: both names are NUL-terminated.
        let cd = with_errno(|| unsafe { iconv_open(to.as_ptr(), from.as_ptr()) });
        if cd as isize == -1 {
            None
        } else {
            Some(Cd(cd))
        }
    }

    /// One `iconv()` call converting the front of `*input` into
    /// `out[*pos..]`; both advance past what was consumed and produced.
    /// Returns iconv's result ([`FAILED`]: see [`errno`]).
    ///
    /// Callers pass slices of NUL-terminated buffers (a [`NulCopy`]) where
    /// php passes a zend_string, so a library that reads past the input's
    /// end sees what it sees under php.
    pub fn conv(&self, input: &mut &[u8], out: &mut [u8], pos: &mut usize) -> usize {
        let mut in_p = input.as_ptr() as *mut c_char;
        let mut in_left = input.len();
        let start = (*pos).min(out.len());
        let mut out_p = out[start..].as_mut_ptr() as *mut c_char;
        let mut out_left = out.len() - start;
        let room = out_left;
        // SAFETY: the pointers and counts describe live slices (a byte read
        // past the input is still in the caller's buffer, see above); iconv
        // writes at most `out_left` bytes.
        let r = with_errno(|| unsafe {
            iconv(self.0, &mut in_p, &mut in_left, &mut out_p, &mut out_left)
        });
        *input = &input[input.len() - in_left..];
        *pos = start + (room - out_left);
        r
    }

    /// `iconv(cd, NULL, NULL, &out, &left)`: write the shift sequence back
    /// to the initial state into `out[*pos..]`.
    pub fn flush(&self, out: &mut [u8], pos: &mut usize) -> usize {
        let start = (*pos).min(out.len());
        let mut out_p = out[start..].as_mut_ptr() as *mut c_char;
        let mut out_left = out.len() - start;
        let room = out_left;
        // SAFETY: as in conv; a null input asks for the shift sequence.
        let r = with_errno(|| unsafe {
            iconv(
                self.0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut out_p,
                &mut out_left,
            )
        });
        *pos = start + (room - out_left);
        r
    }

    /// `iconv(cd, NULL, NULL, NULL, NULL)`: back to the initial state.
    pub fn reset(&self) -> usize {
        // SAFETY: all-null arguments are iconv's reset request.
        with_errno(|| unsafe {
            iconv(
                self.0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        })
    }
}

impl Drop for Cd {
    fn drop(&mut self) {
        // SAFETY: an open descriptor, closed once.
        with_errno(|| unsafe { iconv_close(self.0) });
    }
}

/// A copy of some bytes with a NUL after them, as a zend_string sits in
/// memory, for [`Cd::conv`]. Derefs to the bytes without the NUL.
///
/// It stands in for a string php passes by pointer without allocating, so
/// it comes from the system allocator directly: memory_get_usage() and
/// memory_limit don't see it.
pub struct NulCopy {
    ptr: NonNull<u8>,
    len: usize,
}

impl NulCopy {
    pub fn new(b: &[u8]) -> NulCopy {
        let layout = Self::layout(b.len());
        // SAFETY: the layout is at least one byte.
        let Some(ptr) = NonNull::new(unsafe { System.alloc(layout) }) else {
            std::alloc::handle_alloc_error(layout)
        };
        // SAFETY: `ptr` has room for `b.len() + 1` bytes and doesn't overlap
        // `b`.
        unsafe {
            std::ptr::copy_nonoverlapping(b.as_ptr(), ptr.as_ptr(), b.len());
            ptr.as_ptr().add(b.len()).write(0);
        }
        NulCopy { ptr, len: b.len() }
    }

    fn layout(len: usize) -> Layout {
        // A slice is at most isize::MAX bytes, so `len + 1` fits.
        Layout::from_size_align(len + 1, 1).expect("string length")
    }
}

impl std::ops::Deref for NulCopy {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        // SAFETY: `len` initialised bytes, owned until drop.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for NulCopy {
    fn drop(&mut self) {
        // SAFETY: allocated by `new` from `System` with this layout.
        unsafe { System.dealloc(self.ptr.as_ptr(), Self::layout(self.len)) };
    }
}

#[cfg(test)]
mod tests;
