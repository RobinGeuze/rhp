//! Native function + constant registry, and the narrow `Host` interface the
//! interpreter exposes to builtins.
//!
//! Builtins operate on owned `Value`s only — they never touch the arena/AST,
//! so this crate depends on `rhp-values` and `rhp-bytecode` (the stub
//! signatures), plus the C libraries it links, never on the compiler or the
//! VM (no dependency cycle).
//! The interpreter talks back through [`Host`].

// `main` is php-src's main/, not a binary's entry point.
#![allow(special_module_name)]

use std::collections::HashMap;
use std::sync::OnceLock;

use rhp_values::conv;
use rhp_values::{PhpArray, Value};

pub use rhp_values::Thrown;

mod diag;
mod host;
pub mod main;
pub(crate) mod module;
pub mod pcre;
pub mod version;
pub mod zend;

use crate::ext::standard::array::as_array;
use crate::zend::builtin_functions::resolve_const;
use crate::zend::zpp::require_callback;
pub use diag::{
    deprecated, deprecated_fn, flush_stream_diags, notice, open_failed, warn, warn_arg,
};
pub use host::{host_compare, host_loose_eq, Host, MagicFetch, ObCall, StreamKind, StreamMeta};
pub use main::streams::{StatInfo, StreamWrapper, WrapperReg};

/// pcntl's asynchronous signal delivery: the VM polls
/// [`pcntl_interrupt_pending`] between instructions and, when set, calls
/// [`pcntl_dispatch`] to run the queued PHP signal handlers.
pub use ext::pcntl::{dispatch as pcntl_dispatch, interrupt_pending as pcntl_interrupt_pending};

#[rustfmt::skip]
mod const_modules;
mod crypt_fns;
mod ext;
mod file_fns;
mod filter_fns;
mod fs_fns;
mod html;
#[rustfmt::skip]
mod html_tables;
pub mod http;
mod inet;
mod info_fns;
mod ini_parse;
#[rustfmt::skip]
mod ini_table;
mod json_fns;
mod misc_fns;
mod opcache_fns;
mod parse_fns;
pub mod php_fns;
mod preg_fns;
mod printf_fns;
mod proc_fns;
mod query_fns;
mod runtime_fns;
mod socket_fns;
mod stdext_fns;
mod str_fns;
mod strarr2_fns;
mod strcmp_fns;
mod stream_fns;
mod strsearch_fns;
mod sysmisc_fns;
pub(crate) mod url;
mod webenc_fns;

#[cfg(test)]
mod test_support;

/// The E_DEPRECATED message for reading a `#[\Deprecated]` extension
/// constant, if `name` is one.
pub fn constant_deprecation(name: &[u8]) -> Option<String> {
    ext::curl::constant_deprecation(name)
        .or_else(|| ext::date::constant_deprecation(name))
        .or_else(|| ext::dom::constant_deprecation(name))
}

/// The value php's compiler substitutes for the global constant `name`
/// (zend_try_ct_eval_const): a builtin constant that is not deprecated.
pub fn ct_constant(name: &[u8]) -> Option<Value> {
    if constant_deprecation(name).is_some() {
        return None;
    }
    constants()
        .into_iter()
        .find(|(n, _)| n.as_bytes() == name)
        .map(|(_, v)| v)
}

/// The E_DEPRECATED a fetch of a built-in class's `#[\Deprecated]`
/// constant raises (`class` is the declaring class).
pub fn class_constant_deprecation(class: &[u8], name: &[u8]) -> Option<String> {
    ext::date::class_constant_deprecation(class, name)
        .or_else(|| ext::zip::class_constant_deprecation(class, name))
}

/// The engine's object free hook (`rhp_values::set_object_free_hook`), for
/// the per-object state still kept by object id.
pub fn object_freed(id: u32) {
    ext::standard::array::object_freed(id);
    ext::openssl::object_freed(id);
    ext::curl::object_freed(id);
}

// php's error levels, for the builtins and the VM.
pub use rhp_values::{
    E_ALL, E_COMPILE_ERROR, E_COMPILE_WARNING, E_CORE_ERROR, E_CORE_WARNING, E_DEPRECATED, E_ERROR,
    E_NOTICE, E_PARSE, E_RECOVERABLE_ERROR, E_STRICT, E_USER_DEPRECATED, E_USER_ERROR,
    E_USER_NOTICE, E_USER_WARNING, E_WARNING,
};

/// The refcount-0 destructor of a native (non-stream) resource, given its
/// payload: a process closes its pipes and reaps the child if it already
/// exited (proc_open_rsrc_dtor); anything else is just dropped.
pub fn native_resource_dtor(host: &mut dyn Host, data: Box<dyn std::any::Any>) {
    proc_fns::proc_dtor(host, data);
}

/// A native builtin function over owned values.
pub type NativeFn = fn(&mut dyn Host, &[Value]) -> Result<Value, Thrown>;

/// Registry entry: a native function plus its arity bounds.
pub struct Builtin {
    pub name: &'static str,
    pub func: NativeFn,
    pub min_args: usize,
    /// `None` = variadic (no upper bound).
    pub max_args: Option<usize>,
}

