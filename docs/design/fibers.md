# Fibers

php's `Fiber` (Zend/zend_fibers.c) runs a callable on a stack of its own
and switches stacks with boost.context. rhp does the same with the
[corosensei](https://crates.io/crates/corosensei) crate: each fiber is a
`corosensei::Coroutine` on a `DefaultStack` (mmap'd, with a guard page).
The code is in `crates/rhp-vm/src/fibers.rs`; every `unsafe` block of the
switching is there.

## Why a machine stack

A fiber can suspend from any depth: from a callback a builtin runs
(`array_map`, `usort`), a destructor, an error handler, a generator body.
Those run in nested Rust run loops (`run_proto_sync`, `run_closure_sync`,
`advance_gen` call `run_frames` again), so suspending means suspending Rust
frames too. Only a separate machine stack does that.

## Per-stack VM state

What php keeps per fiber (zend_fiber_capture_vm_state) is an `ExecCtx`:
the frame stack, the running builtins (`native_calls`), the generator
stack, the magic-fetch mode, the `@` level and its stack, the constructor
frames, the timeout and unused-call markers, the autoload depths,
error_reporting and the stack bounds the `zend.max_allowed_stack_size`
checks use. A switch swaps the fiber's `ExecCtx` into the `Vm` and keeps the
resumer's in the fiber. Frame indices are per stack, so the indices cached
by suspended run loops (`top`) stay valid on both sides, and the run loop
pays nothing for fibers.

A fiber's stack starts with an internal bottom frame (php's
zend_fiber_function): what it calls is called from `[internal function]`.
Backtraces walk the running stack, then each resumer's (`fiber_trace_tail`,
following `FiberData::prev`), as zend_fetch_debug_backtrace follows the
fiber's bottom frame to the frame that resumed it. An error raised while the fiber's
stack has no user frame yet (the callable is a builtin, or its arguments
fail to bind) is reported at the resumer's line (`fiber_resumer_site`).

## The switch

`Fiber::start()/resume()/throw()` call `fiber_switch`: swap the context,
set `active_fiber` (EG(active_fiber)), and `Coroutine::resume` with a raw
`*mut Vm`. The coroutine body re-derives `&mut Vm` from it; the resumer is
blocked in `resume` and does not touch the VM until the fiber suspends or
returns, so there is one live access at a time. `Fiber::suspend()` passes
the pointer back out through the yielder. Values and exceptions cross as
data: an exception escaping the fiber's function ends the fiber and is
rethrown by the resumer; `throw()` resumes with an exception raised at the
`suspend()` call; `exit` and fatal errors (`Thrown::Exit`) end the fiber
and unwind the resumer too (php's bailout transfer).

Nothing may hold a `RefCell` borrow, an iterator into `vm.stack` or a
`&mut Frame` across a switch.

## Destruction

A Fiber's state is its object's payload. The class's `dtor_obj` destroys a
suspended fiber as zend_fiber_object_destroy does: it resumes it with php's
graceful exit, which no `catch` matches and which runs `finally` blocks;
`Fiber::suspend()` there throws "Cannot suspend in a force-closed fiber".
Fibers left at shutdown are destroyed with the other objects. A fiber that
is never destroyed (a fatal error's bailout) is dropped with
`Coroutine::force_reset`, leaking what its stack held, as php does.

The cycle collector reaches what a suspended fiber holds through
`fibers::fiber_gc_values` (php's zend_fiber_object_gc), and runs the
destructors of a collection inside a fiber in its own destructor fiber
(see gc.md). A fiber resumed only to be destroyed is not linked to its
resumer: traces from it end at its bottom, as php's do.

## Stack size

`fiber.stack_size` (default 2M) is checked as php checks it: a negative
value is refused with a warning, one below two pages makes `start()` throw.
rhp's frames take more machine stack than php's, so the stack reserved is
four times the size (at least 8M; address space, committed only as used),
while the stack limit checks use `fiber.stack_size`. Tick functions and
signal handlers block switching (zend_fiber_switch_block).
