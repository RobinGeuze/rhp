//! The ini machinery (Zend/zend_ini.c, main/php_ini.c and the handlers in
//! main/main.c): each directive's request value over the reference build's
//! table ([`INI_TABLE`]), the startup (`-d`) values, `zend_alter_ini_entry`
//! with each directive's on-modify check, and the quantity and bool
//! parsers. The `ini_*` functions are in ext/standard/basic_functions.rs.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::main::ini_table::INI_TABLE;
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

thread_local! {
    /// php's `configuration_hash`: every php.ini, scanned .ini and `-d`
    /// entry, known directive or not (`get_cfg_var`).
    static CONFIG: RefCell<rhp_values::PhpArray> = RefCell::new(rhp_values::PhpArray::new());
    /// `php_ini_opened_path` and `php_ini_scanned_files`.
    static OPENED_PATH: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    static SCANNED_FILES: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

/// A configuration hash entry: `name=value`, or `name[off]=value` /
/// `name[]=value` (an array entry, as php_ini_parser_cb's POP_ENTRY).
pub fn config_define(name: &[u8], value: &[u8]) {
    use rhp_values::{ArrayKey, PhpArray, Value};
    CONFIG.with(|c| {
        let mut c = c.borrow_mut();
        let open = name.iter().position(|&b| b == b'[');
        match open.filter(|_| name.ends_with(b"]")) {
            Some(i) => {
                let key = ArrayKey::Str(rhp_values::PhpStr::from_bytes(name[..i].to_vec()));
                let off = &name[i + 1..name.len() - 1];
                if !matches!(c.get(&key), Some(Value::Array(_))) {
                    c.insert(key.clone(), Value::Array(PhpArray::new()));
                }
                if let Some(Value::Array(sub)) = c.get_mut(&key) {
                    if off.is_empty() {
                        sub.push(Value::str(value.to_vec()));
                    } else {
                        sub.insert(ArrayKey::from_bytes(off), Value::str(value.to_vec()));
                    }
                }
            }
            None => {
                let key = ArrayKey::Str(rhp_values::PhpStr::from_bytes(name.to_vec()));
                c.insert(key, Value::str(value.to_vec()));
            }
        }
    });
}

/// `cfg_get_entry`: a configuration hash entry.
pub(crate) fn cfg_get(name: &[u8]) -> Option<rhp_values::Value> {
    let key = rhp_values::ArrayKey::Str(rhp_values::PhpStr::from_bytes(name.to_vec()));
    CONFIG.with(|c| c.borrow().get(&key).cloned())
}

/// zend_parse_ini_file into the configuration hash (php_init_config): its
/// entries also set the directives they name. False on a syntax error.
pub fn config_load_file(host: &dyn Host, src: &[u8]) -> bool {
    use rhp_values::Value;
    let mode = crate::zend::ini_scanner::MODE_NORMAL;
    let Some(entries) = crate::ext::standard::basic_functions::parse_ini(host, src, false, mode)
    else {
        return false;
    };
    for (k, v) in entries.iter() {
        let name = k.to_value();
        let name = rhp_values::conv::to_string_bytes(&name);
        match v {
            Value::Array(_) => {
                let key = rhp_values::ArrayKey::Str(rhp_values::PhpStr::from_bytes(name));
                CONFIG.with(|c| c.borrow_mut().insert(key, v.clone()));
            }
            v => {
                let value = rhp_values::conv::to_string_bytes(v);
                config_define(&name, &value);
                ini_startup(&name, &value);
            }
        }
    }
    true
}

/// The php.ini php_init_config opened (`cfg_file_path` too).
pub fn config_set_opened_path(path: &[u8]) {
    config_define(b"cfg_file_path", path);
    OPENED_PATH.with(|p| *p.borrow_mut() = Some(path.to_vec()));
}

/// The scanned .ini files, as php_ini_scanned_files() lists them.
pub fn config_set_scanned_files(files: &[Vec<u8>]) {
    if files.is_empty() {
        return;
    }
    let mut list = Vec::new();
    for (i, f) in files.iter().enumerate() {
        list.extend_from_slice(f);
        list.extend_from_slice(if i + 1 < files.len() { b",\n" } else { b"\n" });
    }
    SCANNED_FILES.with(|s| *s.borrow_mut() = Some(list));
}

/// `php_ini_opened_path` / `php_ini_scanned_files`.
pub(crate) fn config_files() -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    (
        OPENED_PATH.with(|p| p.borrow().clone()),
        SCANNED_FILES.with(|s| s.borrow().clone()),
    )
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
    "zend.assertions",
    "pcre.backtrack_limit",
    "pcre.recursion_limit",
    "bcmath.scale",
    "fiber.stack_size",
    "zlib.output_compression_level",
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
    if crate::ext::iconv::ini_refuses(name, v) || (v.is_empty() && STRING_UNEMPTY.contains(&name)) {
        return (Vec::new(), None);
    }
    let mut warnings = Vec::new();
    // OnUpdate_zlib_output_compression parses all but "on"/"off"
    let zlib_quantity = name == b"zlib.output_compression"
        && !v.eq_ignore_ascii_case(b"on")
        && !v.eq_ignore_ascii_case(b"off");
    if zlib_quantity || QUANTITY_DIRECTIVES.iter().any(|d| d.as_bytes() == name) {
        if let Some(err) = quantity_error(v) {
            let name = String::from_utf8_lossy(name);
            warnings.push(format!("Invalid \"{name}\" setting. {err}"));
        }
    }
    // OnUpdateFiberStackSize.
    if name == b"fiber.stack_size" && ini_parse_quantity(v) < 0 {
        warnings.push("fiber.stack_size must be a positive number".to_string());
        return (warnings, None);
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
    let err = quantity_error(v);
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
    if let Some(err) = quantity_error(v) {
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
    // mbstring's MINIT, once its directives are in.
    #[cfg(feature = "mbstring-libmbfl")]
    if module_loaded("mbstring") {
        let pos = INI_TABLE
            .iter()
            .position(|e| e.1 == "mbstring.detect_order")
            .unwrap_or(0);
        for (level, msg) in crate::ext::mbstring::startup() {
            diags.push((pos, String::new(), level, format!("PHP Startup: {msg}")));
        }
    }
    // php_ini_register_extensions: an `extension=` library that fails to load.
    if let Some(rhp_values::Value::Str(name)) = cfg_get(b"extension") {
        if let Some(msg) = crate::ext::standard::dl::startup_extension_error(name.as_bytes()) {
            diags.push((
                usize::MAX - 1,
                String::new(),
                rhp_values::E_CORE_WARNING,
                msg,
            ));
        }
    }
    // php_module_startup's check of removed and deprecated directives.
    for (level, phrase, names) in REMOVED_DIRECTIVES {
        for name in *names {
            let set = cfg_get(name.as_bytes())
                .is_some_and(|v| c_atol(&rhp_values::conv::to_string_bytes(&v)) != 0);
            if set {
                let msg = phrase.replace("%s", name);
                diags.push((usize::MAX, String::new(), *level, msg));
            }
        }
    }
    diags.sort_by_key(|d| d.0);
    diags.into_iter().map(|d| (d.2, d.3)).collect()
}

/// php_module_startup's directives that are deprecated or gone.
const REMOVED_DIRECTIVES: &[(i64, &str, &[&str])] = &[
    (
        crate::E_DEPRECATED,
        "Directive '%s' is deprecated",
        &["allow_url_include"],
    ),
    (
        crate::E_CORE_ERROR,
        "Directive '%s' is no longer available in PHP",
        &[
            "allow_call_time_pass_reference",
            "asp_tags",
            "define_syslog_variables",
            "highlight.bg",
            "magic_quotes_gpc",
            "magic_quotes_runtime",
            "magic_quotes_sybase",
            "register_globals",
            "register_long_arrays",
            "safe_mode",
            "safe_mode_gid",
            "safe_mode_include_dir",
            "safe_mode_exec_dir",
            "safe_mode_allowed_env_vars",
            "safe_mode_protected_env_vars",
            "zend.ze1_compatibility_mode",
            "track_errors",
            "disable_classes",
        ],
    ),
];

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
    crate::registry::reset_disabled();
    CONFIG.with(|c| *c.borrow_mut() = rhp_values::PhpArray::new());
    OPENED_PATH.with(|p| *p.borrow_mut() = None);
    SCANNED_FILES.with(|s| *s.borrow_mut() = None);
    rhp_values::fmt::reset_precision();
    crate::zend::exceptions::reset_ini();
    crate::zend::set_assertions(1);
    crate::ext::standard::assert::reset_state();
}

/// Is a directive's owning module loaded in this engine?
pub(crate) fn module_loaded(module: &str) -> bool {
    crate::zend::builtin_functions::loaded_ext(module.as_bytes()).is_some()
}

fn ini_entry(
    name: &[u8],
) -> Option<&'static (&'static str, &'static str, Option<&'static str>, u8)> {
    // INI_TABLE's rows by name (a name can belong to several modules), so
    // a lookup doesn't scan the whole table.
    static BY_NAME: std::sync::OnceLock<HashMap<&'static [u8], Vec<usize>>> =
        std::sync::OnceLock::new();
    let by_name = BY_NAME.get_or_init(|| {
        let mut m: HashMap<&'static [u8], Vec<usize>> = HashMap::new();
        for (i, e) in INI_TABLE.iter().enumerate() {
            m.entry(e.1.as_bytes()).or_default().push(i);
        }
        m
    });
    by_name
        .get(name)?
        .iter()
        .map(|&i| &INI_TABLE[i])
        .find(|e| module_loaded(e.0))
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

/// main.c's directives whose handler is OnUpdateStringUnempty.
const STRING_UNEMPTY: &[&[u8]] = &[
    b"doc_root",
    b"extension_dir",
    b"include_path",
    b"sys_temp_dir",
    b"upload_tmp_dir",
    b"variables_order",
];

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
    // assert.c registers its directives in its own order (the table is
    // sorted by name).
    const ASSERT_ORDER: [&[u8]; 5] = [
        b"assert.active",
        b"assert.bail",
        b"assert.warning",
        b"assert.callback",
        b"assert.exception",
    ];
    let pos = match ASSERT_ORDER.iter().position(|n| *n == name) {
        Some(rank) => INI_TABLE
            .iter()
            .position(|e| e.1 == "assert.active")
            .map_or(pos, |base| base + rank),
        None => pos,
    };
    let key = String::from_utf8_lossy(name).into_owned();
    let mut level = crate::E_WARNING;
    let (warnings, stored) = if v.is_empty() && STRING_UNEMPTY.contains(&name) {
        // OnUpdateStringUnempty refuses an empty value: the default stays.
        (Vec::new(), None)
    } else if name == b"date.timezone" && !tz_valid(v) {
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
    } else if name == b"report_memleaks" && !ini_parse_bool(v) {
        // OnUpdateReportMemleaks.
        level = crate::E_DEPRECATED;
        let msg = "PHP Startup: Directive 'report_memleaks' is deprecated".to_string();
        (vec![msg], Some(v.to_vec()))
    } else if let Some(msg) = crate::ext::standard::assert::ini_deprecation(name, v) {
        level = crate::E_DEPRECATED;
        (vec![format!("PHP Startup: {msg}")], Some(v.to_vec()))
    } else if name == b"auto_detect_line_endings" && ini_parse_bool(v) {
        // OnUpdateAutoDetectLineEndings (zend_error: no "PHP Startup").
        level = crate::E_DEPRECATED;
        let msg = "auto_detect_line_endings is deprecated".to_string();
        (vec![msg], Some(v.to_vec()))
    } else if name == b"disable_functions" {
        // php_module_startup's zend_disable_functions, once the ini is in.
        crate::registry::reset_disabled();
        (crate::registry::disable_functions(v), Some(v.to_vec()))
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
        // The assert.* handlers don't deprecate at shutdown; ini_restore
        // raises theirs itself.
        if ok && !replaced && !warnings.is_empty() && !name.starts_with(b"assert.") {
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
    // the realpath cache's startup settings (php_request_startup turns it
    // off under open_basedir)
    use crate::zend::virtual_cwd::realpath_cache::configure;
    match name {
        b"realpath_cache_size" => configure(Some(ini_parse_quantity(&v)), None, None),
        b"realpath_cache_ttl" => configure(None, Some(ini_parse_quantity(&v)), None),
        b"open_basedir" => {
            crate::main::fopen_wrappers::set(Some(&v));
            configure(None, None, Some(!v.is_empty()))
        }
        _ => {}
    }
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

/// php_log_err's file mode: error_log_mode when it is a valid mode, else
/// 0644.
pub fn error_log_mode() -> u32 {
    let m = ini_value(b"error_log_mode")
        .flatten()
        .map_or(0, |v| ini_parse_quantity(&v));
    if (1..=0o777).contains(&m) {
        m as u32
    } else {
        0o644
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
        // OnSetExceptionStringParamMaxLen.
        b"zend.exception_string_param_max_len" if !(0..=1_000_000).contains(&c_atol(v)) => Err(()),
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
        b"zend.assertions" => crate::zend::set_assertions(v.map_or(1, ini_parse_quantity)),
        b"assert.active" | b"assert.bail" | b"assert.warning" | b"assert.exception"
        | b"assert.callback" => crate::ext::standard::assert::ini_update(name, v),
        b"zend.enable_gc" => {
            rhp_values::gc::enable(v.is_none_or(ini_parse_bool));
        }
        b"zend.exception_ignore_args" => {
            crate::zend::exceptions::set_ignore_args(v.is_some_and(ini_parse_bool))
        }
        b"zend.exception_string_param_max_len" => {
            crate::zend::exceptions::set_string_param_max_len(v.map_or(15, c_atol) as usize)
        }
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
    alter_ini_entry_ex(host, name, newv, func, false)
}

/// [`alter_ini_entry`]; `assert_options` is assert_options()'s own stage
/// (ZEND_INI_STAGE_ASSERT_OPTIONS), where the assert.* handlers don't
/// deprecate.
pub(crate) fn alter_ini_entry_ex(
    host: &mut dyn Host,
    name: &[u8],
    newv: Vec<u8>,
    func: &str,
    assert_options: bool,
) -> Result<Option<Vec<u8>>, Thrown> {
    let Some(e) = ini_entry(name) else {
        return Ok(None);
    };
    if e.3 & PHP_INI_USER == 0 {
        return Ok(None);
    }
    mark_changed(&String::from_utf8_lossy(name));
    if name == b"report_memleaks" && !ini_parse_bool(&newv) {
        crate::deprecated(host, func, "Directive 'report_memleaks' is deprecated")?;
    }
    if name == b"auto_detect_line_endings" && ini_parse_bool(&newv) {
        host.emit_diagnostic(
            crate::E_DEPRECATED,
            b"auto_detect_line_endings is deprecated",
        )?;
    }
    // OnUpdateBaseDir: only a tightening (the entry keeps the value as
    // given, the check uses each entry expanded).
    if name == b"open_basedir" && !crate::main::fopen_wrappers::on_update(&newv) {
        return Ok(None);
    }
    // OnUpdateErrorLog / OnUpdateMailLog: a log file must be within
    // open_basedir.
    if crate::main::fopen_wrappers::active()
        && ((name == b"error_log" && newv != b"syslog") || name == b"mail.log")
        && !newv.is_empty()
        && !crate::main::fopen_wrappers::check(host, func, &newv, true)?
    {
        return Ok(None);
    }
    let (warnings, stored) = on_modify_check(name, &newv);
    for w in &warnings {
        host.emit_diagnostic(crate::E_WARNING, w.as_bytes())?;
    }
    let Some(newv) = stored else {
        return Ok(None);
    };
    if !crate::ext::opcache::on_modify(host, name, &newv)? {
        return Ok(None);
    }
    crate::ext::iconv::ini_runtime_update(host, name, func)?;
    if !crate::ext::zlib::output::ini_update(host, name, &newv, func)? {
        return Ok(None);
    }
    #[cfg(feature = "mbstring-libmbfl")]
    let mb_changed = matches!(
        name,
        b"internal_encoding"
            | b"input_encoding"
            | b"output_encoding"
            | b"default_charset"
            | b"mbstring.internal_encoding"
    );
    if name == b"date.timezone" && !tz_valid(&newv) {
        let func = format!("{func}()");
        host.emit_diagnostic(crate::E_WARNING, invalid_timezone(&newv, &func).as_bytes())?;
        return Ok(None);
    }
    if ini_validate(name, &newv).is_err() {
        // php: the directive's handler warns (where it does) and refuses.
        if name == b"zend.assertions" {
            host.emit_diagnostic(
                crate::E_WARNING,
                b"zend.assertions may be completely enabled or disabled only in php.ini",
            )?;
        }
        return Ok(None);
    }
    if !assert_options {
        if let Some(msg) = crate::ext::standard::assert::ini_deprecation(name, &newv) {
            crate::diag::deprecated(host, func, &msg)?;
        }
    }
    let old = ini_value(name).flatten().unwrap_or_default();
    ini_apply(name, Some(&newv));
    if name == b"max_execution_time" {
        crate::zend::timeout::on_update(&newv);
    }
    ini_store(name, newv);
    #[cfg(feature = "mbstring-libmbfl")]
    if mb_changed {
        for (level, msg) in crate::ext::mbstring::ini_changed(name) {
            let msg = format!("{func}(): {msg}");
            host.emit_diagnostic(level, msg.as_bytes())?;
        }
    }
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

/// `smart_str_append_escaped`.
fn escaped(s: &[u8]) -> String {
    let mut o = String::new();
    for &c in s {
        match c {
            b'\n' => o.push_str("\\n"),
            b'\r' => o.push_str("\\r"),
            b'\t' => o.push_str("\\t"),
            0x0c => o.push_str("\\f"),
            0x0b => o.push_str("\\v"),
            b'\\' => o.push_str("\\\\"),
            0x1b => o.push_str("\\e"),
            c if !(32..127).contains(&c) => o.push_str(&format!("\\x{c:02X}")),
            c => o.push(c as char),
        }
    }
    o
}

/// The warning `zend_ini_parse_quantity` reports for a malformed quantity.
pub(crate) fn quantity_error(v: &[u8]) -> Option<String> {
    let ws = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    let mut a = 0;
    let mut b = v.len();
    while a < b && ws(v[a]) {
        a += 1;
    }
    while b > a && ws(v[b - 1]) {
        b -= 1;
    }
    if a == b {
        return None;
    }
    let inv = escaped(v);
    let mut d = a;
    let negative = v[d] == b'-';
    if v[d] == b'+' || v[d] == b'-' {
        d += 1;
    }
    let no_digits = || {
        Some(format!(
            "Invalid quantity \"{inv}\": no valid leading digits, interpreting as \"0\" for backwards compatibility"
        ))
    };
    if d >= b || !v[d].is_ascii_digit() {
        return no_digits();
    }
    let mut base_digits: fn(u8) -> bool = |c: u8| c.is_ascii_digit();
    // ZEND_STRTOUL's base: 0 (decimal, or octal after a leading 0) unless
    // an explicit prefix picked one.
    let mut radix = if v[d] == b'0' { 8 } else { 10 };
    if v[d] == b'0' && d + 1 < b && !v[d + 1].is_ascii_digit() {
        match v[d + 1] {
            b'g' | b'G' | b'm' | b'M' | b'k' | b'K' => {}
            b'x' | b'X' => {
                base_digits = |c| c.is_ascii_hexdigit();
                radix = 16;
                d += 2;
            }
            b'o' | b'O' => {
                base_digits = |c| (b'0'..=b'7').contains(&c);
                radix = 8;
                d += 2;
            }
            b'b' | b'B' => {
                base_digits = |c| c == b'0' || c == b'1';
                radix = 2;
                d += 2;
            }
            c => {
                return Some(format!(
                    "Invalid prefix \"0{}\", interpreting as \"0\" for backwards compatibility",
                    c as char
                ))
            }
        }
        if d >= b || !base_digits(v[d]) {
            return Some(format!(
                "Invalid quantity \"{inv}\": no digits after base prefix, interpreting as \"0\" for backwards compatibility"
            ));
        }
    }
    let mut e = d;
    while e < b && base_digits(v[e]) {
        e += 1;
    }
    // strtoul's ERANGE, then the signed-result check (only PHP_INT_MIN may
    // have its top bit set).
    let mut value: u64 = 0;
    let mut overflow = false;
    for &c in &v[d..e] {
        let digit = (c as char).to_digit(radix);
        let Some(digit) = digit else { break };
        match value
            .checked_mul(u64::from(radix))
            .and_then(|x| x.checked_add(u64::from(digit)))
        {
            Some(x) => value = x,
            None => {
                overflow = true;
                value = u64::MAX;
            }
        }
    }
    let mut signed = value as i64;
    if !overflow {
        if negative && value == 1u64 << 63 {
            signed = i64::MIN;
        } else if signed < 0 {
            overflow = true;
        } else if negative {
            signed = -signed;
        }
    }
    let out_of_range = || {
        Some(format!(
            "Invalid quantity \"{inv}\": value is out of range, using overflow result for backwards compatibility"
        ))
    };
    while e < b && ws(v[e]) {
        e += 1;
    }
    if e == b {
        return if overflow { out_of_range() } else { None };
    }
    let last = v[b - 1];
    let interp = escaped(&v[..e]);
    let chr = escaped(&[last]);
    let factor: i64 = match last {
        b'g' | b'G' => 1 << 30,
        b'm' | b'M' => 1 << 20,
        b'k' | b'K' => 1 << 10,
        _ => {
            return Some(format!(
                "Invalid quantity \"{inv}\": unknown multiplier \"{chr}\", interpreting as \"{interp}\" for backwards compatibility"
            ))
        }
    };
    if !overflow {
        overflow = if signed > 0 {
            signed > i64::MAX / factor
        } else {
            signed < i64::MIN / factor
        };
    }
    if e != b - 1 {
        return Some(format!(
            "Invalid quantity \"{inv}\", interpreting as \"{interp}{chr}\" for backwards compatibility"
        ));
    }
    if overflow {
        return out_of_range();
    }
    None
}
