//! ext/standard/pageinfo.c: `getmypid`, `getmyuid`, `getmygid` and
//! `getmyinode`, `getlastmod`.

use rhp_values::Value;

use crate::{Host, Thrown};

fn fn_getmypid(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(std::process::id() as i64))
}

fn fn_getmyuid(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(script_stat(host)
        .map(|m| Value::Int(std::os::unix::fs::MetadataExt::uid(&m) as i64))
        .unwrap_or(Value::Bool(false)))
}

fn fn_getmygid(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(script_stat(host)
        .map(|m| Value::Int(std::os::unix::fs::MetadataExt::gid(&m) as i64))
        .unwrap_or(Value::Bool(false)))
}

fn fn_getmyinode(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(script_stat(host)
        .map(|m| Value::Int(std::os::unix::fs::MetadataExt::ino(&m) as i64))
        .unwrap_or(Value::Bool(false)))
}

/// `getlastmod()`: the main script's mtime.
fn fn_getlastmod(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(script_stat(host)
        .map(|m| Value::Int(std::os::unix::fs::MetadataExt::mtime(&m)))
        .unwrap_or(Value::Bool(false)))
}

/// Stat of the main script (php's `sapi_get_stat`).
pub(crate) fn script_stat(host: &dyn Host) -> Option<std::fs::Metadata> {
    let p = host.running_main_path()?;
    std::fs::metadata(crate::zend::virtual_cwd::os_path(&p)).ok()
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("getmypid", fn_getmypid),
            ("getmyuid", fn_getmyuid),
            ("getmygid", fn_getmygid),
            ("getmyinode", fn_getmyinode),
            ("getlastmod", fn_getlastmod),
        ],
    );
}
