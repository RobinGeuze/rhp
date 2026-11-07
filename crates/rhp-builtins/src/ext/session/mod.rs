//! ext/session (session.c, mod_user.c, mod_user_class.c): the session_*
//! functions, the session.* ini handlers, the `files` and `user` save
//! handlers, the php / php_binary / php_serialize serializers, the session
//! cookie and cache limiter headers, and trans-sid through the URL
//! rewriter (`url_scanner_ex`).
//!
//! The module globals (`PS()`) are request state ([`reset_state`]). They are
//! set up on first use, as RINIT would have set them (the ini handlers set
//! them up before they change anything), so a request that never touches
//! a session pays nothing. session.auto_start starts the session from
//! [`rinit`]; [`rshutdown`] writes and closes it.

mod mod_files;

use std::cell::RefCell;
use std::rc::Rc;

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, PhpRef, Value};

use crate::main::ini::{alter_ini_entry_ex, c_atol, ini_parse_bool, ini_parse_quantity};
use crate::{Host, Thrown, E_DEPRECATED, E_ERROR, E_NOTICE, E_WARNING};

pub const PRELUDE_PHP: &str = include_str!("prelude.php");

/// PS_MAX_SID_LENGTH.
const MAX_SID_LENGTH: usize = 256;

const DISABLED: i64 = 0;
const NONE: i64 = 1;
const ACTIVE: i64 = 2;

const FORBIDDEN_CHARS: &[u8] = b"=,;.[ \t\r\n\x0b\x0c";

/// A save handler (`ps_module`).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Module {
    Files,
    User,
}

impl Module {
    fn name(self) -> &'static str {
        match self {
            Module::Files => "files",
            Module::User => "user",
        }
    }
}

/// `_php_find_ps_module` (strcasecmp of a C string).
fn find_module(name: &[u8]) -> Option<Module> {
    let name = c_str(name);
    [Module::Files, Module::User]
        .into_iter()
        .find(|m| m.name().as_bytes().eq_ignore_ascii_case(name))
}

/// A serializer (`ps_serializer`).
#[derive(Clone, Copy, PartialEq)]
enum Serializer {
    PhpSerialize,
    Php,
    PhpBinary,
}

/// `_php_find_ps_serializer`.
fn find_serializer(name: &[u8]) -> Option<Serializer> {
    let name = c_str(name);
    [
        (Serializer::PhpSerialize, "php_serialize"),
        (Serializer::Php, "php"),
        (Serializer::PhpBinary, "php_binary"),
    ]
    .into_iter()
    .find(|(_, n)| n.as_bytes().eq_ignore_ascii_case(name))
    .map(|(s, _)| s)
}

fn c_str(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
}

/// `PS(mod_user_names)`: the user handler's callables.
#[derive(Default, Clone)]
struct UserNames {
    open: Option<Value>,
    close: Option<Value>,
    read: Option<Value>,
    write: Option<Value>,
    destroy: Option<Value>,
    gc: Option<Value>,
    create_sid: Option<Value>,
    validate_sid: Option<Value>,
    update_timestamp: Option<Value>,
}

/// `php_ps_globals`, the part rhp keeps (the settings are read from the
/// ini entries).
struct Ps {
    /// RINIT has run (see the module doc).
    init: bool,
    status: i64,
    id: Option<Vec<u8>>,
    module: Option<Module>,
    default_mod: Option<Module>,
    serializer: Option<Serializer>,
    /// `PS(mod_data)` of the files handler.
    mod_data: Option<mod_files::Files>,
    mod_user_implemented: bool,
    mod_user_is_open: bool,
    mod_user_class_name: Option<Vec<u8>>,
    user: UserNames,
    in_save_handler: bool,
    set_handler: bool,
    define_sid: bool,
    send_cookie: bool,
    /// The data read, for lazy_write.
    session_vars: Option<Vec<u8>>,
    /// `PS(http_session_vars)`: the reference `$_SESSION` is.
    vars: Option<PhpRef>,
    started_at: Option<(Vec<u8>, u32)>,
    /// RSHUTDOWN is running: no user code is on the stack.
    in_shutdown: bool,
}

impl Default for Ps {
    fn default() -> Self {
        Ps {
            init: false,
            status: NONE,
            id: None,
            module: None,
            default_mod: None,
            serializer: None,
            mod_data: None,
            mod_user_implemented: false,
            mod_user_is_open: false,
            mod_user_class_name: None,
            user: UserNames::default(),
            in_save_handler: false,
            set_handler: false,
            define_sid: true,
            send_cookie: false,
            session_vars: None,
            vars: None,
            started_at: None,
            in_shutdown: false,
        }
    }
}

/// A session call in progress: the origin its diagnostics name (None: the
/// running builtin) and the exception it holds (php's EG(exception) while
/// the C code goes on).
struct Frame {
    origin: Option<String>,
    exception: Option<Thrown>,
}

thread_local! {
    static PS: RefCell<Ps> = RefCell::new(Ps::default());
    static FRAMES: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
}

/// Back to a fresh request.
pub(crate) fn reset_state() {
    // Values dropped outside the borrow (a destructor may look at us).
    let old = PS.with(|p| std::mem::take(&mut *p.borrow_mut()));
    drop(old);
    FRAMES.with(|f| f.borrow_mut().clear());
}

fn ps<T>(f: impl FnOnce(&mut Ps) -> T) -> T {
    PS.with(|p| f(&mut p.borrow_mut()))
}

/// php_rinit_session (without auto_start): the save handler and the
/// serializer from the ini entries.
fn ensure_init() {
    if ps(|p| p.init) {
        return;
    }
    let module = find_module(&ini(b"session.save_handler"));
    let serializer = find_serializer(&ini(b"session.serialize_handler"));
    ps(|p| {
        p.init = true;
        p.module = module;
        p.serializer = serializer;
        if module.is_none() || serializer.is_none() {
            p.status = DISABLED;
        }
    });
}

fn ini(name: &[u8]) -> Vec<u8> {
    crate::ini_value(name).flatten().unwrap_or_default()
}

fn ini_bool(name: &[u8]) -> bool {
    ini_parse_bool(&ini(name))
}

fn ini_long(name: &[u8]) -> i64 {
    ini_parse_quantity(&ini(name))
}

fn use_cookies() -> bool {
    ini_bool(b"session.use_cookies")
}

fn use_only_cookies() -> bool {
    ini_bool(b"session.use_only_cookies")
}

fn session_name() -> Vec<u8> {
    ini(b"session.name")
}

fn save_path() -> Vec<u8> {
    ini(b"session.save_path")
}

fn gc_maxlifetime() -> i64 {
    ini_long(b"session.gc_maxlifetime")
}

fn sid_length() -> usize {
    c_atol(&ini(b"session.sid_length")).clamp(22, MAX_SID_LENGTH as i64) as usize
}

fn sid_bits() -> u32 {
    c_atol(&ini(b"session.sid_bits_per_character")).clamp(4, 6) as u32
}

// ---------------------------------------------------------------------------
// Frames, diagnostics, exceptions
// ---------------------------------------------------------------------------

/// Run `f` as a session call from `origin` (None: the running builtin).
/// An exception the call recorded is thrown when it returns.
fn session_call<T>(
    host: &mut dyn Host,
    origin: Option<String>,
    f: impl FnOnce(&mut dyn Host) -> Result<T, Thrown>,
) -> Result<T, Thrown> {
    FRAMES.with(|fr| {
        fr.borrow_mut().push(Frame {
            origin,
            exception: None,
        })
    });
    let r = f(host);
    let exc = FRAMES.with(|fr| fr.borrow_mut().pop().and_then(|f| f.exception));
    match (r, exc) {
        (Err(t), Some(e)) if t.is_catchable() => Err(host.chain_thrown(t, e)),
        (Err(t), _) => Err(t),
        (Ok(_), Some(e)) => Err(e),
        (Ok(v), None) => Ok(v),
    }
}

/// The name php_error_docref prefixes for the current session call.
fn origin(host: &dyn Host) -> String {
    let o = FRAMES.with(|fr| fr.borrow().last().and_then(|f| f.origin.clone()));
    o.unwrap_or_else(|| format!("{}()", host.active_function_name()))
}

/// php_error_docref from `origin`.
fn docref(host: &mut dyn Host, origin: &str, level: i64, msg: &str) -> Result<(), Thrown> {
    let m = format!("{origin}: {msg}");
    host.emit_diagnostic(level, m.as_bytes())
}

fn warn(host: &mut dyn Host, msg: &str) -> Result<(), Thrown> {
    let o = origin(host);
    docref(host, &o, E_WARNING, msg)
}

/// EG(exception): the current call holds one.
fn has_exception(host: &dyn Host) -> bool {
    host.has_pending_throw()
        || FRAMES.with(|fr| fr.borrow().last().is_some_and(|f| f.exception.is_some()))
}

/// zend_throw_error and friends: the exception the C code goes on with.
fn throw(host: &mut dyn Host, t: Thrown) {
    let prev = FRAMES.with(|fr| fr.borrow_mut().last_mut().and_then(|f| f.exception.take()));
    let t = match prev {
        Some(p) => host.chain_thrown(t, p),
        None => t,
    };
    let placed = FRAMES.with(|fr| match fr.borrow_mut().last_mut() {
        Some(f) => {
            f.exception = Some(t);
            None
        }
        None => Some(t),
    });
    if let Some(t) = placed {
        host.set_pending_throw(t);
    }
}

/// php_session_headers_already_sent_error.
fn headers_sent_error(host: &mut dyn Host, level: i64, msg: &str) -> Result<(), Thrown> {
    let start = host
        .response()
        .and_then(|r| r.output_start.clone())
        .filter(|(f, _)| !f.is_empty());
    let msg = match start {
        Some((f, l)) => format!(
            "{msg} (sent from {} on line {l})",
            String::from_utf8_lossy(&f)
        ),
        None => msg.to_string(),
    };
    let o = origin(host);
    docref(host, &o, level, &msg)
}

/// php_session_session_already_started_error.
fn already_started_error(host: &mut dyn Host, level: i64, msg: &str) -> Result<(), Thrown> {
    let msg = match ps(|p| p.started_at.clone()) {
        Some((f, l)) => format!(
            "{msg} (started from {} on line {l})",
            String::from_utf8_lossy(&f)
        ),
        None if ini_bool(b"session.auto_start") => format!("{msg} (session started automatically)"),
        None => msg.to_string(),
    };
    let o = origin(host);
    docref(host, &o, level, &msg)
}

fn headers_sent(host: &mut dyn Host) -> bool {
    host.response().is_some_and(|r| r.sent && !r.no_headers)
}

// ---------------------------------------------------------------------------
// $_SESSION
// ---------------------------------------------------------------------------

/// IF_SESSION_VARS: `$_SESSION`'s reference, when it holds an array.
fn session_vars() -> Option<PhpRef> {
    ps(|p| p.vars.clone()).filter(|c| matches!(&*c.borrow(), Value::Array(_)))
}

/// php_session_track_init: a fresh `$_SESSION = []` reference.
fn track_init(host: &mut dyn Host) {
    let cell: PhpRef = Rc::new(RefCell::new(Value::Array(PhpArray::new())));
    let old = ps(|p| p.vars.replace(cell.clone()));
    host.set_global_ref(b"_SESSION", cell, true);
    drop(old);
}

