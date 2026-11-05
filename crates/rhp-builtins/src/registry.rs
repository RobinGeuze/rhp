//! The builtin function table (php names to native functions, with php's
//! arity), the engine's constant table and PHP prelude, and the hooks the
//! engine calls into every module: request reset, object freed, constant
//! deprecation.

use std::collections::HashMap;
use std::sync::OnceLock;

use rhp_values::Value;

use crate::{ext, main, sapi, zend, Host, Thrown, WrapperReg};

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

/// Engine constants (`PHP_EOL`, `PHP_INT_MAX`, …), each from the module
/// that owns it.
pub fn constants() -> Vec<(&'static str, Value)> {
    let own: Vec<Vec<(&'static str, Value)>> = vec![
        zend::constants::constants(),
        main::constants(),
        main::output::constants(),
        main::rfc1867::constants(),
        ext::standard::basic_functions::constants(),
        ext::standard::array::constants(),
        ext::standard::string::constants(),
        ext::standard::math::constants(),
        ext::standard::file::constants(),
        ext::standard::file::fnmatch_constants(),
        ext::standard::dir::constants(),
        ext::standard::url::constants(),
        ext::standard::http::constants(),
        ext::standard::html::constants(),
        ext::standard::info::constants(),
        ext::standard::crypt::constants(),
        ext::standard::streamsfuncs::constants(),
        ext::pcre::php_pcre::preg_constants(),
        ext::json::json_constants(),
        ext::random::mt_rand_constants(),
        ext::filter::constants(),
        #[cfg(feature = "mbstring-libmbfl")]
        ext::mbstring::constants(),
    ];
    own.into_iter()
        .flatten()
        // Per-extension constants (GMP_*, PASSWORD_*, ZLIB_*, …) live with
        // their module; pcntl's only under the SAPIs that have it.
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
