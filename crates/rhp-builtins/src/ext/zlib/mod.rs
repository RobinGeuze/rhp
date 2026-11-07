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

pub(crate) mod context;
pub mod output;
pub(crate) mod zlib_filter;
pub(crate) mod zraw;

use std::any::Any;
use std::rc::Rc;

use rhp_values::ops::deref;
use rhp_values::{conv, PhpArray, Value};

use crate::main::streams;
use crate::main::streams::{StatInfo, StreamWrapper, WrapperReg};
use crate::{Host, Thrown};

fn bytes_arg(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&deref(&args[i]))
}

fn int_arg(args: &[Value], i: usize, default: i64) -> i64 {
    match args.get(i).map(deref) {
        None => default,
        Some(v) => conv::to_int(&v),
    }
}

/// zlib's `zError(status)` message (its `z_errmsg` table).
pub(crate) fn z_error(status: i32) -> String {
    match status {
        2 => "need dictionary",
        1 => "stream end",
        0 => "",
        -1 => "file error",
        -2 => "stream error",
        -3 => "data error",
        -4 => "insufficient memory",
        -5 => "buffer error",
        -6 => "incompatible version",
        _ => "",
    }
    .to_string()
}

/// `PHP_ZLIB_ENCODE_FUNC`: validate level/encoding, then deflate.
fn encode(
    host: &mut dyn Host,
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
            Err(status) => {
                host.raise_diagnostic(
                    crate::E_WARNING,
                    format!("{name}(): {}", z_error(status)).as_bytes(),
                );
                Value::Bool(false)
            }
        },
    )
}

/// `PHP_ZLIB_DECODE_FUNC`.
fn decode(host: &mut dyn Host, name: &str, args: &[Value], encoding: i64) -> Result<Value, Thrown> {
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
        Err(status) => {
            host.raise_diagnostic(
                crate::E_WARNING,
                format!("{name}(): {}", z_error(status)).as_bytes(),
            );
            Value::Bool(false)
        }
    })
}

