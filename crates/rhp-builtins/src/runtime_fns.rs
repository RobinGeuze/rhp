//! Runtime / class-table / SAPI builtins: the class relation family
//! (`class_alias`, `class_implements`, `class_parents`, `class_uses`),
//! `get_defined_constants`, the ini table (`ini_get`/`ini_set`/`ini_get_all`/
//! `ini_restore`), `error_log`, CLI header
//! semantics (`header`, `headers_sent`, `headers_list`, `header_remove`,
//! `http_response_code`), and `sleep`/`usleep`.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::ini_table::INI_TABLE;
use crate::{Host, Thrown};

// ---------------------------------------------------------------------------
// class relations
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// get_defined_constants
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// ini table
// ---------------------------------------------------------------------------

const PHP_INI_USER: u8 = 1;

thread_local! {
    /// Runtime `ini_set` overrides (name → current value; `None` = null).
    pub(crate) static INI: RefCell<HashMap<String, Option<Vec<u8>>>> = RefCell::new(HashMap::new());
    /// Startup (`-d`) values: phpinfo's "Master Value".
    pub(crate) static INI_STARTUP: RefCell<HashMap<String, Vec<u8>>> = RefCell::new(HashMap::new());
}

/// Diagnostics of startup (`-d`) ini values, and of re-applying them when a
/// runtime change is undone (`ini_restore`, `zend_ini_deactivate` at request
/// end).
#[derive(Default)]
pub(crate) struct StartupDiags {
    /// Raised while the `-d` values were applied (module startup): the
    /// directive's INI_TABLE position (php applies them in registration
    /// order), its name, the level (E_WARNING or E_ERROR), the message.
    startup: Vec<(usize, String, i64, String)>,
    /// Directives whose accepted startup value warned: php's handler warns
    /// again each time it restores that value.
    pub(crate) restore: HashMap<String, Vec<String>>,
    /// Directives `ini_set` changed this request, in order
    /// (`EG(modified_ini_directives)`).
    pub(crate) changed: Vec<String>,
}

thread_local! {
    pub(crate) static STARTUP_DIAGS: RefCell<StartupDiags> = RefCell::new(StartupDiags::default());
}

/// Directives whose handler parses the value with
/// `zend_ini_parse_quantity_warn` (OnUpdateLong and handlers built on it),
/// among those rhp applies.
const QUANTITY_DIRECTIVES: &[&str] = &[
    "pcre.backtrack_limit",
    "pcre.recursion_limit",
    "bcmath.scale",
];

/// The warnings a directive's OnModify handler raises for `v`, and the
/// value it keeps (`None`: refused).
fn on_modify_check(name: &[u8], v: &[u8]) -> (Vec<String>, Option<Vec<u8>>) {
    if name == b"memory_hard_limit_percent" {
        let (warnings, ok) = memory_hard_limit_percent_check(v);
        return (warnings, ok.then(|| v.to_vec()));
    }
    if name == b"memory_limit" {
        return memory_limit_modify(v);
    }
    if crate::ext::iconv::ini_refuses(name, v) {
        return (Vec::new(), None);
    }
    let mut warnings = Vec::new();
    if QUANTITY_DIRECTIVES.iter().any(|d| d.as_bytes() == name) {
        if let Some(err) = crate::main::rfc1867::quantity_error(v) {
            let name = String::from_utf8_lossy(name);
            warnings.push(format!("Invalid \"{name}\" setting. {err}"));
        }
    }
    // bcmath's OnUpdateScale: 0..=INT_MAX.
    if name == b"bcmath.scale" && !(0..=i64::from(i32::MAX)).contains(&ini_parse_quantity(v)) {
        return (warnings, None);
    }
    (warnings, Some(v.to_vec()))
}

/// OnChangeMemoryLimit (main.c): a value above max_memory_limit becomes
/// max_memory_limit (without changing the allocator's limit, as in php),
/// and one below the memory in use is refused.
fn memory_limit_modify(v: &[u8]) -> (Vec<String>, Option<Vec<u8>>) {
    let mut warnings = Vec::new();
    let (value, err) = ini_parse_uquantity(v);
    if let Some(err) = err {
        warnings.push(format!("Invalid \"memory_limit\" setting. {err}"));
    }
    let max_raw = ini_value(b"max_memory_limit").flatten().unwrap_or_default();
    let max = ini_parse_uquantity(&max_raw).0;
    if value > max {
        if value != u64::MAX {
            warnings.push(format!(
                "Failed to set memory_limit to {} bytes. Setting to max_memory_limit instead (currently: {} bytes)",
                value as i64, max as i64
            ));
        }
        return (warnings, Some(max_raw));
    }
    if !rhp_values::mem::set_limit(value as usize) {
        warnings.push(format!(
            "Failed to set memory limit to {} bytes (Current memory usage is {} bytes)",
            value as i64,
            rhp_values::mem::usage(true)
        ));
        return (warnings, None);
    }
    (warnings, Some(v.to_vec()))
}

