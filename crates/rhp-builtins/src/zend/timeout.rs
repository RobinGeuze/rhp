//! max_execution_time (Zend/zend_execute_API.c's zend_set_timeout,
//! zend_timeout_handler and zend_timeout): when the timer fires it sets
//! `EG(timed_out)` and the VM interrupt flag; the VM then raises "Maximum
//! execution time of N seconds exceeded" where php's interrupt checks are:
//! where a jump lands, where a user function starts, or when a builtin call
//! returns.
//!
//! Two timers, as php has two builds:
//!
//! - The command-line SAPIs (one request per process) use php's NTS timer:
//!   a process-wide one-shot interval timer (ITIMER_PROF, ITIMER_REAL on
//!   Apple Silicon where ITIMER_PROF is broken) whose signal handler sets
//!   the flags. hard_timeout re-arms it from the handler, so shutdown
//!   functions get one more window; a timeout the VM does not get to in
//!   time kills the process. A blocking builtin ends early only if the
//!   signal interrupts its system call (sleep under ITIMER_REAL).
//! - Every other SAPI (an embedding server running requests on many
//!   threads) gets php's ZTS behaviour (zend_max_execution_timer.c): a
//!   wall-clock timer per thread, here one watchdog thread that sets the
//!   timed-out thread's own flags. There is no hard timeout, as in ZTS
//!   builds. A blocking builtin is not interrupted: the error comes when it
//!   returns.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

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

/// Whether this process may own the timer (one request at a time): the
/// command-line SAPIs. Every other SAPI uses its thread's timer.
fn armable() -> bool {
    crate::main::sapi::is_command_line()
}

/// zend_set_timeout_ex: arm the timer for `seconds` (0: leave it), and with
/// `reset_signals` (re)install the handler.
fn set_timeout_ex(seconds: i64, reset_signals: bool) {
    if armable() {
        arm(seconds, reset_signals);
    } else {
        thread_arm(seconds);
    }
}

/// A thread's timer state (ZTS's `EG(timed_out)` and `EG(vm_interrupt)`),
/// shared with the watchdog.
struct ThreadTimer {
    /// The VM's interrupt flag on this thread ([`interrupt_flag`]).
    interrupt: AtomicBool,
    /// `EG(timed_out)`.
    timed_out: AtomicBool,
}

thread_local! {
    static THREAD_TIMER: Arc<ThreadTimer> = Arc::new(ThreadTimer {
        interrupt: AtomicBool::new(false),
        timed_out: AtomicBool::new(false),
    });
    /// `EG(timeout_seconds)` of the request on this thread.
    static THREAD_SECONDS: Cell<i64> = const { Cell::new(0) };
}

/// The deadlines of the armed thread timers, and the thread that fires
/// them.
struct Watchdog {
    armed: Mutex<Vec<(Instant, Arc<ThreadTimer>)>>,
    wake: Condvar,
}

fn watchdog() -> &'static Watchdog {
    static W: OnceLock<Watchdog> = OnceLock::new();
    static STARTED: std::sync::Once = std::sync::Once::new();
    let w = W.get_or_init(|| Watchdog {
        armed: Mutex::new(Vec::new()),
        wake: Condvar::new(),
    });
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("rhp-timeout".into())
            .spawn(move || watch(w));
    });
    w
}

/// The watchdog thread: wait for the earliest deadline, then set that
/// thread's flags (zend_timeout_handler under ZTS).
fn watch(w: &'static Watchdog) {
    let mut armed = w.armed.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        let now = Instant::now();
        armed.retain(|(at, t)| {
            if *at > now {
                return true;
            }
            t.timed_out.store(true, Ordering::SeqCst);
            t.interrupt.store(true, Ordering::SeqCst);
            false
        });
        armed = match armed.iter().map(|e| e.0).min() {
            Some(at) => {
                w.wake
                    .wait_timeout(armed, at - now)
                    .unwrap_or_else(|e| e.into_inner())
                    .0
            }
            None => w.wake.wait(armed).unwrap_or_else(|e| e.into_inner()),
        };
    }
}

