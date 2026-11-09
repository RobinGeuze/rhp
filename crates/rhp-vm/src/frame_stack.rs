//! The call stack and its memory, counted as php counts its VM stack
//! (Zend/zend_execute.c: zend_vm_stack_extend).
//!
//! php puts call frames on 256 KiB pages from zend_mm: a frame takes
//! `(ZEND_CALL_FRAME_SLOT + args + variables + temporaries)` zvals, a frame
//! that does not fit takes a new page, and the page goes when that frame
//! returns. So runaway recursion ends in "Allowed memory size of N bytes
//! exhausted (tried to allocate 2097152 bytes)" when a page needs a new
//! chunk. rhp's frames live in a `Vec<Frame>` and their registers in a
//! `Vec<Value>` each; neither allocation is counted. [`FrameStack`]
//! charges php's pages instead, sized from the frames' register counts.
//!
//! Register files are recycled through [`RegPool`]: a call takes the
//! last one given back (no malloc or free per call).

use std::ops::{Deref, DerefMut};

use rhp_values::mem::MemState;
use rhp_values::Value;

use crate::Frame;

/// ZEND_VM_STACK_PAGE_SIZE: 16 Ki zvals.
const PAGE_SIZE: usize = 16 * 1024 * 16;
/// ZEND_FIBER_VM_STACK_SIZE: a fiber's pages (1 Ki zvals).
pub(crate) const FIBER_PAGE_SIZE: usize = 1024 * 16;
/// ZEND_VM_STACK_HEADER_SLOTS, in bytes.
const HEADER: usize = 2 * 16;
/// ZEND_CALL_FRAME_SLOT: zend_execute_data, in zvals.
const FRAME_SLOTS: usize = 5;

/// The frames of one stack (the request's, or a fiber's), with php's VM
/// stack pages counted for them. It dereferences to the frames as a
/// slice: frames come and go only through `push`, `pop` and `truncate`.
pub(crate) struct FrameStack {
    frames: Vec<Frame>,
    /// Bytes left on the current page.
    room: usize,
    /// Pages taken after the first: (size, room left on the page below).
    pages: Vec<(usize, usize)>,
    /// EG(vm_stack_page_size).
    page_size: usize,
    /// The first page is charged here (a fiber's): given back on drop.
    first_page: usize,
    mem: &'static MemState,
}

impl FrameStack {
    /// The request's stack: its first page is part of php's baseline.
    pub(crate) fn new(mem: &'static MemState) -> Self {
        FrameStack {
            frames: Vec::new(),
            room: PAGE_SIZE - HEADER,
            pages: Vec::new(),
            page_size: PAGE_SIZE,
            first_page: 0,
            mem,
        }
    }

    /// A fiber's stack (zend_fiber_execute's own first page).
    pub(crate) fn new_fiber(mem: &'static MemState) -> Self {
        mem.charge(FIBER_PAGE_SIZE);
        FrameStack {
            frames: Vec::new(),
            room: FIBER_PAGE_SIZE - HEADER,
            pages: Vec::new(),
            page_size: FIBER_PAGE_SIZE,
            first_page: FIBER_PAGE_SIZE,
            mem,
        }
    }

    /// Push a frame, taking a page when it does not fit
    /// (zend_vm_stack_push_call_frame_ex).
    #[inline]
    pub(crate) fn push(&mut self, mut frame: Frame) {
        let size = (FRAME_SLOTS + frame.regs.len() + frame.extra_args.len()) * 16;
        frame.vm_size = size as u32;
        frame.vm_page = size > self.room;
        if frame.vm_page {
            self.new_page(size);
        } else {
            self.room -= size;
        }
        if self.frames.len() == self.frames.capacity() {
            self.grow();
        }
        self.frames.push(frame);
    }

