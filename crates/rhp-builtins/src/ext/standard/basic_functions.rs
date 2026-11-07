//! ext/standard/basic_functions.c: the standard extension's own functions
//! (`constant`, `call_user_func`, `getenv`/`putenv`, the `ini_*` functions,
//! `getopt`, `sleep`, `error_log`, `print_r`, ...).

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::ext::standard::array::as_array;
use crate::ext::standard::pageinfo::script_stat;
use crate::main::ini::{
    alter_ini_entry, ini_apply, ini_default_value, ini_parse_uquantity, ini_value, module_loaded,
    INI, INI_STARTUP, STARTUP_DIAGS,
};
use crate::main::ini_table::INI_TABLE;
use crate::main::network;
use crate::main::output::ob;
use crate::zend::builtin_functions::resolve_const;
use crate::zend::ini_parser::{syntax_error, Event, Parser};
use crate::zend::ini_scanner::{lexer, trim_ws, MODE_NORMAL, MODE_RAW, MODE_TYPED};
use crate::zend::virtual_cwd::os_path;
use crate::zend::zpp::{bool_param, int_param, str_or_null_param, str_param, zpp_long};
use crate::{ext, zend, Host, Thrown};

fn fn_print_r(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = Vec::new();
    let r = ext::standard::var::resolve_views(
        host,
        &args[0],
        zend::object_handlers::PropPurpose::Debug,
    );
    if r.is_ok() {
        ext::standard::var::print_r(host, &mut out, &args[0], 0);
    }
    ext::standard::var::clear_views();
    r?;
    let return_mode = args.get(1).map(conv::to_bool).unwrap_or(false);
    if return_mode {
        Ok(Value::str(out))
    } else {
        host.write_out(&out);
        Ok(Value::Bool(true))
    }
}

fn fn_constant(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = conv::to_string_bytes(&deref(&args[0]));
    Ok(resolve_const(host, &name, false)?.unwrap_or(Value::Null))
}

// The engine's zpp checked the callback (Z_PARAM_FUNC).
fn fn_call_user_func(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    host.call_callable(&args[0], &args[1..])
}

fn fn_call_user_func_array(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let params = as_array(&args[1], "call_user_func_array")?;
    let mut argv: Vec<Value> = Vec::new();
    let mut named: Vec<(rhp_values::PhpStr, Value)> = Vec::new();
    for (k, v) in params.iter() {
        match k {
            rhp_values::ArrayKey::Int(_) if !named.is_empty() => {
                return Err(Thrown::error(
                    "Error",
                    "Cannot use positional argument after named argument",
                ))
            }
            rhp_values::ArrayKey::Int(_) => argv.push(v.clone()),
            rhp_values::ArrayKey::Str(s) => named.push((s.clone(), v.clone())),
        }
    }
    if named.is_empty() {
        return host.call_callable(&args[0], &argv);
    }
    host.call_callable_named(&args[0], &argv, named)
}

/// basic_functions.c's check that forward_static_call runs in a method.
fn forward_scope_check(host: &dyn Host, func: &str) -> Result<(), Thrown> {
    if host.caller_scope().is_none() {
        return Err(Thrown::error(
            "Error",
            format!("Cannot call {func}() when no class scope is active"),
        ));
    }
    Ok(())
}

// The engine's zpp checked the callback (Z_PARAM_FUNC).
fn fn_forward_static_call(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    forward_scope_check(host, "forward_static_call")?;
    host.call_callable_forward(&args[0], &args[1..])
}

fn fn_forward_static_call_array(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let params = as_array(&args[1], "forward_static_call_array")?;
    // (No class-scope check here, unlike forward_static_call.)
    let mut argv: Vec<Value> = Vec::new();
    let mut named: Vec<(rhp_values::PhpStr, Value)> = Vec::new();
    for (k, v) in params.iter() {
        match k {
            ArrayKey::Int(_) if !named.is_empty() => {
                return Err(Thrown::error(
                    "Error",
                    "Cannot use positional argument after named argument",
                ))
            }
            ArrayKey::Int(_) => argv.push(v.clone()),
            ArrayKey::Str(s) => named.push((s.clone(), v.clone())),
        }
    }
    if !named.is_empty() {
        return host.call_callable_named(&args[0], &argv, named);
    }
    host.call_callable_forward(&args[0], &argv)
}

/// getservbyname(3) and friends return static storage: one caller at a
/// time.
static NETDB: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn c_str(b: &[u8]) -> std::ffi::CString {
    // path_param has rejected NUL bytes.
    std::ffi::CString::new(b).unwrap_or_default()
}

/// `getservbyname($service, $protocol)`: the port, or false.
fn fn_getservbyname(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = path_param(host, args, 0, "getservbyname", "service")?;
    let proto = path_param(host, args, 1, "getservbyname", "protocol")?;
    let (name, proto) = (c_str(&name), c_str(&proto));
    let _g = NETDB.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: NUL-terminated arguments; the entry is read under NETDB.
    let port = unsafe {
        let se = libc::getservbyname(name.as_ptr(), proto.as_ptr());
        (!se.is_null()).then(|| u16::from_be((*se).s_port as u16))
    };
    Ok(port.map_or(Value::Bool(false), |p| Value::Int(p as i64)))
}

/// `getservbyport($port, $protocol)`: the service name, or false.
fn fn_getservbyport(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let port = conv::to_int(&deref(&args[0]));
    let proto = path_param(host, args, 1, "getservbyport", "protocol")?;
    let proto = c_str(&proto);
    let _g = NETDB.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: as in getservbyname; the name is copied under NETDB.
    let name = unsafe {
        let se = libc::getservbyport((port as u16).to_be() as libc::c_int, proto.as_ptr());
        (!se.is_null()).then(|| std::ffi::CStr::from_ptr((*se).s_name).to_bytes().to_vec())
    };
    Ok(name.map_or(Value::Bool(false), Value::str))
}

/// `getprotobyname($protocol)`: the protocol number, or false.
fn fn_getprotobyname(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = path_param(host, args, 0, "getprotobyname", "protocol")?;
    let name = c_str(&name);
    let _g = NETDB.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: as in getservbyname.
    let n = unsafe {
        let pe = libc::getprotobyname(name.as_ptr());
        (!pe.is_null()).then(|| (*pe).p_proto)
    };
    Ok(n.map_or(Value::Bool(false), |n| Value::Int(n as i64)))
}

/// `getprotobynumber($protocol)`: the protocol name, or false.
fn fn_getprotobynumber(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = conv::to_int(&deref(&args[0]));
    let _g = NETDB.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: as in getservbyport.
    let name = unsafe {
        let pe = libc::getprotobynumber(n as libc::c_int);
        (!pe.is_null()).then(|| std::ffi::CStr::from_ptr((*pe).p_name).to_bytes().to_vec())
    };
    Ok(name.map_or(Value::Bool(false), Value::str))
}

fn fn_flush(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::SapiFlush)
}

/// One `putenv` change: name and new value (`None` = unset).
pub type EnvChange = (Vec<u8>, Option<Vec<u8>>);

