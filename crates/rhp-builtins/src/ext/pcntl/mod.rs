//! The `pcntl` extension (php-src `ext/pcntl/pcntl.c`), signal-handling part:
//! `pcntl_signal` / `pcntl_signal_get_handler` / `pcntl_async_signals` /
//! `pcntl_signal_dispatch`, the `pcntl_w*` wait-status decoders, and the
//! `SIG*` / `W*` constants (with the host platform's numbers).
//!
//! Handlers are recorded per run exactly as php's signal table does. `SIG_DFL`
//! and `SIG_IGN` are installed for real. A PHP callable gets a real
//! `sigaction` handler (`SA_SIGINFO`, all signals masked, `SA_RESTART` unless
//! asked otherwise) that only queues the signal in a lock-free ring; the queued
//! signals are delivered to the PHP callbacks by [`dispatch`] — from
//! `pcntl_signal_dispatch()`, or, with `pcntl_async_signals(true)`, from the
//! VM's interrupt check ([`interrupt_pending`]) between instructions, as php's
//! `zend_interrupt_function` does.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicUsize, Ordering};

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::proc_fns::zpp_long;
use crate::{Host, Thrown};

#[derive(Default)]
struct PcntlState {
    async_signals: bool,
    handlers: HashMap<i64, Value>,
    last_error: i32,
    /// Signals taken off the ring but not yet handed to their callback (a
    /// handler threw; php puts the rest back on the queue).
    backlog: VecDeque<SigInfo>,
    /// php's `processing_signal_queue`: no re-entrant dispatch.
    processing: bool,
}

// ---------------------------------------------------------------------------
// Signal queue (written from the async signal handler)
// ---------------------------------------------------------------------------

/// The `siginfo_t` fields php copies into the handler's `$siginfo` array.
#[derive(Clone, Copy, Default)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct SigInfo {
    signo: i32,
    errno: i32,
    code: i32,
    pid: i64,
    uid: i64,
    status: i64,
    addr: i64,
    band: i64,
    utime: i64,
    stime: i64,
}

struct Slot {
    signo: AtomicI32,
    errno: AtomicI32,
    code: AtomicI32,
    pid: AtomicI64,
    uid: AtomicI64,
    status: AtomicI64,
    addr: AtomicI64,
    band: AtomicI64,
    utime: AtomicI64,
    stime: AtomicI64,
}

#[allow(clippy::declare_interior_mutable_const)]
const EMPTY_SLOT: Slot = Slot {
    signo: AtomicI32::new(0),
    errno: AtomicI32::new(0),
    code: AtomicI32::new(0),
    pid: AtomicI64::new(0),
    uid: AtomicI64::new(0),
    status: AtomicI64::new(0),
    addr: AtomicI64::new(0),
    band: AtomicI64::new(0),
    utime: AtomicI64::new(0),
    stime: AtomicI64::new(0),
};

/// Ring capacity (php pre-allocates `num_signals` records and drops signals
/// beyond them).
const QCAP: usize = 128;
static QUEUE: [Slot; QCAP] = [EMPTY_SLOT; QCAP];
/// Ring read/write cursors (monotonic; slot = cursor % QCAP).
static QHEAD: AtomicUsize = AtomicUsize::new(0);
static QTAIL: AtomicUsize = AtomicUsize::new(0);
/// php's `PCNTL_G(pending_signals)`.
static PENDING: AtomicBool = AtomicBool::new(false);
/// Mirror of `PCNTL_G(async_signals)` readable from the signal handler.
static ASYNC: AtomicBool = AtomicBool::new(false);
/// php's `EG(vm_interrupt)`: set by the handler when async signals are on.
static INTERRUPT: AtomicBool = AtomicBool::new(false);

/// Is a signal waiting for asynchronous dispatch? (The VM polls this between
/// instructions; a relaxed load.)
#[inline]
pub fn interrupt_pending() -> bool {
    INTERRUPT.load(Ordering::Relaxed)
}

