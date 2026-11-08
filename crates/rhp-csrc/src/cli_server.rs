//! sapi/cli's C for the built-in web server (`rhp -S`): php_http_parser,
//! the request parser php_cli_server.c feeds what it reads, and
//! mime_type_map.h, the extension → mime type table for static files.

use std::ffi::{c_char, c_int, c_void, CStr};

/// `enum php_http_method`, the values the server tests.
pub mod method {
    pub const DELETE: u8 = 0;
    pub const GET: u8 = 1;
    pub const HEAD: u8 = 2;
    pub const POST: u8 = 3;
    pub const PUT: u8 = 4;
    pub const PATCH: u8 = 5;
    pub const NOT_IMPLEMENTED: u8 = 26;
}

/// One parser callback (`php_http_parser_settings`), with its data.
#[derive(Debug, PartialEq, Eq)]
pub enum Event<'a> {
    MessageBegin,
    Path(&'a [u8]),
    QueryString(&'a [u8]),
    /// The URL, with the request's method (a [`method`] value).
    Url(&'a [u8], u8),
    Fragment(&'a [u8]),
    HeaderField(&'a [u8]),
    HeaderValue(&'a [u8]),
    HeadersComplete,
    Body(&'a [u8]),
    /// The request is complete; its `http_major * 100 + http_minor`.
    MessageComplete(i32),
}

type EventFn =
    unsafe extern "C" fn(*mut c_void, c_int, *const c_char, usize, c_int, c_int) -> c_int;

// glue/prefix_cli_server.h gives the parser's own symbols an rhp_ prefix;
// the glue's are named rhp_ already.
extern "C" {
    fn rhp_http_parser_size() -> usize;
    fn rhp_http_parser_init(p: *mut c_void);
    fn rhp_http_parser_execute(
        p: *mut c_void,
        cb: EventFn,
        ctx: *mut c_void,
        data: *const c_char,
        len: usize,
    ) -> usize;
    fn rhp_http_parser_method(p: *const c_void) -> c_int;
    fn rhp_http_parser_version(p: *const c_void) -> c_int;
    fn rhp_http_parser_at_request_start(p: *const c_void) -> c_int;
    fn rhp_http_method_name(m: c_int) -> *const c_char;
    fn rhp_mime_type_count() -> usize;
    fn rhp_mime_type_pair(i: usize, ext: *mut *const c_char, mime: *mut *const c_char);
}

/// A `php_http_parser` for requests (`PHP_HTTP_REQUEST`).
pub struct HttpParser {
    /// The C struct, in 8-byte aligned storage.
    raw: Box<[u64]>,
}

impl Default for HttpParser {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpParser {
    /// `php_http_parser_init(parser, PHP_HTTP_REQUEST)`.
    pub fn new() -> Self {
        // SAFETY: plain size query.
        let words = unsafe { rhp_http_parser_size() }.div_ceil(8);
        let mut raw = vec![0u64; words].into_boxed_slice();
        // SAFETY: raw holds a whole parser struct.
        unsafe { rhp_http_parser_init(raw.as_mut_ptr().cast()) };
        HttpParser { raw }
    }

    /// `php_http_parser_execute`: feed `data`, calling `on` for each event
    /// (returning false stops the parser with an error). Returns the bytes
    /// consumed: fewer than `data.len()` is a parse error.
    pub fn execute(&mut self, data: &[u8], on: &mut dyn FnMut(Event<'_>) -> bool) -> usize {
        unsafe extern "C" fn trampoline(
            ctx: *mut c_void,
            kind: c_int,
            at: *const c_char,
            len: usize,
            method: c_int,
            version: c_int,
        ) -> c_int {
            // SAFETY: ctx is the `&mut dyn FnMut` execute passed, alive for
            // the call; `at` points to `len` bytes of the buffer being parsed.
            let on = unsafe { &mut *(ctx as *mut &mut dyn FnMut(Event<'_>) -> bool) };
            let bytes: &[u8] = if at.is_null() {
                &[]
            } else {
                unsafe { std::slice::from_raw_parts(at.cast(), len) }
            };
            let ev = match kind {
                0 => Event::MessageBegin,
                1 => Event::Path(bytes),
                2 => Event::QueryString(bytes),
                3 => Event::Url(bytes, method as u8),
                4 => Event::Fragment(bytes),
                5 => Event::HeaderField(bytes),
                6 => Event::HeaderValue(bytes),
                7 => Event::HeadersComplete,
                8 => Event::Body(bytes),
                _ => Event::MessageComplete(version),
            };
            // A panic must not unwind into C.
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| on(ev))) {
                Ok(true) => 0,
                Ok(false) => 1,
                Err(_) => std::process::abort(),
            }
        }
        let mut on = on;
        let ctx = &mut on as *mut &mut dyn FnMut(Event<'_>) -> bool;
        // SAFETY: the parser struct is live; ctx outlives the call.
        unsafe {
            rhp_http_parser_execute(
                self.raw.as_mut_ptr().cast(),
                trampoline,
                ctx.cast(),
                data.as_ptr().cast(),
                data.len(),
            )
        }
    }

    /// The request's method (`parser->method`, a [`method`] value).
    pub fn method(&self) -> u8 {
        // SAFETY: the parser struct is live.
        unsafe { rhp_http_parser_method(self.raw.as_ptr().cast()) as u8 }
    }

    /// `http_major * 100 + http_minor`.
    pub fn protocol_version(&self) -> i32 {
        // SAFETY: the parser struct is live.
        unsafe { rhp_http_parser_version(self.raw.as_ptr().cast()) }
    }

    /// Whether the parser is still waiting for a request line
    /// (`parser.state == s_start_req`).
    pub fn at_request_start(&self) -> bool {
        // SAFETY: the parser struct is live.
        unsafe { rhp_http_parser_at_request_start(self.raw.as_ptr().cast()) != 0 }
    }
}

/// `php_http_method_str`: a method's name ("GET", …, "NOTIMPLEMENTED").
pub fn method_str(m: u8) -> &'static str {
    // SAFETY: the C returns a static string.
    let p = unsafe { rhp_http_method_name(m.min(method::NOT_IMPLEMENTED) as c_int) };
    unsafe { CStr::from_ptr(p) }
        .to_str()
        .unwrap_or("NOTIMPLEMENTED")
}

/// mime_type_map.h: (extension, mime type) pairs, in the table's order.
pub fn mime_types() -> impl Iterator<Item = (&'static [u8], &'static [u8])> {
    // SAFETY: plain size query.
    let n = unsafe { rhp_mime_type_count() };
    (0..n).map(|i| {
        let (mut ext, mut mime) = (std::ptr::null(), std::ptr::null());
        // SAFETY: i is in range; the C hands out static strings.
        unsafe {
            rhp_mime_type_pair(i, &mut ext, &mut mime);
            (
                CStr::from_ptr(ext).to_bytes(),
                CStr::from_ptr(mime).to_bytes(),
            )
        }
    })
}

#[cfg(test)]
mod tests;
