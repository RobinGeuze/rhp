//! php_error_docref-style diagnostics raised from builtins. Messages are
//! bytes, as php's are: a file name or argument quoted in one goes out as
//! given, never UTF-8-checked.

use crate::main::streams;
use crate::main::streams::plain_wrapper;
use crate::{Host, Thrown, E_DEPRECATED, E_NOTICE, E_WARNING};

/// `fname(): msg`, php_error_docref's prefix.
fn docref(fname: &str, msg: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(fname.len() + 4 + msg.len());
    m.extend_from_slice(fname.as_bytes());
    m.extend_from_slice(b"(): ");
    m.extend_from_slice(msg);
    m
}

/// `php_error_docref(NULL, E_WARNING, msg)` from builtin `fname`.
pub fn warn(
    host: &mut dyn Host,
    fname: &str,
    msg: &(impl AsRef<[u8]> + ?Sized),
) -> Result<(), Thrown> {
    host.emit_diagnostic(E_WARNING, &docref(fname, msg.as_ref()))
}

/// Report the diagnostics stream operations queued (see
/// [`plain_wrapper::Diag`]) as raised by builtin `fname`.
pub fn flush_stream_diags(host: &mut dyn Host, fname: &str) -> Result<(), Thrown> {
    for d in plain_wrapper::take_diags() {
        if d.docref {
            host.emit_diagnostic(d.level, &docref(fname, d.msg.as_bytes()))?;
        } else {
            host.emit_diagnostic(d.level, d.msg.as_bytes())?;
        }
    }
    Ok(())
}

/// `php_error_docref1(NULL, arg, E_WARNING, msg)`: `fname(arg): msg`.
pub fn warn_arg(
    host: &mut dyn Host,
    fname: &str,
    arg: &[u8],
    msg: &(impl AsRef<[u8]> + ?Sized),
) -> Result<(), Thrown> {
    let msg = msg.as_ref();
    let mut m = Vec::with_capacity(fname.len() + arg.len() + 4 + msg.len());
    m.extend_from_slice(fname.as_bytes());
    m.push(b'(');
    m.extend_from_slice(arg);
    m.extend_from_slice(b"): ");
    m.extend_from_slice(msg);
    host.emit_diagnostic(E_WARNING, &m)
}

thread_local! {
    /// [`locate_url`] already warned about an unknown wrapper for the open
    /// that may now fail.
    static UNKNOWN_WRAPPER_WARNED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether `scheme` names a registered wrapper (as written: php's hash is
/// case-sensitive, the lowercase retry is the caller's).
pub(crate) fn wrapper_registered(host: &dyn Host, scheme: &[u8]) -> bool {
    host.stream_wrapper_schemes().iter().any(|s| s == scheme)
}

/// `php_stream_locate_url_wrapper(path)` from builtin `fname`, with its
/// "Unable to find the wrapper" warning.
pub fn locate_url(
    host: &mut dyn Host,
    fname: &str,
    path: &[u8],
) -> Result<streams::Located, Thrown> {
    UNKNOWN_WRAPPER_WARNED.with(|w| w.set(false));
    let located = streams::locate_url_wrapper(path, |s| wrapper_registered(host, s));
    if let streams::Located::Unknown(name) = &located {
        let mut m = b"Unable to find the wrapper \"".to_vec();
        m.extend_from_slice(name);
        m.extend_from_slice(b"\" - did you forget to enable it when you configured PHP?");
        warn(host, fname, &m)?;
        UNKNOWN_WRAPPER_WARNED.with(|w| w.set(true));
    }
    Ok(located)
}

/// The wrapper lookup of `php_stream_open_wrapper_ex(path, REPORT_ERRORS)`
/// from builtin `fname`: the path the stream layer should open (a
/// `file://` URL as its local path, an unknown wrapper's path as the plain
/// file php falls back to), or `None` after php's warnings when no wrapper
/// fits.
pub fn locate_for_open(
    host: &mut dyn Host,
    fname: &str,
    path: &[u8],
) -> Result<Option<Vec<u8>>, Thrown> {
    let located = locate_url(host, fname, path)?;
    if matches!(
        located,
        streams::Located::Plain(_) | streams::Located::Unknown(_)
    ) && !wrapper_registered(host, b"file")
    {
        // php falls back on the plain-files wrapper, which is unregistered
        UNKNOWN_WRAPPER_WARNED.with(|w| w.set(false));
        warn(
            host,
            fname,
            "file:// wrapper is disabled in the server configuration",
        )?;
        warn_arg(
            host,
            fname,
            path,
            "Failed to open stream: no suitable wrapper could be found",
        )?;
        return Ok(None);
    }
    Ok(match located {
        // the stream layer takes a file:// prefix off itself
        streams::Located::Plain(_) => Some(path.to_vec()),
        streams::Located::Wrapper(p) => Some(p),
        streams::Located::Unknown(_) => Some(crate::zend::virtual_cwd::expand_local(path)),
        streams::Located::RemoteHost => {
            let mut m = b"Remote host file access not supported, ".to_vec();
            m.extend_from_slice(path);
            warn(host, fname, &m)?;
            warn_arg(
                host,
                fname,
                path,
                "Failed to open stream: no suitable wrapper could be found",
            )?;
            None
        }
    })
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
    if UNKNOWN_WRAPPER_WARNED.with(|w| w.replace(false)) {
        // locate_url warned; the plain open's errno is the reason
        return warn_arg(
            host,
            fname,
            path,
            &format!("Failed to open stream: {}", streams::last_strerror()),
        );
    }
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
        let mut m = b"Unable to find the wrapper \"".to_vec();
        m.extend_from_slice(&scheme);
        m.extend_from_slice(b"\" - did you forget to enable it when you configured PHP?");
        warn(host, fname, &m)?;
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
pub fn notice(
    host: &mut dyn Host,
    fname: &str,
    msg: &(impl AsRef<[u8]> + ?Sized),
) -> Result<(), Thrown> {
    host.emit_diagnostic(E_NOTICE, &docref(fname, msg.as_ref()))
}

/// zend_deprecated_function: the notice for calling `#[\Deprecated]`
/// function `fname`, from its stub's `since` and `message`. The VM raises
/// it for a builtin call; a builtin a prelude wrapper reaches raises it
/// itself.
pub fn deprecated_function(host: &mut dyn Host, fname: &str) -> Result<(), Thrown> {
    let f = rhp_bytecode::signatures::function(fname.as_bytes());
    let (since, message) = f
        .and_then(rhp_bytecode::signatures::deprecation)
        .unwrap_or_default();
    let mut msg = format!("Function {fname}() is deprecated");
    if !since.is_empty() {
        msg.push_str(" since ");
        msg.push_str(since);
    }
    if !message.is_empty() {
        msg.push_str(", ");
        msg.push_str(message);
    }
    host.emit_diagnostic(E_DEPRECATED, msg.as_bytes())
}

/// `php_error_docref(NULL, E_DEPRECATED, msg)` from builtin `fname`.
pub fn deprecated(
    host: &mut dyn Host,
    fname: &str,
    msg: &(impl AsRef<[u8]> + ?Sized),
) -> Result<(), Thrown> {
    host.emit_diagnostic(E_DEPRECATED, &docref(fname, msg.as_ref()))
}
