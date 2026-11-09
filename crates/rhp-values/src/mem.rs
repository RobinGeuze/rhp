//! Per-thread memory accounting: `memory_get_usage()`, `memory_limit`.
//!
//! [`CountingAlloc`] is the global allocator of every binary that runs PHP.
//! It forwards to the system allocator and, while a request is active on the
//! current thread, keeps zend_mm's statistics (Zend/zend_alloc.c) for it:
//!
//! - `size` / `peak`: bytes in use, each block rounded about as zend_mm
//!   rounds it (16-byte granules up to 3 KiB, 4 KiB pages up to a chunk,
//!   page-aligned huge blocks);
//! - `real` / `real_peak`: memory taken from the system: 2 MiB chunks for the
//!   small and large blocks (an emptied chunk stays, as zend_mm caches it,
//!   until a lower limit needs it back) plus the huge blocks, so
//!   `memory_get_usage(true)` and the limit move in the same steps as php's.
//!
//! The allocator never fails or unwinds because of the limit (a global
//! allocator can't): crossing `memory_limit` records the size that crossed
//! it, and the VM raises php's "Allowed memory size of N bytes exhausted"
//! fatal error at its next safe point, as php 8.5 delays it to the VM's next
//! interrupt check. Builtins that know a large size up front call [`reserve`]
//! first and refuse, as php's fallible allocations do. Operations that
//! allocate a size they know but can't refuse (concatenation) call
//! [`alloc_check`] / [`realloc_check`]: they record the crossing with php's
//! size, and stop at php's hard limit instead of allocating.
//!
//! Counting starts at [`begin_request`], so engine startup and the prelude
//! don't count. A block freed by a thread other than the one that allocated
//! it, or freed in a request after being allocated before it, is subtracted
//! from the freeing thread's counters; they saturate at zero instead of
//! wrapping, so such frees can only make the counts read low.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

/// zend_mm's chunk: real usage grows in these steps.
pub const CHUNK_SIZE: usize = 2 * 1024 * 1024;
/// zend_mm's page.
const PAGE_SIZE: usize = 4096;
/// Largest small block (zend_mm serves these from bins).
const MAX_SMALL: usize = 3072;
/// Largest block served from a chunk's pages (ZEND_MM_MAX_LARGE_SIZE); the
/// first page of a chunk holds its header.
const MAX_LARGE: usize = CHUNK_SIZE - PAGE_SIZE;
/// ZEND_MM_LIMIT_MAX: what an unlimited (`-1`) memory_limit becomes.
pub const LIMIT_MAX: usize = usize::MAX >> 1;
/// php's default `memory_limit` (128M).
const DEFAULT_LIMIT: usize = 128 * 1024 * 1024;

/// A limit crossing the VM has not reported yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Oom {
    /// `Allowed memory size of {limit} bytes exhausted (tried to allocate {tried} bytes)`.
    Limit { limit: usize, tried: usize },
    /// `Possible integer overflow in memory allocation ({n} * {size} + {offset})`.
    Overflow {
        n: usize,
        size: usize,
        offset: usize,
    },
    /// `Hard memory limit of {limit} bytes exhausted (tried to allocate
    /// {tried} bytes)`: php prints it to stderr and exits at once.
    Hard { limit: usize, tried: usize },
}

impl Oom {
    /// The fatal error's message.
    pub fn message(&self) -> String {
        match *self {
            Oom::Limit { limit, tried } => {
                format!("Allowed memory size of {limit} bytes exhausted (tried to allocate {tried} bytes)")
            }
            Oom::Overflow { n, size, offset } => {
                format!("Possible integer overflow in memory allocation ({n} * {size} + {offset})")
            }
            Oom::Hard { limit, tried } => {
                format!("Hard memory limit of {limit} bytes exhausted (tried to allocate {tried} bytes)")
            }
        }
    }
}