/// (pid, uid, status, addr, band, utime, stime) of a `siginfo_t`. For SIGPOLL
/// the utime slot carries `si_fd` instead.
#[cfg(target_os = "linux")]
#[allow(clippy::unnecessary_cast)] // libc's field types vary by target arch
unsafe fn siginfo_fields(info: &libc::siginfo_t) -> [i64; 7] {
    if info.si_signo == libc::SIGPOLL {
        return [0, 0, 0, 0, info.si_band() as i64, info.si_fd() as i64, 0];
    }
    [
        info.si_pid() as i64,
        info.si_uid() as i64,
        info.si_status() as i64,
        info.si_addr() as i64,
        0,
        info.si_utime() as i64,
        info.si_stime() as i64,
    ]
}

#[cfg(not(target_os = "linux"))]
unsafe fn siginfo_fields(info: &libc::siginfo_t) -> [i64; 7] {
    [
        info.si_pid as i64,
        info.si_uid as i64,
        info.si_status as i64,
        info.si_addr as i64,
        0,
        0,
        0,
    ]
}

/// php's `pcntl_signal_handler`: record the signal (async-signal-safe: atomics
/// only) and request a VM interrupt when async signals are enabled.
extern "C" fn signal_handler(signo: i32, info: *mut libc::siginfo_t, _ctx: *mut libc::c_void) {
    let tail = QTAIL.load(Ordering::Acquire);
    if tail.wrapping_sub(QHEAD.load(Ordering::Acquire)) >= QCAP {
        // "too many signals for us to track, so we'll forget about this one"
        return;
    }
    let slot = &QUEUE[tail % QCAP];
    slot.signo.store(signo, Ordering::Relaxed);
    let (errno, code, fields) = if info.is_null() {
        (0, 0, [0; 7])
    } else {
        let info = unsafe { &*info };
        (info.si_errno, info.si_code, unsafe { siginfo_fields(info) })
    };
    slot.errno.store(errno, Ordering::Relaxed);
    slot.code.store(code, Ordering::Relaxed);
    let dst = [
        &slot.pid,
        &slot.uid,
        &slot.status,
        &slot.addr,
        &slot.band,
        &slot.utime,
        &slot.stime,
    ];
    for (d, v) in dst.iter().zip(fields) {
        d.store(v, Ordering::Relaxed);
    }
    QTAIL.store(tail.wrapping_add(1), Ordering::Release);
    PENDING.store(true, Ordering::Release);
    if ASYNC.load(Ordering::Relaxed) {
        INTERRUPT.store(true, Ordering::Release);
    }
}

/// Move every queued signal from the ring into `out` (signals blocked).
fn drain_ring(out: &mut VecDeque<SigInfo>) {
    loop {
        let head = QHEAD.load(Ordering::Acquire);
        if head == QTAIL.load(Ordering::Acquire) {
            return;
        }
        let s = &QUEUE[head % QCAP];
        out.push_back(SigInfo {
            signo: s.signo.load(Ordering::Relaxed),
            errno: s.errno.load(Ordering::Relaxed),
            code: s.code.load(Ordering::Relaxed),
            pid: s.pid.load(Ordering::Relaxed),
            uid: s.uid.load(Ordering::Relaxed),
            status: s.status.load(Ordering::Relaxed),
            addr: s.addr.load(Ordering::Relaxed),
            band: s.band.load(Ordering::Relaxed),
            utime: s.utime.load(Ordering::Relaxed),
            stime: s.stime.load(Ordering::Relaxed),
        });
        QHEAD.store(head.wrapping_add(1), Ordering::Release);
    }
}