/// The builtin table under construction, keyed by lowercase name.
pub(crate) type Registry = HashMap<&'static str, Builtin>;

/// Register php functions by name. Their arity is php's, from the stub
/// signatures (every registered php function has a stub row; the
/// `php_builtins_have_stubs` test checks), raised to [`BODY_MIN`].
pub(crate) fn register(m: &mut Registry, fns: &[(&'static str, NativeFn)]) {
    use rhp_bytecode::signatures::{self, Signature};
    for &(name, func) in fns {
        let (min_args, max_args) = signatures::function(name.as_bytes())
            .map_or((0, None), |f| (f.min_args(), f.max_args()));
        let min_args = BODY_MIN
            .iter()
            .find(|(n, _)| *n == name)
            .map_or(min_args, |&(_, min)| min);
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args,
                max_args,
            },
        );
    }
}

/// Register engine helpers: `__` builtins only prelude code calls, with no
/// stub row, so their arity (min, max; `None` = variadic) is given here.
pub(crate) fn register_helpers(
    m: &mut Registry,
    fns: &[(&'static str, NativeFn, usize, Option<usize>)],
) {
    for &(name, func, min_args, max_args) in fns {
        debug_assert!(name.starts_with("__"), "{name} is not an engine helper");
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args,
                max_args,
            },
        );
    }
}

/// Functions whose php body requires more arguments than their arginfo
/// (`array_udiff(): expects at least 2 arguments`; openssl_cms_verify's zpp
/// wants `$flags`).
const BODY_MIN: &[(&str, usize)] = &[
    ("array_diff_uassoc", 2),
    ("array_diff_ukey", 2),
    ("array_intersect_uassoc", 2),
    ("array_intersect_ukey", 2),
    ("array_udiff", 2),
    ("array_udiff_assoc", 2),
    ("array_udiff_uassoc", 3),
    ("array_uintersect", 2),
    ("array_uintersect_assoc", 2),
    ("array_uintersect_uassoc", 3),
    ("openssl_cms_verify", 2),
];

pub use php_fns::{apply_env_overrides, env_overrides};
pub use zend::zpp::{zval_type_name, zval_value_name};

/// The php version this engine reports (`PHP_VERSION`).
pub const PHP_VERSION: &str = version::PHP_VERSION;

/// A directive's current value: `None` = unknown directive, `Some(None)` =
/// registered with a null value.
pub fn ini_value(name: &[u8]) -> Option<Option<Vec<u8>>> {
    runtime_fns::ini_value(name)
}

/// C `atol` (php's ZEND_ATOL): leading blanks, sign, digits; saturating.
pub fn c_atol(s: &[u8]) -> i64 {
    runtime_fns::c_atol(s)
}

/// C `atoi`: [`c_atol`] cut to 32 bits.
pub fn c_atoi(s: &[u8]) -> i32 {
    runtime_fns::c_atoi(s)
}

/// `zend_ini_parse_bool`: "true", "yes", "on" (any case), else atoi != 0.
pub fn ini_parse_bool(v: &[u8]) -> bool {
    runtime_fns::ini_parse_bool(v)
}

/// `zend_ini_parse_quantity`: `8M` → 8388608.
pub fn ini_parse_quantity(s: &[u8]) -> i64 {
    runtime_fns::ini_parse_quantity(s)
}

/// A directive's startup value (`-d`, else the SAPI's or the built-in
/// default): what php's PHP_INI_SYSTEM directives keep. `None` = unknown.
pub fn ini_startup_value(name: &[u8]) -> Option<Vec<u8>> {
    runtime_fns::ini_default_value(name).map(Option::unwrap_or_default)
}

pub use opcache_fns::{
    accel_log_dest, accel_log_line, asctime_local, preload_user_check, take_accel_log,
};

/// A startup ini setting (`php -d name=value`): any registered directive,
/// applied to the engine state it drives (precision, ...). Unknown names are
/// ignored. Call after [`reset_request_state`].
pub fn ini_startup_set(name: &[u8], value: &[u8]) {
    runtime_fns::ini_startup(name, value);
}

pub use module::{output_handler_alias, OutputHandlerAlias, OutputHandlerCtx, OutputHandlerFn};
pub use runtime_fns::{apply_memory_limits, take_shutdown_diagnostics, take_startup_diagnostics};

/// Reset every builtin's per-request state (ini overrides, default timezone,
/// precision, strtok/preg/json/mbstring state, process title, putenv
/// overlay). Engines call this on construction so state never leaks from one
/// script run to the next on the same thread.
pub fn reset_request_state() {
    zend::virtual_cwd::reset_state();
    main::streams::reset_state();
    main::streams::plain_wrapper::reset_state();
    ext::standard::var::clear_views();
    ext::standard::array::reset_state();
    socket_fns::reset_state();
    ext::sockets::reset_state();
    ext::libxml::reset_state();
    ext::date::reset_state();
    runtime_fns::reset_state();
    sysmisc_fns::reset_state();
    preg_fns::reset_state();
    json_fns::reset_state();
    #[cfg(feature = "mbstring-libmbfl")]
    ext::mbstring::reset_state();
    strsearch_fns::reset_state();
    php_fns::reset_state();
    stdext_fns::reset_state();
    main::sapi::reset_state();
    ext::curl::reset_state();
    ext::openssl::reset_state();
    ext::bz2::reset_state();
    ext::iconv::reset_state();
    ext::gmp::reset_state();
    ext::intl::error_reset();
    rhp_values::reset_dropped_streams();
}

