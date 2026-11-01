//! PHP runtime / environment introspection (php's `ext/standard` "info" group):
//! `extension_loaded`, `function_exists`, `phpversion`, `php_sapi_name`,
//! `version_compare`, the error/exception-handler setters (stored, not invoked —
//! this engine emits no diagnostics), and a handful of no-op-ish runtime knobs.
//! Enough to boot a real framework / Composer.

use std::collections::HashMap;

use rhp_values::{conv, PhpArray, Value};

use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn str_arg(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&deref(&args[i]))
}

/// Extensions this engine reports as loaded (lowercased). These are the ones
/// whose functions/classes rhp actually provides.
const LOADED_EXTS: &[&str] = &[
    "core",
    "standard",
    "spl",
    "pcre",
    "json",
    "hash",
    "filter",
    "date",
    "ctype",
    "mbstring",
    "tokenizer",
    "phar",
    "zlib",
    "bz2",
    "bcmath",
    "gmp",
    "reflection",
    "iconv",
    "openssl",
    "zip",
    // XML family: reported so composer resolution passes for packages that
    // require ext-xml/ext-dom (symfony etc.). Runtime XML support is partial.
    "libxml",
    "xml",
    "dom",
    "simplexml",
    "xmlreader",
    "xmlwriter",
];

/// The loaded-extension list (lowercased), for phpinfo(INFO_MODULES).
pub(crate) fn loaded_extension_names() -> &'static [&'static str] {
    LOADED_EXTS
}

fn fn_extension_loaded(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = String::from_utf8_lossy(&str_arg(args, 0)).to_ascii_lowercase();
    Ok(Value::Bool(LOADED_EXTS.contains(&name.as_str())))
}

fn fn_get_loaded_extensions(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    for e in LOADED_EXTS {
        a.push(Value::str(*e));
    }
    Ok(Value::Array(a))
}

fn fn_function_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.function_exists(&str_arg(args, 0))))
}

fn fn_phpversion(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // phpversion($extension) returns the ext's version or false; plain → engine.
    match args.first() {
        Some(v) if !matches!(deref(v), Value::Null) => {
            let name =
                String::from_utf8_lossy(&conv::to_string_bytes(&deref(v))).to_ascii_lowercase();
            if LOADED_EXTS.contains(&name.as_str()) {
                Ok(Value::str("8.5.8"))
            } else {
                Ok(Value::Bool(false))
            }
        }
        _ => Ok(Value::str("8.5.8")),
    }
}

fn fn_php_sapi_name(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str("cli"))
}

/// rhp has no php.ini, so this reports "no scanned files".
fn fn_php_ini_scanned_files(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(false))
}

fn fn_func_num_args(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(host.current_arg_count()))
}

fn fn_func_get_args(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    for v in host.current_args() {
        a.push(v);
    }
    Ok(Value::Array(a))
}

fn fn_func_get_arg(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let i = conv::to_int(&deref(&args[0]));
    Ok(host
        .current_args()
        .into_iter()
        .nth(i.max(0) as usize)
        .unwrap_or(Value::Bool(false)))
}

/// `debug_backtrace(int $options = DEBUG_BACKTRACE_PROVIDE_OBJECT, int $limit = 0)`.
fn fn_debug_backtrace(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    use rhp_values::ArrayKey as K;
    let options = args.first().map(|v| conv::to_int(&deref(v))).unwrap_or(1);
    let limit = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if let Some(Value::Array(t)) = host.backtrace(options & 1 != 0, options & 2 != 0) {
        if limit <= 0 {
            return Ok(Value::Array(t));
        }
        let mut out = PhpArray::new();
        for (_, v) in t.iter().take(limit as usize) {
            out.push(v.clone());
        }
        return Ok(Value::Array(out));
    }
    let mut arr = PhpArray::new();
    for (func, class, file, line, this) in host.call_stack() {
        let mut e = PhpArray::new();
        if !file.is_empty() {
            e.insert(K::from_bytes(b"file"), Value::str(file));
            e.insert(K::from_bytes(b"line"), Value::Int(line as i64));
        }
        e.insert(K::from_bytes(b"function"), Value::str(func));
        if let Some(c) = class {
            e.insert(K::from_bytes(b"class"), Value::str(c));
            e.insert(K::from_bytes(b"type"), Value::str("->"));
            if let Some(obj) = this {
                e.insert(K::from_bytes(b"object"), obj);
            }
        }
        arr.push(Value::Array(e));
    }
    Ok(Value::Array(arr))
}

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
                out.extend_from_slice(&crate::dump::c_str(name));
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