/// One thread's counters. Only [`CountingAlloc`] and this module write the
/// usage fields; the VM reads [`MemState::attention`] between instructions.
pub struct MemState {
    active: Cell<bool>,
    /// Peak of zend_mm's `size` (`paged + huge`).
    peak: Cell<usize>,
    /// Bytes of small and large blocks (served from chunks).
    paged: Cell<usize>,
    /// Chunks taken from the system, in use or cached: at least the main
    /// chunk.
    chunks: Cell<usize>,
    /// Bytes of huge blocks.
    huge: Cell<usize>,
    real_peak: Cell<usize>,
    limit: Cell<usize>,
    /// `memory_hard_limit_percent` (-1: php's default).
    hard_percent: Cell<i64>,
    /// A crossing waits for the VM ([`MemState::take_oom`]) or is being
    /// reported: the VM's cheap per-instruction check.
    attention: Cell<bool>,
    /// A cycle collection is due ([`crate::gc`]).
    gc_due: Cell<bool>,
    /// `attention || gc_due`: the one flag the VM reads per instruction.
    tick: Cell<bool>,
    pending: Cell<Option<Oom>>,
    /// An OOM was recorded this request (php's `ZEND_HAS_OOM()`): later
    /// crossings are not recorded until [`MemState::reset_oom`].
    latched: Cell<bool>,
    /// Compiling (zend_is_compiling): a new chunk or huge block checks the
    /// hard limit too, as php's non-fallible emalloc does ([`compiling`]).
    compiling: Cell<bool>,
    /// `paged` above this takes the slow path: a new peak or another chunk.
    /// Never above either threshold ([`MemState::update_mark`]).
    mark: Cell<usize>,
}

thread_local! {
    static STATE: MemState = const {
        MemState {
            active: Cell::new(false),
            peak: Cell::new(0),
            paged: Cell::new(0),
            chunks: Cell::new(1),
            huge: Cell::new(0),
            real_peak: Cell::new(CHUNK_SIZE),
            limit: Cell::new(DEFAULT_LIMIT),
            hard_percent: Cell::new(-1),
            attention: Cell::new(false),
            gc_due: Cell::new(false),
            tick: Cell::new(false),
            pending: Cell::new(None),
            latched: Cell::new(false),
            compiling: Cell::new(false),
            mark: Cell::new(0),
        }
    };
}

/// The current thread's counters, for a caller that checks them often (the
/// VM keeps this reference instead of looking up the thread-local each time).
/// The state lives as long as the thread and is never moved; values that
/// hold it are tied to the thread anyway (the engine is not `Send`).
pub fn state() -> &'static MemState {
    STATE.with(|s| {
        // SAFETY: a `const`-initialised thread-local without a destructor
        // stays valid, at a fixed address, until the thread exits, and the
        // reference is only used on this thread (see above).
        unsafe { &*(s as *const MemState) }
    })
}

/// The system's page size: huge blocks are rounded to it (REAL_PAGE_SIZE).
fn real_page_size() -> usize {
    static PAGE: AtomicUsize = AtomicUsize::new(0);
    match PAGE.load(Ordering::Relaxed) {
        0 => {
            // SAFETY: sysconf has no preconditions and doesn't allocate.
            let p = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
            let p = if p > 0 { p as usize } else { PAGE_SIZE };
            PAGE.store(p, Ordering::Relaxed);
            p
        }
        p => p,
    }
}

