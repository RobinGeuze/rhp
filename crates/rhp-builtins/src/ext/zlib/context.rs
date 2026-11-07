//! The incremental contexts of ext/zlib/zlib.c: `deflate_init` /
//! `deflate_add` and `inflate_init` / `inflate_add` /
//! `inflate_get_status` / `inflate_get_read_len`. A context's z_stream is
//! the `DeflateContext` / `InflateContext` object's payload.

use std::cell::RefCell;
use std::rc::Rc;

use libz_sys as z;
use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use super::zraw::{ZStream, ENCODING_DEFLATE, ENCODING_GZIP, ENCODING_RAW};
use crate::zend::object_handlers::{ClassHandlers, ObjectHandlers};
use crate::{Host, Thrown};

pub(crate) const CLASSES: &[ClassHandlers] = &[
    ClassHandlers {
        class: "InflateContext",
        handlers: &ObjectHandlers::OPAQUE,
        create_object: true,
    },
    ClassHandlers {
        class: "DeflateContext",
        handlers: &ObjectHandlers::OPAQUE,
        create_object: true,
    },
];

pub const PRELUDE_PHP: &str = "final class InflateContext {
    public function __construct() { throw new Error('Cannot directly construct InflateContext, use inflate_init() instead'); }
}
final class DeflateContext {
    public function __construct() { throw new Error('Cannot directly construct DeflateContext, use deflate_init() instead'); }
}
";

struct Inflate {
    z: ZStream,
    dict: Option<Vec<u8>>,
    status: i32,
}

struct Deflate {
    z: ZStream,
}

/// `PHP_ZLIB_BUFFER_SIZE_GUESS`.
pub(crate) fn buffer_size_guess(n: usize) -> usize {
    (n as f64 * 1.015) as usize + 10 + 8 + 4 + 1
}

/// An option of the `$options` array (or object's properties).
fn option(options: Option<&Value>, name: &[u8]) -> Option<Value> {
    match options {
        Some(Value::Array(a)) => a.get(&rhp_values::ArrayKey::from_bytes(name)).map(deref),
        Some(Value::Object(o)) => o
            .borrow()
            .props()
            .get(&rhp_values::PhpStr::from_bytes(name))
            .map(deref),
        _ => None,
    }
}

/// `zlib_create_dictionary_string`: the dictionary, if any.
fn dictionary(
    host: &mut dyn Host,
    fname: &str,
    options: Option<&Value>,
) -> Result<Option<Vec<u8>>, Thrown> {
    let Some(d) = option(options, b"dictionary") else {
        return Ok(None);
    };
    match d {
        Value::Str(s) => Ok(Some(s.as_bytes().to_vec())),
        Value::Array(a) => {
            if a.is_empty() {
                return Ok(None);
            }
            let mut out = Vec::new();
            for v in a.values() {
                let s = crate::zend::zpp::zval_get_string(host, &deref(v))?;
                if s.is_empty() {
                    return Err(Thrown::error(
                        "ValueError",
                        format!("{fname}(): Argument #2 ($options) must not contain empty strings"),
                    ));
                }
                if s.contains(&0) {
                    return Err(Thrown::error(
                        "ValueError",
                        format!(
                            "{fname}(): Argument #2 ($options) must not contain strings with null bytes"
                        ),
                    ));
                }
                out.extend_from_slice(&s);
                out.push(0);
            }
            Ok(Some(out))
        }
        other => Err(Thrown::type_error(format!(
            "{fname}(): Argument #2 ($options) must be of type zero-terminated string or array, {} given",
            crate::zend::zpp::zval_value_name(host, &other)
        ))),
    }
}

fn long_option(options: Option<&Value>, name: &[u8], default: i64) -> i64 {
    option(options, name).map_or(default, |v| conv::to_int(&v))
}

/// The `$options` argument: an array or an object (`H`), else a TypeError.
fn options_arg(host: &dyn Host, args: &[Value], fname: &str) -> Result<Option<Value>, Thrown> {
    match args.get(1).map(deref) {
        None => Ok(None),
        Some(v @ (Value::Array(_) | Value::Object(_))) => Ok(Some(v)),
        Some(v) => Err(Thrown::type_error(format!(
            "{fname}(): Argument #2 ($options) must be of type array, {} given",
            crate::zend::zpp::zval_value_name(host, &v)
        ))),
    }
}

fn new_object<T: 'static>(host: &mut dyn Host, class: &[u8], state: T) -> Result<Value, Thrown> {
    let obj = host
        .instantiate(class, Vec::new())
        .ok_or_else(|| Thrown::error("Error", "zlib context class is not available"))?;
    if let Value::Object(h) = &obj {
        crate::zend::object_handlers::set_payload(h, Rc::new(RefCell::new(state)));
    }
    Ok(obj)
}