/// php's `pcntl_siginfo_to_zval`.
fn siginfo_array(si: &SigInfo) -> Value {
    let mut a = rhp_values::PhpArray::new();
    let mut put = |k: &str, v: Value| {
        a.insert(rhp_values::ArrayKey::from_bytes(k.as_bytes()), v);
    };
    put("signo", Value::Int(si.signo as i64));
    put("errno", Value::Int(si.errno as i64));
    put("code", Value::Int(si.code as i64));
    match si.signo {
        libc::SIGCHLD => {
            put("status", Value::Int(si.status));
            #[cfg(target_os = "linux")]
            {
                put("utime", Value::Float(si.utime as f64));
                put("stime", Value::Float(si.stime as f64));
            }
            put("pid", Value::Int(si.pid));
            put("uid", Value::Int(si.uid));
        }
        libc::SIGUSR1 | libc::SIGUSR2 => {
            put("pid", Value::Int(si.pid));
            put("uid", Value::Int(si.uid));
        }
        libc::SIGILL | libc::SIGFPE | libc::SIGSEGV | libc::SIGBUS => {
            put("addr", Value::Float(si.addr as f64));
        }
        #[cfg(target_os = "linux")]
        libc::SIGPOLL => {
            put("band", Value::Int(si.band));
            put("fd", Value::Int(si.utime));
        }
        _ => {}
    }
    #[cfg(target_os = "linux")]
    if (libc::SIGRTMIN()..=libc::SIGRTMAX()).contains(&si.signo) {
        put("pid", Value::Int(si.pid));
        put("uid", Value::Int(si.uid));
    }
    Value::Array(a)
}

struct MaskGuard(libc::sigset_t);

impl MaskGuard {
    /// Block every signal (php masks all signals while it plays the queue).
    fn block_all() -> MaskGuard {
        unsafe {
            let mut all: libc::sigset_t = std::mem::zeroed();
            let mut old: libc::sigset_t = std::mem::zeroed();
            libc::sigfillset(&mut all);
            libc::sigprocmask(libc::SIG_BLOCK, &all, &mut old);
            MaskGuard(old)
        }
    }
}

impl Drop for MaskGuard {
    fn drop(&mut self) {
        unsafe {
            libc::sigprocmask(libc::SIG_SETMASK, &self.0, std::ptr::null_mut());
        }
    }
}

/// php's `pcntl_signal_dispatch()`: hand every queued signal to its PHP
/// callback (`handler($signo, $siginfo)`), in arrival order. A throwing
/// handler stops the dispatch; the remaining signals stay queued.
pub fn dispatch(host: &mut dyn Host) -> Result<(), Thrown> {
    if !PENDING.load(Ordering::Acquire) {
        INTERRUPT.store(false, Ordering::Relaxed);
        return Ok(());
    }
    let _mask = MaskGuard::block_all();
    let Some(st) = state(host) else {
        return Ok(());
    };
    if st.processing {
        return Ok(());
    }
    INTERRUPT.store(false, Ordering::Relaxed);
    let mut queue = std::mem::take(&mut st.backlog);
    drain_ring(&mut queue);
    if queue.is_empty() {
        PENDING.store(false, Ordering::Release);
        return Ok(());
    }
    st.processing = true;
    let mut result = Ok(());
    while let Some(si) = queue.pop_front() {
        let handler = state(host).and_then(|st| st.handlers.get(&(si.signo as i64)).cloned());
        if let Some(h) = handler {
            if !matches!(h, Value::Int(_)) {
                let args = [Value::Int(si.signo as i64), siginfo_array(&si)];
                if let Err(e) = host.call_callable(&h, &args) {
                    result = Err(e);
                    break;
                }
            }
        }
    }
    let Some(st) = state(host) else {
        return result;
    };
    st.processing = false;
    if queue.is_empty() {
        PENDING.store(false, Ordering::Release);
    } else {
        // Put back what the throwing handler did not get to, and ask to come
        // back once the exception has been handled.
        st.backlog = queue;
        if st.async_signals {
            INTERRUPT.store(true, Ordering::Release);
        }
    }
    // Signals that arrived meanwhile are still queued: re-arm the interrupt.
    if QHEAD.load(Ordering::Acquire) != QTAIL.load(Ordering::Acquire) {
        PENDING.store(true, Ordering::Release);
        if ASYNC.load(Ordering::Relaxed) {
            INTERRUPT.store(true, Ordering::Release);
        }
    }
    result
}

