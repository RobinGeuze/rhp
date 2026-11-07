//! `PhpStr`: php's zend_string as one allocation.
//!
//! A `PhpStr` is a thin pointer to one heap block: a 24-byte header
//! (refcount, reserved flags, cached hash, length) followed by the bytes,
//! as zend_string keeps them (Zend/zend_types.h). Clone bumps the refcount;
//! mutation goes through [`StrMut`], which copies a shared string first.
//!
//! The block's size is always [`alloc_size`] of the length: php's
//! `_ZSTR_STRUCT_SIZE` rounded to the allocator's block, so the length
//! alone gives the capacity and `memory_get_usage()` counts what php's
//! zend_mm would. Growing within the block is free; growing past it
//! reallocates to the new length's block, as zend_string_extend does.
//!
//! The block is allocated with alignment 1 so that a `Vec<u8>`'s buffer
//! can become a string in place (realloc plus a move of the bytes, with no
//! second copy alive at once); the header is read and written unaligned.
//! All the unsafe code of strings is in this module.

use std::alloc::{alloc, dealloc, handle_alloc_error, realloc, Layout};
use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ptr::{self, NonNull};
use std::rc::Rc;

use bstr::{BStr, BString};

/// Header bytes before the string's bytes.
const HDR: usize = 24;
const OFF_RC: usize = 0;
// 4..8: reserved flags (php's GC type info: interned, immutable).
const OFF_HASH: usize = 8;
const OFF_LEN: usize = 16;
/// Owned buffers up to this length are copied, not realloc'd.
const COPY_MAX: usize = 4096;

/// The block size of a string of `len` bytes: zend_string's size
/// (header + bytes + NUL, 8-aligned) rounded as zend_mm rounds it.
#[inline]
fn alloc_size(len: usize) -> usize {
    crate::mem::block_size(crate::mem::zstr_size(len))
}

#[inline]
fn layout(size: usize) -> Layout {
    match Layout::from_size_align(size, 1) {
        Ok(l) => l,
        Err(_) => capacity_overflow(),
    }
}

#[cold]
#[inline(never)]
fn capacity_overflow() -> ! {
    panic!("string size overflow")
}

/// Owned, binary-safe, copy-on-write PHP string. Clone is O(1); mutation is
/// copy-on-write through [`PhpStr::make_mut`].
pub struct PhpStr {
    ptr: NonNull<u8>,
    /// Not `Send`/`Sync`: the refcount is not atomic.
    _rc: PhantomData<Rc<()>>,
}

impl PhpStr {
    // -- header ------------------------------------------------------------

    #[inline]
    fn rc(&self) -> u32 {
        // SAFETY: the block always starts with the header.
        unsafe { self.ptr.as_ptr().add(OFF_RC).cast::<u32>().read_unaligned() }
    }

    #[inline]
    fn set_rc(&self, rc: u32) {
        // SAFETY: as above; the header is never behind a reference.
        unsafe {
            self.ptr
                .as_ptr()
                .add(OFF_RC)
                .cast::<u32>()
                .write_unaligned(rc)
        }
    }

    #[inline]
    fn cached_hash(&self) -> u64 {
        // SAFETY: as above.
        unsafe {
            self.ptr
                .as_ptr()
                .add(OFF_HASH)
                .cast::<u64>()
                .read_unaligned()
        }
    }

    #[inline]
    fn set_hash(&self, h: u64) {
        // SAFETY: as above. Shared handles may write the hash: they are all
        // on this thread (`!Send`) and nothing borrows the header.
        unsafe {
            self.ptr
                .as_ptr()
                .add(OFF_HASH)
                .cast::<u64>()
                .write_unaligned(h)
        }
    }

    #[inline]
    fn set_len_field(&mut self, len: usize) {
        // SAFETY: as above.
        unsafe {
            self.ptr
                .as_ptr()
                .add(OFF_LEN)
                .cast::<usize>()
                .write_unaligned(len)
        }
    }

    #[inline]
    fn data(&self) -> *mut u8 {
        // SAFETY: the bytes follow the header in the same block.
        unsafe { self.ptr.as_ptr().add(HDR) }
    }

    // -- construction ------------------------------------------------------

    /// A fresh block for `len` bytes, refcount 1; the bytes are left
    /// uninitialized and must be written before the string is read.
    fn alloc_uninit(len: usize) -> PhpStr {
        let l = layout(alloc_size(len));
        // SAFETY: the size is at least HDR + 1.
        let p = unsafe { alloc(l) };
        let Some(ptr) = NonNull::new(p) else {
            handle_alloc_error(l)
        };
        let mut s = PhpStr {
            ptr,
            _rc: PhantomData,
        };
        // SAFETY: header bytes of the new block.
        unsafe { ptr.as_ptr().add(4).cast::<u32>().write_unaligned(0) };
        s.set_rc(1);
        s.set_hash(0);
        s.set_len_field(len);
        s
    }

