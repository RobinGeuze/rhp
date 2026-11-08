//! ext/readline (readline.c) on libedit, the library php links: the
//! `readline_*` functions, `READLINE_LIB` and the module's phpinfo block.
//! The interactive shell (readline_cli.c, `php -a`) is in [`cli`].
//!
//! libedit calls back into PHP (the completion function, the callback
//! interface's line handler) from inside `readline()` and
//! `rl_callback_read_char()`: those builtins publish their `Host` for the
//! duration of the call, and a throw from the callback surfaces when the
//! builtin returns.

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_int, CStr, CString};

use rhp_values::ops::deref;
use rhp_values::{PhpArray, Value};

use crate::zend::zpp::{path_param, require_callback, str_param, zval_get_long, zval_get_string};
use crate::{Host, Thrown};

pub mod cli;
mod sys;

thread_local! {
    /// `_readline_completion`: the completion callable.
    static COMPLETION: RefCell<Option<Value>> = const { RefCell::new(None) };
    /// `_prepped_callback`: the callback interface's line handler.
    static PREPPED: RefCell<Option<Value>> = const { RefCell::new(None) };
    /// The matches the completion callable returned, as strings, and the
    /// generator's position in them.
    static MATCHES: RefCell<(Vec<Vec<u8>>, usize)> = const { RefCell::new((Vec::new(), 0)) };
    /// The host of the builtin libedit is running under.
    static HOST: Cell<Option<*mut (dyn Host + 'static)>> = const { Cell::new(None) };
    /// A throw from a callback, for the builtin to return.
    static THROWN: RefCell<Option<Thrown>> = const { RefCell::new(None) };
    /// `php_readline_custom_line_buffer` / `_readline_name`: the copies
    /// readline_info() handed libedit.
    static CUSTOM: Cell<(*mut c_char, *mut c_char)> =
        const { Cell::new((std::ptr::null_mut(), std::ptr::null_mut())) };
}

/// Run `f` (a libedit call that may call back into PHP) with `host`
/// published to the callbacks; then the callbacks' throw, if any.
fn with_host<R>(host: &mut dyn Host, f: impl FnOnce() -> R) -> Result<R, Thrown> {
    // SAFETY: the pointer is only used by the callbacks f runs, while the
    // borrow it came from is suspended in this frame.
    let ptr: *mut (dyn Host + 'static) = unsafe { std::mem::transmute(host as *mut dyn Host) };
    let prev = HOST.with(|h| h.replace(Some(ptr)));
    let r = f();
    HOST.with(|h| h.set(prev));
    match THROWN.with(|t| t.borrow_mut().take()) {
        Some(t) => Err(t),
        None => Ok(r),
    }
}

/// Call `callable` from a libedit callback; a throw is kept for later.
fn callback(callable: &Value, args: &[Value]) -> Option<Value> {
    let host = HOST.with(|h| h.get())?;
    if THROWN.with(|t| t.borrow().is_some()) {
        return None;
    }
    // SAFETY: with_host published it for this call.
    let host = unsafe { &mut *host };
    match host.call_callable(callable, args) {
        Ok(v) => Some(v),
        Err(t) => {
            THROWN.with(|c| *c.borrow_mut() = Some(t));
            None
        }
    }
}

/// A C string of `s` up to its first NUL, as C sees it.
fn cstr(s: &[u8]) -> CString {
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    CString::new(&s[..end]).unwrap_or_default()
}

/// `SAFE_STRING`.
fn safe(p: *const c_char) -> Vec<u8> {
    if p.is_null() {
        Vec::new()
    } else {
        // SAFETY: libedit's strings are NUL-terminated.
        unsafe { CStr::from_ptr(p) }.to_bytes().to_vec()
    }
}

/// `readline(?string $prompt = null)`.
fn fn_readline(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let prompt = match args.first().map(deref) {
        None | Some(Value::Null) => None,
        Some(_) => Some(cstr(&str_param(host, args, 0, "readline", "prompt")?)),
    };
    let p = prompt.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
    // SAFETY: p is NULL or a live C string; the result is malloc'd.
    let line = with_host(host, || unsafe { sys::readline(p) })?;
    if line.is_null() {
        return Ok(Value::Bool(false));
    }
    let v = safe(line);
    // SAFETY: readline's result is the caller's to free.
    unsafe { libc::free(line.cast()) };
    Ok(Value::str(v))
}

/// `readline_info(?string $var_name = null, $value = null)`.
fn fn_readline_info(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let what = match args.first().map(deref) {
        None | Some(Value::Null) => None,
        Some(_) => Some(str_param(host, args, 0, "readline_info", "var_name")?),
    };
    let value = args.get(1).map(deref).filter(|v| !matches!(v, Value::Null));
    // SAFETY: libedit's globals, read and written on this thread only.
    unsafe {
        let Some(what) = what else {
            let mut a = PhpArray::new();
            let s = |k: &str| rhp_values::ArrayKey::from_bytes(k.as_bytes());
            a.insert(s("line_buffer"), Value::str(safe(sys::rl_line_buffer)));
            a.insert(s("point"), Value::Int(sys::rl_point as i64));
            a.insert(s("end"), Value::Int(sys::rl_end as i64));
            a.insert(
                s("library_version"),
                Value::str(safe(sys::rl_library_version)),
            );
            a.insert(s("readline_name"), Value::str(safe(sys::rl_readline_name)));
            a.insert(
                s("attempted_completion_over"),
                Value::Int(sys::rl_attempted_completion_over as i64),
            );
            return Ok(Value::Array(a));
        };
        let what = what.to_ascii_lowercase();
        Ok(match &what[..] {
            b"line_buffer" => {
                let old = Value::str(safe(sys::rl_line_buffer));
                if let Some(v) = value {
                    let s = zval_get_string(host, &v)?;
                    let copy = libc::strdup(cstr(&s).as_ptr());
                    sys::rl_line_buffer = copy;
                    let (lb, name) = CUSTOM.with(|c| c.get());
                    if !lb.is_null() {
                        libc::free(lb.cast());
                    }
                    CUSTOM.with(|c| c.set((copy, name)));
                    sys::rl_end = s.len() as c_int;
                }
                old
            }
            b"point" => Value::Int(sys::rl_point as i64),
            b"end" => Value::Int(sys::rl_end as i64),
            b"library_version" => Value::str(safe(sys::rl_library_version)),
            b"readline_name" => {
                let old = Value::str(safe(sys::rl_readline_name));
                if let Some(v) = value {
                    let s = zval_get_string(host, &v)?;
                    let copy = libc::strdup(cstr(&s).as_ptr());
                    sys::rl_readline_name = copy;
                    let (lb, name) = CUSTOM.with(|c| c.get());
                    if !name.is_null() {
                        libc::free(name.cast());
                    }
                    CUSTOM.with(|c| c.set((lb, copy)));
                }
                old
            }
            b"attempted_completion_over" => {
                let old = sys::rl_attempted_completion_over as i64;
                if let Some(v) = value {
                    sys::rl_attempted_completion_over = zval_get_long(host, &v)? as c_int;
                }
                Value::Int(old)
            }
            _ => Value::Null,
        })
    }
}

fn fn_readline_add_history(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let line = cstr(&str_param(host, args, 0, "readline_add_history", "prompt")?);
    // SAFETY: a live C string.
    unsafe { sys::add_history(line.as_ptr()) };
    Ok(Value::Bool(true))
}

fn fn_readline_clear_history(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    // SAFETY: libedit calls without arguments. php calls using_history
    // first: clear_history alone does not initialise libedit.
    unsafe {
        sys::using_history();
        sys::clear_history();
    }
    Ok(Value::Bool(true))
}

/// The `?string $filename` of the history functions (`|p!`), after the
/// open_basedir check; `Err(None)`: refused.
fn history_file(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
) -> Result<Result<Option<CString>, ()>, Thrown> {
    match args.first().map(deref) {
        None | Some(Value::Null) => Ok(Ok(None)),
        Some(_) => {
            let p = path_param(host, args, 0, fname, "filename")?;
            if !crate::main::fopen_wrappers::check(host, fname, &p, true)? {
                return Ok(Err(()));
            }
            Ok(Ok(Some(cstr(&p))))
        }
    }
}

fn fn_readline_read_history(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Ok(f) = history_file(host, args, "readline_read_history")? else {
        return Ok(Value::Bool(false));
    };
    let p = f.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
    // SAFETY: NULL (~/.history) or a live C string.
    Ok(Value::Bool(unsafe { sys::read_history(p) } == 0))
}

fn fn_readline_write_history(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Ok(f) = history_file(host, args, "readline_write_history")? else {
        return Ok(Value::Bool(false));
    };
    let p = f.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
    // SAFETY: NULL (~/.history) or a live C string.
    Ok(Value::Bool(unsafe { sys::write_history(p) } == 0))
}

/// `_readline_command_generator`: the returned entries that start with
/// `text`, one per call.
unsafe extern "C" fn command_generator(text: *const c_char, state: c_int) -> *mut c_char {
    let prefix = safe(text);
    MATCHES.with(|m| {
        let mut m = m.borrow_mut();
        if state == 0 {
            m.1 = 0;
        }
        while m.1 < m.0.len() {
            let i = m.1;
            m.1 += 1;
            if m.0[i].starts_with(&prefix) {
                // SAFETY: libedit frees what the generator returns.
                return unsafe { libc::strdup(cstr(&m.0[i]).as_ptr()) };
            }
        }
        std::ptr::null_mut()
    })
}

/// `php_readline_completion_cb`.
unsafe extern "C" fn completion_cb(
    text: *const c_char,
    start: c_int,
    end: c_int,
) -> *mut *mut c_char {
    let Some(cb) = COMPLETION.with(|c| c.borrow().clone()) else {
        return std::ptr::null_mut();
    };
    let t = if text.is_null() {
        Value::Null
    } else {
        Value::str(safe(text))
    };
    let Some(Value::Array(a)) =
        callback(&cb, &[t, Value::Int(start as i64), Value::Int(end as i64)]).map(|v| deref(&v))
    else {
        return std::ptr::null_mut();
    };
    if a.is_empty() {
        // libedit reads matches[2].
        // SAFETY: a zeroed array of three pointers, the first a "".
        unsafe {
            let m = libc::calloc(3, std::mem::size_of::<*mut c_char>()) as *mut *mut c_char;
            if !m.is_null() {
                *m = libc::strdup(c"".as_ptr());
            }
            return m;
        }
    }
    // convert_to_string on each entry.
    let mut words = Vec::new();
    if let Some(host) = HOST.with(|h| h.get()) {
        // SAFETY: published by with_host for this call.
        let host = unsafe { &mut *host };
        for (_, v) in a.iter() {
            match zval_get_string(host, v) {
                Ok(s) => words.push(s),
                Err(t) => {
                    THROWN.with(|c| *c.borrow_mut() = Some(t));
                    return std::ptr::null_mut();
                }
            }
        }
    }
    MATCHES.with(|m| *m.borrow_mut() = (words, 0));
    // SAFETY: text is libedit's; the generator returns malloc'd strings.
    unsafe { sys::rl_completion_matches(text, command_generator) }
}

fn fn_readline_completion_function(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cb = args.first().map(deref).unwrap_or(Value::Null);
    require_callback(host, "readline_completion_function", 1, &cb)?;
    COMPLETION.with(|c| *c.borrow_mut() = Some(cb));
    // SAFETY: libedit's hook, set on this thread.
    unsafe { sys::rl_attempted_completion_function = Some(completion_cb) };
    Ok(Value::Bool(true))
}

/// `php_rl_callback_handler`.
unsafe extern "C" fn line_handler(line: *mut c_char) {
    let Some(cb) = PREPPED.with(|p| p.borrow().clone()) else {
        return;
    };
    let arg = if line.is_null() {
        Value::Null
    } else {
        Value::str(safe(line))
    };
    let _ = callback(&cb, &[arg]);
}

fn fn_readline_callback_handler_install(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let prompt = cstr(&str_param(
        host,
        args,
        0,
        "readline_callback_handler_install",
        "prompt",
    )?);
    let cb = args.get(1).map(deref).unwrap_or(Value::Null);
    require_callback(host, "readline_callback_handler_install", 2, &cb)?;
    // SAFETY: libedit's callback interface, on this thread.
    unsafe {
        if PREPPED.with(|p| p.borrow().is_some()) {
            sys::rl_callback_handler_remove();
        }
        PREPPED.with(|p| *p.borrow_mut() = Some(cb));
        with_host(host, || {
            sys::rl_callback_handler_install(prompt.as_ptr(), line_handler)
        })?;
    }
    Ok(Value::Bool(true))
}

fn fn_readline_callback_read_char(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    if PREPPED.with(|p| p.borrow().is_some()) {
        // SAFETY: a handler is installed.
        with_host(host, || unsafe { sys::rl_callback_read_char() })?;
    }
    Ok(Value::Null)
}

fn fn_readline_callback_handler_remove(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    if PREPPED.with(|p| p.borrow_mut().take()).is_some() {
        // SAFETY: a handler was installed.
        unsafe { sys::rl_callback_handler_remove() };
        return Ok(Value::Bool(true));
    }
    Ok(Value::Bool(false))
}

fn fn_readline_redisplay(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    // SAFETY: libedit calls without arguments (using_history initialises
    // libedit, which rl_redisplay does not; bug #72538).
    unsafe {
        sys::using_history();
        sys::rl_redisplay();
    }
    Ok(Value::Null)
}

fn fn_readline_on_new_line(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    // SAFETY: a libedit call without arguments.
    unsafe { sys::rl_on_new_line() };
    Ok(Value::Null)
}

/// `READLINE_LIB`.
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    vec![("READLINE_LIB", Value::str(b"libedit".as_slice()))]
}

/// `rl_library_version` (phpinfo's "Readline library").
pub fn library_version() -> Vec<u8> {
    // SAFETY: libedit's constant string.
    let v = safe(unsafe { sys::rl_library_version });
    if v.is_empty() {
        b"Unknown".to_vec()
    } else {
        v
    }
}

/// PHP_RSHUTDOWN_FUNCTION(readline).
pub(crate) fn reset_state() {
    COMPLETION.with(|c| *c.borrow_mut() = None);
    MATCHES.with(|m| *m.borrow_mut() = (Vec::new(), 0));
    THROWN.with(|t| *t.borrow_mut() = None);
    if PREPPED.with(|p| p.borrow_mut().take()).is_some() {
        // SAFETY: a handler was installed.
        unsafe { sys::rl_callback_handler_remove() };
    }
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("readline", fn_readline),
            ("readline_info", fn_readline_info),
            ("readline_add_history", fn_readline_add_history),
            ("readline_clear_history", fn_readline_clear_history),
            ("readline_read_history", fn_readline_read_history),
            ("readline_write_history", fn_readline_write_history),
            (
                "readline_completion_function",
                fn_readline_completion_function,
            ),
            (
                "readline_callback_handler_install",
                fn_readline_callback_handler_install,
            ),
            (
                "readline_callback_read_char",
                fn_readline_callback_read_char,
            ),
            (
                "readline_callback_handler_remove",
                fn_readline_callback_handler_remove,
            ),
            ("readline_redisplay", fn_readline_redisplay),
            ("readline_on_new_line", fn_readline_on_new_line),
        ],
    );
}
