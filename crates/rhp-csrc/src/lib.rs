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
#[cfg(feature = "crypt")]
pub mod crypt;

use std::alloc::{alloc, dealloc, handle_alloc_error, realloc, Layout};
use std::ffi::c_void;

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
    fn emalloc_block_becomes_a_vec() {
        for (size, offset, len) in [(1usize, 0usize, 1usize), (64, 24, 40), (1 << 20, 24, 1000)] {
            let p = super::rhp_csrc_emalloc(size) as *mut u8;
            // SAFETY: the block holds `size` bytes, `offset + len` of them
            // written; it is not used after emalloc_into_vec.
            let v = unsafe {
                for i in 0..offset + len {
                    *p.add(i) = i as u8;
                }
                super::emalloc_into_vec(p as *mut c_void, offset, len)
            };
            assert_eq!(v.len(), len);
            assert!(v.iter().enumerate().all(|(i, &b)| b == (offset + i) as u8));
        }
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