/// The builtin function table, keyed by lowercase name (PHP function names are
/// case-insensitive).
pub fn registry() -> &'static HashMap<&'static str, Builtin> {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(|| {
        let mut m = Registry::new();
        register(
            &mut m,
            &[
                ("print_r", fn_print_r),
                ("constant", fn_constant),
                ("str_repeat", fn_str_repeat),
                ("call_user_func", fn_call_user_func),
                ("call_user_func_array", fn_call_user_func_array),
                ("implode", fn_implode),
                ("join", fn_implode),
                ("explode", fn_explode),
                ("strtoupper", fn_strtoupper),
                ("strtolower", fn_strtolower),
                ("str_contains", fn_str_contains),
                ("ord", fn_ord),
                ("chr", fn_chr),
            ],
        );
        for (_, register_module) in builtin_modules() {
            register_module(&mut m);
        }
        m
    })
}

type Registrar = Box<dyn Fn(&mut Registry)>;

/// Every builtin module's registration, in registry order (a later module
/// would override an earlier one's function of the same name; the
/// `no_duplicate_builtins` test keeps names unique).
fn builtin_modules() -> Vec<(&'static str, Registrar)> {
    let mut v: Vec<(&'static str, Registrar)> = vec![
        ("str_fns", Box::new(str_fns::register)),
        ("math", Box::new(ext::standard::math::register)),
        ("array", Box::new(ext::standard::array::register)),
        ("misc_fns", Box::new(misc_fns::register)),
        ("file_fns", Box::new(file_fns::register)),
        ("type", Box::new(ext::standard::r#type::register)),
    ];
    // Discrete extensions (php-src ext/<name>/ layout): each a self-contained
    // module owning its functions/constants/classes/wrappers.
    for md in ext::modules() {
        v.push((md.name, Box::new(move |m| (md.functions)(m))));
    }
    let rest: Vec<(&'static str, Registrar)> = vec![
        ("webenc_fns", Box::new(webenc_fns::register)),
        ("html", Box::new(html::register)),
        ("strarr2_fns", Box::new(strarr2_fns::register)),
        ("var", Box::new(ext::standard::var::register)),
        ("filter_fns", Box::new(filter_fns::register)),
        ("strcmp_fns", Box::new(strcmp_fns::register)),
        ("printf_fns", Box::new(printf_fns::register)),
        ("preg_fns", Box::new(preg_fns::register)),
        ("json_fns", Box::new(json_fns::register)),
        ("parse_fns", Box::new(parse_fns::register)),
        ("php_fns", Box::new(php_fns::register)),
        ("info_fns", Box::new(info_fns::register)),
        ("stream_fns", Box::new(stream_fns::register)),
        ("socket_fns", Box::new(socket_fns::register)),
        ("strsearch_fns", Box::new(strsearch_fns::register)),
        ("query_fns", Box::new(query_fns::register)),
        ("runtime_fns", Box::new(runtime_fns::register)),
        (
            "builtin_functions",
            Box::new(zend::builtin_functions::register),
        ),
        ("exceptions", Box::new(zend::exceptions::register)),
        ("closures", Box::new(zend::closures::register)),
        ("zpp", Box::new(zend::zpp::register)),
        ("sapi", Box::new(main::sapi::register)),
        ("rfc1867", Box::new(main::rfc1867::register)),
        ("opcache_fns", Box::new(opcache_fns::register)),
        ("ini_parse", Box::new(ini_parse::register)),
        ("sysmisc_fns", Box::new(sysmisc_fns::register)),
        ("stdext_fns", Box::new(stdext_fns::register)),
        ("crypt_fns", Box::new(crypt_fns::register)),
        ("proc_fns", Box::new(proc_fns::register)),
        ("fs_fns", Box::new(fs_fns::register)),
    ];
    v.extend(rest);
    v
}

#[cfg(test)]
mod tests;

/// Case-insensitive builtin lookup.
pub fn lookup(name: &str) -> Option<&'static Builtin> {
    let lower = name.to_ascii_lowercase();
    lookup_lc(&lower)
}

/// [`lookup`] for a name that is already lowercase (no allocation).
pub fn lookup_lc(name_lc: &str) -> Option<&'static Builtin> {
    let b = registry().get(name_lc)?;
    if matches!(name_lc.as_bytes().first(), Some(b'a' | b'c' | b'g' | b'p'))
        && main::sapi::builtin_hidden(name_lc)
    {
        return None;
    }
    Some(b)
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

/// Engine constants (`PHP_EOL`, `PHP_INT_MAX`, …).
pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("PHP_EOL", Value::str("\n")),
        ("PHP_INT_MAX", Value::Int(i64::MAX)),
        ("PHP_INT_MIN", Value::Int(i64::MIN)),
        ("PHP_INT_SIZE", Value::Int(8)),
        ("PHP_FLOAT_EPSILON", Value::Float(f64::EPSILON)),
        ("PHP_FLOAT_MAX", Value::Float(f64::MAX)),
        ("PHP_FLOAT_MIN", Value::Float(f64::MIN_POSITIVE)),
        ("PHP_VERSION", Value::str(version::PHP_VERSION)),
        ("PHP_MAJOR_VERSION", Value::Int(version::PHP_MAJOR_VERSION)),
        ("PHP_MINOR_VERSION", Value::Int(version::PHP_MINOR_VERSION)),
        (
            "PHP_RELEASE_VERSION",
            Value::Int(version::PHP_RELEASE_VERSION),
        ),
        ("PHP_EXTRA_VERSION", Value::str(version::PHP_EXTRA_VERSION)),
        ("PHP_VERSION_ID", Value::Int(version::PHP_VERSION_ID)),
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
        ("ZEND_THREAD_SAFE", Value::Bool(false)),
        ("ZEND_DEBUG_BUILD", Value::Bool(false)),
        ("ZEND_VM_KIND", Value::str(version::ZEND_VM_KIND)),
        ("TRUE", Value::Bool(true)),
        ("FALSE", Value::Bool(false)),
        ("NULL", Value::Null),
        ("PHP_FD_SETSIZE", Value::Int(1024)),
        ("UPLOAD_ERR_OK", Value::Int(0)),
        ("UPLOAD_ERR_INI_SIZE", Value::Int(1)),
        ("UPLOAD_ERR_FORM_SIZE", Value::Int(2)),
        ("UPLOAD_ERR_PARTIAL", Value::Int(3)),
        ("UPLOAD_ERR_NO_FILE", Value::Int(4)),
        ("UPLOAD_ERR_NO_TMP_DIR", Value::Int(6)),
        ("UPLOAD_ERR_CANT_WRITE", Value::Int(7)),
        ("UPLOAD_ERR_EXTENSION", Value::Int(8)),
        // php-cli only (the VM drops it under other SAPIs, like STDIN).
        ("PHP_CLI_PROCESS_TITLE", Value::Bool(true)),
        ("PHP_DEBUG", Value::Bool(false)),
        ("PHP_ZTS", Value::Bool(false)),
        // LIBXML_* live with the libxml module (ext/libxml).
        ("FILE_USE_INCLUDE_PATH", Value::Int(1)),
        ("FILE_IGNORE_NEW_LINES", Value::Int(2)),
        ("FILE_SKIP_EMPTY_LINES", Value::Int(4)),
        ("FILE_APPEND", Value::Int(8)),
        ("FILE_NO_DEFAULT_CONTEXT", Value::Int(16)),
        ("FILE_TEXT", Value::Int(0)),
        ("FILE_BINARY", Value::Int(0)),
        // SEEK_* come from fs_fns::constants().
        ("LOCK_SH", Value::Int(1)),
        ("LOCK_EX", Value::Int(2)),
        ("LOCK_UN", Value::Int(3)),
        ("LOCK_NB", Value::Int(4)),
        #[cfg(feature = "mbstring-libmbfl")]
        (
            "MB_ONIGURUMA_VERSION",
            Value::str(version::ONIGURUMA_VERSION),
        ),
        ("DIRECTORY_SEPARATOR", Value::str("/")),
        ("PATH_SEPARATOR", Value::str(":")),
        ("PHP_MAXPATHLEN", Value::Int(libc::PATH_MAX as i64)), // MAXPATHLEN: 1024 on macOS, 4096 on Linux
        ("PHP_BINDIR", Value::str(version::BINDIR)),
        ("PHP_PREFIX", Value::str(version::PREFIX)),
        ("PHP_EXTENSION_DIR", Value::str(version::EXTENSION_DIR)),
        ("DEFAULT_INCLUDE_PATH", Value::str(version::INCLUDE_PATH)),
        ("PHP_BUILD_DATE", Value::str(version::BUILD_DATE)),
        ("PHP_BUILD_PROVIDER", Value::str(version::BUILD_PROVIDER)),
        ("PEAR_INSTALL_DIR", Value::str(version::PEAR_INSTALL_DIR)),
        (
            "PEAR_EXTENSION_DIR",
            Value::str(version::PEAR_EXTENSION_DIR),
        ),
        ("PHP_SBINDIR", Value::str(version::SBINDIR)),
        ("PHP_MANDIR", Value::str(version::MANDIR)),
        ("PHP_LIBDIR", Value::str(version::LIBDIR)),
        ("PHP_DATADIR", Value::str(version::DATADIR)),
        ("PHP_SYSCONFDIR", Value::str(version::SYSCONFDIR)),
        ("PHP_LOCALSTATEDIR", Value::str(version::LOCALSTATEDIR)),
        (
            "PHP_CONFIG_FILE_PATH",
            Value::str(version::CONFIG_FILE_PATH),
        ),
        (
            "PHP_CONFIG_FILE_SCAN_DIR",
            Value::str(version::CONFIG_FILE_SCAN_DIR),
        ),
        ("PHP_SHLIB_SUFFIX", Value::str(version::SHLIB_SUFFIX)),
        ("LC_CTYPE", Value::Int(0)),
        ("LC_NUMERIC", Value::Int(1)),
        ("LC_TIME", Value::Int(2)),
        ("LC_COLLATE", Value::Int(3)),
        ("LC_MONETARY", Value::Int(4)),
        ("LC_MESSAGES", Value::Int(5)),
        ("LC_ALL", Value::Int(6)),
        ("INI_SCANNER_NORMAL", Value::Int(0)),
        ("INI_SCANNER_RAW", Value::Int(1)),
        ("INI_SCANNER_TYPED", Value::Int(2)),
        ("PHP_QUERY_RFC1738", Value::Int(1)),
        ("PHP_QUERY_RFC3986", Value::Int(2)),
        ("PHP_URL_SCHEME", Value::Int(0)),
        ("PHP_URL_HOST", Value::Int(1)),
        ("PHP_URL_PORT", Value::Int(2)),
        ("PHP_URL_USER", Value::Int(3)),
        ("PHP_URL_PASS", Value::Int(4)),
        ("PHP_URL_PATH", Value::Int(5)),
        ("PHP_URL_QUERY", Value::Int(6)),
        ("PHP_URL_FRAGMENT", Value::Int(7)),
        ("PATHINFO_DIRNAME", Value::Int(1)),
        ("PATHINFO_BASENAME", Value::Int(2)),
        ("PATHINFO_EXTENSION", Value::Int(4)),
        ("PATHINFO_FILENAME", Value::Int(8)),
        ("PATHINFO_ALL", Value::Int(15)),
        ("DEBUG_BACKTRACE_PROVIDE_OBJECT", Value::Int(1)),
        ("DEBUG_BACKTRACE_IGNORE_ARGS", Value::Int(2)),
        ("E_ERROR", Value::Int(E_ERROR)),
        ("E_PARSE", Value::Int(E_PARSE)),
        ("E_CORE_ERROR", Value::Int(E_CORE_ERROR)),
        ("E_CORE_WARNING", Value::Int(E_CORE_WARNING)),
        ("E_COMPILE_ERROR", Value::Int(E_COMPILE_ERROR)),
        ("E_COMPILE_WARNING", Value::Int(E_COMPILE_WARNING)),
        ("E_USER_ERROR", Value::Int(E_USER_ERROR)),
        ("E_USER_WARNING", Value::Int(E_USER_WARNING)),
        ("E_USER_NOTICE", Value::Int(E_USER_NOTICE)),
        ("E_STRICT", Value::Int(E_STRICT)),
        ("E_RECOVERABLE_ERROR", Value::Int(E_RECOVERABLE_ERROR)),
        ("E_DEPRECATED", Value::Int(E_DEPRECATED)),
        ("E_USER_DEPRECATED", Value::Int(E_USER_DEPRECATED)),
        ("NAN", Value::Float(f64::NAN)),
        ("INF", Value::Float(f64::INFINITY)),
        ("E_ALL", Value::Int(E_ALL)),
        ("E_WARNING", Value::Int(E_WARNING)),
        ("E_NOTICE", Value::Int(E_NOTICE)),
        ("COUNT_NORMAL", Value::Int(0)),
        ("COUNT_RECURSIVE", Value::Int(1)),
        // Placeholders: the VM binds the stream resources when a request
        // starts (php-cli), or removes them (other SAPIs).
        ("STDIN", Value::Int(0)),
        ("STDOUT", Value::Int(1)),
        ("STDERR", Value::Int(2)),
        ("CONNECTION_ABORTED", Value::Int(1)),
        ("CONNECTION_NORMAL", Value::Int(0)),
        ("CONNECTION_TIMEOUT", Value::Int(2)),
        ("STR_PAD_RIGHT", Value::Int(1)),
        ("STR_PAD_LEFT", Value::Int(0)),
        ("STR_PAD_BOTH", Value::Int(2)),
        ("SORT_REGULAR", Value::Int(0)),
        ("SORT_NUMERIC", Value::Int(1)),
        ("SORT_STRING", Value::Int(2)),
        ("SORT_DESC", Value::Int(3)),
        ("SORT_ASC", Value::Int(4)),
        ("SORT_LOCALE_STRING", Value::Int(5)),
        ("SORT_NATURAL", Value::Int(6)),
        ("SORT_FLAG_CASE", Value::Int(8)),
        #[cfg(feature = "mbstring-libmbfl")]
        ("MB_CASE_UPPER", Value::Int(0)),
        #[cfg(feature = "mbstring-libmbfl")]
        ("MB_CASE_LOWER", Value::Int(1)),
        #[cfg(feature = "mbstring-libmbfl")]
        ("MB_CASE_TITLE", Value::Int(2)),
        #[cfg(feature = "mbstring-libmbfl")]
        ("MB_CASE_FOLD", Value::Int(3)),
        #[cfg(feature = "mbstring-libmbfl")]
        ("MB_CASE_UPPER_SIMPLE", Value::Int(4)),
        #[cfg(feature = "mbstring-libmbfl")]
        ("MB_CASE_LOWER_SIMPLE", Value::Int(5)),
        #[cfg(feature = "mbstring-libmbfl")]
        ("MB_CASE_TITLE_SIMPLE", Value::Int(6)),
        #[cfg(feature = "mbstring-libmbfl")]
        ("MB_CASE_FOLD_SIMPLE", Value::Int(7)),
        ("FILTER_DEFAULT", Value::Int(516)),
        ("FILTER_UNSAFE_RAW", Value::Int(516)),
        ("FILTER_VALIDATE_INT", Value::Int(257)),
        ("FILTER_VALIDATE_BOOLEAN", Value::Int(258)),
        ("FILTER_VALIDATE_BOOL", Value::Int(258)),
        ("FILTER_VALIDATE_FLOAT", Value::Int(259)),
        ("FILTER_VALIDATE_REGEXP", Value::Int(272)),
        ("FILTER_VALIDATE_DOMAIN", Value::Int(277)),
        ("FILTER_VALIDATE_URL", Value::Int(273)),
        ("FILTER_VALIDATE_EMAIL", Value::Int(274)),
        ("FILTER_VALIDATE_IP", Value::Int(275)),
        ("FILTER_VALIDATE_MAC", Value::Int(276)),
        ("FILTER_SANITIZE_ENCODED", Value::Int(514)),
        ("FILTER_SANITIZE_SPECIAL_CHARS", Value::Int(515)),
        ("FILTER_SANITIZE_EMAIL", Value::Int(517)),
        ("FILTER_SANITIZE_URL", Value::Int(518)),
        ("FILTER_SANITIZE_NUMBER_INT", Value::Int(519)),
        ("FILTER_SANITIZE_NUMBER_FLOAT", Value::Int(520)),
        ("FILTER_SANITIZE_FULL_SPECIAL_CHARS", Value::Int(522)),
        ("FILTER_SANITIZE_ADD_SLASHES", Value::Int(523)),
        ("FILTER_FLAG_NONE", Value::Int(0)),
        ("FILTER_NULL_ON_FAILURE", Value::Int(134217728)),
        ("FILTER_FLAG_ALLOW_OCTAL", Value::Int(1)),
        ("FILTER_FLAG_ALLOW_HEX", Value::Int(2)),
        ("FILTER_FLAG_ALLOW_THOUSAND", Value::Int(8192)),
        ("FILTER_FLAG_ALLOW_SCIENTIFIC", Value::Int(16384)),
        ("FILTER_FLAG_IPV4", Value::Int(1048576)),
        ("FILTER_FLAG_IPV6", Value::Int(2097152)),
        ("FILTER_FLAG_NO_PRIV_RANGE", Value::Int(8388608)),
        ("FILTER_FLAG_NO_RES_RANGE", Value::Int(4194304)),
        ("FILTER_FLAG_PATH_REQUIRED", Value::Int(262144)),
        ("FILTER_FLAG_QUERY_REQUIRED", Value::Int(524288)),
        ("FILTER_FLAG_HOSTNAME", Value::Int(1048576)),
        ("FILTER_FLAG_EMAIL_UNICODE", Value::Int(1048576)),
        ("ENT_QUOTES", Value::Int(3)),
        ("ENT_COMPAT", Value::Int(2)),
        ("ENT_NOQUOTES", Value::Int(0)),
        ("ENT_HTML401", Value::Int(0)),
        ("ENT_HTML5", Value::Int(48)),
        ("ENT_XML1", Value::Int(16)),
        ("ENT_XHTML", Value::Int(32)),
        ("ENT_IGNORE", Value::Int(4)),
        ("ENT_SUBSTITUTE", Value::Int(8)),
        ("ENT_DISALLOWED", Value::Int(128)),
        ("HTML_SPECIALCHARS", Value::Int(0)),
        ("HTML_ENTITIES", Value::Int(1)),
        ("PHP_ROUND_HALF_UP", Value::Int(1)),
        ("PHP_ROUND_HALF_DOWN", Value::Int(2)),
        ("PHP_ROUND_HALF_EVEN", Value::Int(3)),
        ("PHP_ROUND_HALF_ODD", Value::Int(4)),
        ("ARRAY_FILTER_USE_KEY", Value::Int(2)),
        ("ARRAY_FILTER_USE_BOTH", Value::Int(1)),
        ("M_PI", Value::Float(std::f64::consts::PI)),
        ("M_E", Value::Float(std::f64::consts::E)),
        ("M_SQRT2", Value::Float(std::f64::consts::SQRT_2)),
        ("PHP_FLOAT_DIG", Value::Int(15)),
        // main/php_output.h
        ("PHP_OUTPUT_HANDLER_START", Value::Int(0x01)),
        ("PHP_OUTPUT_HANDLER_WRITE", Value::Int(0x00)),
        ("PHP_OUTPUT_HANDLER_FLUSH", Value::Int(0x04)),
        ("PHP_OUTPUT_HANDLER_CLEAN", Value::Int(0x02)),
        ("PHP_OUTPUT_HANDLER_FINAL", Value::Int(0x08)),
        ("PHP_OUTPUT_HANDLER_CONT", Value::Int(0x00)),
        ("PHP_OUTPUT_HANDLER_END", Value::Int(0x08)),
        ("PHP_OUTPUT_HANDLER_CLEANABLE", Value::Int(0x10)),
        ("PHP_OUTPUT_HANDLER_FLUSHABLE", Value::Int(0x20)),
        ("PHP_OUTPUT_HANDLER_REMOVABLE", Value::Int(0x40)),
        ("PHP_OUTPUT_HANDLER_STDFLAGS", Value::Int(0x70)),
        ("PHP_OUTPUT_HANDLER_STARTED", Value::Int(0x1000)),
        ("PHP_OUTPUT_HANDLER_DISABLED", Value::Int(0x2000)),
        ("PHP_OUTPUT_HANDLER_PROCESSED", Value::Int(0x4000)),
    ]
    .into_iter()
    .chain(preg_fns::preg_constants())
    .chain(json_fns::json_constants())
    .chain(info_fns::constants())
    .chain(fs_fns::constants())
    .chain(stdext_fns::constants())
    .chain(crypt_fns::constants())
    .chain(socket_fns::constants())
    // Per-extension constants (GMP_*, PASSWORD_*, ZLIB_*, …) live with their
    // module; pcntl's only under the SAPIs that have it.
    .chain(
        ext::modules()
            .into_iter()
            .filter(|md| md.name != "pcntl" || main::sapi::has_pcntl())
            .flat_map(|md| (md.constants)()),
    )
    .collect()
}