/// `__zpp_arg($fn, $n, $name, $type, $value)`: coerce one argument of a
/// prelude-implemented internal method the way php's zpp does in coercive
/// mode (`string`, `int`, `?string`, `?int`, `?Throwable`), or raise its
/// TypeError ("Fn(): Argument #n ($name) must be of type T, X given").
fn fn_zpp_arg(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let func = str_arg(args, 0);
    let n = conv::to_int(&deref(&args[1]));
    let name = str_arg(args, 2);
    let ty = str_arg(args, 3);
    let v = deref(&args[4]);
    let (nullable, base) = match ty.strip_prefix(b"?") {
        Some(b) => (true, b.to_vec()),
        None => (false, ty.clone()),
    };
    let coerced: Option<Value> = match (&v, base.as_slice()) {
        (Value::Null, _) if nullable => Some(Value::Null),
        // Coercive mode: null to a scalar internal param is the scalar's zero.
        (Value::Null, b"string") => Some(Value::str("")),
        (Value::Null, b"int") => Some(Value::Int(0)),
        (Value::Str(_), b"string") => Some(v.clone()),
        (Value::Int(_) | Value::Float(_) | Value::Bool(_), b"string") => {
            Some(Value::str(conv::to_string_bytes(&v)))
        }
        (Value::Object(_), b"string") => {
            let cls = host.object_class_name(&v).unwrap_or_default();
            if host.reflect_is_a(&cls, b"Stringable") || {
                let id = match &v {
                    Value::Object(h) => h.borrow().class,
                    _ => unreachable!(),
                };
                host.class_has_method(id, b"__tostring")
            } {
                let mut cb = PhpArray::new();
                cb.push(v.clone());
                cb.push(Value::str("__toString"));
                Some(host.call_callable(&Value::Array(cb), &[])?)
            } else {
                None
            }
        }
        (Value::Int(_), b"int") => Some(v.clone()),
        (Value::Bool(b), b"int") => Some(Value::Int(*b as i64)),
        (Value::Float(f), b"int") if f.is_finite() => Some(Value::Int(*f as i64)),
        (Value::Str(s), b"int") => conv::numeric_string(s.as_bytes()).map(|num| {
            Value::Int(match num {
                conv::Num::Int(i) => i,
                conv::Num::Float(f) => f as i64,
            })
        }),
        (Value::Object(_), b"Throwable") => {
            let cls = host.object_class_name(&v).unwrap_or_default();
            host.reflect_is_a(&cls, b"Throwable").then(|| v.clone())
        }
        _ => None,
    };
    match coerced {
        Some(c) => Ok(c),
        None => {
            let given = match &v {
                Value::Null => "null".to_string(),
                Value::Bool(_) => "bool".to_string(),
                Value::Int(_) => "int".to_string(),
                Value::Float(_) => "float".to_string(),
                Value::Str(_) => "string".to_string(),
                Value::Array(_) => "array".to_string(),
                Value::Closure(_) => "Closure".to_string(),
                Value::Resource(_) => "resource".to_string(),
                o @ Value::Object(_) => String::from_utf8_lossy(&crate::dump::c_str(
                    host.object_class_name(o).unwrap_or_default(),
                ))
                .into_owned(),
                Value::Ref(_) => unreachable!(),
            };
            Err(Thrown::type_error(format!(
                "{}(): Argument #{} (${}) must be of type {}, {} given",
                String::from_utf8_lossy(&func),
                n,
                String::from_utf8_lossy(&name),
                String::from_utf8_lossy(&ty),
                given
            )))
        }
    }
}

/// `__throwable_string($e)`: backs `Exception::__toString()` / `Error::__toString()`.
fn fn_throwable_string(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(throwable_to_string(host, &args[0])))
}

/// `gethostbyaddr($ip)`: reverse DNS via getnameinfo(NI_NAMEREQD); the address
/// itself when it has no name, false for a malformed address
/// (ext/standard/dns.c php_gethostbyaddr).
fn fn_gethostbyaddr(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    use std::net::IpAddr;
    let ip = str_arg(args, 0);
    let Some(addr) = std::str::from_utf8(&ip)
        .ok()
        .and_then(|s| s.parse::<IpAddr>().ok())
    else {
        return Ok(Value::Bool(false));
    };
    let mut host = [0 as libc::c_char; 1025];
    // SAFETY: the sockaddr structs are fully initialised for their family and
    // `host` is a writable buffer of the advertised length.
    let rc = unsafe {
        match addr {
            IpAddr::V4(v4) => {
                let mut sa: libc::sockaddr_in = std::mem::zeroed();
                sa.sin_family = libc::AF_INET as libc::sa_family_t;
                sa.sin_addr.s_addr = u32::from_ne_bytes(v4.octets());
                #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
                {
                    sa.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
                }
                libc::getnameinfo(
                    &sa as *const _ as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
                    host.as_mut_ptr(),
                    host.len() as libc::socklen_t,
                    std::ptr::null_mut(),
                    0,
                    libc::NI_NAMEREQD,
                )
            }
            IpAddr::V6(v6) => {
                let mut sa: libc::sockaddr_in6 = std::mem::zeroed();
                sa.sin6_family = libc::AF_INET6 as libc::sa_family_t;
                sa.sin6_addr.s6_addr = v6.octets();
                #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
                {
                    sa.sin6_len = std::mem::size_of::<libc::sockaddr_in6>() as u8;
                }
                libc::getnameinfo(
                    &sa as *const _ as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
                    host.as_mut_ptr(),
                    host.len() as libc::socklen_t,
                    std::ptr::null_mut(),
                    0,
                    libc::NI_NAMEREQD,
                )
            }
        }
    };
    if rc != 0 {
        return Ok(Value::str(ip));
    }
    // SAFETY: getnameinfo NUL-terminated `host` on success.
    let name = unsafe { std::ffi::CStr::from_ptr(host.as_ptr()) };
    Ok(Value::str(name.to_bytes().to_vec()))
}

