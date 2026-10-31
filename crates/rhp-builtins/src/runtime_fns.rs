//! Runtime / class-table / SAPI builtins: the class relation family
//! (`class_alias`, `class_implements`, `class_parents`, `class_uses`),
//! `get_defined_constants`, the ini table (`ini_get`/`ini_set`/`ini_get_all`/
//! `ini_restore`), `date_default_timezone_*`, `error_log`, CLI header
//! semantics (`header`, `headers_sent`, `headers_list`, `header_remove`,
//! `http_response_code`), and `sleep`/`usleep`.

use std::cell::RefCell;
use std::collections::HashMap;

use rhp_semantics::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::argparse::{bool_param, int_param, str_or_null_param, str_param, value_name};
use crate::const_modules::CONST_MODULES;
use crate::ini_table::INI_TABLE;
use crate::{Builtin, Host, Thrown};

// ---------------------------------------------------------------------------
// class relations
// ---------------------------------------------------------------------------

fn fn_class_alias(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let class = str_param(host, args, 0, "class_alias", "class")?;
    let alias = str_param(host, args, 1, "class_alias", "alias")?;
    let autoload = bool_param(host, args, 2, "class_alias", "autoload", true)?;
    let id = if autoload {
        host.resolve_class_autoload(&class)
    } else {
        host.resolve_class(&class)
    };
    match id {
        // php: Warning "Cannot declare class X, because the name is already in use"
        Some(id) => Ok(Value::Bool(host.class_alias(id, &alias))),
        // php: Warning "Class "X" not found"
        None => Ok(Value::Bool(false)),
    }
}

/// Shared argument handling for `class_implements`/`class_parents`/`class_uses`.
fn relation(
    host: &mut dyn Host,
    args: &[Value],
    func: &str,
    pick: fn((Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<Vec<u8>>)) -> Vec<Vec<u8>>,
) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    let autoload = bool_param(host, args, 1, func, "autoload", true)?;
    let id = match &v {
        Value::Object(h) => Some(h.borrow().class),
        Value::Str(s) => {
            if autoload {
                host.resolve_class_autoload(s.as_bytes())
            } else {
                host.resolve_class(s.as_bytes())
            }
        }
        other => {
            return Err(Thrown::type_error(format!(
                "{func}(): Argument #1 ($object_or_class) must be of type object|string, {} given",
                value_name(host, other)
            )))
        }
    };
    // php: Warning "Class X does not exist[ and could not be loaded]"
    let Some(id) = id else {
        return Ok(Value::Bool(false));
    };
    let Some(rel) = host.class_relations(id) else {
        return Ok(Value::Array(PhpArray::new()));
    };
    let mut out = PhpArray::new();
    for n in pick(rel) {
        out.insert(ArrayKey::from_bytes(&n), Value::str(n));
    }
    Ok(Value::Array(out))
}

fn fn_class_implements(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    relation(host, args, "class_implements", |r| r.1)
}

fn fn_class_parents(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    relation(host, args, "class_parents", |r| r.0)
}

fn fn_class_uses(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    relation(host, args, "class_uses", |r| r.2)
}

// ---------------------------------------------------------------------------
// get_defined_constants
// ---------------------------------------------------------------------------

fn fn_get_defined_constants(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let categorize = bool_param(host, args, 0, "get_defined_constants", "categorize", false)?;
    let user = host.user_constants();
    let is_user = |n: &[u8]| user.iter().any(|(u, _)| u == n);
    let mut flat = PhpArray::new();
    let mut cats = PhpArray::new();
    for (module, names) in CONST_MODULES {
        let mut cat = PhpArray::new();
        for name in names.split(' ') {
            if is_user(name.as_bytes()) {
                continue;
            }
            if let Some(v) = host.get_const(name.as_bytes()) {
                cat.insert(ArrayKey::from_bytes(name.as_bytes()), v);
            }
        }
        if cat.is_empty() {
            continue;
        }
        if categorize {
            cats.insert(ArrayKey::from_bytes(module.as_bytes()), Value::Array(cat));
        } else {
            for (k, v) in cat.iter() {
                flat.insert(k.clone(), v.clone());
            }
        }
    }
    let mut ucat = PhpArray::new();
    for (k, v) in user {
        ucat.insert(ArrayKey::from_bytes(&k), v);
    }
    if categorize {
        if !ucat.is_empty() {
            cats.insert(ArrayKey::from_bytes(b"user"), Value::Array(ucat));
        }
        Ok(Value::Array(cats))
    } else {
        for (k, v) in ucat.iter() {
            flat.insert(k.clone(), v.clone());
        }
        Ok(Value::Array(flat))
    }
}