/// zend_max_execution_timer_settime for this thread: fire in `seconds`
/// (0, or out of range: leave the timer as it is).
fn thread_arm(seconds: i64) {
    if !(1..=999_999_999).contains(&seconds) {
        return;
    }
    let t = THREAD_TIMER.with(Arc::clone);
    let w = watchdog();
    let mut armed = w.armed.lock().unwrap_or_else(|e| e.into_inner());
    armed.retain(|(_, x)| !Arc::ptr_eq(x, &t));
    armed.push((Instant::now() + Duration::from_secs(seconds as u64), t));
    w.wake.notify_one();
}

/// zend_max_execution_timer_settime(0): disarm this thread's timer.
fn thread_disarm() {
    let t = THREAD_TIMER.with(Arc::clone);
    let w = watchdog();
    let mut armed = w.armed.lock().unwrap_or_else(|e| e.into_inner());
    armed.retain(|(_, x)| !Arc::ptr_eq(x, &t));
}

/// The flag the VM polls between instructions for this thread's engine
/// (`EG(vm_interrupt)`): the process-wide one (pcntl's signals and the
/// interval timer) under the command-line SAPIs, else this thread's own.
/// Valid for as long as the thread runs.
pub fn interrupt_flag() -> &'static AtomicBool {
    if armable() {
        return crate::ext::pcntl::process_interrupt();
    }
    THREAD_TIMER.with(|t| {
        // SAFETY: the thread-local's Arc keeps the timer alive until the
        // thread exits; the engine using it lives on this thread.
        unsafe { &*(&t.interrupt as *const AtomicBool) }
    })
}

/// This thread's `EG(timed_out)` (threaded SAPIs).
fn thread_timed_out() -> &'static AtomicBool {
    THREAD_TIMER.with(|t| {
        // SAFETY: as in interrupt_flag.
        unsafe { &*(&t.timed_out as *const AtomicBool) }
    })
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

/// Where the engine is executing, for the hard timeout's message: copies
/// the innermost user frame's file name into the buffer and returns its
/// length and the line. It runs in the signal handler, so it may only
/// read (no allocation, no locks).
pub type Locator = unsafe fn(ctx: *const (), file: &mut [u8]) -> Option<(usize, u32)>;

/// The running engine's [`Locator`] and its argument (`EG(current_execute_data)`).
static LOCATE_CTX: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
static LOCATE_FN: AtomicUsize = AtomicUsize::new(0);

/// Install the running engine's locator (`ctx` must stay valid until
/// [`clear_locator`]). Only the process-wide timer has a hard timeout, so
/// only the command-line SAPIs install one.
pub fn set_locator(ctx: *const (), f: Locator) {
    if !armable() {
        return;
    }
    LOCATE_FN.store(f as usize, Ordering::Relaxed);
    LOCATE_CTX.store(ctx as *mut (), Ordering::Release);
}

/// The engine stopped executing: the hard timeout names no file.
pub fn clear_locator() {
    LOCATE_CTX.store(std::ptr::null_mut(), Ordering::Release);
}

/// zend_get_executed_filename/lineno for the handler: the file (into
/// `buf`) and line, or "Unknown" and 0 when nothing executes.
fn executed_location(buf: &mut [u8]) -> (usize, u32) {
    let ctx = LOCATE_CTX.load(Ordering::Acquire);
    let f = LOCATE_FN.load(Ordering::Relaxed);
    if !ctx.is_null() && f != 0 {
        // SAFETY: set_locator stored a Locator and a context it keeps alive.
        let f: Locator = unsafe { std::mem::transmute::<usize, Locator>(f) };
        if let Some(r) = unsafe { f(ctx, buf) } {
            return r;
        }
    }
    buf[..7].copy_from_slice(b"Unknown");
    (7, 0)
}

