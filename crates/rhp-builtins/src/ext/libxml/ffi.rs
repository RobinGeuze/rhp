//! How C callbacks from libxml2 reach the VM.
//!
//! A builtin that calls into libxml2 runs the C call inside [`with_host`],
//! which pushes the builtin's `Host` onto a thread-local stack. A callback
//! (`extern "C"`) runs its body through [`guard`], which catches panics, and
//! reaches PHP through [`call_host`], which finds the innermost host.
//!
//! php longjmps out of libxml2 on `exit` and fatal errors; Rust cannot. So a
//! callback stashes what PHP threw, and [`with_host`] rethrows it once the C
//! call has returned. An exception leaves the C operation running, as in
//! php, where handlers check `EG(exception)` and do nothing
//! ([`aborted`]); anything else that unwinds (`exit`) also stops it, through
//! the stop function the caller registered (`xmlStopParser`).

use std::any::Any;
use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::{Host, Thrown};

struct Frame {
    host: *mut (dyn Host + 'static),
    fname: String,
    pending: Option<Thrown>,
    panic: Option<Box<dyn Any + Send>>,
    stop: Option<Box<dyn Fn()>>,
}

thread_local! {
    static FRAMES: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
}

/// Run `f` (a libxml2 call) with `host` reachable from the callbacks it
/// triggers, which raise diagnostics as builtin `fname`. Rethrows what a
/// callback stashed, and resumes a callback's panic.
pub fn with_host<R>(host: &mut dyn Host, fname: &str, f: impl FnOnce() -> R) -> Result<R, Thrown> {
    match with_host_keep(host, fname, f) {
        (_, Some(t)) => Err(t),
        (r, None) => Ok(r),
    }
}

/// [`with_host`], handing back `f`'s result also when a callback threw, for
/// a caller that has to free what the result owns.
pub fn with_host_keep<R>(
    host: &mut dyn Host,
    fname: &str,
    f: impl FnOnce() -> R,
) -> (R, Option<Thrown>) {
    let raw: *mut (dyn Host + '_) = host;
    // SAFETY: the pointer is popped before `host`'s borrow ends, and `host`
    // is not used while `f` runs.
    let raw: *mut (dyn Host + 'static) = unsafe { std::mem::transmute(raw) };
    FRAMES.with(|s| {
        s.borrow_mut().push(Frame {
            host: raw,
            fname: fname.to_string(),
            pending: None,
            panic: None,
            stop: None,
        })
    });
    struct Pop;
    impl Drop for Pop {
        fn drop(&mut self) {
            let _ = FRAMES.try_with(|s| s.borrow_mut().pop());
        }
    }
    let pop = Pop;
    let r = f();
    let frame = FRAMES.with(|s| s.borrow_mut().pop());
    std::mem::forget(pop);
    let frame = frame.expect("libxml host frame");
    if let Some(p) = frame.panic {
        std::panic::resume_unwind(p);
    }
    (r, frame.pending)
}

/// While the innermost [`with_host`] runs, `stop` ends its C operation
/// early (on `exit` or a panic in a callback).
pub fn set_stop(stop: impl Fn() + 'static) {
    FRAMES.with(|s| {
        if let Some(f) = s.borrow_mut().last_mut() {
            f.stop = Some(Box::new(stop));
        }
    });
}

/// Whether a host is reachable (a builtin is running a libxml2 call).
pub fn has_host() -> bool {
    FRAMES.with(|s| !s.borrow().is_empty())
}

/// The builtin the innermost libxml2 call runs for.
pub fn fname() -> String {
    FRAMES.with(|s| {
        s.borrow()
            .last()
            .map(|f| f.fname.clone())
            .unwrap_or_default()
    })
}

/// php's `EG(exception) != NULL` inside a callback: something PHP threw is
/// waiting to be rethrown.
pub fn aborted() -> bool {
    FRAMES.with(|s| s.borrow().last().is_some_and(|f| f.pending.is_some()))
}

/// Run `f` on the innermost host. What it throws is stashed for
/// [`with_host`] to rethrow, and `None` returned; also `None` when no host is
/// reachable.
pub fn call_host<R>(f: impl FnOnce(&mut dyn Host) -> Result<R, Thrown>) -> Option<R> {
    let host = FRAMES.with(|s| s.borrow().last().map(|f| f.host))?;
    // SAFETY: the frame's host outlives the C call (see `with_host`), and
    // the stack is not borrowed while PHP runs, so nested libxml2 calls push
    // their own frames.
    let r = f(unsafe { &mut *host });
    match r {
        Ok(v) => Some(v),
        Err(t) => {
            stash(t);
            None
        }
    }
}

/// Keep `t` for [`with_host`] to rethrow. The first one wins, as in php
/// (later handlers see `EG(exception)` and do nothing). Anything but an
/// exception stops the C operation.
pub fn stash(t: Thrown) {
    let stop = !t.is_catchable();
    let stopper = FRAMES.with(|s| {
        let mut s = s.borrow_mut();
        let f = s.last_mut()?;
        if f.pending.is_none() {
            f.pending = Some(t);
        }
        if stop {
            f.stop.take()
        } else {
            None
        }
    });
    if let Some(stop) = stopper {
        stop();
    }
}

/// The body of an `extern "C"` callback: a panic must not unwind into C. It
/// is kept, the C operation stopped, and the panic resumed after the C call
/// returns; meanwhile the callback returns `default`.
pub fn guard<R>(default: R, f: impl FnOnce() -> R) -> R {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(p) => {
            let stopper = FRAMES
                .try_with(|s| {
                    let mut s = s.try_borrow_mut().ok()?;
                    let fr = s.last_mut()?;
                    if fr.panic.is_none() {
                        fr.panic = Some(p);
                    }
                    fr.stop.take()
                })
                .ok()
                .flatten();
            if let Some(stop) = stopper {
                stop();
            }
            default
        }
    }
}
