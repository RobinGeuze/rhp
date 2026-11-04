//! php-src's libmbfl and php_unicode.c at the pinned tag (`vendor/php-src`,
//! written by `crates/rhp-csrc/tools/sync-php-src-c.sh`), compiled into one
//! static archive. libmbfl is LGPL-2.1, so it lives in this crate alone
//! (rhp-builtins' `mbstring-libmbfl` feature).
//!
//! The C shares rhp-csrc's Zend shim: its strings come from rhp-csrc's
//! `emalloc` (counted against `memory_limit`), and its fallible growth
//! fails past the limit as php's does. Every global symbol has an rhp_
//! prefix (`glue/prefix_mbfl.h`); `glue/mbfl.c` checks at compile time the
//! constants and layouts mirrored here.
//!
//! The API is libmbfl's own: [`Encoding`] with its `to_wchar` /
//! `from_wchar` / `check` / `cut` hooks, [`ConvertBuf`] (`mb_convert_buf`),
//! [`fast_convert`], [`convert_case`] and [`strcut`]. Strings come back as
//! `Vec<u8>` copies; the C buffers are freed here.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr};

// The shim's symbols (emalloc family, zend_empty_string) are rhp-csrc's.
extern crate rhp_csrc;

/// `MBFL_ENCTYPE_SBCS`: one byte per character.
pub const ENCTYPE_SBCS: u32 = 1;
/// `MBFL_ENCTYPE_WCS2`: two bytes per character.
pub const ENCTYPE_WCS2: u32 = 2;
/// `MBFL_ENCTYPE_WCS4`: four bytes per character.
pub const ENCTYPE_WCS4: u32 = 4;

/// `MBFL_BAD_INPUT`: the error marker `to_wchar` emits.
pub const BAD_INPUT: u32 = 0xFFFF_FFFF;
/// `MBSTRING_MIN_WCHAR_BUFSIZE`.
pub const MIN_WCHAR_BUFSIZE: usize = 5;
/// `FIRST_DOUBLEWIDTH_CODEPOINT` (eaw_table.h).
pub const FIRST_DOUBLEWIDTH_CODEPOINT: u32 = 0x1100;
/// `MBFL_VERSION_MAJOR`, `_MINOR`, `_TEENY`.
pub const VERSION: (u32, u32, u32) = (1, 3, 2);

/// `MBFL_OUTPUTFILTER_ILLEGAL_MODE_*`: what replaces an unconvertible
/// character.
pub mod illegal_mode {
    pub const NONE: u32 = 0;
    pub const CHAR: u32 = 1;
    pub const LONG: u32 = 2;
    pub const ENTITY: u32 = 3;
    /// Internal: an invalid UTF-8 byte (0xFF) as the marker.
    pub const BADUTF8: u32 = 4;
}

/// `enum mbfl_no_encoding` values mbstring.c compares against.
pub mod no {
    pub const INVALID: i32 = -1;
    pub const QPRINT: i32 = 5;
    pub const CHARSET_MIN: i32 = 8;
    pub const UTF8: i32 = 21;
    pub const UTF8_SB: i32 = 25;
    pub const UTF7: i32 = 26;
    pub const UTF7IMAP: i32 = 27;
    pub const JIS: i32 = 41;
    pub const ISO2022JPMS: i32 = 45;
    pub const CP50220: i32 = 79;
    pub const CP50222: i32 = 81;
}

/// `php_case_mode` (php_unicode.h), the `MB_CASE_*` constants.
pub mod case_mode {
    pub const UPPER: i32 = 0;
    pub const LOWER: i32 = 1;
    pub const TITLE: i32 = 2;
    pub const FOLD: i32 = 3;
    pub const UPPER_SIMPLE: i32 = 4;
    pub const LOWER_SIMPLE: i32 = 5;
    pub const TITLE_SIMPLE: i32 = 6;
    pub const FOLD_SIMPLE: i32 = 7;
    /// `PHP_UNICODE_CASE_MODE_MAX`.
    pub const MAX: i32 = 8;
}

#[repr(C)]
struct ZendString {
    gc: [u32; 2],
    h: usize,
    len: usize,
    val: [u8; 1],
}

type ToWcharFn =
    unsafe extern "C" fn(*mut *mut u8, *mut usize, *mut u32, usize, *mut c_uint) -> usize;
type FromWcharFn = unsafe extern "C" fn(*mut u32, usize, *mut RawBuf, bool);
type CheckFn = unsafe extern "C" fn(*mut u8, usize) -> bool;
type CutFn = unsafe extern "C" fn(*mut u8, usize, usize, *mut u8) -> *mut ZendString;