/// zend_timeout_handler.
extern "C" fn handler(_sig: libc::c_int) {
    if TIMED_OUT.load(Ordering::Relaxed) {
        // The VM never got to the first timeout: die (hard timeout).
        let secs = SECONDS.load(Ordering::Relaxed);
        let hard = HARD.load(Ordering::Relaxed);
        // Formatted into stack buffers: no allocation in a signal handler.
        let mut file = [0u8; 1024];
        let (n, line) = executed_location(&mut file);
        // php's log_buffer is 2048 bytes.
        let mut buf = [0u8; 2048];
        let mut w = &mut buf[..];
        use std::io::Write;
        let _ = write!(
            w,
            "\nFatal error: Maximum execution time of {secs}+{hard} seconds exceeded (terminated) in "
        );
        let _ = w.write_all(&file[..n]);
        let _ = writeln!(w, " on line {line}");
        let len = 2048 - w.len();
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
    if armable() {
        SECONDS.store(seconds, Ordering::Relaxed);
        set_timeout_ex(seconds, reset_signals);
        TIMED_OUT.store(false, Ordering::Relaxed);
    } else {
        THREAD_SECONDS.with(|s| s.set(seconds));
        thread_arm(seconds);
        thread_timed_out().store(false, Ordering::Relaxed);
    }
}

/// zend_unset_timeout: disarm the timer.
pub fn unset_timeout() {
    if !armable() {
        if THREAD_SECONDS.with(Cell::get) != 0 {
            thread_disarm();
        }
        // After the disarm: a deadline that passed meanwhile is forgotten.
        thread_timed_out().store(false, Ordering::Relaxed);
        return;
    }
    if SECONDS.load(Ordering::Relaxed) != 0 {
        // SAFETY: a zeroed itimerval disarms the timer.
        unsafe {
            let t: libc::itimerval = std::mem::zeroed();
            libc::setitimer(TIMER.0, &t, std::ptr::null_mut());
        }
    }
    TIMED_OUT.store(false, Ordering::Relaxed);
}

fn ini_long(name: &[u8]) -> i64 {
    crate::main::ini::c_atol(&crate::ini_value(name).flatten().unwrap_or_default())
}

/// php_request_startup's timer: max_input_time while the request's input
/// is read, else max_execution_time (hard_timeout read once, as at module
/// startup).
pub fn request_startup() {
    HARD.store(ini_long(b"hard_timeout"), Ordering::Relaxed);
    let input = ini_long(b"max_input_time");
    if input == -1 {
        set_timeout(ini_long(b"max_execution_time"), true);
    } else {
        set_timeout(input, true);
    }
}

/// php_execute_script: with a max_input_time, the script itself runs
/// under max_execution_time.
pub fn script_startup() {
    if ini_long(b"max_input_time") != -1 {
        set_timeout(ini_long(b"max_execution_time"), false);
    }
}

/// OnUpdateTimeout at runtime (`set_time_limit`, `ini_set`).
pub fn on_update(v: &[u8]) {
    unset_timeout();
    set_timeout(crate::main::ini::c_atol(v), false);
}

/// Whether the timer fired and the VM has not raised the error yet.
#[inline]
pub fn timed_out() -> bool {
    if armable() {
        TIMED_OUT.load(Ordering::Relaxed)
    } else {
        thread_timed_out().load(Ordering::SeqCst)
    }
}

/// zend_timeout: the fatal error's message, once the timer fired.
pub fn take_timed_out() -> Option<String> {
    let secs = if armable() {
        if !TIMED_OUT.swap(false, Ordering::Relaxed) {
            return None;
        }
        SECONDS.load(Ordering::Relaxed)
    } else {
        if !thread_timed_out().swap(false, Ordering::Relaxed) {
            return None;
        }
        THREAD_SECONDS.with(Cell::get)
    };
    // php_on_timeout.
    crate::main::set_connection_timeout();
    let s = if secs == 1 { "" } else { "s" };
    Some(format!(
        "Maximum execution time of {secs} second{s} exceeded"
    ))
}
