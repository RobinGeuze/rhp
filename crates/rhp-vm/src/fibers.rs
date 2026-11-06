//! Fibers (Zend/zend_fibers.c): each fiber runs on its own machine stack,
//! switched with corosensei the way php switches with boost.context, so a
//! fiber can suspend from any depth, including from a callback a builtin
//! runs (the nested Rust run loops are suspended with it).
//!
//! The VM's per-stack state (the frame stack, the running builtins, the
//! generator stack, the `@` level and error_reporting, ...) is an
//! [`ExecCtx`]: switching into a fiber swaps the fiber's context into the
//! [`Vm`] and keeps the resumer's in the fiber, so frame indices cached by
//! the suspended run loops stay valid on both sides, and backtraces walk
//! the fiber's frames and then each resumer's. See docs/design/fibers.md.
//!
//! All `unsafe` of the switching is in this file.

use std::cell::RefCell;
use std::rc::Rc;

use corosensei::stack::DefaultStack;
use corosensei::{Coroutine, CoroutineResult, Yielder};
use rhp_builtins as builtins;
use rhp_builtins::zend::fibers::FiberOp;
use rhp_bytecode::{FunctionProto, Reg};
use rhp_values::{ObjectHandle, PhpArray, PhpStr, Thrown, Value};

use crate::calls::NativeCall;
use crate::{deref, Frame, Vm};

/// What a resume hands the fiber: the VM (see [`Vm::fiber_switch`]) and the
/// value `Fiber::suspend()` returns, or the exception it throws.
struct Resume {
    vm: *mut Vm,
    value: Result<Value, Thrown>,
}

/// What `Fiber::suspend()` hands back to the resumer.
struct Suspended {
    vm: *mut Vm,
    value: Value,
}

/// What a finished fiber hands back: its exception, if it threw.
struct Finished {
    vm: *mut Vm,
    result: Result<(), Thrown>,
}

type Co = Coroutine<Resume, Suspended, Finished, DefaultStack>;

/// php's ZEND_FIBER_STATUS_*. A fiber that resumed another is `Running`
/// too (php: SUSPENDED with a caller, which its methods treat as running).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Init,
    Running,
    Suspended,
    Dead,
}

/// The per-stack execution state php keeps per fiber
/// (zend_fiber_capture_vm_state), swapped into the [`Vm`] on each switch.
pub(crate) struct ExecCtx {
    stack: Vec<Frame>,
    native_calls: Vec<NativeCall>,
    gen_stack: Vec<u32>,
    yield_out: Option<(Value, Value, Reg)>,
    magic_fetch: (usize, builtins::MagicFetch),
    pending_magic_fetch: builtins::MagicFetch,
    silence: u32,
    silence_stack: Vec<(usize, u32, i64)>,
    ctor_frames: Vec<(usize, usize)>,
    timeout_prev: Option<(usize, usize)>,
    timeout_wait: Option<(usize, usize)>,
    unused_call_depth: usize,
    autoload_depths: Vec<usize>,
    /// EG(error_reporting).
    error_reporting: i64,
    /// EG(stack_base), EG(stack_limit).
    stack_bounds: (usize, usize),
}

/// A Fiber object's state: its payload.
pub(crate) struct FiberData {
    status: Status,
    /// ZEND_FIBER_FLAG_THREW / _BAILOUT / _DESTROYED.
    threw: bool,
    bailout: bool,
    destroyed: bool,
    /// The callable (Undef before the constructor ran and once it returned).
    callable: Value,
    args: Vec<Value>,
    named: Vec<(PhpStr, Value)>,
    result: Value,
    /// The coroutine, while not running.
    co: Option<Co>,
    /// The fiber's own context while it does not run; the resumer's while
    /// it runs.
    ctx: Option<Box<ExecCtx>>,
    /// The fiber's yielder, set when it starts running. It lives on the
    /// fiber's stack and stays valid until the coroutine finishes.
    yielder: *const Yielder<Resume, Suspended>,
    /// The fiber that resumed this one (None: the main context), while it
    /// runs.
    prev: Option<ObjectHandle>,
}

type FiberCell = RefCell<FiberData>;