/// `debug_print_backtrace(int $options = 0, int $limit = 0)`: the trace in
/// getTraceAsString() format, without the `{main}` line.
fn fn_debug_print_backtrace(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let options = args.first().map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    let limit = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if let Some(Value::Array(t)) = host.backtrace(false, options & 2 != 0) {
        let mut frames = PhpArray::new();
        for (_, v) in t.iter().take(if limit > 0 {
            limit as usize
        } else {
            usize::MAX
        }) {
            frames.push(v.clone());
        }
        let mut s = trace_to_string(host, &Value::Array(frames));
        // Strip the trailing "#N {main}".
        if let Some(pos) = s.iter().rposition(|&b| b == b'#') {
            s.truncate(pos);
        }
        host.write_out(&s);
        return Ok(Value::Null);
    }
    let mut out = Vec::new();
    for (i, (func, class, file, line, _this)) in host.call_stack().into_iter().enumerate() {
        let call = match &class {
            Some(c) => format!(
                "{}->{}",
                String::from_utf8_lossy(c),
                String::from_utf8_lossy(&func)
            ),
            None => String::from_utf8_lossy(&func).into_owned(),
        };
        out.extend_from_slice(
            format!(
                "#{} {}() called at [{}:{}]\n",
                i,
                call,
                String::from_utf8_lossy(&file),
                line
            )
            .as_bytes(),
        );
    }
    host.write_out(&out);
    Ok(Value::Null)
}

fn fn_ob_start(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    host.ob_start();
    Ok(Value::Bool(true))
}

fn fn_ob_get_contents(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(host
        .ob_get_contents()
        .map(Value::str)
        .unwrap_or(Value::Bool(false)))
}

fn fn_ob_get_clean(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(host
        .ob_get_clean()
        .map(Value::str)
        .unwrap_or(Value::Bool(false)))
}

fn fn_ob_get_level(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(host.ob_get_level() as i64))
}

fn fn_ob_end_clean(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.ob_end_clean()))
}

fn fn_ob_end_flush(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.ob_end_flush()))
}

fn fn_ob_get_flush(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let contents = host.ob_get_contents();
    host.ob_end_flush();
    Ok(contents.map(Value::str).unwrap_or(Value::Bool(false)))
}

fn fn_getenv(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    match args.first() {
        Some(v) if !matches!(deref(v), Value::Null) => {
            let name = String::from_utf8_lossy(&conv::to_string_bytes(&deref(v))).into_owned();
            match std::env::var(&name) {
                Ok(val) => Ok(Value::str(val.into_bytes())),
                Err(_) => Ok(Value::Bool(false)),
            }
        }
        _ => {
            let mut a = PhpArray::new();
            for (k, v) in std::env::vars() {
                a.insert(
                    rhp_values::ArrayKey::from_bytes(k.as_bytes()),
                    Value::str(v.into_bytes()),
                );
            }
            Ok(Value::Array(a))
        }
    }
}

fn fn_putenv(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    // No-op (we don't mutate the process environment); report success.
    Ok(Value::Bool(true))
}

/// Directives php knows, with their `php -n` CLI defaults (`ini_get` /
/// `ini_set` return false for anything else).
pub fn ini_default(name: &[u8]) -> Option<&'static str> {
    Some(match name {
        b"memory_limit" => "-1",
        b"max_execution_time" => "0",
        b"precision" => "14",
        b"serialize_precision" => "-1",
        b"date.timezone" => "UTC",
        b"default_charset" => "UTF-8",
        b"allow_url_fopen" => "1",
        b"disable_functions" => "",
        b"open_basedir" => "",
        // Error handling (main.c PHP_INI; the CLI SAPI forces display_errors=1).
        b"display_errors" => "1",
        b"display_startup_errors" => "1",
        b"log_errors" => "0",
        b"error_log" => "",
        b"html_errors" => "0",
        b"error_prepend_string" => "",
        b"error_append_string" => "",
        // No php.ini value: EG(error_reporting) keeps its E_ALL default.
        b"error_reporting" => "",
        b"zend.exception_ignore_args" => "0",
        _ => return None,
    })
}

