//! ext/standard/sha1.c: `sha1` and `sha1_file` (RustCrypto's sha1).

use digest::Digest;
use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::ext::hash::{as_bool, digest};
use crate::ext::standard::string::to_hex;
use crate::{Host, Thrown};

fn fn_sha1(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let digest = sha1::Sha1::digest(&s);
    let binary = args
        .get(1)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    Ok(Value::str(if binary {
        digest.to_vec()
    } else {
        to_hex(&digest)
    }))
}

fn fn_sha1_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = conv::to_string_bytes(&args[0]);
    let binary = as_bool(args.get(1));
    let Some(data) = host.stream_get_contents(&path) else {
        return Ok(Value::Bool(false));
    };
    host.consume_resource_ids(1);
    let raw = digest(b"sha1", &data).unwrap();
    Ok(Value::str(if binary { raw } else { to_hex(&raw) }))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("sha1", fn_sha1), ("sha1_file", fn_sha1_file)]);
}