/// The space a block of `size` bytes takes: 16-byte granules for small
/// blocks (zend_mm's bins round up a little more; this is on the hot path),
/// 4 KiB pages up to a chunk, system pages for huge blocks.
#[inline]
fn rounded(size: usize) -> usize {
    if size <= MAX_SMALL {
        (size + 15) & !15
    } else if size <= MAX_LARGE {
        (size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
    } else {
        huge_rounded(size)
    }
}

#[cold]
fn huge_rounded(size: usize) -> usize {
    let p = real_page_size();
    size.checked_add(p - 1)
        .map_or(usize::MAX & !(p - 1), |s| s & !(p - 1))
}

/// zend_alloc.c's scale_by_percent: `value * percent / 100`, saturating.
fn scale_by_percent(value: usize, percent: i64) -> usize {
    if percent <= 0 {
        return 0;
    }
    let p = percent as usize;
    let (q, r) = (value / 100, value % 100);
    let hi = q.saturating_mul(p);
    let lo = r * (p / 100) + r * (p % 100) / 100;
    hi.saturating_add(lo)
}

/// Chunks needed to hold `paged` bytes of small and large blocks.
fn chunks_for(paged: usize) -> usize {
    paged.div_ceil(MAX_LARGE).max(1)
}

impl MemState {
    fn real(&self) -> usize {
        self.chunks
            .get()
            .saturating_mul(CHUNK_SIZE)
            .saturating_add(self.huge.get())
    }

    /// Real usage as php reports it: before a request starts, the main
    /// chunk only.
    fn real_now(&self) -> usize {
        if self.active.get() {
            self.real()
        } else {
            CHUNK_SIZE
        }
    }

    /// zend_mm_update_hard_limit: `memory_hard_limit_percent` of the limit,
    /// by default twice the limit, or the limit plus 1 GiB above 1 GiB.
    fn hard_limit(&self) -> usize {
        let limit = self.limit.get();
        let mut percent = self.hard_percent.get();
        if percent == -1 {
            const GIB: usize = 1 << 30;
            if limit > GIB {
                return limit.saturating_add(GIB).min(LIMIT_MAX);
            }
            percent = 200;
        }
        scale_by_percent(limit, percent).min(LIMIT_MAX)
    }

    /// zend_mm_check_limit: would taking `add` more bytes from the system
    /// cross the limit? Records the crossing (once per request).
    fn check_limit(&self, add: usize) -> bool {
        let (limit, real) = (self.limit.get(), self.real());
        if limit >= real && add <= limit - real {
            return true;
        }
        self.record(Oom::Limit { limit, tried: add });
        false
    }

    /// zend_mm_check_limit for a non-fallible allocation: past the hard
    /// limit, records that (php exits there) and returns `false`; past the
    /// limit, records the crossing and returns `true`.
    fn check_hard_limit(&self, add: usize) -> bool {
        let (limit, real) = (self.limit.get(), self.real());
        if limit >= real && add <= limit - real {
            return true;
        }
        let hard = self.hard_limit();
        if real > hard || add > hard - real {
            // Recorded even after another out-of-memory error.
            self.pending.set(Some(Oom::Hard {
                limit: hard,
                tried: add,
            }));
            self.set_attention(true);
            self.latched.set(true);
            return false;
        }
        self.record(Oom::Limit { limit, tried: add });
        true
    }

    /// The allocator's limit check: while compiling, the hard limit's too.
    /// The allocation goes ahead either way; the compiler stops at its next
    /// check ([`hard_oom_pending`]).
    fn check_limit_or_hard(&self, add: usize) {
        if self.compiling.get() {
            self.check_hard_limit(add);
        } else {
            self.check_limit(add);
        }
    }

    fn record(&self, oom: Oom) {
        if !self.latched.get() {
            self.latched.set(true);
            self.pending.set(Some(oom));
            self.set_attention(true);
        }
    }

    /// Small blocks are inline: the rest, and a new peak or chunk, are not.
    #[inline]
    fn on_alloc(&self, size: usize) {
        if size <= MAX_SMALL {
            let paged = self.paged.get().saturating_add((size + 15) & !15);
            self.paged.set(paged);
            if paged > self.mark.get() {
                self.passed_mark();
            }
        } else {
            self.on_alloc_big(size);
        }
    }

    #[cold]
    #[inline(never)]
    fn on_alloc_big(&self, size: usize) {
        let r = rounded(size);
        if size <= MAX_LARGE {
            self.paged.set(self.paged.get().saturating_add(r));
        } else {
            self.add_huge(r);
        }
        self.passed_mark();
    }

    /// `paged` went above the mark: take a chunk if the blocks outgrew
    /// them, and record a new peak.
    #[inline(never)]
    fn passed_mark(&self) {
        let paged = self.paged.get();
        if paged > self.chunks.get() * MAX_LARGE {
            self.grow_chunks(paged);
        }
        let s = self.size();
        if s > self.peak.get() {
            self.peak.set(s);
        }
        self.update_mark();
    }

    /// Recompute [`MemState::mark`]: due after `chunks` or `peak` went down
    /// or `huge` went up.
    fn update_mark(&self) {
        let chunk_room = self.chunks.get().saturating_mul(MAX_LARGE);
        let peak_room = self.peak.get().saturating_sub(self.huge.get());
        self.mark.set(chunk_room.min(peak_room));
    }

    /// zend_mm's `size`: every block, rounded.
    fn size(&self) -> usize {
        self.paged.get().saturating_add(self.huge.get())
    }

    /// The small and large blocks outgrew the chunks: take another one.
    #[cold]
    #[inline(never)]
    fn grow_chunks(&self, paged: usize) {
        self.check_limit_or_hard(CHUNK_SIZE);
        self.chunks.set(chunks_for(paged));
        self.real_peak.set(self.real_peak.get().max(self.real()));
    }

    #[cold]
    #[inline(never)]
    fn add_huge(&self, r: usize) {
        self.check_limit_or_hard(r);
        self.huge.set(self.huge.get().saturating_add(r));
        self.real_peak.set(self.real_peak.get().max(self.real()));
    }

    #[inline]
    fn on_free(&self, size: usize) {
        if size <= MAX_SMALL {
            self.paged
                .set(self.paged.get().saturating_sub((size + 15) & !15));
        } else {
            self.on_free_big(size);
        }
    }

    #[cold]
    #[inline(never)]
    fn on_free_big(&self, size: usize) {
        let r = rounded(size);
        if size <= MAX_LARGE {
            // The chunk stays: zend_mm keeps emptied chunks cached (still
            // counted as real usage) until the limit needs them back.
            self.paged.set(self.paged.get().saturating_sub(r));
        } else {
            self.huge.set(self.huge.get().saturating_sub(r));
        }
    }

    #[inline]
    fn on_realloc(&self, old: usize, new: usize) {
        if old > MAX_LARGE && new > MAX_LARGE {
            self.resize_huge(rounded(old), rounded(new));
        } else {
            self.on_free(old);
            self.on_alloc(new);
        }
    }

    /// zend_mm_realloc_huge: growing a huge block checks the limit for the
    /// growth only.
    #[cold]
    #[inline(never)]
    fn resize_huge(&self, old: usize, new: usize) {
        if new <= old {
            self.huge.set(self.huge.get().saturating_sub(old - new));
            return;
        }
        self.check_limit_or_hard(new - old);
        self.huge.set(self.huge.get().saturating_add(new - old));
        self.real_peak.set(self.real_peak.get().max(self.real()));
        self.peak.set(self.peak.get().max(self.size()));
        self.update_mark();
    }

    fn set_attention(&self, on: bool) {
        self.attention.set(on);
        self.tick.set(on || self.gc_due.get());
    }

    /// Count `size` bytes the VM takes outside the allocator, as one block
    /// (a VM stack page, which rhp keeps as frames): it may cross the limit
    /// like any allocation.
    #[inline]
    pub fn charge(&self, size: usize) {
        if self.active.get() {
            self.on_alloc(size);
        }
    }

    /// Give back what [`MemState::charge`] counted.
    #[inline]
    pub fn discharge(&self, size: usize) {
        if self.active.get() {
            self.on_free(size);
        }
    }

    /// [`uncounted`] through this reference (no thread-local lookup).
    #[inline]
    pub fn uncounted<T>(&self, f: impl FnOnce() -> T) -> T {
        let was = self.active.replace(false);
        let r = f();
        self.active.set(was);
        r
    }

    /// A cycle collection is due, or no longer.
    pub fn gc_due(&self, on: bool) {
        self.gc_due.set(on);
        self.tick.set(on || self.attention.get());
    }

    /// Whether a collection is due.
    #[inline]
    pub fn gc_is_due(&self) -> bool {
        self.gc_due.get()
    }

    /// The VM's per-instruction check: [`MemState::attention`] or a due
    /// cycle collection.
    #[inline]
    pub fn tick(&self) -> bool {
        self.tick.get()
    }

    /// Whether a limit crossing waits for the VM or is being reported.
    #[inline]
    pub fn attention(&self) -> bool {
        self.attention.get()
    }

    /// The recorded crossing, once: the VM notes where it happened and
    /// keeps [`MemState::attention`] set until it has raised the error.
    pub fn take_oom(&self) -> Option<Oom> {
        self.pending.take()
    }

    /// The VM raised the error; later crossings stay unrecorded until
    /// [`MemState::reset_oom`].
    pub fn oom_raised(&self) {
        self.set_attention(false);
    }

    /// The VM raised this request's out-of-memory error and the request is
    /// ending (php's ZEND_OOM_STATE_HANDLING / HANDLED).
    pub fn oom_handled(&self) -> bool {
        self.latched.get() && !self.attention.get()
    }

    /// php_request_shutdown's `EG(delayed_oom).state = NONE`: shutdown code
    /// may run out of memory again.
    pub fn reset_oom(&self) {
        self.latched.set(false);
        self.pending.set(None);
        self.set_attention(false);
    }

    /// The current limit (`LIMIT_MAX` when unlimited).
    pub fn limit(&self) -> usize {
        self.limit.get()
    }
}

/// php's `ZEND_HAS_OOM()`: this request crossed the limit and the error
/// is pending or being reported. Code that builds something big checks it
/// to stop early, as php's loops do.
#[inline]
pub fn has_oom() -> bool {
    state().latched.get()
}

/// Start counting this thread's allocations for a new request, from zero.
pub fn begin_request() {
    let s = state();
    s.peak.set(0);
    s.paged.set(0);
    s.chunks.set(1);
    s.huge.set(0);
    s.real_peak.set(CHUNK_SIZE);
    s.update_mark();
    s.reset_oom();
    s.active.set(true);
}

/// Stop counting (the request is over).
pub fn end_request() {
    state().active.set(false);
}

/// Run `f` (parsing and compiling a script) with the allocator checking
/// php's hard limit as well as `memory_limit`: php's compiler allocates
/// with emalloc, which exits past the hard limit ("Hard memory limit of N
/// bytes exhausted"). Elsewhere rhp's own allocations only record crossing
/// the limit, as its sizes differ from php's.
pub fn compiling<T>(f: impl FnOnce() -> T) -> T {
    let s = state();
    let was = s.compiling.replace(true);
    let r = f();
    s.compiling.set(was);
    r
}

/// Has an allocation gone past the hard limit ([`compiling`])? The
/// compiler then stops.
#[inline]
pub fn hard_oom_pending() -> bool {
    matches!(state().pending.get(), Some(Oom::Hard { .. }))
}

/// Run `f` with counting paused: what it allocates (engine data that lives
/// as long as the request, like the prelude's classes) doesn't count.
pub fn uncounted<T>(f: impl FnOnce() -> T) -> T {
    let s = state();
    let was = s.active.replace(false);
    let r = f();
    s.active.set(was);
    r
}

/// zend_set_memory_limit: `false` (and nothing changes) when `bytes` is
/// below what is in use already, after giving cached chunks back. Values
/// above [`LIMIT_MAX`] are clamped.
pub fn set_limit(bytes: usize) -> bool {
    let s = state();
    let bytes = bytes.min(LIMIT_MAX);
    if bytes < s.real_now() {
        let need = chunks_for(s.paged.get());
        if !s.active.get() || bytes < need * CHUNK_SIZE + s.huge.get() {
            return false;
        }
        s.chunks.set(need);
        s.update_mark();
    }
    s.limit.set(bytes);
    true
}

/// zend_set_memory_hard_limit_percent: `false` for a value below 100 (other
/// than -1), or while the usage is above the limit.
pub fn set_hard_limit_percent(percent: i64) -> bool {
    let s = state();
    if (percent < 100 && percent != -1) || s.real_now() > s.limit.get() {
        return false;
    }
    s.hard_percent.set(percent);
    true
}

/// php's non-fallible allocation of a `size`-byte block (emalloc), checked
/// before allocating: crossing `memory_limit` is recorded with php's size
/// (the allocator would record the smaller Rust block). `false` past php's
/// hard limit, where php prints "Hard memory limit of N bytes exhausted"
/// and exits: that is recorded for the VM, and the caller must not
/// allocate. Small and large blocks are left to the allocator.
#[inline]
pub fn alloc_check(size: usize) -> bool {
    size <= MAX_LARGE || huge_check(0, size)
}

/// [`alloc_check`] for growing an `old`-byte block to `new` bytes
/// (erealloc): when both are huge blocks, php checks only the growth.
#[inline]
pub fn realloc_check(old: usize, new: usize) -> bool {
    new <= MAX_LARGE || new <= old || huge_check(old, new)
}

#[cold]
#[inline(never)]
fn huge_check(old: usize, new: usize) -> bool {
    let s = state();
    if !s.active.get() {
        return true;
    }
    let add = if old > MAX_LARGE {
        rounded(new).saturating_sub(rounded(old))
    } else {
        rounded(new)
    };
    add == 0 || s.check_hard_limit(add)
}

/// The space zend_mm gives a block of `size` bytes (what it is counted
/// as): a buffer may grow within it without another allocation.
#[inline]
pub fn block_size(size: usize) -> usize {
    rounded(size)
}

/// Set the limit even below the current usage (a request's starting value).
pub fn force_limit(bytes: usize) {
    state().limit.set(bytes.min(LIMIT_MAX));
}

/// `memory_get_usage($real)`.
pub fn usage(real: bool) -> usize {
    let s = state();
    if real {
        s.real_now()
    } else {
        s.size()
    }
}

/// `memory_get_peak_usage($real)`.
pub fn peak_usage(real: bool) -> usize {
    let s = state();
    if real {
        s.real_peak.get()
    } else {
        s.peak.get()
    }
}

/// `memory_reset_peak_usage()`.
pub fn reset_peak() {
    let s = state();
    s.peak.set(s.size());
    s.real_peak.set(s.real());
    s.update_mark();
}

/// php's fallible allocation of a `size`-byte block: `false` when it would
/// cross the limit. The crossing is then recorded (the VM reports it at its
/// next safe point) and the caller must give up without allocating, like a
/// php builtin that gets NULL from a fallible allocator and `RETURN_OOM()`s.
pub fn reserve(size: usize) -> bool {
    let s = state();
    if !s.active.get() {
        return true;
    }
    if size > MAX_LARGE {
        return s.check_limit(rounded(size));
    }
    let need = chunks_for(s.paged.get().saturating_add(rounded(size)));
    need <= s.chunks.get() || s.check_limit(CHUNK_SIZE)
}

/// [`reserve`] for growing an `old`-byte block to `new` bytes (php's
/// fallible erealloc): a huge block that stays huge checks only its growth,
/// as zend_mm_realloc_huge does; shrinking never fails; anything else needs
/// a new block of `new` bytes.
pub fn reserve_realloc(old: usize, new: usize) -> bool {
    if new <= old {
        return true;
    }
    if old > MAX_LARGE && new > MAX_LARGE {
        let s = state();
        if !s.active.get() {
            return true;
        }
        let (o, n) = (rounded(old), rounded(new));
        return n <= o || s.check_limit(n - o);
    }
    reserve(new)
}

/// [`reserve`] for `n * size + offset` bytes, computed as zend_safe_address
/// does: an overflow records php's "Possible integer overflow in memory
/// allocation" error instead.
pub fn reserve_safe(n: usize, size: usize, offset: usize) -> bool {
    match n.checked_mul(size).and_then(|b| b.checked_add(offset)) {
        Some(b) => reserve(b),
        None => {
            state().record(Oom::Overflow { n, size, offset });
            false
        }
    }
}

/// `_ZSTR_STRUCT_SIZE(len)`: the block a php string of `len` bytes takes
/// (24-byte header, the bytes, a NUL, 8-byte aligned).
pub const fn zstr_size(len: usize) -> usize {
    (24 + len + 1 + 7) & !7
}

/// [`reserve`] for a php string of `len` bytes (zend_string_alloc).
pub fn reserve_str(len: usize) -> bool {
    match len.checked_add(24 + 1 + 7) {
        Some(_) => reserve(zstr_size(len)),
        None => reserve_safe(1, len, 24),
    }
}

/// zend_hash_check_size: the table size php gives an array of `n` elements
/// (a power of two, at least 8), or its overflow error above HT_MAX_SIZE.
fn ht_table_size(n: usize) -> Result<usize, Oom> {
    const HT_MAX_SIZE: usize = 0x4000_0000;
    if n > HT_MAX_SIZE {
        return Err(Oom::Overflow {
            n: n as u32 as usize,
            size: 32,
            offset: 32,
        });
    }
    Ok(n.max(8).next_power_of_two())
}

/// [`reserve`] for a packed (list) array of `n` elements, as
/// zend_hash_real_finit_packed allocates it: 16 bytes per slot.
pub fn reserve_packed_array(n: usize) -> bool {
    match ht_table_size(n) {
        Ok(t) => reserve(t * 16 + 8),
        Err(oom) => {
            state().record(oom);
            false
        }
    }
}

/// [`reserve`] for a hash array of `n` elements (zend_hash_real_finit_mixed:
/// a 32-byte bucket and two hash slots per element).
pub fn reserve_hash_array(n: usize) -> bool {
    match ht_table_size(n) {
        Ok(t) => reserve(t * 40),
        Err(oom) => {
            state().record(oom);
            false
        }
    }
}

/// The global allocator for binaries that run PHP: the system allocator
/// plus this thread's request accounting. Install it once per binary:
///
/// ```ignore
/// #[global_allocator]
/// static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;
/// ```
///
/// Without it, requests still run, but `memory_get_usage()` reads zero and
/// `memory_limit` is only enforced by the builtins that check large sizes
/// up front.
pub struct CountingAlloc;

impl CountingAlloc {
    #[inline]
    fn count(f: impl FnOnce(&MemState)) {
        // `try_with`: the thread-local can't be destroyed (no destructor),
        // but allocation during thread teardown must never panic.
        let _ = STATE.try_with(|s| {
            if s.active.get() {
                f(s)
            }
        });
    }
}

// SAFETY: every method forwards to `System` with the same arguments; the
// bookkeeping only touches this thread's `Cell`s and never allocates.
unsafe impl GlobalAlloc for CountingAlloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            Self::count(|s| s.on_alloc(layout.size()));
        }
        p
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            Self::count(|s| s.on_alloc(layout.size()));
        }
        p
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // Counted first, so that the free is a tail call.
        Self::count(|s| s.on_free(layout.size()));
        System.dealloc(ptr, layout);
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = System.realloc(ptr, layout, new_size);
        if !p.is_null() {
            Self::count(|s| s.on_realloc(layout.size(), new_size));
        }
        p
    }
}

#[cfg(test)]
mod tests;
