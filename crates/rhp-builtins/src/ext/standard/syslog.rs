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
    let mut buf = Vec::with_capacity(msg.len());
    for &c in msg {
        match c {
            0x20..=0x7e | 0x80..=0xff => buf.push(c),
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
    Ok(Value::Bool(true))
}

fn fn_closelog(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut g = IDENT.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: closelog takes no arguments; the ident is freed after it.
    unsafe { libc::closelog() };
    *g = None;
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
