//! The `pcntl` extension (php-src `ext/pcntl/pcntl.c`), signal-handling part:
//! `pcntl_signal` / `pcntl_signal_get_handler` / `pcntl_async_signals` /
//! `pcntl_signal_dispatch`, the `pcntl_w*` wait-status decoders, and the
//! `SIG*` / `W*` constants (with the host platform's numbers).
//!
//! Handlers are recorded per run exactly as php's signal table does. `SIG_DFL`
//! and `SIG_IGN` are installed for real; a PHP callable handler is recorded
//! (so `pcntl_signal_get_handler` returns it) but not yet invoked on delivery
//! — the VM has no asynchronous tick to run it from.

use std::collections::HashMap;

use rhp_values::{conv, Value};

use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn value_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(true) => "true",
        Value::Bool(false) => "false",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) | Value::Closure(_) => "object",
        Value::Resource(_) => "resource",
        Value::Ref(r) => value_name(&r.borrow()),
    }
}

#[derive(Default)]
struct PcntlState {
    async_signals: bool,
    handlers: HashMap<i64, Value>,
    last_error: i32,
}

fn state(host: &mut dyn Host) -> Option<&mut PcntlState> {
    host.module_state("pcntl", || Box::<PcntlState>::default())
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
    }
    Ok(Value::Bool(old))
}

fn fn_signal(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let signo = conv::to_int(&deref(&args[0]));
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
    let uncatchable = signo == libc::SIGKILL as i64 || signo == libc::SIGSTOP as i64;
    if let Value::Int(h) = handler {
        if h != 0 && h != 1 {
            return Err(Thrown::error(
                "ValueError",
                "pcntl_signal(): Argument #2 ($handler) must be either SIG_DFL or SIG_IGN when an integer value is given",
            ));
        }
        let disp = if h == 0 { libc::SIG_DFL } else { libc::SIG_IGN };
        if unsafe { libc::signal(signo as i32, disp) } == libc::SIG_ERR {
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
            value_name(&handler)
        )));
    }
    if uncatchable {
        // sigaction() refuses SIGKILL/SIGSTOP: "Error assigning signal".
        if let Some(st) = state(host) {
            st.last_error = libc::EINVAL;
        }
        return Ok(Value::Bool(false));
    }
    if let Some(st) = state(host) {
        st.handlers.insert(signo, handler);
    }
    Ok(Value::Bool(true))
}

fn fn_signal_get_handler(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let signo = conv::to_int(&deref(&args[0]));
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

fn fn_signal_dispatch(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}

fn fn_get_last_error(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(
        state(host).map(|s| s.last_error).unwrap_or(0) as i64
    ))
}

fn fn_strerror(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let e = conv::to_int(&deref(&args[0])) as i32;
    let p = unsafe { libc::strerror(e) };
    let s = if p.is_null() {
        Vec::new()
    } else {
        unsafe { std::ffi::CStr::from_ptr(p).to_bytes().to_vec() }
    };
    Ok(Value::str(s))
}

macro_rules! wfn {
    ($name:ident, |$s:ident| $body:expr) => {
        fn $name(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            let $s = conv::to_int(&deref(&args[0])) as i32;
            Ok($body)
        }
    };
}

wfn!(fn_wifexited, |s| Value::Bool(libc::WIFEXITED(s)));
wfn!(fn_wifstopped, |s| Value::Bool(libc::WIFSTOPPED(s)));
wfn!(fn_wifsignaled, |s| Value::Bool(libc::WIFSIGNALED(s)));
wfn!(fn_wifcontinued, |s| Value::Bool(libc::WIFCONTINUED(s)));
wfn!(fn_wexitstatus, |s| Value::Int(libc::WEXITSTATUS(s) as i64));
wfn!(fn_wtermsig, |s| Value::Int(libc::WTERMSIG(s) as i64));
wfn!(fn_wstopsig, |s| Value::Int(libc::WSTOPSIG(s) as i64));

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

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
    put(m, "pcntl_async_signals", fn_async_signals, 0, Some(1));
    put(m, "pcntl_signal", fn_signal, 2, Some(3));
    put(
        m,
        "pcntl_signal_get_handler",
        fn_signal_get_handler,
        1,
        Some(1),
    );
    put(m, "pcntl_signal_dispatch", fn_signal_dispatch, 0, Some(0));
    put(m, "pcntl_get_last_error", fn_get_last_error, 0, Some(0));
    put(m, "pcntl_errno", fn_get_last_error, 0, Some(0));
    put(m, "pcntl_strerror", fn_strerror, 1, Some(1));
    put(m, "pcntl_wifexited", fn_wifexited, 1, Some(1));
    put(m, "pcntl_wifstopped", fn_wifstopped, 1, Some(1));
    put(m, "pcntl_wifsignaled", fn_wifsignaled, 1, Some(1));
    put(m, "pcntl_wifcontinued", fn_wifcontinued, 1, Some(1));
    put(m, "pcntl_wexitstatus", fn_wexitstatus, 1, Some(1));
    put(m, "pcntl_wtermsig", fn_wtermsig, 1, Some(1));
    put(m, "pcntl_wstopsig", fn_wstopsig, 1, Some(1));
}
