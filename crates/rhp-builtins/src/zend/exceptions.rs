//! Zend/zend_exceptions.c: the string forms of a backtrace and of a
//! Throwable, and the engine helpers the prelude's exception classes call.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::{Host, Thrown};

/// One trace argument as `zend_exceptions.c::_build_trace_args` renders it
/// (scalars via `smart_str_append_scalar` with a 15-byte string cap).
fn trace_arg(host: &dyn Host, v: &Value, out: &mut Vec<u8>) {
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
            out.push(b'\'');
            append_escaped(out, &b[..b.len().min(15)]);
            if b.len() > 15 {
                out.extend_from_slice(b"...");
            }
            out.push(b'\'');
        }
        Value::Array(_) => out.extend_from_slice(b"Array"),
        Value::Resource(r) => {
            out.extend_from_slice(b"Resource id #");
            out.extend_from_slice(r.id.to_string().as_bytes());
        }
        Value::Closure(_) => out.extend_from_slice(b"Object(Closure)"),
        o @ Value::Object(_) => {
            let name = host.object_class_name(&o).unwrap_or_default();
            let cid = match &o {
                Value::Object(h) => h.borrow().class,
                _ => unreachable!(),
            };
            if host.is_enum_class(cid) {
                out.extend_from_slice(&name);
                out.extend_from_slice(b"::");
                let case = host.object_prop(&o, b"name").unwrap_or(Value::Null);
                out.extend_from_slice(&conv::to_string_bytes(&case));
            } else {
                out.extend_from_slice(b"Object(");
                out.extend_from_slice(&crate::ext::standard::var::c_str(name));
                out.push(b')');
            }
        }
        Value::Ref(_) => unreachable!(),
    }
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
    use rhp_values::ArrayKey as K;
    let mut out = Vec::new();
    let mut num = 0usize;
    if let Value::Array(frames) = deref(trace) {
        for (_, frame) in frames.iter() {
            let Value::Array(f) = deref(frame) else {
                continue;
            };
            out.push(b'#');
            out.extend_from_slice(num.to_string().as_bytes());
            out.push(b' ');
            num += 1;
            match f.get(&K::from_bytes(b"file")) {
                Some(file) => {
                    out.extend_from_slice(&conv::to_string_bytes(&deref(file)));
                    out.push(b'(');
                    let line = f
                        .get(&K::from_bytes(b"line"))
                        .map(|l| conv::to_int(&deref(l)))
                        .unwrap_or(0);
                    out.extend_from_slice(line.to_string().as_bytes());
                    out.extend_from_slice(b"): ");
                }
                None => out.extend_from_slice(b"[internal function]: "),
            }
            for key in [&b"class"[..], b"type", b"function"] {
                if let Some(v) = f.get(&K::from_bytes(key)) {
                    out.extend_from_slice(&conv::to_string_bytes(&deref(v)));
                }
            }
            out.push(b'(');
            if let Some(Value::Array(args)) = f.get(&K::from_bytes(b"args")).map(deref) {
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
                    trace_arg(host, a, &mut out);
                }
            }
            out.extend_from_slice(b")\n");
        }
    }
    out.push(b'#');
    out.extend_from_slice(num.to_string().as_bytes());
    out.extend_from_slice(b" {main}");
    out
}

/// `__exception_trace_string($trace)`: backs `Exception::getTraceAsString()`.
fn fn_exception_trace_string(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(trace_to_string(host, &args[0])))
}

/// `Exception::__toString()` (zend_exceptions.c): the chain from the innermost
/// `previous` outwards, each as `Class: msg in file:line\nStack trace:\n…`,
/// joined by `\n\nNext `.
pub fn throwable_to_string(host: &dyn Host, exc: &Value) -> Vec<u8> {
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
        let prop = |n: &[u8]| host.object_prop(&cur, n).map(|v| deref(&v));
        let mut message = prop(b"message")
            .map(|v| conv::to_string_bytes(&v))
            .unwrap_or_default();
        let file = prop(b"file")
            .map(|v| conv::to_string_bytes(&v))
            .unwrap_or_default();
        let line = prop(b"line").map(|v| conv::to_int(&v)).unwrap_or(0);
        if (name == b"TypeError" || name == b"ArgumentCountError")
            && message.windows(12).any(|w| w == b", called in ")
        {
            message.extend_from_slice(b" and defined");
        }
        let mut trace = prop(b"trace")
            .map(|t| trace_to_string(host, &t))
            .unwrap_or_else(|| b"#0 {main}".to_vec());
        if trace.is_empty() {
            trace = b"#0 {main}\n".to_vec();
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
        cur = prop(b"previous").unwrap_or(Value::Null);
    }
    s
}

/// `__throwable_string($e)`: backs `Exception::__toString()` / `Error::__toString()`.
fn fn_throwable_string(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(throwable_to_string(host, &args[0])))
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