/// php's `php_signal4`: install `handler` (SIG_DFL / SIG_IGN / our queueing
/// handler) for `signo`. False when the kernel refuses (SIGKILL/SIGSTOP).
fn install(signo: i32, disp: libc::sighandler_t, restart: bool, mask_all: bool) -> bool {
    unsafe {
        let mut act: libc::sigaction = std::mem::zeroed();
        act.sa_sigaction = disp;
        if mask_all {
            libc::sigfillset(&mut act.sa_mask);
        } else {
            libc::sigemptyset(&mut act.sa_mask);
        }
        act.sa_flags = libc::SA_ONSTACK | libc::SA_SIGINFO;
        if restart {
            act.sa_flags |= libc::SA_RESTART;
        }
        libc::sigaction(signo, &act, std::ptr::null_mut()) == 0
    }
}

fn new_state() -> Box<dyn std::any::Any> {
    // A fresh run starts with async signals off (php's RINIT).
    ASYNC.store(false, Ordering::Relaxed);
    Box::<PcntlState>::default()
}

fn state(host: &mut dyn Host) -> Option<&mut PcntlState> {
    host.module_state("pcntl", new_state)
        .and_then(|s| s.downcast_mut::<PcntlState>())
}

/// php's `PCNTL_G(num_signals)` (NSIG).
#[cfg(target_os = "linux")]
const NUM_SIGNALS: i64 = 65;
#[cfg(not(target_os = "linux"))]
const NUM_SIGNALS: i64 = 32;

/// The largest signal `pcntl_signal_get_handler` accepts (NSIG-1 or SIGRTMAX).
#[cfg(target_os = "linux")]
const SIGMAX: i64 = 64;
#[cfg(not(target_os = "linux"))]
const SIGMAX: i64 = 31;

fn fn_async_signals(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let on = match args.first().map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(conv::to_bool(&v)),
    };
    let Some(st) = state(host) else {
        return Ok(Value::Bool(false));
    };
    let old = st.async_signals;
    if let Some(on) = on {
        st.async_signals = on;
        ASYNC.store(on, Ordering::Relaxed);
    }
    Ok(Value::Bool(old))
}

fn fn_signal(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let signo = zpp_long(host, args, 0, "pcntl_signal", "signal")?;
    if signo < 1 {
        return Err(Thrown::error(
            "ValueError",
            "pcntl_signal(): Argument #1 ($signal) must be greater than or equal to 1",
        ));
    }
    if signo >= NUM_SIGNALS {
        return Err(Thrown::error(
            "ValueError",
            format!("pcntl_signal(): Argument #1 ($signal) must be less than {NUM_SIGNALS}"),
        ));
    }
    let handler = deref(&args[1]);
    // `$restart_syscalls` (default true, except false for SIGALRM).
    let restart = match args.get(2).map(deref) {
        None | Some(Value::Null) => signo != libc::SIGALRM as i64,
        Some(v) => conv::to_bool(&v),
    };
    if let Value::Int(h) = handler {
        if h != 0 && h != 1 {
            return Err(Thrown::error(
                "ValueError",
                "pcntl_signal(): Argument #2 ($handler) must be either SIG_DFL or SIG_IGN when an integer value is given",
            ));
        }
        let disp = if h == 0 { libc::SIG_DFL } else { libc::SIG_IGN };
        if !install(signo as i32, disp, restart, false) {
            let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if let Some(st) = state(host) {
                st.last_error = e;
            }
            return Ok(Value::Bool(false));
        }
        if let Some(st) = state(host) {
            st.handlers.insert(signo, handler);
        }
        return Ok(Value::Bool(true));
    }
    if !host.is_callable_value(&handler) {
        if let Some(st) = state(host) {
            st.last_error = libc::EINVAL;
        }
        return Err(Thrown::type_error(format!(
            "pcntl_signal(): Argument #2 ($handler) must be of type callable|int, {} given",
            crate::zend::zpp::zval_value_name(host, &handler)
        )));
    }
    let queue_handler = signal_handler
        as extern "C" fn(i32, *mut libc::siginfo_t, *mut libc::c_void)
        as libc::sighandler_t;
    if !install(signo as i32, queue_handler, restart, true) {
        // sigaction() refuses SIGKILL/SIGSTOP: "Error assigning signal".
        let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if let Some(st) = state(host) {
            st.last_error = e;
        }
        return Ok(Value::Bool(false));
    }
    if let Some(st) = state(host) {
        st.handlers.insert(signo, handler);
    }
    Ok(Value::Bool(true))
}

