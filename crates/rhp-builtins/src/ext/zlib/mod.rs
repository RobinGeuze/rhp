//! The `zlib` extension: DEFLATE-family (de)compression.
//!
//! Three wire formats share one DEFLATE core and differ only in header/trailer:
//!   * raw DEFLATE (RFC 1951)  — `gzdeflate` / `gzinflate`
//!   * zlib (RFC 1950)         — `gzcompress` / `gzuncompress`
//!   * gzip (RFC 1952)         — `gzencode` / `gzdecode`
//!
//! Backed by `flate2` (pure-Rust `miniz_oxide`). **Decompression is byte-exact**
//! vs php and verified by differential tests; the *compressors* emit valid but
//! not byte-identical output (no pure-Rust lib reproduces zlib's exact bytes), so
//! they are proven by round-trip (php↔rhp) rather than byte comparison.
//!
//! Resource-based `gzopen`/`gzread`/… and the `compress.zlib://` wrapper are a
//! later (resource) stage.

use std::collections::HashMap;
use std::io::{Read, Write};

use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use flate2::Compression;
use rhp_values::{conv, Value};

use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn bytes_arg(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&deref(&args[i]))
}

/// PHP level: -1 (default) → zlib default (6); 0..9 clamped.
fn level_arg(args: &[Value], i: usize) -> Compression {
    match args.get(i).map(|v| conv::to_int(&deref(v))) {
        None | Some(-1) => Compression::default(),
        Some(n) => Compression::new(n.clamp(0, 9) as u32),
    }
}

fn encode<W, F: FnOnce(Vec<u8>) -> W>(data: &[u8], make: F) -> Vec<u8>
where
    W: Write + FinishInto,
{
    let mut enc = make(Vec::new());
    let _ = enc.write_all(data);
    enc.finish_into()
}

/// Small shim so the three flate2 write-encoders share one code path.
trait FinishInto {
    fn finish_into(self) -> Vec<u8>;
}
impl FinishInto for DeflateEncoder<Vec<u8>> {
    fn finish_into(self) -> Vec<u8> {
        self.finish().unwrap_or_default()
    }
}
impl FinishInto for ZlibEncoder<Vec<u8>> {
    fn finish_into(self) -> Vec<u8> {
        self.finish().unwrap_or_default()
    }
}
impl FinishInto for GzEncoder<Vec<u8>> {
    fn finish_into(self) -> Vec<u8> {
        self.finish().unwrap_or_default()
    }
}

fn decode<R: Read>(mut dec: R) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    dec.read_to_end(&mut out).ok().map(|_| out)
}

fn fn_gzdeflate(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = bytes_arg(args, 0);
    let lvl = level_arg(args, 1);
    Ok(Value::str(encode(&data, |w| DeflateEncoder::new(w, lvl))))
}

fn fn_gzinflate(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = bytes_arg(args, 0);
    Ok(decode(DeflateDecoder::new(&data[..]))
        .map(Value::str)
        .unwrap_or(Value::Bool(false)))
}

fn fn_gzcompress(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = bytes_arg(args, 0);
    let lvl = level_arg(args, 1);
    Ok(Value::str(encode(&data, |w| ZlibEncoder::new(w, lvl))))
}

fn fn_gzuncompress(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = bytes_arg(args, 0);
    Ok(decode(ZlibDecoder::new(&data[..]))
        .map(Value::str)
        .unwrap_or(Value::Bool(false)))
}

fn fn_gzencode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = bytes_arg(args, 0);
    let lvl = level_arg(args, 1);
    Ok(Value::str(encode(&data, |w| GzEncoder::new(w, lvl))))
}

fn fn_gzdecode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = bytes_arg(args, 0);
    Ok(decode(GzDecoder::new(&data[..]))
        .map(Value::str)
        .unwrap_or(Value::Bool(false)))
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
    put(m, "gzdeflate", fn_gzdeflate, 1, Some(3));
    put(m, "gzinflate", fn_gzinflate, 1, Some(2));
    put(m, "gzcompress", fn_gzcompress, 1, Some(3));
    put(m, "gzuncompress", fn_gzuncompress, 1, Some(2));
    put(m, "gzencode", fn_gzencode, 1, Some(3));
    put(m, "gzdecode", fn_gzdecode, 1, Some(2));
}

/// `ZLIB_*` / `FORCE_*` encoding constants.
pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("ZLIB_ENCODING_RAW", Value::Int(-15)),
        ("ZLIB_ENCODING_DEFLATE", Value::Int(15)),
        ("ZLIB_ENCODING_GZIP", Value::Int(31)),
        ("FORCE_GZIP", Value::Int(31)),
        ("FORCE_DEFLATE", Value::Int(15)),
        ("ZLIB_VERSION", Value::str("1.3.1")),
    ]
}
