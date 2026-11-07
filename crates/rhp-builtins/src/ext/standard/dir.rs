//! ext/standard/dir.c: directory streams (the `opendir` family and the
//! `Directory` class), `scandir`, `glob`, `getcwd` and `chdir`.

use rhp_values::ops::deref;
use rhp_values::{PhpArray, Value};

use crate::ext::standard::file::{int_arg, path_param, str_arg, touch_ctx};
use crate::main::php_glob::{php_glob, GLOB_AVAILABLE, GLOB_NOMATCH, GLOB_ONLYDIR};
use crate::{Host, StreamKind, Thrown};

// --- directory streams -------------------------------------------------------

fn do_opendir(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
) -> Result<Option<(Vec<u8>, Value)>, Thrown> {
    let path = path_param(args, 0, fname, "directory")?;
    touch_ctx(host, args, 1);
    crate::main::streams::clear_errno();
    if let Some((w, p)) = crate::main::streams::userspace::locate(host, &path) {
        let ctx = match args.get(1).map(deref) {
            Some(v @ Value::Resource(_)) => v,
            _ => host.default_stream_context(),
        };
        // _php_stream_opendir leaves orig_path unset (no "uri" in the meta data)
        let Some(s) = crate::main::streams::userspace::opendir(host, &w, &p, 0, Some(&ctx)) else {
            user_dir_failed(host, fname, &path)?;
            return Ok(None);
        };
        let h = host.user_stream_insert(s, Some(ctx));
        return Ok(Some((path, h)));
    }
    if !dir_allowed(host, fname, &path)? {
        dir_open_failed(host, fname, &path)?;
        return Ok(None);
    }
    let Some(entries) = host.stream_opendir(&path) else {
        dir_open_failed(host, fname, &path)?;
        return Ok(None);
    };
    Ok(host.dir_open(&path, entries).map(|h| (path, h)))
}

/// The plain-files dir opener's open_basedir check (it sets errno).
fn dir_allowed(host: &mut dyn Host, fname: &str, path: &[u8]) -> Result<bool, Thrown> {
    if !crate::main::fopen_wrappers::active() {
        return Ok(true);
    }
    let local = match crate::main::streams::locate_url_wrapper(path, |s| {
        crate::diag::wrapper_registered(host, s)
    }) {
        crate::main::streams::Located::Plain(_) => crate::main::streams::file_path(path),
        crate::main::streams::Located::Unknown(_) => path,
        _ => return Ok(true),
    };
    crate::main::fopen_wrappers::check(host, fname, local, true)
}

/// A user wrapper's dir open failed: its queued error as the reason.
fn user_dir_failed(host: &mut dyn Host, fname: &str, path: &[u8]) -> Result<(), Thrown> {
    if host.has_pending_throw() {
        crate::main::streams::take_wrapper_error();
        return Ok(());
    }
    let reason = crate::main::streams::take_wrapper_error()
        .unwrap_or_else(|| "operation failed".to_string());
    crate::warn_arg(
        host,
        fname,
        path,
        &format!("Failed to open directory: {reason}"),
    )
}

/// "fname(path): Failed to open directory: <strerror>".
fn dir_open_failed(host: &mut dyn Host, fname: &str, path: &[u8]) -> Result<(), Thrown> {
    let reason = match crate::main::streams::last_errno() {
        Some(c) => crate::main::streams::strerror(c),
        None => "operation failed".into(),
    };
    crate::warn_arg(
        host,
        fname,
        path,
        &format!("Failed to open directory: {reason}"),
    )
}

fn fn_opendir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(do_opendir(host, args, "opendir")?
        .map(|(_, h)| h)
        .unwrap_or(Value::Bool(false)))
}

fn fn_dir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some((path, h)) = do_opendir(host, args, "dir")? else {
        return Ok(Value::Bool(false));
    };
    Ok(host
        .instantiate(
            b"Directory",
            vec![
                (b"path".to_vec(), Value::str(path)),
                (b"handle".to_vec(), h),
            ],
        )
        .unwrap_or(Value::Bool(false)))
}

