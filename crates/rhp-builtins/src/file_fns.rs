//! Filesystem & path builtins (php `ext/standard` file functions): existence
//! checks, `basename`/`dirname`, `realpath`/`getcwd`, and whole-file read/write.
//! Local paths only (no stream wrappers / URLs yet).

use std::collections::HashMap;

use rhp_values::{conv, Value};

use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn path_arg(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&deref(&args[i]))
}

fn as_str(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

fn fn_file_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.stream_exists(&path_arg(args, 0))))
}

fn fn_is_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.stream_stat(&path_arg(args, 0)).is_some_and(|s| !s.is_dir)))
}

fn fn_is_dir(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.stream_stat(&path_arg(args, 0)).is_some_and(|s| s.is_dir)))
}

fn fn_is_readable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // Approximate: readable iff it exists (no per-user perm modelling).
    Ok(Value::Bool(host.stream_exists(&path_arg(args, 0))))
}

/// `basename(path, suffix="")` — the trailing name component, byte-wise, matching
/// php (trailing slashes ignored; an exact suffix stripped unless it's the whole).
fn fn_basename(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = path_arg(args, 0);
    let suffix = args.get(1).map(|v| conv::to_string_bytes(&deref(v))).unwrap_or_default();
    // drop trailing '/'
    let end = path.iter().rposition(|&b| b != b'/').map(|i| i + 1).unwrap_or(0);
    let trimmed = &path[..end];
    let start = trimmed.iter().rposition(|&b| b == b'/').map(|i| i + 1).unwrap_or(0);
    let mut base = &trimmed[start..];
    if !suffix.is_empty() && base.len() > suffix.len() && base.ends_with(&suffix[..]) {
        base = &base[..base.len() - suffix.len()];
    }
    Ok(Value::str(base.to_vec()))
}

/// `dirname(path, levels=1)` — matching php: "." when no slash, "/" at root.
fn fn_dirname(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut path = path_arg(args, 0);
    let levels = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(1).max(1);
    for _ in 0..levels {
        path = dirname_once(&path);
    }
    Ok(Value::str(path))
}

fn dirname_once(path: &[u8]) -> Vec<u8> {
    // drop trailing slashes
    let end = path.iter().rposition(|&b| b != b'/').map(|i| i + 1).unwrap_or(0);
    let trimmed = &path[..end];
    match trimmed.iter().rposition(|&b| b == b'/') {
        None => b".".to_vec(),
        Some(0) => b"/".to_vec(),
        Some(i) => trimmed[..i].to_vec(),
    }
}

fn fn_realpath(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match std::fs::canonicalize(&*as_str(&path_arg(args, 0))) {
        Ok(p) => Ok(Value::str(p.to_string_lossy().into_owned().into_bytes())),
        Err(_) => Ok(Value::Bool(false)),
    }
}

fn fn_getcwd(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    match std::env::current_dir() {
        Ok(p) => Ok(Value::str(p.to_string_lossy().into_owned().into_bytes())),
        Err(_) => Ok(Value::Bool(false)),
    }
}

fn fn_file_get_contents(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match host.stream_get_contents(&path_arg(args, 0)) {
        Some(bytes) => Ok(Value::str(bytes)),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_file_put_contents(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = conv::to_string_bytes(&deref(&args[1]));
    match host.stream_put_contents(&path_arg(args, 0), &data) {
        Some(n) => Ok(Value::Int(n as i64)),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_unlink(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(std::fs::remove_file(&*as_str(&path_arg(args, 0))).is_ok()))
}

fn fn_sys_get_temp_dir(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(std::env::temp_dir().to_string_lossy().into_owned().into_bytes()))
}

fn fn_is_writable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // Approximate: writable iff it exists (no per-user perm modelling).
    Ok(Value::Bool(host.stream_exists(&path_arg(args, 0))))
}

fn fn_mkdir(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = as_str(&path_arg(args, 0)).into_owned();
    let recursive = args.get(2).map(|v| conv::to_bool(&deref(v))).unwrap_or(false);
    let r = if recursive { std::fs::create_dir_all(&path) } else { std::fs::create_dir(&path) };
    Ok(Value::Bool(r.is_ok()))
}

fn fn_rmdir(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(std::fs::remove_dir(&*as_str(&path_arg(args, 0))).is_ok()))
}