// ---------------------------------------------------------------------------
// ini table
// ---------------------------------------------------------------------------

const PHP_INI_USER: u8 = 1;

thread_local! {
    /// Runtime `ini_set` overrides (name → current value; `None` = null).
    static INI: RefCell<HashMap<String, Option<Vec<u8>>>> = RefCell::new(HashMap::new());
}

/// Is a directive's owning module loaded in this engine?
fn module_loaded(module: &str) -> bool {
    module == "core" || crate::php_fns::LOADED_EXTS.contains(&module)
}

fn ini_entry(
    name: &[u8],
) -> Option<&'static (&'static str, &'static str, Option<&'static str>, u8)> {
    INI_TABLE
        .iter()
        .find(|e| e.1.as_bytes() == name && module_loaded(e.0))
}

/// Current value of an ini directive: `None` = unknown directive,
/// `Some(None)` = registered with a null value.
pub(crate) fn ini_value(name: &[u8]) -> Option<Option<Vec<u8>>> {
    let e = ini_entry(name)?;
    let key = String::from_utf8_lossy(name).into_owned();
    if let Some(v) = INI.with(|m| m.borrow().get(&key).cloned()) {
        return Some(v);
    }
    Some(e.2.map(|s| s.as_bytes().to_vec()))
}

fn ini_opt_value(v: Option<Vec<u8>>) -> Value {
    match v {
        Some(s) => Value::str(s),
        None => Value::Null,
    }
}

fn fn_ini_get(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, "ini_get", "option")?;
    Ok(match ini_value(&name) {
        None => Value::Bool(false),
        Some(v) => Value::str(v.unwrap_or_default()),
    })
}

fn ini_set_impl(host: &mut dyn Host, args: &[Value], func: &str) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, func, "option")?;
    let newv = match deref(&args[1]) {
        Value::Null => Vec::new(),
        v @ (Value::Str(_) | Value::Int(_) | Value::Float(_) | Value::Bool(_)) => {
            conv::to_string_bytes(&v)
        }
        other => {
            return Err(Thrown::type_error(format!(
            "{func}(): Argument #2 ($value) must be of type string|int|float|bool|null, {} given",
            value_name(host, &other)
        )))
        }
    };
    let Some(e) = ini_entry(&name) else {
        return Ok(Value::Bool(false));
    };
    if e.3 & PHP_INI_USER == 0 {
        return Ok(Value::Bool(false));
    }
    let old = ini_value(&name).flatten().unwrap_or_default();
    let key = String::from_utf8_lossy(&name).into_owned();
    INI.with(|m| m.borrow_mut().insert(key, Some(newv)));
    Ok(Value::str(old))
}

fn fn_ini_set(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ini_set_impl(host, args, "ini_set")
}

fn fn_ini_alter(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ini_set_impl(host, args, "ini_alter")
}

fn fn_ini_restore(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, "ini_restore", "option")?;
    let key = String::from_utf8_lossy(&name).into_owned();
    INI.with(|m| m.borrow_mut().remove(&key));
    Ok(Value::Null)
}

