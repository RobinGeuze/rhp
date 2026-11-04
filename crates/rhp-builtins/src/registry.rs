//! The builtin function table (php names to native functions, with php's
//! arity), the engine's constant table and PHP prelude, and the hooks the
//! engine calls into every module: request reset, object freed, constant
//! deprecation.

use std::collections::HashMap;
use std::sync::OnceLock;

use rhp_values::Value;

use crate::{
    ext, main, sapi, version, zend, Host, Thrown, WrapperReg, E_ALL, E_COMPILE_ERROR,
    E_COMPILE_WARNING, E_CORE_ERROR, E_CORE_WARNING, E_DEPRECATED, E_ERROR, E_NOTICE, E_PARSE,
    E_RECOVERABLE_ERROR, E_STRICT, E_USER_DEPRECATED, E_USER_ERROR, E_USER_NOTICE, E_USER_WARNING,
    E_WARNING,
};

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

/// The refcount-0 destructor of a native (non-stream) resource, given its
/// payload: a process closes its pipes and reaps the child if it already
/// exited (proc_open_rsrc_dtor); anything else is just dropped.
pub fn native_resource_dtor(host: &mut dyn Host, data: Box<dyn std::any::Any>) {
    ext::standard::proc_open::proc_dtor(host, data);
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
    main::streams::xp_socket::reset_state();
    ext::sockets::reset_state();
    ext::libxml::reset_state();
    ext::date::reset_state();
    main::ini::reset_state();
    sapi::cli::reset_state();
    ext::pcre::php_pcre::reset_state();
    ext::json::reset_state();
    #[cfg(feature = "mbstring-libmbfl")]
    ext::mbstring::reset_state();
    ext::standard::string::reset_state();
    ext::standard::basic_functions::reset_state();
    ext::random::reset_state();
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
        ("string", Box::new(ext::standard::string::register)),
        ("base64", Box::new(ext::standard::base64::register)),
        ("md5", Box::new(ext::standard::md5::register)),
        ("sha1", Box::new(ext::standard::sha1::register)),
        ("crc32", Box::new(ext::standard::crc32::register)),
        ("math", Box::new(ext::standard::math::register)),
        ("array", Box::new(ext::standard::array::register)),
        (
            "basic_functions",
            Box::new(ext::standard::basic_functions::register),
        ),
        ("file", Box::new(ext::standard::file::register)),
        ("filestat", Box::new(ext::standard::filestat::register)),
        ("dir", Box::new(ext::standard::dir::register)),
        ("link", Box::new(ext::standard::link::register)),
        ("type", Box::new(ext::standard::r#type::register)),
    ];
    // Discrete extensions (php-src ext/<name>/ layout): each a self-contained
    // module owning its functions/constants/classes/wrappers.
    for md in ext::modules() {
        v.push((md.name, Box::new(move |m| (md.functions)(m))));
    }
    let rest: Vec<(&'static str, Registrar)> = vec![
        ("url", Box::new(ext::standard::url::register)),
        ("head", Box::new(ext::standard::head::register)),
        ("html", Box::new(ext::standard::html::register)),
        ("var", Box::new(ext::standard::var::register)),
        ("filter", Box::new(ext::filter::register)),
        (
            "formatted_print",
            Box::new(ext::standard::formatted_print::register),
        ),
        ("pcre", Box::new(ext::pcre::php_pcre::register)),
        ("json", Box::new(ext::json::register)),
        ("scanf", Box::new(ext::standard::scanf::register)),
        ("soundex", Box::new(ext::standard::soundex::register)),
        (
            "levenshtein",
            Box::new(ext::standard::levenshtein::register),
        ),
        ("pack", Box::new(ext::standard::pack::register)),
        ("uniqid", Box::new(ext::standard::uniqid::register)),
        ("microtime", Box::new(ext::standard::microtime::register)),
        ("hrtime", Box::new(ext::standard::hrtime::register)),
        ("assert", Box::new(ext::standard::assert::register)),
        ("pageinfo", Box::new(ext::standard::pageinfo::register)),
        ("dns", Box::new(ext::standard::dns::register)),
        ("versioning", Box::new(ext::standard::versioning::register)),
        ("output", Box::new(main::output::register)),
        ("main", Box::new(main::register)),
        ("info", Box::new(ext::standard::info::register)),
        (
            "streamsfuncs",
            Box::new(ext::standard::streamsfuncs::register),
        ),
        ("fsock", Box::new(ext::standard::fsock::register)),
        ("http", Box::new(ext::standard::http::register)),
        (
            "builtin_functions",
            Box::new(zend::builtin_functions::register),
        ),
        ("exceptions", Box::new(zend::exceptions::register)),
        ("closures", Box::new(zend::closures::register)),
        ("zpp", Box::new(zend::zpp::register)),
        ("sapi", Box::new(main::sapi::register)),
        ("opcache", Box::new(ext::opcache::register)),
        ("cli", Box::new(sapi::cli::register)),
        ("cgi", Box::new(sapi::cgi::register)),
        ("random", Box::new(ext::random::register)),
        ("readline", Box::new(ext::readline::register)),
        ("crypt", Box::new(ext::standard::crypt::register)),
        ("proc_open", Box::new(ext::standard::proc_open::register)),
        ("exec", Box::new(ext::standard::exec::register)),
    ];
    v.extend(rest);
    v
}

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
        // SEEK_* come from ext::standard::file::constants().
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
    .chain(ext::pcre::php_pcre::preg_constants())
    .chain(ext::json::json_constants())
    .chain(ext::standard::info::constants())
    .chain(ext::standard::file::constants())
    .chain(ext::standard::file::fnmatch_constants())
    .chain(ext::standard::array::case_constants())
    .chain(ext::random::mt_rand_constants())
    .chain(ext::standard::crypt::constants())
    .chain(ext::standard::streamsfuncs::constants())
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
    s.push_str(ext::standard::dir::PRELUDE_PHP);
    s.push('\n');
    s.push_str(ext::standard::string::PRELUDE_PHP);
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

#[cfg(test)]
mod tests;