/// `zend_ini_parse_uquantity`: the value and php's warning text. Only `-1`
/// may be negative (it means the maximum); another sign is an out-of-range
/// value that keeps its digits.
pub(crate) fn ini_parse_uquantity(v: &[u8]) -> (u64, Option<String>) {
    let ws = |c: &u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    let start = v.iter().position(|c| !ws(c)).unwrap_or(v.len());
    let end = v.iter().rposition(|c| !ws(c)).map_or(start, |e| e + 1);
    let t = &v[start..end];
    if t == b"-1" {
        return (u64::MAX, None);
    }
    let err = crate::main::rfc1867::quantity_error(v);
    let out_of_range = |e: &Option<String>| e.as_ref().is_some_and(|m| m.contains("out of range"));
    if let Some(rest) = t.strip_prefix(b"-") {
        let value = ini_parse_quantity(rest) as u64;
        let err = match err {
            Some(m) if !m.contains("out of range") => Some(m),
            _ => Some(format!(
                "Invalid quantity \"{}\": value is out of range, using overflow result for backwards compatibility",
                String::from_utf8_lossy(t)
            )),
        };
        return (value, err);
    }
    let value = ini_parse_quantity(v) as u64;
    // Signed overflow that still fits unsigned (2^63 ..< 2^64) is fine here.
    if out_of_range(&err) && unsigned_fits(t) {
        return (value, None);
    }
    (value, err)
}

/// Whether a quantity's digits times its suffix fit in 64 unsigned bits.
fn unsigned_fits(t: &[u8]) -> bool {
    let t = t.strip_prefix(b"+").unwrap_or(t);
    let (body, factor) = match t.last() {
        Some(b'g' | b'G') => (&t[..t.len() - 1], 1u64 << 30),
        Some(b'm' | b'M') => (&t[..t.len() - 1], 1 << 20),
        Some(b'k' | b'K') => (&t[..t.len() - 1], 1 << 10),
        _ => (t, 1),
    };
    let body = body.trim_ascii_end();
    let (digits, radix) = match body {
        [b'0', b'x' | b'X', d @ ..] => (d, 16),
        [b'0', b'o' | b'O', d @ ..] => (d, 8),
        [b'0', b'b' | b'B', d @ ..] => (d, 2),
        [b'0', d @ ..] if !d.is_empty() => (d, 8),
        d => (d, 10),
    };
    let Ok(s) = std::str::from_utf8(digits) else {
        return false;
    };
    u64::from_str_radix(s, radix)
        .ok()
        .and_then(|n| n.checked_mul(factor))
        .is_some()
}

/// Give the allocator the limits the current `memory_limit` and
/// `memory_hard_limit_percent` ask for (a request starts).
pub fn apply_memory_limits() {
    let v = ini_value(b"memory_limit").flatten().unwrap_or_default();
    rhp_values::mem::force_limit(ini_parse_uquantity(&v).0.min(usize::MAX as u64) as usize);
    let p = ini_value(b"memory_hard_limit_percent")
        .flatten()
        .map_or(-1, |p| ini_parse_quantity(&p));
    rhp_values::mem::set_hard_limit_percent(p);
}

/// `ini_set` is changing `key`: zend_alter_ini_entry_ex marks it modified
/// before its handler runs, so it is restored at request end even when the
/// handler refuses the new value.
fn mark_changed(key: &str) {
    STARTUP_DIAGS.with(|d| {
        let mut d = d.borrow_mut();
        if !d.changed.iter().any(|c| c == key) {
            d.changed.push(key.to_string());
        }
    });
}

/// OnChangeMemoryHardLimitPercent's checks: the warnings to raise, and
/// whether the value is accepted.
fn memory_hard_limit_percent_check(v: &[u8]) -> (Vec<String>, bool) {
    let mut warnings = Vec::new();
    if let Some(err) = crate::main::rfc1867::quantity_error(v) {
        warnings.push(format!(
            "Invalid \"memory_hard_limit_percent\" setting. {err}"
        ));
    }
    let q = ini_parse_quantity(v);
    if q < 100 && q != -1 {
        warnings.push("memory_hard_limit_percent must be >= 100, or -1".to_string());
        return (warnings, false);
    }
    if !rhp_values::mem::set_hard_limit_percent(q) {
        warnings.push(format!("Failed to set memory_hard_limit_percent to {q}"));
        return (warnings, false);
    }
    (warnings, true)
}

