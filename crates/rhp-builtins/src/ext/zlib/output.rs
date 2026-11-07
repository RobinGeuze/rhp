//! zlib's output compression (ext/zlib/zlib.c): the "zlib output
//! compression" / `ob_gzhandler` output handler, `zlib.output_compression`
//! started at request startup or by ini_set, and the `ob_gzhandler()`
//! function. Only one of the handlers can run (they conflict), so their
//! state is per thread, as php's ZLIBG.

use std::cell::RefCell;

use libz_sys as z;
use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, Value};

use super::context::buffer_size_guess;
use super::zraw::{ZStream, ENCODING_DEFLATE, ENCODING_GZIP, MAX_MEM_LEVEL};
use crate::module::{OutputHandlerAlias, OutputHandlerCtx};
use crate::{Host, ObCall, Thrown};

const OUTPUT_START: i64 = 0x01;
const OUTPUT_CLEAN: i64 = 0x02;
const OUTPUT_FLUSH: i64 = 0x04;
const OUTPUT_FINAL: i64 = 0x08;
/// PHP_OUTPUT_HANDLER_STDFLAGS.
const STDFLAGS: i64 = 0x70;
/// PHP_OUTPUT_HANDLER_DEFAULT_SIZE.
const DEFAULT_SIZE: i64 = 0x4000;
/// PHP_ZLIB_OUTPUT_HANDLER_NAME.
pub(crate) const HANDLER_NAME: &str = "zlib output compression";

const CONFLICTS: &[&str] = &[
    HANDLER_NAME,
    "ob_gzhandler",
    "mb_output_handler",
    "URL-Rewriter",
];

/// The output handlers this extension registers by name.
pub(crate) const OUTPUT_HANDLERS: &[OutputHandlerAlias] = &[
    OutputHandlerAlias {
        name: "ob_gzhandler",
        func: output_handler,
        conflicts: CONFLICTS,
        init: Some(handler_init),
    },
    OutputHandlerAlias {
        name: HANDLER_NAME,
        func: output_handler,
        conflicts: CONFLICTS,
        init: Some(handler_init),
    },
];

/// php_zlib_context for the output handler.
#[derive(Default)]
struct HCtx {
    z: Option<ZStream>,
    buffer: Vec<u8>,
}

