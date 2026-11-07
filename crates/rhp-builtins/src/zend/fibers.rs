//! Zend/zend_fibers.c outside the engine: the `__fiber` helper behind the
//! prelude's `Fiber` class (the engine does the switching, see the VM's
//! fibers.rs), the class's `dtor_obj`, the `fiber.stack_size` directive and
//! php's switch blocking (no switch while tick functions or signal handlers
//! run).

use std::cell::Cell;

use rhp_values::{ObjectHandle, Thrown, Value};

use crate::Host;

/// What `__fiber` asks the engine for: one per `Fiber` method, plus the
/// object's destruction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FiberOp {
    Construct,
    Start,
    Resume,
    Throw,
    IsStarted,
    IsSuspended,
    IsRunning,
    IsTerminated,
    GetReturn,
    GetCurrent,
    Suspend,
    /// zend_fiber_object_destroy (the class's `dtor_obj`).
    Destroy,
    /// ReflectionFiber::getTrace(), getExecutingLine(), getExecutingFile(),
    /// getCallable() on the fiber.
    ReflectTrace,
    ReflectLine,
    ReflectFile,
    ReflectCallable,
}

impl FiberOp {
    fn from_int(n: i64) -> Option<FiberOp> {
        use FiberOp::*;
        Some(match n {
            0 => Construct,
            1 => Start,
            2 => Resume,
            3 => Throw,
            4 => IsStarted,
            5 => IsSuspended,
            6 => IsRunning,
            7 => IsTerminated,
            8 => GetReturn,
            9 => GetCurrent,
            10 => Suspend,
            11 => ReflectTrace,
            12 => ReflectLine,
            13 => ReflectFile,
            14 => ReflectCallable,
            _ => return None,
        })
    }
}

/// `__fiber(op, $this, ...$args)`: the prelude's Fiber methods.
fn fn_fiber(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = match args.first() {
        Some(Value::Int(n)) => FiberOp::from_int(*n),
        _ => None,
    }
    .ok_or_else(|| Thrown::error("Error", "__fiber(): unknown operation"))?;
    let this = match args.get(1).map(crate::zend::zpp::deref) {
        Some(Value::Object(h)) => Some(h),
        _ => None,
    };
    host.fiber(op, this.as_ref(), args.get(2..).unwrap_or(&[]))
}

/// zend_fiber_object_destroy: a suspended fiber is resumed to unwind.
pub(crate) fn object_dtor(host: &mut dyn Host, obj: &ObjectHandle) -> Result<(), Thrown> {
    host.fiber(FiberOp::Destroy, Some(obj), &[]).map(drop)
}

thread_local! {
    /// zend_fiber_switch_blocking.
    static BLOCKING: Cell<u32> = const { Cell::new(0) };
}

/// zend_fiber_switch_blocked: a tick function or signal handler is running.
pub fn switch_blocked() -> bool {
    BLOCKING.with(Cell::get) != 0
}

/// zend_fiber_switch_block / zend_fiber_switch_unblock, as a guard.
pub struct SwitchBlock(());

impl SwitchBlock {
    pub fn new() -> SwitchBlock {
        BLOCKING.with(|b| b.set(b.get() + 1));
        SwitchBlock(())
    }
}

impl Default for SwitchBlock {
    fn default() -> Self {
        SwitchBlock::new()
    }
}

impl Drop for SwitchBlock {
    fn drop(&mut self) {
        BLOCKING.with(|b| b.set(b.get().saturating_sub(1)));
    }
}

/// ZEND_FIBER_DEFAULT_C_STACK_SIZE (64-bit).
const DEFAULT_STACK_SIZE: usize = 4096 * 512;

/// The system page size (zend_fiber_get_page_size).
pub fn page_size() -> usize {
    // SAFETY: a plain sysconf query.
    let p = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if p > 0 && (p as usize).is_power_of_two() {
        p as usize
    } else {
        4096
    }
}

/// EG(fiber_stack_size) checked as zend_fiber_stack_allocate does: the
/// size rounded up to whole pages, or php's Exception for one below a
/// page plus the guard page.
pub fn stack_size() -> Result<usize, Thrown> {
    let size = match crate::ini_value(b"fiber.stack_size").flatten() {
        Some(v) => crate::main::ini::ini_parse_quantity(&v).max(0) as usize,
        None => DEFAULT_STACK_SIZE,
    };
    let page = page_size();
    let minimum = 2 * page;
    if size < minimum {
        return Err(Thrown::error(
            "Exception",
            format!("Fiber stack size is too small, it needs to be at least {minimum} bytes"),
        ));
    }
    Ok(size.div_ceil(page) * page)
}

pub fn register(m: &mut crate::Registry) {
    crate::register_helpers(m, &[("__fiber", fn_fiber, 2, None)]);
}