/// The diagnostics raised while applying the startup (`-d`) ini values,
/// once: (level, message).
pub fn take_startup_diagnostics() -> Vec<(i64, String)> {
    let mut diags = STARTUP_DIAGS.with(|d| std::mem::take(&mut d.borrow_mut().startup));
    diags.sort_by_key(|d| d.0);
    diags.into_iter().map(|d| (d.2, d.3)).collect()
}

/// The warnings raised at request end when startup values are restored
/// over runtime changes, once.
pub fn take_shutdown_diagnostics() -> Vec<String> {
    STARTUP_DIAGS.with(|d| {
        let mut d = d.borrow_mut();
        let changed = std::mem::take(&mut d.changed);
        changed
            .iter()
            .filter_map(|k| d.restore.get(k))
            .flatten()
            .cloned()
            .collect()
    })
}

/// Reset this module's per-request state (a new Vm).
pub(crate) fn reset_state() {
    INI.with(|m| m.borrow_mut().clear());
    INI_STARTUP.with(|m| m.borrow_mut().clear());
    STARTUP_DIAGS.with(|d| *d.borrow_mut() = StartupDiags::default());
    rhp_values::fmt::reset_precision();
    crate::ext::bcmath::set_default_scale(0);
}

/// Is a directive's owning module loaded in this engine?
pub(crate) fn module_loaded(module: &str) -> bool {
    if module == "cgi-fcgi" {
        return crate::main::sapi::is_cgi();
    }
    crate::zend::builtin_functions::loaded_ext(module.as_bytes()).is_some()
}

