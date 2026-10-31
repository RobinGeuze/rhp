//! The `zlib` extension (php-src ext/zlib/zlib.c + zlib_fopen_wrapper.c).
//!
//! Three wire formats share one DEFLATE core and differ only in framing:
//!   * raw DEFLATE (RFC 1951)  — `gzdeflate` / `gzinflate`
//!   * zlib (RFC 1950)         — `gzcompress` / `gzuncompress`
//!   * gzip (RFC 1952)         — `gzencode` / `gzdecode`
//!
//! plus `zlib_encode`/`zlib_decode` (any of them, decode auto-detects), the
//! `gzopen` resource family and the `compress.zlib://` wrapper. Everything goes
//! through C zlib with php's exact parameters (see `zraw`), so compressed bytes
//! are identical to php's.

mod zraw;

use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;

use rhp_values::{conv, PhpArray, Value};

use crate::streams::{self, StatInfo, StreamWrapper, WrapperReg};
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

fn int_arg(args: &[Value], i: usize, default: i64) -> i64 {
    match args.get(i).map(deref) {
        None => default,
        Some(v) => conv::to_int(&v),
    }
}

/// `PHP_ZLIB_ENCODE_FUNC`: validate level/encoding, then deflate.
fn encode(
    name: &str,
    data: &[u8],
    level: i64,
    encoding: i64,
    level_argn: u32,
    enc_argn: u32,
) -> Result<Value, Thrown> {
    if !(-1..=9).contains(&level) {
        return Err(Thrown::error(
            "ValueError",
            format!("{name}(): Argument #{level_argn} ($level) must be between -1 and 9"),
        ));
    }
    if !matches!(
        encoding,
        zraw::ENCODING_RAW | zraw::ENCODING_GZIP | zraw::ENCODING_DEFLATE
    ) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{name}(): Argument #{enc_argn} ($encoding) must be one of ZLIB_ENCODING_RAW, ZLIB_ENCODING_GZIP, or ZLIB_ENCODING_DEFLATE"
            ),
        ));
    }
    Ok(
        match zraw::deflate(data, level, encoding, zraw::MAX_MEM_LEVEL, false) {
            Ok(out) => Value::str(out),
            Err(_) => Value::Bool(false),
        },
    )
}

/// `PHP_ZLIB_DECODE_FUNC`.
fn decode(name: &str, args: &[Value], encoding: i64) -> Result<Value, Thrown> {
    let data = bytes_arg(args, 0);
    let max = int_arg(args, 1, 0);
    if max < 0 {
        return Err(Thrown::error(
            "ValueError",
            format!("{name}(): Argument #2 ($max_length) must be greater than or equal to 0"),
        ));
    }
    Ok(match zraw::decode(&data, encoding, max as usize) {
        Ok(out) => Value::str(out),
        Err(_) => Value::Bool(false),
    })
}

fn fn_gzdeflate(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    encode(
        "gzdeflate",
        &bytes_arg(args, 0),
        int_arg(args, 1, -1),
        int_arg(args, 2, zraw::ENCODING_RAW),
        2,
        3,
    )
}
fn fn_gzcompress(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    encode(
        "gzcompress",
        &bytes_arg(args, 0),
        int_arg(args, 1, -1),
        int_arg(args, 2, zraw::ENCODING_DEFLATE),
        2,
        3,
    )
}
fn fn_gzencode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    encode(
        "gzencode",
        &bytes_arg(args, 0),
        int_arg(args, 1, -1),
        int_arg(args, 2, zraw::ENCODING_GZIP),
        2,
        3,
    )
}
fn fn_zlib_encode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    encode(
        "zlib_encode",
        &bytes_arg(args, 0),
        int_arg(args, 2, -1),
        int_arg(args, 1, 0),
        3,
        2,
    )
}
fn fn_gzinflate(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    decode("gzinflate", args, zraw::ENCODING_RAW)
}
fn fn_gzuncompress(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    decode("gzuncompress", args, zraw::ENCODING_DEFLATE)
}
fn fn_gzdecode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    decode("gzdecode", args, zraw::ENCODING_GZIP)
}
fn fn_zlib_decode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    decode("zlib_decode", args, zraw::ENCODING_ANY)
}

fn fn_zlib_get_coding_type(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(false))
}

// --- compress.zlib:// --------------------------------------------------------

const SCHEME_PREFIX: &[u8] = b"compress.zlib://";

