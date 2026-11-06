//! ext/standard/datetime.c: `strptime()` over the C library's strptime(3).

use rhp_values::{conv, ops::deref, ArrayKey, PhpArray, Value};

use crate::{Host, Thrown};

fn fn_strptime(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ts = conv::to_string_bytes(&deref(&args[0]));
    let format = conv::to_string_bytes(&deref(&args[1]));
    // php passes the C strings: each ends at its first NUL.
    let cut = |b: &[u8]| {
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        std::ffi::CString::new(&b[..end]).expect("no NUL")
    };
    let (ts, format) = (cut(&ts), cut(&format));
    // SAFETY: all-zero is a valid tm (php memsets it).
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: NUL-terminated inputs; the result points into `ts`.
    let rest = unsafe { libc::strptime(ts.as_ptr(), format.as_ptr(), &mut tm) };
    if rest.is_null() {
        return Ok(Value::Bool(false));
    }
    // SAFETY: strptime returned a pointer into the NUL-terminated `ts`.
    let unparsed = unsafe { std::ffi::CStr::from_ptr(rest) }
        .to_bytes()
        .to_vec();
    let mut a = PhpArray::new();
    for (k, v) in [
        ("tm_sec", tm.tm_sec),
        ("tm_min", tm.tm_min),
        ("tm_hour", tm.tm_hour),
        ("tm_mday", tm.tm_mday),
        ("tm_mon", tm.tm_mon),
        ("tm_year", tm.tm_year),
        ("tm_wday", tm.tm_wday),
        ("tm_yday", tm.tm_yday),
    ] {
        a.insert(ArrayKey::from_bytes(k.as_bytes()), Value::Int(v as i64));
    }
    a.insert(ArrayKey::from_bytes(b"unparsed"), Value::str(unparsed));
    Ok(Value::Array(a))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("strptime", fn_strptime)]);
}