thread_local! {
    /// `putenv` changes for this request, layered over the process
    /// environment (`None` = unset). The process environment itself is never
    /// mutated: that would race with other threads' `getenv` (the test
    /// harness runs engines on parallel threads) and leak across engines.
    static ENV_OVERLAY: std::cell::RefCell<Vec<EnvChange>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// basic_functions.c's `INI_SCANNER_*`, `CONNECTION_*` and `INI_*`.
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("INI_SCANNER_NORMAL", Value::Int(MODE_NORMAL)),
        ("INI_SCANNER_RAW", Value::Int(MODE_RAW)),
        ("INI_SCANNER_TYPED", Value::Int(MODE_TYPED)),
        ("CONNECTION_ABORTED", Value::Int(1)),
        ("CONNECTION_NORMAL", Value::Int(0)),
        ("CONNECTION_TIMEOUT", Value::Int(2)),
        ("INI_USER", Value::Int(1)),
        ("INI_PERDIR", Value::Int(2)),
        ("INI_SYSTEM", Value::Int(4)),
        ("INI_ALL", Value::Int(7)),
    ]
}

pub(crate) fn reset_state() {
    ENV_OVERLAY.with(|e| e.borrow_mut().clear());
    restore_process_tz();
    TICK_FUNCS.with(|t| t.borrow_mut().clear());
}

/// One register_tick_function() entry (user_tick_function_entry).
struct TickFn {
    id: u64,
    callback: Value,
    params: Vec<Value>,
    /// Running now: not called again from its own ticks.
    calling: bool,
}

thread_local! {
    /// BG(user_tick_functions), in registration order.
    static TICK_FUNCS: std::cell::RefCell<Vec<TickFn>> = const { std::cell::RefCell::new(Vec::new()) };
    static TICK_NEXT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// php_run_ticks: call every registered tick function (the engine's
/// ZEND_TICKS when its count is reached). A function already running is
/// skipped.
pub fn run_tick_functions(host: &mut dyn Host) -> Result<(), Thrown> {
    let ids: Vec<u64> = TICK_FUNCS.with(|t| t.borrow().iter().map(|f| f.id).collect());
    for id in ids {
        let call = TICK_FUNCS.with(|t| {
            let mut t = t.borrow_mut();
            let f = t.iter_mut().find(|f| f.id == id && !f.calling)?;
            f.calling = true;
            Some((f.callback.clone(), f.params.clone()))
        });
        let Some((cb, params)) = call else {
            continue;
        };
        // ZEND_TICKS blocks fiber switches around the tick functions.
        let block = crate::zend::fibers::SwitchBlock::new();
        let r = host.call_callable(&cb, &params);
        drop(block);
        TICK_FUNCS.with(|t| {
            if let Some(f) = t.borrow_mut().iter_mut().find(|f| f.id == id) {
                f.calling = false;
            }
        });
        r?;
    }
    Ok(())
}

/// What zend_fcc_equals compares of a callable: the bound object (by
/// identity) or closure, and the lowercased class and function names.
#[derive(PartialEq)]
enum CallableKey {
    Closure(usize),
    Func(Vec<u8>),
    Method(Option<usize>, Vec<u8>, Vec<u8>),
}

fn callable_key(host: &dyn Host, v: &Value) -> Option<CallableKey> {
    let name = |b: &[u8]| {
        let b = b.strip_prefix(b"\\").unwrap_or(b);
        b.to_ascii_lowercase()
    };
    match deref(v) {
        Value::Closure(c) => Some(CallableKey::Closure(
            std::rc::Rc::as_ptr(&c) as *const u8 as usize
        )),
        Value::Str(s) => {
            let s = s.as_bytes();
            match s.windows(2).position(|w| w == b"::") {
                Some(i) => Some(CallableKey::Method(None, name(&s[..i]), name(&s[i + 2..]))),
                None => Some(CallableKey::Func(name(s))),
            }
        }
        Value::Array(a) if a.len() == 2 => {
            let target = deref(a.get(&ArrayKey::Int(0))?);
            let method = conv::to_string_bytes(&deref(a.get(&ArrayKey::Int(1))?));
            match target {
                Value::Object(o) => {
                    let class = host.object_class_name(&Value::Object(o.clone()))?;
                    let id = std::rc::Rc::as_ptr(&o) as *const u8 as usize;
                    Some(CallableKey::Method(Some(id), name(&class), name(&method)))
                }
                Value::Str(c) => Some(CallableKey::Method(None, name(c.as_bytes()), name(&method))),
                _ => None,
            }
        }
        _ => None,
    }
}

fn fn_register_tick_function(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let id = TICK_NEXT_ID.with(|n| {
        n.set(n.get() + 1);
        n.get()
    });
    let f = TickFn {
        id,
        callback: deref(&args[0]),
        params: args[1..].iter().map(deref).collect(),
        calling: false,
    };
    TICK_FUNCS.with(|t| t.borrow_mut().push(f));
    Ok(Value::Bool(true))
}

fn fn_unregister_tick_function(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let key = callable_key(host, &args[0]);
    let funcs: Vec<(u64, Value, bool)> = TICK_FUNCS.with(|t| {
        t.borrow()
            .iter()
            .map(|f| (f.id, f.callback.clone(), f.calling))
            .collect()
    });
    // zend_llist_del_element: the first equal entry.
    for (id, cb, calling) in funcs {
        if key.is_some() && callable_key(host, &cb) == key {
            if calling {
                return Err(Thrown::error(
                    "Error",
                    "Registered tick function cannot be unregistered while it is being executed",
                ));
            }
            TICK_FUNCS.with(|t| t.borrow_mut().retain(|f| f.id != id));
            break;
        }
    }
    Ok(Value::Null)
}

/// The request's `putenv` changes in order (`None` = unset). A spawner of
/// child processes must apply these (`Command::env` / `env_remove`) so the
/// child sees the same environment as `getenv`, as in php.
pub fn env_overrides() -> Vec<EnvChange> {
    ENV_OVERLAY.with(|e| e.borrow().clone())
}

/// Apply [`env_overrides`] to a child-process command.
pub fn apply_env_overrides(cmd: &mut std::process::Command) {
    for (k, v) in env_overrides() {
        let k = String::from_utf8_lossy(&k).into_owned();
        match v {
            Some(v) => {
                cmd.env(k, String::from_utf8_lossy(&v).into_owned());
            }
            None => {
                cmd.env_remove(k);
            }
        }
    }
}

/// `getenv(name)` as php sees it: the request's `putenv` changes, then the
/// process environment.
pub(crate) fn env_get(name: &[u8]) -> Option<Vec<u8>> {
    if let Some(v) = ENV_OVERLAY.with(|e| {
        e.borrow()
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }) {
        return v;
    }
    std::env::var_os(String::from_utf8_lossy(name).as_ref())
        .map(|v| v.to_string_lossy().into_owned().into_bytes())
}

/// `$local_only` (arg 2) skips the SAPI's own environment. rhp's SAPIs
/// keep the request environment in the process one, so both lookups agree.
fn fn_getenv(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match args.first() {
        Some(v) if !matches!(deref(v), Value::Null) => {
            let name = conv::to_string_bytes(&deref(v));
            match env_get(&name) {
                Some(val) => Ok(Value::str(val)),
                None => Ok(Value::Bool(false)),
            }
        }
        _ => {
            let mut a = PhpArray::new();
            for (k, v) in std::env::vars_os() {
                a.insert(
                    rhp_values::ArrayKey::from_bytes(k.to_string_lossy().as_bytes()),
                    Value::str(v.to_string_lossy().into_owned().into_bytes()),
                );
            }
            for (k, v) in env_overrides() {
                let key = rhp_values::ArrayKey::from_bytes(&k);
                match v {
                    Some(v) => {
                        a.insert(key, Value::str(v));
                    }
                    None => {
                        a.remove(&key);
                    }
                }
            }
            Ok(Value::Array(a))
        }
    }
}

/// `putenv("NAME=value")` sets, `putenv("NAME")` unsets (in the request's
/// environment overlay: `getenv` sees it; see [`env_overrides`]).
fn fn_putenv(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = crate::zend::zpp::str_param(host, args, 0, "putenv", "assignment")?;
    if a.is_empty() || a[0] == b'=' {
        return Err(Thrown::error(
            "ValueError",
            "putenv(): Argument #1 ($assignment) must have a valid syntax",
        ));
    }
    let (name, value) = match a.iter().position(|&b| b == b'=') {
        Some(p) => (&a[..p], Some(&a[p + 1..])),
        None => (&a[..], None),
    };
    if name.contains(&0) || value.is_some_and(|v| v.contains(&0)) {
        return Ok(Value::Bool(false));
    }
    if name == b"TZ" {
        set_process_tz(value);
    }
    let entry = (name.to_vec(), value.map(|v| v.to_vec()));
    ENV_OVERLAY.with(|e| e.borrow_mut().push(entry));
    Ok(Value::Bool(true))
}

/// TZ as the process started with it, once putenv() changed it.
static ORIG_TZ: std::sync::Mutex<Option<Option<Vec<u8>>>> = std::sync::Mutex::new(None);

/// putenv("TZ=…") changes the C library's time zone, as in php (putenv(3)
/// then tzset()): strptime() and friends read it. The process environment
/// is shared, as it is in php.
fn set_process_tz(value: Option<&[u8]>) {
    let mut orig = ORIG_TZ.lock().unwrap_or_else(|e| e.into_inner());
    if orig.is_none() {
        *orig = Some(std::env::var_os("TZ").map(|v| {
            use std::os::unix::ffi::OsStrExt;
            v.as_bytes().to_vec()
        }));
    }
    apply_tz(value);
}

fn apply_tz(value: Option<&[u8]>) {
    extern "C" {
        fn tzset();
    }
    // SAFETY: valid C strings; php changes the environment the same way.
    unsafe {
        match value.and_then(|v| std::ffi::CString::new(v).ok()) {
            Some(v) => libc::setenv(c"TZ".as_ptr(), v.as_ptr(), 1),
            None => libc::unsetenv(c"TZ".as_ptr()),
        };
        tzset();
    }
}

/// php restores the environment putenv() changed at request end.
fn restore_process_tz() {
    let mut orig = ORIG_TZ.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(tz) = orig.take() {
        apply_tz(tz.as_deref());
    }
}

fn fn_error_get_last(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(match host.last_error() {
        None => Value::Null,
        Some((ty, msg, file, line)) => {
            let mut a = rhp_values::PhpArray::new();
            a.insert(
                rhp_values::ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"type".to_vec())),
                Value::Int(ty),
            );
            a.insert(
                rhp_values::ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"message".to_vec())),
                Value::Str(rhp_values::PhpStr::from_bytes(msg)),
            );
            a.insert(
                rhp_values::ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"file".to_vec())),
                Value::Str(rhp_values::PhpStr::from_bytes(file)),
            );
            a.insert(
                rhp_values::ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"line".to_vec())),
                Value::Int(line),
            );
            if let Some(t) = host.last_error_trace() {
                a.insert(rhp_values::ArrayKey::from_bytes(b"trace"), t);
            }
            Value::Array(a)
        }
    })
}