/// Resolve the `?resource $dir_handle` argument of readdir/rewinddir/closedir.
fn dir_param(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let v = args.first().map(deref).unwrap_or(Value::Null);
    let v = match v {
        Value::Null => {
            crate::deprecated(
                host,
                fname,
                "Passing null is deprecated, instead the last opened directory stream should be provided",
            )?;
            match host.last_dir() {
                Some(d) => d,
                None => return Err(Thrown::type_error("No resource supplied")),
            }
        }
        Value::Resource(_) => v,
        other => {
            return Err(Thrown::type_error(format!(
                "{fname}(): Argument #1 ($dir_handle) must be of type resource or null, {} given",
                crate::zend::zpp::zval_value_name(host, &other)
            )))
        }
    };
    match host.stream_kind(&v) {
        Some(StreamKind::Dir) => Ok(v),
        Some(_) => Err(Thrown::type_error(format!(
            "{fname}(): Argument #1 ($dir_handle) must be a valid Directory resource"
        ))),
        None => Err(Thrown::type_error(format!(
            "{fname}(): Argument #1 ($dir_handle) must be an open stream resource"
        ))),
    }
}

fn fn_readdir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = dir_param(host, args, "readdir")?;
    Ok(host
        .dir_read(&h)
        .map(Value::str)
        .unwrap_or(Value::Bool(false)))
}

fn fn_rewinddir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = dir_param(host, args, "rewinddir")?;
    host.dir_rewind(&h);
    Ok(Value::Null)
}

fn fn_closedir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = dir_param(host, args, "closedir")?;
    host.stream_close(&h);
    Ok(Value::Null)
}

/// `__directory_handle($dir)`: the Directory's handle, null while it is
/// uninitialized (php reads the property slot without the typed-property
/// check, so the op reports an altered stream).
fn fn_directory_handle(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let d = args.first().map(deref).unwrap_or(Value::Null);
    Ok(host
        .object_prop(&d, b"handle")
        .map(|v| deref(&v))
        .unwrap_or(Value::Null))
}

/// `__directory_op($handle, $op)` — backs `Directory::read/rewind/close`.
fn fn_directory_op(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = args.first().map(deref).unwrap_or(Value::Null);
    let op = str_arg(args, 1);
    let opname = String::from_utf8_lossy(&op).into_owned();
    if !matches!(h, Value::Resource(_)) {
        return Err(Thrown::error(
            "Error",
            "Internal directory stream has been altered",
        ));
    }
    if host.stream_kind(&h) != Some(StreamKind::Dir) {
        return Err(Thrown::type_error(format!(
            "Directory::{opname}(): cannot use Directory resource after it has been closed"
        )));
    }
    match op.as_slice() {
        b"read" => Ok(host
            .dir_read(&h)
            .map(Value::str)
            .unwrap_or(Value::Bool(false))),
        b"rewind" => {
            host.dir_rewind(&h);
            Ok(Value::Null)
        }
        _ => {
            host.stream_close(&h);
            Ok(Value::Null)
        }
    }
}

pub const PRELUDE_PHP: &str = include_str!("dir/prelude.php");

/// dir.c's `DIRECTORY_SEPARATOR` and `PATH_SEPARATOR`.
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("DIRECTORY_SEPARATOR", Value::str("/")),
        ("PATH_SEPARATOR", Value::str(":")),
    ]
}