/// `mbfl_encoding`: one of libmbfl's encodings (immutable C data).
#[repr(C)]
pub struct Encoding {
    no_encoding: c_int,
    name: *const c_char,
    mime_name: *const c_char,
    aliases: *const *const c_char,
    mblen_table: *const u8,
    flag: c_uint,
    input_filter: *const c_void,
    output_filter: *const c_void,
    to_wchar: Option<ToWcharFn>,
    from_wchar: Option<FromWcharFn>,
    check: Option<CheckFn>,
    cut: Option<CutFn>,
}

// SAFETY: every mbfl_encoding is a `const` C object; nothing writes to it.
unsafe impl Sync for Encoding {}

#[repr(C)]
struct RawBuf {
    out: *mut u8,
    limit: *mut u8,
    state: u32,
    errors: u32,
    replacement_char: u32,
    error_mode: c_uint,
    str: *mut ZendString,
}

#[repr(C)]
struct MbflString {
    encoding: *const Encoding,
    val: *mut u8,
    len: usize,
}

extern "C" {
    #[link_name = "rhp_mbfl_name2encoding_ex"]
    fn mbfl_name2encoding_ex(name: *const c_char, name_len: usize) -> *const Encoding;
    #[link_name = "rhp_mbfl_no2encoding"]
    fn mbfl_no2encoding(no_encoding: c_int) -> *const Encoding;
    #[link_name = "rhp_mbfl_get_supported_encodings"]
    fn mbfl_get_supported_encodings() -> *const *const Encoding;
    #[link_name = "rhp_mbfl_encoding_preferred_mime_name"]
    fn mbfl_encoding_preferred_mime_name(encoding: *const Encoding) -> *const c_char;
    #[link_name = "rhp_mb_fast_convert"]
    fn mb_fast_convert(
        input: *mut u8,
        in_len: usize,
        from: *const Encoding,
        to: *const Encoding,
        replacement_char: u32,
        error_mode: c_uint,
        num_errors: *mut c_uint,
    ) -> *mut ZendString;
    #[link_name = "rhp_php_unicode_convert_case"]
    fn php_unicode_convert_case(
        case_mode: c_int,
        srcstr: *const c_char,
        srclen: usize,
        src_encoding: *const Encoding,
        dst_encoding: *const Encoding,
        illegal_mode: c_int,
        illegal_substchar: u32,
    ) -> *mut ZendString;
    #[link_name = "rhp_mbfl_strcut"]
    fn mbfl_strcut(
        string: *mut MbflString,
        result: *mut MbflString,
        from: usize,
        length: usize,
    ) -> *mut MbflString;

    fn rhp_mb_convert_buf_init(buf: *mut RawBuf, initsize: usize, repl_char: u32, err_mode: c_uint);
    fn rhp_mb_convert_buf_result(buf: *mut RawBuf, enc: *const Encoding) -> *mut ZendString;
    fn rhp_mb_convert_buf_result_raw(buf: *mut RawBuf) -> *mut ZendString;
    fn rhp_mb_convert_buf_free(buf: *mut RawBuf);
    fn rhp_mb_convert_buf_append(buf: *mut RawBuf, s: *const u8, len: usize) -> bool;
    fn rhp_mbfl_eaw_table(len: *mut usize) -> *const [i32; 2];
    fn rhp_mbfl_rare_cp_bitvec(len: *mut usize) -> *const u32;

    static rhp_zend_empty_string: *mut ZendString;

    #[link_name = "rhp_mbfl_encoding_utf8"]
    static MBFL_ENCODING_UTF8: Encoding;
    #[link_name = "rhp_mbfl_encoding_base64"]
    static MBFL_ENCODING_BASE64: Encoding;
    #[link_name = "rhp_mbfl_encoding_qprint"]
    static MBFL_ENCODING_QPRINT: Encoding;
    #[link_name = "rhp_mbfl_encoding_html_ent"]
    static MBFL_ENCODING_HTML_ENT: Encoding;
    #[link_name = "rhp_mbfl_encoding_uuencode"]
    static MBFL_ENCODING_UUENCODE: Encoding;
    #[link_name = "rhp_mbfl_encoding_pass"]
    static MBFL_ENCODING_PASS: Encoding;
    #[link_name = "rhp_mbfl_encoding_sjis_mac"]
    static MBFL_ENCODING_SJIS_MAC: Encoding;
    #[link_name = "rhp_mbfl_encoding_ucs4be"]
    static MBFL_ENCODING_UCS4BE: Encoding;
    #[link_name = "rhp_mbfl_encoding_utf16be"]
    static MBFL_ENCODING_UTF16BE: Encoding;
    #[link_name = "rhp_mbfl_encoding_utf16le"]
    static MBFL_ENCODING_UTF16LE: Encoding;
    #[link_name = "rhp_mbfl_encoding_ascii"]
    static MBFL_ENCODING_ASCII: Encoding;
}