fn fn_error_clear_last(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    host.clear_last_error();
    Ok(Value::Null)
}

/// `register_shutdown_function(callable $callback, mixed ...$args): void`.
fn fn_register_shutdown_function(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cb = deref(&args[0]);
    // Checked by the engine's zpp already.
    if let Some(why) = host.callable_error(&cb) {
        return Err(Thrown::type_error(format!(
            "register_shutdown_function(): Argument #1 ($callback) must be a valid callback, {why}"
        )));
    }
    // php keeps the resolved zend_fcall_info_cache: `self` / `static` mean
    // the registering scope, not the (absent) one at shutdown.
    let cb = match &cb {
        Value::Array(a) if a.len() == 2 => {
            let mut it = a.iter();
            let (Some((k0, Value::Str(c))), Some((_, m))) = (it.next(), it.next()) else {
                return finish_shutdown_registration(host, cb.clone(), args);
            };
            let scope = if c.as_bytes().eq_ignore_ascii_case(b"self") {
                host.caller_scope()
            } else if c.as_bytes().eq_ignore_ascii_case(b"static") {
                host.caller_called_scope()
            } else {
                None
            };
            match scope.and_then(|id| host.class_name(id)) {
                Some(name) if matches!(k0, rhp_values::ArrayKey::Int(0)) => {
                    let mut r = rhp_values::PhpArray::new();
                    r.push(Value::str(name));
                    r.push(m.clone());
                    Value::Array(r)
                }
                _ => cb.clone(),
            }
        }
        _ => cb,
    };
    finish_shutdown_registration(host, cb, args)
}

fn finish_shutdown_registration(
    host: &mut dyn Host,
    cb: Value,
    args: &[Value],
) -> Result<Value, Thrown> {
    host.register_shutdown_function(cb, args[1..].iter().map(deref).collect())?;
    Ok(Value::Null)
}

fn ini_opt_value(v: Option<Vec<u8>>) -> Value {
    match v {
        Some(s) => Value::str(s),
        None => Value::Null,
    }
}

fn fn_ini_get(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, "ini_get", "option")?;
    Ok(match ini_value(&name) {
        None => Value::Bool(false),
        Some(v) => Value::str(v.unwrap_or_default()),
    })
}

fn ini_set_impl(host: &mut dyn Host, args: &[Value], func: &str) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, func, "option")?;
    let newv = match deref(&args[1]) {
        Value::Null => Vec::new(),
        v @ (Value::Str(_) | Value::Int(_) | Value::Float(_) | Value::Bool(_)) => {
            conv::to_string_bytes(&v)
        }
        _ => {
            return Err(Thrown::type_error(format!(
                "{func}(): Argument #2 ($value) must be of type string|int|float|bool|null"
            )))
        }
    };
    Ok(match alter_ini_entry(host, &name, newv, func)? {
        Some(old) => Value::str(old),
        None => Value::Bool(false),
    })
}

fn fn_ini_set(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ini_set_impl(host, args, "ini_set")
}

fn fn_ini_alter(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ini_set_impl(host, args, "ini_alter")
}