/// php_set_session_var.
fn set_session_var(name: &[u8], v: Value) {
    if let Some(c) = session_vars() {
        if let Value::Array(a) = &mut *c.borrow_mut() {
            a.insert(ArrayKey::from_bytes(name), v);
        }
    }
}

/// zend_hash_clean of `$_SESSION`.
fn clean_session_vars() {
    if let Some(c) = session_vars() {
        let old = std::mem::replace(&mut *c.borrow_mut(), Value::Array(PhpArray::new()));
        drop(old);
    }
}

// ---------------------------------------------------------------------------
// Serializers
// ---------------------------------------------------------------------------

/// PS_ENCODE_LOOP: each string-keyed session variable (looked up again, in
/// case user code changed `$_SESSION` meanwhile).
fn encode_loop(
    host: &mut dyn Host,
    mut code: impl FnMut(&mut dyn Host, &[u8], &Value) -> Result<bool, Thrown>,
) -> Result<(), Thrown> {
    let Some(cell) = ps(|p| p.vars.clone()) else {
        return Ok(());
    };
    let Value::Array(copy) = cell.borrow().clone() else {
        return Ok(());
    };
    for (k, _) in copy.iter() {
        let key = match k {
            ArrayKey::Int(n) => {
                warn(host, &format!("Skipping numeric key {n}"))?;
                continue;
            }
            ArrayKey::Str(s) => s.as_bytes().to_vec(),
        };
        let cur = match &*cell.borrow() {
            Value::Array(a) => a.get(k).cloned(),
            _ => None,
        };
        if let Some(v) = cur {
            if !code(host, &key, &v)? {
                break;
            }
        }
    }
    Ok(())
}

/// The serializer's encode: `None` is NULL (nothing written).
fn encode_with(host: &mut dyn Host, s: Serializer) -> Result<Option<Vec<u8>>, Thrown> {
    let mut buf = Vec::new();
    match s {
        Serializer::PhpSerialize => {
            if let Some(c) = session_vars() {
                let v = c.borrow().clone();
                let mut ser = crate::ext::standard::var::SessionSer::begin();
                let r = ser.value(host, &mut buf, &v);
                ser.end();
                r?;
            }
            Ok(Some(buf))
        }
        Serializer::PhpBinary => {
            let mut ser = crate::ext::standard::var::SessionSer::begin();
            let r = encode_loop(host, |host, key, v| {
                if key.len() > 127 {
                    return Ok(true);
                }
                buf.push(key.len() as u8);
                buf.extend_from_slice(key);
                ser.value(host, &mut buf, v)?;
                Ok(true)
            });
            ser.end();
            r?;
            // An empty smart_str is NULL.
            Ok((!buf.is_empty()).then_some(buf))
        }
        Serializer::Php => {
            let mut ser = crate::ext::standard::var::SessionSer::begin();
            let mut fail = false;
            let r = encode_loop(host, |host, key, v| {
                buf.extend_from_slice(key);
                if key.contains(&b'|') {
                    fail = true;
                    let msg = format!(
                        "Failed to write session data. Data contains invalid key \"{}\"",
                        String::from_utf8_lossy(c_str(key))
                    );
                    warn(host, &msg)?;
                    return Ok(false);
                }
                buf.push(b'|');
                ser.value(host, &mut buf, v)?;
                Ok(true)
            });
            ser.end();
            r?;
            Ok((!fail && !buf.is_empty()).then_some(buf))
        }
    }
}

