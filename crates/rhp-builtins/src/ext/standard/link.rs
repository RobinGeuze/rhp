//! ext/standard/link.c: `link`, `linkinfo`, `readlink` and `symlink`.

use rhp_values::Value;

use crate::ext::standard::file::{os_text, path_arg, path_param};
use crate::main::streams::split_scheme;
use crate::{Host, Thrown};

// --- hard links -----------------------------------------------------------------------

/// `link($target, $link)`: a hard link; plain paths only.
fn fn_link(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let target = path_param(args, 0, "link", "target")?;
    let link = path_param(args, 1, "link", "link")?;
    if target.is_empty() || link.is_empty() {
        return Ok(Value::Bool(false));
    }
    let is_url = |p: &[u8]| split_scheme(p).0 != b"file" || p.starts_with(b"file://");
    if is_url(&target) || is_url(&link) {
        return Ok(Value::Bool(false));
    }
    let (Some(t), Some(l)) = (
        crate::zend::virtual_cwd::c_path(&target),
        crate::zend::virtual_cwd::c_path(&link),
    ) else {
        return Ok(Value::Bool(false));
    };
    if unsafe { libc::link(t.as_ptr(), l.as_ptr()) } != 0 {
        let e = std::io::Error::last_os_error();
        let text = crate::main::streams::strerror(e.raw_os_error().unwrap_or(0));
        crate::warn(host, "link", &text)?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

/// `linkinfo($path)`: `st_dev` of the link itself, or -1.
fn fn_linkinfo(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "linkinfo", "path")?;
    Ok(Value::Int(
        crate::main::streams::sys_stat(&path, true)
            .map(|s| s.dev)
            .unwrap_or(-1),
    ))
}

fn fn_readlink(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match std::fs::read_link(crate::zend::virtual_cwd::os_path(&path_arg(args, 0))) {
        Ok(p) => Ok(Value::str(p.to_string_lossy().into_owned().into_bytes())),
        Err(e) => {
            crate::warn(host, "readlink", &os_text(&e))?;
            Ok(Value::Bool(false))
        }
    }
}

/// `symlink()`: the target is stored as given (it is relative to the
/// link), the link is made from the working directory.
fn fn_symlink(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    use std::os::unix::ffi::OsStrExt;
    let target = path_arg(args, 0);
    let link = path_arg(args, 1);
    let target = std::ffi::OsStr::from_bytes(&target);
    match std::os::unix::fs::symlink(target, crate::zend::virtual_cwd::os_path(&link)) {
        Ok(()) => Ok(Value::Bool(true)),
        Err(e) => {
            crate::warn(host, "symlink", &os_text(&e))?;
            Ok(Value::Bool(false))
        }
    }
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("link", fn_link),
            ("linkinfo", fn_linkinfo),
            ("readlink", fn_readlink),
            ("symlink", fn_symlink),
        ],
    );
}