fn fn_scandir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "scandir", "directory")?;
    if path.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "scandir(): Argument #1 ($directory) must not be empty",
        ));
    }
    let flags = int_arg(args, 1, 0);
    crate::main::streams::clear_errno();
    let user = crate::main::streams::userspace::locate(host, &path);
    let names = if let Some((w, p)) = user {
        // scandir takes a context only when one is given
        let ctx = match args.get(2).map(deref) {
            Some(v @ Value::Resource(_)) => Some(v),
            _ => None,
        };
        match crate::main::streams::userspace::opendir(host, &w, &p, 0, ctx.as_ref()) {
            Some(mut s) => {
                let mut names = Vec::new();
                while let Some(n) = s.readdir(host) {
                    names.push(n);
                }
                s.close(host);
                Some(names)
            }
            None => {
                user_dir_failed(host, "scandir", &path)?;
                crate::warn(host, "scandir", "(errno 0): Success")?;
                return Ok(Value::Bool(false));
            }
        }
    } else if dir_allowed(host, "scandir", &path)? {
        host.stream_opendir(&path)
    } else {
        None
    };
    let Some(mut names) = names else {
        let errno = crate::main::streams::last_errno();
        dir_open_failed(host, "scandir", &path)?;
        let (code, text) = match errno {
            Some(c) => (c, crate::main::streams::strerror(c)),
            None => (0, "operation failed".into()),
        };
        crate::warn(host, "scandir", &format!("(errno {code}): {text}"))?;
        return Ok(Value::Bool(false));
    };
    host.consume_resource_ids(1);
    if flags == 0 {
        names.sort();
    } else if flags != 2 {
        names.sort_by(|a, b| b.cmp(a));
    }
    let mut a = PhpArray::new();
    for n in names {
        a.push(Value::str(n));
    }
    Ok(Value::Array(a))
}

fn fn_glob(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let pattern = path_param(args, 0, "glob", "pattern")?;
    let flags = int_arg(args, 1, 0);
    if pattern.len() >= 1024 {
        return Ok(Value::Bool(false));
    }
    if (GLOB_AVAILABLE & flags) != flags {
        return Ok(Value::Bool(false));
    }
    let (ret, paths) = php_glob(&pattern, flags);
    if ret != 0 && ret != GLOB_NOMATCH {
        return Ok(Value::Bool(false));
    }
    let mut a = PhpArray::new();
    let ob = crate::main::fopen_wrappers::active();
    let mut basedir_limit = false;
    for p in paths {
        if ob && !crate::main::fopen_wrappers::check(host, "glob", &p, false)? {
            basedir_limit = true;
            continue;
        }
        if flags & GLOB_ONLYDIR != 0 {
            match crate::main::streams::sys_stat(&p, false) {
                Some(s) if s.is_dir => {}
                _ => continue,
            }
        }
        a.push(Value::str(p));
    }
    if basedir_limit && a.is_empty() {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Array(a))
}

fn fn_getcwd(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(crate::zend::virtual_cwd::current_dir().map_or(Value::Bool(false), Value::str))
}

/// `chdir()`: the request's working directory (see [`crate::zend::virtual_cwd`]).
fn fn_chdir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let dir = crate::zend::zpp::path_param(host, args, 0, "chdir", "directory")?;
    if !crate::main::fopen_wrappers::check(host, "chdir", &dir, true)? {
        return Ok(Value::Bool(false));
    }
    match crate::zend::virtual_cwd::chdir(&dir) {
        Ok(()) => Ok(Value::Bool(true)),
        Err(e) => {
            let msg = format!("{} (errno {e})", crate::main::streams::strerror(e));
            crate::warn(host, "chdir", &msg)?;
            Ok(Value::Bool(false))
        }
    }
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("opendir", fn_opendir),
            ("dir", fn_dir),
            ("readdir", fn_readdir),
            ("rewinddir", fn_rewinddir),
            ("closedir", fn_closedir),
            ("scandir", fn_scandir),
            ("glob", fn_glob),
            ("getcwd", fn_getcwd),
            ("chdir", fn_chdir),
        ],
    );
    crate::register_helpers(
        m,
        &[
            ("__directory_op", fn_directory_op, 2, Some(2)),
            ("__directory_handle", fn_directory_handle, 1, Some(1)),
        ],
    );
}
