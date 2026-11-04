//! php-src's main/: the stream layer, the SAPI layer, request bodies,
//! output control and the ini machinery. This file is main/main.c's part
//! (`set_time_limit`).

use rhp_values::Value;

use crate::{Host, Thrown};

pub mod output;
pub mod rfc1867;
pub mod sapi;
pub mod streams;

fn fn_set_time_limit(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let secs = crate::zend::zpp::int_param(host, args, 0, "set_time_limit", "seconds", 0)?;
    crate::runtime_fns::ini_store(b"max_execution_time", secs.to_string().into_bytes());
    Ok(Value::Bool(true))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("set_time_limit", fn_set_time_limit)]);
}