fn fn_rename(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let from = as_str(&path_arg(args, 0)).into_owned();
    let to = as_str(&path_arg(args, 1)).into_owned();
    Ok(Value::Bool(std::fs::rename(&from, &to).is_ok()))
}

fn fn_copy(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let from = as_str(&path_arg(args, 0)).into_owned();
    let to = as_str(&path_arg(args, 1)).into_owned();
    Ok(Value::Bool(std::fs::copy(&from, &to).is_ok()))
}

fn fn_filemtime(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match host.stream_stat(&path_arg(args, 0)) {
        Some(s) => Ok(Value::Int(s.mtime)),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_filesize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match host.stream_stat(&path_arg(args, 0)) {
        Some(s) => Ok(Value::Int(s.size as i64)),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_touch(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // Create the file if absent; we don't model timestamps.
    let path = as_str(&path_arg(args, 0)).into_owned();
    let ok = std::path::Path::new(&path).exists()
        || std::fs::OpenOptions::new().create(true).append(true).open(&path).is_ok();
    Ok(Value::Bool(ok))
}

fn fn_true1(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}

fn fn_fileperms(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(0o100644))
}

fn fn_scandir(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = as_str(&path_arg(args, 0)).into_owned();
    match std::fs::read_dir(&path) {
        Ok(rd) => {
            let mut names: Vec<Vec<u8>> = vec![b".".to_vec(), b"..".to_vec()];
            for e in rd.flatten() {
                names.push(e.file_name().to_string_lossy().into_owned().into_bytes());
            }
            names.sort();
            let mut a = rhp_values::PhpArray::new();
            for n in names {
                a.push(Value::str(n));
            }
            Ok(Value::Array(a))
        }
        Err(_) => Ok(Value::Bool(false)),
    }
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(name, Builtin { name, func, min_args: min, max_args: max });
    }
    put(m, "file_exists", fn_file_exists, 1, Some(1));
    put(m, "is_file", fn_is_file, 1, Some(1));
    put(m, "is_dir", fn_is_dir, 1, Some(1));
    put(m, "is_readable", fn_is_readable, 1, Some(1));
    put(m, "basename", fn_basename, 1, Some(2));
    put(m, "dirname", fn_dirname, 1, Some(2));
    put(m, "realpath", fn_realpath, 1, Some(1));
    put(m, "getcwd", fn_getcwd, 0, Some(0));
    put(m, "file_get_contents", fn_file_get_contents, 1, Some(1));
    put(m, "file_put_contents", fn_file_put_contents, 2, Some(3));
    put(m, "unlink", fn_unlink, 1, Some(1));
    put(m, "sys_get_temp_dir", fn_sys_get_temp_dir, 0, Some(0));
    put(m, "is_writable", fn_is_writable, 1, Some(1));
    put(m, "is_writeable", fn_is_writable, 1, Some(1));
    put(m, "is_executable", fn_is_writable, 1, Some(1));
    put(m, "mkdir", fn_mkdir, 1, Some(4));
    put(m, "rmdir", fn_rmdir, 1, Some(2));
    put(m, "rename", fn_rename, 2, Some(3));
    put(m, "copy", fn_copy, 2, Some(3));
    put(m, "filemtime", fn_filemtime, 1, Some(1));
    put(m, "filesize", fn_filesize, 1, Some(1));
    put(m, "fileperms", fn_fileperms, 1, Some(1));
    put(m, "touch", fn_touch, 1, Some(3));
    put(m, "chmod", fn_true1, 2, Some(2));
    put(m, "chdir", fn_true1, 1, Some(1));
    put(m, "clearstatcache", fn_true1, 0, Some(2));
    put(m, "scandir", fn_scandir, 1, Some(3));
}