impl Drop for FiberData {
    fn drop(&mut self) {
        if let Some(co) = &mut self.co {
            if co.started() && !co.done() {
                // A fiber never destroyed (a fatal error's bailout left it
                // suspended): its stack is dropped without unwinding.
                // SAFETY: the fiber's Rust frames hold no borrows of data
                // that outlives it and need no Drop for soundness (values
                // they own leak, as php leaks them after a bailout); the
                // coroutine is never resumed again.
                unsafe { co.force_reset() };
            }
        }
    }
}

/// The class php gives the exception a destroyed fiber unwinds with
/// (zend_create_graceful_exit): no catch matches it, finally blocks run.
const GRACEFUL_EXIT: &str = "\0GracefulExit";

pub(crate) fn graceful_exit() -> Thrown {
    Thrown::Error {
        class: GRACEFUL_EXIT,
        message: String::new(),
    }
}

/// zend_is_graceful_exit.
pub(crate) fn is_graceful_exit(t: &Thrown) -> bool {
    matches!(t, Thrown::Error { class, .. } if *class == GRACEFUL_EXIT)
}

fn fiber_error(msg: &str) -> Thrown {
    Thrown::error("FiberError", msg)
}

fn cell_of(h: &ObjectHandle) -> Rc<FiberCell> {
    if let Some(c) = h.borrow().payload::<FiberCell>() {
        return c;
    }
    let c = Rc::new(RefCell::new(FiberData {
        status: Status::Init,
        threw: false,
        bailout: false,
        destroyed: false,
        callable: Value::Undef,
        args: Vec::new(),
        named: Vec::new(),
        result: Value::Null,
        co: None,
        ctx: None,
        yielder: std::ptr::null(),
        prev: None,
    }));
    h.borrow_mut().set_payload(c.clone());
    c
}

/// The bottom frame of a fiber's stack (php's zend_fiber_function): code
/// it calls is called from `[internal function]`.
fn entry_frame() -> Frame {
    let mut p = FunctionProto::new(PhpStr::from("{fiber}"));
    p.debug.file = PhpStr::from("");
    p.flags.insert(rhp_bytecode::ProtoFlags::INTERNAL);
    p.flags.insert(rhp_bytecode::ProtoFlags::SHUTDOWN_SCOPE);
    Frame::new(Rc::new(p), 0, 0)
}

impl Vm {
    /// `__fiber`: the Fiber methods and the object's destruction.
    pub(crate) fn fiber_op(
        &mut self,
        op: FiberOp,
        this: Option<&ObjectHandle>,
        args: &[Value],
    ) -> Result<Value, Thrown> {
        let arg = |i: usize| args.get(i).map(deref).unwrap_or(Value::Null);
        match op {
            FiberOp::GetCurrent => {
                return Ok(self.active_fiber.clone().map_or(Value::Null, Value::Object))
            }
            FiberOp::Suspend => return self.fiber_suspend(arg(0)),
            _ => {}
        }
        let Some(h) = this else {
            return Ok(Value::Null);
        };
        if op == FiberOp::Destroy && h.borrow().payload::<FiberCell>().is_none() {
            return Ok(Value::Null);
        }
        let cell = cell_of(h);
        let status = cell.borrow().status;
        let blocked = || builtins::zend::fibers::switch_blocked();
        match op {
            FiberOp::Construct => {
                // Z_PARAM_FUNC.
                if let Some(err) = builtins::Host::check_callable(self, &arg(0))? {
                    return Err(Thrown::type_error(format!(
                        "Fiber::__construct(): Argument #1 ($callback) must be a valid callback, {err}"
                    )));
                }
                let constructed = {
                    let d = cell.borrow();
                    d.status != Status::Init || !d.callable.is_undef()
                };
                if constructed {
                    return Err(fiber_error("Cannot call constructor twice"));
                }
                let callable = self.fiber_bind_callable(arg(0));
                cell.borrow_mut().callable = callable;
                Ok(Value::Null)
            }
            FiberOp::Start => {
                if blocked() {
                    return Err(fiber_error(
                        "Cannot switch fibers in current execution context",
                    ));
                }
                if status != Status::Init {
                    return Err(fiber_error(
                        "Cannot start a fiber that has already been started",
                    ));
                }
                let size = builtins::zend::fibers::stack_size()?;
                let (pos, named) = self.split_call_args(&arg(0))?;
                self.fiber_init(&cell, size, pos, named)?;
                self.fiber_switch(h, &cell, Ok(Value::Null))
            }
            FiberOp::Resume | FiberOp::Throw => {
                if blocked() {
                    return Err(fiber_error(
                        "Cannot switch fibers in current execution context",
                    ));
                }
                if status != Status::Suspended {
                    return Err(fiber_error("Cannot resume a fiber that is not suspended"));
                }
                let input = match (op, arg(0)) {
                    (FiberOp::Throw, Value::Object(e)) => Err(Thrown::Object(e)),
                    (_, v) => Ok(v),
                };
                self.fiber_switch(h, &cell, input)
            }
            FiberOp::IsStarted => Ok(Value::Bool(status != Status::Init)),
            FiberOp::IsSuspended => Ok(Value::Bool(status == Status::Suspended)),
            FiberOp::IsRunning => Ok(Value::Bool(status == Status::Running)),
            FiberOp::IsTerminated => Ok(Value::Bool(status == Status::Dead)),
            FiberOp::GetReturn => {
                let d = cell.borrow();
                let msg = match d.status {
                    Status::Dead if d.threw => "The fiber threw an exception",
                    Status::Dead if d.bailout => "The fiber exited with a fatal error",
                    Status::Dead => return Ok(deref(&d.result)),
                    Status::Init => "The fiber has not been started",
                    _ => "The fiber has not returned",
                };
                Err(fiber_error(&format!(
                    "Cannot get fiber return value: {msg}"
                )))
            }
            FiberOp::Destroy => {
                if status != Status::Suspended {
                    return Ok(Value::Null);
                }
                cell.borrow_mut().destroyed = true;
                self.fiber_switch(h, &cell, Err(graceful_exit()))
                    .map(|_| Value::Null)
            }
            FiberOp::GetCurrent | FiberOp::Suspend => unreachable!(),
        }
    }