    #[cold]
    #[inline(never)]
    fn new_page(&mut self, size: usize) {
        let page = if size < self.page_size - HEADER {
            self.page_size
        } else {
            (size + HEADER + self.page_size - 1) & !(self.page_size - 1)
        };
        self.pages.push((page, self.room));
        self.room = page - HEADER - size;
        self.mem.charge(page);
    }

    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        let n = self.frames.capacity().max(16);
        self.mem.uncounted(|| self.frames.reserve(n));
    }

    /// Pop the top frame, giving its page back when it took one.
    #[inline]
    pub(crate) fn pop(&mut self) -> Option<Frame> {
        let f = self.frames.pop()?;
        self.release(&f);
        Some(f)
    }

    #[inline]
    fn release(&mut self, f: &Frame) {
        if f.vm_page {
            if let Some((page, room)) = self.pages.pop() {
                self.room = room;
                self.mem.discharge(page);
            }
        } else {
            self.room += f.vm_size as usize;
        }
    }

    /// Drop every frame, bottom first (a Vec's order).
    pub(crate) fn clear(&mut self) {
        self.frames.clear();
        for (page, _) in self.pages.drain(..) {
            self.mem.discharge(page);
        }
        self.room = self.page_size - HEADER;
    }

    /// Pop frames down to `len` (each dropped as it goes, top first).
    pub(crate) fn truncate(&mut self, len: usize) {
        while self.frames.len() > len {
            drop(self.pop());
        }
    }
}

impl Deref for FrameStack {
    type Target = [Frame];
    #[inline(always)]
    fn deref(&self) -> &[Frame] {
        &self.frames
    }
}

impl DerefMut for FrameStack {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut [Frame] {
        &mut self.frames
    }
}

impl Drop for FrameStack {
    fn drop(&mut self) {
        // The frames go bottom first (a Vec's order), then the uncounted
        // buffer and the pages.
        self.clear();
        let v = std::mem::take(&mut self.frames);
        self.mem.uncounted(|| drop(v));
        self.mem.discharge(self.first_page);
    }
}

/// Register files given back by returning frames, for the next calls.
/// Their memory is not counted (php's frames are in VM stack pages).
#[derive(Default)]
pub(crate) struct RegPool {
    free: Vec<Vec<Value>>,
}

/// Register files kept for reuse.
const POOL_MAX: usize = 64;
/// The largest register file kept.
const POOL_REGS_MAX: usize = 1024;

impl RegPool {
    /// A register file of `n` undefined registers.
    #[inline]
    pub(crate) fn take(&mut self, n: usize, mem: &MemState) -> Vec<Value> {
        let mut regs = match self.free.pop() {
            Some(v) => v,
            None => return alloc_regs(n, mem),
        };
        if regs.capacity() < n {
            mem.uncounted(|| regs.reserve_exact(n));
        }
        // Every register starts undefined (a zero tag: a fill of zeroes).
        regs.resize_with(n, || Value::Undef);
        regs
    }

    /// Keep `regs` (emptied) for a later call, or free it.
    #[inline]
    pub(crate) fn give(&mut self, mut regs: Vec<Value>, mem: &MemState) {
        regs.clear();
        if self.free.len() < POOL_MAX && regs.capacity() <= POOL_REGS_MAX {
            if self.free.len() == self.free.capacity() {
                mem.uncounted(|| self.free.reserve(POOL_MAX));
            }
            self.free.push(regs);
        } else {
            mem.uncounted(|| drop(regs));
        }
    }
}

impl Drop for RegPool {
    fn drop(&mut self) {
        let free = std::mem::take(&mut self.free);
        rhp_values::mem::uncounted(|| drop(free));
    }
}

/// A new register file of `n` undefined registers, not counted.
#[cold]
pub(crate) fn alloc_regs(n: usize, mem: &MemState) -> Vec<Value> {
    let mut regs = mem.uncounted(|| Vec::with_capacity(n));
    regs.resize_with(n, || Value::Undef);
    regs
}