/// The serializer's decode: whether it succeeded.
fn decode_with(host: &mut dyn Host, s: Serializer, val: &[u8]) -> Result<bool, Thrown> {
    use crate::ext::standard::var::SessionUnser;
    let fname = origin(host);
    let fname = fname.strip_suffix("()").unwrap_or(&fname).to_string();
    let mut u = SessionUnser::begin();
    // The session variables parsed, by name, and whether all parsed.
    let mut names: Vec<Vec<u8>> = Vec::new();
    let mut p = 0usize;
    let mut ok = true;
    // php_set_session_var as each value is parsed (what it replaces goes
    // now); the final values once all are parsed.
    let parse = |host: &mut dyn Host,
                 u: &mut SessionUnser,
                 at: usize,
                 name: Option<&[u8]>|
     -> Result<Option<usize>, Thrown> {
        let (good, end, cell) = u.value(host, val, at, &fname)?;
        if let (Some(name), Some(cell)) = (name, cell) {
            set_session_var(name, Value::Ref(cell));
        }
        Ok(good.then_some(end))
    };
    let r: Result<(), Thrown> = (|| {
        match s {
            Serializer::PhpSerialize => {
                ok = parse(host, &mut u, 0, None)?.is_some();
            }
            Serializer::PhpBinary => {
                while p < val.len() {
                    let namelen = (val[p] & 0x7f) as usize;
                    if p + namelen >= val.len() {
                        ok = false;
                        break;
                    }
                    let name = val[p + 1..p + 1 + namelen].to_vec();
                    p += namelen + 1;
                    match parse(host, &mut u, p, Some(&name))? {
                        Some(end) => {
                            names.push(name);
                            p = end;
                        }
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
            }
            Serializer::Php => {
                while p < val.len() {
                    let Some(bar) = val[p..].iter().position(|&c| c == b'|') else {
                        ok = false;
                        break;
                    };
                    let name = val[p..p + bar].to_vec();
                    match parse(host, &mut u, p + bar + 1, Some(&name))? {
                        Some(end) => {
                            names.push(name);
                            p = end;
                        }
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    })();
    let values = u.take_values();
    if let Err(t) = r {
        u.end(host, true)?;
        return Err(t);
    }
    if s == Serializer::PhpSerialize {
        let v = match values.into_iter().next() {
            Some(v) if ok && !matches!(deref(&v), Value::Null) => deref(&v),
            _ => Value::Array(PhpArray::new()),
        };
        let cell: PhpRef = Rc::new(RefCell::new(v));
        let old = ps(|p| p.vars.replace(cell.clone()));
        host.set_global_ref(b"_SESSION", cell, false);
        drop(old);
        u.end(host, false)?;
        return Ok(ok || val.is_empty());
    }
    for (name, v) in names.iter().zip(values) {
        set_session_var(name, v);
    }
    u.end(host, false)?;
    Ok(ok)
}

/// php_session_encode.
fn session_encode(host: &mut dyn Host) -> Result<Option<Vec<u8>>, Thrown> {
    if session_vars().is_none() {
        warn(host, "Cannot encode non-existent session")?;
        return Ok(None);
    }
    let Some(s) = ps(|p| p.serializer) else {
        return Ok(None);
    };
    encode_with(host, s)
}

/// php_session_cancel_decode.
fn cancel_decode(host: &mut dyn Host) -> Result<(), Thrown> {
    session_destroy(host)?;
    track_init(host);
    warn(
        host,
        "Failed to decode session object. Session has been destroyed",
    )
}

/// php_session_decode: whether it succeeded.
fn session_decode(host: &mut dyn Host, data: &[u8]) -> Result<bool, Thrown> {
    let Some(s) = ps(|p| p.serializer) else {
        return Ok(false);
    };
    match decode_with(host, s, data) {
        Ok(true) => Ok(true),
        Ok(false) => {
            cancel_decode(host)?;
            Ok(false)
        }
        Err(t) if t.is_catchable() => {
            throw(host, t);
            cancel_decode(host)?;
            Ok(false)
        }
        Err(t) => {
            let _ = cancel_decode(host);
            Err(t)
        }
    }
}

// ---------------------------------------------------------------------------
// Session ids
// ---------------------------------------------------------------------------

const HEXCONVTAB: &[u8; 64] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ,-";

/// php_session_create_id: sid_length characters of sid_bits_per_character
/// random bits each. `None` when no randomness (the exception is held).
fn create_id(host: &mut dyn Host) -> Result<Option<Vec<u8>>, Thrown> {
    let len = sid_length();
    let nbits = sid_bits();
    let mut rbuf = vec![0u8; len];
    if let Err(t) = crate::ext::random::engine::csprng(&mut rbuf) {
        throw(host, t);
        return Ok(None);
    }
    let mut out = Vec::with_capacity(len);
    let (mut w, mut have) = (0u32, 0u32);
    let mask = (1u32 << nbits) - 1;
    let mut p = rbuf.iter();
    while out.len() < len {
        if have < nbits {
            match p.next() {
                Some(&b) => {
                    w |= (b as u32) << have;
                    have += 8;
                }
                None => break,
            }
        }
        out.push(HEXCONVTAB[(w & mask) as usize]);
        w >>= nbits;
        have -= nbits;
    }
    Ok(Some(out))
}

// ---------------------------------------------------------------------------
// Save handlers
// ---------------------------------------------------------------------------

/// ps_call_handler: `None` is an undefined result (no call, or it threw).
fn call_handler(
    host: &mut dyn Host,
    pick: fn(&UserNames) -> Option<Value>,
    args: Vec<Value>,
) -> Result<Option<Value>, Thrown> {
    if ps(|p| p.in_save_handler) {
        ps(|p| p.in_save_handler = false);
        warn(
            host,
            "Cannot call session save handler in a recursive manner",
        )?;
        return Ok(None);
    }
    let Some(f) = ps(|p| pick(&p.user)) else {
        return Ok(None);
    };
    if has_exception(host) {
        // zend_call_function does not call with an exception pending.
        return Ok(Some(Value::Null));
    }
    ps(|p| p.in_save_handler = true);
    let r = host.call_callable(&f, &args);
    ps(|p| p.in_save_handler = false);
    match r {
        Ok(v) => Ok(Some(deref(&v))),
        // At RSHUTDOWN no frame can catch: zend_call_function reports the
        // exception as uncaught and bails out, with the handler still
        // marked as running.
        Err(t) if t.is_catchable() && ps(|p| p.in_shutdown) => {
            ps(|p| p.in_save_handler = true);
            host.exception_error(t, E_ERROR);
            Err(Thrown::Exit(255))
        }
        // An exception, or an exit (php's unwind_exit): held while the C
        // code finishes.
        Err(t) if t.is_catchable() || matches!(t, Thrown::Exit(_)) => {
            throw(host, t);
            Ok(None)
        }
        Err(t) => Err(t),
    }
}

/// verify_bool_return_type_userland_calls: SUCCESS?
fn verify_bool(host: &mut dyn Host, v: &Option<Value>) -> Result<bool, Thrown> {
    let Some(v) = v else {
        return Ok(false);
    };
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Int(n @ (-1 | 0)) => {
            if !has_exception(host) {
                let o = origin(host);
                docref(
                    host,
                    &o,
                    E_DEPRECATED,
                    "Session callback must have a return value of type bool, int returned",
                )?;
            }
            Ok(*n == 0)
        }
        other => {
            if !has_exception(host) {
                let msg = format!(
                    "Session callback must have a return value of type bool, {} returned",
                    crate::zend::zpp::zval_value_name(host, other)
                );
                throw(host, Thrown::type_error(msg));
            }
            Ok(false)
        }
    }
}

/// Run `f` on the files handler's data (taken out while it runs).
fn with_files<T>(
    host: &mut dyn Host,
    default: T,
    f: impl FnOnce(&mut dyn Host, &mut mod_files::Files) -> Result<T, Thrown>,
) -> Result<T, Thrown> {
    let Some(mut data) = ps(|p| p.mod_data.take()) else {
        return Ok(default);
    };
    let r = f(host, &mut data);
    let displaced = ps(|p| {
        if p.mod_data.is_none() {
            p.mod_data = Some(data);
            None
        } else {
            Some(data)
        }
    });
    drop(displaced);
    r
}

fn s_open(host: &mut dyn Host, m: Module, path: &[u8], name: &[u8]) -> Result<bool, Thrown> {
    match m {
        Module::Files => {
            let o = origin(host);
            let data = mod_files::open(host, &o, path)?;
            let ok = data.is_some();
            let old = ps(|p| std::mem::replace(&mut p.mod_data, data));
            if let Some(old) = old {
                mod_files::close(old);
            }
            Ok(ok)
        }
        Module::User => {
            let args = vec![Value::str(c_str(path)), Value::str(c_str(name))];
            let r = call_handler(host, |u| u.open.clone(), args);
            let r = match r {
                Ok(r) => r,
                Err(t) => {
                    ps(|p| p.status = NONE);
                    return Err(t);
                }
            };
            ps(|p| p.mod_user_implemented = true);
            verify_bool(host, &r)
        }
    }
}

fn s_close(host: &mut dyn Host, m: Module) -> Result<bool, Thrown> {
    match m {
        Module::Files => {
            if let Some(d) = ps(|p| p.mod_data.take()) {
                mod_files::close(d);
            }
            Ok(true)
        }
        Module::User => {
            if !ps(|p| p.mod_user_implemented) {
                return Ok(true);
            }
            let r = call_handler(host, |u| u.close.clone(), Vec::new());
            ps(|p| p.mod_user_implemented = false);
            let r = r?;
            verify_bool(host, &r)
        }
    }
}

fn s_read(host: &mut dyn Host, m: Module, key: &[u8]) -> Result<Option<Vec<u8>>, Thrown> {
    match m {
        Module::Files => {
            let o = origin(host);
            with_files(host, None, |host, f| mod_files::read(host, &o, f, key))
        }
        Module::User => {
            let r = call_handler(host, |u| u.read.clone(), vec![Value::str(key)])?;
            Ok(match r {
                Some(Value::Str(s)) => Some(s.as_bytes().to_vec()),
                _ => None,
            })
        }
    }
}

fn s_write(host: &mut dyn Host, m: Module, key: &[u8], val: &[u8]) -> Result<bool, Thrown> {
    match m {
        Module::Files => {
            let o = origin(host);
            with_files(host, false, |host, f| {
                mod_files::write(host, &o, f, key, val)
            })
        }
        Module::User => {
            let r = call_handler(
                host,
                |u| u.write.clone(),
                vec![Value::str(key), Value::str(val)],
            )?;
            verify_bool(host, &r)
        }
    }
}

fn s_destroy(host: &mut dyn Host, m: Module, key: &[u8]) -> Result<bool, Thrown> {
    match m {
        Module::Files => with_files(host, false, |_, f| Ok(mod_files::destroy(f, key))),
        Module::User => {
            let r = call_handler(host, |u| u.destroy.clone(), vec![Value::str(key)])?;
            verify_bool(host, &r)
        }
    }
}

/// PS_GC_FUNC: the number deleted (-1: failure).
fn s_gc(host: &mut dyn Host, m: Module, maxlifetime: i64) -> Result<i64, Thrown> {
    match m {
        Module::Files => {
            let o = origin(host);
            with_files(host, -1, |host, f| mod_files::gc(host, &o, f, maxlifetime))
        }
        Module::User => {
            let r = call_handler(host, |u| u.gc.clone(), vec![Value::Int(maxlifetime)])?;
            Ok(match r {
                Some(Value::Int(n)) => n,
                Some(Value::Bool(true)) => 1,
                _ => -1,
            })
        }
    }
}

fn s_create_sid(host: &mut dyn Host, m: Module) -> Result<Option<Vec<u8>>, Thrown> {
    match m {
        Module::Files => {
            let data = ps(|p| p.mod_data.take());
            let r = mod_files::create_sid(host, data.as_ref());
            ps(|p| {
                if p.mod_data.is_none() {
                    p.mod_data = data;
                }
            });
            r
        }
        Module::User => {
            if ps(|p| p.user.create_sid.is_none()) {
                return create_id(host);
            }
            let r = call_handler(host, |u| u.create_sid.clone(), Vec::new())?;
            match r {
                Some(Value::Str(s)) => Ok(Some(s.as_bytes().to_vec())),
                Some(other) => {
                    // zval_ptr_dtor: its destructor may throw.
                    drop(other);
                    if let Err(t) = host.run_released_destructors() {
                        throw(host, t);
                    }
                    if !has_exception(host) {
                        throw(host, Thrown::error("Error", "Session id must be a string"));
                    }
                    Ok(None)
                }
                None => {
                    if !has_exception(host) {
                        throw(
                            host,
                            Thrown::error("Error", "No session id returned by function"),
                        );
                    }
                    Ok(None)
                }
            }
        }
    }
}

/// PS_VALIDATE_SID_FUNC: SUCCESS (the session exists)?
fn s_validate_sid(host: &mut dyn Host, m: Module, key: &[u8]) -> Result<bool, Thrown> {
    match m {
        Module::Files => Ok(ps(|p| mod_files::validate_sid(p.mod_data.as_ref(), key))),
        Module::User => {
            if ps(|p| p.user.validate_sid.is_none()) {
                return Ok(true);
            }
            let r = call_handler(host, |u| u.validate_sid.clone(), vec![Value::str(key)])?;
            verify_bool(host, &r)
        }
    }
}

fn s_update_timestamp(
    host: &mut dyn Host,
    m: Module,
    key: &[u8],
    val: &[u8],
) -> Result<bool, Thrown> {
    match m {
        Module::Files => {
            let o = origin(host);
            with_files(host, false, |host, f| {
                mod_files::update_timestamp(host, &o, f, key, val)
            })
        }
        Module::User => {
            let args = vec![Value::str(key), Value::str(val)];
            let r = if ps(|p| p.user.update_timestamp.is_some()) {
                call_handler(host, |u| u.update_timestamp.clone(), args)?
            } else {
                call_handler(host, |u| u.write.clone(), args)?
            };
            verify_bool(host, &r)
        }
    }
}

/// `PS(mod_data) || PS(mod_user_implemented)`.
fn mod_open() -> bool {
    ps(|p| p.mod_data.is_some() || p.mod_user_implemented)
}

fn module() -> Option<Module> {
    ps(|p| p.module)
}

// ---------------------------------------------------------------------------
// Session lifecycle
// ---------------------------------------------------------------------------

/// php_rshutdown_session_globals + php_rinit_session_globals.
fn reset_globals(host: &mut dyn Host) -> Result<(), Thrown> {
    let vars = ps(|p| p.vars.take());
    drop(vars);
    let r = if mod_open() {
        match module() {
            Some(m) => s_close(host, m).map(|_| ()),
            None => Ok(()),
        }
    } else {
        Ok(())
    };
    let old = ps(|p| {
        p.id = None;
        p.session_vars = None;
        p.started_at = None;
        p.status = NONE;
        p.in_save_handler = false;
        p.set_handler = false;
        p.mod_user_is_open = false;
        p.define_sid = true;
        p.mod_user_class_name.take();
        p.mod_data.take()
    });
    drop(old);
    r
}

/// php_session_destroy: whether it succeeded.
fn session_destroy(host: &mut dyn Host) -> Result<bool, Thrown> {
    if ps(|p| p.status) != ACTIVE {
        warn(host, "Trying to destroy uninitialized session")?;
        return Ok(false);
    }
    let mut ok = true;
    if let (Some(id), Some(m)) = ps(|p| (p.id.clone(), p.module)) {
        if !s_destroy(host, m, &id)? {
            ok = false;
            if !has_exception(host) {
                warn(host, "Session object destruction failed")?;
            }
        }
    }
    reset_globals(host)?;
    Ok(ok)
}

/// php_session_abort.
fn session_abort(host: &mut dyn Host) -> Result<bool, Thrown> {
    if ps(|p| p.status) != ACTIVE {
        return Ok(false);
    }
    if mod_open() {
        if let Some(m) = module() {
            s_close(host, m)?;
        }
    }
    ps(|p| p.status = NONE);
    Ok(true)
}

/// php_session_gc: the number deleted (-1: none).
fn session_gc(host: &mut dyn Host, immediate: bool) -> Result<i64, Thrown> {
    if !mod_open() {
        return Ok(-1);
    }
    let mut collect = immediate;
    let prob = ini_long(b"session.gc_probability");
    if !collect && prob > 0 {
        let div = ini_long(b"session.gc_divisor").max(1);
        let mut b = [0u8; 8];
        let _ = crate::ext::random::engine::csprng(&mut b);
        collect = (u64::from_le_bytes(b) % div as u64) < prob as u64;
    }
    let mut n = -1;
    if collect {
        if let Some(m) = module() {
            n = s_gc(host, m, gc_maxlifetime())?;
        }
    }
    Ok(n)
}

/// Fail and close: php_session_abort with `has_exception` checks done by
/// the caller.
fn abort_quiet(host: &mut dyn Host) -> Result<(), Thrown> {
    session_abort(host).map(|_| ())
}

/// php_session_initialize: whether it succeeded.
fn session_initialize(host: &mut dyn Host, auto: bool) -> Result<bool, Thrown> {
    ps(|p| p.status = ACTIVE);
    let Some(m) = module() else {
        ps(|p| p.status = DISABLED);
        warn(
            host,
            "No storage module chosen - failed to initialize session",
        )?;
        return Ok(false);
    };
    let (path, name) = (save_path(), session_name());
    if !s_open(host, m, &path, &name)? {
        abort_quiet(host)?;
        if !has_exception(host) {
            let msg = format!(
                "Failed to initialize storage module: {} (path: {})",
                m.name(),
                String::from_utf8_lossy(c_str(&path))
            );
            warn(host, &msg)?;
        }
        return Ok(false);
    }
    let id = ps(|p| p.id.clone());
    let need_new = match &id {
        None => true,
        Some(id) => id.first().is_none_or(|&c| c == 0),
    };
    let strict_reject = !need_new
        && ini_bool(b"session.use_strict_mode")
        && !s_validate_sid(host, m, id.as_deref().unwrap_or_default())?;
    if need_new || strict_reject {
        ps(|p| p.id = None);
        let new = s_create_sid(host, m)?;
        let Some(new) = new else {
            abort_quiet(host)?;
            if !has_exception(host) {
                let msg = format!(
                    "Failed to create session ID: {} (path: {})",
                    m.name(),
                    String::from_utf8_lossy(c_str(&path))
                );
                throw(host, Thrown::error("Error", msg));
            }
            return Ok(false);
        };
        ps(|p| p.id = Some(new));
        if use_cookies() {
            ps(|p| p.send_cookie = true);
        }
    }
    if !reset_id(host)? {
        abort_quiet(host)?;
        return Ok(false);
    }
    track_init(host);
    let id = ps(|p| p.id.clone()).unwrap_or_default();
    let val = s_read(host, m, &id)?;
    let Some(val) = val else {
        abort_quiet(host)?;
        if !has_exception(host) {
            let msg = format!(
                "Failed to read session data: {} (path: {})",
                m.name(),
                String::from_utf8_lossy(c_str(&path))
            );
            warn(host, &msg)?;
        }
        return Ok(false);
    };
    session_gc(host, false)?;
    ps(|p| p.session_vars = None);
    if ini_bool(b"session.lazy_write") {
        ps(|p| p.session_vars = Some(val.clone()));
    }
    session_decode(host, &val)?;
    let at = if auto {
        None
    } else {
        host.executing_file().map(|f| (f, host.executing_line()))
    };
    ps(|p| p.started_at = at);
    Ok(true)
}

/// php_session_save_current_state.
fn save_current_state(host: &mut dyn Host, write: bool) -> Result<(), Thrown> {
    let m = module();
    if write && session_vars().is_some() {
        if let Some(m) = m.filter(|_| mod_open()) {
            let mut fn_name = "write";
            let id = ps(|p| p.id.clone()).unwrap_or_default();
            let ok = match session_encode(host)? {
                Some(val) => {
                    // lazy_write: unchanged data only has its timestamp
                    // updated (both handlers have s_update_timestamp).
                    let unchanged = ps(|p| p.session_vars.as_deref() == Some(&val[..]));
                    if ini_bool(b"session.lazy_write") && unchanged {
                        fn_name = if ps(|p| p.mod_user_class_name.is_some()) {
                            "updateTimestamp"
                        } else {
                            "update_timestamp"
                        };
                        s_update_timestamp(host, m, &id, &val)?
                    } else {
                        s_write(host, m, &id, &val)?
                    }
                }
                None => s_write(host, m, &id, b"")?,
            };
            if !ok && !has_exception(host) {
                let path = save_path();
                let path = String::from_utf8_lossy(c_str(&path)).into_owned();
                let msg = if !ps(|p| p.mod_user_implemented) {
                    format!("Failed to write session data ({}). Please verify that the current setting of session.save_path is correct ({path})", m.name())
                } else if let Some(cls) = ps(|p| p.mod_user_class_name.clone()) {
                    format!("Failed to write session data using user defined save handler. (session.save_path: {path}, handler: {}::{fn_name})", String::from_utf8_lossy(&cls))
                } else {
                    format!("Failed to write session data using user defined save handler. (session.save_path: {path}, handler: {fn_name})")
                };
                warn(host, &msg)?;
            }
        }
    }
    if mod_open() {
        if let Some(m) = m {
            s_close(host, m)?;
        }
    }
    Ok(())
}

/// php_session_flush.
fn session_flush(host: &mut dyn Host, write: bool) -> Result<bool, Thrown> {
    if ps(|p| p.status) != ACTIVE {
        return Ok(false);
    }
    save_current_state(host, write)?;
    ps(|p| p.status = NONE);
    Ok(true)
}

/// php_session_reset_id: whether it succeeded.
fn reset_id(host: &mut dyn Host) -> Result<bool, Thrown> {
    let Some(id) = ps(|p| p.id.clone()) else {
        warn(
            host,
            "Cannot set session ID - session ID is not initialized",
        )?;
        return Ok(false);
    };
    if use_cookies() && ps(|p| p.send_cookie) {
        if !send_cookie(host)? {
            return Ok(false);
        }
        ps(|p| p.send_cookie = false);
    }
    let name = session_name();
    let sid = if ps(|p| p.define_sid) {
        let mut v = name.clone();
        v.push(b'=');
        v.extend_from_slice(&id);
        v
    } else {
        Vec::new()
    };
    host.set_module_const(b"SID", Value::str(sid));
    // Trans-sid when there is no session cookie.
    let mut apply = ini_bool(b"session.use_trans_sid") && !use_only_cookies();
    if apply && use_cookies() {
        if let Some(Value::Array(c)) = host.global_var(b"_COOKIE") {
            if c.get(&ArrayKey::from_bytes(&name)).is_some() {
                apply = false;
            }
        }
    }
    if apply {
        crate::ext::standard::url_scanner_ex::reset_session_var(host, &name);
        crate::ext::standard::url_scanner_ex::add_session_var(host, c_str(&name), c_str(&id))?;
    }
    Ok(true)
}

/// php_session_send_cookie: whether it succeeded.
fn send_cookie(host: &mut dyn Host) -> Result<bool, Thrown> {
    if headers_sent(host) {
        headers_sent_error(
            host,
            E_WARNING,
            "Session cookie cannot be sent after headers have already been sent",
        )?;
        return Ok(false);
    }
    let secure = ini_bool(b"session.cookie_secure");
    if ini_bool(b"session.cookie_partitioned") && !secure {
        warn(
            host,
            "Partitioned session cookie cannot be used without also configuring it as secure",
        )?;
        return Ok(false);
    }
    let name = session_name();
    let id = ps(|p| p.id.clone()).unwrap_or_default();
    let mut c = b"Set-Cookie: ".to_vec();
    c.extend_from_slice(&name);
    c.push(b'=');
    c.extend_from_slice(&url_encode(&id));
    let lifetime = ini_long(b"session.cookie_lifetime");
    if lifetime > 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let t = now.saturating_add(lifetime);
        if t > 0 {
            c.extend_from_slice(b"; expires=");
            c.extend_from_slice(&crate::ext::date::php_format_date(
                b"D, d M Y H:i:s \\G\\M\\T",
                t,
                false,
            ));
            c.extend_from_slice(b"; Max-Age=");
            c.extend_from_slice(lifetime.to_string().as_bytes());
        }
    }
    let path = ini(b"session.cookie_path");
    if !path.is_empty() {
        c.extend_from_slice(b"; path=");
        c.extend_from_slice(&path);
    }
    let domain = ini(b"session.cookie_domain");
    if !domain.is_empty() {
        c.extend_from_slice(b"; domain=");
        c.extend_from_slice(&domain);
    }
    if secure {
        c.extend_from_slice(b"; secure");
    }
    if ini_bool(b"session.cookie_partitioned") {
        c.extend_from_slice(b"; Partitioned");
    }
    if ini_bool(b"session.cookie_httponly") {
        c.extend_from_slice(b"; HttpOnly");
    }
    let samesite = ini(b"session.cookie_samesite");
    if !samesite.is_empty() {
        c.extend_from_slice(b"; SameSite=");
        c.extend_from_slice(&samesite);
    }
    // php_session_remove_cookie: an earlier session cookie goes.
    let mut prefix = b"Set-Cookie: ".to_vec();
    prefix.extend_from_slice(&name);
    prefix.push(b'=');
    if let Some(r) = host.response() {
        r.headers.retain(|h| {
            !(h.len() > prefix.len() && h[..prefix.len()].eq_ignore_ascii_case(&prefix))
        });
    }
    crate::main::sapi::header_op(host, crate::main::sapi::HeaderOp::Add(&c, 0))
}

/// php_url_encode.
fn url_encode(s: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = Vec::with_capacity(s.len());
    for &b in s {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(b),
            b' ' => out.push(b'+'),
            _ => out.extend_from_slice(&[b'%', HEX[(b >> 4) as usize], HEX[(b & 0xf) as usize]]),
        }
    }
    out
}

/// php_session_cache_limiter: 0 done, -1 nothing sent, -2 headers sent.
fn cache_limiter(host: &mut dyn Host) -> Result<i32, Thrown> {
    let lim = ini(b"session.cache_limiter");
    if lim.is_empty() {
        return Ok(0);
    }
    if ps(|p| p.status) != ACTIVE {
        return Ok(-1);
    }
    if headers_sent(host) {
        session_abort(host)?;
        headers_sent_error(
            host,
            E_WARNING,
            "Session cache limiter cannot be sent after headers have already been sent",
        )?;
        return Ok(-2);
    }
    let expire = ini_long(b"session.cache_expire");
    let lim = c_str(&lim).to_ascii_lowercase();
    let add = |host: &mut dyn Host, line: &[u8]| -> Result<(), Thrown> {
        crate::main::sapi::header_op(host, crate::main::sapi::HeaderOp::Replace(line, 0))
            .map(|_| ())
    };
    let private_no_expire = |host: &mut dyn Host| -> Result<(), Thrown> {
        let l = format!(
            "Cache-Control: private, max-age={}",
            expire.wrapping_mul(60)
        );
        add(host, l.as_bytes())?;
        last_modified(host)
    };
    match &lim[..] {
        b"public" => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            let l = format!(
                "Expires: {}",
                gmt(now.wrapping_add(expire.wrapping_mul(60)))
            );
            add(host, l.as_bytes())?;
            let l = format!("Cache-Control: public, max-age={}", expire.wrapping_mul(60));
            add(host, l.as_bytes())?;
            last_modified(host)?;
        }
        b"private" => {
            add(host, b"Expires: Thu, 19 Nov 1981 08:52:00 GMT")?;
            private_no_expire(host)?;
        }
        b"private_no_expire" => private_no_expire(host)?,
        b"nocache" => {
            add(host, b"Expires: Thu, 19 Nov 1981 08:52:00 GMT")?;
            add(host, b"Cache-Control: no-store, no-cache, must-revalidate")?;
            add(host, b"Pragma: no-cache")?;
        }
        _ => return Ok(-1),
    }
    Ok(0)
}

/// strcpy_gmt.
fn gmt(t: i64) -> String {
    String::from_utf8_lossy(&crate::ext::date::php_format_date(
        b"D, d M Y H:i:s \\G\\M\\T",
        t,
        false,
    ))
    .into_owned()
}

/// last_modified: the script's mtime.
fn last_modified(host: &mut dyn Host) -> Result<(), Thrown> {
    let path = host.request_info().and_then(|r| {
        r.server
            .iter()
            .find(|(k, _)| k == b"PATH_TRANSLATED" || k == b"SCRIPT_FILENAME")
            .map(|(_, v)| v.clone())
    });
    let Some(path) = path else {
        return Ok(());
    };
    let Ok(st) = crate::zend::virtual_cwd::stat(&path, true) else {
        return Ok(());
    };
    let l = format!("Last-Modified: {}", gmt(st.st_mtime));
    crate::main::sapi::header_op(host, crate::main::sapi::HeaderOp::Replace(l.as_bytes(), 0))
        .map(|_| ())
}

/// proposed_session_id_to_session_id.
fn propose_id(v: &Value) {
    match deref(v) {
        Value::Str(s) => ps(|p| {
            p.id = Some(s.as_bytes().to_vec());
            p.send_cookie = false;
        }),
        _ => ps(|p| {
            p.id = None;
            p.send_cookie = true;
        }),
    }
}

/// try_find_session_id_in_global / _in_cookies.
fn find_id_in(host: &mut dyn Host, global: &[u8], cookie: bool) {
    if ps(|p| p.id.is_some()) {
        return;
    }
    let Some(Value::Array(a)) = host.global_var(global).map(|v| deref(&v)) else {
        return;
    };
    if let Some(v) = a.get(&ArrayKey::from_bytes(&session_name())) {
        propose_id(v);
        if cookie {
            ps(|p| {
                p.send_cookie = false;
                p.define_sid = false;
            });
        }
    }
}

/// php_session_start: whether it succeeded.
fn session_start(host: &mut dyn Host, auto: bool) -> Result<bool, Thrown> {
    ensure_init();
    match ps(|p| p.status) {
        ACTIVE => {
            already_started_error(
                host,
                E_NOTICE,
                "Ignoring session_start() because a session has already been started",
            )?;
            return Ok(false);
        }
        DISABLED => {
            if module().is_none() {
                let v = ini(b"session.save_handler");
                match find_module(&v) {
                    Some(m) => ps(|p| p.module = Some(m)),
                    None => {
                        let msg = format!(
                            "Cannot find session save handler \"{}\" - session startup failed",
                            String::from_utf8_lossy(c_str(&v))
                        );
                        warn(host, &msg)?;
                        return Ok(false);
                    }
                }
            }
            if ps(|p| p.serializer.is_none()) {
                let v = ini(b"session.serialize_handler");
                match find_serializer(&v) {
                    Some(s) => ps(|p| p.serializer = Some(s)),
                    None => {
                        let msg = format!("Cannot find session serialization handler \"{}\" - session startup failed", String::from_utf8_lossy(c_str(&v)));
                        warn(host, &msg)?;
                        return Ok(false);
                    }
                }
            }
            ps(|p| p.status = NONE);
        }
        _ => {}
    }
    let only = use_only_cookies();
    ps(|p| {
        p.define_sid = !only;
        p.send_cookie = use_cookies() || only;
    });
    if ps(|p| p.id.is_none()) {
        if use_cookies() {
            find_id_in(host, b"_COOKIE", true);
        }
        if !only {
            find_id_in(host, b"_GET", false);
            find_id_in(host, b"_POST", false);
            // session.referer_check
            let chk = ini(b"session.referer_check");
            if ps(|p| p.id.is_some()) && !c_str(&chk).is_empty() {
                if let Some(Value::Array(s)) = host.global_var(b"_SERVER") {
                    if let Some(Value::Str(r)) =
                        s.get(&ArrayKey::from_bytes(b"HTTP_REFERER")).map(deref)
                    {
                        let r = c_str(r.as_bytes());
                        let chk = c_str(&chk);
                        if !r.is_empty() && !r.windows(chk.len()).any(|w| w == chk) {
                            ps(|p| p.id = None);
                        }
                    }
                }
            }
        }
    }
    // An id holding characters that are unsafe in HTML is dropped.
    if ps(|p| {
        p.id.as_ref()
            .is_some_and(|id| c_str(id).iter().any(|c| b"\r\n\t <>'\"\\".contains(c)))
    }) {
        ps(|p| p.id = None);
    }
    if !session_initialize(host, auto)? || cache_limiter(host)? == -2 {
        ps(|p| {
            p.status = NONE;
            p.id = None;
        });
        return Ok(false);
    }
    Ok(true)
}

/// RINIT: session.auto_start.
pub(crate) fn rinit(host: &mut dyn Host) -> Result<(), Thrown> {
    if !crate::main::ini::module_loaded("session") || !ini_bool(b"session.auto_start") {
        return Ok(());
    }
    ensure_init();
    if ps(|p| p.status) == DISABLED {
        return Ok(());
    }
    session_call(host, Some("PHP Request Startup".into()), |host| {
        session_start(host, true).map(|_| ())
    })
}

/// RSHUTDOWN: write and close an active session.
pub(crate) fn rshutdown(host: &mut dyn Host) -> Result<(), Thrown> {
    if !ps(|p| p.init) {
        return Ok(());
    }
    ps(|p| p.in_shutdown = true);
    let r = session_call(host, Some("PHP Request Shutdown".into()), |host| {
        // zend_try: a bailout in the flush still closes the session.
        let flushed = if ps(|p| p.status) == ACTIVE {
            session_flush(host, true).map(|_| ())
        } else {
            Ok(())
        };
        let closed = reset_globals(host);
        flushed.and(closed)
    });
    let user = ps(|p| std::mem::take(&mut p.user));
    drop(user);
    r
}

// ---------------------------------------------------------------------------
// ini handlers
// ---------------------------------------------------------------------------

/// Where an ini handler runs (ZEND_INI_STAGE_*).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Stage {
    Startup,
    Runtime,
}

/// The diagnostics of a session.* ini handler for `name = v`, raised (at
/// run time, from `func`) or returned (at startup), and whether it accepts
/// the value. `None`: not a session directive.
pub(crate) fn on_modify(
    host: Option<&mut dyn Host>,
    func: &str,
    name: &[u8],
    v: &[u8],
    stage: Stage,
) -> Result<Option<(bool, Diags)>, Thrown> {
    let Some(key) = name.strip_prefix(b"session.") else {
        return Ok(None);
    };
    let mut diags = Diags::new();
    let runtime = stage == Stage::Runtime;
    // The trans-sid settings belong to the URL rewriter (no session checks).
    match key {
        b"trans_sid_tags" => {
            if !v.starts_with(b"a=href,area=href,frame=src,form=") {
                diags.push((
                    E_DEPRECATED,
                    "Usage of session.trans_sid_tags INI setting is deprecated".into(),
                ));
            }
            return emit(host, func, true, diags);
        }
        b"trans_sid_hosts" => {
            if !v.is_empty() {
                diags.push((
                    E_DEPRECATED,
                    "Usage of session.trans_sid_hosts INI setting is deprecated".into(),
                ));
            }
            return emit(host, func, true, diags);
        }
        _ => {}
    }
    let checked = !matches!(
        key,
        b"auto_start"
            | b"upload_progress.enabled"
            | b"upload_progress.cleanup"
            | b"upload_progress.prefix"
            | b"upload_progress.name"
            | b"upload_progress.freq"
            | b"upload_progress.min_freq"
    );
    let mut host = host;
    if runtime {
        ensure_init();
    }
    if checked && runtime {
        let h = host.as_deref_mut().expect("a host at run time");
        // SESSION_CHECK_ACTIVE_STATE, SESSION_CHECK_OUTPUT_STATE.
        if ps(|p| p.status) == ACTIVE {
            already_started_error_from(
                h,
                func,
                "Session ini settings cannot be changed when a session is active",
            )?;
            return Ok(Some((false, Vec::new())));
        }
        if headers_sent(h) {
            headers_sent_error_from(
                h,
                func,
                "Session ini settings cannot be changed after headers have already been sent",
            )?;
            return Ok(Some((false, Vec::new())));
        }
    }
    let err_runtime = if runtime { E_WARNING } else { E_ERROR };
    let ok = match key {
        b"save_handler" => {
            let tmp = find_module(v);
            if runtime && tmp.is_none() {
                diags.push((
                    err_runtime,
                    format!(
                        "Session save handler \"{}\" cannot be found",
                        String::from_utf8_lossy(c_str(v))
                    ),
                ));
                false
            } else if !ps(|p| p.set_handler) && tmp == Some(Module::User) {
                diags.push((
                    err_runtime,
                    "Session save handler \"user\" cannot be set by ini_set()".into(),
                ));
                false
            } else {
                if runtime {
                    ps(|p| {
                        p.default_mod = p.module;
                        p.module = tmp;
                    });
                }
                true
            }
        }
        b"serialize_handler" => {
            let tmp = find_serializer(v);
            if runtime && tmp.is_none() {
                diags.push((
                    err_runtime,
                    format!(
                        "Serialization handler \"{}\" cannot be found",
                        String::from_utf8_lossy(c_str(v))
                    ),
                ));
                false
            } else {
                if runtime {
                    ps(|p| p.serializer = tmp);
                }
                true
            }
        }
        b"save_path" => {
            if runtime && v.contains(&0) {
                false
            } else if runtime {
                // The open_basedir check of the directory: after "N;" and
                // "MODE;".
                let mut p = v;
                if let Some(i) = v.iter().position(|&c| c == b';') {
                    p = &v[i + 1..];
                    if let Some(j) = p.iter().position(|&c| c == b';') {
                        p = &p[j + 1..];
                    }
                }
                let h = host.as_deref_mut().expect("a host at run time");
                !(crate::main::fopen_wrappers::active()
                    && !p.is_empty()
                    && !crate::main::fopen_wrappers::check(h, func, p, true)?)
            } else {
                true
            }
        }
        b"name" => {
            let numeric = conv::numeric_string(v).is_some();
            if v.is_empty()
                || v.contains(&0)
                || numeric
                || v.iter().any(|c| FORBIDDEN_CHARS.contains(c))
            {
                diags.push((
                    E_WARNING,
                    format!("session.name \"{}\" must not be numeric, empty, contain null bytes or any of the following characters \"=,;.[ \\t\\r\\n\\013\\014\"", String::from_utf8_lossy(c_str(v))),
                ));
                false
            } else {
                true
            }
        }
        b"cookie_lifetime" => {
            if c_atol(v) < 0 {
                diags.push((E_WARNING, "CookieLifetime cannot be negative".into()));
                false
            } else {
                true
            }
        }
        b"sid_length" => {
            let (val, rest) = strtol_rest(v);
            if val != 32 {
                diags.push((
                    E_DEPRECATED,
                    "session.sid_length INI setting is deprecated".into(),
                ));
            }
            if rest && (22..=MAX_SID_LENGTH as i64).contains(&val) {
                true
            } else {
                diags.push((
                    E_WARNING,
                    "session.configuration \"session.sid_length\" must be between 22 and 256"
                        .into(),
                ));
                false
            }
        }
        b"sid_bits_per_character" => {
            let (val, rest) = strtol_rest(v);
            if val != 4 {
                diags.push((
                    E_DEPRECATED,
                    "session.sid_bits_per_character INI setting is deprecated".into(),
                ));
            }
            if rest && (4..=6).contains(&val) {
                true
            } else {
                diags.push((
                    E_WARNING,
                    "session.configuration \"session.sid_bits_per_character\" must be between 4 and 6".into(),
                ));
                false
            }
        }
        b"gc_probability" => {
            if let Some(e) = crate::main::ini::quantity_error(v) {
                diags.push((
                    QUANTITY,
                    format!("Invalid \"session.gc_probability\" setting. {e}"),
                ));
            }
            if ini_parse_quantity(v) < 0 {
                diags.push((
                    E_WARNING,
                    "session.gc_probability must be greater than or equal to 0".into(),
                ));
                false
            } else {
                true
            }
        }
        b"gc_divisor" => {
            if let Some(e) = crate::main::ini::quantity_error(v) {
                diags.push((
                    QUANTITY,
                    format!("Invalid \"session.gc_divisor\" setting. {e}"),
                ));
            }
            if ini_parse_quantity(v) <= 0 {
                diags.push((
                    E_WARNING,
                    "session.gc_divisor must be greater than 0".into(),
                ));
                false
            } else {
                true
            }
        }
        b"gc_maxlifetime" | b"cache_expire" => {
            if let Some(e) = crate::main::ini::quantity_error(v) {
                diags.push((
                    QUANTITY,
                    format!(
                        "Invalid \"session.{}\" setting. {e}",
                        String::from_utf8_lossy(key)
                    ),
                ));
            }
            true
        }
        b"use_only_cookies" => {
            if !ini_parse_bool(v) {
                diags.push((
                    E_DEPRECATED,
                    "Disabling session.use_only_cookies INI setting is deprecated".into(),
                ));
            }
            true
        }
        b"use_trans_sid" => {
            if ini_parse_bool(v) {
                diags.push((
                    E_DEPRECATED,
                    "Enabling session.use_trans_sid INI setting is deprecated".into(),
                ));
            }
            true
        }
        b"referer_check" => {
            if !v.is_empty() {
                diags.push((
                    E_DEPRECATED,
                    "Usage of session.referer_check INI setting is deprecated".into(),
                ));
            }
            true
        }
        b"upload_progress.freq" => {
            let tmp = c_atol(v) as i32;
            if tmp < 0 {
                diags.push((
                    E_WARNING,
                    "session.upload_progress.freq must be greater than or equal to 0".into(),
                ));
                false
            } else if v.last() == Some(&b'%') && tmp > 100 {
                diags.push((
                    E_WARNING,
                    "session.upload_progress.freq must be less than or equal to 100%".into(),
                ));
                false
            } else {
                true
            }
        }
        _ => true,
    };
    emit(host, func, ok, diags)
}

/// An ini handler's diagnostics: (level, message). [`QUANTITY`] is a
/// zend_ini_parse_quantity_warn warning (zend_error: no docref prefix).
pub(crate) type Diags = Vec<(i64, String)>;

/// The level of a zend_ini_parse_quantity_warn warning in [`Diags`].
pub(crate) const QUANTITY: i64 = -E_WARNING;

/// Raise `diags` (at run time) or hand them back (startup).
fn emit(
    host: Option<&mut dyn Host>,
    func: &str,
    ok: bool,
    diags: Diags,
) -> Result<Option<(bool, Diags)>, Thrown> {
    match host {
        Some(h) => {
            for (level, msg) in diags {
                if level == QUANTITY {
                    h.emit_diagnostic(E_WARNING, msg.as_bytes())?;
                } else {
                    docref(h, &format!("{func}()"), level, &msg)?;
                }
            }
            Ok(Some((ok, Vec::new())))
        }
        None => Ok(Some((ok, diags))),
    }
}

/// `strtol(v, &end, 10)` and whether it read the whole string.
fn strtol_rest(v: &[u8]) -> (i64, bool) {
    let mut i = 0;
    while i < v.len() && matches!(v[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let start = i;
    if matches!(v.get(i), Some(b'+' | b'-')) {
        i += 1;
    }
    let ds = i;
    while v.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    if i == ds {
        // No digits: endptr is the start of the string.
        return (0, v.is_empty());
    }
    let n = std::str::from_utf8(&v[start..i])
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(if v[start] == b'-' { i64::MIN } else { i64::MAX });
    (n, i == v.len())
}

fn already_started_error_from(host: &mut dyn Host, func: &str, msg: &str) -> Result<(), Thrown> {
    session_call(host, Some(format!("{func}()")), |h| {
        already_started_error(h, E_WARNING, msg)
    })
}

fn headers_sent_error_from(host: &mut dyn Host, func: &str, msg: &str) -> Result<(), Thrown> {
    session_call(host, Some(format!("{func}()")), |h| {
        headers_sent_error(h, E_WARNING, msg)
    })
}

/// zend_alter_ini_entry at run time from the running builtin.
fn alter_ini(host: &mut dyn Host, name: &[u8], v: Vec<u8>) -> Result<bool, Thrown> {
    let func = host.active_function_name();
    Ok(alter_ini_entry_ex(host, name, v, &func, false)?.is_some())
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

fn arg(args: &[Value], i: usize) -> Option<Value> {
    args.get(i).map(deref).filter(|v| !matches!(v, Value::Null))
}

fn arg_bytes(args: &[Value], i: usize) -> Option<Vec<u8>> {
    arg(args, i).map(|v| conv::to_string_bytes(&v))
}

/// zpp's `P`: a path, without NUL bytes.
fn path_arg(args: &[Value], i: usize, f: &str, pname: &str) -> Result<Option<Vec<u8>>, Thrown> {
    match arg_bytes(args, i) {
        Some(p) if p.contains(&0) => Err(crate::zend::zpp::value_error(
            f,
            i + 1,
            pname,
            "must not contain any null bytes",
        )),
        p => Ok(p),
    }
}

/// Run a session_* builtin.
fn call(
    host: &mut dyn Host,
    f: impl FnOnce(&mut dyn Host) -> Result<Value, Thrown>,
) -> Result<Value, Thrown> {
    ensure_init();
    session_call(host, None, f)
}

/// php_can_change_session_setting.
fn can_change(host: &mut dyn Host, what: &str, check_cookies: bool) -> Result<bool, Thrown> {
    if ps(|p| p.status) == ACTIVE {
        let msg = format!("Session {what} cannot be changed when a session is active");
        already_started_error(host, E_WARNING, &msg)?;
        return Ok(false);
    }
    if headers_sent(host) && (!check_cookies || use_cookies()) {
        let msg = format!("Session {what} cannot be changed after headers have already been sent");
        headers_sent_error(host, E_WARNING, &msg)?;
        return Ok(false);
    }
    Ok(true)
}

fn fn_session_name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        let name = path_arg(args, 0, "session_name", "name")?;
        if name.is_some() && !can_change(host, "name", false)? {
            return Ok(Value::Bool(false));
        }
        let old = Value::str(session_name());
        if let Some(n) = name {
            alter_ini(host, b"session.name", n)?;
        }
        Ok(old)
    })
}

fn fn_session_module_name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        let name = arg_bytes(args, 0);
        if name.is_some() && !can_change(host, "save handler module", false)? {
            return Ok(Value::Bool(false));
        }
        let old = Value::str(module().map_or("", Module::name));
        if let Some(n) = name {
            if n.eq_ignore_ascii_case(b"user") {
                return Err(crate::zend::zpp::value_error(
                    "session_module_name",
                    1,
                    "module",
                    "cannot be \"user\"",
                ));
            }
            if find_module(&n).is_none() {
                let msg = format!(
                    "Session handler module \"{}\" cannot be found",
                    String::from_utf8_lossy(c_str(&n))
                );
                warn(host, &msg)?;
                return Ok(Value::Bool(false));
            }
            if mod_open() {
                if let Some(m) = module() {
                    s_close(host, m)?;
                }
            }
            let old_data = ps(|p| p.mod_data.take());
            drop(old_data);
            alter_ini(host, b"session.save_handler", n)?;
        }
        Ok(old)
    })
}