fn fn_signal_get_handler(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let signo = zpp_long(host, args, 0, "pcntl_signal_get_handler", "signal")?;
    if !(1..=SIGMAX).contains(&signo) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "pcntl_signal_get_handler(): Argument #1 ($signal) must be between 1 and {SIGMAX}"
            ),
        ));
    }
    Ok(state(host)
        .and_then(|st| st.handlers.get(&signo).cloned())
        .unwrap_or(Value::Int(0)))
}

fn fn_signal_dispatch(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    dispatch(host)?;
    Ok(Value::Bool(true))
}

/// `pcntl_alarm($seconds)`: alarm(2); returns the seconds left on the previous alarm.
fn fn_alarm(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let secs = zpp_long(host, args, 0, "pcntl_alarm", "seconds")?;
    Ok(Value::Int(
        unsafe { libc::alarm(secs as libc::c_uint) } as i64
    ))
}

fn fn_get_last_error(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(
        state(host).map(|s| s.last_error).unwrap_or(0) as i64
    ))
}

fn fn_strerror(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let e = zpp_long(host, args, 0, "pcntl_strerror", "error_code")? as i32;
    let p = unsafe { libc::strerror(e) };
    let s = if p.is_null() {
        Vec::new()
    } else {
        unsafe { std::ffi::CStr::from_ptr(p).to_bytes().to_vec() }
    };
    Ok(Value::str(s))
}

fn write_ref(arg: Option<&Value>, value: Value) {
    if let Some(Value::Ref(rc)) = arg {
        *rc.borrow_mut() = value;
    }
}

fn set_last_error(host: &mut dyn Host, e: i32) {
    if let Some(st) = state(host) {
        st.last_error = e;
    }
}

/// `pcntl_fork()`: fork(2). The host first hands anything it holds back to
/// the process (so the child does not repeat it); the child continues with a
/// copy of the whole engine, as php's does.
fn fn_fork(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    host.prepare_fork();
    let id = unsafe { libc::fork() };
    if id == -1 {
        let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        set_last_error(host, e);
        let what = match e {
            libc::EAGAIN => ": Reached the maximum limit of number of processes",
            libc::ENOMEM => ": Insufficient memory",
            libc::ENOSYS => ": Unimplemented",
            libc::EBADF => ": File descriptor concurrency issue",
            // php's default case prints the bare number.
            _ => "",
        };
        crate::warn(host, "pcntl_fork", &format!("Error {e}{what}"))?;
    }
    Ok(Value::Int(id as i64))
}

/// php's `PHP_RUSAGE_TO_ARRAY`.
// `as` casts: the field widths differ across platforms (macOS vs Linux).
#[allow(clippy::unnecessary_cast)]
fn rusage_array(ru: &libc::rusage) -> Value {
    let mut a = rhp_values::PhpArray::new();
    let mut put = |k: &str, v: i64| {
        a.insert(
            rhp_values::ArrayKey::from_bytes(k.as_bytes()),
            Value::Int(v),
        );
    };
    put("ru_oublock", ru.ru_oublock as i64);
    put("ru_inblock", ru.ru_inblock as i64);
    put("ru_msgsnd", ru.ru_msgsnd as i64);
    put("ru_msgrcv", ru.ru_msgrcv as i64);
    put("ru_maxrss", ru.ru_maxrss as i64);
    put("ru_ixrss", ru.ru_ixrss as i64);
    put("ru_idrss", ru.ru_idrss as i64);
    put("ru_minflt", ru.ru_minflt as i64);
    put("ru_majflt", ru.ru_majflt as i64);
    put("ru_nsignals", ru.ru_nsignals as i64);
    put("ru_nvcsw", ru.ru_nvcsw as i64);
    put("ru_nivcsw", ru.ru_nivcsw as i64);
    put("ru_nswap", ru.ru_nswap as i64);
    put("ru_utime.tv_usec", ru.ru_utime.tv_usec as i64);
    put("ru_utime.tv_sec", ru.ru_utime.tv_sec as i64);
    put("ru_stime.tv_usec", ru.ru_stime.tv_usec as i64);
    put("ru_stime.tv_sec", ru.ru_stime.tv_sec as i64);
    Value::Array(a)
}

