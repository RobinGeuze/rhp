//! ext/standard/md5.c: `md5` and `md5_file` (RustCrypto's md5).

use digest::Digest;
use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::ext::hash::{as_bool, digest};
use crate::ext::standard::string::to_hex;
use crate::{Host, Thrown};

// ---------------------------------------------------------------------------
// builtin wrappers
// ---------------------------------------------------------------------------

fn fn_md5(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let digest = md5::Md5::digest(&s);
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

fn fn_md5_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = conv::to_string_bytes(&args[0]);
    let binary = as_bool(args.get(1));
    let Some(data) = host.stream_get_contents(&path) else {
        return Ok(Value::Bool(false));
    };
    host.consume_resource_ids(1);
    let raw = digest(b"md5", &data).unwrap();
    Ok(Value::str(if binary { raw } else { to_hex(&raw) }))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("md5", fn_md5), ("md5_file", fn_md5_file)]);
}
