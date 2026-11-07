//! Zend/zend_exceptions.c: the string forms of a backtrace and of a
//! Throwable, and the engine helpers the prelude's exception classes call.

use std::cell::Cell;

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::{Host, Thrown};

thread_local! {
    /// EG(exception_ignore_args) (`zend.exception_ignore_args`).
    static IGNORE_ARGS: Cell<bool> = const { Cell::new(false) };
    /// EG(exception_string_param_max_len).
    static STRING_PARAM_MAX_LEN: Cell<usize> = const { Cell::new(15) };
}

/// `zend.exception_ignore_args`: traces leave out `args`.
pub fn ignore_args() -> bool {
    IGNORE_ARGS.with(Cell::get)
}

pub(crate) fn set_ignore_args(v: bool) {
    IGNORE_ARGS.with(|c| c.set(v));
}

/// `zend.exception_string_param_max_len` (validated: 0..=1000000).
pub fn string_param_max_len() -> usize {
    STRING_PARAM_MAX_LEN.with(Cell::get)
}

pub(crate) fn set_string_param_max_len(v: usize) {
    STRING_PARAM_MAX_LEN.with(|c| c.set(v));
}

pub(crate) fn reset_ini() {
    set_ignore_args(false);
    set_string_param_max_len(15);
}

/// One trace argument as `zend_exceptions.c::_build_trace_args` renders it.
fn trace_arg(host: &dyn Host, v: &Value, out: &mut Vec<u8>) {
    if append_zval(host, v, out) {
        return;
    }
    match deref(v) {
        Value::Array(_) => out.extend_from_slice(b"Array"),
        Value::Resource(r) => {
            out.extend_from_slice(b"Resource id #");
            out.extend_from_slice(r.id.to_string().as_bytes());
        }
        Value::Closure(_) => out.extend_from_slice(b"Object(Closure)"),
        o @ Value::Object(_) => {
            let name = host.object_class_name(&o).unwrap_or_default();
            out.extend_from_slice(b"Object(");
            out.extend_from_slice(&crate::ext::standard::var::c_str(name));
            out.push(b')');
        }
        _ => unreachable!(),
    }
}

