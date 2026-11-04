//! PHP runtime / environment introspection (php's `ext/standard` "info" group):
//! `extension_loaded`, `function_exists`, `phpversion`, `php_sapi_name`,
//! `version_compare`, the error/exception-handler setters (stored, not invoked —
//! this engine emits no diagnostics), and a handful of no-op-ish runtime knobs.
//! Enough to boot a real framework / Composer.

use rhp_values::ops::deref;
use rhp_values::{conv, PhpArray, Value};

use crate::zend::zpp::arg_bytes;
use crate::{Host, Thrown};

/// `gethostbyaddr($ip)` (dns.c php_gethostbyaddr): inet_pton as AF_INET6, then
/// AF_INET, then getnameinfo(NI_NAMEREQD). The address itself when it has no
/// name; a warning and false when it is not an address.
fn fn_gethostbyaddr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ip = crate::zend::zpp::path_param(host, args, 0, "gethostbyaddr", "ip")?;
    let mut out = [0 as libc::c_char; 1025];
    let name = |sa: *const libc::sockaddr, len: usize, out: &mut [libc::c_char]| {
        // SAFETY: `sa` points at a zeroed sockaddr_in/_in6 of `len` bytes with
        // family and address set (as php fills it), and `out` is writable for
        // the length passed.
        let rc = unsafe {
            libc::getnameinfo(
                sa,
                len as libc::socklen_t,
                out.as_mut_ptr(),
                out.len() as libc::socklen_t,
                std::ptr::null_mut(),
                0,
                libc::NI_NAMEREQD,
            )
        };
        // SAFETY: getnameinfo NUL-terminated `out` on success.
        (rc == 0).then(|| {
            unsafe { std::ffi::CStr::from_ptr(out.as_ptr()) }
                .to_bytes()
                .to_vec()
        })
    };
    let found = if let Some(a) = crate::inet::pton6(&ip) {
        // SAFETY: all-zero is a valid sockaddr_in6.
        let mut sa: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
        sa.sin6_family = libc::AF_INET6 as libc::sa_family_t;
        sa.sin6_addr.s6_addr = a;
        name(
            &sa as *const _ as *const libc::sockaddr,
            std::mem::size_of_val(&sa),
            &mut out,
        )
    } else if let Some(a) = crate::inet::pton4(&ip) {
        // SAFETY: all-zero is a valid sockaddr_in.
        let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        sa.sin_family = libc::AF_INET as libc::sa_family_t;
        sa.sin_addr.s_addr = u32::from_ne_bytes(a);
        name(
            &sa as *const _ as *const libc::sockaddr,
            std::mem::size_of_val(&sa),
            &mut out,
        )
    } else {
        crate::warn(
            host,
            "gethostbyaddr",
            "Address is not a valid IPv4 or IPv6 address",
        )?;
        return Ok(Value::Bool(false));
    };
    Ok(Value::str(found.unwrap_or(ip)))
}

fn ob(host: &mut dyn Host, call: crate::ObCall) -> Result<Value, Thrown> {
    host.output_control(call)
}

fn fn_ob_start(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let callback = args.first().map(deref).unwrap_or(Value::Null);
    let chunk_size = crate::zend::zpp::int_param(host, args, 1, "ob_start", "chunk_size", 0)?;
    let flags = crate::zend::zpp::int_param(host, args, 2, "ob_start", "flags", 0x70)?;
    ob(
        host,
        crate::ObCall::Start {
            callback,
            chunk_size,
            flags,
        },
    )
}

fn fn_ob_flush(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::Flush)
}

fn fn_ob_clean(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::Clean)
}

fn fn_ob_end_flush(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::EndFlush)
}

fn fn_ob_end_clean(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::EndClean)
}

fn fn_ob_get_flush(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::GetFlush)
}

fn fn_ob_get_clean(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::GetClean)
}

fn fn_ob_get_contents(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::GetContents)
}

fn fn_ob_get_length(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::GetLength)
}

fn fn_ob_get_level(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::GetLevel)
}

fn fn_ob_list_handlers(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    ob(host, crate::ObCall::ListHandlers)
}

fn fn_ob_get_status(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let full = crate::zend::zpp::bool_param(host, args, 0, "ob_get_status", "full_status", false)?;
    ob(host, crate::ObCall::GetStatus { full })
}