fn ini_entry(
    name: &[u8],
) -> Option<&'static (&'static str, &'static str, Option<&'static str>, u8)> {
    INI_TABLE
        .iter()
        .find(|e| e.1.as_bytes() == name && module_loaded(e.0))
        // php registers pcre.jit only when the PCRE2 it links has a JIT.
        .filter(|e| e.1 != "pcre.jit" || crate::ext::pcre::jit_supported())
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

/// A directive's startup value (phpinfo's "Master Value"): `None` = unknown.
pub(crate) fn ini_default_value(name: &[u8]) -> Option<Option<Vec<u8>>> {
    let e = ini_entry(name)?;
    let key = String::from_utf8_lossy(name);
    if let Some(v) = INI_STARTUP.with(|m| m.borrow().get(key.as_ref()).cloned()) {
        return Some(Some(v));
    }
    Some(ini_default(e))
}

/// A directive's startup value: the SAPI's ini default, else the built-in one.
fn ini_default(e: &(&'static str, &'static str, Option<&'static str>, u8)) -> Option<Vec<u8>> {
    crate::main::sapi::sapi_ini_default(e.1).or_else(|| e.2.map(|s| s.as_bytes().to_vec()))
}

/// Store a runtime override for a directive (`ini_set` / `set_include_path`).
pub(crate) fn ini_store(name: &[u8], v: Vec<u8>) {
    let key = String::from_utf8_lossy(name).into_owned();
    INI.with(|m| m.borrow_mut().insert(key, Some(v)));
}

/// `-d name=value` at startup: no INI_USER / validator checks. The
/// directive's handler runs: its warnings are reported "in Unknown on line
/// 0", and a refused value leaves the default (NULL for
/// memory_hard_limit_percent). A later `-d` for the same name replaces an
/// earlier one, as in php's configuration hash.
pub(crate) fn ini_startup(name: &[u8], v: &[u8]) {
    let Some(pos) = INI_TABLE
        .iter()
        .position(|e| e.1.as_bytes() == name)
        .filter(|_| ini_entry(name).is_some())
    else {
        return;
    };
    let key = String::from_utf8_lossy(name).into_owned();
    let mut level = crate::E_WARNING;
    let (warnings, stored) = if name == b"date.timezone" && !tz_valid(v) {
        (vec![invalid_timezone(v, "PHP Startup")], None)
    } else if name == b"openssl.libctx" && !matches!(v, b"default" | b"custom") {
        // OnUpdateLibCtx: an E_ERROR at startup, and the entry keeps
        // "custom". (PHP_INI_PERDIR: ini_set can't reach the handler.)
        level = 1;
        let msg = format!(
            "PHP Startup: OpenSSL libctx \"{}\" cannot be found",
            String::from_utf8_lossy(v)
        );
        (vec![msg], None)
    } else if let Some(w) = crate::ext::opcache::startup_check(name, v) {
        crate::ext::opcache::accel_log(name, crate::ext::opcache::LOG_WARNING, w);
        (Vec::new(), None)
    } else {
        on_modify_check(name, v)
    };
    let ok = stored.is_some();
    // A value the handler replaced (memory_limit clamped to
    // max_memory_limit) is what the entry holds: restoring it is quiet.
    let replaced = stored.as_deref().is_some_and(|s| s != v);
    STARTUP_DIAGS.with(|d| {
        let mut d = d.borrow_mut();
        d.startup.retain(|s| s.1 != key);
        if ok && !replaced && !warnings.is_empty() {
            d.restore.insert(key.clone(), warnings.clone());
        } else {
            d.restore.remove(&key);
        }
        d.startup
            .extend(warnings.into_iter().map(|w| (pos, key.clone(), level, w)));
    });
    let Some(v) = stored else {
        INI.with(|m| m.borrow_mut().remove(&key));
        INI_STARTUP.with(|m| m.borrow_mut().remove(&key));
        return;
    };
    ini_apply(name, Some(&v));
    ini_store(name, v.clone());
    INI_STARTUP.with(|m| m.borrow_mut().insert(key, v));
    if name == b"max_memory_limit" {
        // php registers max_memory_limit first: memory_limit's handler then
        // runs against it, whatever the order of the -d options.
        let ml = ini_default_value(b"memory_limit")
            .flatten()
            .unwrap_or_default();
        ini_startup(b"memory_limit", &ml);
    }
}

/// C `strtol(s, NULL, 10)` / `atol` (php's ZEND_ATOL): leading whitespace,
/// sign, digits; saturates on overflow.
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
    // The magnitude, saturated past i64::MIN's.
    let mut n: u64 = 0;
    while let Some(d) = s.get(i).filter(|c| c.is_ascii_digit()) {
        n = n.saturating_mul(10).saturating_add((d - b'0') as u64);
        i += 1;
    }
    if neg {
        0i64.checked_sub_unsigned(n).unwrap_or(i64::MIN)
    } else {
        i64::try_from(n).unwrap_or(i64::MAX)
    }
}

/// `zend_ini_parse_quantity` (signed): digits with an optional 0x/0o/0b
/// prefix, then an optional k/m/g suffix as the last character. Malformed
/// input keeps the valid leading part (php warns and does the same).
pub(crate) fn ini_parse_quantity(s: &[u8]) -> i64 {
    // zend_ini_parse_quantity (signed), overflow wrapping included: php
    // keeps the overflowed result "for backwards compatibility".
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
    let neg = t[0] == b'-';
    if matches!(t[0], b'-' | b'+') {
        i = 1;
    }
    if !t.get(i).is_some_and(u8::is_ascii_digit) {
        return 0;
    }
    // ZEND_STRTOUL's base 0: octal after a leading 0, unless a 0x/0o/0b
    // prefix picks the base.
    let mut radix = if t[i] == b'0' { 8 } else { 10 };
    if t[i] == b'0' && t.get(i + 1).is_some_and(|c| !c.is_ascii_digit()) {
        match t[i + 1] {
            b'g' | b'G' | b'm' | b'M' | b'k' | b'K' => {}
            b'x' | b'X' => radix = 16,
            b'o' | b'O' => radix = 8,
            b'b' | b'B' => radix = 2,
            _ => return 0,
        }
        if radix != 8 || matches!(t[i + 1], b'o' | b'O') {
            i += 2;
            if !t.get(i).is_some_and(|c| (*c as char).is_digit(radix)) {
                return 0;
            }
        }
    }
    let mut v: u64 = 0;
    let mut overflow = false;
    while let Some(d) = t.get(i).and_then(|c| (*c as char).to_digit(radix)) {
        match v
            .checked_mul(u64::from(radix))
            .and_then(|x| x.checked_add(u64::from(d)))
        {
            Some(x) => v = x,
            None => overflow = true,
        }
        i += 1;
    }
    if overflow {
        v = u64::MAX; // strtoul's ERANGE result
    } else if neg && v == 1 << 63 {
        // PHP_INT_MIN: 0 - v has the same bits.
    } else if (v as i64) >= 0 && neg {
        v = v.wrapping_neg();
    }
    while i < t.len() && ws(t[i]) {
        i += 1;
    }
    if i == t.len() {
        return v as i64;
    }
    let factor: u64 = match t[t.len() - 1] {
        b'g' | b'G' => 1 << 30,
        b'm' | b'M' => 1 << 20,
        b'k' | b'K' => 1 << 10,
        _ => return v as i64,
    };
    v.wrapping_mul(factor) as i64
}

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
        // OnUpdateDefaultCharset.
        b"default_charset" if v.iter().any(|&c| matches!(c, 0 | b'\r' | b'\n')) => Err(()),
        b"date.timezone" if !tz_valid(v) => Err(()),
        _ => Ok(()),
    }
}