    /// php keeps the constructor's checked fcc: a method callable stays
    /// resolved in the scope (and with the `$this`) it was checked in.
    fn fiber_bind_callable(&self, callable: Value) -> Value {
        let this = match (&callable, self.resolve_callable(&callable)) {
            (Value::Str(_) | Value::Array(_), Ok(crate::calls::Callee::Method(_, this, _))) => this,
            _ => return callable,
        };
        let (scope, called, _) = self.callable_scope();
        Value::Closure(Rc::new(rhp_values::ClosureData {
            fnidx: u32::MAX,
            captures: Vec::new(),
            bound_this: this,
            bound_class: scope,
            bound_static: called,
            wrapped: Some(Box::new(callable)),
            // Not an object php makes: no handle.
            id: 0,
            statics: Default::default(),
        }))
    }

    /// zend_fiber_init_context: the stack and the coroutine, and the
    /// fiber's fresh execution context (zend_fiber_execute's setup).
    fn fiber_init(
        &mut self,
        cell: &Rc<FiberCell>,
        size: usize,
        args: Vec<Value>,
        named: Vec<(PhpStr, Value)>,
    ) -> Result<(), Thrown> {
        // rhp's frames take more machine stack than php's: reserve more
        // address space than fiber.stack_size (it is committed only as
        // used); the stack limit checks still use fiber.stack_size.
        let real = size.saturating_mul(4).max(8 << 20);
        let stack = DefaultStack::new(real).map_err(|e| {
            Thrown::error(
                "Exception",
                format!(
                    "Fiber stack allocate failed: mmap failed: {e} ({})",
                    e.raw_os_error().unwrap_or(0)
                ),
            )
        })?;
        let base = corosensei::stack::Stack::base(&stack).get();
        let co: Co =
            Coroutine::with_stack(stack, |y: &Yielder<Resume, Suspended>, input: Resume| {
                // SAFETY: the resumer passes the VM it is running on and does
                // not touch it until this fiber suspends or returns (it is
                // blocked in `resume`), so this is the only live access; the
                // pointer goes back out with every suspend and the return.
                let vm = unsafe { &mut *input.vm };
                let result = vm.fiber_execute(y);
                Finished {
                    vm: vm as *mut Vm,
                    result,
                }
            });
        // zend_fiber_execute: error_reporting starts from the ini value
        // (not one an active `@` lowered).
        let error_reporting = match self.silence_stack.first() {
            Some(&(_, _, saved)) => saved,
            None => builtins::zend::builtin_functions::error_reporting_level(self),
        };
        let ctx = ExecCtx {
            stack: vec![entry_frame()],
            native_calls: Vec::new(),
            gen_stack: Vec::new(),
            yield_out: None,
            magic_fetch: (0, builtins::MagicFetch::default()),
            pending_magic_fetch: builtins::MagicFetch::default(),
            silence: 0,
            silence_stack: Vec::new(),
            ctor_frames: Vec::new(),
            timeout_prev: None,
            timeout_wait: None,
            unused_call_depth: usize::MAX,
            autoload_depths: Vec::new(),
            error_reporting,
            stack_bounds: builtins::zend::call_stack::fiber_bounds(base, size),
        };
        let mut d = cell.borrow_mut();
        d.co = Some(co);
        d.ctx = Some(Box::new(ctx));
        d.args = args;
        d.named = named;
        Ok(())
    }

