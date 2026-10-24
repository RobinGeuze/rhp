//! Filesystem & path builtins (php `ext/standard` file functions): existence
//! checks, `basename`/`dirname`, `realpath`/`getcwd`, and whole-file read/write.
//! Local paths only (no stream wrappers / URLs yet).

use std::collections::HashMap;
use std::path::Path;

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

fn fn_file_exists(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(Path::new(&*as_str(&path_arg(args, 0))).exists()))
}

fn fn_is_file(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(Path::new(&*as_str(&path_arg(args, 0))).is_file()))
}

fn fn_is_dir(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(Path::new(&*as_str(&path_arg(args, 0))).is_dir()))
}

fn fn_is_readable(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // Approximate: readable iff it exists (no per-user perm modelling).
    Ok(Value::Bool(Path::new(&*as_str(&path_arg(args, 0))).exists()))
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

fn fn_file_get_contents(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match std::fs::read(&*as_str(&path_arg(args, 0))) {
        Ok(bytes) => Ok(Value::str(bytes)),
        Err(_) => Ok(Value::Bool(false)),
    }
}

fn fn_file_put_contents(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = conv::to_string_bytes(&deref(&args[1]));
    match std::fs::write(&*as_str(&path_arg(args, 0)), &data) {
        Ok(()) => Ok(Value::Int(data.len() as i64)),
        Err(_) => Ok(Value::Bool(false)),
    }
}

fn fn_unlink(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(std::fs::remove_file(&*as_str(&path_arg(args, 0))).is_ok()))
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
}