fn can_handler_change(host: &mut dyn Host) -> Result<bool, Thrown> {
    if ps(|p| p.status) == ACTIVE {
        already_started_error(
            host,
            E_WARNING,
            "Session save handler cannot be changed when a session is active",
        )?;
        return Ok(false);
    }
    if headers_sent(host) {
        headers_sent_error(
            host,
            E_WARNING,
            "Session save handler cannot be changed after headers have already been sent",
        )?;
        return Ok(false);
    }
    Ok(true)
}

/// set_user_save_handler_ini.
fn set_user_save_handler_ini(host: &mut dyn Host) -> Result<(), Thrown> {
    ps(|p| p.set_handler = true);
    let r = alter_ini(host, b"session.save_handler", b"user".to_vec());
    ps(|p| p.set_handler = false);
    r.map(|_| ())
}

fn fn_session_set_save_handler(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        const F: &str = "session_set_save_handler";
        if args.len() <= 2 {
            // The object version.
            let obj = deref(&args[0]);
            let is_handler = match &obj {
                Value::Object(h) => {
                    let cid = h.borrow().class;
                    host.class_name(cid)
                        .is_some_and(|n| host.reflect_is_a(&n, b"SessionHandlerInterface"))
                }
                _ => false,
            };
            if !is_handler {
                return Err(Thrown::type_error(format!(
                    "{F}(): Argument #1 ($open) must be of type SessionHandlerInterface, {} given",
                    crate::zend::zpp::zval_type_name(host, &obj)
                )));
            }
            let register_shutdown = match args.get(1).map(deref) {
                None => true,
                Some(_) => crate::zend::zpp::bool_param(host, args, 1, F, "close", true)?,
            };
            if !can_handler_change(host)? {
                return Ok(Value::Bool(false));
            }
            let Value::Object(h) = &obj else {
                unreachable!()
            };
            let cid = h.borrow().class;
            let cname = host.class_name(cid).unwrap_or_default();
            let method = |m: &str| {
                let mut a = PhpArray::new();
                a.push(obj.clone());
                a.push(Value::str(m));
                Some(Value::Array(a))
            };
            let has = |host: &dyn Host, m: &str| host.class_has_method(cid, m.as_bytes());
            let is_a = |host: &dyn Host, i: &[u8]| host.reflect_is_a(&cname, i);
            let create_sid = if is_a(host, b"SessionIdInterface") || has(host, "create_sid") {
                method("create_sid")
            } else {
                None
            };
            let (validate_sid, update_timestamp) =
                if is_a(host, b"SessionUpdateTimestampHandlerInterface") {
                    (method("validateid"), method("updatetimestamp"))
                } else {
                    (
                        has(host, "validateid")
                            .then(|| method("validateid"))
                            .flatten(),
                        has(host, "updatetimestamp")
                            .then(|| method("updatetimestamp"))
                            .flatten(),
                    )
                };
            let user = UserNames {
                open: method("open"),
                close: method("close"),
                read: method("read"),
                write: method("write"),
                destroy: method("destroy"),
                gc: method("gc"),
                create_sid,
                validate_sid,
                update_timestamp,
            };
            let old = ps(|p| {
                p.mod_user_class_name = Some(cname.clone());
                std::mem::replace(&mut p.user, user)
            });
            drop(old);
            if register_shutdown {
                host.register_named_shutdown(
                    b"session_shutdown",
                    Value::str("session_register_shutdown"),
                );
            } else {
                host.remove_named_shutdown(b"session_shutdown");
            }
            if ps(|p| p.status) != ACTIVE && module() != Some(Module::User) {
                set_user_save_handler_ini(host)?;
            }
            return Ok(Value::Bool(true));
        }
        let o = origin(host);
        docref(
            host,
            &o,
            E_DEPRECATED,
            "Providing individual callbacks instead of an object implementing SessionHandlerInterface is deprecated",
        )?;
        let names = [
            "open",
            "close",
            "read",
            "write",
            "destroy",
            "gc",
            "create_sid",
            "validate_sid",
            "update_timestamp",
        ];
        if args.len() < 6 {
            return Err(Thrown::error(
                "ArgumentCountError",
                format!("{F}() expects at least 6 arguments, {} given", args.len()),
            ));
        }
        let mut cbs: Vec<Option<Value>> = Vec::new();
        for (i, n) in names.iter().enumerate() {
            let v = args.get(i).map(deref);
            let v = match v {
                None => None,
                Some(Value::Null) if i >= 6 => None,
                Some(v) => {
                    if let Some(why) = host.check_callable(&v)? {
                        let null = if i >= 6 { " or null" } else { "" };
                        return Err(Thrown::type_error(format!(
                            "{F}(): Argument #{} (${n}) must be a valid callback{null}, {why}",
                            i + 1
                        )));
                    }
                    Some(v)
                }
            };
            cbs.push(v);
        }
        if !can_handler_change(host)? {
            return Ok(Value::Bool(false));
        }
        ps(|p| p.mod_user_class_name = None);
        host.remove_named_shutdown(b"session_shutdown");
        if module() != Some(Module::User) {
            set_user_save_handler_ini(host)?;
        }
        let mut it = cbs.into_iter();
        let mut next = || it.next().flatten();
        let mut user = ps(|p| p.user.clone());
        user.open = next();
        user.close = next();
        user.read = next();
        user.write = next();
        user.destroy = next();
        user.gc = next();
        if let Some(v) = next() {
            user.create_sid = Some(v);
        }
        if let Some(v) = next() {
            user.validate_sid = Some(v);
        }
        if let Some(v) = next() {
            user.update_timestamp = Some(v);
        }
        let old = ps(|p| std::mem::replace(&mut p.user, user));
        drop(old);
        Ok(Value::Bool(true))
    })
}