fn fn_ini_get(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(match host.ini_get(&str_arg(args, 0)) {
        Some(v) => Value::str(v),
        None => Value::Bool(false),
    })
}

/// `ini_set(string $option, string|int|float|bool|null $value): string|false`.
fn fn_ini_set(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_arg(args, 0);
    let value = match deref(&args[1]) {
        v @ (Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Str(_)) => {
            conv::to_string_bytes(&v)
        }
        _ => {
            return Err(Thrown::type_error(
                "ini_set(): Argument #2 ($value) must be of type string|int|float|bool|null",
            ))
        }
    };
    Ok(match host.ini_set(&name, value) {
        Some(old) => Value::str(old),
        None => Value::Bool(false),
    })
}

/// zend_zval_type_name (`int`, `bool`, `string`, `null`, …).
pub fn zend_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        Value::Resource(_) => "resource",
        Value::Closure(_) => "Closure",
        Value::Object(_) => "object",
        Value::Ref(r) => zend_type_name(&r.borrow()),
    }
}

/// php's E_ALL (8.4+: E_STRICT is gone).
pub const E_ALL: i64 = 30719;

/// EG(error_reporting): the `error_reporting` directive, E_ALL while unset.
pub fn error_reporting_level(host: &dyn Host) -> i64 {
    match host.ini_get(b"error_reporting") {
        Some(v) if !v.is_empty() => atol(&v),
        _ => E_ALL,
    }
}

/// C `atol`/`ZEND_ATOL`: optional whitespace and sign, then leading digits.
pub fn atol(s: &[u8]) -> i64 {
    let mut i = 0;
    while i < s.len() && (s[i] == b' ' || (b'\t'..=b'\r').contains(&s[i])) {
        i += 1;
    }
    let neg = match s.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut n: i64 = 0;
    while let Some(d) = s.get(i).filter(|c| c.is_ascii_digit()) {
        n = n.wrapping_mul(10).wrapping_add((d - b'0') as i64);
        i += 1;
    }
    if neg {
        n.wrapping_neg()
    } else {
        n
    }
}

fn fn_set_handler(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    // Error/exception handlers are stored by php and invoked on diagnostics;
    // this engine emits none, so we accept and return "no previous handler".
    Ok(Value::Null)
}

/// `set_exception_handler(?callable $callback)`: returns the previous handler.
fn fn_set_exception_handler(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = deref(&args[0]);
    let why = match &h {
        Value::Null => None,
        Value::Str(s) if !host.is_callable_value(&h) => Some(format!(
            "function \"{}\" not found or invalid function name",
            String::from_utf8_lossy(s.as_bytes())
        )),
        Value::Str(_) | Value::Array(_) | Value::Closure(_) | Value::Object(_) => None,
        _ => Some("no array or string given".to_string()),
    };
    if let Some(why) = why {
        return Err(Thrown::type_error(format!(
            "set_exception_handler(): Argument #1 ($callback) must be a valid callback or null, {why}"
        )));
    }
    Ok(host.set_exception_handler(h))
}

fn fn_restore_exception_handler(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    host.restore_exception_handler();
    Ok(Value::Bool(true))
}

fn fn_restore_handler(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}

fn fn_trigger_error(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}

/// `error_reporting(?int $error_level = null): int`: the old level; sets a
/// new one when given.
fn fn_error_reporting(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let old = error_reporting_level(host);
    if let Some(v) = args.first().map(deref) {
        if !matches!(v, Value::Null) {
            let n = conv::to_int(&v);
            host.ini_set(b"error_reporting", n.to_string().into_bytes());
        }
    }
    Ok(Value::Int(old))
}

fn fn_error_get_last(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(match host.last_error() {
        None => Value::Null,
        Some((ty, msg, file, line)) => {
            let mut a = rhp_values::PhpArray::new();
            a.insert(
                rhp_values::ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"type".to_vec())),
                Value::Int(ty),
            );
            a.insert(
                rhp_values::ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"message".to_vec())),
                Value::Str(rhp_values::PhpStr::from_bytes(msg)),
            );
            a.insert(
                rhp_values::ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"file".to_vec())),
                Value::Str(rhp_values::PhpStr::from_bytes(file)),
            );
            a.insert(
                rhp_values::ArrayKey::Str(rhp_values::PhpStr::from_bytes(b"line".to_vec())),
                Value::Int(line),
            );
            Value::Array(a)
        }
    })
}

fn fn_error_clear_last(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    host.clear_last_error();
    Ok(Value::Null)
}