fn fn_ini_get_all(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ext = str_or_null_param(host, args, 0, "ini_get_all", "extension")?;
    let details = bool_param(host, args, 1, "ini_get_all", "details", true)?;
    let ext_lc = ext.as_ref().map(|e| e.to_ascii_lowercase());
    if let Some(e) = &ext_lc {
        let e = String::from_utf8_lossy(e);
        if !module_loaded(&e) {
            // php: Warning "Extension "X" cannot be found"
            return Ok(Value::Bool(false));
        }
    }
    let mut entries: Vec<_> = INI_TABLE
        .iter()
        .filter(|e| module_loaded(e.0))
        .filter(|e| {
            ext_lc
                .as_ref()
                .is_none_or(|x| x.as_slice() == e.0.as_bytes())
        })
        .collect();
    entries.sort_by(|a, b| a.1.cmp(b.1));
    let mut out = PhpArray::new();
    for e in entries {
        let cur = ini_value(e.1.as_bytes()).flatten();
        let v = if details {
            let mut d = PhpArray::new();
            d.insert(
                ArrayKey::from_bytes(b"global_value"),
                ini_opt_value(e.2.map(|s| s.as_bytes().to_vec())),
            );
            d.insert(ArrayKey::from_bytes(b"local_value"), ini_opt_value(cur));
            d.insert(ArrayKey::from_bytes(b"access"), Value::Int(e.3 as i64));
            Value::Array(d)
        } else {
            ini_opt_value(cur)
        };
        out.insert(ArrayKey::from_bytes(e.1.as_bytes()), v);
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// date_default_timezone_*
// ---------------------------------------------------------------------------

thread_local! {
    static TZ: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

/// Is `id` a known timezone identifier (case-insensitive, as timelib)?
fn tz_valid(id: &[u8]) -> bool {
    if id.eq_ignore_ascii_case(b"UTC") {
        return true;
    }
    let Ok(s) = std::str::from_utf8(id) else {
        return false;
    };
    if s.is_empty()
        || s.starts_with('/')
        || s.split('/').any(|c| c.is_empty() || c == "." || c == "..")
    {
        return false;
    }
    // Walk /usr/share/zoneinfo matching each component case-insensitively.
    let mut dir = std::path::PathBuf::from("/usr/share/zoneinfo");
    let comps: Vec<&str> = s.split('/').collect();
    for (i, comp) in comps.iter().enumerate() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return false;
        };
        let found = rd
            .flatten()
            .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(comp));
        let Some(ent) = found else {
            return false;
        };
        dir = ent.path();
        let is_dir = dir.is_dir();
        if i + 1 == comps.len() {
            return !is_dir && std::fs::read(&dir).is_ok_and(|b| b.starts_with(b"TZif"));
        }
        if !is_dir {
            return false;
        }
    }
    false
}

fn fn_date_default_timezone_get(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let tz = TZ.with(|t| t.borrow().clone());
    Ok(Value::str(tz.unwrap_or_else(|| {
        ini_value(b"date.timezone")
            .flatten()
            .filter(|v| !v.is_empty() && tz_valid(v))
            .unwrap_or_else(|| b"UTC".to_vec())
    })))
}

fn fn_date_default_timezone_set(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let id = str_param(host, args, 0, "date_default_timezone_set", "timezoneId")?;
    if !tz_valid(&id) {
        // php: Notice "Timezone ID 'X' is invalid"
        return Ok(Value::Bool(false));
    }
    TZ.with(|t| *t.borrow_mut() = Some(id));
    Ok(Value::Bool(true))
}

// ---------------------------------------------------------------------------
// error_log
// ---------------------------------------------------------------------------

fn fn_error_log(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    use std::io::Write;
    let msg = str_param(host, args, 0, "error_log", "message")?;
    let ty = int_param(host, args, 1, "error_log", "message_type", 0)?;
    let dest = str_or_null_param(host, args, 2, "error_log", "destination")?;
    let _headers = str_or_null_param(host, args, 3, "error_log", "additional_headers")?;
    match ty {
        // Mail: no mailer configured.
        1 => Ok(Value::Bool(false)),
        3 => {
            let Some(path) = dest else {
                return Ok(Value::Bool(false));
            };
            let p = String::from_utf8_lossy(&path).into_owned();
            let ok = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .and_then(|mut f| f.write_all(&msg))
                .is_ok();
            Ok(Value::Bool(ok))
        }
        // 0 / 4: no error_log ini → the CLI SAPI logger writes "msg\n" to stderr.
        _ => {
            let log_target = ini_value(b"error_log").flatten().filter(|v| !v.is_empty());
            match log_target {
                Some(path) if ty == 0 => {
                    let p = String::from_utf8_lossy(&path).into_owned();
                    let mut line = msg;
                    line.push(b'\n');
                    let _ = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(p)
                        .and_then(|mut f| f.write_all(&line));
                }
                _ => {
                    let mut e = std::io::stderr();
                    let _ = e.write_all(&msg);
                    let _ = e.write_all(b"\n");
                    let _ = e.flush();
                }
            }
            Ok(Value::Bool(true))
        }
    }
}

// ---------------------------------------------------------------------------
// headers (CLI SAPI)
// ---------------------------------------------------------------------------

thread_local! {
    static RESPONSE_CODE: RefCell<i64> = const { RefCell::new(0) };
}