/// Shared body of `pcntl_waitpid` / `pcntl_wait`: wait4(2), writing the
/// status (and, when asked for, the resource usage) back.
fn wait_common(
    host: &mut dyn Host,
    pid: i64,
    status_arg: Option<&Value>,
    options: i64,
    rusage_arg: Option<&Value>,
) -> Value {
    let mut status = status_arg.map_or(0, |v| conv::to_int(&deref(v))) as i32;
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    if rusage_arg.is_some() {
        write_ref(rusage_arg, Value::Array(rhp_values::PhpArray::new()));
    }
    // Our queueing handlers carry SA_RESTART like php's, so a signal only
    // interrupts the wait (EINTR, -1) when a handler asked for no restart.
    let child = unsafe { libc::wait4(pid as libc::pid_t, &mut status, options as i32, &mut ru) };
    if child < 0 {
        let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        set_last_error(host, e);
    }
    if child > 0 && rusage_arg.is_some() {
        write_ref(rusage_arg, rusage_array(&ru));
    }
    write_ref(status_arg, Value::Int(status as i64));
    Value::Int(child as i64)
}

fn fn_waitpid(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let pid = zpp_long(host, args, 0, "pcntl_waitpid", "process_id")?;
    let options = match args.get(2) {
        Some(_) => zpp_long(host, args, 2, "pcntl_waitpid", "flags")?,
        None => 0,
    };
    Ok(wait_common(host, pid, args.get(1), options, args.get(3)))
}

fn fn_wait(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let options = match args.get(1) {
        Some(_) => zpp_long(host, args, 1, "pcntl_wait", "flags")?,
        None => 0,
    };
    Ok(wait_common(host, -1, args.first(), options, args.get(2)))
}

/// `pcntl_sigprocmask($mode, $signals, &$old_signals)`.
fn fn_sigprocmask(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let how = zpp_long(host, args, 0, "pcntl_sigprocmask", "mode")?;
    if how != libc::SIG_BLOCK as i64
        && how != libc::SIG_UNBLOCK as i64
        && how != libc::SIG_SETMASK as i64
    {
        return Err(Thrown::error(
            "ValueError",
            "pcntl_sigprocmask(): Argument #1 ($mode) must be one of SIG_BLOCK, SIG_UNBLOCK, or SIG_SETMASK",
        ));
    }
    let sigs = crate::zend::zpp::array_param(host, args, 1, "pcntl_sigprocmask")?;
    if sigs.is_empty() && how != libc::SIG_SETMASK as i64 {
        return Err(Thrown::error(
            "ValueError",
            "pcntl_sigprocmask(): Argument #2 ($signals) must not be empty",
        ));
    }
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    let mut old: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut set);
        libc::sigemptyset(&mut old);
    }
    for (_, v) in sigs.iter() {
        let v = deref(v);
        let Some(n) = crate::zend::zpp::try_get_long(host, &v)? else {
            return Err(Thrown::type_error(format!(
                "pcntl_sigprocmask(): Argument #2 ($signals) signals must be of type int, {} given",
                crate::zend::zpp::zval_value_name(host, &v)
            )));
        };
        if !(1..NUM_SIGNALS).contains(&n) {
            return Err(Thrown::error(
                "ValueError",
                format!(
                    "pcntl_sigprocmask(): Argument #2 ($signals) signals must be between 1 and {}",
                    NUM_SIGNALS - 1
                ),
            ));
        }
        unsafe { libc::sigaddset(&mut set, n as i32) };
    }
    if unsafe { libc::sigprocmask(how as i32, &set, &mut old) } != 0 {
        let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        set_last_error(host, e);
        crate::warn(
            host,
            "pcntl_sigprocmask",
            &crate::main::streams::strerror(e),
        )?;
        return Ok(Value::Bool(false));
    }
    if args.get(2).is_some() {
        let mut a = rhp_values::PhpArray::new();
        for s in 1..NUM_SIGNALS {
            if unsafe { libc::sigismember(&old, s as i32) } == 1 {
                a.push(Value::Int(s));
            }
        }
        write_ref(args.get(2), Value::Array(a));
    }
    Ok(Value::Bool(true))
}

