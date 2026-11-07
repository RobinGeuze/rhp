//! The parse arena's memory: a bump allocator over fixed-size chunks, as
//! php's compiler keeps its AST (CG(ast_arena), 32 KiB pages). blink-alloc
//! (mago's default arena) doubles each chunk to the arena's total size, so
//! up to half of the last chunk sat unused and still counted against
//! memory_limit.

use std::alloc::Layout;
use std::cell::{Cell, RefCell};
use std::ptr::NonNull;

use mago_allocator::alloc::{AllocError, Allocator};

/// A chunk's size. A block above a quarter of it gets a chunk of its own.
const CHUNK: usize = 32 * 1024;

pub(crate) struct Bump {
    /// The current chunk's free space: `cur..end`.
    cur: Cell<*mut u8>,
    end: Cell<*mut u8>,
    /// Every block taken from the global allocator, freed on drop.
    chunks: RefCell<Vec<(NonNull<u8>, Layout)>>,
}

impl Bump {
    pub(crate) fn new() -> Self {
        Bump {
            cur: Cell::new(std::ptr::null_mut()),
            end: Cell::new(std::ptr::null_mut()),
            chunks: RefCell::new(Vec::new()),
        }
    }

    /// A block of `layout` from the global allocator, kept until drop.
    fn system(&self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        // SAFETY: `layout` has a non-zero size (callers handle zero).
        let p = NonNull::new(unsafe { std::alloc::alloc(layout) }).ok_or(AllocError)?;
        self.chunks.borrow_mut().push((p, layout));
        Ok(p)
    }

    #[cold]
    #[inline(never)]
    fn allocate_slow(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() > CHUNK / 4 {
            let p = self.system(layout)?;
            return Ok(NonNull::slice_from_raw_parts(p, layout.size()));
        }
        let chunk =
            Layout::from_size_align(CHUNK, layout.align().max(16)).map_err(|_| AllocError)?;
        let base = self.system(chunk)?.as_ptr();
        // SAFETY: the chunk is CHUNK bytes and aligned for `layout`, which
        // fits in a quarter of it.
        unsafe {
            self.cur.set(base.add(layout.size()));
            self.end.set(base.add(CHUNK));
            Ok(NonNull::slice_from_raw_parts(
                NonNull::new_unchecked(base),
                layout.size(),
            ))
        }
    }
}

// SAFETY: blocks stay valid until the arena drops (deallocate is a no-op);
// each is handed out once, sized and aligned as asked.
unsafe impl Allocator for Bump {
    #[inline]
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let size = layout.size();
        if size == 0 {
            // A dangling, aligned pointer, as for any zero-sized block.
            let p = std::ptr::without_provenance_mut::<u8>(layout.align());
            // SAFETY: `align` is non-zero, so `p` is too.
            return Ok(NonNull::slice_from_raw_parts(
                unsafe { NonNull::new_unchecked(p) },
                0,
            ));
        }
        let cur = self.cur.get();
        if !cur.is_null() {
            let pad = cur.align_offset(layout.align());
            let room = self.end.get() as usize - cur as usize;
            if pad <= room && size <= room - pad {
                // SAFETY: `pad + size` bytes from `cur` lie in the chunk.
                unsafe {
                    let p = cur.add(pad);
                    self.cur.set(p.add(size));
                    return Ok(NonNull::slice_from_raw_parts(
                        NonNull::new_unchecked(p),
                        size,
                    ));
                }
            }
        }
        self.allocate_slow(layout)
    }

    #[inline]
    unsafe fn deallocate(&self, _ptr: NonNull<u8>, _layout: Layout) {}
}

impl Drop for Bump {
    fn drop(&mut self) {
        for (p, layout) in self.chunks.get_mut().drain(..) {
            // SAFETY: allocated in `system` with this layout, freed once.
            unsafe { std::alloc::dealloc(p.as_ptr(), layout) };
        }
    }
}