fn fn_header(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = str_param(host, args, 0, "header", "header")?;
    let _replace = bool_param(host, args, 1, "header", "replace", true)?;
    let code = int_param(host, args, 2, "header", "response_code", 0)?;
    if host.output_started() {
        // php: Warning "Cannot modify header information - headers already sent"
        return Ok(Value::Null);
    }
    if code > 0 {
        RESPONSE_CODE.with(|c| *c.borrow_mut() = code);
    }
    if h.len() >= 5 && h[..5].eq_ignore_ascii_case(b"HTTP/") {
        // sapi_extract_response_code: atoi after the first space.
        if let Some(sp) = h.iter().position(|&b| b == b' ') {
            let rest = &h[sp + 1..];
            let n: i64 = rest
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .fold(0, |a, &b| a * 10 + (b - b'0') as i64);
            if n > 0 {
                RESPONSE_CODE.with(|c| *c.borrow_mut() = n);
            }
        }
    } else if code == 0 && h.len() >= 9 && h[..9].eq_ignore_ascii_case(b"location:") {
        RESPONSE_CODE.with(|c| {
            let cur = *c.borrow();
            if !(300..=399).contains(&cur) && cur != 201 {
                *c.borrow_mut() = 302;
            }
        });
    }
    Ok(Value::Null)
}

fn fn_headers_sent(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.output_started()))
}

fn fn_headers_list(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    // The CLI SAPI never stores headers.
    Ok(Value::Array(PhpArray::new()))
}

fn fn_header_remove(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let _ = str_or_null_param(host, args, 0, "header_remove", "name")?;
    Ok(Value::Null)
}

fn fn_http_response_code(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = int_param(host, args, 0, "http_response_code", "response_code", 0)?;
    let old = RESPONSE_CODE.with(|c| *c.borrow());
    if code != 0 {
        if host.output_started() {
            return Ok(Value::Bool(false));
        }
        RESPONSE_CODE.with(|c| *c.borrow_mut() = code);
        return Ok(if old != 0 {
            Value::Int(old)
        } else {
            Value::Bool(true)
        });
    }
    Ok(if old != 0 {
        Value::Int(old)
    } else {
        Value::Bool(false)
    })
}

// ---------------------------------------------------------------------------
// sleep / usleep
// ---------------------------------------------------------------------------

fn fn_sleep(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = int_param(host, args, 0, "sleep", "seconds", 0)?;
    if s < 0 {
        return Err(Thrown::error(
            "ValueError",
            "sleep(): Argument #1 ($seconds) must be greater than or equal to 0",
        ));
    }
    std::thread::sleep(std::time::Duration::from_secs(s as u64));
    Ok(Value::Int(0))
}

fn fn_usleep(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let us = int_param(host, args, 0, "usleep", "microseconds", 0)?;
    if us < 0 {
        return Err(Thrown::error(
            "ValueError",
            "usleep(): Argument #1 ($microseconds) must be greater than or equal to 0",
        ));
    }
    std::thread::sleep(std::time::Duration::from_micros(us as u64));
    Ok(Value::Null)
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    let mut put = |name: &'static str, func: crate::NativeFn, min: usize, max: usize| {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: Some(max),
            },
        );
    };
    put("class_alias", fn_class_alias, 2, 3);
    put("class_implements", fn_class_implements, 1, 2);
    put("class_parents", fn_class_parents, 1, 2);
    put("class_uses", fn_class_uses, 1, 2);
    put("get_defined_constants", fn_get_defined_constants, 0, 1);
    put("ini_get", fn_ini_get, 1, 1);
    put("ini_set", fn_ini_set, 2, 2);
    put("ini_alter", fn_ini_alter, 2, 2);
    put("ini_restore", fn_ini_restore, 1, 1);
    put("ini_get_all", fn_ini_get_all, 0, 2);
    put(
        "date_default_timezone_get",
        fn_date_default_timezone_get,
        0,
        0,
    );
    put(
        "date_default_timezone_set",
        fn_date_default_timezone_set,
        1,
        1,
    );
    put("error_log", fn_error_log, 1, 4);
    put("header", fn_header, 1, 3);
    put("headers_sent", fn_headers_sent, 0, 2);
    put("headers_list", fn_headers_list, 0, 0);
    put("header_remove", fn_header_remove, 0, 1);
    put("http_response_code", fn_http_response_code, 0, 1);
    put("sleep", fn_sleep, 1, 1);
    put("usleep", fn_usleep, 1, 1);
}
