//! php's builtin functions, constants and prelude classes, and the narrow
//! [`Host`] interface the interpreter exposes to them.
//!
//! Builtins operate on owned `Value`s only — they never touch the arena/AST,
//! so this crate depends on `rhp-values` and `rhp-bytecode` (the stub
//! signatures), plus the C libraries it links, never on the compiler or the
//! VM (no dependency cycle).
//! The interpreter talks back through [`Host`].
//!
//! Files follow php-src (docs/design/builtins-layout.md): `zend/`, `main/`,
//! `ext/<name>/` and `sapi/` hold what php-src keeps in the directory of
//! that name, one Rust file per php-src file. `registry.rs` builds the
//! function and constant tables, `host.rs` is the [`Host`] trait and
//! `diag.rs` the diagnostics helpers; this file re-exports the crate's API.

// `main` is php-src's main/, not a binary's entry point.
#![allow(special_module_name)]

use rhp_values::ops::deref;

pub use rhp_values::Thrown;

mod diag;
mod host;
pub mod main;
pub(crate) mod module;
pub(crate) mod registry;
pub mod version;
pub mod zend;

pub use diag::{
    deprecated, deprecated_function, flush_stream_diags, locate_for_open, locate_url, notice,
    open_failed, warn, warn_arg,
};
pub use ext::pcre;
pub use ext::standard::http_fopen_wrapper;
pub use host::{
    host_compare, host_loose_eq, Host, MagicFetch, ObCall, ScopeVar, StreamKind, StreamMeta,
    UnserProp, WrapperRestore,
};
pub use main::streams::{StatInfo, StreamWrapper, WrapperReg};
pub use registry::{
    class_constant_deprecation, constant_deprecation, constants, ct_constant, functions,
    is_refused, lookup, lookup_lc, native_function, native_resource_dtor, object_freed, prelude,
    registry, reset_request_state, sapi_function_module, stream_wrappers, Builtin, NativeFn,
};
pub(crate) use registry::{register, register_helpers, Registry};

pub use ext::pcntl::{dispatch as pcntl_dispatch, interrupt_pending as pcntl_interrupt_pending};
/// pcntl's asynchronous signal delivery: the VM polls
/// [`pcntl_interrupt_pending`] between instructions and, when set, calls
/// [`pcntl_dispatch`] to run the queued PHP signal handlers.
pub use ext::standard::basic_functions::run_tick_functions;
pub use ext::zlib::output::request_startup as zlib_request_startup;

#[rustfmt::skip]
mod const_modules;
#[rustfmt::skip]
mod func_modules;
mod ext;
mod sapi;

#[cfg(test)]
mod test_support;

// php's error levels, for the builtins and the VM.
pub use rhp_values::{
    E_ALL, E_COMPILE_ERROR, E_COMPILE_WARNING, E_CORE_ERROR, E_CORE_WARNING, E_DEPRECATED, E_ERROR,
    E_NOTICE, E_PARSE, E_RECOVERABLE_ERROR, E_STRICT, E_USER_DEPRECATED, E_USER_ERROR,
    E_USER_NOTICE, E_USER_WARNING, E_WARNING,
};

pub use ext::standard::basic_functions::{apply_env_overrides, env_overrides};
pub use zend::zpp::{zval_type_name, zval_type_name_by, zval_value_name, zval_value_name_by};

/// The php version this engine reports (`PHP_VERSION`).
pub const PHP_VERSION: &str = version::PHP_VERSION;

/// A directive's current value: `None` = unknown directive, `Some(None)` =
/// registered with a null value.
pub fn ini_value(name: &[u8]) -> Option<Option<Vec<u8>>> {
    main::ini::ini_value(name)
}

/// C `atol` (php's ZEND_ATOL): leading blanks, sign, digits; saturating.
pub fn c_atol(s: &[u8]) -> i64 {
    main::ini::c_atol(s)
}

/// C `atoi`: [`c_atol`] cut to 32 bits.
pub fn c_atoi(s: &[u8]) -> i32 {
    main::ini::c_atoi(s)
}

/// `zend_ini_parse_bool`: "true", "yes", "on" (any case), else atoi != 0.
pub fn ini_parse_bool(v: &[u8]) -> bool {
    main::ini::ini_parse_bool(v)
}

/// `zend_ini_parse_quantity`: `8M` → 8388608.
pub fn ini_parse_quantity(s: &[u8]) -> i64 {
    main::ini::ini_parse_quantity(s)
}

/// A directive's startup value (`-d`, else the SAPI's or the built-in
/// default): what php's PHP_INI_SYSTEM directives keep. `None` = unknown.
pub fn ini_startup_value(name: &[u8]) -> Option<Vec<u8>> {
    main::ini::ini_default_value(name).map(Option::unwrap_or_default)
}

pub use ext::opcache::{
    accel_log_dest, accel_log_line, asctime_local, preload_user_check, take_accel_log,
};

/// A startup ini setting (`php -d name=value`): any registered directive,
/// applied to the engine state it drives (precision, ...). Unknown names are
/// ignored. Call after [`reset_request_state`].
pub fn ini_startup_set(name: &[u8], value: &[u8]) {
    main::ini::config_define(name, value);
    main::ini::ini_startup(name, value);
}

/// The short_open_tag ini: whether `<?` opens PHP code.
pub use ext::tokenizer::lexer::short_tags as short_open_tag;
pub use main::ini::{config_load_file, config_set_opened_path, config_set_scanned_files};

pub use main::ini::{apply_memory_limits, take_shutdown_diagnostics, take_startup_diagnostics};
pub use module::{output_handler_alias, OutputHandlerAlias, OutputHandlerCtx, OutputHandlerFn};