/// `register_shutdown_function(callable $callback, mixed ...$args): void`.
fn fn_register_shutdown_function(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let cb = deref(&args[0]);
    if let Some(why) = host.check_callable(&cb)? {
        return Err(Thrown::type_error(format!(
            "register_shutdown_function(): Argument #1 ($callback) must be a valid callback, {why}"
        )));
    }
    host.register_shutdown_function(cb, args[1..].iter().map(deref).collect())?;
    Ok(Value::Null)
}

fn fn_memory_usage(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(2_097_152))
}

fn fn_true(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}

fn fn_zero(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(0))
}

fn fn_getmypid(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(std::process::id() as i64))
}

fn fn_php_ini_loaded_file(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(false))
}

/// `assert($assertion, $description = null)` — PHP 8: evaluate `$assertion`; if
/// falsy, throw AssertionError (assertions are treated as enabled, the CLI
/// default). A string `$description` becomes the message; a Throwable is thrown.
fn fn_assert(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if conv::to_bool(&deref(&args[0])) {
        return Ok(Value::Bool(true));
    }
    match args.get(1).map(deref) {
        Some(Value::Str(s)) => Err(Thrown::error(
            "AssertionError",
            String::from_utf8_lossy(s.as_bytes()).into_owned(),
        )),
        _ => Err(Thrown::error("AssertionError", "assert failed")),
    }
}

fn fn_setlocale(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    // We don't model locales; report the C locale.
    Ok(Value::str("C"))
}

fn is_utf8ish(cs: &str) -> bool {
    matches!(cs, "UTF-8" | "UTF8" | "")
}

/// `iconv($from, $to, $string)` — pragmatic subset: UTF-8↔UTF-8 (and same
/// charset) passes through; `ASCII`/`US-ASCII` targets drop non-ASCII (honouring
/// `//IGNORE`/`//TRANSLIT` best-effort); other charsets pass through unchanged.
fn fn_iconv(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let from = String::from_utf8_lossy(&str_arg(args, 0)).to_uppercase();
    let to_raw = String::from_utf8_lossy(&str_arg(args, 1)).to_uppercase();
    let data = str_arg(args, 2);
    let from_c = from.split("//").next().unwrap_or("");
    let to_c = to_raw.split("//").next().unwrap_or("");
    if from_c == to_c || (is_utf8ish(from_c) && is_utf8ish(to_c)) {
        return Ok(Value::str(data));
    }
    if matches!(to_c, "ASCII" | "US-ASCII") {
        let out: Vec<u8> = data.into_iter().filter(|&b| b < 0x80).collect();
        return Ok(Value::str(out));
    }
    Ok(Value::str(data))
}

/// Count UTF-8 code points (continuation bytes `10xxxxxx` don't start a char).
fn utf8_len(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| (b & 0xC0) != 0x80).count()
}

fn fn_iconv_strlen(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(utf8_len(&str_arg(args, 0)) as i64))
}

fn fn_iconv_substr(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = str_arg(args, 0);
    let text = String::from_utf8_lossy(&s);
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len() as i64;
    let mut start = conv::to_int(&deref(&args[1]));
    if start < 0 {
        start = (len + start).max(0);
    }
    let start = start.min(len) as usize;
    let count = match args.get(2).map(|v| conv::to_int(&deref(v))) {
        None => chars.len() - start,
        Some(n) if n < 0 => ((len + n) as usize).saturating_sub(start),
        Some(n) => (n as usize).min(chars.len() - start),
    };
    Ok(Value::str(
        chars[start..start + count]
            .iter()
            .collect::<String>()
            .into_bytes(),
    ))
}

/// `Closure::fromCallable($callable)`: wrap a callable in a Closure (a closure is
/// returned as-is), mirroring the first-class-callable `MakeFcc` wrapping.
fn fn_closure_from_callable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let c = deref(&args[0]);
    if matches!(c, Value::Closure(_)) {
        return Ok(c);
    }
    // zend_create_closure_from_callable's error (a TypeError).
    if let Some(err) = host.check_callable(&c)? {
        return Err(Thrown::type_error(format!(
            "Failed to create closure from callable: {err}"
        )));
    }
    Ok(host.wrap_callable(c))
}

/// `Closure::bind($closure, $newThis, $newScope)` and the `bindTo` instance form.
fn fn_closure_bind(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let closure = deref(&args[0]);
    let new_this = args.get(1).map(deref).unwrap_or(Value::Null);
    let scope = args.get(2).map(deref).unwrap_or(Value::Null);
    Ok(host
        .closure_bind(&closure, &new_this, &scope)
        .unwrap_or(Value::Null))
}

// --- randomness --------------------------------------------------------------

fn rand_u64() -> u64 {
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    u64::from_le_bytes(b)
}