fn context_arg<T: 'static>(
    host: &dyn Host,
    args: &[Value],
    fname: &str,
    class: &str,
) -> Result<Rc<RefCell<T>>, Thrown> {
    let v = args.first().map(deref).unwrap_or(Value::Null);
    if let Some(p) = crate::zend::object_handlers::payload::<RefCell<T>>(&v) {
        return Ok(p);
    }
    Err(Thrown::type_error(format!(
        "{fname}(): Argument #1 ($context) must be of type {class}, {} given",
        crate::zend::zpp::zval_value_name(host, &v)
    )))
}

fn window_bits(encoding: i64, window: i64) -> i64 {
    if encoding < 0 {
        encoding + 15 - window
    } else {
        encoding - (15 - window)
    }
}

fn flush_arg(args: &[Value], fname: &str) -> Result<i64, Thrown> {
    let flush = args
        .get(2)
        .map_or(z::Z_SYNC_FLUSH as i64, |v| conv::to_int(&deref(v)));
    match flush as i32 {
        z::Z_NO_FLUSH | z::Z_PARTIAL_FLUSH | z::Z_SYNC_FLUSH | z::Z_FULL_FLUSH | z::Z_BLOCK
        | z::Z_FINISH
            if flush == flush as i32 as i64 =>
        {
            Ok(flush)
        }
        _ => Err(Thrown::error(
            "ValueError",
            format!("{fname}(): Argument #3 ($flush_mode) must be one of ZLIB_NO_FLUSH, ZLIB_PARTIAL_FLUSH, ZLIB_SYNC_FLUSH, ZLIB_FULL_FLUSH, ZLIB_BLOCK, or ZLIB_FINISH"),
        )),
    }
}

fn fn_inflate_init(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let encoding = args.first().map_or(0, |v| conv::to_int(&deref(v)));
    let options = options_arg(host, args, "inflate_init")?;
    let window = long_option(options.as_ref(), b"window", 15);
    if !(8..=15).contains(&window) {
        return Err(Thrown::error(
            "ValueError",
            format!("zlib window size (logarithm) ({window}) must be within 8..15"),
        ));
    }
    if ![ENCODING_RAW, ENCODING_GZIP, ENCODING_DEFLATE].contains(&encoding) {
        return Err(Thrown::error(
            "ValueError",
            "Encoding mode must be ZLIB_ENCODING_RAW, ZLIB_ENCODING_GZIP or ZLIB_ENCODING_DEFLATE",
        ));
    }
    let mut dict = dictionary(host, "inflate_init", options.as_ref())?;
    let Ok(mut zs) = ZStream::inflate_init(window_bits(encoding, window)) else {
        crate::warn(
            host,
            "inflate_init",
            "Failed allocating zlib.inflate context",
        )?;
        return Ok(Value::Bool(false));
    };
    if encoding == ENCODING_RAW {
        if let Some(d) = dict.take() {
            if zs.set_dictionary(&d) == z::Z_DATA_ERROR {
                crate::warn(
                    host,
                    "inflate_init",
                    "Dictionary does not match expected dictionary (incorrect adler32 hash)",
                )?;
            }
        }
    }
    new_object(
        host,
        b"InflateContext",
        Inflate {
            z: zs,
            dict,
            status: z::Z_OK,
        },
    )
}

fn fn_inflate_add(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ctx = context_arg::<Inflate>(host, args, "inflate_add", "InflateContext")?;
    let input = crate::zend::zpp::str_arg(host, args, 1, "inflate_add", "data")?;
    let flush = flush_arg(args, "inflate_add")?;
    let mut c = ctx.borrow_mut();
    if c.status == z::Z_STREAM_END {
        c.status = z::Z_OK;
        c.z.reset();
    }
    if input.is_empty() && flush != z::Z_FINISH as i64 {
        return Ok(Value::str(Vec::new()));
    }
    const CHUNK: usize = 8192;
    let mut out = vec![0u8; input.len().max(CHUNK)];
    c.z.set_io(&input, &mut out);
    let mut used;
    loop {
        let status = c.z.run(flush);
        used = out.len() - c.z.avail_out();
        c.status = status;
        let grow = match status {
            z::Z_OK => c.z.avail_out() == 0,
            z::Z_STREAM_END => false,
            z::Z_BUF_ERROR => flush == z::Z_FINISH as i64 && c.z.avail_out() == 0,
            z::Z_NEED_DICT => match c.dict.take() {
                Some(d) => {
                    if c.z.set_dictionary(&d) == z::Z_DATA_ERROR {
                        drop(c);
                        crate::warn(
                            host,
                            "inflate_add",
                            "Dictionary does not match expected dictionary (incorrect adler32 hash)",
                        )?;
                        return Ok(Value::Bool(false));
                    }
                    continue;
                }
                None => {
                    drop(c);
                    crate::warn(
                        host,
                        "inflate_add",
                        "Inflating this data requires a preset dictionary, please specify it in the options array of inflate_init()",
                    )?;
                    return Ok(Value::Bool(false));
                }
            },
            other => {
                drop(c);
                crate::warn(host, "inflate_add", &super::z_error(other))?;
                return Ok(Value::Bool(false));
            }
        };
        if !grow {
            break;
        }
        out.resize(out.len() + CHUNK, 0);
        c.z.set_in_tail(&input);
        c.z.set_out(&mut out, used);
    }
    out.truncate(used);
    Ok(Value::str(out))
}