fn fn_ini_restore(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, "ini_restore", "option")?;
    let key = String::from_utf8_lossy(&name).into_owned();
    if name == b"open_basedir" && STARTUP_DIAGS.with(|d| d.borrow().changed.contains(&key)) {
        // OnUpdateBaseDir runs at run time on the startup value: a loosening
        // fails, and the entry keeps its value.
        let startup = INI_STARTUP.with(|m| m.borrow().get(&key).cloned());
        if !crate::main::fopen_wrappers::on_update(&startup.unwrap_or_default()) {
            return Ok(Value::Null);
        }
    }
    // zend_restore_ini_entry: a directive ini_set changed is no longer
    // modified, and its handler runs again on the startup value.
    let restore = STARTUP_DIAGS.with(|d| {
        let mut d = d.borrow_mut();
        let pos = d.changed.iter().position(|c| *c == key)?;
        d.changed.remove(pos);
        Some(d.restore.get(&key).cloned())
    });
    let modified = restore.is_some();
    if modified {
        // ext/session's handlers run on the value restored, and may refuse.
        let v = INI_STARTUP
            .with(|m| m.borrow().get(&key).cloned())
            .or_else(|| ini_default_value(&name).flatten())
            .unwrap_or_default();
        use crate::ext::session::{on_modify, Stage};
        if let Some((false, _)) = on_modify(Some(host), "ini_restore", &name, &v, Stage::Runtime)? {
            return Ok(Value::Null);
        }
        crate::ext::iconv::ini_runtime_update(host, &name, "ini_restore")?;
    }
    for w in restore.flatten().unwrap_or_default() {
        host.emit_diagnostic(crate::E_WARNING, w.as_bytes())?;
    }
    // Back to the startup value (`-d`), else the default.
    let startup = INI_STARTUP.with(|m| m.borrow().get(&key).cloned());
    INI.with(|m| match startup {
        Some(v) => m.borrow_mut().insert(key, Some(v)),
        None => m.borrow_mut().remove(&key),
    });
    // The assert.* handlers deprecate on a restore at run time too.
    if modified {
        let v = ini_value(&name).flatten().unwrap_or_default();
        if let Some(msg) = crate::ext::standard::assert::ini_deprecation(&name, &v) {
            crate::diag::deprecated(host, "ini_restore", &msg)?;
        }
    }
    if let Some(v) = ini_value(&name) {
        if name == b"memory_limit" {
            // The handler on the startup value (already within
            // max_memory_limit). Below the memory in use, the current limit
            // stays.
            let v = v.as_deref().map_or(u64::MAX, |v| ini_parse_uquantity(v).0);
            rhp_values::mem::set_limit(v.min(usize::MAX as u64) as usize);
        }
        ini_apply(&name, v.as_deref());
    }
    Ok(Value::Null)
}

