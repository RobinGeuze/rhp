//! C that rhp compiles from source and links statically: php-src's own C at
//! the pinned tag (`vendor/php-src`, written by `tools/sync-php-src-c.sh`)
//! and frozen libraries whose only release is the one php links
//! (`vendor/argon2`). All php-src C shares one Zend shim, `shim/zend_shim.h`,
//! rhp-mbfl-sys's included: its out-of-line symbols (`zend_empty_string`,
//! the emalloc family below) are defined here only.
//!
//! One cargo feature per library. Each module wraps its C in safe functions
//! that take what php passes and return what php's C returns. Every global
//! symbol the C defines gets an rhp_ prefix at compile time
//! (`glue/prefix_<lib>.h`), so other static C libraries can't clash with it.

#[cfg(feature = "argon2")]
pub mod argon2;
#[cfg(feature = "avifinfo")]
pub mod avifinfo;
#[cfg(feature = "calendar")]
pub mod calendar;
#[cfg(feature = "cli_server")]
pub mod cli_server;
#[cfg(feature = "crypt")]
pub mod crypt;
#[cfg(feature = "fileinfo")]
pub mod fileinfo;
#[cfg(feature = "lexbor")]
pub mod lexbor;
#[cfg(feature = "timelib")]
pub mod timelib;

use std::alloc::{alloc, dealloc, handle_alloc_error, realloc, Layout};
use std::ffi::{c_char, c_int, c_void};

/// Bytes before each block emalloc hands out: the block's layout (size and
/// alignment), for efree (C frees by pointer only). 16 keeps malloc's
/// alignment.
const EMALLOC_HEADER: usize = 16;
/// The alignment C gets from emalloc, as from malloc.
const EMALLOC_ALIGN: usize = 16;

/// The bytes a block holding `size` bytes after the header takes.
fn block_total(size: usize) -> usize {
    size.max(1)
        .checked_add(EMALLOC_HEADER)
        .filter(|&n| n <= isize::MAX as usize - EMALLOC_ALIGN)
        .unwrap_or_else(|| handle_alloc_error(Layout::new::<[u8; EMALLOC_HEADER]>()))
}

/// # Safety
///
/// `base` is the start of a block from [`alloc_block`].
unsafe fn block_layout(base: *const u8) -> Layout {
    let p = base as *const usize;
    Layout::from_size_align_unchecked(p.read(), p.add(1).read())
}

/// # Safety
///
/// `base` points to at least [`EMALLOC_HEADER`] writable, aligned bytes.
unsafe fn set_block_layout(base: *mut u8, layout: Layout) {
    let p = base as *mut usize;
    p.write(layout.size());
    p.add(1).write(layout.align());
}

/// A block of `total` bytes, header included. Blocks are requested
/// byte-aligned, so that one can become a `Vec<u8>` ([`emalloc_into_vec`]);
/// the system allocator returns malloc-aligned memory for them anyway, and
/// a block that comes back less aligned is allocated again 16-aligned.
///
/// # Safety
///
/// `total` comes from [`block_total`].
unsafe fn alloc_block(total: usize) -> *mut u8 {
    let bytes = Layout::from_size_align_unchecked(total, 1);
    let base = alloc(bytes);
    if base.is_null() {
        handle_alloc_error(bytes);
    }
    if (base as usize).is_multiple_of(EMALLOC_ALIGN) {
        set_block_layout(base, bytes);
        return base;
    }
    dealloc(base, bytes);
    let aligned = Layout::from_size_align_unchecked(total, EMALLOC_ALIGN);
    let base = alloc(aligned);
    if base.is_null() {
        handle_alloc_error(aligned);
    }
    set_block_layout(base, aligned);
    base
}

/// zend_shim.h's `emalloc`: through Rust's global allocator, so vendored C
/// counts against `memory_limit` like php's own emalloc. Never NULL.
#[no_mangle]
pub extern "C" fn rhp_csrc_emalloc(size: usize) -> *mut c_void {
    // SAFETY: the header fits in front of the block and keeps it 16-byte
    // aligned.
    unsafe { alloc_block(block_total(size)).add(EMALLOC_HEADER) as *mut c_void }
}

/// zend_shim.h's `efree`, for blocks from [`rhp_csrc_emalloc`].
///
/// # Safety
///
/// `ptr` is NULL or came from `rhp_csrc_emalloc` and is not used again.
#[no_mangle]
pub unsafe extern "C" fn rhp_csrc_efree(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    let base = (ptr as *mut u8).sub(EMALLOC_HEADER);
    dealloc(base, block_layout(base));
}

/// The bytes `ptr`'s block holds after the header (0 for NULL).
///
/// # Safety
///
/// `ptr` is NULL or a live block from [`rhp_csrc_emalloc`].
unsafe fn block_size(ptr: *mut c_void) -> usize {
    if ptr.is_null() {
        0
    } else {
        block_layout((ptr as *mut u8).sub(EMALLOC_HEADER)).size() - EMALLOC_HEADER
    }
}

/// zend_shim.h's `erealloc`: like php's, it never fails (a crossing of
/// `memory_limit` is recorded for the VM).
///
/// # Safety
///
/// `ptr` is NULL or came from `rhp_csrc_emalloc` and is not used again.
#[no_mangle]
pub unsafe extern "C" fn rhp_csrc_erealloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    if ptr.is_null() {
        return rhp_csrc_emalloc(size);
    }
    let base = (ptr as *mut u8).sub(EMALLOC_HEADER);
    let old = block_layout(base);
    let total = block_total(size);
    let mut base = realloc(base, old, total);
    if base.is_null() {
        handle_alloc_error(Layout::from_size_align_unchecked(total, old.align()));
    }
    let new = Layout::from_size_align_unchecked(total, old.align());
    if (base as usize).is_multiple_of(EMALLOC_ALIGN) {
        set_block_layout(base, new);
    } else {
        // A byte-aligned block that moved to a less aligned address.
        let moved = alloc_block(total);
        std::ptr::copy_nonoverlapping(
            base.add(EMALLOC_HEADER),
            moved.add(EMALLOC_HEADER),
            old.size().min(total) - EMALLOC_HEADER,
        );
        dealloc(base, new);
        base = moved;
    }
    base.add(EMALLOC_HEADER) as *mut c_void
}