/// The PHP prelude the compiler compiles into every program: the engine's
/// core classes and interfaces (core_prelude.php), then each module's
/// PHP-source classes (GMP, Phar, …) and helper functions (iterator_*, …).
/// The core comes first, so its class indices never change. Built once per
/// process, outside any request's memory.
pub fn prelude() -> &'static str {
    static PRELUDE: OnceLock<String> = OnceLock::new();
    PRELUDE.get_or_init(|| rhp_values::mem::uncounted(build_prelude))
}

fn build_prelude() -> String {
    let mut s = String::from("<?php\n");
    s.push_str(include_str!("core_prelude.php"));
    s.push('\n');
    s.push_str(fs_fns::PRELUDE_PHP);
    s.push('\n');
    s.push_str(stdext_fns::PRELUDE_PHP);
    s.push('\n');
    for md in ext::modules() {
        if !md.prelude_php.is_empty() {
            s.push_str(md.prelude_php);
            s.push('\n');
        }
    }
    s
}

/// Built-in stream wrappers contributed by extension modules (e.g. `phar://`),
/// installed into the VM's per-run registry at startup.
pub fn stream_wrappers() -> Vec<WrapperReg> {
    ext::modules()
        .into_iter()
        .flat_map(|md| (md.wrappers)())
        .collect()
}