fn fn_ob_implicit_flush(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let on = crate::zend::zpp::bool_param(host, args, 0, "ob_implicit_flush", "enable", true)?;
    ob(host, crate::ObCall::ImplicitFlush(on))
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

pub(crate) fn reset_state() {
    ENV_OVERLAY.with(|e| e.borrow_mut().clear());
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
    let entry = (name.to_vec(), value.map(|v| v.to_vec()));
    ENV_OVERLAY.with(|e| e.borrow_mut().push(entry));
    Ok(Value::Bool(true))
}

fn fn_set_time_limit(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let secs = crate::zend::zpp::int_param(host, args, 0, "set_time_limit", "seconds", 0)?;
    crate::runtime_fns::ini_store(b"max_execution_time", secs.to_string().into_bytes());
    Ok(Value::Bool(true))
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
    if let Some(why) = host.check_callable(&cb)? {
        return Err(Thrown::type_error(format!(
            "register_shutdown_function(): Argument #1 ($callback) must be a valid callback, {why}"
        )));
    }
    host.register_shutdown_function(cb, args[1..].iter().map(deref).collect())?;
    Ok(Value::Null)
}

fn fn_getmypid(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(std::process::id() as i64))
}

/// `assert($assertion, $description = null)` — PHP 8: evaluate `$assertion`; if
/// falsy, throw AssertionError (assertions are treated as enabled, the CLI
/// default). A string `$description` becomes the message; a Throwable is thrown.
fn fn_assert(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if conv::to_bool(&deref(&args[0])) {
        return Ok(Value::Bool(true));
    }
    match args.get(1).map(deref) {
        Some(Value::Str(s)) => Err(Thrown::error(
            "AssertionError",
            String::from_utf8_lossy(s.as_bytes()).into_owned(),
        )),
        _ => Err(Thrown::error("AssertionError", "assert failed")),
    }
}

// --- randomness --------------------------------------------------------------

fn rand_u64() -> u64 {
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    u64::from_le_bytes(b)
}

/// A uniform integer in `[min, max]` (inclusive) via rejection sampling.
fn rand_range(min: i64, max: i64) -> i64 {
    if min >= max {
        return min;
    }
    let span = (max as i128 - min as i128 + 1) as u128;
    // Rejection-sample to avoid modulo bias.
    let limit = u128::MAX - (u128::MAX % span);
    loop {
        let r = ((rand_u64() as u128) << 64) | rand_u64() as u128;
        if r < limit {
            return (min as i128 + (r % span) as i128) as i64;
        }
    }
}

fn fn_random_bytes(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let len = conv::to_int(&deref(&args[0]));
    if len <= 0 {
        return Err(Thrown::error(
            "ValueError",
            "random_bytes(): Argument #1 ($length) must be greater than 0",
        ));
    }
    let mut b = vec![0u8; len as usize];
    if getrandom::getrandom(&mut b).is_err() {
        return Err(Thrown::error(
            "Random\\RandomException",
            "Could not gather sufficient random data",
        ));
    }
    Ok(Value::str(b))
}

fn fn_random_int(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let min = conv::to_int(&deref(&args[0]));
    let max = conv::to_int(&deref(&args[1]));
    if min > max {
        return Err(Thrown::error(
            "ValueError",
            "random_int(): Argument #1 ($min) must be less than or equal to argument #2 ($max)",
        ));
    }
    Ok(Value::Int(rand_range(min, max)))
}

fn fn_getrandmax(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(2147483647))
}

fn fn_uniqid(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let prefix = args
        .first()
        .map(|v| conv::to_string_bytes(&deref(v)))
        .unwrap_or_default();
    let more = args
        .get(1)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let mut s = prefix;
    // php: 8 hex of seconds + 5 hex of microseconds.
    s.extend_from_slice(
        format!(
            "{:08x}{:05x}",
            now.as_secs() & 0xffff_ffff,
            now.subsec_micros() & 0xf_ffff
        )
        .as_bytes(),
    );
    if more {
        s.extend_from_slice(format!(".{:08}", rand_range(0, 99999999)).as_bytes());
    }
    Ok(Value::str(s))
}

