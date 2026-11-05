//! ext/standard/link.c: `link`, `linkinfo`, `readlink` and `symlink`.

use rhp_values::Value;

use crate::ext::standard::file::{os_text, path_param};
use crate::{Host, Thrown};

// --- hard links -----------------------------------------------------------------------

/// `link($target, $link)`: a hard link.
fn fn_link(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let target = path_param(args, 0, "link", "target")?;
    let link = path_param(args, 1, "link", "link")?;
    // expand_filepath fails on an empty path
    if target.is_empty() || link.is_empty() {
        crate::warn(host, "link", "No such file or directory")?;
        return Ok(Value::Bool(false));
    }
    let (Some(t), Some(l)) = (
        crate::zend::virtual_cwd::c_path(&target),
        crate::zend::virtual_cwd::c_path(&link),
    ) else {
        return Ok(Value::Bool(false));
    };
    if unsafe { libc::link(t.as_ptr(), l.as_ptr()) } != 0 {
        let text = crate::main::streams::strerror(crate::main::streams::errno());
        crate::warn(host, "link", &text)?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

/// `linkinfo($path)`: `st_dev` of the link itself, or -1 with a warning.
fn fn_linkinfo(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "linkinfo", "path")?;
    match crate::zend::virtual_cwd::stat(&path, false) {
        Ok(st) => Ok(Value::Int(
            crate::main::streams::StatInfo::from_libc(&st).dev,
        )),
        Err(e) => {
            crate::warn(host, "linkinfo", &os_text(&e))?;
            Ok(Value::Int(-1))
        }
    }
}

fn fn_readlink(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_param(args, 0, "readlink", "path")?;
    match std::fs::read_link(crate::zend::virtual_cwd::os_path(&path)) {
        Ok(p) => Ok(Value::str(p.into_os_string().into_encoded_bytes())),
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
    let target = path_param(args, 0, "symlink", "target")?;
    let link = path_param(args, 1, "symlink", "link")?;
    // expand_filepath of either path fails when it is empty
    if target.is_empty() || link.is_empty() {
        crate::warn(host, "symlink", "No such file or directory")?;
        return Ok(Value::Bool(false));
    }
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
