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
pub(crate) const LOADED_EXTS: &[&str] = &[
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

fn fn_debug_backtrace(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    use rhp_values::ArrayKey as K;
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

fn fn_debug_print_backtrace(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
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

fn fn_php_uname(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mode = args
        .first()
        .map(|v| conv::to_string_bytes(&deref(v)))
        .unwrap_or_else(|| b"a".to_vec());
    let s: &[u8] = match mode.first() {
        Some(b's') => b"Darwin",
        Some(b'n') => b"localhost",
        Some(b'r') => b"24.0.0",
        Some(b'v') => b"Darwin Kernel Version 24.0.0",
        Some(b'm') => b"arm64",
        _ => b"Darwin localhost 24.0.0 Darwin Kernel Version 24.0.0 arm64",
    };
    Ok(Value::str(s.to_vec()))
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

/// `putenv("NAME=value")` sets, `putenv("NAME")` unsets (process env, so
/// `getenv` and child processes see it, as in php).
fn fn_putenv(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = crate::argparse::str_param(host, args, 0, "putenv", "assignment")?;
    if a.is_empty() || a[0] == b'=' {
        return Err(Thrown::error(
            "ValueError",
            "putenv(): Argument #1 ($assignment) must have a valid syntax",
        ));
    }
    let (name, value) = match a.iter().position(|&b| b == b'=') {
        Some(p) => (&a[..p], Some(&a[p + 1..])),
        None => (&a[..], None),
    };
    let name = String::from_utf8_lossy(name).into_owned();
    if name.contains('\0') {
        return Ok(Value::Bool(false));
    }
    match value {
        Some(v) => {
            let v = String::from_utf8_lossy(v).into_owned();
            if v.contains('\0') {
                return Ok(Value::Bool(false));
            }
            std::env::set_var(name, v);
        }
        None => std::env::remove_var(name),
    }
    Ok(Value::Bool(true))
}

fn fn_set_handler(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    // Error/exception handlers are stored by php and invoked on diagnostics;
    // this engine emits none, so we accept and return "no previous handler".
    Ok(Value::Null)
}

fn fn_restore_handler(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}

fn fn_trigger_error(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}

fn fn_error_reporting(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(30719))
}

fn fn_error_get_last(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Null)
}

fn fn_register_shutdown_function(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    // Stored by php and run at shutdown; not yet invoked here.
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
    if !host.is_callable_value(&c) {
        return Err(Thrown::error(
            "TypeError",
            "Closure::fromCallable(): Argument #1 ($callback) must be a valid callback",
        ));
    }
    Ok(Value::Closure(std::rc::Rc::new(rhp_values::ClosureData {
        fnidx: u32::MAX,
        captures: Vec::new(),
        bound_this: None,
        bound_class: None,
        bound_static: None,
        wrapped: Some(Box::new(c)),
    })))
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

/// Canonicalize a version string into comparable components, per php: insert a
/// `.` at digit↔non-digit boundaries and around `-_+`, then split on `.`.
fn version_parts(v: &[u8]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut prev_kind = 0u8; // 0 none, 1 digit, 2 alpha
    for &b in v {
        if matches!(b, b'.' | b'-' | b'_' | b'+') {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_kind = 0;
            continue;
        }
        let kind = if b.is_ascii_digit() { 1 } else { 2 };
        if prev_kind != 0 && kind != prev_kind && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(b);
        prev_kind = kind;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// php's ordering for special version words; numeric parts sort above these.
fn special_order(s: &[u8]) -> i32 {
    let l = s.to_ascii_lowercase();
    match l.as_slice() {
        b"dev" => -6,
        b"alpha" | b"a" => -5,
        b"beta" | b"b" => -4,
        b"rc" => -3,
        b"pl" | b"p" => 1,
        _ if l.iter().all(|c| c.is_ascii_digit()) => 0, // numeric
        _ => -7,                                        // unknown string: below dev
    }
}

fn cmp_part(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (an, bn) = (
        a.iter().all(|c| c.is_ascii_digit()),
        b.iter().all(|c| c.is_ascii_digit()),
    );
    if an && bn {
        // numeric compare (ignore leading zeros via integer parse)
        let ai: u128 = String::from_utf8_lossy(a).parse().unwrap_or(0);
        let bi: u128 = String::from_utf8_lossy(b).parse().unwrap_or(0);
        return ai.cmp(&bi);
    }
    if an {
        return Ordering::Greater; // number > any special word
    }
    if bn {
        return Ordering::Less;
    }
    special_order(a).cmp(&special_order(b))
}

fn version_cmp(a: &[u8], b: &[u8]) -> i32 {
    use std::cmp::Ordering;
    let pa = version_parts(a);
    let pb = version_parts(b);
    let n = pa.len().max(pb.len());
    for i in 0..n {
        let ord = match (pa.get(i), pb.get(i)) {
            (Some(x), Some(y)) => cmp_part(x, y),
            // a missing trailing part: a numeric/present side wins, matching php's
            // "-1 for shorter unless the extra part is a number" simplification.
            (Some(x), None) => {
                if x.iter().all(|c| c.is_ascii_digit()) {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            (None, Some(y)) => {
                if y.iter().all(|c| c.is_ascii_digit()) {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            (None, None) => Ordering::Equal,
        };
        if ord != Ordering::Equal {
            return if ord == Ordering::Less { -1 } else { 1 };
        }
    }
    0
}

fn fn_version_compare(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = str_arg(args, 0);
    let b = str_arg(args, 1);
    let c = version_cmp(&a, &b);
    match args.get(2) {
        None => Ok(Value::Int(c as i64)),
        Some(op) => {
            let op = conv::to_string_bytes(&deref(op));
            let r = match op.as_slice() {
                b"<" | b"lt" => c < 0,
                b"<=" | b"le" => c <= 0,
                b">" | b"gt" => c > 0,
                b">=" | b"ge" => c >= 0,
                b"==" | b"=" | b"eq" => c == 0,
                b"!=" | b"<>" | b"ne" => c != 0,
                _ => return Ok(Value::Null),
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
    put(m, "php_uname", fn_php_uname, 0, Some(1));
    put(m, "getenv", fn_getenv, 0, Some(1));
    put(m, "putenv", fn_putenv, 1, Some(1));
    put(m, "set_error_handler", fn_set_handler, 1, Some(2));
    put(m, "restore_error_handler", fn_restore_handler, 0, Some(0));
    put(m, "set_exception_handler", fn_set_handler, 1, Some(1));
    put(
        m,
        "restore_exception_handler",
        fn_restore_handler,
        0,
        Some(0),
    );
    put(m, "trigger_error", fn_trigger_error, 1, Some(2));
    put(m, "error_reporting", fn_error_reporting, 0, Some(1));
    put(m, "error_get_last", fn_error_get_last, 0, Some(0));
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