macro_rules! wfn {
    ($name:ident, $php:literal, |$s:ident| $body:expr) => {
        fn $name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            let $s = zpp_long(host, args, 0, $php, "status")? as i32;
            Ok($body)
        }
    };
}

wfn!(fn_wifexited, "pcntl_wifexited", |s| Value::Bool(
    libc::WIFEXITED(s)
));
wfn!(fn_wifstopped, "pcntl_wifstopped", |s| Value::Bool(
    libc::WIFSTOPPED(s)
));
wfn!(fn_wifsignaled, "pcntl_wifsignaled", |s| Value::Bool(
    libc::WIFSIGNALED(s)
));
wfn!(fn_wifcontinued, "pcntl_wifcontinued", |s| Value::Bool(
    libc::WIFCONTINUED(s)
));
wfn!(fn_wexitstatus, "pcntl_wexitstatus", |s| Value::Int(
    libc::WEXITSTATUS(s) as i64
));
wfn!(fn_wtermsig, "pcntl_wtermsig", |s| Value::Int(
    libc::WTERMSIG(s) as i64
));
wfn!(fn_wstopsig, "pcntl_wstopsig", |s| Value::Int(
    libc::WSTOPSIG(s) as i64
));

pub fn constants() -> Vec<(&'static str, Value)> {
    let i = |n: i32| Value::Int(n as i64);
    #[allow(unused_mut)]
    let mut v = vec![
        ("WNOHANG", i(libc::WNOHANG)),
        ("WUNTRACED", i(libc::WUNTRACED)),
        ("WCONTINUED", i(libc::WCONTINUED)),
        ("SIG_IGN", Value::Int(1)),
        ("SIG_DFL", Value::Int(0)),
        ("SIG_ERR", Value::Int(-1)),
        ("SIGHUP", i(libc::SIGHUP)),
        ("SIGINT", i(libc::SIGINT)),
        ("SIGQUIT", i(libc::SIGQUIT)),
        ("SIGILL", i(libc::SIGILL)),
        ("SIGTRAP", i(libc::SIGTRAP)),
        ("SIGABRT", i(libc::SIGABRT)),
        ("SIGIOT", i(libc::SIGIOT)),
        ("SIGBUS", i(libc::SIGBUS)),
        ("SIGFPE", i(libc::SIGFPE)),
        ("SIGKILL", i(libc::SIGKILL)),
        ("SIGUSR1", i(libc::SIGUSR1)),
        ("SIGSEGV", i(libc::SIGSEGV)),
        ("SIGUSR2", i(libc::SIGUSR2)),
        ("SIGPIPE", i(libc::SIGPIPE)),
        ("SIGALRM", i(libc::SIGALRM)),
        ("SIGTERM", i(libc::SIGTERM)),
        ("SIGCHLD", i(libc::SIGCHLD)),
        ("SIGCONT", i(libc::SIGCONT)),
        ("SIGSTOP", i(libc::SIGSTOP)),
        ("SIGTSTP", i(libc::SIGTSTP)),
        ("SIGTTIN", i(libc::SIGTTIN)),
        ("SIGTTOU", i(libc::SIGTTOU)),
        ("SIGURG", i(libc::SIGURG)),
        ("SIGXCPU", i(libc::SIGXCPU)),
        ("SIGXFSZ", i(libc::SIGXFSZ)),
        ("SIGVTALRM", i(libc::SIGVTALRM)),
        ("SIGPROF", i(libc::SIGPROF)),
        ("SIGWINCH", i(libc::SIGWINCH)),
        ("SIGIO", i(libc::SIGIO)),
        ("SIGSYS", i(libc::SIGSYS)),
        ("SIGBABY", i(libc::SIGSYS)),
        ("SIG_BLOCK", i(libc::SIG_BLOCK)),
        ("SIG_UNBLOCK", i(libc::SIG_UNBLOCK)),
        ("SIG_SETMASK", i(libc::SIG_SETMASK)),
        ("PRIO_PGRP", Value::Int(libc::PRIO_PGRP as i64)),
        ("PRIO_USER", Value::Int(libc::PRIO_USER as i64)),
        ("PRIO_PROCESS", Value::Int(libc::PRIO_PROCESS as i64)),
        ("PCNTL_EINTR", i(libc::EINTR)),
        ("PCNTL_ECHILD", i(libc::ECHILD)),
        ("PCNTL_EINVAL", i(libc::EINVAL)),
        ("PCNTL_EAGAIN", i(libc::EAGAIN)),
        ("PCNTL_ESRCH", i(libc::ESRCH)),
        ("PCNTL_EACCES", i(libc::EACCES)),
        ("PCNTL_EPERM", i(libc::EPERM)),
        ("PCNTL_ENOMEM", i(libc::ENOMEM)),
        ("PCNTL_E2BIG", i(libc::E2BIG)),
        ("PCNTL_EFAULT", i(libc::EFAULT)),
        ("PCNTL_EIO", i(libc::EIO)),
        ("PCNTL_EISDIR", i(libc::EISDIR)),
        ("PCNTL_ELOOP", i(libc::ELOOP)),
        ("PCNTL_EMFILE", i(libc::EMFILE)),
        ("PCNTL_ENAMETOOLONG", i(libc::ENAMETOOLONG)),
        ("PCNTL_ENFILE", i(libc::ENFILE)),
        ("PCNTL_ENOENT", i(libc::ENOENT)),
        ("PCNTL_ENOEXEC", i(libc::ENOEXEC)),
        ("PCNTL_ENOTDIR", i(libc::ENOTDIR)),
        ("PCNTL_ETXTBSY", i(libc::ETXTBSY)),
        ("PCNTL_ENOSPC", i(libc::ENOSPC)),
        ("PCNTL_EUSERS", i(libc::EUSERS)),
    ];
    #[cfg(target_os = "linux")]
    {
        v.push(("SIGSTKFLT", i(libc::SIGSTKFLT)));
        v.push(("SIGCLD", i(libc::SIGCHLD)));
        v.push(("SIGPOLL", i(libc::SIGPOLL)));
        v.push(("SIGPWR", i(libc::SIGPWR)));
        v.push(("SIGRTMIN", i(libc::SIGRTMIN())));
        v.push(("SIGRTMAX", i(libc::SIGRTMAX())));
    }
    v
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("pcntl_async_signals", fn_async_signals),
            ("pcntl_signal", fn_signal),
            ("pcntl_signal_get_handler", fn_signal_get_handler),
            ("pcntl_signal_dispatch", fn_signal_dispatch),
            ("pcntl_alarm", fn_alarm),
            ("pcntl_get_last_error", fn_get_last_error),
            ("pcntl_errno", fn_get_last_error),
            ("pcntl_strerror", fn_strerror),
            ("pcntl_fork", fn_fork),
            ("pcntl_waitpid", fn_waitpid),
            ("pcntl_wait", fn_wait),
            ("pcntl_sigprocmask", fn_sigprocmask),
            ("pcntl_wifexited", fn_wifexited),
            ("pcntl_wifstopped", fn_wifstopped),
            ("pcntl_wifsignaled", fn_wifsignaled),
            ("pcntl_wifcontinued", fn_wifcontinued),
            ("pcntl_wexitstatus", fn_wexitstatus),
            ("pcntl_wtermsig", fn_wtermsig),
            ("pcntl_wstopsig", fn_wstopsig),
        ],
    );
}