fn fn_session_save_path(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        let name = path_arg(args, 0, "session_save_path", "path")?;
        if name.is_some() && !can_change(host, "save path", false)? {
            return Ok(Value::Bool(false));
        }
        let old = Value::str(save_path());
        if let Some(n) = name {
            alter_ini(host, b"session.save_path", n)?;
        }
        Ok(old)
    })
}

fn fn_session_id(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        let name = arg_bytes(args, 0);
        if name.is_some() && !can_change(host, "ID", true)? {
            return Ok(Value::Bool(false));
        }
        let old = Value::str(ps(|p| {
            p.id.as_deref().map(c_str).unwrap_or_default().to_vec()
        }));
        if let Some(n) = name {
            ps(|p| p.id = Some(n));
        }
        Ok(old)
    })
}

fn fn_session_regenerate_id(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        let del = args.first().is_some_and(|v| conv::to_bool(&deref(v)));
        if ps(|p| p.status) != ACTIVE {
            already_started_error(
                host,
                E_WARNING,
                "Session ID cannot be regenerated when there is no active session",
            )?;
            return Ok(Value::Bool(false));
        }
        if headers_sent(host) {
            headers_sent_error(
                host,
                E_WARNING,
                "Session ID cannot be regenerated after headers have already been sent",
            )?;
            return Ok(Value::Bool(false));
        }
        let Some(m) = module() else {
            return Ok(Value::Bool(false));
        };
        let path = save_path();
        let path_s = String::from_utf8_lossy(c_str(&path)).into_owned();
        let id = ps(|p| p.id.clone()).unwrap_or_default();
        if del {
            if !s_destroy(host, m, &id)? {
                s_close(host, m)?;
                ps(|p| p.status = NONE);
                if !has_exception(host) {
                    let msg = format!(
                        "Session object destruction failed. ID: {} (path: {path_s})",
                        m.name()
                    );
                    warn(host, &msg)?;
                }
                return Ok(Value::Bool(false));
            }
        } else {
            let data = session_encode(host)?.unwrap_or_default();
            if !s_write(host, m, &id, &data)? {
                s_close(host, m)?;
                ps(|p| p.status = NONE);
                let msg = format!("Session write failed. ID: {} (path: {path_s})", m.name());
                warn(host, &msg)?;
                return Ok(Value::Bool(false));
            }
        }
        s_close(host, m)?;
        ps(|p| {
            p.session_vars = None;
            p.id = None;
        });
        let name = session_name();
        if !s_open(host, m, &path, &name)? {
            ps(|p| p.status = NONE);
            if !has_exception(host) {
                let msg = format!("Failed to open session: {} (path: {path_s})", m.name());
                throw(host, Thrown::error("Error", msg));
            }
            return Ok(Value::Null);
        }
        let Some(new) = s_create_sid(host, m)? else {
            ps(|p| p.status = NONE);
            if !has_exception(host) {
                let msg = format!(
                    "Failed to create new session ID: {} (path: {path_s})",
                    m.name()
                );
                throw(host, Thrown::error("Error", msg));
            }
            return Ok(Value::Null);
        };
        ps(|p| p.id = Some(new));
        if ini_bool(b"session.use_strict_mode") {
            let can_validate = match m {
                Module::Files => !ps(|p| p.mod_user_implemented),
                Module::User => false,
            } || ps(|p| p.user.validate_sid.is_some());
            if can_validate {
                let mut limit = 3;
                loop {
                    if limit == 0 {
                        break;
                    }
                    limit -= 1;
                    let id = ps(|p| p.id.clone()).unwrap_or_default();
                    if !s_validate_sid(host, m, &id)? {
                        break;
                    }
                    ps(|p| p.id = None);
                    let Some(new) = s_create_sid(host, m)? else {
                        s_close(host, m)?;
                        ps(|p| p.status = NONE);
                        if !has_exception(host) {
                            let msg = format!(
                                "Failed to create session ID by collision: {} (path: {path_s})",
                                m.name()
                            );
                            throw(host, Thrown::error("Error", msg));
                        }
                        return Ok(Value::Null);
                    };
                    ps(|p| p.id = Some(new));
                }
            }
        }
        let id = ps(|p| p.id.clone()).unwrap_or_default();
        if s_read(host, m, &id)?.is_none() {
            s_close(host, m)?;
            ps(|p| p.status = NONE);
            if !has_exception(host) {
                let msg = format!(
                    "Failed to create(read) session ID: {} (path: {path_s})",
                    m.name()
                );
                throw(host, Thrown::error("Error", msg));
            }
            return Ok(Value::Null);
        }
        if use_cookies() {
            ps(|p| p.send_cookie = true);
        }
        Ok(Value::Bool(reset_id(host)?))
    })
}