    /// A string holding `b` (anything [`StrBytes`]: slices, `str`, `Vec`,
    /// `String`, …). An owned buffer becomes the string without a second
    /// copy being alive at once.
    #[inline]
    pub fn from_bytes(b: impl StrBytes) -> Self {
        b.into_php_str()
    }

    /// A copy of `b`.
    pub fn from_slice(b: &[u8]) -> PhpStr {
        let s = PhpStr::alloc_uninit(b.len());
        // SAFETY: the block holds `b.len()` bytes after the header.
        unsafe { ptr::copy_nonoverlapping(b.as_ptr(), s.data(), b.len()) };
        s
    }

    /// `a . b` in one allocation.
    pub fn concat(a: &[u8], b: &[u8]) -> PhpStr {
        let len = a
            .len()
            .checked_add(b.len())
            .unwrap_or_else(|| capacity_overflow());
        let s = PhpStr::alloc_uninit(len);
        // SAFETY: the block holds `len` bytes after the header.
        unsafe {
            ptr::copy_nonoverlapping(a.as_ptr(), s.data(), a.len());
            ptr::copy_nonoverlapping(b.as_ptr(), s.data().add(a.len()), b.len());
        }
        s
    }

    /// Take over a `Vec`'s buffer. A short one is copied (cheaper than a
    /// realloc); a long one is realloc'd to the string's block and its
    /// bytes moved behind the header, so no second copy is alive at once.
    #[inline]
    pub fn from_vec(v: Vec<u8>) -> PhpStr {
        if v.len() <= COPY_MAX {
            return PhpStr::from_slice(&v);
        }
        PhpStr::from_vec_in_place(v)
    }

    #[inline(never)]
    fn from_vec_in_place(v: Vec<u8>) -> PhpStr {
        let (cap, len) = (v.capacity(), v.len());
        let mut v = std::mem::ManuallyDrop::new(v);
        let old = v.as_mut_ptr();
        let new_size = alloc_size(len);
        // SAFETY: `old` was allocated by the global allocator with
        // `Layout::array::<u8>(cap)` (size `cap`, align 1), which is ours.
        let p = unsafe { realloc(old, layout(cap), new_size) };
        let Some(ptr) = NonNull::new(p) else {
            handle_alloc_error(layout(new_size))
        };
        // SAFETY: the new block has room for HDR + len; `copy` handles the
        // overlap.
        unsafe { ptr::copy(ptr.as_ptr(), ptr.as_ptr().add(HDR), len) };
        let mut s = PhpStr {
            ptr,
            _rc: PhantomData,
        };
        // SAFETY: header bytes of the block.
        unsafe { ptr.as_ptr().add(4).cast::<u32>().write_unaligned(0) };
        s.set_rc(1);
        s.set_hash(0);
        s.set_len_field(len);
        s
    }

    // -- reading -----------------------------------------------------------

    #[inline]
    pub fn len(&self) -> usize {
        // SAFETY: header read, see `rc`.
        unsafe {
            self.ptr
                .as_ptr()
                .add(OFF_LEN)
                .cast::<usize>()
                .read_unaligned()
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `len` initialized bytes follow the header; no `StrMut`
        // can exist while `self` is borrowed shared.
        unsafe { std::slice::from_raw_parts(self.data(), self.len()) }
    }

    /// How many values share this string (debug_zval_dump's refcount).
    #[inline]
    pub fn ref_count(&self) -> usize {
        self.rc() as usize
    }

    /// Whether both share one buffer.
    #[inline]
    pub fn ptr_eq(&self, other: &PhpStr) -> bool {
        self.ptr == other.ptr
    }

    /// The string's hash, computed once and cached in the header like
    /// zend_string's `h` (never 0, so 0 means "not computed").
    #[inline]
    pub fn hash_value(&self) -> u64 {
        let h = self.cached_hash();
        if h != 0 {
            return h;
        }
        self.compute_hash()
    }

    #[inline(never)]
    fn compute_hash(&self) -> u64 {
        let mut st = rustc_hash::FxHasher::default();
        st.write(self.as_bytes());
        let h = st.finish() | (1 << 63);
        self.set_hash(h);
        h
    }

    // -- writing -----------------------------------------------------------

    /// Copy-on-write access: copies the bytes first when shared.
    #[inline]
    pub fn make_mut(&mut self) -> StrMut<'_> {
        if self.rc() != 1 {
            *self = PhpStr::from_slice(self.as_bytes());
        }
        self.set_hash(0);
        StrMut { s: self }
    }

