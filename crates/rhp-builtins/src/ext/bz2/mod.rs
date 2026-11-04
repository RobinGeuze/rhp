//! The `bz2` extension (php-src ext/bz2): `bzcompress`/`bzdecompress` with
//! php's exact libbz2 calls (bzip2-sys builds the reference libbz2, so output
//! bytes match), the `compress.bzip2://` wrapper, and the `bzopen` family as
//! stream aliases.

use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;

use bzip2::{Action, Compress, Compression, Decompress, Status};
use rhp_semantics::deref;
use rhp_values::{conv, Value};

use crate::streams::{self, StatInfo, StreamWrapper, WrapperReg};
use crate::{Host, Thrown};

const BZ_SEQUENCE_ERROR: i64 = -1;
const BZ_PARAM_ERROR: i64 = -2;
const BZ_MEM_ERROR: i64 = -3;
const BZ_DATA_ERROR: i64 = -4;
const BZ_DATA_ERROR_MAGIC: i64 = -5;

fn int_arg(args: &[Value], i: usize, default: i64) -> i64 {
    match args.get(i).map(deref) {
        None => default,
        Some(v) => conv::to_int(&v),
    }
}

fn err_code(e: bzip2::Error) -> i64 {
    match e {
        bzip2::Error::Sequence => BZ_SEQUENCE_ERROR,
        bzip2::Error::Data => BZ_DATA_ERROR,
        bzip2::Error::DataMagic => BZ_DATA_ERROR_MAGIC,
        bzip2::Error::Param => BZ_PARAM_ERROR,
    }
}

/// `BZ2_bzBuffToBuffCompress(…, block_size, 0, work_factor)`.
pub fn compress(data: &[u8], block_size: u32, work_factor: u32) -> Vec<u8> {
    let mut c = Compress::new(Compression::new(block_size), work_factor);
    let mut out = Vec::with_capacity(data.len() + data.len() / 100 + 600);
    loop {
        let consumed = c.total_in() as usize;
        match c.compress_vec(&data[consumed..], &mut out, Action::Finish) {
            Ok(Status::StreamEnd) | Err(_) => break,
            Ok(_) => out.reserve(out.capacity().max(1024)),
        }
    }
    out
}

/// php's `bzdecompress` loop: `Ok(bytes)` or the libbz2 error code. Truncated
/// input yields what was decoded (BZ_OK with input exhausted), as in php.
pub fn decompress(data: &[u8], small: bool) -> Result<Vec<u8>, i64> {
    let mut d = Decompress::new(small);
    let mut out: Vec<u8> = Vec::with_capacity(data.len() * 2);
    loop {
        let consumed = d.total_in() as usize;
        match d.decompress_vec(&data[consumed..], &mut out) {
            Ok(Status::StreamEnd) => return Ok(out),
            Ok(Status::MemNeeded) => return Err(BZ_MEM_ERROR),
            Ok(_) => {
                if (d.total_in() as usize) < data.len() {
                    out.reserve(data.len().max(1));
                } else {
                    return Ok(out);
                }
            }
            Err(e) => return Err(err_code(e)),
        }
    }
}

fn fn_bzcompress(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = conv::to_string_bytes(&deref(&args[0]));
    let block = int_arg(args, 1, 4);
    let work = int_arg(args, 2, 0);
    if !(1..=9).contains(&block) {
        return Err(Thrown::error(
            "ValueError",
            "bzcompress(): Argument #2 ($block_size) must be between 1 and 9",
        ));
    }
    if !(0..=250).contains(&work) {
        return Err(Thrown::error(
            "ValueError",
            "bzcompress(): Argument #3 ($work_factor) must be between 0 and 250",
        ));
    }
    Ok(Value::str(compress(&data, block as u32, work as u32)))
}

fn fn_bzdecompress(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = conv::to_string_bytes(&deref(&args[0]));
    let small = args
        .get(1)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    Ok(match decompress(&data, small) {
        Ok(out) => Value::str(out),
        Err(code) => Value::Int(code),
    })
}

// --- compress.bzip2:// -------------------------------------------------------

const SCHEME_PREFIX: &[u8] = b"compress.bzip2://";

fn inner(url: &[u8]) -> &[u8] {
    url.strip_prefix(SCHEME_PREFIX).unwrap_or(url)
}

pub struct Bzip2Wrapper;