/// `microtime(bool $as_float = false)` — current time with microseconds.
fn fn_microtime(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let as_float = args
        .first()
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    if as_float {
        Ok(Value::Float(now.as_secs_f64()))
    } else {
        // php format: "0.xxxxxxxx sssssssss" (microsecond fraction, then seconds).
        Ok(Value::str(
            format!(
                "{:.8} {}",
                now.subsec_micros() as f64 / 1_000_000.0,
                now.as_secs()
            )
            .into_bytes(),
        ))
    }
}

/// `hrtime(bool $as_number = false)` — high-resolution time. As a monotonic-ish
/// value derived from the wall clock (no separate monotonic source needed here).
fn fn_hrtime(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let as_number = args
        .first()
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    if as_number {
        Ok(Value::Int(now.as_nanos() as i64))
    } else {
        let mut a = PhpArray::new();
        a.push(Value::Int(now.as_secs() as i64));
        a.push(Value::Int(now.subsec_nanos() as i64));
        Ok(Value::Array(a))
    }
}

// --- version_compare ---------------------------------------------------------

/// `php_canonicalize_version` (ext/standard/versioning.c).
fn canonicalize_version(v: &[u8]) -> Vec<u8> {
    if v.is_empty() {
        return Vec::new();
    }
    let isdig = |c: u8| c.is_ascii_digit();
    let isndig = |c: u8| !c.is_ascii_digit() && c != b'.';
    let mut q: Vec<u8> = Vec::with_capacity(v.len() * 2);
    let mut lp = v[0];
    q.push(lp);
    for &c in &v[1..] {
        let lq = *q.last().unwrap();
        if matches!(c, b'-' | b'_' | b'+') {
            if lq != b'.' {
                q.push(b'.');
            }
        } else if (isndig(lp) && isdig(c)) || (isdig(lp) && isndig(c)) {
            if lq != b'.' {
                q.push(b'.');
            }
            q.push(c);
        } else if !c.is_ascii_alphanumeric() {
            if lq != b'.' {
                q.push(b'.');
            }
        } else {
            q.push(c);
        }
        lp = c;
    }
    if q.last() == Some(&b'.') {
        q.pop();
    }
    q
}

fn special_version_form(s: &[u8]) -> i32 {
    const FORMS: [(&[u8], i32); 10] = [
        (b"dev", 0),
        (b"alpha", 1),
        (b"a", 1),
        (b"beta", 2),
        (b"b", 2),
        (b"RC", 3),
        (b"rc", 3),
        (b"#", 4),
        (b"pl", 5),
        (b"p", 5),
    ];
    FORMS
        .iter()
        .find(|(n, _)| s.starts_with(n))
        .map_or(-1, |(_, o)| *o)
}

fn compare_special_version_forms(a: &[u8], b: &[u8]) -> i32 {
    (special_version_form(a) - special_version_form(b)).signum()
}

fn strtol_prefix(s: &[u8]) -> i128 {
    let mut n: i128 = 0;
    for &c in s {
        if !c.is_ascii_digit() {
            break;
        }
        n = (n * 10 + (c - b'0') as i128).min(i64::MAX as i128);
    }
    n
}

