//! ext/standard/crc32.c: `crc32`, from the linked zlib.

use rhp_values::{conv, Value};

use crate::{Host, Thrown};

fn fn_crc32(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    Ok(Value::Int(crate::ext::zlib::zraw::crc32(&s) as i64))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("crc32", fn_crc32)]);
}