/// A uniform integer in `[min, max]` (inclusive) via rejection sampling.
fn rand_range(min: i64, max: i64) -> i64 {
    if min >= max {
        return min;
    }
    let span = (max as i128 - min as i128 + 1) as u128;
    // Rejection-sample to avoid modulo bias.
    let limit = u128::MAX - (u128::MAX % span);
    loop {
        let r = ((rand_u64() as u128) << 64) | rand_u64() as u128;
        if r < limit {
            return (min as i128 + (r % span) as i128) as i64;
        }
    }
}

fn fn_random_bytes(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let len = conv::to_int(&deref(&args[0]));
    if len <= 0 {
        return Err(Thrown::error(
            "ValueError",
            "random_bytes(): Argument #1 ($length) must be greater than 0",
        ));
    }
    let mut b = vec![0u8; len as usize];
    if getrandom::getrandom(&mut b).is_err() {
        return Err(Thrown::error(
            "Random\\RandomException",
            "Could not gather sufficient random data",
        ));
    }
    Ok(Value::str(b))
}

fn fn_random_int(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let min = conv::to_int(&deref(&args[0]));
    let max = conv::to_int(&deref(&args[1]));
    if min > max {
        return Err(Thrown::error(
            "ValueError",
            "random_int(): Argument #1 ($min) must be less than or equal to argument #2 ($max)",
        ));
    }
    Ok(Value::Int(rand_range(min, max)))
}

fn fn_mt_rand(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if args.len() >= 2 {
        let min = conv::to_int(&deref(&args[0]));
        let max = conv::to_int(&deref(&args[1]));
        Ok(Value::Int(rand_range(min, max)))
    } else {
        Ok(Value::Int(rand_range(0, 2147483647)))
    }
}

fn fn_getrandmax(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(2147483647))
}

fn fn_noop_null(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Null)
}

fn fn_uniqid(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let prefix = args
        .first()
        .map(|v| conv::to_string_bytes(&deref(v)))
        .unwrap_or_default();
    let more = args
        .get(1)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let mut s = prefix;
    // php: 8 hex of seconds + 5 hex of microseconds.
    s.extend_from_slice(
        format!(
            "{:08x}{:05x}",
            now.as_secs() & 0xffff_ffff,
            now.subsec_micros() & 0xf_ffff
        )
        .as_bytes(),
    );
    if more {
        s.extend_from_slice(format!(".{:08}", rand_range(0, 99999999)).as_bytes());
    }
    Ok(Value::str(s))
}

/// `time(): int` — current unix timestamp (seconds).
fn fn_time(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Ok(Value::Int(now.as_secs() as i64))
}

/// `microtime(bool $as_float = false)` — current time with microseconds.
fn fn_microtime(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let as_float = args
        .first()
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    if as_float {
        Ok(Value::Float(now.as_secs_f64()))
    } else {
        // php format: "0.xxxxxxxx sssssssss" (microsecond fraction, then seconds).
        Ok(Value::str(
            format!(
                "{:.8} {}",
                now.subsec_micros() as f64 / 1_000_000.0,
                now.as_secs()
            )
            .into_bytes(),
        ))
    }
}

/// `hrtime(bool $as_number = false)` — high-resolution time. As a monotonic-ish
/// value derived from the wall clock (no separate monotonic source needed here).
fn fn_hrtime(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let as_number = args
        .first()
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(false);
    if as_number {
        Ok(Value::Int(now.as_nanos() as i64))
    } else {
        let mut a = PhpArray::new();
        a.push(Value::Int(now.as_secs() as i64));
        a.push(Value::Int(now.subsec_nanos() as i64));
        Ok(Value::Array(a))
    }
}

// --- version_compare ---------------------------------------------------------

/// `php_canonicalize_version` (ext/standard/versioning.c).
fn canonicalize_version(v: &[u8]) -> Vec<u8> {
    if v.is_empty() {
        return Vec::new();
    }
    let isdig = |c: u8| c.is_ascii_digit();
    let isndig = |c: u8| !c.is_ascii_digit() && c != b'.';
    let mut q: Vec<u8> = Vec::with_capacity(v.len() * 2);
    let mut lp = v[0];
    q.push(lp);
    for &c in &v[1..] {
        let lq = *q.last().unwrap();
        if matches!(c, b'-' | b'_' | b'+') {
            if lq != b'.' {
                q.push(b'.');
            }
        } else if (isndig(lp) && isdig(c)) || (isdig(lp) && isndig(c)) {
            if lq != b'.' {
                q.push(b'.');
            }
            q.push(c);
        } else if !c.is_ascii_alphanumeric() {
            if lq != b'.' {
                q.push(b'.');
            }
        } else {
            q.push(c);
        }
        lp = c;
    }
    if q.last() == Some(&b'.') {
        q.pop();
    }
    q
}

