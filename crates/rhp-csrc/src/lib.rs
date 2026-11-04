//! C that rhp compiles from source and links statically: php-src's own C at
//! the pinned tag (`vendor/php-src`, written by `tools/sync-php-src-c.sh`)
//! and frozen libraries whose only release is the one php links
//! (`vendor/argon2`). All php-src C shares one Zend shim, `shim/zend_shim.h`.
//!
//! One cargo feature per library. Each module wraps its C in safe functions
//! that take what php passes and return what php's C returns. Every global
//! symbol the C defines gets an rhp_ prefix at compile time
//! (`glue/prefix_<lib>.h`), so other static C libraries can't clash with it.

#[cfg(feature = "argon2")]
pub mod argon2;
#[cfg(feature = "crypt")]
pub mod crypt;

use std::alloc::{alloc, dealloc, handle_alloc_error, realloc, Layout};
use std::ffi::c_void;

/// Bytes before each block emalloc hands out: the block's layout size, for
/// efree (C frees by pointer only). 16 keeps malloc's alignment.
const EMALLOC_HEADER: usize = 16;

/// The layout of a block holding `size` bytes after the header.
fn block_layout(size: usize) -> Layout {
    size.max(1)
        .checked_add(EMALLOC_HEADER)
        .and_then(|n| Layout::from_size_align(n, EMALLOC_HEADER).ok())
        .unwrap_or_else(|| handle_alloc_error(Layout::new::<[u8; EMALLOC_HEADER]>()))
}

/// zend_shim.h's `emalloc`: through Rust's global allocator, so vendored C
/// counts against `memory_limit` like php's own emalloc. Never NULL.
#[no_mangle]
pub extern "C" fn rhp_csrc_emalloc(size: usize) -> *mut c_void {
    let layout = block_layout(size);
    // SAFETY: the layout has a non-zero size; the header fits in front of
    // the block and keeps it 16-byte aligned.
    unsafe {
        let base = alloc(layout);
        if base.is_null() {
            handle_alloc_error(layout);
        }
        (base as *mut usize).write(layout.size());
        base.add(EMALLOC_HEADER) as *mut c_void
    }
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
    let size = (base as *const usize).read();
    dealloc(
        base,
        Layout::from_size_align_unchecked(size, EMALLOC_HEADER),
    );
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
        ((ptr as *mut u8).sub(EMALLOC_HEADER) as *const usize).read() - EMALLOC_HEADER
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
    let old = Layout::from_size_align_unchecked((base as *const usize).read(), EMALLOC_HEADER);
    let new = block_layout(size);
    let base = realloc(base, old, new.size());
    if base.is_null() {
        handle_alloc_error(new);
    }
    (base as *mut usize).write(new.size());
    base.add(EMALLOC_HEADER) as *mut c_void
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

#[cfg(test)]
mod tests {
    use std::ffi::c_void;
    use std::path::Path;

    #[test]
    fn emalloc_blocks_round_trip() {
        for size in [0usize, 1, 15, 16, 4096, 1 << 20] {
            let p = super::rhp_csrc_emalloc(size) as *mut u8;
            assert_eq!(p as usize % 16, 0);
            // SAFETY: the block holds at least `size` bytes.
            unsafe {
                std::ptr::write_bytes(p, 0xab, size);
                let q = super::rhp_csrc_erealloc(p as *mut c_void, size * 2 + 1) as *mut u8;
                assert_eq!(q as usize % 16, 0);
                assert!((0..size).all(|i| *q.add(i) == 0xab));
                super::rhp_csrc_efree(q as *mut c_void);
            }
        }
        // SAFETY: NULL is a no-op, as in php.
        unsafe { super::rhp_csrc_efree(std::ptr::null_mut()) };
    }

    #[test]
    fn fallible_realloc_stops_at_the_limit() {
        use rhp_values::mem;
        std::thread::spawn(|| {
            mem::begin_request();
            mem::force_limit(4 * mem::CHUNK_SIZE);
            let p = super::rhp_csrc_emalloc(16);
            // SAFETY: p came from rhp_csrc_emalloc.
            unsafe {
                let q = super::rhp_csrc_safe_ferealloc(p, 1, 1 << 20, 24);
                assert!(!q.is_null());
                let r = super::rhp_csrc_safe_ferealloc(q, 1, 64 << 20, 24);
                assert!(r.is_null());
                assert!(mem::state().attention());
                assert!(super::rhp_csrc_safe_ferealloc(q, usize::MAX, 2, 0).is_null());
                super::rhp_csrc_efree(q);
            }
            assert_eq!(super::rhp_csrc_safe_address(usize::MAX, 2, 0), usize::MAX);
            mem::end_request();
        })
        .join()
        .unwrap();
    }
    use std::process::Command;

    /// Every global symbol the C archives define carries the rhp_ prefix
    /// (glue/prefix_<lib>.h), so another static C library can't clash with
    /// ours or bind to it. Hidden symbols count: visibility doesn't matter in
    /// a static link.
    #[test]
    fn archives_define_only_prefixed_symbols() {
        let out = Path::new(env!("OUT_DIR"));
        let mut seen = Vec::new();
        let mut bad = Vec::new();
        for entry in std::fs::read_dir(out).unwrap() {
            let path = entry.unwrap().path();
            if path.extension() != Some("a".as_ref()) {
                continue;
            }
            seen.push(path.file_name().unwrap().to_string_lossy().into_owned());
            // -P: "name type [value size]", plus "archive[member]:" lines.
            let nm = Command::new("nm")
                .args(["-g", "-P"])
                .arg(&path)
                .output()
                .expect("this test runs nm (binutils or Xcode command line tools)");
            assert!(nm.status.success(), "nm {}: {:?}", path.display(), nm);
            for line in String::from_utf8_lossy(&nm.stdout).lines() {
                let mut fields = line.split_whitespace();
                let (Some(name), Some(ty)) = (fields.next(), fields.next()) else {
                    continue;
                };
                if matches!(ty, "U" | "w" | "v") {
                    continue;
                }
                let name = if cfg!(target_vendor = "apple") {
                    name.strip_prefix('_').unwrap_or(name)
                } else {
                    name
                };
                if !name.starts_with("rhp_") {
                    bad.push(format!("{}: {ty} {name}", path.display()));
                }
            }
        }
        seen.sort();
        let mut want = Vec::new();
        if cfg!(feature = "argon2") {
            want.push("librhp_csrc_argon2.a");
        }
        if cfg!(feature = "crypt") {
            want.push("librhp_csrc_crypt.a");
        }
        want.push("librhp_csrc_zend.a");
        assert_eq!(seen, want);
        assert!(bad.is_empty(), "unprefixed symbols:\n{}", bad.join("\n"));
    }
}
