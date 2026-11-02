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
        host.resolve_class_autoload(&class)?
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

/// (parents, interfaces, traits) as returned by `Host::class_relations`.
type Relations = (Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<Vec<u8>>);

/// Shared argument handling for `class_implements`/`class_parents`/`class_uses`.
fn relation(
    host: &mut dyn Host,
    args: &[Value],
    func: &str,
    pick: fn(Relations) -> Vec<Vec<u8>>,
) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    let autoload = bool_param(host, args, 1, func, "autoload", true)?;
    let id = match &v {
        Value::Object(h) => Some(h.borrow().class),
        Value::Str(s) => {
            if autoload {
                host.resolve_class_autoload(s.as_bytes())?
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

/// Reset this module's per-request state (a new Vm).
pub(crate) fn reset_state() {
    INI.with(|m| m.borrow_mut().clear());
    TZ.with(|t| *t.borrow_mut() = None);
    rhp_values::fmt::reset_precision();
}

/// Is a directive's owning module loaded in this engine?
fn module_loaded(module: &str) -> bool {
    if module == "cgi-fcgi" {
        return crate::sapi::is_cgi();
    }
    crate::php_fns::loaded_ext(module.as_bytes()).is_some()
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
    Some(ini_default(e))
}

/// A directive's startup value: the SAPI's ini default, else the built-in one.
fn ini_default(e: &(&'static str, &'static str, Option<&'static str>, u8)) -> Option<Vec<u8>> {
    crate::sapi::sapi_ini_default(e.1).or_else(|| e.2.map(|s| s.as_bytes().to_vec()))
}

/// Store a runtime override for a directive (`ini_set` / `set_include_path`).
pub(crate) fn ini_store(name: &[u8], v: Vec<u8>) {
    let key = String::from_utf8_lossy(name).into_owned();
    INI.with(|m| m.borrow_mut().insert(key, Some(v)));
}

/// `-d name=value` at startup: no INI_USER / validator checks.
pub(crate) fn ini_startup(name: &[u8], v: &[u8]) {
    if ini_entry(name).is_some() {
        ini_apply(name, Some(v));
        ini_store(name, v.to_vec());
    }
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

/// C `strtol(s, NULL, 10)` / `atoi`: leading whitespace, sign, digits.
pub(crate) fn c_atol(s: &[u8]) -> i64 {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
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
        n = n.saturating_mul(10).saturating_add((d - b'0') as i64);
        i += 1;
    }
    if neg {
        n.saturating_neg()
    } else {
        n
    }
}

/// `zend_ini_parse_quantity` (signed): digits with an optional 0x/0o/0b
/// prefix, then an optional k/m/g suffix as the last character. Malformed
/// input keeps the valid leading part (php warns and does the same).
pub(crate) fn ini_parse_quantity(s: &[u8]) -> i64 {
    let ws = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    let mut a = 0;
    let mut b = s.len();
    while a < b && ws(s[a]) {
        a += 1;
    }
    while b > a && ws(s[b - 1]) {
        b -= 1;
    }
    let t = &s[a..b];
    if t.is_empty() {
        return 0;
    }
    let mut i = 0;
    let neg = match t[0] {
        b'-' => {
            i = 1;
            true
        }
        b'+' => {
            i = 1;
            false
        }
        _ => false,
    };
    let mut base = 10;
    if t.get(i) == Some(&b'0') {
        match t.get(i + 1).map(|c| c.to_ascii_lowercase()) {
            Some(b'x') => base = 16,
            Some(b'o') => base = 8,
            Some(b'b') => base = 2,
            _ => {}
        }
        if base != 10 {
            i += 2;
        }
    }
    let start = i;
    let mut n: i64 = 0;
    while let Some(d) = t.get(i).and_then(|c| (*c as char).to_digit(base)) {
        n = n.saturating_mul(base as i64).saturating_add(d as i64);
        i += 1;
    }
    if i == start {
        return 0;
    }
    let shift = match t.last().map(|c| c.to_ascii_lowercase()) {
        Some(b'k') if i < t.len() => 10,
        Some(b'm') if i < t.len() => 20,
        Some(b'g') if i < t.len() => 30,
        _ => 0,
    };
    let v = if shift > 0 {
        n.checked_shl(shift)
            .filter(|r| r >> shift == n)
            .unwrap_or(i64::MAX)
    } else {
        n
    };
    if neg {
        v.saturating_neg()
    } else {
        v
    }
}

/// Process memory the engine reports (`memory_get_usage(true)`); a
/// `memory_limit` below it is refused, as php's allocator does.
const REAL_MEMORY_USAGE: i64 = 2_097_152;

/// The `OnModify` validators of the directives whose values rhp checks or
/// uses. `Err(())` means php's handler returns FAILURE (`ini_set` → false).
fn ini_validate(name: &[u8], v: &[u8]) -> Result<(), ()> {
    match name {
        b"precision" | b"serialize_precision" => {
            if c_atol(v) >= -1 {
                Ok(())
            } else {
                Err(())
            }
        }
        b"memory_limit" => {
            let q = ini_parse_quantity(v);
            // zend_ini_parse_uquantity: only -1 may be negative (unlimited).
            let q = if q < 0 && q != -1 { 0 } else { q };
            if q != -1 && q < REAL_MEMORY_USAGE {
                Err(())
            } else {
                Ok(())
            }
        }
        b"zend.assertions" => {
            let cur = ini_value(name)
                .flatten()
                .map(|c| ini_parse_quantity(&c))
                .unwrap_or(1);
            let val = ini_parse_quantity(v);
            if cur != val && (cur < 0 || val < 0) {
                Err(())
            } else {
                Ok(())
            }
        }
        b"arg_separator.output" if v.is_empty() => Err(()),
        b"date.timezone" if !tz_valid(v) => Err(()),
        _ => Ok(()),
    }
}

/// Push a directive's (new) value into the engine state that uses it.
fn ini_apply(name: &[u8], v: Option<&[u8]>) {
    match name {
        b"precision" => rhp_values::fmt::set_precision(v.map_or(14, c_atol)),
        b"serialize_precision" => rhp_values::fmt::set_serialize_precision(v.map_or(-1, c_atol)),
        _ => {}
    }
}

fn ini_set_impl(host: &mut dyn Host, args: &[Value], func: &str) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, func, "option")?;
    let newv = match deref(&args[1]) {
        Value::Null => Vec::new(),
        v @ (Value::Str(_) | Value::Int(_) | Value::Float(_) | Value::Bool(_)) => {
            conv::to_string_bytes(&v)
        }
        _ => {
            return Err(Thrown::type_error(format!(
                "{func}(): Argument #2 ($value) must be of type string|int|float|bool|null"
            )))
        }
    };
    let Some(e) = ini_entry(&name) else {
        return Ok(Value::Bool(false));
    };
    if e.3 & PHP_INI_USER == 0 {
        return Ok(Value::Bool(false));
    }
    if ini_validate(&name, &newv).is_err() {
        // php: the directive's handler warns (where it does) and refuses.
        return Ok(Value::Bool(false));
    }
    let old = ini_value(&name).flatten().unwrap_or_default();
    ini_apply(&name, Some(&newv));
    ini_store(&name, newv);
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
    if let Some(v) = ini_value(&name) {
        ini_apply(&name, v.as_deref());
    }
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
                ini_opt_value(ini_default(e)),
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
pub(crate) fn tz_valid(id: &[u8]) -> bool {
    id.eq_ignore_ascii_case(b"UTC") || crate::tzdb::load(id).is_some()
}

/// The effective default timezone name: `date_default_timezone_set()`, then a
/// valid `date.timezone`, then `UTC` (php_date.c `guess_timezone`).
pub(crate) fn default_timezone_name() -> Vec<u8> {
    TZ.with(|t| t.borrow().clone()).unwrap_or_else(|| {
        ini_value(b"date.timezone")
            .flatten()
            .filter(|v| !v.is_empty() && tz_valid(v))
            .unwrap_or_else(|| b"UTC".to_vec())
    })
}

fn fn_date_default_timezone_get(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(default_timezone_name()))
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
                    // php_log_err: "[d-M-Y H:i:s e] message\n".
                    let p = String::from_utf8_lossy(&path).into_owned();
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    let mut line = b"[".to_vec();
                    line.extend_from_slice(&crate::datetime_fns::php_date(
                        b"d-M-Y H:i:s e",
                        now,
                        true,
                    ));
                    line.extend_from_slice(b"] ");
                    line.extend_from_slice(&msg);
                    line.push(b'\n');
                    let _ = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(p)
                        .and_then(|mut f| f.write_all(&line));
                }
                _ => host.log_message(&msg),
            }
            Ok(Value::Bool(true))
        }
    }
}

// ---------------------------------------------------------------------------
// sleep / usleep
// ---------------------------------------------------------------------------

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
}