/// If `name` is a deliberately-refused high-risk function, return why.
pub fn is_refused(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "extract" => Some("writes arbitrary variables into the local scope"),
        "compact" => Some("resolves variable names dynamically from strings"),
        "create_function" => Some("compiles a string into a function (eval-equivalent)"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// builtin implementations
// ---------------------------------------------------------------------------

use rhp_values::ops::deref;

fn fn_print_r(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut out = Vec::new();
    let r = ext::standard::var::resolve_views(
        host,
        &args[0],
        zend::object_handlers::PropPurpose::Debug,
    );
    if r.is_ok() {
        ext::standard::var::print_r(host, &mut out, &args[0], 0);
    }
    ext::standard::var::clear_views();
    r?;
    let return_mode = args.get(1).map(conv::to_bool).unwrap_or(false);
    if return_mode {
        Ok(Value::str(out))
    } else {
        host.write_out(&out);
        Ok(Value::Bool(true))
    }
}

fn fn_constant(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = conv::to_string_bytes(&deref(&args[0]));
    resolve_const(host, &name)?.ok_or_else(|| {
        let shown = name.strip_prefix(b"\\").unwrap_or(&name);
        Thrown::error(
            "Error",
            format!("Undefined constant \"{}\"", String::from_utf8_lossy(shown)),
        )
    })
}

fn fn_str_repeat(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let n = conv::to_int(&args[1]);
    if n < 0 {
        return Err(Thrown::error(
            "ValueError",
            "str_repeat(): Argument #2 ($times) must be greater than or equal to 0",
        ));
    }
    let n = n as usize;
    // zend_string_safe_falloc(len, mult, 0): php refuses past memory_limit.
    if !s.is_empty()
        && n != 0
        && !rhp_values::mem::reserve_safe(s.len(), n, rhp_values::mem::zstr_size(0))
    {
        return Ok(Value::Null);
    }
    Ok(Value::str(s.repeat(n)))
}

fn fn_call_user_func(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    require_callback(host, "call_user_func", 1, &args[0])?;
    host.call_callable(&args[0], &args[1..])
}

fn fn_call_user_func_array(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    require_callback(host, "call_user_func_array", 1, &args[0])?;
    let params = as_array(&args[1], "call_user_func_array")?;
    let mut argv: Vec<Value> = Vec::new();
    let mut named: Vec<(rhp_values::PhpStr, Value)> = Vec::new();
    for (k, v) in params.iter() {
        match k {
            rhp_values::ArrayKey::Int(_) if !named.is_empty() => {
                return Err(Thrown::error(
                    "Error",
                    "Cannot use positional argument after named argument",
                ))
            }
            rhp_values::ArrayKey::Int(_) => argv.push(v.clone()),
            rhp_values::ArrayKey::Str(s) => named.push((s.clone(), v.clone())),
        }
    }
    if named.is_empty() {
        return host.call_callable(&args[0], &argv);
    }
    host.call_callable_named(&args[0], &argv, named)
}

fn fn_implode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // implode($array) or implode($separator, $array)
    let (glue, arr) = match (deref(&args[0]), args.get(1).map(deref)) {
        (Value::Array(a), None | Some(Value::Null)) => (Vec::new(), a),
        (Value::Array(_), Some(_)) => {
            return Err(Thrown::type_error(
                "implode(): Argument #1 ($separator) must be of type string, array given",
            ))
        }
        (g, Some(Value::Array(a))) => (conv::to_string_bytes(&g), a),
        _ => {
            return Err(Thrown::type_error(
                "implode(): If argument #1 ($separator) is of type string, argument #2 ($array) must be of type array, null given",
            ))
        }
    };
    let parts: Vec<std::borrow::Cow<[u8]>> = arr
        .iter()
        .map(|(_, v)| match v {
            Value::Str(s) => std::borrow::Cow::Borrowed(s.as_bytes()),
            v => std::borrow::Cow::Owned(conv::to_string_bytes(v)),
        })
        .collect();
    let len: usize = parts.iter().map(|p| p.len()).sum();
    // zend_string_safe_falloc(n - 1, glue_len, len): the glue repeated per
    // element can dwarf the input, so php checks the limit first.
    if parts.len() > 1
        && !rhp_values::mem::reserve_safe(
            parts.len() - 1,
            glue.len(),
            rhp_values::mem::zstr_size(len),
        )
    {
        return Ok(Value::Null);
    }
    let mut out = Vec::with_capacity(len + glue.len() * parts.len().saturating_sub(1));
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(&glue);
        }
        out.extend_from_slice(p);
    }
    Ok(Value::str(out))
}