fn fn_ini_get_all(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ext = str_or_null_param(host, args, 0, "ini_get_all", "extension")?;
    let details = bool_param(host, args, 1, "ini_get_all", "details", true)?;
    let ext_lc = ext.as_ref().map(|e| e.to_ascii_lowercase());
    if let Some(e) = &ext_lc {
        if !module_loaded(&String::from_utf8_lossy(e)) {
            let shown = ext.as_deref().unwrap_or_default();
            let shown = &shown[..shown.iter().position(|&b| b == 0).unwrap_or(shown.len())];
            let msg = [b"Extension \"".as_slice(), shown, b"\" cannot be found"].concat();
            crate::warn(host, "ini_get_all", &msg)?;
            return Ok(Value::Bool(false));
        }
    }
    let mut entries: Vec<_> = INI_TABLE
        .iter()
        .filter(|e| module_loaded(e.0))
        .filter(|e| {
            ext_lc
                .as_ref()
                .is_none_or(|x| x.as_slice() == e.0.as_bytes())
        })
        .collect();
    entries.sort_by(|a, b| a.1.cmp(b.1));
    let mut out = PhpArray::new();
    for e in entries {
        let cur = ini_value(e.1.as_bytes()).flatten();
        let v = if details {
            let mut d = PhpArray::new();
            d.insert(
                ArrayKey::from_bytes(b"global_value"),
                ini_opt_value(ini_default_value(e.1.as_bytes()).flatten()),
            );
            d.insert(ArrayKey::from_bytes(b"local_value"), ini_opt_value(cur));
            d.insert(ArrayKey::from_bytes(b"access"), Value::Int(e.3 as i64));
            Value::Array(d)
        } else {
            ini_opt_value(cur)
        };
        out.insert(ArrayKey::from_bytes(e.1.as_bytes()), v);
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// error_log
// ---------------------------------------------------------------------------

fn fn_error_log(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let msg = str_param(host, args, 0, "error_log", "message")?;
    let ty = int_param(host, args, 1, "error_log", "message_type", 0)?;
    let dest = str_or_null_param(host, args, 2, "error_log", "destination")?;
    let _headers = str_or_null_param(host, args, 3, "error_log", "additional_headers")?;
    match ty {
        // Mail: no mailer configured.
        1 => Ok(Value::Bool(false)),
        3 => {
            let Some(path) = dest else {
                return Ok(Value::Bool(false));
            };
            if crate::main::streams::split_scheme(&path).0 == b"file"
                && !crate::main::fopen_wrappers::check_open(
                    host,
                    "error_log",
                    crate::main::streams::file_path(&path),
                    &path,
                )?
            {
                return Ok(Value::Bool(false));
            }
            let ok = crate::zend::virtual_cwd::append_file(&path, &msg, 0o666).is_ok();
            Ok(Value::Bool(ok))
        }
        // 0 / 4: no error_log ini → the CLI SAPI logger writes "msg\n" to stderr.
        _ => {
            let log_target = ini_value(b"error_log").flatten().filter(|v| !v.is_empty());
            match log_target {
                Some(path) if ty == 0 => {
                    // php_log_err: "[d-M-Y H:i:s e] message\n".
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    let mut line = b"[".to_vec();
                    line.extend_from_slice(&crate::ext::date::php_format_date(
                        b"d-M-Y H:i:s e",
                        now,
                        true,
                    ));
                    line.extend_from_slice(b"] ");
                    line.extend_from_slice(&msg);
                    line.push(b'\n');
                    let mode = crate::main::ini::error_log_mode();
                    let _ = crate::zend::virtual_cwd::append_file(&path, &line, mode);
                }
                _ => host.log_message(&msg),
            }
            Ok(Value::Bool(true))
        }
    }
}

fn fn_sys_getloadavg(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut l = [0f64; 3];
    if unsafe { libc::getloadavg(l.as_mut_ptr(), 3) } == -1 {
        return Ok(Value::Bool(false));
    }
    let mut a = PhpArray::new();
    for v in l {
        a.push(Value::Float(v));
    }
    Ok(Value::Array(a))
}

// --- getopt (main/getopt.c + ext/standard/basic_functions.c) -----------------

struct OptSpec {
    ch: u8,
    name: Option<Vec<u8>>,
    need_param: u8,
}

/// `parse_opts`: the leading run of alphanumeric option letters, each
/// optionally followed by `:` (required value) or `::` (optional value).
fn parse_short_opts(s: &[u8]) -> Vec<OptSpec> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() && s[i].is_ascii_alphanumeric() {
        let ch = s[i];
        i += 1;
        let mut need = 0;
        if s.get(i) == Some(&b':') {
            need = 1;
            i += 1;
            if s.get(i) == Some(&b':') {
                need = 2;
                i += 1;
            }
        }
        out.push(OptSpec {
            ch,
            name: None,
            need_param: need,
        });
    }
    out
}

/// One argv string as C sees it (cut at the first NUL), indexable past the end.
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

enum Got {
    Eof,
    Invalid,
    Opt(usize, Option<Vec<u8>>),
}

struct GetoptState {
    optchr: usize,
    dash: bool,
}

/// `php_getopt` for one option (show_err = 0, arg_start = 1).
fn php_getopt(argv: &[Vec<u8>], opts: &[OptSpec], optind: &mut usize, st: &mut GetoptState) -> Got {
    let argc = argv.len();
    if *optind >= argc {
        return Got::Eof;
    }
    let cur = |optind: usize| argv[optind].as_slice();
    if !st.dash {
        if at(cur(*optind), 0) != b'-' {
            return Got::Eof;
        }
        if at(cur(*optind), 1) == 0 {
            return Got::Eof;
        }
    }
    let mut idx: Option<usize> = None;
    let mut arg_start: usize;
    let a = cur(*optind);
    if at(a, 0) == b'-' && at(a, 1) == b'-' {
        if at(a, 2) == 0 {
            *optind += 1;
            return Got::Eof;
        }
        arg_start = 2;
        let mut arg_end = a.len() - 1;
        let search_len = arg_end - arg_start;
        if let Some(p) = a[arg_start..arg_start + search_len]
            .iter()
            .position(|&b| b == b'=')
        {
            arg_end = p;
            arg_start += 1;
        } else {
            arg_end -= 1;
        }
        let rest = &a[2..];
        // strncmp(argv + 2, name, arg_end) == 0 && arg_end == strlen(name)
        let found = opts.iter().position(|o| {
            o.name
                .as_deref()
                .is_some_and(|n| n.len() == arg_end && rest.starts_with(n))
        });
        match found {
            Some(i) => idx = Some(i),
            None => {
                *optind += 1;
                return Got::Invalid;
            }
        }
        st.optchr = 0;
        st.dash = false;
        arg_start += opts[idx.unwrap()].name.as_ref().unwrap().len();
    } else {
        if !st.dash {
            st.dash = true;
            st.optchr = 1;
        }
        if at(a, st.optchr) == b':' {
            st.dash = false;
            *optind += 1;
            return Got::Invalid;
        }
        arg_start = 1 + st.optchr;
    }
    let idx = match idx {
        Some(i) => i,
        None => {
            let c = at(a, st.optchr);
            match opts.iter().position(|o| o.name.is_none() && o.ch == c) {
                Some(i) => i,
                None => {
                    if at(a, st.optchr + 1) == 0 {
                        st.dash = false;
                        *optind += 1;
                    } else {
                        st.optchr += 1;
                    }
                    return Got::Invalid;
                }
            }
        }
    };
    let o = &opts[idx];
    if o.need_param > 0 {
        st.dash = false;
        let mut optarg = None;
        if at(a, arg_start) == 0 {
            *optind += 1;
            if *optind == argc {
                if o.need_param == 1 {
                    return Got::Invalid;
                }
            } else if o.need_param == 1 {
                optarg = Some(argv[*optind].clone());
                *optind += 1;
            }
        } else if at(a, arg_start) == b'=' {
            optarg = Some(a[arg_start + 1..].to_vec());
            *optind += 1;
        } else {
            optarg = Some(a[arg_start..].to_vec());
            *optind += 1;
        }
        Got::Opt(idx, optarg)
    } else {
        if arg_start >= 2 && !(at(a, 0) == b'-' && at(a, 1) == b'-') {
            if at(a, st.optchr + 1) == 0 {
                st.dash = false;
                *optind += 1;
            } else {
                st.optchr += 1;
            }
        } else {
            *optind += 1;
        }
        Got::Opt(idx, None)
    }
}

fn cstr(b: &[u8]) -> Vec<u8> {
    match b.iter().position(|&c| c == 0) {
        Some(p) => b[..p].to_vec(),
        None => b.to_vec(),
    }
}

/// `getopt(string $short_options, array $long_options = [], &$rest_index = null)`.
fn fn_getopt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let short = cstr(&str_param(host, args, 0, "getopt", "short_options")?);
    let longs = match args.get(1) {
        None => PhpArray::new(),
        Some(_) => crate::zend::zpp::array_param(host, args, 1, "getopt")?,
    };
    let set_rest = |v: i64| {
        if let Some(Value::Ref(r)) = args.get(2) {
            *r.borrow_mut() = Value::Int(v);
        }
    };
    set_rest(1);
    let Some(argv) = host.cli_argv() else {
        return Ok(Value::Bool(false));
    };
    let argv: Vec<Vec<u8>> = argv.iter().map(|a| cstr(a)).collect();
    let mut opts = parse_short_opts(&short);
    for (_, v) in longs.iter() {
        let mut name = cstr(&conv::to_string_bytes(&deref(v)));
        let mut need = 0;
        if name.last() == Some(&b':') {
            need = 1;
            name.pop();
            if name.last() == Some(&b':') {
                need = 2;
                name.pop();
            }
        }
        opts.push(OptSpec {
            ch: 0,
            name: Some(name),
            need_param: need,
        });
    }
    let mut out = PhpArray::new();
    let mut optind = 1usize;
    let mut st = GetoptState {
        optchr: 0,
        dash: false,
    };
    loop {
        match php_getopt(&argv, &opts, &mut optind, &mut st) {
            Got::Eof => break,
            Got::Invalid => continue,
            Got::Opt(i, optarg) => {
                let name = match &opts[i].name {
                    Some(n) => n.clone(),
                    None => vec![opts[i].ch],
                };
                let val = optarg.map(Value::str).unwrap_or(Value::Bool(false));
                // A numeric option name ("1") becomes an integer key.
                let key = ArrayKey::from_bytes(&name);
                match out.get_mut(&key) {
                    Some(existing) => {
                        if let Value::Array(a) = existing {
                            a.push(val);
                        } else {
                            let mut a = PhpArray::new();
                            a.push(existing.clone());
                            a.push(val);
                            *existing = Value::Array(a);
                        }
                    }
                    None => out.insert(key, val),
                }
            }
        }
    }
    set_rest(optind as i64);
    Ok(Value::Array(out))
}

fn fn_ip2long(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = crate::zend::zpp::path_param(host, args, 0, "ip2long", "ip")?;
    Ok(match crate::main::network::pton4(&s) {
        Some(a) if !s.is_empty() => Value::Int(u32::from_be_bytes(a) as i64),
        _ => Value::Bool(false),
    })
}

fn fn_long2ip(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = int_param(host, args, 0, "long2ip", "ip", 0)? as u64 as u32;
    let text = crate::main::network::ntop(&n.to_be_bytes()).expect("inet_ntop of an IPv4 address");
    Ok(Value::str(text))
}

fn fn_get_cfg_var(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, "get_cfg_var", "option")?;
    Ok(crate::main::ini::cfg_get(&name).unwrap_or(Value::Bool(false)))
}

