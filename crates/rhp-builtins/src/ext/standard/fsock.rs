//! ext/standard/fsock.c: `fsockopen` and `pfsockopen`.

use rhp_values::Value;

use crate::ext::standard::streamsfuncs::{c_str, write_ref};
use crate::main::streams::xp_socket::{
    default_socket_timeout, escape_persistent_key, persistent_lookup, register_stream,
    xport_client, Opened,
};
use crate::zend::zpp::{zpp_double_or_null, zpp_long, zpp_string};
use crate::{Host, Thrown};

fn fsockopen_impl(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Value, Thrown> {
    let persistent = fname == "pfsockopen";
    let hostname = zpp_string(host, args, 0, fname, "hostname")?;
    let port = match args.get(1) {
        Some(_) => zpp_long(host, args, 1, fname, "port")?,
        None => -1,
    };
    let timeout = zpp_double_or_null(host, args, 4, fname, "timeout")?
        .unwrap_or(default_socket_timeout(host) as f64);
    let max = u64::MAX as f64 / 1_000_000.0;
    if timeout != -1.0 && !(0.0..=max).contains(&timeout) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{fname}(): Argument #5 ($timeout) must be -1 or between 0 and {}",
                max as u64
            ),
        ));
    }
    // (time_t) (timeout * 1000000.0), split
    let conv = (timeout * 1_000_000.0) as i64;
    let tv = (conv / 1_000_000, conv % 1_000_000);
    let mut target = hostname.clone();
    if port > 0 {
        target.extend_from_slice(format!(":{port}").as_bytes());
    }
    let persistent_key = persistent.then(|| {
        let mut k = b"pfsockopen__".to_vec();
        k.extend_from_slice(&escape_persistent_key(&hostname));
        k.extend_from_slice(format!(":{port}").as_bytes());
        k
    });
    if let Some(v) = persistent_key
        .as_deref()
        .and_then(|k| persistent_lookup(host, k))
    {
        write_ref(args.get(2), Value::Int(0));
        write_ref(args.get(3), Value::str(""));
        return Ok(v);
    }
    match xport_client(host, fname, &target, true, false, tv, &Value::Null)? {
        Opened::Ok(stream) => {
            write_ref(args.get(2), Value::Int(0));
            write_ref(args.get(3), Value::str(""));
            Ok(register_stream(
                host,
                *stream,
                &Value::Null,
                persistent_key.as_deref(),
            ))
        }
        Opened::Fail {
            code,
            text,
            allocated,
        } => {
            if allocated {
                host.consume_resource_ids(1);
            }
            crate::warn(
                host,
                fname,
                &format!(
                    "Unable to connect to {}:{} ({})",
                    c_str(&hostname),
                    port,
                    text.as_deref().unwrap_or("Unknown error")
                ),
            )?;
            write_ref(args.get(2), Value::Int(code as i64));
            if let Some(t) = text {
                write_ref(args.get(3), Value::str(t));
            }
            Ok(Value::Bool(false))
        }
    }
}

fn fn_fsockopen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    fsockopen_impl(host, args, "fsockopen")
}

fn fn_pfsockopen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    fsockopen_impl(host, args, "pfsockopen")
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[("fsockopen", fn_fsockopen), ("pfsockopen", fn_pfsockopen)],
    );
}
