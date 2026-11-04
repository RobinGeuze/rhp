//! php_error_docref-style diagnostics raised from builtins.

use crate::{fdstream, streams, Host, Thrown, E_DEPRECATED, E_NOTICE, E_WARNING};

/// `php_error_docref(NULL, E_WARNING, msg)` from builtin `fname`.
pub fn warn(host: &mut dyn Host, fname: &str, msg: &str) -> Result<(), Thrown> {
    host.emit_diagnostic(E_WARNING, &format!("{fname}(): {msg}"))
}

/// Report the diagnostics stream operations queued (see
/// [`fdstream::Diag`]) as raised by builtin `fname`.
pub fn flush_stream_diags(host: &mut dyn Host, fname: &str) -> Result<(), Thrown> {
    for d in fdstream::take_diags() {
        if d.docref {
            host.emit_diagnostic(d.level, &format!("{fname}(): {}", d.msg))?;
        } else {
            host.emit_diagnostic(d.level, &d.msg)?;
        }
    }
    Ok(())
}

/// `php_error_docref1(NULL, arg, E_WARNING, msg)`: `fname(arg): msg`.
pub fn warn_arg(host: &mut dyn Host, fname: &str, arg: &[u8], msg: &str) -> Result<(), Thrown> {
    host.emit_diagnostic(
        E_WARNING,
        &format!("{fname}({}): {msg}", String::from_utf8_lossy(arg)),
    )
}

/// php_stream_open_wrapper's failure report for `path` from builtin `fname`:
/// an unknown wrapper / bad php:// URL first, then "Failed to open stream:"
/// with the errno the failing syscall recorded (see [`streams::note_errno`]).
pub fn open_failed(host: &mut dyn Host, fname: &str, path: &[u8]) -> Result<(), Thrown> {
    let scheme = streams::split_scheme(path).0.to_ascii_lowercase();
    // Only the plain-files wrapper reports errno; others say "operation failed".
    let mut reason = if scheme == b"file" {
        streams::last_strerror()
    } else {
        "operation failed".to_string()
    };
    let known = host.stream_wrapper_schemes();
    if let Some(msg) = streams::take_wrapper_error() {
        return warn_arg(host, fname, path, &format!("Failed to open stream: {msg}"));
    }
    if scheme == b"php" {
        warn(host, fname, "Invalid php:// URL specified")?;
        reason = "operation failed".into();
    } else if scheme != b"file"
        && !matches!(&scheme[..], b"http" | b"https" | b"data" | b"glob")
        && !known.iter().any(|k| k.eq_ignore_ascii_case(&scheme))
    {
        let name = String::from_utf8_lossy(&scheme).into_owned();
        warn(
            host,
            fname,
            &format!(
                "Unable to find the wrapper \"{name}\" - did you forget to enable it when you configured PHP?"
            ),
        )?;
        reason = streams::strerror(libc::ENOENT);
    }
    warn_arg(
        host,
        fname,
        path,
        &format!("Failed to open stream: {reason}"),
    )
}

/// `php_error_docref(NULL, E_NOTICE, msg)` from builtin `fname`.
pub fn notice(host: &mut dyn Host, fname: &str, msg: &str) -> Result<(), Thrown> {
    host.emit_diagnostic(E_NOTICE, &format!("{fname}(): {msg}"))
}

/// `php_error_docref(NULL, E_DEPRECATED, msg)` from builtin `fname`.
pub fn deprecated(host: &mut dyn Host, fname: &str, msg: &str) -> Result<(), Thrown> {
    host.emit_diagnostic(E_DEPRECATED, &format!("{fname}(): {msg}"))
}

/// The `#[\Deprecated]` notice php raises on calling a deprecated function:
/// `Function f() is deprecated since V, MSG`.
pub fn deprecated_fn(
    host: &mut dyn Host,
    fname: &str,
    since: &str,
    msg: &str,
) -> Result<(), Thrown> {
    host.emit_diagnostic(
        E_DEPRECATED,
        &format!("Function {fname}() is deprecated since {since}, {msg}"),
    )
}