/// `php_version_compare` (ext/standard/versioning.c), ported step for step.
fn version_cmp(a: &[u8], b: &[u8]) -> i32 {
    let cut = |s: &[u8]| -> Vec<u8> {
        match s.iter().position(|&c| c == 0) {
            Some(i) => s[..i].to_vec(),
            None => s.to_vec(),
        }
    };
    let (a, b) = (cut(a), cut(b));
    if a.is_empty() || b.is_empty() {
        return if a.is_empty() && b.is_empty() {
            0
        } else if !a.is_empty() {
            1
        } else {
            -1
        };
    }
    let v1 = if a[0] == b'#' {
        a.clone()
    } else {
        canonicalize_version(&a)
    };
    let v2 = if b[0] == b'#' {
        b.clone()
    } else {
        canonicalize_version(&b)
    };
    let at = |v: &[u8], i: usize| v.get(i).copied().unwrap_or(0);
    let find_dot = |v: &[u8], p: usize| {
        v[p.min(v.len())..]
            .iter()
            .position(|&c| c == b'.')
            .map(|i| i + p)
    };
    let (mut p1, mut p2) = (0usize, 0usize);
    let (mut n1, mut n2): (Option<usize>, Option<usize>) = (Some(0), Some(0));
    let mut compare = 0;
    while at(&v1, p1) != 0 && at(&v2, p2) != 0 && n1.is_some() && n2.is_some() {
        n1 = find_dot(&v1, p1);
        n2 = find_dot(&v2, p2);
        let s1 = &v1[p1..n1.unwrap_or(v1.len())];
        let s2 = &v2[p2..n2.unwrap_or(v2.len())];
        let d1 = at(&v1, p1).is_ascii_digit();
        let d2 = at(&v2, p2).is_ascii_digit();
        compare = if d1 && d2 {
            (strtol_prefix(s1) - strtol_prefix(s2)).signum() as i32
        } else if !d1 && !d2 {
            compare_special_version_forms(s1, s2)
        } else if d1 {
            compare_special_version_forms(b"#N#", s2)
        } else {
            compare_special_version_forms(s1, b"#N#")
        };
        if compare != 0 {
            break;
        }
        if let Some(n) = n1 {
            p1 = n + 1;
        }
        if let Some(n) = n2 {
            p2 = n + 1;
        }
    }
    if compare == 0 {
        if n1.is_some() {
            compare = if at(&v1, p1).is_ascii_digit() {
                1
            } else {
                version_cmp(&v1[p1.min(v1.len())..], b"#N#")
            };
        } else if n2.is_some() {
            compare = if at(&v2, p2).is_ascii_digit() {
                -1
            } else {
                version_cmp(b"#N#", &v2[p2.min(v2.len())..])
            };
        }
    }
    compare
}

fn fn_version_compare(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = arg_bytes(args, 0);
    let b = arg_bytes(args, 1);
    let c = version_cmp(&a, &b);
    match args.get(2).map(deref) {
        None | Some(Value::Null) => Ok(Value::Int(c as i64)),
        Some(op) => {
            let op = conv::to_string_bytes(&op);
            let r = match op.as_slice() {
                b"<" | b"lt" => c == -1,
                b"<=" | b"le" => c != 1,
                b">" | b"gt" => c == 1,
                b">=" | b"ge" => c != -1,
                b"==" | b"=" | b"eq" => c == 0,
                b"!=" | b"<>" | b"ne" => c != 0,
                _ => {
                    return Err(Thrown::error(
                        "ValueError",
                        "version_compare(): Argument #3 ($operator) must be a valid comparison operator",
                    ))
                }
            };
            Ok(Value::Bool(r))
        }
    }
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("gethostbyaddr", fn_gethostbyaddr),
            ("ob_start", fn_ob_start),
            ("ob_flush", fn_ob_flush),
            ("ob_clean", fn_ob_clean),
            ("ob_end_flush", fn_ob_end_flush),
            ("ob_end_clean", fn_ob_end_clean),
            ("ob_get_flush", fn_ob_get_flush),
            ("ob_get_clean", fn_ob_get_clean),
            ("ob_get_contents", fn_ob_get_contents),
            ("ob_get_length", fn_ob_get_length),
            ("ob_get_level", fn_ob_get_level),
            ("ob_list_handlers", fn_ob_list_handlers),
            ("ob_get_status", fn_ob_get_status),
            ("ob_implicit_flush", fn_ob_implicit_flush),
            ("flush", fn_flush),
            ("getenv", fn_getenv),
            ("putenv", fn_putenv),
            ("error_get_last", fn_error_get_last),
            ("error_clear_last", fn_error_clear_last),
            ("register_shutdown_function", fn_register_shutdown_function),
            ("set_time_limit", fn_set_time_limit),
            ("getmypid", fn_getmypid),
            ("assert", fn_assert),
            ("random_bytes", fn_random_bytes),
            ("random_int", fn_random_int),
            ("mt_getrandmax", fn_getrandmax),
            ("getrandmax", fn_getrandmax),
            ("uniqid", fn_uniqid),
            ("microtime", fn_microtime),
            ("hrtime", fn_hrtime),
            ("version_compare", fn_version_compare),
        ],
    );
}
