//! sapi/cli/: php-cli's SAPI module, and php_cli_process_title.c's
//! `cli_set_process_title` and `cli_get_process_title`.

use rhp_values::Value;

use crate::sapi::SapiDef;
use crate::zend::zpp::str_param;
use crate::{Host, NativeFn, Thrown};

/// php-cli: its additional functions.
pub(crate) static SAPI: SapiDef = SapiDef {
    name: "cli",
    functions: FUNCTIONS,
    module: None,
    command_line: true,
    reset: reset_state,
};

const FUNCTIONS: &[(&str, NativeFn)] = &[
    ("dl", crate::ext::standard::dl::fn_dl),
    ("cli_set_process_title", fn_cli_set_process_title),
    ("cli_get_process_title", fn_cli_get_process_title),
];

pub(crate) fn fn_cli_set_process_title(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let t = str_param(host, args, 0, "cli_set_process_title", "title")?;
    TITLE.with(|c| *c.borrow_mut() = Some(t));
    Ok(Value::Bool(true))
}

thread_local! {
    static TITLE: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
}

fn reset_state() {
    TITLE.with(|c| *c.borrow_mut() = None);
}

pub(crate) fn fn_cli_get_process_title(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(
        TITLE.with(|c| c.borrow().clone()).unwrap_or_default(),
    ))
}

/// php-cli's `-m` (print_modules + print_extensions): the module names
/// sorted with strcasecmp, then the Zend extensions.
pub fn modules_text() -> Vec<u8> {
    let mut mods = crate::zend::builtin_functions::loaded_extension_names();
    if let Some((name, _)) = crate::main::sapi::sapi_module().and_then(|d| d.module) {
        mods.push(name);
    }
    mods.sort_by_key(|m| m.to_ascii_lowercase());
    let mut out = b"[PHP Modules]\n".to_vec();
    for m in mods {
        out.extend_from_slice(m.as_bytes());
        out.push(b'\n');
    }
    out.extend_from_slice(b"\n[Zend Modules]\nZend OPcache\n\n");
    out
}

/// php-cli's `--ri <name>`: the module's phpinfo section, or for "main"
/// (no module of that name) the Core directives. `None`: no such module.
pub fn ext_info_text(name: &str) -> Option<Vec<u8>> {
    match crate::zend::builtin_functions::loaded_ext(name.as_bytes()) {
        Some(m) => Some(crate::ext::standard::info::module_info(m)),
        None if name == "main" => Some(crate::ext::standard::info::core_ini_info()),
        None => None,
    }
}

/// php-cli's `--ini`: where php.ini is looked for and what was loaded.
/// `ignore`: `-n` (no scan directory then).
pub fn ini_config_text(ignore: bool) -> Vec<u8> {
    let (opened, scanned) = crate::main::ini::config_files();
    let quoted = |p: Option<Vec<u8>>| match p {
        Some(p) => {
            let mut q = b"\"".to_vec();
            q.extend_from_slice(&p);
            q.push(b'"');
            q
        }
        None => b"(none)".to_vec(),
    };
    // php_ini_scanned_path: $PHP_INI_SCAN_DIR (rhp has no built-in one),
    // unless -n or empty.
    let scan_dir = std::env::var_os("PHP_INI_SCAN_DIR")
        .map(|d| d.into_encoded_bytes())
        .filter(|d| !ignore && !d.is_empty());
    let mut out = format!(
        "Configuration File (php.ini) Path: \"{}\"\n",
        crate::version::CONFIG_FILE_PATH
    )
    .into_bytes();
    out.extend_from_slice(b"Loaded Configuration File:         ");
    out.extend_from_slice(&quoted(opened));
    out.extend_from_slice(b"\nScan for additional .ini files in: ");
    out.extend_from_slice(&quoted(scan_dir));
    out.extend_from_slice(b"\nAdditional .ini files parsed:      ");
    out.extend_from_slice(&scanned.unwrap_or_else(|| b"(none)".to_vec()));
    out.push(b'\n');
    out
}

/// php-cli's `--ini=diff`: the directives whose value differs from the
/// built-in default, sorted by name (strcasecmp).
pub fn ini_diff_text() -> Vec<u8> {
    let mut out = b"Non-default INI settings:\n".to_vec();
    for (name, def, value) in crate::main::ini::non_default_entries() {
        let show = |v: &Option<Vec<u8>>| match v {
            Some(v) => {
                let mut q = b"\"".to_vec();
                q.extend_from_slice(v);
                q.push(b'"');
                q
            }
            None => b"(none)".to_vec(),
        };
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(&show(&def));
        out.extend_from_slice(b" -> ");
        out.extend_from_slice(&show(&value));
        out.push(b'\n');
    }
    out
}