/// `len` bytes at `ptr + offset` in an emalloc block, as a `Vec<u8>`. A
/// byte-aligned block (the usual case) becomes the vector's buffer: the
/// bytes move to its start and nothing is allocated or copied elsewhere,
/// so a C result is held once. Other blocks are copied and freed.
///
/// # Safety
///
/// `ptr` came from `rhp_csrc_emalloc`, `offset + len` bytes of it are
/// initialised, and it is not used again.
pub unsafe fn emalloc_into_vec(ptr: *mut c_void, offset: usize, len: usize) -> Vec<u8> {
    let base = (ptr as *mut u8).sub(EMALLOC_HEADER);
    let layout = block_layout(base);
    let src = (ptr as *const u8).add(offset);
    if layout.align() == 1 {
        std::ptr::copy(src, base, len);
        // The block was allocated as `Layout::array::<u8>(layout.size())`.
        Vec::from_raw_parts(base, len, layout.size())
    } else {
        let v = std::slice::from_raw_parts(src, len).to_vec();
        rhp_csrc_efree(ptr);
        v
    }
}

/// zend_shim.h's `zend_safe_address_guarded`: `nmemb * size + offset`. On
/// overflow it records php's "Possible integer overflow in memory
/// allocation" error and returns `usize::MAX`.
#[no_mangle]
pub extern "C" fn rhp_csrc_safe_address(nmemb: usize, size: usize, offset: usize) -> usize {
    match nmemb.checked_mul(size).and_then(|b| b.checked_add(offset)) {
        Some(n) => n,
        None => {
            rhp_values::mem::reserve_safe(nmemb, size, offset);
            usize::MAX
        }
    }
}

/// zend_shim.h's `safe_ferealloc` (always fallible): NULL, with `ptr` left
/// alone, on overflow or when the growth would cross `memory_limit`; the
/// error is recorded for the VM, as php's fallible heap delays it.
///
/// # Safety
///
/// `ptr` is NULL or came from `rhp_csrc_emalloc`; on success it is not used
/// again.
#[no_mangle]
pub unsafe extern "C" fn rhp_csrc_safe_ferealloc(
    ptr: *mut c_void,
    nmemb: usize,
    size: usize,
    offset: usize,
) -> *mut c_void {
    let Some(n) = nmemb.checked_mul(size).and_then(|b| b.checked_add(offset)) else {
        rhp_values::mem::reserve_safe(nmemb, size, offset);
        return std::ptr::null_mut();
    };
    if !rhp_values::mem::reserve_realloc(block_size(ptr), n) {
        return std::ptr::null_mut();
    }
    rhp_csrc_erealloc(ptr, n)
}

/// Copy `bytes` and a NUL into `buf` (`cap` bytes), cut to fit; the length
/// copied.
///
/// # Safety
///
/// `buf` holds `cap` writable bytes, `cap > 0`.
// c_char is i8 or u8 depending on the target.
#[allow(clippy::unnecessary_cast)]
unsafe fn put_c(bytes: &[u8], buf: *mut c_char, cap: usize) -> usize {
    let n = bytes.len().min(cap - 1);
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, n);
    *buf.add(n) = 0;
    n
}

/// shim/php_printf.c's `php_conv_fp` (main/snprintf.c), for a finite
/// `num`: the digits in `buf` (NUL-terminated) and their length.
///
/// # Safety
///
/// `buf` holds `cap` writable bytes, `cap > 0`; `is_negative` is writable.
// c_char is i8 or u8 depending on the target.
#[allow(clippy::unnecessary_cast)]
#[no_mangle]
pub unsafe extern "C" fn rhp_csrc_conv_fp(
    format: c_char,
    num: f64,
    add_dp: bool,
    precision: c_int,
    dec_point: c_char,
    is_negative: *mut bool,
    buf: *mut c_char,
    cap: usize,
) -> usize {
    let (digits, neg) = rhp_values::fmt::conv_fp(
        format as u8,
        num,
        precision.max(0) as usize,
        add_dp,
        dec_point as u8,
    );
    *is_negative = neg;
    put_c(&digits, buf, cap)
}

/// shim/php_printf.c's `zend_gcvt(value, ndigit, dec_point, exp_char, buf)`.
///
/// # Safety
///
/// `buf` holds `cap` writable bytes, `cap > 0`.
// c_char is i8 or u8 depending on the target.
#[allow(clippy::unnecessary_cast)]
#[no_mangle]
pub unsafe extern "C" fn rhp_csrc_gcvt(
    value: f64,
    ndigit: c_int,
    dec_point: c_char,
    exp_char: c_char,
    buf: *mut c_char,
    cap: usize,
) -> usize {
    let mut s = rhp_values::fmt::zend_gcvt(value, ndigit as i64, exp_char as u8);
    if dec_point as u8 != b'.' {
        s.iter_mut()
            .filter(|c| **c == b'.')
            .for_each(|c| *c = dec_point as u8);
    }
    put_c(&s, buf, cap)
}

#[cfg(test)]
mod tests;
