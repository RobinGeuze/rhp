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
    let Some(data) = read_file_for_hash(host, "md5_file", &path)? else {
        return Ok(Value::Bool(false));
    };
    let raw = digest(b"md5", &data).unwrap();
    Ok(Value::str(if binary { raw } else { to_hex(&raw) }))
}

/// `php_stream_open_wrapper(path, "rb", REPORT_ERRORS)` and a read to the
/// end, for md5_file / sha1_file / hash_file: an empty path is a
/// ValueError, a failed open warns.
pub(crate) fn read_file_for_hash(
    host: &mut dyn Host,
    fname: &str,
    path: &[u8],
) -> Result<Option<Vec<u8>>, Thrown> {
    if path.is_empty() {
        return Err(Thrown::error("ValueError", "Path must not be empty"));
    }
    let Some(url) = crate::locate_for_open(host, fname, path)? else {
        return Ok(None);
    };
    crate::main::streams::clear_errno();
    match host.stream_get_contents(&url) {
        Some(data) => {
            host.consume_resource_ids(1);
            Ok(Some(data))
        }
        None => {
            crate::open_failed(host, fname, path)?;
            Ok(None)
        }
    }
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("md5", fn_md5), ("md5_file", fn_md5_file)]);
}
