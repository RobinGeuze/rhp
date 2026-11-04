//! libc `inet_pton` / `inet_ntop`, which php calls for every textual IP
//! address. Results follow the platform, as php's do: macOS accepts leading
//! zeros and `%scope` suffixes and prints `::1.2.3.4`, glibc differs.

use std::ffi::{c_char, c_int, c_void, CString};

// The libc crate doesn't bind these.
extern "C" {
    fn inet_pton(af: c_int, src: *const c_char, dst: *mut c_void) -> c_int;
    fn inet_ntop(
        af: c_int,
        src: *const c_void,
        dst: *mut c_char,
        size: libc::socklen_t,
    ) -> *const c_char;
}

/// `inet_pton(AF_INET, text)`: the address in network order. Text containing
/// a NUL byte fails (php callers cut or reject it first).
pub(crate) fn pton4(text: &[u8]) -> Option<[u8; 4]> {
    pton(libc::AF_INET, text)
}

/// `inet_pton(AF_INET6, text)`.
pub(crate) fn pton6(text: &[u8]) -> Option<[u8; 16]> {
    pton(libc::AF_INET6, text)
}

fn pton<const N: usize>(af: c_int, text: &[u8]) -> Option<[u8; N]> {
    let text = CString::new(text).ok()?;
    let mut out = [0u8; N];
    // SAFETY: `text` is NUL-terminated and `out` is the size of an in_addr
    // (N = 4) or in6_addr (N = 16), matching `af`.
    let rc = unsafe { inet_pton(af, text.as_ptr(), out.as_mut_ptr().cast()) };
    (rc == 1).then_some(out)
}

/// `inet_ntop` of a 4-byte (AF_INET) or 16-byte (AF_INET6) address.
pub(crate) fn ntop(addr: &[u8]) -> Option<Vec<u8>> {
    let af = match addr.len() {
        4 => libc::AF_INET,
        16 => libc::AF_INET6,
        _ => return None,
    };
    // INET6_ADDRSTRLEN.
    let mut buf = [0 as c_char; 46];
    // SAFETY: `addr` holds an address of `af`'s size and `buf` is writable
    // for the length passed.
    let p = unsafe {
        inet_ntop(
            af,
            addr.as_ptr().cast(),
            buf.as_mut_ptr(),
            buf.len() as libc::socklen_t,
        )
    };
    if p.is_null() {
        return None;
    }
    // SAFETY: inet_ntop NUL-terminated `buf` on success.
    Some(
        unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }
            .to_bytes()
            .to_vec(),
    )
}

#[cfg(test)]
mod tests;