fn fn_session_create_id(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        let prefix = path_arg(args, 0, "session_create_id", "prefix")?.unwrap_or_default();
        let mut id = Vec::new();
        if !prefix.is_empty() {
            if prefix.len() > MAX_SID_LENGTH {
                return Err(crate::zend::zpp::value_error(
                    "session_create_id",
                    1,
                    "prefix",
                    &format!("cannot be longer than {MAX_SID_LENGTH} characters"),
                ));
            }
            if !mod_files::valid_key(&prefix) {
                warn(host, "Prefix cannot contain special characters. Only the A-Z, a-z, 0-9, \"-\", and \",\" characters are allowed")?;
                return Ok(Value::Bool(false));
            }
            id.extend_from_slice(&prefix);
        }
        let mut new_id = None;
        if !ps(|p| p.in_save_handler) && ps(|p| p.status) == ACTIVE {
            if let Some(m) = module() {
                let mut limit = 3;
                while limit > 0 {
                    limit -= 1;
                    new_id = s_create_sid(host, m)?;
                    let Some(nid) = &new_id else {
                        break;
                    };
                    let no_validate = m == Module::User && ps(|p| p.user.validate_sid.is_none());
                    if no_validate {
                        break;
                    }
                    if s_validate_sid(host, m, nid)? {
                        new_id = None;
                        continue;
                    }
                    break;
                }
            }
        } else {
            new_id = create_id(host)?;
        }
        match new_id {
            Some(n) => {
                id.extend_from_slice(&n);
                Ok(Value::str(id))
            }
            None => {
                if has_exception(host) {
                    return Ok(Value::Null);
                }
                warn(host, "Failed to create new ID")?;
                Ok(Value::Bool(false))
            }
        }
    })
}