fn fn_get_include_path(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(
        match crate::main::ini::ini_value(b"include_path").flatten() {
            Some(v) => Value::str(v),
            None => Value::Bool(false),
        },
    )
}

fn fn_set_include_path(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let p = str_param(host, args, 0, "set_include_path", "include_path")?;
    if p.is_empty() {
        return Ok(Value::Bool(false));
    }
    let old = crate::main::ini::ini_value(b"include_path").flatten();
    crate::main::ini::ini_store(b"include_path", p);
    Ok(old.map(Value::str).unwrap_or(Value::Bool(false)))
}

/// `get_current_user`: the owner of the main script file.
fn fn_get_current_user(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let Some(m) = script_stat(host) else {
        return Ok(Value::str(""));
    };
    let uid = std::os::unix::fs::MetadataExt::uid(&m);
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut res: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut res) };
    if rc != 0 || res.is_null() {
        return Ok(Value::str(""));
    }
    let name = unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) };
    Ok(Value::str(name.to_bytes().to_vec()))
}

// ---------------------------------------------------------------------------
// inet_pton / inet_ntop
// ---------------------------------------------------------------------------

/// `inet_pton(string $ip): string|false` (basic_functions.c): AF_INET6 when the
/// text has a ':', otherwise AF_INET if it has a '.'.
fn fn_inet_pton(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ip = crate::zend::zpp::path_param(host, args, 0, "inet_pton", "ip")?;
    let packed = if ip.contains(&b':') {
        network::pton6(&ip).map(|a| a.to_vec())
    } else if ip.contains(&b'.') {
        network::pton4(&ip).map(|a| a.to_vec())
    } else {
        None
    };
    Ok(packed.map_or(Value::Bool(false), Value::str))
}

/// `inet_ntop(string $ip): string|false`: a 4- or 16-byte address as text.
fn fn_inet_ntop(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let b = crate::zend::zpp::str_param(host, args, 0, "inet_ntop", "ip")?;
    Ok(network::ntop(&b).map_or(Value::Bool(false), Value::str))
}

fn fn_usleep(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let us = zpp_long(host, args, 0, "usleep", "microseconds")?;
    if us < 0 {
        return Err(Thrown::error(
            "ValueError",
            "usleep(): Argument #1 ($microseconds) must be greater than or equal to 0",
        ));
    }
    // php calls usleep(3) once: a signal cuts the sleep short.
    unsafe { libc::usleep(us as libc::useconds_t) };
    Ok(Value::Null)
}

fn fn_sleep(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = zpp_long(host, args, 0, "sleep", "seconds")?;
    if s < 0 {
        return Err(Thrown::error(
            "ValueError",
            "sleep(): Argument #1 ($seconds) must be greater than or equal to 0",
        ));
    }
    // php's sleep(3): returns the unslept seconds when a signal interrupts.
    Ok(Value::Int(unsafe { libc::sleep(s as libc::c_uint) } as i64))
}

/// `ini_parse_quantity($shorthand)`: zend_ini_parse_quantity, its
/// complaint as a plain warning.
fn fn_ini_parse_quantity(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = str_param(host, args, 0, "ini_parse_quantity", "shorthand")?;
    let n = crate::main::ini::ini_parse_quantity(&s);
    if let Some(e) = crate::main::ini::quantity_error(&s) {
        host.emit_diagnostic(crate::E_WARNING, e.as_bytes())?;
    }
    Ok(Value::Int(n))
}

/// `time_nanosleep($seconds, $nanoseconds)`: true, or the remainder when a
/// signal cut the sleep short.
fn fn_time_nanosleep(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let sec = zpp_long(host, args, 0, "time_nanosleep", "seconds")?;
    let nsec = zpp_long(host, args, 1, "time_nanosleep", "nanoseconds")?;
    for (n, v, p) in [(1, sec, "seconds"), (2, nsec, "nanoseconds")] {
        if v < 0 {
            return Err(Thrown::error(
                "ValueError",
                format!(
                    "time_nanosleep(): Argument #{n} (${p}) must be greater than or equal to 0"
                ),
            ));
        }
    }
    let req = libc::timespec {
        tv_sec: sec as libc::time_t,
        tv_nsec: nsec as _,
    };
    let mut rem = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: both point at live timespecs.
    if unsafe { libc::nanosleep(&req, &mut rem) } == 0 {
        return Ok(Value::Bool(true));
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::EINTR) => {
            let mut a = PhpArray::new();
            a.insert(ArrayKey::from_bytes(b"seconds"), Value::Int(rem.tv_sec));
            a.insert(
                ArrayKey::from_bytes(b"nanoseconds"),
                Value::Int(rem.tv_nsec),
            );
            Ok(Value::Array(a))
        }
        Some(libc::EINVAL) => Err(Thrown::error(
            "ValueError",
            "Nanoseconds was not in the range 0 to 999 999 999 or seconds was negative",
        )),
        _ => Ok(Value::Bool(false)),
    }
}

/// `time_sleep_until($timestamp)`: sleep until a time of day.
fn fn_time_sleep_until(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    const NS: u64 = 1_000_000_000;
    let target =
        crate::zend::zpp::zpp_double_or_null(host, args, 0, "time_sleep_until", "timestamp")?
            .unwrap_or(0.0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let top = (u64::MAX / NS) as f64;
    if !(target >= 0.0 && target <= top) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "time_sleep_until(): Argument #1 ($timestamp) must be between 0 and {}",
                u64::MAX / NS
            ),
        ));
    }
    let target_ns = (target * NS as f64) as u64;
    // gettimeofday's microseconds.
    let current_ns = now.as_secs() * NS + now.subsec_micros() as u64 * 1000;
    if target_ns < current_ns {
        crate::warn(
            host,
            "time_sleep_until",
            "Argument #1 ($timestamp) must be greater than or equal to the current time",
        )?;
        return Ok(Value::Bool(false));
    }
    let diff = target_ns - current_ns;
    let mut req = libc::timespec {
        tv_sec: (diff / NS) as libc::time_t,
        tv_nsec: (diff % NS) as _,
    };
    let mut rem = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: both point at live timespecs.
    while unsafe { libc::nanosleep(&req, &mut rem) } != 0 {
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            return Ok(Value::Bool(false));
        }
        req = rem;
    }
    Ok(Value::Bool(true))
}

/// `ignore_user_abort(?bool $enable = null)`: the old setting; the engine
/// never sees the client go away, so the setting has no other effect.
fn fn_ignore_user_abort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let old = crate::main::ini::ini_value(b"ignore_user_abort")
        .flatten()
        .map(|v| i64::from(crate::ini_parse_bool(&v)))
        .unwrap_or(0);
    match args.first().map(deref) {
        None | Some(Value::Null) => {}
        Some(_) => {
            let on = bool_param(host, args, 0, "ignore_user_abort", "enable", false)?;
            crate::main::ini::ini_store(
                b"ignore_user_abort",
                if on { b"1".to_vec() } else { b"0".to_vec() },
            );
        }
    }
    Ok(Value::Int(old))
}

/// `connection_status()` / `connection_aborted()`: always CONNECTION_NORMAL
/// (no abort detection yet).
fn fn_connection_status(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(0))
}