fn fn_gzdeflate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    encode(
        host,
        "gzdeflate",
        &bytes_arg(args, 0),
        int_arg(args, 1, -1),
        int_arg(args, 2, zraw::ENCODING_RAW),
        2,
        3,
    )
}
fn fn_gzcompress(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    encode(
        host,
        "gzcompress",
        &bytes_arg(args, 0),
        int_arg(args, 1, -1),
        int_arg(args, 2, zraw::ENCODING_DEFLATE),
        2,
        3,
    )
}
fn fn_gzencode(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    encode(
        host,
        "gzencode",
        &bytes_arg(args, 0),
        int_arg(args, 1, -1),
        int_arg(args, 2, zraw::ENCODING_GZIP),
        2,
        3,
    )
}
fn fn_zlib_encode(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    encode(
        host,
        "zlib_encode",
        &bytes_arg(args, 0),
        int_arg(args, 2, -1),
        int_arg(args, 1, 0),
        3,
        2,
    )
}
fn fn_gzinflate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    decode(host, "gzinflate", args, zraw::ENCODING_RAW)
}
fn fn_gzuncompress(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    decode(host, "gzuncompress", args, zraw::ENCODING_DEFLATE)
}
fn fn_gzdecode(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    decode(host, "gzdecode", args, zraw::ENCODING_GZIP)
}
fn fn_zlib_decode(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    decode(host, "zlib_decode", args, zraw::ENCODING_ANY)
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
    /// php's zlib wrapper has no `url_stat`: `file_exists`/`is_file`/`stat`
    /// on a `compress.zlib://` URL are false.
    fn exists(&self, _url: &[u8]) -> bool {
        false
    }
    fn get_contents(&self, url: &[u8]) -> Option<Vec<u8>> {
        let raw = streams::default_get_contents(inner(url))?;
        zraw::gunzip_transparent(&raw)
    }
    fn stat(&self, _url: &[u8]) -> Option<StatInfo> {
        None
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

/// `$use_include_path`: php_stream_open_wrapper's USE_PATH resolves the
/// inner path when that file exists on the include path.
fn include_path_arg(host: &mut dyn Host, args: &[Value], i: usize, path: Vec<u8>) -> Vec<u8> {
    if !args.get(i).is_some_and(|v| conv::to_bool(&deref(v))) {
        return path;
    }
    crate::ext::standard::file::resolve_include_path(host, inner(&path)).unwrap_or(path)
}

/// The plain-files opener's open_basedir check of a gz function's file
/// (its warning, then the "Failed to open stream" one).
fn basedir_allowed(host: &mut dyn Host, fname: &str, path: &[u8]) -> Result<bool, Thrown> {
    let rel = inner(path);
    if !crate::main::fopen_wrappers::active() || streams::split_scheme(rel).0 != b"file" {
        return Ok(true);
    }
    crate::main::fopen_wrappers::check_open(host, fname, streams::file_path(rel), rel)
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

/// The `strerror` text php's plain-files wrapper reports when opening `path`
/// with `mode` fails ("No such file or directory", …), or its mode error.
pub(crate) fn open_failure_reason(path: &[u8], mode: &[u8]) -> String {
    if path.windows(3).any(|w| w == b"://") {
        return "operation failed".into();
    }
    let w = libc::O_WRONLY | libc::O_CREAT;
    let flags = match mode.first() {
        Some(b'r') => libc::O_RDONLY,
        Some(b'w' | b'c') => w,
        Some(b'a') => w | libc::O_APPEND,
        Some(b'x') => w | libc::O_EXCL,
        _ => {
            return format!(
                "`{}' is not a valid mode for fopen",
                String::from_utf8_lossy(mode)
            );
        }
    };
    match crate::zend::virtual_cwd::open(path, flags | libc::O_CLOEXEC, 0o666) {
        Ok(_) => "operation failed".into(),
        Err(e) => match e.raw_os_error() {
            Some(code) => crate::main::streams::strerror(code),
            None => e.to_string(),
        },
    }
}

/// php's "fn(path): Failed to open stream: reason" warning.
pub(crate) fn warn_open_failed(host: &mut dyn Host, func: &str, path: &[u8], mode: &[u8]) {
    let reason = open_failure_reason(path, mode);
    let msg = [
        func.as_bytes(),
        b"(",
        path,
        format!("): Failed to open stream: {reason}").as_bytes(),
    ]
    .concat();
    host.raise_diagnostic(crate::E_WARNING, &msg);
}

fn fn_gzopen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = include_path_arg(host, args, 2, bytes_arg(args, 0));
    let mode = bytes_arg(args, 1);
    if mode.contains(&b'+') {
        host.raise_diagnostic(
            crate::E_WARNING,
            b"gzopen(): Cannot open a zlib stream for reading and writing at the same time!",
        );
        return Ok(Value::Bool(false));
    }
    // The inner plain-files open validates the mode by its first letter.
    if !matches!(mode.first(), Some(b'r' | b'w' | b'a' | b'x' | b'c')) {
        warn_open_failed(host, "gzopen", inner(&path), &mode);
        return Ok(Value::Bool(false));
    }
    // zlib-only mode letters (level digits, strategies) are not fopen modes.
    let fmode: Vec<u8> = mode
        .iter()
        .copied()
        .filter(|c| matches!(c, b'r' | b'w' | b'a' | b'x' | b'c' | b'b' | b't'))
        .collect();
    if !basedir_allowed(host, "gzopen", &path)? {
        return Ok(Value::Bool(false));
    }
    // php opens (creating/truncating) the underlying file right away.
    let rel = inner(&path);
    if mode[0] != b'r' && !rel.windows(3).any(|w| w == b"://") {
        let w = libc::O_WRONLY | libc::O_CREAT | libc::O_CLOEXEC;
        let flags = match mode[0] {
            b'w' => w | libc::O_TRUNC,
            b'a' => w | libc::O_APPEND,
            b'x' => w | libc::O_EXCL,
            _ => w,
        };
        if crate::zend::virtual_cwd::open(rel, flags, 0o666).is_err() {
            warn_open_failed(host, "gzopen", rel, &mode);
            return Ok(Value::Bool(false));
        }
    }
    Ok(match host.stream_open(&zlib_url(&path), &fmode) {
        Some(v) => v,
        None => {
            warn_open_failed(host, "gzopen", inner(&path), &mode);
            Value::Bool(false)
        }
    })
}

fn fn_gzfile(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = include_path_arg(host, args, 1, bytes_arg(args, 0));
    if !basedir_allowed(host, "gzfile", &path)? {
        return Ok(Value::Bool(false));
    }
    let Some(data) = host.stream_get_contents(&zlib_url(&path)) else {
        warn_open_failed(host, "gzfile", inner(&path), b"rb");
        return Ok(Value::Bool(false));
    };
    let mut a = PhpArray::new();
    let mut start = 0;
    for (i, &b) in data.iter().enumerate() {
        if b == b'\n' {
            a.push(Value::str(&data[start..=i]));
            start = i + 1;
        }
    }
    if start < data.len() {
        a.push(Value::str(&data[start..]));
    }
    Ok(Value::Array(a))
}

fn fn_readgzfile(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = include_path_arg(host, args, 1, bytes_arg(args, 0));
    if !basedir_allowed(host, "readgzfile", &path)? {
        return Ok(Value::Bool(false));
    }
    let Some(data) = host.stream_get_contents(&zlib_url(&path)) else {
        warn_open_failed(host, "readgzfile", inner(&path), b"rb");
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

/// `gzread` & co. are the plain stream functions applied to the gz stream
/// (their argument errors name the gz function).
fn alias(host: &mut dyn Host, target: &str, args: &[Value]) -> Result<Value, Thrown> {
    alias_as(host, target, target, args)
}

fn alias_as(
    host: &mut dyn Host,
    target: &str,
    name: &str,
    args: &[Value],
) -> Result<Value, Thrown> {
    let Some(b) = crate::lookup(target) else {
        return Ok(Value::Bool(false));
    };
    match (b.func)(host, args) {
        Err(Thrown::Error { class, message }) if message.starts_with(&format!("{target}(")) => {
            Err(Thrown::Error {
                class,
                message: format!("{name}{}", &message[target.len()..]),
            })
        }
        r => r,
    }
}

macro_rules! aliases {
    ($($name:ident => $target:literal as $gz:literal),* $(,)?) => {
        $(fn $name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            alias_as(host, $target, $gz, args)
        })*
    };
}
aliases! {
    fn_gzread => "fread" as "gzread",
    fn_gzwrite => "fwrite" as "gzwrite",
    fn_gzclose => "fclose" as "gzclose",
    fn_gzeof => "feof" as "gzeof",
    fn_gzgets => "fgets" as "gzgets",
    fn_gzgetc => "fgetc" as "gzgetc",
    fn_gzrewind => "rewind" as "gzrewind",
    fn_gztell => "ftell" as "gztell",
}

/// `gzseek` is `fseek` on the gz stream, whose seek op rejects SEEK_END.
fn fn_gzseek(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if int_arg(args, 2, 0) == 2 {
        host.raise_diagnostic(crate::E_WARNING, b"gzseek(): SEEK_END is not supported");
        return Ok(Value::Int(-1));
    }
    alias(host, "fseek", args)
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("gzdeflate", fn_gzdeflate),
            ("gzinflate", fn_gzinflate),
            ("gzcompress", fn_gzcompress),
            ("gzuncompress", fn_gzuncompress),
            ("gzencode", fn_gzencode),
            ("gzdecode", fn_gzdecode),
            ("zlib_encode", fn_zlib_encode),
            ("zlib_decode", fn_zlib_decode),
            ("zlib_get_coding_type", output::fn_zlib_get_coding_type),
            ("ob_gzhandler", output::fn_ob_gzhandler),
            ("gzopen", fn_gzopen),
            ("gzfile", fn_gzfile),
            ("readgzfile", fn_readgzfile),
            ("gzpassthru", fn_gzpassthru),
            ("gzread", fn_gzread),
            ("gzwrite", fn_gzwrite),
            ("gzputs", fn_gzwrite),
            ("gzclose", fn_gzclose),
            ("gzeof", fn_gzeof),
            ("gzgets", fn_gzgets),
            ("gzgetc", fn_gzgetc),
            ("gzrewind", fn_gzrewind),
            ("gzseek", fn_gzseek),
            ("gztell", fn_gztell),
        ],
    );
    context::register(m);
}

/// `ZLIB_*` / `FORCE_*` constants.
pub fn constants() -> Vec<(&'static str, Value)> {
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
        // The zlib headers the reference php was compiled against, not the
        // libz this process happens to link.
        ("ZLIB_VERSION", Value::str(crate::version::ZLIB_VERSION)),
        ("ZLIB_VERNUM", Value::Int(crate::version::ZLIB_VERNUM)),
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