fn fn_session_cache_limiter(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        let lim = arg_bytes(args, 0);
        if lim.is_some() && ps(|p| p.status) == ACTIVE {
            already_started_error(
                host,
                E_WARNING,
                "Session cache limiter cannot be changed when a session is active",
            )?;
            return Ok(Value::Bool(false));
        }
        if lim.is_some() && headers_sent(host) {
            headers_sent_error(
                host,
                E_WARNING,
                "Session cache limiter cannot be changed after headers have already been sent",
            )?;
            return Ok(Value::Bool(false));
        }
        let old = Value::str(ini(b"session.cache_limiter"));
        if let Some(l) = lim {
            alter_ini(host, b"session.cache_limiter", l)?;
        }
        Ok(old)
    })
}

fn fn_session_cache_expire(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        let exp = arg(args, 0).map(|v| conv::to_int(&v));
        let cur = Value::Int(ini_long(b"session.cache_expire"));
        if exp.is_some() && ps(|p| p.status) == ACTIVE {
            already_started_error(
                host,
                E_WARNING,
                "Session cache expiration cannot be changed when a session is active",
            )?;
            return Ok(cur);
        }
        if exp.is_some() && headers_sent(host) {
            headers_sent_error(
                host,
                E_WARNING,
                "Session cache expiration cannot be changed after headers have already been sent",
            )?;
            return Ok(Value::Bool(false));
        }
        if let Some(e) = exp {
            alter_ini(host, b"session.cache_expire", e.to_string().into_bytes())?;
        }
        Ok(cur)
    })
}

fn fn_session_encode(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        Ok(match session_encode(host)? {
            Some(v) => Value::str(v),
            None => Value::Bool(false),
        })
    })
}

fn fn_session_decode(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        let data = conv::to_string_bytes(&deref(&args[0]));
        if ps(|p| p.status) != ACTIVE {
            warn(
                host,
                "Session data cannot be decoded when there is no active session",
            )?;
            return Ok(Value::Bool(false));
        }
        Ok(Value::Bool(session_decode(host, &data)?))
    })
}

fn fn_session_start(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        const F: &str = "session_start";
        if ps(|p| p.status) == ACTIVE {
            already_started_error(
                host,
                E_NOTICE,
                "Ignoring session_start() because a session is already active",
            )?;
            return Ok(Value::Bool(true));
        }
        if use_cookies() && headers_sent(host) {
            headers_sent_error(
                host,
                E_WARNING,
                "Session cannot be started after headers have already been sent",
            )?;
            return Ok(Value::Bool(false));
        }
        let mut read_and_close = false;
        if let Some(Value::Array(opts)) = args.first().map(deref) {
            for (k, v) in opts.iter() {
                let ArrayKey::Str(k) = k else {
                    return Err(crate::zend::zpp::value_error(
                        F,
                        1,
                        "options",
                        "must be of type array with keys as string",
                    ));
                };
                let key = k.as_bytes();
                let v = deref(v);
                match &v {
                    Value::Str(_) | Value::Bool(_) | Value::Int(_) => {
                        if key == b"read_and_close" {
                            let n = match &v {
                                Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
                                    Some(conv::Num::Int(n)) => n,
                                    _ => {
                                        return Err(Thrown::type_error(format!(
                                            "{F}(): Option \"{}\" value must be of type compatible with int, \"{}\" given",
                                            String::from_utf8_lossy(key),
                                            String::from_utf8_lossy(s.as_bytes())
                                        )))
                                    }
                                },
                                other => conv::to_int(other),
                            };
                            read_and_close = n > 0;
                        } else {
                            let mut name = b"session.".to_vec();
                            name.extend_from_slice(key);
                            let val = conv::to_string_bytes(&v);
                            if !alter_ini_entry_ex(host, &name, val, F, false)?.is_some() {
                                let msg = format!(
                                    "Setting option \"{}\" failed",
                                    String::from_utf8_lossy(c_str(key))
                                );
                                warn(host, &msg)?;
                            }
                        }
                    }
                    other => {
                        return Err(Thrown::type_error(format!(
                            "{F}(): Option \"{}\" must be of type string|int|bool, {} given",
                            String::from_utf8_lossy(key),
                            crate::zend::zpp::zval_value_name(host, other)
                        )))
                    }
                }
            }
        }
        session_start(host, false)?;
        if ps(|p| p.status) != ACTIVE {
            clean_session_vars();
            return Ok(Value::Bool(false));
        }
        if read_and_close {
            session_flush(host, false)?;
        }
        Ok(Value::Bool(true))
    })
}