fn fn_explode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let delim = conv::to_string_bytes(&deref(&args[0]));
    let s = conv::to_string_bytes(&deref(&args[1]));
    if delim.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            "explode(): Argument #1 ($separator) must not be empty",
        ));
    }
    // All pieces, in order.
    let mut parts: Vec<Vec<u8>> = Vec::new();
    let mut start = 0;
    while let Some(pos) = find_subslice(&s[start..], &delim) {
        parts.push(s[start..start + pos].to_vec());
        start += pos + delim.len();
    }
    parts.push(s[start..].to_vec());

    let limit = args.get(2).map(|v| conv::to_int(&deref(v)));
    let n = parts.len();
    let pieces: Vec<Vec<u8>> = match limit {
        None => parts,
        // Positive: at most `limit` pieces; the last keeps the remainder.
        Some(l) if l > 0 => {
            let l = l as usize;
            if l >= n {
                parts
            } else {
                let mut out: Vec<Vec<u8>> = parts[..l - 1].to_vec();
                out.push(parts[l - 1..].join(&delim[..]));
                out
            }
        }
        // Zero is treated as 1: the whole string as one piece.
        Some(0) => vec![s.clone()],
        // Negative: drop the last `|limit|` pieces.
        Some(l) => {
            let drop = (-l) as usize;
            if drop >= n {
                Vec::new()
            } else {
                parts[..n - drop].to_vec()
            }
        }
    };
    let mut arr = PhpArray::new();
    for p in pieces {
        arr.push(Value::str(p));
    }
    Ok(Value::Array(arr))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

