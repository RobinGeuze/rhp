//! php's bundled libmagic 5.46 (ext/fileinfo/libmagic, with php's patches)
//! and its compiled magic database (data_file.c), as ext/fileinfo uses them.
//! libmagic reads files and runs regexes through php's streams and PCRE;
//! here it calls the [`Hooks`] rhp-builtins installs with [`set_hooks`].

pub mod sys;

#[cfg(test)]
mod tests;

use std::ffi::{c_void, CStr, CString};
use std::ptr::NonNull;
use std::sync::Once;

pub use sys::{Hooks, Stat};

/// magic.h's flags that php exports as `FILEINFO_*` (glue/fileinfo.c checks
/// the two it asserts).
pub const MAGIC_NONE: i32 = 0x0000000;
pub const MAGIC_SYMLINK: i32 = 0x0000002;
pub const MAGIC_DEVICES: i32 = 0x0000008;
pub const MAGIC_MIME_TYPE: i32 = 0x0000010;
pub const MAGIC_CONTINUE: i32 = 0x0000020;
pub const MAGIC_PRESERVE_ATIME: i32 = 0x0000080;
pub const MAGIC_RAW: i32 = 0x0000100;
pub const MAGIC_MIME_ENCODING: i32 = 0x0000400;
pub const MAGIC_MIME: i32 = MAGIC_MIME_TYPE | MAGIC_MIME_ENCODING;
pub const MAGIC_APPLE: i32 = 0x0000800;
pub const MAGIC_EXTENSION: i32 = 0x1000000;

/// Install the hooks libmagic's streams, PCRE and warnings go through.
/// Process-wide, like php's function table: the hooks find the running VM
/// themselves (per thread).
pub fn set_hooks(h: &'static Hooks) {
    // SAFETY: the table lives for the whole process.
    unsafe { sys::rhp_fi_set_hooks(h) }
}

/// libmagic's lazily built type tables (apprentice.c's init_file_tables)
/// are process globals written on first use: build them once, before any
/// thread can race on them.
fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        let ms = sys::rhp_magic_open(0);
        if !ms.is_null() {
            sys::rhp_magic_load(ms, std::ptr::null());
            sys::rhp_magic_close(ms);
        }
    });
}

/// `magic_version()`: 546 for libmagic 5.46.
pub fn version() -> i32 {
    unsafe { sys::rhp_magic_version() }
}

/// A `struct magic_set` (php's finfo object holds one).
pub struct Magic(NonNull<sys::MagicSet>);

impl Magic {
    /// `magic_open(flags)`; `None` for flags libmagic refuses.
    pub fn open(flags: i32) -> Option<Magic> {
        init();
        NonNull::new(unsafe { sys::rhp_magic_open(flags) }).map(Magic)
    }

    /// `magic_load(ms, file)`: the built-in database for `None`.
    pub fn load(&mut self, file: Option<&[u8]>) -> bool {
        let c = file.map(|f| CString::new(f).unwrap_or_default());
        let p = c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
        unsafe { sys::rhp_magic_load(self.0.as_ptr(), p) != -1 }
    }

    /// `magic_buffer(ms, data)`.
    pub fn buffer(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let r = unsafe { sys::rhp_magic_buffer(self.0.as_ptr(), data.as_ptr().cast(), data.len()) };
        cbytes(r)
    }

    /// `magic_stream(ms, stream)`, `stream` being a handle the hooks know.
    ///
    /// # Safety
    ///
    /// `stream` is valid for the installed hooks.
    pub unsafe fn stream(&mut self, stream: *mut c_void) -> Option<Vec<u8>> {
        cbytes(sys::rhp_magic_stream(self.0.as_ptr(), stream))
    }

    pub fn flags(&self) -> i32 {
        unsafe { sys::rhp_magic_getflags(self.0.as_ptr()) }
    }

    /// `magic_setflags`: false only for MAGIC_PRESERVE_ATIME without
    /// utime(s), which never happens here.
    pub fn set_flags(&mut self, flags: i32) -> bool {
        unsafe { sys::rhp_magic_setflags(self.0.as_ptr(), flags) != -1 }
    }

    pub fn errno(&self) -> i32 {
        unsafe { sys::rhp_magic_errno(self.0.as_ptr()) }
    }

    /// `magic_error()`: the last error, if any.
    pub fn error(&self) -> Option<Vec<u8>> {
        cbytes(unsafe { sys::rhp_magic_error(self.0.as_ptr()) })
    }
}

impl Drop for Magic {
    fn drop(&mut self) {
        unsafe { sys::rhp_magic_close(self.0.as_ptr()) }
    }
}

fn cbytes(p: *const std::ffi::c_char) -> Option<Vec<u8>> {
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_bytes().to_vec())
}
