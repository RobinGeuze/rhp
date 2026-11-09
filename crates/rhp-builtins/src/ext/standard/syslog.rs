//! ext/standard/syslog.c: `openlog`, `syslog`, `closelog` and the `LOG_*`
//! constants (the C library's values), with main/php_syslog.c's message
//! filter.

use std::ffi::CString;
use std::sync::Mutex;

use rhp_values::{conv, ops::deref, Value};

use crate::{Host, Thrown};

/// The ident passed to openlog(3), which keeps the pointer (php's
/// BG(syslog_device)). syslog is process state, so this is too.
static IDENT: Mutex<Option<CString>> = Mutex::new(None);

/// php's PG(have_called_openlog): openlog was called (by openlog() or by
/// [`php_syslog`]) and closelog() not since.
static OPENED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    use libc as l;
    let items: [(&'static str, libc::c_int); 33] = [
        ("LOG_EMERG", l::LOG_EMERG),
        ("LOG_ALERT", l::LOG_ALERT),
        ("LOG_CRIT", l::LOG_CRIT),
        ("LOG_ERR", l::LOG_ERR),
        ("LOG_WARNING", l::LOG_WARNING),
        ("LOG_NOTICE", l::LOG_NOTICE),
        ("LOG_INFO", l::LOG_INFO),
        ("LOG_DEBUG", l::LOG_DEBUG),
        ("LOG_KERN", l::LOG_KERN),
        ("LOG_USER", l::LOG_USER),
        ("LOG_MAIL", l::LOG_MAIL),
        ("LOG_DAEMON", l::LOG_DAEMON),
        ("LOG_AUTH", l::LOG_AUTH),
        ("LOG_SYSLOG", l::LOG_SYSLOG),
        ("LOG_LPR", l::LOG_LPR),
        ("LOG_NEWS", l::LOG_NEWS),
        ("LOG_UUCP", l::LOG_UUCP),
        ("LOG_CRON", l::LOG_CRON),
        ("LOG_AUTHPRIV", l::LOG_AUTHPRIV),
        ("LOG_LOCAL0", l::LOG_LOCAL0),
        ("LOG_LOCAL1", l::LOG_LOCAL1),
        ("LOG_LOCAL2", l::LOG_LOCAL2),
        ("LOG_LOCAL3", l::LOG_LOCAL3),
        ("LOG_LOCAL4", l::LOG_LOCAL4),
        ("LOG_LOCAL5", l::LOG_LOCAL5),
        ("LOG_LOCAL6", l::LOG_LOCAL6),
        ("LOG_LOCAL7", l::LOG_LOCAL7),
        ("LOG_PID", l::LOG_PID),
        ("LOG_CONS", l::LOG_CONS),
        ("LOG_ODELAY", l::LOG_ODELAY),
        ("LOG_NDELAY", l::LOG_NDELAY),
        ("LOG_NOWAIT", l::LOG_NOWAIT),
        ("LOG_PERROR", l::LOG_PERROR),
    ];
    items
        .into_iter()
        .map(|(n, v)| (n, Value::Int(v as i64)))
        .collect()
}

/// One syslog(3) call with the bytes as they are.
fn send(priority: i64, msg: &[u8]) {
    // SAFETY: "%.*s" reads exactly `len` bytes from `msg`, which outlives
    // the call; the format is a static NUL-terminated string.
    unsafe {
        libc::syslog(
            priority as libc::c_int,
            c"%.*s".as_ptr(),
            msg.len() as libc::c_int,
            msg.as_ptr() as *const libc::c_char,
        );
    }
}

/// php_syslog_str with the default `syslog.filter` (no-ctrl): printable
/// ASCII and high bytes pass, a newline starts another message, other
/// control bytes become `\xNN`.
pub(crate) fn php_syslog_str(priority: i64, msg: &[u8]) {
    syslog_filtered(priority, msg, b"no-ctrl");
}

/// main/php_syslog.c's php_syslog_str under `syslog.filter` `filter`
/// (`all`, `no-ctrl`, `ascii`, `raw`).
fn syslog_filtered(priority: i64, msg: &[u8], filter: &[u8]) {
    if filter == b"raw" {
        let end = msg.iter().position(|&b| b == 0).unwrap_or(msg.len());
        send(priority, &msg[..end]);
        return;
    }
    let mut buf = Vec::with_capacity(msg.len());
    for &c in msg {
        match c {
            0x20..=0x7e => buf.push(c),
            0x80..=0xff if filter != b"ascii" => buf.push(c),
            0..=0x1f if c != b'\n' && filter == b"all" => buf.push(c),
            b'\n' => {
                send(priority, &buf);
                buf.clear();
            }
            _ => {
                const X: &[u8; 16] = b"0123456789abcdef";
                buf.extend_from_slice(&[b'\\', b'x', X[(c >> 4) as usize], X[(c & 15) as usize]]);
            }
        }
    }
    send(priority, &buf);
}