/// `smart_str_append_zval` with `zend.exception_string_param_max_len`: a
/// scalar (a string quoted, escaped and cut) or an enum case. `false` for
/// anything else.
pub fn append_zval(host: &dyn Host, v: &Value, out: &mut Vec<u8>) -> bool {
    match deref(v) {
        Value::Null => out.extend_from_slice(b"NULL"),
        Value::Bool(b) => out.extend_from_slice(if b { b"true" } else { b"false" }),
        Value::Int(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::Float(f) => {
            let s = rhp_values::fmt::float_echo(f);
            let has_dot = s.contains(&b'.');
            out.extend_from_slice(&s);
            if f.is_finite() && !has_dot {
                out.extend_from_slice(b".0");
            }
        }
        Value::Str(s) => {
            let b = s.as_bytes();
            let max = string_param_max_len();
            out.push(b'\'');
            append_escaped(out, &b[..b.len().min(max)]);
            if b.len() > max {
                out.extend_from_slice(b"...");
            }
            out.push(b'\'');
        }
        o @ Value::Object(_) => {
            let Value::Object(h) = &o else { unreachable!() };
            if !host.is_enum_class(h.borrow().class) {
                return false;
            }
            out.extend_from_slice(&host.object_class_name(&o).unwrap_or_default());
            out.extend_from_slice(b"::");
            let case = host.object_prop(&o, b"name").unwrap_or(Value::Null);
            out.extend_from_slice(&conv::to_string_bytes(&case));
        }
        _ => return false,
    }
    true
}

/// `smart_str_append_escaped`: C-style escapes for control / non-ASCII bytes.
fn append_escaped(out: &mut Vec<u8>, s: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for &c in s {
        if c < 32 || c == b'\\' || c > 126 {
            out.push(b'\\');
            match c {
                b'\n' => out.push(b'n'),
                b'\r' => out.push(b'r'),
                b'\t' => out.push(b't'),
                0x0c => out.push(b'f'),
                0x0b => out.push(b'v'),
                b'\\' => out.push(b'\\'),
                0x1b => out.push(b'e'),
                _ => {
                    out.push(b'x');
                    out.push(HEX[(c >> 4) as usize]);
                    out.push(HEX[(c & 0xf) as usize]);
                }
            }
        } else {
            out.push(c);
        }
    }
}

/// `zend_trace_to_string`: render a `getTrace()` array as php's
/// `getTraceAsString()` text (`#0 file(line): Class->fn(args)` … `#N {main}`).
pub fn trace_to_string(host: &dyn Host, trace: &Value) -> Vec<u8> {
    trace_to_string_ex(host, trace, &mut |_| {})
}

/// [`trace_to_string`], passing php's warnings about malformed frames to
/// `warn`.
pub fn trace_to_string_ex(host: &dyn Host, trace: &Value, warn: &mut dyn FnMut(String)) -> Vec<u8> {
    use rhp_values::ArrayKey as K;
    let mut out = Vec::new();
    let mut num = 0usize;
    if let Value::Array(frames) = deref(trace) {
        for (key, frame) in frames.iter() {
            let Value::Array(f) = deref(frame) else {
                // ZEND_HASH_FOREACH_NUM_KEY_VAL: a string key reads as 0.
                let index = match key {
                    K::Int(i) => i as u64,
                    K::Str(_) => 0,
                };
                warn(format!("Expected array for frame {index}"));
                continue;
            };
            build_trace_string(host, &f, num, &mut out, warn);
            num += 1;
        }
    }
    out.push(b'#');
    out.extend_from_slice(num.to_string().as_bytes());
    out.extend_from_slice(b" {main}");
    out
}

/// `_build_trace_string`: one frame. String values print up to a NUL (C
/// strings), so an anonymous class shows as `class@anonymous`.
fn build_trace_string(
    host: &dyn Host,
    f: &rhp_values::PhpArray,
    num: usize,
    out: &mut Vec<u8>,
    warn: &mut dyn FnMut(String),
) {
    use rhp_values::ArrayKey as K;
    let c_str = |b: &[u8]| -> usize { b.iter().position(|&c| c == 0).unwrap_or(b.len()) };
    out.push(b'#');
    out.extend_from_slice(num.to_string().as_bytes());
    out.push(b' ');
    match f.get(&K::from_bytes(b"file")).map(deref) {
        Some(Value::Str(file)) => {
            let line = match f.get(&K::from_bytes(b"line")).map(deref) {
                Some(Value::Int(n)) => n,
                Some(_) => {
                    warn("Line is not an int".into());
                    0
                }
                None => 0,
            };
            out.extend_from_slice(file.as_bytes());
            out.push(b'(');
            out.extend_from_slice(line.to_string().as_bytes());
            out.extend_from_slice(b"): ");
        }
        Some(_) => {
            warn("File name is not a string".into());
            out.extend_from_slice(b"[unknown file]: ");
        }
        None => out.extend_from_slice(b"[internal function]: "),
    }
    for key in ["class", "type", "function"] {
        match f.get(&K::from_bytes(key.as_bytes())).map(deref) {
            Some(Value::Str(v)) => {
                let b = v.as_bytes();
                out.extend_from_slice(&b[..c_str(b)]);
            }
            Some(_) => {
                warn(format!("Value for {key} is not a string"));
                out.extend_from_slice(b"[unknown]");
            }
            None => {}
        }
    }
    out.push(b'(');
    match f.get(&K::from_bytes(b"args")).map(deref) {
        Some(Value::Array(args)) => {
            let mut first = true;
            for (k, a) in args.iter() {
                if !first {
                    out.extend_from_slice(b", ");
                }
                first = false;
                if let K::Str(name) = k {
                    out.extend_from_slice(name.as_bytes());
                    out.extend_from_slice(b": ");
                }
                trace_arg(host, a, out);
            }
        }
        Some(_) => warn("args element is not an array".into()),
        None => {}
    }
    out.extend_from_slice(b")\n");
}

/// `__exception_trace_string($trace)`: backs `Exception::getTraceAsString()`.
fn fn_exception_trace_string(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut warnings = Vec::new();
    let s = trace_to_string_ex(host, &args[0], &mut |w| warnings.push(w));
    for w in warnings {
        host.emit_diagnostic(crate::E_WARNING, w.as_bytes())?;
    }
    Ok(Value::str(s))
}

/// `Exception::__toString()` (zend_exceptions.c): the chain from the innermost
/// `previous` outwards, each as `Class: msg in file:line\nStack trace:\n…`,
/// joined by `\n\nNext `.
pub fn throwable_to_string(host: &mut dyn Host, exc: &Value) -> Result<Vec<u8>, Thrown> {
    use crate::zend::zpp::{zval_get_long, zval_get_string};
    let mut s: Vec<u8> = Vec::new();
    let mut cur = deref(exc);
    let mut seen: Vec<u32> = Vec::new();
    while let Value::Object(h) = &cur {
        let id = h.borrow().id;
        let name = host.object_class_name(&cur).unwrap_or_default();
        if seen.contains(&id) || !host.reflect_is_a(&name, b"Throwable") {
            break;
        }
        seen.push(id);
        let prop = |host: &dyn Host, n: &[u8]| {
            host.object_prop(&cur, n)
                .map(|v| deref(&v))
                .unwrap_or(Value::Null)
        };
        let v = prop(host, b"message");
        let mut message = zval_get_string(host, &v)?;
        let v = prop(host, b"file");
        let file = zval_get_string(host, &v)?;
        let v = prop(host, b"line");
        let line = zval_get_long(host, &v)?;
        let mut warnings = Vec::new();
        let trace = trace_to_string_ex(host, &prop(host, b"trace"), &mut |w| warnings.push(w));
        for w in warnings {
            host.emit_diagnostic(crate::E_WARNING, w.as_bytes())?;
        }
        if (name == b"TypeError" || name == b"ArgumentCountError")
            && message.windows(12).any(|w| w == b", called in ")
        {
            message.extend_from_slice(b" and defined");
        }
        let mut item = name.clone();
        if !message.is_empty() {
            item.extend_from_slice(b": ");
            item.extend_from_slice(&message);
        }
        item.extend_from_slice(b" in ");
        item.extend_from_slice(&file);
        item.push(b':');
        item.extend_from_slice(line.to_string().as_bytes());
        item.extend_from_slice(b"\nStack trace:\n");
        item.extend_from_slice(&trace);
        if !s.is_empty() {
            item.extend_from_slice(b"\n\nNext ");
            item.extend_from_slice(&s);
        }
        s = item;
        cur = prop(host, b"previous");
    }
    Ok(s)
}

/// `__throwable_string($e)`: backs `Exception::__toString()` / `Error::__toString()`.
fn fn_throwable_string(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(throwable_to_string(host, &args[0])?))
}

pub fn register(m: &mut crate::Registry) {
    crate::register_helpers(
        m,
        &[
            (
                "__exception_trace_string",
                fn_exception_trace_string,
                1,
                Some(1),
            ),
            ("__throwable_string", fn_throwable_string, 1, Some(1)),
        ],
    );
}