fn fn_inflate_get_status(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ctx = context_arg::<Inflate>(host, args, "inflate_get_status", "InflateContext")?;
    let s = ctx.borrow().status;
    Ok(Value::Int(s as i64))
}

fn fn_inflate_get_read_len(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ctx = context_arg::<Inflate>(host, args, "inflate_get_read_len", "InflateContext")?;
    let n = ctx.borrow().z.total_in();
    Ok(Value::Int(n as i64))
}

fn fn_deflate_init(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let encoding = args.first().map_or(0, |v| conv::to_int(&deref(v)));
    let options = options_arg(host, args, "deflate_init")?;
    let o = options.as_ref();
    let level = long_option(o, b"level", -1);
    if !(-1..=9).contains(&level) {
        return Err(Thrown::error(
            "ValueError",
            "deflate_init(): \"level\" option must be between -1 and 9",
        ));
    }
    let memory = long_option(o, b"memory", 8);
    if !(1..=9).contains(&memory) {
        return Err(Thrown::error(
            "ValueError",
            "deflate_init(): \"memory\" option must be between 1 and 9",
        ));
    }
    let window = long_option(o, b"window", 15);
    if !(8..=15).contains(&window) {
        return Err(Thrown::error(
            "ValueError",
            "deflate_init(): \"window\" option must be between 8 and 15",
        ));
    }
    let strategy = long_option(o, b"strategy", z::Z_DEFAULT_STRATEGY as i64);
    if ![
        z::Z_FILTERED,
        z::Z_HUFFMAN_ONLY,
        z::Z_RLE,
        z::Z_FIXED,
        z::Z_DEFAULT_STRATEGY,
    ]
    .iter()
    .any(|&s| s as i64 == strategy)
    {
        return Err(Thrown::error(
            "ValueError",
            "deflate_init(): \"strategy\" option must be one of ZLIB_FILTERED, ZLIB_HUFFMAN_ONLY, ZLIB_RLE, ZLIB_FIXED, or ZLIB_DEFAULT_STRATEGY",
        ));
    }
    if ![ENCODING_RAW, ENCODING_GZIP, ENCODING_DEFLATE].contains(&encoding) {
        return Err(Thrown::error(
            "ValueError",
            "deflate_init(): Argument #1 ($encoding) must be one of ZLIB_ENCODING_RAW, ZLIB_ENCODING_GZIP, or ZLIB_ENCODING_DEFLATE",
        ));
    }
    let dict = dictionary(host, "deflate_init", o)?;
    let Ok(mut zs) = ZStream::deflate_init(level, window_bits(encoding, window), memory, strategy)
    else {
        crate::warn(
            host,
            "deflate_init",
            "Failed allocating zlib.deflate context",
        )?;
        return Ok(Value::Bool(false));
    };
    if let Some(d) = dict {
        zs.set_dictionary(&d);
    }
    new_object(host, b"DeflateContext", Deflate { z: zs })
}

fn fn_deflate_add(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ctx = context_arg::<Deflate>(host, args, "deflate_add", "DeflateContext")?;
    let input = crate::zend::zpp::str_arg(host, args, 1, "deflate_add", "data")?;
    let flush = flush_arg(args, "deflate_add")?;
    if input.is_empty() && flush != z::Z_FINISH as i64 {
        return Ok(Value::str(Vec::new()));
    }
    let mut c = ctx.borrow_mut();
    let size = buffer_size_guess(input.len()).max(64);
    let mut out = vec![0u8; size];
    c.z.set_io(&input, &mut out);
    let mut status;
    loop {
        if c.z.avail_out() == 0 {
            let used = out.len();
            out.resize(used + 64, 0);
            c.z.set_in_tail(&input);
            c.z.set_out(&mut out, used);
        }
        status = c.z.run(flush);
        if !(status == z::Z_OK && c.z.avail_out() == 0) {
            break;
        }
    }
    match status {
        z::Z_OK | z::Z_STREAM_END => {
            let used = out.len() - c.z.avail_out();
            out.truncate(used);
            if status == z::Z_STREAM_END {
                c.z.reset();
            }
            Ok(Value::str(out))
        }
        other => {
            drop(c);
            crate::warn(
                host,
                "deflate_add",
                &format!("zlib error ({})", super::z_error(other)),
            )?;
            Ok(Value::Bool(false))
        }
    }
}

pub(crate) fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("inflate_init", fn_inflate_init),
            ("inflate_add", fn_inflate_add),
            ("inflate_get_status", fn_inflate_get_status),
            ("inflate_get_read_len", fn_inflate_get_read_len),
            ("deflate_init", fn_deflate_init),
            ("deflate_add", fn_deflate_add),
        ],
    );
}