/// The bytes of a zend_string the C returned, which it gives up: a copy,
/// and the C buffer freed (never the static empty string).
///
/// # Safety
///
/// `s` is a live zend_string from libmbfl that nothing else owns.
unsafe fn take(s: *mut ZendString) -> Vec<u8> {
    let len = (*s).len;
    let v = std::slice::from_raw_parts((*s).val.as_ptr(), len).to_vec();
    if s != rhp_zend_empty_string {
        rhp_csrc::rhp_csrc_efree(s as *mut c_void);
    }
    v
}

/// A C string from libmbfl's tables.
fn cstr(p: *const c_char) -> &'static [u8] {
    // SAFETY: libmbfl's names are static NUL-terminated strings.
    unsafe { CStr::from_ptr(p).to_bytes() }
}

/// The encodings mbstring.c refers to by name.
pub mod encodings {
    use super::Encoding;

    macro_rules! enc {
        ($($f:ident => $s:ident),* $(,)?) => {$(
            pub fn $f() -> &'static Encoding {
                // SAFETY: a const C object that lives as long as the process.
                unsafe { &super::$s }
            }
        )*};
    }
    enc! {
        utf8 => MBFL_ENCODING_UTF8,
        base64 => MBFL_ENCODING_BASE64,
        qprint => MBFL_ENCODING_QPRINT,
        html_ent => MBFL_ENCODING_HTML_ENT,
        uuencode => MBFL_ENCODING_UUENCODE,
        pass => MBFL_ENCODING_PASS,
        sjis_mac => MBFL_ENCODING_SJIS_MAC,
        ucs4be => MBFL_ENCODING_UCS4BE,
        utf16be => MBFL_ENCODING_UTF16BE,
        utf16le => MBFL_ENCODING_UTF16LE,
        ascii => MBFL_ENCODING_ASCII,
    }
}

impl PartialEq for Encoding {
    /// Identity, as mbstring.c compares `mbfl_encoding` pointers.
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}
impl Eq for Encoding {}

impl std::fmt::Debug for Encoding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&String::from_utf8_lossy(self.name()))
    }
}