fn special_version_form(s: &[u8]) -> i32 {
    const FORMS: [(&[u8], i32); 10] = [
        (b"dev", 0),
        (b"alpha", 1),
        (b"a", 1),
        (b"beta", 2),
        (b"b", 2),
        (b"RC", 3),
        (b"rc", 3),
        (b"#", 4),
        (b"pl", 5),
        (b"p", 5),
    ];
    FORMS
        .iter()
        .find(|(n, _)| s.starts_with(n))
        .map_or(-1, |(_, o)| *o)
}

fn compare_special_version_forms(a: &[u8], b: &[u8]) -> i32 {
    (special_version_form(a) - special_version_form(b)).signum()
}

fn strtol_prefix(s: &[u8]) -> i128 {
    let mut n: i128 = 0;
    for &c in s {
        if !c.is_ascii_digit() {
            break;
        }
        n = (n * 10 + (c - b'0') as i128).min(i64::MAX as i128);
    }
    n
}

/// `php_version_compare` (ext/standard/versioning.c), ported step for step.
fn version_cmp(a: &[u8], b: &[u8]) -> i32 {
    let cut = |s: &[u8]| -> Vec<u8> {
        match s.iter().position(|&c| c == 0) {
            Some(i) => s[..i].to_vec(),
            None => s.to_vec(),
        }
    };
    let (a, b) = (cut(a), cut(b));
    if a.is_empty() || b.is_empty() {
        return if a.is_empty() && b.is_empty() {
            0
        } else if !a.is_empty() {
            1
        } else {
            -1
        };
    }
    let v1 = if a[0] == b'#' {
        a.clone()
    } else {
        canonicalize_version(&a)
    };
    let v2 = if b[0] == b'#' {
        b.clone()
    } else {
        canonicalize_version(&b)
    };
    let at = |v: &[u8], i: usize| v.get(i).copied().unwrap_or(0);
    let find_dot = |v: &[u8], p: usize| {
        v[p.min(v.len())..]
            .iter()
            .position(|&c| c == b'.')
            .map(|i| i + p)
    };
    let (mut p1, mut p2) = (0usize, 0usize);
    let (mut n1, mut n2): (Option<usize>, Option<usize>) = (Some(0), Some(0));
    let mut compare = 0;
    while at(&v1, p1) != 0 && at(&v2, p2) != 0 && n1.is_some() && n2.is_some() {
        n1 = find_dot(&v1, p1);
        n2 = find_dot(&v2, p2);
        let s1 = &v1[p1..n1.unwrap_or(v1.len())];
        let s2 = &v2[p2..n2.unwrap_or(v2.len())];
        let d1 = at(&v1, p1).is_ascii_digit();
        let d2 = at(&v2, p2).is_ascii_digit();
        compare = if d1 && d2 {
            (strtol_prefix(s1) - strtol_prefix(s2)).signum() as i32
        } else if !d1 && !d2 {
            compare_special_version_forms(s1, s2)
        } else if d1 {
            compare_special_version_forms(b"#N#", s2)
        } else {
            compare_special_version_forms(s1, b"#N#")
        };
        if compare != 0 {
            break;
        }
        if let Some(n) = n1 {
            p1 = n + 1;
        }
        if let Some(n) = n2 {
            p2 = n + 1;
        }
    }
    if compare == 0 {
        if n1.is_some() {
            compare = if at(&v1, p1).is_ascii_digit() {
                1
            } else {
                version_cmp(&v1[p1.min(v1.len())..], b"#N#")
            };
        } else if n2.is_some() {
            compare = if at(&v2, p2).is_ascii_digit() {
                -1
            } else {
                version_cmp(b"#N#", &v2[p2.min(v2.len())..])
            };
        }
    }
    compare
}