/// ZLIBG.
#[derive(Default)]
struct State {
    /// The encoding the request accepts (`ENCODING_GZIP` /
    /// `ENCODING_DEFLATE`, 0 when not looked up or none).
    compression_coding: i64,
    output_compression: i64,
    handler_registered: bool,
    handler: HCtx,
    /// `ob_gzhandler()` called as a function.
    ob_gzhandler: Option<HCtx>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

/// A new request: php's RSHUTDOWN/RINIT reset.
pub(crate) fn reset_state() {
    STATE.with(|s| *s.borrow_mut() = State::default());
}

/// OnUpdate_zlib_output_compression's parse: "on"/"off", else a quantity.
fn parse_compression(v: &[u8]) -> i64 {
    if v.eq_ignore_ascii_case(b"off") {
        0
    } else if v.eq_ignore_ascii_case(b"on") {
        1
    } else {
        crate::main::ini::ini_parse_quantity(v) as i32 as i64
    }
}

/// `php_zlib_output_encoding`: the accepted encoding from
/// `$_SERVER['HTTP_ACCEPT_ENCODING']`, looked up once.
fn output_encoding(host: &mut dyn Host) -> i64 {
    let known = with(|s| s.compression_coding);
    if known != 0 {
        return known;
    }
    let enc = match host.global_var(b"_SERVER") {
        Some(Value::Array(a)) => a
            .get(&ArrayKey::from_bytes(b"HTTP_ACCEPT_ENCODING"))
            .map(|v| conv::to_string_bytes(&deref(v))),
        _ => None,
    };
    let coding = match enc {
        Some(e) if contains(&e, b"gzip") => ENCODING_GZIP,
        Some(e) if contains(&e, b"deflate") => ENCODING_DEFLATE,
        _ => 0,
    };
    with(|s| s.compression_coding = coding);
    coding
}

/// `strstr` (the string is a C string).
fn contains(hay: &[u8], needle: &[u8]) -> bool {
    let hay = &hay[..hay.iter().position(|&c| c == 0).unwrap_or(hay.len())];
    hay.windows(needle.len()).any(|w| w == needle)
}

/// `php_zlib_output_handler_init`'s ZLIBG part.
fn handler_init(chunk: usize) {
    with(|s| {
        if s.output_compression == 0 {
            s.output_compression = if chunk != 0 {
                chunk as i64
            } else {
                DEFAULT_SIZE
            };
        }
        s.handler_registered = true;
        s.handler = HCtx::default();
    });
}

fn level(host: &dyn Host) -> i64 {
    host.ini_get(b"zlib.output_compression_level")
        .map_or(-1, |v| crate::main::ini::c_atol(&v))
}

/// `php_zlib_output_handler_ex`: compress `input`; `None` is FAILURE.
fn handler_ex(cx: &mut HCtx, op: i64, input: &[u8], level: i64, coding: i64) -> Option<Vec<u8>> {
    let init = || ZStream::deflate_init(level, coding, MAX_MEM_LEVEL as i64, 0).ok();
    if op & OUTPUT_START != 0 {
        cx.z = Some(init()?);
    }
    if op & OUTPUT_CLEAN != 0 {
        cx.z = None;
        if op & OUTPUT_FINAL != 0 {
            return Some(Vec::new());
        }
        cx.z = Some(init()?);
        cx.buffer.clear();
        return Some(Vec::new());
    }
    cx.buffer.extend_from_slice(input);
    let mut out = vec![0u8; buffer_size_guess(input.len())];
    let flags = if op & OUTPUT_FINAL != 0 {
        z::Z_FINISH
    } else if op & OUTPUT_FLUSH != 0 {
        z::Z_FULL_FLUSH
    } else {
        z::Z_SYNC_FLUSH
    };
    let zs = cx.z.as_mut()?;
    zs.set_io(&cx.buffer, &mut out);
    let status = zs.run(flags as i64);
    match status {
        z::Z_OK if flags == z::Z_FINISH => {
            cx.z = None;
            return None;
        }
        z::Z_OK | z::Z_STREAM_END => {
            let left = zs.avail_in();
            let used = out.len() - zs.avail_out();
            let n = cx.buffer.len();
            cx.buffer.drain(..n - left);
            out.truncate(used);
        }
        _ => {
            cx.z = None;
            return None;
        }
    }
    if op & OUTPUT_FINAL != 0 {
        cx.z = None;
    }
    Some(out)
}

/// `sapi_add_header_ex`: sapi_header_op with REPLACE or ADD.
fn add_header(host: &mut dyn Host, line: &[u8], replace: bool) -> Result<bool, Thrown> {
    use crate::main::sapi::{header_op, HeaderOp};
    header_op(
        host,
        if replace {
            HeaderOp::Replace(line, 0)
        } else {
            HeaderOp::Add(line, 0)
        },
    )
}

/// `php_zlib_output_handler`.
fn output_handler(host: &mut dyn Host, cx: &mut OutputHandlerCtx) -> bool {
    let op = cx.op;
    let coding = output_encoding(host);
    if coding == 0 {
        if op & OUTPUT_START != 0 && op != (OUTPUT_START | OUTPUT_CLEAN | OUTPUT_FINAL) {
            let _ = add_header(host, b"Vary: Accept-Encoding", false);
        }
        return false;
    }
    let level = level(host);
    let Some(out) = with(|s| handler_ex(&mut s.handler, op, cx.input, level, coding)) else {
        return false;
    };
    if (op & OUTPUT_CLEAN == 0 || (op & OUTPUT_START != 0 && op & OUTPUT_FINAL == 0))
        && op & OUTPUT_START != 0
    {
        let sent = host.response().is_some_and(|r| r.sent);
        if sent || with(|s| s.output_compression) == 0 {
            with(|s| s.handler.z = None);
            return false;
        }
        let line: &[u8] = if coding == ENCODING_GZIP {
            b"Content-Encoding: gzip"
        } else {
            b"Content-Encoding: deflate"
        };
        let _ = add_header(host, line, true);
        let _ = add_header(host, b"Vary: Accept-Encoding", false);
        cx.immutable = true;
    }
    cx.out = out;
    true
}

/// The handlers on the stack (`php_output_handler_started`).
fn handler_started(host: &mut dyn Host, name: &str) -> bool {
    match host.output_control(ObCall::ListHandlers) {
        Ok(Value::Array(a)) => a
            .iter()
            .any(|(_, v)| conv::to_string_bytes(&deref(v)) == name.as_bytes()),
        _ => false,
    }
}

/// `php_zlib_output_compression_start`.
fn compression_start(host: &mut dyn Host) -> Result<(), Thrown> {
    let oc = with(|s| s.output_compression);
    if oc == 0 {
        return Ok(());
    }
    if oc == 1 {
        with(|s| s.output_compression = DEFAULT_SIZE);
    }
    if output_encoding(host) == 0 {
        return Ok(());
    }
    let chunk = with(|s| s.output_compression);
    handler_init(chunk as usize);
    let started = host.output_control(ObCall::StartInternal {
        name: HANDLER_NAME,
        func: output_handler,
        chunk_size: chunk.max(0) as usize,
    })?;
    if matches!(started, Value::Bool(true)) {
        if let Some(h) = host
            .ini_get(b"zlib.output_handler")
            .filter(|h| !h.is_empty())
        {
            host.output_control(ObCall::Start {
                callback: Value::str(h),
                chunk_size: chunk,
                flags: STDFLAGS,
            })?;
        }
    }
    Ok(())
}

/// PHP_RINIT_FUNCTION(zlib): start compression when the directive asks.
pub fn request_startup(host: &mut dyn Host) -> Result<(), Thrown> {
    let v = host.ini_get(b"zlib.output_compression").unwrap_or_default();
    with(|s| {
        s.compression_coding = 0;
        s.output_compression = parse_compression(&v);
    });
    if !with(|s| s.handler_registered) {
        compression_start(host)?;
    }
    Ok(())
}

/// OnUpdate_zlib_output_compression at runtime: whether the value is
/// accepted (the output may not have been sent yet).
pub(crate) fn ini_update(
    host: &mut dyn Host,
    name: &[u8],
    v: &[u8],
    func: &str,
) -> Result<bool, Thrown> {
    if name != b"zlib.output_compression" {
        return Ok(true);
    }
    let value = parse_compression(v);
    if host.response().is_some_and(|r| r.sent) {
        crate::warn(
            host,
            func,
            "Cannot change zlib.output_compression - headers already sent",
        )?;
        return Ok(false);
    }
    with(|s| s.output_compression = value);
    if value != 0 && !handler_started(host, HANDLER_NAME) {
        compression_start(host)?;
    }
    Ok(true)
}

/// `ob_gzhandler(string $data, int $flags): string|false`.
pub(crate) fn fn_ob_gzhandler(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = crate::zend::zpp::str_arg(host, args, 0, "ob_gzhandler", "data")?;
    let flags = args.get(1).map_or(0, |v| conv::to_int(&deref(v)));
    let coding = output_encoding(host);
    if coding == 0 {
        return Ok(Value::Bool(false));
    }
    if flags & OUTPUT_START != 0 {
        let line: &[u8] = if coding == ENCODING_GZIP {
            b"Content-Encoding: gzip"
        } else {
            b"Content-Encoding: deflate"
        };
        add_header(host, line, true)?;
        add_header(host, b"Vary: Accept-Encoding", false)?;
    }
    let level = level(host);
    let out = with(|s| {
        let cx = s.ob_gzhandler.get_or_insert_with(HCtx::default);
        let r = handler_ex(cx, flags, &data, level, coding);
        if r.is_none() {
            s.ob_gzhandler = None;
        }
        r
    });
    Ok(match out {
        Some(o) => Value::str(o),
        None => Value::Bool(false),
    })
}

/// `zlib_get_coding_type(): string|false`.
pub(crate) fn fn_zlib_get_coding_type(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(match with(|s| s.compression_coding) {
        ENCODING_GZIP => Value::str("gzip"),
        ENCODING_DEFLATE => Value::str("deflate"),
        _ => Value::Bool(false),
    })
}