impl Encoding {
    /// `mbfl_name2encoding(name)`: the name, MIME name or alias, ignoring
    /// case. Like the C API it reads `name` up to its first NUL byte.
    pub fn by_name(name: &[u8]) -> Option<&'static Encoding> {
        let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
        Self::by_name_ex(&name[..end])
    }

    /// `mbfl_name2encoding_ex(name, len)`.
    pub fn by_name_ex(name: &[u8]) -> Option<&'static Encoding> {
        // SAFETY: the C reads at most `name.len()` bytes; the result is a
        // static encoding or NULL.
        unsafe { mbfl_name2encoding_ex(name.as_ptr() as *const c_char, name.len()).as_ref() }
    }

    /// `mbfl_no2encoding`.
    pub fn by_no(no: i32) -> Option<&'static Encoding> {
        // SAFETY: as above.
        unsafe { mbfl_no2encoding(no).as_ref() }
    }

    /// `mbfl_get_supported_encodings()`, in libmbfl's order.
    pub fn supported() -> impl Iterator<Item = &'static Encoding> {
        // SAFETY: a static NULL-terminated array of static encodings.
        let mut p = unsafe { mbfl_get_supported_encodings() };
        std::iter::from_fn(move || unsafe {
            let e = (*p).as_ref()?;
            p = p.add(1);
            Some(e)
        })
    }

    /// `no_encoding`.
    pub fn no(&self) -> i32 {
        self.no_encoding
    }
    pub fn name(&self) -> &'static [u8] {
        cstr(self.name)
    }
    pub fn mime_name(&self) -> Option<&'static [u8]> {
        (!self.mime_name.is_null()).then(|| cstr(self.mime_name))
    }
    /// `mbfl_encoding_preferred_mime_name`.
    pub fn preferred_mime_name(&self) -> Option<&'static [u8]> {
        // SAFETY: self is a libmbfl encoding.
        let p = unsafe { mbfl_encoding_preferred_mime_name(self) };
        (!p.is_null()).then(|| cstr(p))
    }
    pub fn aliases(&self) -> impl Iterator<Item = &'static [u8]> {
        let mut p = self.aliases;
        std::iter::from_fn(move || {
            if p.is_null() {
                return None;
            }
            // SAFETY: a static NULL-terminated array of C strings.
            unsafe {
                if (*p).is_null() {
                    return None;
                }
                let a = cstr(*p);
                p = p.add(1);
                Some(a)
            }
        })
    }
    /// `mblen_table`: the byte length of a character by its first byte.
    pub fn mblen_table(&self) -> Option<&'static [u8; 256]> {
        // SAFETY: a static 256-entry table when present.
        unsafe { (self.mblen_table as *const [u8; 256]).as_ref() }
    }
    pub fn flag(&self) -> u32 {
        self.flag
    }

    /// `to_wchar`: decodes from the front of `input` (advancing it) into
    /// `out`, returning how many codepoints it wrote. `out` must hold at
    /// least [`MIN_WCHAR_BUFSIZE`].
    pub fn to_wchar(&self, input: &mut &[u8], out: &mut [u32], state: &mut u32) -> usize {
        assert!(out.len() >= MIN_WCHAR_BUFSIZE);
        let f = self.to_wchar.expect("encoding has a to_wchar hook");
        let mut p = input.as_ptr() as *mut u8;
        let mut len = input.len();
        // SAFETY: the C reads at most `len` bytes from `p` (it never writes
        // through it) and writes at most `out.len()` codepoints.
        let n = unsafe { f(&mut p, &mut len, out.as_mut_ptr(), out.len(), state) };
        let used = input.len() - len;
        *input = &input[used..];
        n
    }

    /// `from_wchar`: encodes `input` onto `buf`; `end` flushes any state.
    pub fn from_wchar(&self, input: &[u32], buf: &mut ConvertBuf, end: bool) {
        let f = self.from_wchar.expect("encoding has a from_wchar hook");
        // SAFETY: the C reads `input` (it takes a mutable pointer but
        // doesn't write) and appends to the initialised buffer.
        unsafe { f(input.as_ptr() as *mut u32, input.len(), &mut buf.raw, end) }
    }

    /// Whether the encoding has a `check` hook.
    pub fn has_check(&self) -> bool {
        self.check.is_some()
    }

    /// `check`: is `s` valid in this encoding? Encodings without the hook
    /// are checked by decoding (php_mb_check_encoding).
    pub fn check(&self, s: &[u8]) -> Option<bool> {
        let f = self.check?;
        // SAFETY: the C reads `s`.
        Some(unsafe { f(s.as_ptr() as *mut u8, s.len()) })
    }

    /// Whether the encoding has a `cut` hook (mb_strcut).
    pub fn has_cut(&self) -> bool {
        self.cut.is_some()
    }

    /// `cut`: the bytes from `from` for `len`, moved to character
    /// boundaries. `from` must be within `s`.
    pub fn cut(&self, s: &[u8], from: usize, len: usize) -> Option<Vec<u8>> {
        let f = self.cut?;
        assert!(from <= s.len());
        let p = s.as_ptr() as *mut u8;
        // SAFETY: the C reads within [p, p + s.len()) and returns a new
        // string or the static empty one.
        Some(unsafe { take(f(p, from, len, p.add(s.len()))) })
    }
}

/// `mb_convert_buf`: an output buffer that `from_wchar` appends to.
pub struct ConvertBuf {
    raw: RawBuf,
    live: bool,
}

impl ConvertBuf {
    /// `mb_convert_buf_init`.
    pub fn new(initsize: usize, replacement_char: u32, error_mode: u32) -> ConvertBuf {
        let mut raw = RawBuf {
            out: std::ptr::null_mut(),
            limit: std::ptr::null_mut(),
            state: 0,
            errors: 0,
            replacement_char: 0,
            error_mode: 0,
            str: std::ptr::null_mut(),
        };
        // SAFETY: initialises `raw`, allocating its string.
        unsafe { rhp_mb_convert_buf_init(&mut raw, initsize, replacement_char, error_mode) };
        ConvertBuf { raw, live: true }
    }

    /// Characters the encoder could not convert.
    pub fn errors(&self) -> u32 {
        self.raw.errors
    }