    /// Swap the running context with `c` (zend_fiber_capture_vm_state and
    /// zend_fiber_restore_vm_state in one).
    fn swap_ctx(&mut self, c: &mut ExecCtx) {
        use std::mem::swap;
        swap(&mut self.stack, &mut c.stack);
        swap(&mut self.native_calls, &mut c.native_calls);
        swap(&mut self.gen_stack, &mut c.gen_stack);
        swap(&mut self.yield_out, &mut c.yield_out);
        swap(&mut self.magic_fetch, &mut c.magic_fetch);
        swap(&mut self.pending_magic_fetch, &mut c.pending_magic_fetch);
        swap(&mut self.silence, &mut c.silence);
        swap(&mut self.silence_stack, &mut c.silence_stack);
        swap(&mut self.ctor_frames, &mut c.ctor_frames);
        swap(&mut self.timeout_prev, &mut c.timeout_prev);
        swap(&mut self.timeout_wait, &mut c.timeout_wait);
        swap(&mut self.unused_call_depth, &mut c.unused_call_depth);
        swap(&mut self.autoload_depths, &mut c.autoload_depths);
        let cur = builtins::zend::builtin_functions::error_reporting_level(self);
        if cur != c.error_reporting {
            self.set_error_reporting(c.error_reporting);
        }
        c.error_reporting = cur;
        let b = builtins::zend::call_stack::bounds();
        builtins::zend::call_stack::set_bounds(c.stack_bounds);
        c.stack_bounds = b;
    }

    /// zend_fiber_resume_internal + zend_fiber_switch_to: run fiber `h`
    /// until it suspends (its value) or finishes (null, or its exception).
    fn fiber_switch(
        &mut self,
        h: &ObjectHandle,
        cell: &Rc<FiberCell>,
        input: Result<Value, Thrown>,
    ) -> Result<Value, Thrown> {
        let (mut co, mut ctx) = {
            let mut d = cell.borrow_mut();
            (
                d.co.take().expect("a fiber's coroutine"),
                d.ctx.take().expect("a fiber's context"),
            )
        };
        self.swap_ctx(&mut ctx);
        let prev = self.active_fiber.replace(h.clone());
        {
            let mut d = cell.borrow_mut();
            d.ctx = Some(ctx);
            d.prev = prev.clone();
            d.status = Status::Running;
        }
        let vm: *mut Vm = self;
        // The fiber runs on the VM through `vm` (see fiber_init's SAFETY);
        // nothing here holds a borrow across the switch.
        let r = co.resume(Resume { vm, value: input });
        let back = match &r {
            CoroutineResult::Yield(s) => s.vm,
            CoroutineResult::Return(f) => f.vm,
        };
        debug_assert!(std::ptr::eq(back, vm));
        let mut ctx = cell.borrow_mut().ctx.take().expect("the resumer's context");
        self.swap_ctx(&mut ctx);
        self.active_fiber = prev;
        let mut d = cell.borrow_mut();
        d.prev = None;
        match r {
            CoroutineResult::Yield(s) => {
                d.co = Some(co);
                d.ctx = Some(ctx);
                d.status = Status::Suspended;
                Ok(s.value)
            }
            CoroutineResult::Return(f) => {
                d.status = Status::Dead;
                drop(d);
                drop(co);
                drop(ctx);
                f.result.map(|()| Value::Null)
            }
        }
    }