impl StreamWrapper for Bzip2Wrapper {
    fn exists(&self, url: &[u8]) -> bool {
        streams::default_exists(inner(url))
    }
    fn get_contents(&self, url: &[u8]) -> Option<Vec<u8>> {
        let raw = streams::default_get_contents(inner(url))?;
        // php's bzip2 stream reads the first bzip2 stream only.
        decompress(&raw, false).ok()
    }
    fn stat(&self, url: &[u8]) -> Option<StatInfo> {
        streams::default_stat(inner(url))
    }
    fn put_contents(&self, url: &[u8], data: &[u8]) -> Option<usize> {
        // php's bzip2 stream: BZ2_bzWriteOpen(blockSize100k 9, workFactor 0).
        streams::default_put_contents(inner(url), &compress(data, 9, 0)).map(|_| data.len())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn wrappers() -> Vec<WrapperReg> {
    vec![WrapperReg {
        scheme: "compress.bzip2",
        factory: || Rc::new(Bzip2Wrapper),
    }]
}

fn fn_bzopen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let file = deref(&args[0]);
    let mode = conv::to_string_bytes(&deref(&args[1]));
    if mode != b"r" && mode != b"w" {
        return Err(Thrown::error(
            "ValueError",
            "bzopen(): Argument #2 ($mode) must be either \"r\" or \"w\"",
        ));
    }
    if let Value::Resource(_) = file {
        // Wrapping an already-open stream is not supported here.
        return Ok(Value::Bool(false));
    }
    let path = conv::to_string_bytes(&file);
    if path.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "bzopen(): Argument #1 ($file) must not be empty",
        ));
    }
    let mut url = SCHEME_PREFIX.to_vec();
    url.extend_from_slice(&path);
    let fmode: &[u8] = if mode == b"r" { b"rb" } else { b"wb" };
    let rel = inner(&path).to_vec();
    if mode == b"r" && !rel.windows(3).any(|w| w == b"://") {
        // libbz2 opens lazily: a readable file that isn't bzip2 still opens,
        // and the first read fails with BZ_DATA_ERROR_MAGIC.
        if let Ok(raw) = std::fs::read(crate::vcwd::os_path(&rel)) {
            if let Err(code) = decompress(&raw, false) {
                if let Some(res) = host.stream_open(b"php://memory", b"rb") {
                    if let Value::Resource(r) = &res {
                        BZ_ERRORS.with(|t| t.borrow_mut().insert(r.id, code));
                    }
                    return Ok(res);
                }
            }
        }
    }
    Ok(match host.stream_open(&url, fmode) {
        Some(v) => v,
        None => {
            crate::ext::zlib::warn_open_failed(host, "bzopen", &rel, fmode);
            Value::Bool(false)
        }
    })
}

thread_local! {
    /// The libbz2 error state of bzip2 streams whose read failed, by resource id.
    static BZ_ERRORS: std::cell::RefCell<HashMap<u32, i64>> = std::cell::RefCell::new(HashMap::new());
}

/// Forget stream error states (a new request: resource ids restart).
pub(crate) fn reset_state() {
    BZ_ERRORS.with(|e| e.borrow_mut().clear());
}

fn bz_errno(args: &[Value]) -> i64 {
    match args.first().map(deref) {
        Some(Value::Resource(r)) => BZ_ERRORS.with(|t| t.borrow().get(&r.id).copied().unwrap_or(0)),
        _ => 0,
    }
}

/// `BZ2_bzerror`'s names, indexed by `-errno`.
fn bz_errstr(code: i64) -> &'static str {
    const NAMES: [&str; 10] = [
        "OK",
        "SEQUENCE_ERROR",
        "PARAM_ERROR",
        "MEM_ERROR",
        "DATA_ERROR",
        "DATA_ERROR_MAGIC",
        "IO_ERROR",
        "UNEXPECTED_EOF",
        "OUTBUFF_FULL",
        "CONFIG_ERROR",
    ];
    NAMES.get((-code).max(0) as usize).copied().unwrap_or("???")
}

fn alias(host: &mut dyn Host, target: &str, args: &[Value]) -> Result<Value, Thrown> {
    match crate::lookup(target) {
        Some(b) => (b.func)(host, args),
        None => Ok(Value::Bool(false)),
    }
}

fn fn_bzread(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if let Some(len) = args.get(1) {
        if conv::to_int(&deref(len)) < 0 {
            return Err(Thrown::error(
                "ValueError",
                "bzread(): Argument #2 ($length) must be greater than or equal to 0",
            ));
        }
    }
    if bz_errno(args) < 0 {
        return Ok(Value::Bool(false));
    }
    let mut a = args.to_vec();
    if a.len() < 2 {
        a.push(Value::Int(1024));
    }
    alias(host, "fread", &a)
}
fn fn_bzwrite(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    alias(host, "fwrite", args)
}
fn fn_bzclose(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    alias(host, "fclose", args)
}
fn fn_bzflush(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}
fn fn_bzerrno(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(bz_errno(args)))
}
fn fn_bzerrstr(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(bz_errstr(bz_errno(args))))
}
fn fn_bzerror(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = bz_errno(args);
    let mut a = rhp_values::PhpArray::new();
    a.insert(rhp_values::ArrayKey::from_bytes(b"errno"), Value::Int(code));
    a.insert(
        rhp_values::ArrayKey::from_bytes(b"errstr"),
        Value::str(bz_errstr(code)),
    );
    Ok(Value::Array(a))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("bzcompress", fn_bzcompress),
            ("bzdecompress", fn_bzdecompress),
            ("bzopen", fn_bzopen),
            ("bzread", fn_bzread),
            ("bzwrite", fn_bzwrite),
            ("bzflush", fn_bzflush),
            ("bzclose", fn_bzclose),
            ("bzerrno", fn_bzerrno),
            ("bzerrstr", fn_bzerrstr),
            ("bzerror", fn_bzerror),
        ],
    );
}