    /// `mb_convert_buf_len`.
    pub fn len(&self) -> usize {
        // SAFETY: `out` points into the live string's buffer.
        unsafe { self.raw.out.offset_from((*self.raw.str).val.as_ptr()) as usize }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Appends raw bytes (`MB_CONVERT_BUF_ENSURE_EX` + copy). False when the
    /// buffer could not grow (memory_limit).
    pub fn append(&mut self, s: &[u8]) -> bool {
        // SAFETY: the buffer is live; the C grows it fallibly.
        unsafe { rhp_mb_convert_buf_append(&mut self.raw, s.as_ptr(), s.len()) }
    }

    /// `mb_convert_buf_result`: the bytes, consuming the buffer.
    pub fn result(mut self, enc: &Encoding) -> Vec<u8> {
        self.live = false;
        // SAFETY: the buffer is live; the result owns its string.
        unsafe { take(rhp_mb_convert_buf_result(&mut self.raw, enc)) }
    }

    /// `mb_convert_buf_result_raw`.
    pub fn result_raw(mut self) -> Vec<u8> {
        self.live = false;
        // SAFETY: as above.
        unsafe { take(rhp_mb_convert_buf_result_raw(&mut self.raw)) }
    }
}

impl Drop for ConvertBuf {
    fn drop(&mut self) {
        if self.live {
            // SAFETY: the buffer still owns its string.
            unsafe { rhp_mb_convert_buf_free(&mut self.raw) };
        }
    }
}

/// `mb_fast_convert`: `input` from one encoding to another, and the number
/// of characters that could not be converted.
pub fn fast_convert(
    input: &[u8],
    from: &Encoding,
    to: &Encoding,
    replacement_char: u32,
    error_mode: u32,
) -> (Vec<u8>, u32) {
    let mut errors: c_uint = 0;
    // SAFETY: the C reads `input` and returns a new string.
    let s = unsafe {
        mb_fast_convert(
            input.as_ptr() as *mut u8,
            input.len(),
            from,
            to,
            replacement_char,
            error_mode,
            &mut errors,
        )
    };
    // SAFETY: a fresh string from libmbfl.
    (unsafe { take(s) }, errors)
}

/// `php_unicode_convert_case`: `mode` is one of [`case_mode`].
pub fn convert_case(
    mode: i32,
    input: &[u8],
    src: &Encoding,
    dst: &Encoding,
    illegal_mode: u32,
    illegal_substchar: u32,
) -> Vec<u8> {
    assert!((0..case_mode::MAX).contains(&mode));
    // SAFETY: the C reads `input` and returns a new string.
    unsafe {
        take(php_unicode_convert_case(
            mode,
            input.as_ptr() as *const c_char,
            input.len(),
            src,
            dst,
            illegal_mode as c_int,
            illegal_substchar,
        ))
    }
}

/// `mbfl_strcut`: the bytes from byte `from` for `len` bytes, moved to
/// character boundaries. None when its buffer could not grow (memory_limit).
pub fn strcut(input: &[u8], enc: &Encoding, from: usize, len: usize) -> Option<Vec<u8>> {
    let mut string = MbflString {
        encoding: enc,
        val: input.as_ptr() as *mut u8,
        len: input.len(),
    };
    let mut result = MbflString {
        encoding: std::ptr::null(),
        val: std::ptr::null_mut(),
        len: 0,
    };
    // SAFETY: the C reads `input` and fills `result` with an emalloc'd copy.
    let r = unsafe { mbfl_strcut(&mut string, &mut result, from, len) };
    if r.is_null() {
        return None;
    }
    // SAFETY: `result.val` holds `result.len` bytes the caller owns.
    unsafe {
        let v = if result.val.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(result.val, result.len).to_vec()
        };
        rhp_csrc::rhp_csrc_efree(result.val as *mut c_void);
        Some(v)
    }
}

/// eaw_table.h: the ranges of double-width codepoints, sorted.
pub fn eaw_table() -> &'static [[i32; 2]] {
    let mut len = 0;
    // SAFETY: a static array of `len` entries.
    unsafe { std::slice::from_raw_parts(rhp_mbfl_eaw_table(&mut len), len) }
}

/// rare_cp_bitvec.h: one bit per codepoint up to U+FFFF, set for "rare"
/// ones (mbstring.c's estimate_demerits).
pub fn rare_cp_bitvec() -> &'static [u32] {
    let mut len = 0;
    // SAFETY: a static array of `len` entries.
    unsafe { std::slice::from_raw_parts(rhp_mbfl_rare_cp_bitvec(&mut len), len) }
}

#[cfg(test)]
mod tests;