    /// Mutable access when this is the only handle to the string.
    #[inline]
    pub fn get_mut(&mut self) -> Option<StrMut<'_>> {
        if self.rc() != 1 {
            return None;
        }
        self.set_hash(0);
        Some(StrMut { s: self })
    }

    /// Change the length of a unique string, reallocating when the block
    /// size changes. New bytes are uninitialized.
    fn set_len(&mut self, new: usize) {
        debug_assert_eq!(self.rc(), 1);
        let old = self.len();
        let (from, to) = (alloc_size(old), alloc_size(new));
        if from != to {
            let to_l = layout(to);
            // SAFETY: the block was allocated with `layout(from)`.
            let p = unsafe { realloc(self.ptr.as_ptr(), layout(from), to_l.size()) };
            match NonNull::new(p) {
                Some(p) => self.ptr = p,
                None => handle_alloc_error(to_l),
            }
        }
        self.set_len_field(new);
    }
}

impl Clone for PhpStr {
    #[inline]
    fn clone(&self) -> Self {
        let rc = self.rc();
        if rc == u32::MAX {
            std::process::abort();
        }
        self.set_rc(rc + 1);
        PhpStr {
            ptr: self.ptr,
            _rc: PhantomData,
        }
    }
}

impl Drop for PhpStr {
    #[inline]
    fn drop(&mut self) {
        let rc = self.rc();
        if rc > 1 {
            self.set_rc(rc - 1);
        } else {
            self.free();
        }
    }
}

impl PhpStr {
    /// The last handle went away.
    #[inline(never)]
    fn free(&mut self) {
        let size = alloc_size(self.len());
        // SAFETY: the last handle; the block has this layout.
        unsafe { dealloc(self.ptr.as_ptr(), layout(size)) }
    }
}

impl PartialEq for PhpStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || self.eq_bytes(other)
    }
}

impl PhpStr {
    /// Out of line: keeps hash-table probes that compare keys small.
    #[inline(never)]
    fn eq_bytes(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }
        let (a, b) = (self.cached_hash(), other.cached_hash());
        if a != 0 && b != 0 && a != b {
            return false;
        }
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for PhpStr {}

impl Hash for PhpStr {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash_value());
    }
}

impl PartialOrd for PhpStr {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PhpStr {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl Default for PhpStr {
    fn default() -> Self {
        PhpStr::from_slice(&[])
    }
}

impl From<&[u8]> for PhpStr {
    fn from(b: &[u8]) -> Self {
        PhpStr::from_slice(b)
    }
}

impl From<&str> for PhpStr {
    fn from(s: &str) -> Self {
        PhpStr::from_slice(s.as_bytes())
    }
}

impl std::fmt::Debug for PhpStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", BStr::new(self.as_bytes()))
    }
}

/// What [`PhpStr::from_bytes`] takes: borrowed bytes are copied, owned
/// buffers are taken over.
pub trait StrBytes {
    fn into_php_str(self) -> PhpStr;
}

impl StrBytes for Vec<u8> {
    #[inline]
    fn into_php_str(self) -> PhpStr {
        PhpStr::from_vec(self)
    }
}

impl StrBytes for String {
    #[inline]
    fn into_php_str(self) -> PhpStr {
        PhpStr::from_vec(self.into_bytes())
    }
}

impl StrBytes for BString {
    #[inline]
    fn into_php_str(self) -> PhpStr {
        PhpStr::from_vec(self.into())
    }
}

impl StrBytes for Box<[u8]> {
    #[inline]
    fn into_php_str(self) -> PhpStr {
        PhpStr::from_vec(self.into_vec())
    }
}

impl StrBytes for Cow<'_, [u8]> {
    #[inline]
    fn into_php_str(self) -> PhpStr {
        match self {
            Cow::Borrowed(b) => PhpStr::from_slice(b),
            Cow::Owned(v) => PhpStr::from_vec(v),
        }
    }
}

impl StrBytes for Cow<'_, str> {
    #[inline]
    fn into_php_str(self) -> PhpStr {
        match self {
            Cow::Borrowed(b) => PhpStr::from_slice(b.as_bytes()),
            Cow::Owned(v) => PhpStr::from_vec(v.into_bytes()),
        }
    }
}

impl StrBytes for &[u8] {
    #[inline]
    fn into_php_str(self) -> PhpStr {
        PhpStr::from_slice(self)
    }
}