fn fn_strtoupper(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut b = conv::to_string_bytes(&deref(&args[0]));
    b.make_ascii_uppercase();
    Ok(Value::str(b))
}

fn fn_strtolower(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut b = conv::to_string_bytes(&deref(&args[0]));
    b.make_ascii_lowercase();
    Ok(Value::str(b))
}

fn fn_str_contains(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let haystack = conv::to_string_bytes(&deref(&args[0]));
    let needle = conv::to_string_bytes(&deref(&args[1]));
    Ok(Value::Bool(
        needle.is_empty() || find_subslice(&haystack, &needle).is_some(),
    ))
}

fn fn_ord(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let b = conv::to_string_bytes(&deref(&args[0]));
    match b.len() {
        1 => {}
        0 => deprecated(host, "ord", "Providing an empty string is deprecated")?,
        _ => deprecated(
            host,
            "ord",
            "Providing a string that is not one byte long is deprecated. Use ord($str[0]) instead",
        )?,
    }
    Ok(Value::Int(b.first().copied().unwrap_or(0) as i64))
}

fn fn_chr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let c = conv::to_int(&deref(&args[0]));
    if !(0..=255).contains(&c) {
        deprecated(
            host,
            "chr",
            "Providing a value not in-between 0 and 255 is deprecated, this is because a byte \
             value must be in the [0, 255] interval. The value used will be constrained using % 256",
        )?;
    }
    Ok(Value::str(vec![(c & 0xff) as u8]))
}
