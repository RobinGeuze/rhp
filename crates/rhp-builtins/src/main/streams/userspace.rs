//! main/streams/userspace.c: the wrapper-table functions. User wrappers
//! (`stream_wrapper_register`) are not here yet.

use crate::{diag, zend::zpp, Host, Thrown, WrapperRestore};
use rhp_values::Value;

fn fn_stream_wrapper_unregister(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_wrapper_unregister";
    let protocol = zpp::str_arg(host, args, 0, fname, "protocol")?;
    if host.stream_wrapper_unregister(&protocol) {
        return Ok(Value::Bool(true));
    }
    let mut m = b"Unable to unregister protocol ".to_vec();
    m.extend_from_slice(&protocol);
    m.extend_from_slice(b"://");
    diag::warn(host, fname, &m)?;
    Ok(Value::Bool(false))
}

fn fn_stream_wrapper_restore(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "stream_wrapper_restore";
    let protocol = zpp::str_arg(host, args, 0, fname, "protocol")?;
    let mut m = protocol.clone();
    Ok(Value::Bool(match host.stream_wrapper_restore(&protocol) {
        WrapperRestore::Restored => true,
        WrapperRestore::Unchanged => {
            m.extend_from_slice(b":// was never changed, nothing to restore");
            diag::notice(host, fname, &m)?;
            true
        }
        WrapperRestore::NeverExisted => {
            m.extend_from_slice(b":// never existed, nothing to restore");
            diag::warn(host, fname, &m)?;
            false
        }
    }))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("stream_wrapper_unregister", fn_stream_wrapper_unregister),
            ("stream_wrapper_restore", fn_stream_wrapper_restore),
        ],
    );
}