/// Push a directive's (new) value into the engine state that uses it.
/// (memory_limit's handler, `memory_limit_modify`, sets the allocator's
/// limit itself, or leaves it when it clamps to max_memory_limit.)
pub(crate) fn ini_apply(name: &[u8], v: Option<&[u8]>) {
    match name {
        b"precision" => rhp_values::fmt::set_precision(v.map_or(14, c_atol)),
        b"serialize_precision" => rhp_values::fmt::set_serialize_precision(v.map_or(-1, c_atol)),
        b"pcre.backtrack_limit" | b"pcre.recursion_limit" | b"pcre.jit" => {
            crate::ext::pcre::ini_update(name, v.unwrap_or_default())
        }
        b"bcmath.scale" => crate::ext::bcmath::set_default_scale(v.map_or(0, ini_parse_quantity)),
        _ => {}
    }
}

/// C `atoi`: `(int)strtol(s, NULL, 10)`, so a value past the long range
/// saturates first and is then cut to 32 bits.
pub(crate) fn c_atoi(s: &[u8]) -> i32 {
    c_atol(s) as i32
}

/// `zend_ini_parse_bool`: "true", "yes", "on" (any case), else atoi != 0.
pub(crate) fn ini_parse_bool(v: &[u8]) -> bool {
    [&b"true"[..], b"yes", b"on"]
        .iter()
        .any(|w| v.eq_ignore_ascii_case(w))
        || c_atoi(v) != 0
}

/// zend_alter_ini_entry at runtime (PHP_INI_USER) from builtin `func`: the
/// old value, or `None` when the directive is unknown, not changeable at
/// runtime, or its handler refused the value.
pub(crate) fn alter_ini_entry(
    host: &mut dyn Host,
    name: &[u8],
    newv: Vec<u8>,
    func: &str,
) -> Result<Option<Vec<u8>>, Thrown> {
    let Some(e) = ini_entry(name) else {
        return Ok(None);
    };
    if e.3 & PHP_INI_USER == 0 {
        return Ok(None);
    }
    mark_changed(&String::from_utf8_lossy(name));
    let (warnings, stored) = on_modify_check(name, &newv);
    for w in &warnings {
        host.emit_diagnostic(crate::E_WARNING, w)?;
    }
    let Some(newv) = stored else {
        return Ok(None);
    };
    if !crate::ext::opcache::on_modify(host, name, &newv)? {
        return Ok(None);
    }
    crate::ext::iconv::ini_runtime_update(host, name, func)?;
    if name == b"date.timezone" && !tz_valid(&newv) {
        let func = format!("{func}()");
        host.emit_diagnostic(crate::E_WARNING, &invalid_timezone(&newv, &func))?;
        return Ok(None);
    }
    if ini_validate(name, &newv).is_err() {
        // php: the directive's handler warns (where it does) and refuses.
        return Ok(None);
    }
    let old = ini_value(name).flatten().unwrap_or_default();
    ini_apply(name, Some(&newv));
    ini_store(name, newv);
    Ok(Some(old))
}

// ---------------------------------------------------------------------------
// date.timezone
// ---------------------------------------------------------------------------

/// OnUpdate_date_timezone's check (timelib, case-insensitive).
pub(crate) fn tz_valid(id: &[u8]) -> bool {
    crate::ext::date::timezone_id_is_valid(id)
}

/// OnUpdate_date_timezone's warning (a docref from `func`): it names
/// `DATEG(default_timezone)`, the date.timezone value in effect as stored,
/// which is unset at startup (then "UTC").
fn invalid_timezone(v: &[u8], func: &str) -> String {
    let fallback = if func == "PHP Startup" {
        None
    } else {
        ini_value(b"date.timezone").flatten()
    };
    let fallback = fallback
        .filter(|z| !z.is_empty())
        .unwrap_or_else(|| b"UTC".to_vec());
    // Both are printed with %s.
    let c =
        |s: &[u8]| String::from_utf8_lossy(s.split(|&b| b == 0).next().unwrap_or(s)).into_owned();
    format!(
        "{func}: Invalid date.timezone value '{}', using '{}' instead",
        c(v),
        c(&fallback)
    )
}

// ---------------------------------------------------------------------------
// sleep / usleep
// ---------------------------------------------------------------------------