fn fn_session_destroy(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| Ok(Value::Bool(session_destroy(host)?)))
}

fn fn_session_unset(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    call(host, |_| {
        if ps(|p| p.status) != ACTIVE {
            return Ok(Value::Bool(false));
        }
        clean_session_vars();
        Ok(Value::Bool(true))
    })
}

fn fn_session_gc(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        if ps(|p| p.status) != ACTIVE {
            warn(
                host,
                "Session cannot be garbage collected when there is no active session",
            )?;
            return Ok(Value::Bool(false));
        }
        let n = session_gc(host, true)?;
        Ok(if n < 0 {
            Value::Bool(false)
        } else {
            Value::Int(n)
        })
    })
}

fn fn_session_write_close(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        if ps(|p| p.status) != ACTIVE {
            return Ok(Value::Bool(false));
        }
        session_flush(host, true)?;
        Ok(Value::Bool(true))
    })
}

fn fn_session_abort(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        if ps(|p| p.status) != ACTIVE {
            return Ok(Value::Bool(false));
        }
        session_abort(host)?;
        Ok(Value::Bool(true))
    })
}

fn fn_session_reset(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        if ps(|p| p.status) != ACTIVE {
            return Ok(Value::Bool(false));
        }
        session_initialize(host, false)?;
        Ok(Value::Bool(true))
    })
}

fn fn_session_status(_host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ensure_init();
    Ok(Value::Int(ps(|p| p.status)))
}

fn fn_session_register_shutdown(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    host.register_shutdown_function(Value::str("session_write_close"), Vec::new())?;
    Ok(Value::Null)
}

fn fn_session_get_cookie_params(_host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    let mut set = |k: &str, v: Value| a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
    set("lifetime", Value::Int(ini_long(b"session.cookie_lifetime")));
    set("path", Value::str(ini(b"session.cookie_path")));
    set("domain", Value::str(ini(b"session.cookie_domain")));
    set("secure", Value::Bool(ini_bool(b"session.cookie_secure")));
    set(
        "partitioned",
        Value::Bool(ini_bool(b"session.cookie_partitioned")),
    );
    set(
        "httponly",
        Value::Bool(ini_bool(b"session.cookie_httponly")),
    );
    set("samesite", Value::str(ini(b"session.cookie_samesite")));
    Ok(Value::Array(a))
}

fn fn_session_set_cookie_params(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    call(host, |host| {
        const F: &str = "session_set_cookie_params";
        let first = deref(&args[0]);
        let options = match &first {
            Value::Array(a) => Some(a.clone()),
            _ => None,
        };
        let lifetime_long = match (&options, &first) {
            (Some(_), _) => None,
            (None, v) => Some(conv::to_int(v)),
        };
        let mut path = arg_bytes(args, 1);
        let mut domain = arg_bytes(args, 2);
        let mut secure = arg(args, 3).map(|v| conv::to_bool(&v));
        let mut httponly = arg(args, 4).map(|v| conv::to_bool(&v));
        let mut partitioned = None;
        let mut samesite = None;
        let mut lifetime = None;
        if !use_cookies() {
            warn(
                host,
                "Session cookies cannot be used when session.use_cookies is disabled",
            )?;
            return Ok(Value::Bool(false));
        }
        if ps(|p| p.status) == ACTIVE {
            already_started_error(
                host,
                E_WARNING,
                "Session cookie parameters cannot be changed when a session is active",
            )?;
            return Ok(Value::Bool(false));
        }
        if headers_sent(host) {
            headers_sent_error(
                host,
                E_WARNING,
                "Session cookie parameters cannot be changed after headers have already been sent",
            )?;
            return Ok(Value::Bool(false));
        }
        if let Some(opts) = options {
            let must_null = |n: usize, p: &str| {
                crate::zend::zpp::value_error(
                    F,
                    n,
                    p,
                    "must be null when argument #1 ($lifetime_or_options) is an array",
                )
            };
            if path.is_some() {
                return Err(must_null(2, "path"));
            }
            if domain.is_some() {
                return Err(must_null(3, "domain"));
            }
            if secure.is_some() {
                return Err(must_null(4, "secure"));
            }
            if httponly.is_some() {
                return Err(must_null(5, "httponly"));
            }
            let mut found = 0;
            for (k, v) in opts.iter() {
                let ArrayKey::Str(k) = k else {
                    warn(
                        host,
                        "Argument #1 ($lifetime_or_options) cannot contain numeric keys",
                    )?;
                    continue;
                };
                let v = deref(v);
                let key = k.as_bytes().to_ascii_lowercase();
                match &key[..] {
                    b"lifetime" => lifetime = Some(crate::zend::zpp::zval_get_string(host, &v)?),
                    b"path" => path = Some(crate::zend::zpp::zval_get_string(host, &v)?),
                    b"domain" => domain = Some(crate::zend::zpp::zval_get_string(host, &v)?),
                    b"secure" => secure = Some(conv::to_bool(&v)),
                    b"partitioned" => partitioned = Some(conv::to_bool(&v)),
                    b"httponly" => httponly = Some(conv::to_bool(&v)),
                    b"samesite" => samesite = Some(crate::zend::zpp::zval_get_string(host, &v)?),
                    _ => {
                        let msg = format!(
                            "Argument #1 ($lifetime_or_options) contains an unrecognized key \"{}\"",
                            String::from_utf8_lossy(c_str(k.as_bytes()))
                        );
                        warn(host, &msg)?;
                        continue;
                    }
                }
                found += 1;
            }
            if found == 0 {
                return Err(crate::zend::zpp::value_error(
                    F,
                    1,
                    "lifetime_or_options",
                    "must contain at least 1 valid key",
                ));
            }
        } else {
            lifetime = lifetime_long.map(|n| n.to_string().into_bytes());
        }
        let b = |x: bool| if x { b"1".to_vec() } else { b"0".to_vec() };
        let sets: [(&[u8], Option<Vec<u8>>); 7] = [
            (b"session.cookie_lifetime", lifetime),
            (b"session.cookie_path", path),
            (b"session.cookie_domain", domain),
            (b"session.cookie_secure", secure.map(b)),
            (b"session.cookie_partitioned", partitioned.map(b)),
            (b"session.cookie_httponly", httponly.map(b)),
            (b"session.cookie_samesite", samesite),
        ];
        for (name, v) in sets {
            if let Some(v) = v {
                if !alter_ini(host, name, v)? {
                    return Ok(Value::Bool(false));
                }
            }
        }
        Ok(Value::Bool(true))
    })
}

/// `__session_handler($op, ...)`: SessionHandler's methods (mod_user_class.c).
fn fn_session_handler(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    const METHODS: [&str; 7] = [
        "open",
        "close",
        "read",
        "write",
        "destroy",
        "gc",
        "create_sid",
    ];
    let op = conv::to_int(&deref(&args[0])) as usize;
    let name = format!(
        "SessionHandler::{}()",
        METHODS.get(op).copied().unwrap_or("open")
    );
    let s = |i: usize| {
        args.get(i)
            .map_or(Vec::new(), |v| conv::to_string_bytes(&deref(v)))
    };
    ensure_init();
    session_call(host, Some(name), |host| {
        // PS_SANITY_CHECK
        if ps(|p| p.status) != ACTIVE {
            return Err(Thrown::error("Error", "Session is not active"));
        }
        let Some(dm) = ps(|p| p.default_mod) else {
            return Err(Thrown::error(
                "Error",
                "Cannot call default session handler",
            ));
        };
        // PS_SANITY_CHECK_IS_OPEN
        if (1..=5).contains(&op) && !ps(|p| p.mod_user_is_open) {
            warn(host, "Parent session handler is not open")?;
            return Ok(Value::Bool(false));
        }
        Ok(match op {
            0 => {
                let r = s_open(host, dm, &s(1), &s(2));
                let ok = match r {
                    Ok(ok) => ok,
                    Err(t) => {
                        ps(|p| p.status = NONE);
                        return Err(t);
                    }
                };
                if ok {
                    ps(|p| p.mod_user_is_open = true);
                }
                Value::Bool(ok)
            }
            1 => {
                ps(|p| p.mod_user_is_open = false);
                Value::Bool(s_close(host, dm)?)
            }
            2 => match s_read(host, dm, &s(1))? {
                Some(v) => Value::str(v),
                None => Value::Bool(false),
            },
            3 => Value::Bool(s_write(host, dm, &s(1), &s(2))?),
            4 => Value::Bool(s_destroy(host, dm, &s(1))?),
            5 => {
                let max = args.get(1).map_or(0, |v| conv::to_int(&deref(v)));
                match s_gc(host, dm, max)? {
                    -1 => Value::Bool(false),
                    n => Value::Int(n),
                }
            }
            _ => match s_create_sid(host, dm)? {
                Some(id) => Value::str(id),
                None => {
                    if !has_exception(host) {
                        let msg = format!("Failed to create session ID: {}", dm.name());
                        return Err(Thrown::error("Error", msg));
                    }
                    Value::Null
                }
            },
        })
    })
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("session_name", fn_session_name),
            ("session_module_name", fn_session_module_name),
            ("session_save_path", fn_session_save_path),
            ("session_id", fn_session_id),
            ("session_create_id", fn_session_create_id),
            ("session_regenerate_id", fn_session_regenerate_id),
            ("session_decode", fn_session_decode),
            ("session_encode", fn_session_encode),
            ("session_destroy", fn_session_destroy),
            ("session_unset", fn_session_unset),
            ("session_gc", fn_session_gc),
            ("session_get_cookie_params", fn_session_get_cookie_params),
            ("session_write_close", fn_session_write_close),
            ("session_commit", fn_session_write_close),
            ("session_abort", fn_session_abort),
            ("session_reset", fn_session_reset),
            ("session_status", fn_session_status),
            ("session_register_shutdown", fn_session_register_shutdown),
            ("session_set_save_handler", fn_session_set_save_handler),
            ("session_cache_limiter", fn_session_cache_limiter),
            ("session_cache_expire", fn_session_cache_expire),
            ("session_set_cookie_params", fn_session_set_cookie_params),
            ("session_start", fn_session_start),
        ],
    );
    crate::register_helpers(m, &[("__session_handler", fn_session_handler, 1, Some(3))]);
}

pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("PHP_SESSION_DISABLED", Value::Int(DISABLED)),
        ("PHP_SESSION_NONE", Value::Int(NONE)),
        ("PHP_SESSION_ACTIVE", Value::Int(ACTIVE)),
    ]
}