impl StrBytes for &mut [u8] {
    #[inline]
    fn into_php_str(self) -> PhpStr {
        PhpStr::from_slice(self)
    }
}

impl StrBytes for &str {
    #[inline]
    fn into_php_str(self) -> PhpStr {
        PhpStr::from_slice(self.as_bytes())
    }
}

impl StrBytes for &String {
    #[inline]
    fn into_php_str(self) -> PhpStr {
        PhpStr::from_slice(self.as_bytes())
    }
}

impl StrBytes for &Vec<u8> {
    #[inline]
    fn into_php_str(self) -> PhpStr {
        PhpStr::from_slice(self)
    }
}

impl StrBytes for &BStr {
    #[inline]
    fn into_php_str(self) -> PhpStr {
        PhpStr::from_slice(self)
    }
}

impl<const N: usize> StrBytes for [u8; N] {
    #[inline]
    fn into_php_str(self) -> PhpStr {
        PhpStr::from_slice(&self)
    }
}

impl<const N: usize> StrBytes for &[u8; N] {
    #[inline]
    fn into_php_str(self) -> PhpStr {
        PhpStr::from_slice(self)
    }
}

/// A string of a known size built from pieces in its own block (no `Vec`
/// in between), like filling a zend_string_alloc'd string.
pub struct StrBuilder {
    /// Its length field is the capacity until `finish`.
    s: PhpStr,
    at: usize,
}

impl PhpStr {
    /// A builder for a string of at most `cap` bytes.
    pub fn builder(cap: usize) -> StrBuilder {
        StrBuilder {
            s: PhpStr::alloc_uninit(cap),
            at: 0,
        }
    }
}

impl StrBuilder {
    /// Append `b`; panics past the capacity.
    #[inline]
    pub fn push(&mut self, b: &[u8]) {
        assert!(b.len() <= self.s.len() - self.at, "StrBuilder overflow");
        // SAFETY: in bounds of the block, checked above.
        unsafe { ptr::copy_nonoverlapping(b.as_ptr(), self.s.data().add(self.at), b.len()) };
        self.at += b.len();
    }

    /// The string of the bytes pushed.
    pub fn finish(mut self) -> PhpStr {
        if self.at != self.s.len() {
            self.s.set_len(self.at);
        }
        self.s
    }
}

/// Mutable access to a string no other value shares (from
/// [`PhpStr::make_mut`] / [`PhpStr::get_mut`]). Derefs to the bytes; the
/// methods that change the length keep the block at the length's size.
pub struct StrMut<'a> {
    s: &'a mut PhpStr,
}

impl StrMut<'_> {
    /// Append bytes (zend_string_extend + memcpy).
    pub fn extend_from_slice(&mut self, b: &[u8]) {
        let old = self.s.len();
        let new = old
            .checked_add(b.len())
            .unwrap_or_else(|| capacity_overflow());
        self.s.set_len(new);
        // SAFETY: the block now holds `new` bytes; `b` can't point into it
        // (that would need a shared borrow of the string).
        unsafe { ptr::copy_nonoverlapping(b.as_ptr(), self.s.data().add(old), b.len()) };
    }

    /// Append the string to itself (`$s .= $s`).
    pub fn extend_self(&mut self) {
        let old = self.s.len();
        let new = old.checked_mul(2).unwrap_or_else(|| capacity_overflow());
        self.s.set_len(new);
        // SAFETY: the first `old` bytes are initialized; the two halves
        // don't overlap.
        unsafe { ptr::copy_nonoverlapping(self.s.data(), self.s.data().add(old), old) };
    }

    pub fn push(&mut self, c: u8) {
        self.extend_from_slice(&[c]);
    }

    /// Set the length, filling new bytes with `fill`.
    pub fn resize(&mut self, len: usize, fill: u8) {
        let old = self.s.len();
        self.s.set_len(len);
        if len > old {
            // SAFETY: the block now holds `len` bytes.
            unsafe { ptr::write_bytes(self.s.data().add(old), fill, len - old) };
        }
    }

    pub fn truncate(&mut self, len: usize) {
        if len < self.s.len() {
            self.s.set_len(len);
        }
    }
}

impl std::ops::Deref for StrMut<'_> {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        self.s.as_bytes()
    }
}

impl std::ops::DerefMut for StrMut<'_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [u8] {
        // SAFETY: unique handle (refcount 1, `&mut` borrow); `len`
        // initialized bytes follow the header.
        unsafe { std::slice::from_raw_parts_mut(self.s.data(), self.s.len()) }
    }
}

#[cfg(test)]
mod tests;