// ---------------------------------------------------------------------------
// builtins
// ---------------------------------------------------------------------------

/// A `Z_PARAM_PATH` argument: a string without NUL bytes.
fn path_param(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Vec<u8>, Thrown> {
    let p = crate::zend::zpp::str_param(host, args, i, fname, pname)?;
    if p.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{fname}(): Argument #{} (${pname}) must not contain any null bytes",
                i + 1
            ),
        ));
    }
    Ok(p)
}

fn fn_is_uploaded_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(host, args, 0, "is_uploaded_file", "filename")?;
    Ok(Value::Bool(
        host.body_state().is_some_and(|s| s.is_uploaded(&path)),
    ))
}

fn fn_move_uploaded_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let from = path_param(host, args, 0, "move_uploaded_file", "from")?;
    let to = path_param(host, args, 1, "move_uploaded_file", "to")?;
    if !host.body_state().is_some_and(|s| s.is_uploaded(&from)) {
        return Ok(Value::Bool(false));
    }
    if !crate::main::fopen_wrappers::check(host, "move_uploaded_file", &to, true)? {
        return Ok(Value::Bool(false));
    }
    let mut ok = false;
    if std::fs::rename(os_path(&from), os_path(&to)).is_ok() {
        ok = true;
        use std::os::unix::fs::PermissionsExt;
        let mode = 0o666 & !crate::zend::virtual_cwd::umask();
        if let Err(e) =
            std::fs::set_permissions(os_path(&to), std::fs::Permissions::from_mode(mode))
        {
            let msg = crate::main::streams::strerror(e.raw_os_error().unwrap_or(0));
            crate::warn(host, "move_uploaded_file", &msg)?;
        }
    } else {
        match std::fs::read(os_path(&from)) {
            Err(e) => {
                crate::main::streams::note_io_err(&e);
                crate::open_failed(host, "move_uploaded_file", &from)?;
            }
            Ok(data) => match crate::zend::virtual_cwd::write_file(&to, &data) {
                Ok(()) => {
                    let _ = std::fs::remove_file(os_path(&from));
                    ok = true;
                }
                Err(e) => {
                    crate::main::streams::note_io_err(&e);
                    crate::open_failed(host, "move_uploaded_file", &to)?;
                }
            },
        }
    }
    if ok {
        if let Some(s) = host.body_state() {
            s.forget(&from);
        }
    } else {
        crate::warn(
            host,
            "move_uploaded_file",
            &[
                b"Unable to move \"".as_slice(),
                &from,
                b"\" to \"",
                &to,
                b"\"",
            ]
            .concat(),
        )?;
    }
    Ok(Value::Bool(ok))
}

/// symtable key: canonical decimal ints become int keys.
fn sym_key(k: &[u8]) -> ArrayKey {
    ArrayKey::from_bytes(k)
}

fn apply(arr: &mut PhpArray, ev: Event) {
    match ev {
        Event::Section(_) => {}
        Event::Entry(_, None) => {}
        Event::Entry(k, Some(v)) => {
            arr.insert(sym_key(&k.into_bytes()), v.into_value());
        }
        Event::PopEntry(k, v, off) => {
            let k = k.into_bytes();
            let key = if !(k.len() > 1 && k[0] == b'0') && is_long_string(&k) {
                ArrayKey::Int(strtoul(&k))
            } else {
                ArrayKey::Str(rhp_values::PhpStr::from_bytes(k))
            };
            if !matches!(arr.get(&key), Some(Value::Array(_))) {
                arr.insert(key.clone(), Value::Array(PhpArray::new()));
            }
            let Some(Value::Array(sub)) = arr.get_mut(&key) else {
                return;
            };
            let off = off.into_bytes();
            if off.is_empty() {
                sub.push(v.into_value());
            } else {
                sub.insert(sym_key(&off), v.into_value());
            }
        }
    }
}

/// `is_numeric_string(...) == IS_LONG` (whole string, int-typed, no overflow).
fn is_long_string(s: &[u8]) -> bool {
    matches!(conv::numeric_string(s), Some(conv::Num::Int(_)))
}

/// `ZEND_STRTOUL(s, NULL, 0)` (base auto-detect) as php's zend_ulong → key.
fn strtoul(s: &[u8]) -> i64 {
    let t = trim_ws(s);
    let (neg, t) = match t.first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let (base, digits) = if t.len() > 1 && t[0] == b'0' && (t[1] == b'x' || t[1] == b'X') {
        (16, &t[2..])
    } else if t.len() > 1 && t[0] == b'0' {
        (8, &t[1..])
    } else {
        (10, t)
    };
    let mut n: u64 = 0;
    for &c in digits {
        let d = match c {
            b'0'..=b'9' => (c - b'0') as u64,
            b'a'..=b'f' => (c - b'a' + 10) as u64,
            b'A'..=b'F' => (c - b'A' + 10) as u64,
            _ => break,
        };
        if d >= base {
            break;
        }
        n = n.wrapping_mul(base).wrapping_add(d);
    }
    (if neg { n.wrapping_neg() } else { n }) as i64
}

/// php's ini_error: the syntax error as an E_WARNING naming the file
/// ("Unknown" for a string).
fn report_syntax_error(
    host: &mut dyn Host,
    src: &[u8],
    mode: i64,
    file: &[u8],
) -> Result<bool, Thrown> {
    let Some((msg, line)) = syntax_error(src, mode) else {
        return Ok(false);
    };
    let mut m = String::from_utf8_lossy(&msg).into_owned();
    m.push_str(&format!(
        " in {} on line {line}\n",
        String::from_utf8_lossy(file)
    ));
    host.emit_diagnostic(crate::E_WARNING, m.as_bytes())?;
    Ok(true)
}

/// Parse ini text; `None` = syntax error (php returns false).
pub(crate) fn parse_ini(
    host: &dyn Host,
    src: &[u8],
    sections: bool,
    mode: i64,
) -> Option<PhpArray> {
    let mut lx = lexer(src, mode);
    let first = lx.next();
    let mut p = Parser {
        lx,
        tok: first,
        host,
    };
    let mut events = Vec::new();
    p.statement_list(&mut events).ok()?;
    let mut out = PhpArray::new();
    if !sections {
        for ev in events {
            apply(&mut out, ev);
        }
        return Some(out);
    }
    // php_ini_parser_cb_with_sections
    let mut active: Option<ArrayKey> = None;
    for ev in events {
        match ev {
            Event::Section(name) => {
                let k = sym_key(&name.into_bytes());
                out.insert(k.clone(), Value::Array(PhpArray::new()));
                active = Some(k);
            }
            ev => {
                let has_value = !matches!(ev, Event::Entry(_, None));
                if !has_value {
                    continue;
                }
                match &active {
                    Some(k) => {
                        if let Some(Value::Array(sec)) = out.get_mut(k) {
                            apply(sec, ev);
                        }
                    }
                    None => apply(&mut out, ev),
                }
            }
        }
    }
    Some(out)
}

fn mode_ok(mode: i64) -> bool {
    matches!(mode, MODE_NORMAL | MODE_RAW | MODE_TYPED)
}