fn fn_version_compare(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = str_arg(args, 0);
    let b = str_arg(args, 1);
    let c = version_cmp(&a, &b);
    match args.get(2).map(deref) {
        None | Some(Value::Null) => Ok(Value::Int(c as i64)),
        Some(op) => {
            let op = conv::to_string_bytes(&op);
            let r = match op.as_slice() {
                b"<" | b"lt" => c == -1,
                b"<=" | b"le" => c != 1,
                b">" | b"gt" => c == 1,
                b">=" | b"ge" => c != -1,
                b"==" | b"=" | b"eq" => c == 0,
                b"!=" | b"<>" | b"ne" => c != 0,
                _ => {
                    return Err(Thrown::error(
                        "ValueError",
                        "version_compare(): Argument #3 ($operator) must be a valid comparison operator",
                    ))
                }
            };
            Ok(Value::Bool(r))
        }
    }
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
    put(m, "extension_loaded", fn_extension_loaded, 1, Some(1));
    put(
        m,
        "get_loaded_extensions",
        fn_get_loaded_extensions,
        0,
        Some(1),
    );
    put(m, "function_exists", fn_function_exists, 1, Some(1));
    put(m, "phpversion", fn_phpversion, 0, Some(1));
    put(m, "php_sapi_name", fn_php_sapi_name, 0, Some(0));
    put(
        m,
        "php_ini_scanned_files",
        fn_php_ini_scanned_files,
        0,
        Some(0),
    );
    put(m, "func_num_args", fn_func_num_args, 0, Some(0));
    put(m, "func_get_args", fn_func_get_args, 0, Some(0));
    put(m, "func_get_arg", fn_func_get_arg, 1, Some(1));
    put(m, "debug_backtrace", fn_debug_backtrace, 0, Some(2));
    put(
        m,
        "__exception_trace_string",
        fn_exception_trace_string,
        1,
        Some(1),
    );
    put(m, "__throwable_string", fn_throwable_string, 1, Some(1));
    put(m, "__zpp_arg", fn_zpp_arg, 5, Some(5));
    put(m, "gethostbyaddr", fn_gethostbyaddr, 1, Some(1));
    put(
        m,
        "debug_print_backtrace",
        fn_debug_print_backtrace,
        0,
        Some(2),
    );
    put(m, "ob_start", fn_ob_start, 0, Some(3));
    put(m, "ob_get_contents", fn_ob_get_contents, 0, Some(0));
    put(m, "ob_get_clean", fn_ob_get_clean, 0, Some(0));
    put(m, "ob_get_level", fn_ob_get_level, 0, Some(0));
    put(m, "ob_end_clean", fn_ob_end_clean, 0, Some(0));
    put(m, "ob_end_flush", fn_ob_end_flush, 0, Some(0));
    put(m, "ob_get_flush", fn_ob_get_flush, 0, Some(0));
    put(m, "getenv", fn_getenv, 0, Some(1));
    put(m, "putenv", fn_putenv, 1, Some(1));
    put(m, "ini_get", fn_ini_get, 1, Some(1));
    put(m, "ini_set", fn_ini_set, 2, Some(2));
    put(m, "ini_alter", fn_ini_set, 2, Some(2));
    put(m, "set_error_handler", fn_set_handler, 1, Some(2));
    put(m, "restore_error_handler", fn_restore_handler, 0, Some(0));
    put(
        m,
        "set_exception_handler",
        fn_set_exception_handler,
        1,
        Some(1),
    );
    put(
        m,
        "restore_exception_handler",
        fn_restore_exception_handler,
        0,
        Some(0),
    );
    put(m, "trigger_error", fn_trigger_error, 1, Some(2));
    put(m, "error_reporting", fn_error_reporting, 0, Some(1));
    put(m, "error_get_last", fn_error_get_last, 0, Some(0));
    put(m, "error_clear_last", fn_error_clear_last, 0, Some(0));
    put(
        m,
        "register_shutdown_function",
        fn_register_shutdown_function,
        1,
        None,
    );
    put(m, "memory_get_usage", fn_memory_usage, 0, Some(1));
    put(m, "memory_get_peak_usage", fn_memory_usage, 0, Some(1));
    put(m, "gc_enable", fn_true, 0, Some(0));
    put(m, "gc_disable", fn_true, 0, Some(0));
    put(m, "gc_collect_cycles", fn_zero, 0, Some(0));
    put(m, "set_time_limit", fn_true, 1, Some(1));
    put(m, "getmypid", fn_getmypid, 0, Some(0));
    put(m, "php_ini_loaded_file", fn_php_ini_loaded_file, 0, Some(0));
    put(m, "setlocale", fn_setlocale, 2, None);
    put(m, "assert", fn_assert, 1, Some(2));
    put(m, "iconv", fn_iconv, 3, Some(3));
    put(m, "iconv_strlen", fn_iconv_strlen, 1, Some(2));
    put(m, "iconv_substr", fn_iconv_substr, 2, Some(3));
    put(m, "random_bytes", fn_random_bytes, 1, Some(1));
    put(m, "random_int", fn_random_int, 2, Some(2));
    put(m, "mt_rand", fn_mt_rand, 0, Some(2));
    put(m, "rand", fn_mt_rand, 0, Some(2));
    put(m, "mt_getrandmax", fn_getrandmax, 0, Some(0));
    put(m, "getrandmax", fn_getrandmax, 0, Some(0));
    put(m, "mt_srand", fn_noop_null, 0, Some(2));
    put(m, "srand", fn_noop_null, 0, Some(2));
    put(m, "uniqid", fn_uniqid, 0, Some(2));
    put(m, "time", fn_time, 0, Some(0));
    put(m, "microtime", fn_microtime, 0, Some(1));
    put(m, "hrtime", fn_hrtime, 0, Some(1));
    put(
        m,
        "__closure_from_callable",
        fn_closure_from_callable,
        1,
        Some(1),
    );
    put(m, "__closure_bind", fn_closure_bind, 2, Some(3));
    put(m, "version_compare", fn_version_compare, 2, Some(3));
}
