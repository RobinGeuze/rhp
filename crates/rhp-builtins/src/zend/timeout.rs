//! max_execution_time (Zend/zend_execute_API.c's zend_set_timeout,
//! zend_timeout_handler and zend_timeout): a one-shot interval timer whose
//! signal sets `EG(timed_out)` and the VM interrupt flag; the VM then
//! raises "Maximum execution time of N seconds exceeded" where php's
//! interrupt checks are: where a jump lands, where a user function starts,
//! or when a builtin call returns. hard_timeout re-arms the timer from the handler, so
//! shutdown functions get one more window; a timeout the VM does not get
//! to in time kills the process, as in php. Like php's NTS build this uses
//! a process-wide timer (ITIMER_PROF, ITIMER_REAL on Apple Silicon where
//! ITIMER_PROF is broken), so only the command-line SAPIs arm it. A
//! blocking builtin ends early only if the signal interrupts its system
//! call (sleep under ITIMER_REAL), as in php.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

/// `EG(timed_out)`.
static TIMED_OUT: AtomicBool = AtomicBool::new(false);
/// `EG(timeout_seconds)`.
static SECONDS: AtomicI64 = AtomicI64::new(0);
/// `EG(hard_timeout)`.
static HARD: AtomicI64 = AtomicI64::new(0);

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const TIMER: (libc::c_int, libc::c_int) = (libc::ITIMER_REAL, libc::SIGALRM);
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
const TIMER: (libc::c_int, libc::c_int) = (libc::ITIMER_PROF, libc::SIGPROF);

/// Whether this process may own the timer (one request at a time).
fn armable() -> bool {
    crate::main::sapi::is_command_line()
}

/// zend_set_timeout_ex: arm the timer for `seconds` (0: leave it), and with
/// `reset_signals` (re)install the handler.
fn set_timeout_ex(seconds: i64, reset_signals: bool) {
    if armable() {
        arm(seconds, reset_signals);
    }
}

/// [`set_timeout_ex`] without the SAPI check (the signal handler calls it:
/// no thread-locals, no allocation).
fn arm(seconds: i64, reset_signals: bool) {
    let seconds = if (0..=999_999_999).contains(&seconds) {
        seconds
    } else {
        0
    };
    // SAFETY: setitimer/sigaction/sigprocmask on zero-initialised structs;
    // the handler only touches atomics and async-signal-safe calls.
    unsafe {
        if seconds != 0 {
            let mut t: libc::itimerval = std::mem::zeroed();
            t.it_value.tv_sec = seconds as libc::time_t;
            libc::setitimer(TIMER.0, &t, std::ptr::null_mut());
        }
        if reset_signals {
            let mut act: libc::sigaction = std::mem::zeroed();
            act.sa_sigaction = handler as extern "C" fn(libc::c_int) as libc::sighandler_t;
            libc::sigemptyset(&mut act.sa_mask);
            act.sa_flags = libc::SA_ONSTACK | libc::SA_RESETHAND | libc::SA_NODEFER;
            libc::sigaction(TIMER.1, &act, std::ptr::null_mut());
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, TIMER.1);
            libc::sigprocmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
        }
    }
}

/// zend_timeout_handler.
extern "C" fn handler(_sig: libc::c_int) {
    if TIMED_OUT.load(Ordering::Relaxed) {
        // The VM never got to the first timeout: die (hard timeout).
        let secs = SECONDS.load(Ordering::Relaxed);
        let hard = HARD.load(Ordering::Relaxed);
        // Formatted into a stack buffer: no allocation in a signal handler.
        let mut buf = [0u8; 192];
        let mut w = &mut buf[..];
        let _ = std::io::Write::write_fmt(
            &mut w,
            format_args!(
                "\nFatal error: Maximum execution time of {secs}+{hard} seconds exceeded (terminated) in [no active file] on line 0\n"
            ),
        );
        let len = 192 - w.len();
        // SAFETY: write(2) and _exit are async-signal-safe.
        unsafe {
            libc::write(2, buf.as_ptr().cast(), len);
            libc::_exit(124);
        }
    }
    TIMED_OUT.store(true, Ordering::Relaxed);
    crate::ext::pcntl::raise_interrupt();
    let hard = HARD.load(Ordering::Relaxed);
    if hard > 0 {
        arm(hard, true);
    }
}

/// zend_set_timeout: the request's (or set_time_limit's) limit.
pub fn set_timeout(seconds: i64, reset_signals: bool) {
    SECONDS.store(seconds, Ordering::Relaxed);
    set_timeout_ex(seconds, reset_signals);
    TIMED_OUT.store(false, Ordering::Relaxed);
}

/// zend_unset_timeout: disarm the timer.
pub fn unset_timeout() {
    if armable() && SECONDS.load(Ordering::Relaxed) != 0 {
        // SAFETY: a zeroed itimerval disarms the timer.
        unsafe {
            let t: libc::itimerval = std::mem::zeroed();
            libc::setitimer(TIMER.0, &t, std::ptr::null_mut());
        }
    }
    TIMED_OUT.store(false, Ordering::Relaxed);
}

/// php_request_startup's timer: max_execution_time (hard_timeout read
/// once, as at module startup).
pub fn request_startup() {
    let ini = |n: &[u8]| crate::ini_value(n).flatten().unwrap_or_default();
    HARD.store(
        crate::main::ini::c_atol(&ini(b"hard_timeout")),
        Ordering::Relaxed,
    );
    let secs = crate::main::ini::c_atol(&ini(b"max_execution_time"));
    set_timeout(secs, true);
}

/// OnUpdateTimeout at runtime (`set_time_limit`, `ini_set`).
pub fn on_update(v: &[u8]) {
    unset_timeout();
    set_timeout(crate::main::ini::c_atol(v), false);
}

/// Whether the timer fired and the VM has not raised the error yet.
#[inline]
pub fn timed_out() -> bool {
    TIMED_OUT.load(Ordering::Relaxed)
}

/// zend_timeout: the fatal error's message, once the timer fired.
pub fn take_timed_out() -> Option<String> {
    if !TIMED_OUT.swap(false, Ordering::Relaxed) {
        return None;
    }
    // php_on_timeout.
    crate::main::set_connection_timeout();
    let secs = SECONDS.load(Ordering::Relaxed);
    let s = if secs == 1 { "" } else { "s" };
    Some(format!(
        "Maximum execution time of {secs} second{s} exceeded"
    ))
}