fn fn_parse_ini_string(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = str_param(host, args, 0, "parse_ini_string", "ini_string")?;
    let sections = bool_param(host, args, 1, "parse_ini_string", "process_sections", false)?;
    let mode = int_param(
        host,
        args,
        2,
        "parse_ini_string",
        "scanner_mode",
        MODE_NORMAL,
    )?;
    if !mode_ok(mode) {
        host.raise_diagnostic(crate::E_WARNING, b"Invalid scanner mode");
        return Ok(Value::Bool(false));
    }
    if report_syntax_error(host, &s, mode, b"Unknown")? {
        return Ok(Value::Bool(false));
    }
    Ok(match parse_ini(host, &s, sections, mode) {
        Some(a) => Value::Array(a),
        None => Value::Bool(false),
    })
}

fn fn_parse_ini_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = str_param(host, args, 0, "parse_ini_file", "filename")?;
    if path.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "parse_ini_file(): Argument #1 ($filename) must not be empty",
        ));
    }
    if path.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            "parse_ini_file(): Argument #1 ($filename) must not contain any null bytes",
        ));
    }
    let sections = bool_param(host, args, 1, "parse_ini_file", "process_sections", false)?;
    let mode = int_param(host, args, 2, "parse_ini_file", "scanner_mode", MODE_NORMAL)?;
    // zend_stream_open: USE_PATH | REPORT_ERRORS
    crate::main::streams::clear_errno();
    let target = crate::ext::standard::file::include_path_target(host, &path);
    if crate::main::streams::split_scheme(&target).0 == b"file"
        && !crate::main::fopen_wrappers::check_open(
            host,
            "parse_ini_file",
            crate::main::streams::file_path(&target),
            &target,
        )?
    {
        return Ok(Value::Bool(false));
    }
    let Some(src) = host.stream_get_contents(&target) else {
        crate::open_failed(host, "parse_ini_file", &path)?;
        return Ok(Value::Bool(false));
    };
    if !mode_ok(mode) {
        host.raise_diagnostic(crate::E_WARNING, b"Invalid scanner mode");
        return Ok(Value::Bool(false));
    }
    if report_syntax_error(host, &src, mode, &path)? {
        return Ok(Value::Bool(false));
    }
    Ok(match parse_ini(host, &src, sections, mode) {
        Some(a) => Value::Array(a),
        None => Value::Bool(false),
    })
}

/// The source behind `path` for highlighting / stripping
/// (php_stream_open_for_zend: include_path, open errors reported).
fn read_source(host: &mut dyn Host, fname: &str, path: &[u8]) -> Result<Option<Vec<u8>>, Thrown> {
    if path.is_empty() {
        return Err(Thrown::error("ValueError", "Path must not be empty"));
    }
    let target = ext::standard::file::include_path_target(host, path);
    ext::standard::md5::read_file_for_hash(host, fname, &target)
}

fn fn_highlight_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    highlight_file(host, args, "highlight_file")
}

fn fn_show_source(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    highlight_file(host, args, "show_source")
}

/// highlight_file() and its alias show_source().
fn highlight_file(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let path = zend::zpp::path_param(host, args, 0, fname, "filename")?;
    let ret = args.get(1).is_some_and(|v| conv::to_bool(&deref(v)));
    if !crate::main::fopen_wrappers::check(host, fname, &path, true)? {
        return Ok(Value::Bool(false));
    }
    let failed = |host: &mut dyn Host| {
        let mut m = b"Failed opening '".to_vec();
        m.extend_from_slice(&path);
        m.extend_from_slice(b"' for highlighting");
        crate::warn(host, fname, &m)
    };
    if path.is_empty() {
        failed(host)?;
    }
    let Some(src) = read_source(host, fname, &path)? else {
        failed(host)?;
        return Ok(Value::Bool(false));
    };
    let html = zend::highlight::highlight(
        zend::highlight::skip_shebang(&src),
        &zend::highlight::Colors::from_ini(),
    );
    if ret {
        return Ok(Value::str(html));
    }
    host.write_out(&html);
    Ok(Value::Bool(true))
}

fn fn_highlight_string(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let src = conv::to_string_bytes(&deref(&args[0]));
    let ret = args.get(1).is_some_and(|v| conv::to_bool(&deref(v)));
    let html = zend::highlight::highlight(&src, &zend::highlight::Colors::from_ini());
    if ret {
        return Ok(Value::str(html));
    }
    host.write_out(&html);
    Ok(Value::Bool(true))
}

fn fn_php_strip_whitespace(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "php_strip_whitespace";
    let path = zend::zpp::path_param(host, args, 0, fname, "filename")?;
    let Some(src) = read_source(host, fname, &path)? else {
        return Ok(Value::str(""));
    };
    Ok(Value::str(zend::highlight::strip(
        zend::highlight::skip_shebang(&src),
    )))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("highlight_file", fn_highlight_file),
            ("register_tick_function", fn_register_tick_function),
            ("unregister_tick_function", fn_unregister_tick_function),
            ("show_source", fn_show_source),
            ("highlight_string", fn_highlight_string),
            ("php_strip_whitespace", fn_php_strip_whitespace),
            ("print_r", fn_print_r),
            ("constant", fn_constant),
            ("call_user_func", fn_call_user_func),
            ("forward_static_call", fn_forward_static_call),
            ("getservbyname", fn_getservbyname),
            ("time_nanosleep", fn_time_nanosleep),
            ("ini_parse_quantity", fn_ini_parse_quantity),
            ("time_sleep_until", fn_time_sleep_until),
            ("getservbyport", fn_getservbyport),
            ("getprotobyname", fn_getprotobyname),
            ("getprotobynumber", fn_getprotobynumber),
            ("forward_static_call_array", fn_forward_static_call_array),
            ("call_user_func_array", fn_call_user_func_array),
            ("flush", fn_flush),
            ("getenv", fn_getenv),
            ("putenv", fn_putenv),
            ("error_get_last", fn_error_get_last),
            ("error_clear_last", fn_error_clear_last),
            ("register_shutdown_function", fn_register_shutdown_function),
            ("ini_get", fn_ini_get),
            ("ini_set", fn_ini_set),
            ("ini_alter", fn_ini_alter),
            ("ini_restore", fn_ini_restore),
            ("ini_get_all", fn_ini_get_all),
            ("error_log", fn_error_log),
            ("sys_getloadavg", fn_sys_getloadavg),
            ("getopt", fn_getopt),
            ("ip2long", fn_ip2long),
            ("long2ip", fn_long2ip),
            ("get_cfg_var", fn_get_cfg_var),
            ("get_include_path", fn_get_include_path),
            ("set_include_path", fn_set_include_path),
            ("get_current_user", fn_get_current_user),
            ("inet_pton", fn_inet_pton),
            ("inet_ntop", fn_inet_ntop),
            ("usleep", fn_usleep),
            ("sleep", fn_sleep),
            ("ignore_user_abort", fn_ignore_user_abort),
            ("connection_status", fn_connection_status),
            ("connection_aborted", fn_connection_status),
            ("is_uploaded_file", fn_is_uploaded_file),
            ("move_uploaded_file", fn_move_uploaded_file),
            ("parse_ini_string", fn_parse_ini_string),
            ("parse_ini_file", fn_parse_ini_file),
        ],
    );
}
