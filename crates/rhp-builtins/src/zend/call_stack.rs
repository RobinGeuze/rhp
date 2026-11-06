//! The native stack limit (Zend/zend_call_stack.c): builtins that recurse
//! over nested data (count, var_dump, serialize, ...) check how much of the
//! thread's stack is used and throw php's "Maximum call stack size" Error
//! before it runs out. The limit comes from zend.max_allowed_stack_size
//! (0: the thread's stack size, -1: none) less zend.reserved_stack_size.

use std::cell::Cell;

use crate::Thrown;

thread_local! {
    /// `EG(stack_base)` and `EG(stack_limit)` (0: unchecked).
    static BASE: Cell<usize> = const { Cell::new(0) };
    static LIMIT: Cell<usize> = const { Cell::new(0) };
    /// EG(reserved_stack_size).
    static RESERVED: Cell<usize> = const { Cell::new(RESERVED_MIN) };
}

/// zend.reserved_stack_size's floor (php's without alloca).
const RESERVED_MIN: usize = 32 * 1024;

/// zend_call_stack_default_size: the stack size php assumes for a thread
/// it can't inspect.
const DEFAULT_SIZE: usize = 8 * 1024 * 1024;

/// The current stack position (an address in this frame).
#[inline(always)]
fn position() -> usize {
    let marker = 0u8;
    std::hint::black_box(&marker) as *const u8 as usize
}

/// zend_call_stack_get: this thread's stack (base = highest address, size).
fn thread_stack() -> Option<(usize, usize)> {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: plain queries about the calling thread's own stack.
        let (base, size) = unsafe {
            let t = libc::pthread_self();
            (
                libc::pthread_get_stackaddr_np(t) as usize,
                libc::pthread_get_stacksize_np(t),
            )
        };
        (base != 0 && size != 0).then_some((base, size))
    }
    #[cfg(target_os = "linux")]
    {
        // SAFETY: the attribute object is initialised by pthread_getattr_np
        // and destroyed once read.
        unsafe {
            let mut attr: libc::pthread_attr_t = std::mem::zeroed();
            if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
                return None;
            }
            let mut addr: *mut libc::c_void = std::ptr::null_mut();
            let mut size: libc::size_t = 0;
            let ok = libc::pthread_attr_getstack(&attr, &mut addr, &mut size) == 0;
            libc::pthread_attr_destroy(&mut attr);
            (ok && !addr.is_null()).then(|| (addr as usize + size, size))
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    None
}

/// zend_call_stack_limit.
fn limit(base: usize, size: usize, reserved: usize) -> usize {
    match base.checked_sub(size) {
        Some(low) => low.saturating_add(reserved),
        None => 0,
    }
}

/// zend_call_stack_init, from the request's ini values.
pub fn init() {
    let ini = |n: &[u8]| crate::ini_value(n).flatten().unwrap_or_default();
    let max = crate::main::ini::ini_parse_quantity(&ini(b"zend.max_allowed_stack_size"));
    let reserved = match crate::main::ini::ini_parse_uquantity(&ini(b"zend.reserved_stack_size")).0
    {
        0 => RESERVED_MIN,
        r => r as usize,
    };
    let (base, lim) = match max {
        -1 => (0, 0),
        0 => {
            let (base, size) =
                thread_stack().unwrap_or_else(|| (position(), DEFAULT_SIZE - 32 * 1024));
            (base, limit(base, size, reserved))
        }
        m => {
            let base = thread_stack().map_or_else(position, |s| s.0);
            (base, limit(base, m as usize, reserved))
        }
    };
    BASE.with(|b| b.set(base));
    LIMIT.with(|l| l.set(lim));
    RESERVED.with(|r| r.set(reserved));
}

/// The running stack's (base, limit), saved when a fiber switches.
pub fn bounds() -> (usize, usize) {
    (BASE.with(Cell::get), LIMIT.with(Cell::get))
}

/// Make (base, limit) the running stack's (a fiber switch).
pub fn set_bounds((base, lim): (usize, usize)) {
    BASE.with(|b| b.set(base));
    LIMIT.with(|l| l.set(lim));
}

/// zend_fiber_stack_base / zend_fiber_stack_limit: the bounds for a fiber
/// stack of `size` bytes below `base`, checked only when the main stack is.
pub fn fiber_bounds(base: usize, size: usize) -> (usize, usize) {
    if LIMIT.with(Cell::get) == 0 {
        return (0, 0);
    }
    (base, limit(base, size, RESERVED.with(Cell::get)))
}

/// zend_call_stack_overflowed: the stack has grown past the limit.
#[inline]
pub fn overflowed() -> bool {
    let lim = LIMIT.with(Cell::get);
    lim != 0 && position() < lim
}

/// zend_call_stack_size_error's Error.
pub fn size_error() -> Thrown {
    let max = BASE.with(Cell::get).saturating_sub(LIMIT.with(Cell::get));
    Thrown::error(
        "Error",
        format!(
            "Maximum call stack size of {max} bytes (zend.max_allowed_stack_size - zend.reserved_stack_size) reached. Infinite recursion?"
        ),
    )
}

/// The check php's recursive builtins start with: the size Error once the
/// limit is passed.
#[inline]
pub fn check() -> Result<(), Thrown> {
    if overflowed() {
        return Err(size_error());
    }
    Ok(())
}