    /// zend_fiber_execute: the coroutine's body. Calls the callable from
    /// the fiber's bottom frame and records how it ended.
    fn fiber_execute(&mut self, y: &Yielder<Resume, Suspended>) -> Result<(), Thrown> {
        let cell = cell_of(self.active_fiber.as_ref().expect("the starting fiber"));
        let (callable, args, named) = {
            let mut d = cell.borrow_mut();
            d.yielder = y;
            (
                d.callable.clone(),
                std::mem::take(&mut d.args),
                std::mem::take(&mut d.named),
            )
        };
        let r = builtins::Host::call_callable_named(self, &callable, &args, named);
        drop(args);
        // php drops the callable once it returned.
        cell.borrow_mut().callable = Value::Undef;
        drop(callable);
        match r {
            Ok(v) => {
                cell.borrow_mut().result = v;
                Ok(())
            }
            Err(t) if is_graceful_exit(&t) && cell.borrow().destroyed => Ok(()),
            Err(t @ (Thrown::Exit(_) | Thrown::Unsupported { .. })) => {
                cell.borrow_mut().bailout = true;
                Err(t)
            }
            Err(t) => {
                cell.borrow_mut().threw = true;
                Err(self.materialize(t, None))
            }
        }
    }

    /// Fiber::suspend(): hand `value` to the resumer and wait.
    fn fiber_suspend(&mut self, value: Value) -> Result<Value, Thrown> {
        let Some(h) = self.active_fiber.clone() else {
            return Err(fiber_error("Cannot suspend outside of a fiber"));
        };
        let cell = cell_of(&h);
        drop(h);
        if cell.borrow().destroyed {
            return Err(fiber_error("Cannot suspend in a force-closed fiber"));
        }
        if builtins::zend::fibers::switch_blocked() {
            return Err(fiber_error(
                "Cannot switch fibers in current execution context",
            ));
        }
        let y = cell.borrow().yielder;
        drop(cell);
        let vm: *mut Vm = self;
        // SAFETY: `y` is the running fiber's yielder (set by fiber_execute
        // on this fiber's stack, valid until the coroutine finishes, which
        // it cannot while this frame runs). The resumer takes the VM back
        // through `vm` until it resumes us.
        let input = unsafe { &*y }.suspend(Suspended { vm, value });
        debug_assert!(std::ptr::eq(input.vm, vm));
        input.value
    }

    /// The trace frames of the contexts below the running fiber: each
    /// resumer's, innermost first (zend_fetch_debug_backtrace following
    /// the fiber's bottom frame to the resumer).
    pub(crate) fn fiber_trace_tail(
        &mut self,
        out: &mut PhpArray,
        provide_object: bool,
        ignore_args: bool,
    ) {
        let mut cur = self.active_fiber.clone();
        while let Some(h) = cur {
            let cell = cell_of(&h);
            let Some(mut ctx) = cell.borrow_mut().ctx.take() else {
                break;
            };
            std::mem::swap(&mut self.stack, &mut ctx.stack);
            std::mem::swap(&mut self.native_calls, &mut ctx.native_calls);
            let part = self.build_trace_part(provide_object, ignore_args, false, false);
            std::mem::swap(&mut self.stack, &mut ctx.stack);
            std::mem::swap(&mut self.native_calls, &mut ctx.native_calls);
            let mut d = cell.borrow_mut();
            d.ctx = Some(ctx);
            for (_, v) in part.iter() {
                out.push(v.clone());
            }
            cur = d.prev.clone();
        }
    }
}

/// zend_fiber_object_gc: the values a Fiber object holds, for the cycle
/// collector: its callable, arguments and result and, while it is
/// suspended, every value of its suspended frames (variables,
/// temporaries, extra arguments, variables created by name).
#[allow(dead_code)] // the cycle collector's traversal hook
pub(crate) fn fiber_gc_values(h: &ObjectHandle) -> Vec<Value> {
    let Some(cell) = h.borrow().payload::<FiberCell>() else {
        return Vec::new();
    };
    let d = cell.borrow();
    let mut out = vec![d.callable.clone(), d.result.clone()];
    out.extend(d.args.iter().cloned());
    out.extend(d.named.iter().map(|(_, v)| v.clone()));
    if d.status == Status::Suspended {
        if let Some(ctx) = &d.ctx {
            for f in &ctx.stack {
                out.extend(f.regs.iter().cloned());
                out.extend(f.extra_args.iter().cloned());
                out.extend(f.dyn_vars.values().map(|c| Value::Ref(c.clone())));
            }
        }
    }
    out
}
