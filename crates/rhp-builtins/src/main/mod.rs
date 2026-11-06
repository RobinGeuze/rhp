//! php-src's main/: the stream layer, the SAPI layer, request bodies,
//! output control and the ini machinery. This file is main/main.c's part
//! (`set_time_limit`).

use rhp_values::Value;

use crate::{Host, Thrown};

pub mod fopen_wrappers;
pub mod ini;
#[rustfmt::skip]
pub mod ini_table;
pub mod network;
pub mod output;
pub mod php_glob;
pub mod php_variables;
pub mod rfc1867;
pub mod sapi;
pub mod streams;

fn fn_set_time_limit(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let secs = crate::zend::zpp::int_param(host, args, 0, "set_time_limit", "seconds", 0)?;
    let ok = ini::alter_ini_entry(
        host,
        b"max_execution_time",
        secs.to_string().into_bytes(),
        "set_time_limit",
    )?;
    Ok(Value::Bool(ok.is_some()))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("set_time_limit", fn_set_time_limit)]);
    streams::userspace::register(m);
}

/// A new request: reset main/'s thread-locals (the stream layer's, the ini
/// overrides, the SAPI).
pub(crate) fn reset_state() {
    streams::reset_state();
    streams::plain_wrapper::reset_state();
    streams::xp_socket::reset_state();
    ini::reset_state();
    fopen_wrappers::reset_state();
    sapi::reset_state();
}

/// `(PHP_OS, PHP_OS_FAMILY)` for the platform this engine was built for.
fn php_os() -> (&'static str, &'static str) {
    if cfg!(target_os = "macos") {
        ("Darwin", "Darwin")
    } else if cfg!(target_os = "linux") {
        ("Linux", "Linux")
    } else if cfg!(target_os = "windows") {
        ("WINNT", "Windows")
    } else if cfg!(target_os = "freebsd") {
        ("FreeBSD", "BSD")
    } else if cfg!(target_os = "openbsd") {
        ("OpenBSD", "BSD")
    } else if cfg!(target_os = "netbsd") {
        ("NetBSD", "BSD")
    } else if cfg!(target_os = "solaris") {
        ("SunOS", "Solaris")
    } else {
        ("Unknown", "Unknown")
    }
}

/// The constants php_module_startup registers (`PHP_VERSION`, `PHP_EOL`,
/// `PHP_INT_MAX`, the build paths, …), and the CLI's.
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    use crate::version as v;
    vec![
        ("PHP_EOL", Value::str("\n")),
        ("PHP_INT_MAX", Value::Int(i64::MAX)),
        ("PHP_INT_MIN", Value::Int(i64::MIN)),
        ("PHP_INT_SIZE", Value::Int(8)),
        ("PHP_FLOAT_EPSILON", Value::Float(f64::EPSILON)),
        ("PHP_FLOAT_MAX", Value::Float(f64::MAX)),
        ("PHP_FLOAT_MIN", Value::Float(f64::MIN_POSITIVE)),
        ("PHP_FLOAT_DIG", Value::Int(15)),
        ("PHP_VERSION", Value::str(v::PHP_VERSION)),
        ("PHP_MAJOR_VERSION", Value::Int(v::PHP_MAJOR_VERSION)),
        ("PHP_MINOR_VERSION", Value::Int(v::PHP_MINOR_VERSION)),
        ("PHP_RELEASE_VERSION", Value::Int(v::PHP_RELEASE_VERSION)),
        ("PHP_EXTRA_VERSION", Value::str(v::PHP_EXTRA_VERSION)),
        ("PHP_VERSION_ID", Value::Int(v::PHP_VERSION_ID)),
        ("PHP_SAPI", Value::str("cli")),
        // php reports its own executable; so do we (composer spawns PHP_BINARY).
        (
            "PHP_BINARY",
            Value::str(
                std::env::current_exe()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
        ),
        // php bakes in its build host's OS (uname -s) and family.
        ("PHP_OS", Value::str(php_os().0)),
        ("PHP_OS_FAMILY", Value::str(php_os().1)),
        ("PHP_FD_SETSIZE", Value::Int(1024)),
        // php-cli only (the VM drops it under other SAPIs, like STDIN).
        ("PHP_CLI_PROCESS_TITLE", Value::Bool(true)),
        ("PHP_DEBUG", Value::Bool(false)),
        ("PHP_ZTS", Value::Bool(false)),
        ("PHP_MAXPATHLEN", Value::Int(libc::PATH_MAX as i64)), // MAXPATHLEN: 1024 on macOS, 4096 on Linux
        ("PHP_BINDIR", Value::str(v::BINDIR)),
        ("PHP_PREFIX", Value::str(v::PREFIX)),
        ("PHP_EXTENSION_DIR", Value::str(v::EXTENSION_DIR)),
        ("DEFAULT_INCLUDE_PATH", Value::str(v::INCLUDE_PATH)),
        ("PHP_BUILD_DATE", Value::str(v::BUILD_DATE)),
        ("PHP_BUILD_PROVIDER", Value::str(v::BUILD_PROVIDER)),
        ("PEAR_INSTALL_DIR", Value::str(v::PEAR_INSTALL_DIR)),
        ("PEAR_EXTENSION_DIR", Value::str(v::PEAR_EXTENSION_DIR)),
        ("PHP_SBINDIR", Value::str(v::SBINDIR)),
        ("PHP_MANDIR", Value::str(v::MANDIR)),
        ("PHP_LIBDIR", Value::str(v::LIBDIR)),
        ("PHP_DATADIR", Value::str(v::DATADIR)),
        ("PHP_SYSCONFDIR", Value::str(v::SYSCONFDIR)),
        ("PHP_LOCALSTATEDIR", Value::str(v::LOCALSTATEDIR)),
        ("PHP_CONFIG_FILE_PATH", Value::str(v::CONFIG_FILE_PATH)),
        (
            "PHP_CONFIG_FILE_SCAN_DIR",
            Value::str(v::CONFIG_FILE_SCAN_DIR),
        ),
        ("PHP_SHLIB_SUFFIX", Value::str(v::SHLIB_SUFFIX)),
        // Placeholders: the VM binds the stream resources when a request
        // starts (php-cli), or removes them (other SAPIs).
        ("STDIN", Value::Int(0)),
        ("STDOUT", Value::Int(1)),
        ("STDERR", Value::Int(2)),
    ]
}