fn fn_openlog(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ident = conv::to_string_bytes(&deref(&args[0]));
    let option = conv::to_int(&deref(&args[1]));
    let facility = conv::to_int(&deref(&args[2]));
    // Like strndup, the ident ends at its first NUL.
    let end = ident.iter().position(|&b| b == 0).unwrap_or(ident.len());
    let ident = CString::new(&ident[..end]).expect("no NUL");
    let mut g = IDENT.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the ident is kept in IDENT until the next openlog/closelog,
    // which replace it only after syslog has been given the new one.
    unsafe {
        libc::openlog(
            ident.as_ptr(),
            option as libc::c_int,
            facility as libc::c_int,
        )
    };
    *g = Some(ident);
    OPENED.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(Value::Bool(true))
}

fn fn_closelog(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut g = IDENT.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: closelog takes no arguments; the ident is freed after it.
    unsafe { libc::closelog() };
    *g = None;
    OPENED.store(false, std::sync::atomic::Ordering::Relaxed);
    Ok(Value::Bool(true))
}

fn fn_syslog(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let priority = conv::to_int(&deref(&args[0]));
    let msg = conv::to_string_bytes(&deref(&args[1]));
    let _g = IDENT.lock().unwrap_or_else(|e| e.into_inner());
    php_syslog_str(priority, &msg);
    Ok(Value::Bool(true))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("openlog", fn_openlog),
            ("closelog", fn_closelog),
            ("syslog", fn_syslog),
        ],
    );
}

/// main.c's OnSetFacility: a `syslog.facility` value as a facility.
fn facility(v: &[u8]) -> Option<libc::c_int> {
    use libc as l;
    let names: &[(&[u8], &[u8], libc::c_int)] = &[
        (b"LOG_AUTH", b"auth", l::LOG_AUTH),
        (b"LOG_AUTH", b"security", l::LOG_AUTH),
        (b"LOG_AUTHPRIV", b"authpriv", l::LOG_AUTHPRIV),
        (b"LOG_CRON", b"cron", l::LOG_CRON),
        (b"LOG_DAEMON", b"daemon", l::LOG_DAEMON),
        (b"LOG_FTP", b"ftp", l::LOG_FTP),
        (b"LOG_KERN", b"kern", l::LOG_KERN),
        (b"LOG_LPR", b"lpr", l::LOG_LPR),
        (b"LOG_MAIL", b"mail", l::LOG_MAIL),
        (b"LOG_NEWS", b"news", l::LOG_NEWS),
        (b"LOG_SYSLOG", b"syslog", l::LOG_SYSLOG),
        (b"LOG_USER", b"user", l::LOG_USER),
        (b"LOG_UUCP", b"uucp", l::LOG_UUCP),
        (b"LOG_LOCAL0", b"local0", l::LOG_LOCAL0),
        (b"LOG_LOCAL1", b"local1", l::LOG_LOCAL1),
        (b"LOG_LOCAL2", b"local2", l::LOG_LOCAL2),
        (b"LOG_LOCAL3", b"local3", l::LOG_LOCAL3),
        (b"LOG_LOCAL4", b"local4", l::LOG_LOCAL4),
        (b"LOG_LOCAL5", b"local5", l::LOG_LOCAL5),
        (b"LOG_LOCAL6", b"local6", l::LOG_LOCAL6),
        (b"LOG_LOCAL7", b"local7", l::LOG_LOCAL7),
    ];
    names
        .iter()
        .find(|(a, b, _)| v == *a || v == *b)
        .map(|&(_, _, f)| f)
}

/// main/php_syslog.c's php_syslog: `error_log = syslog`. Opens the log
/// with `syslog.ident` / `syslog.facility` unless openlog() was called,
/// then sends `msg` through `syslog.filter`.
pub fn php_syslog(priority: i64, msg: &[u8]) {
    let ini = |n: &[u8]| crate::main::ini::ini_value(n).flatten().unwrap_or_default();
    let mut g = IDENT.lock().unwrap_or_else(|e| e.into_inner());
    if !OPENED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        let ident = ini(b"syslog.ident");
        let end = ident.iter().position(|&b| b == 0).unwrap_or(ident.len());
        let ident = CString::new(&ident[..end]).expect("no NUL");
        let fac = facility(&ini(b"syslog.facility")).unwrap_or(libc::LOG_USER);
        // SAFETY: the ident is kept in IDENT, as for openlog().
        unsafe { libc::openlog(ident.as_ptr(), 0, fac) };
        *g = Some(ident);
    }
    syslog_filtered(priority, msg, &ini(b"syslog.filter"));
}

/// php_error_cb's syslog priority for an error of type `ty`.
pub fn error_priority(ty: i64) -> i64 {
    use crate::*;
    let p = match ty {
        E_ERROR | E_CORE_ERROR | E_COMPILE_ERROR | E_USER_ERROR | E_RECOVERABLE_ERROR | E_PARSE => {
            libc::LOG_ERR
        }
        E_WARNING | E_CORE_WARNING | E_COMPILE_WARNING | E_USER_WARNING => libc::LOG_WARNING,
        E_DEPRECATED | E_USER_DEPRECATED => libc::LOG_INFO,
        _ => libc::LOG_NOTICE,
    };
    p as i64
}