fn inner(url: &[u8]) -> &[u8] {
    url.strip_prefix(SCHEME_PREFIX).unwrap_or(url)
}

/// The `compress.zlib://` wrapper: reads decode gzip transparently (plain
/// files pass through, as zlib's gzread does); writes produce what php's zlib
/// stream writes (gzip, default level, a sync flush then the final block).
pub struct ZlibWrapper;

impl StreamWrapper for ZlibWrapper {
    fn exists(&self, url: &[u8]) -> bool {
        streams::default_exists(inner(url))
    }
    fn get_contents(&self, url: &[u8]) -> Option<Vec<u8>> {
        let raw = streams::default_get_contents(inner(url))?;
        zraw::gunzip_transparent(&raw)
    }
    fn stat(&self, url: &[u8]) -> Option<StatInfo> {
        streams::default_stat(inner(url))
    }
    fn put_contents(&self, url: &[u8], data: &[u8]) -> Option<usize> {
        let gz = zraw::deflate(data, -1, zraw::ENCODING_GZIP, zraw::DEF_MEM_LEVEL, true).ok()?;
        streams::default_put_contents(inner(url), &gz).map(|_| data.len())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn wrappers() -> Vec<WrapperReg> {
    vec![WrapperReg {
        scheme: "compress.zlib",
        factory: || Rc::new(ZlibWrapper),
    }]
}

fn zlib_url(path: &[u8]) -> Vec<u8> {
    if path.starts_with(SCHEME_PREFIX) {
        path.to_vec()
    } else {
        let mut u = SCHEME_PREFIX.to_vec();
        u.extend_from_slice(path);
        u
    }
}

// --- gzopen family -----------------------------------------------------------

fn fn_gzopen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = bytes_arg(args, 0);
    let mode = bytes_arg(args, 1);
    if mode.contains(&b'+') {
        // php: "Cannot open a zlib stream for reading and writing at the same time!"
        return Ok(Value::Bool(false));
    }
    // zlib-only mode letters (level digits, strategies) are not fopen modes.
    let fmode: Vec<u8> = mode
        .iter()
        .copied()
        .filter(|c| matches!(c, b'r' | b'w' | b'a' | b'x' | b'c' | b'b' | b't'))
        .collect();
    Ok(host
        .stream_open(&zlib_url(&path), &fmode)
        .unwrap_or(Value::Bool(false)))
}

fn fn_gzfile(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = bytes_arg(args, 0);
    let Some(data) = host.stream_get_contents(&zlib_url(&path)) else {
        return Ok(Value::Bool(false));
    };
    let mut a = PhpArray::new();
    let mut start = 0;
    for (i, &b) in data.iter().enumerate() {
        if b == b'\n' {
            a.push(Value::str(data[start..=i].to_vec()));
            start = i + 1;
        }
    }
    if start < data.len() {
        a.push(Value::str(data[start..].to_vec()));
    }
    Ok(Value::Array(a))
}

fn fn_readgzfile(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = bytes_arg(args, 0);
    let Some(data) = host.stream_get_contents(&zlib_url(&path)) else {
        return Ok(Value::Bool(false));
    };
    host.write_out(&data);
    Ok(Value::Int(data.len() as i64))
}

fn fn_gzpassthru(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let res = deref(&args[0]);
    let data = host.stream_read(&res, -1).unwrap_or_default();
    host.write_out(&data);
    Ok(Value::Int(data.len() as i64))
}

/// `gzread` & co. are the plain stream functions applied to the gz stream.
fn alias(host: &mut dyn Host, target: &str, args: &[Value]) -> Result<Value, Thrown> {
    match crate::lookup(target) {
        Some(b) => (b.func)(host, args),
        None => Ok(Value::Bool(false)),
    }
}

macro_rules! aliases {
    ($($name:ident => $target:literal),* $(,)?) => {
        $(fn $name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            alias(host, $target, args)
        })*
    };
}
aliases! {
    fn_gzread => "fread",
    fn_gzwrite => "fwrite",
    fn_gzclose => "fclose",
    fn_gzeof => "feof",
    fn_gzgets => "fgets",
    fn_gzgetc => "fgetc",
    fn_gzrewind => "rewind",
    fn_gzseek => "fseek",
    fn_gztell => "ftell",
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    let table: &[(&'static str, crate::NativeFn, usize, Option<usize>)] = &[
        ("gzdeflate", fn_gzdeflate, 1, Some(3)),
        ("gzinflate", fn_gzinflate, 1, Some(2)),
        ("gzcompress", fn_gzcompress, 1, Some(3)),
        ("gzuncompress", fn_gzuncompress, 1, Some(2)),
        ("gzencode", fn_gzencode, 1, Some(3)),
        ("gzdecode", fn_gzdecode, 1, Some(2)),
        ("zlib_encode", fn_zlib_encode, 2, Some(3)),
        ("zlib_decode", fn_zlib_decode, 1, Some(2)),
        ("zlib_get_coding_type", fn_zlib_get_coding_type, 0, Some(0)),
        ("gzopen", fn_gzopen, 2, Some(3)),
        ("gzfile", fn_gzfile, 1, Some(2)),
        ("readgzfile", fn_readgzfile, 1, Some(2)),
        ("gzpassthru", fn_gzpassthru, 1, Some(1)),
        ("gzread", fn_gzread, 2, Some(2)),
        ("gzwrite", fn_gzwrite, 2, Some(3)),
        ("gzputs", fn_gzwrite, 2, Some(3)),
        ("gzclose", fn_gzclose, 1, Some(1)),
        ("gzeof", fn_gzeof, 1, Some(1)),
        ("gzgets", fn_gzgets, 1, Some(2)),
        ("gzgetc", fn_gzgetc, 1, Some(1)),
        ("gzrewind", fn_gzrewind, 1, Some(1)),
        ("gzseek", fn_gzseek, 2, Some(3)),
        ("gztell", fn_gztell, 1, Some(1)),
    ];
    for &(name, func, min, max) in table {
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
}

fn zlib_version() -> String {
    // SAFETY: zlibVersion returns a static NUL-terminated string.
    unsafe {
        std::ffi::CStr::from_ptr(libz_sys::zlibVersion())
            .to_string_lossy()
            .into_owned()
    }
}

/// `ZLIB_*` / `FORCE_*` constants.
pub fn constants() -> Vec<(&'static str, Value)> {
    let ver = zlib_version();
    let nums: Vec<i64> = ver
        .split('.')
        .map(|p| p.parse::<i64>().unwrap_or(0))
        .collect();
    let at = |i: usize| nums.get(i).copied().unwrap_or(0) & 0xF;
    let vernum = (at(0) << 12) | (at(1) << 8) | (at(2) << 4) | at(3);
    vec![
        ("FORCE_GZIP", Value::Int(31)),
        ("FORCE_DEFLATE", Value::Int(15)),
        ("ZLIB_ENCODING_RAW", Value::Int(-15)),
        ("ZLIB_ENCODING_GZIP", Value::Int(31)),
        ("ZLIB_ENCODING_DEFLATE", Value::Int(15)),
        ("ZLIB_NO_FLUSH", Value::Int(0)),
        ("ZLIB_PARTIAL_FLUSH", Value::Int(1)),
        ("ZLIB_SYNC_FLUSH", Value::Int(2)),
        ("ZLIB_FULL_FLUSH", Value::Int(3)),
        ("ZLIB_BLOCK", Value::Int(5)),
        ("ZLIB_FINISH", Value::Int(4)),
        ("ZLIB_FILTERED", Value::Int(1)),
        ("ZLIB_HUFFMAN_ONLY", Value::Int(2)),
        ("ZLIB_RLE", Value::Int(3)),
        ("ZLIB_FIXED", Value::Int(4)),
        ("ZLIB_DEFAULT_STRATEGY", Value::Int(0)),
        ("ZLIB_VERSION", Value::str(ver.into_bytes())),
        ("ZLIB_VERNUM", Value::Int(vernum)),
        ("ZLIB_OK", Value::Int(0)),
        ("ZLIB_STREAM_END", Value::Int(1)),
        ("ZLIB_NEED_DICT", Value::Int(2)),
        ("ZLIB_ERRNO", Value::Int(-1)),
        ("ZLIB_STREAM_ERROR", Value::Int(-2)),
        ("ZLIB_DATA_ERROR", Value::Int(-3)),
        ("ZLIB_MEM_ERROR", Value::Int(-4)),
        ("ZLIB_BUF_ERROR", Value::Int(-5)),
        ("ZLIB_VERSION_ERROR", Value::Int(-6)),
    ]
}

/// Deflate for other modules (ZipArchive): raw, libzip's level/mem level.
pub fn raw_deflate(data: &[u8], level: i64) -> Option<Vec<u8>> {
    zraw::deflate(data, level, zraw::ENCODING_RAW, zraw::MAX_MEM_LEVEL, false).ok()
}
