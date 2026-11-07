//! intl's error state (intl_error.c): the global error behind
//! `intl_get_error_code()`/`intl_get_error_message()`, each object's own
//! (`getErrorCode()`/`getErrorMessage()`), and what `intl.error_level`
//! and `intl.use_exceptions` add to a global error's message.

use std::cell::{Cell, RefCell};
use std::ffi::CStr;

use super::sys::{self, UErrorCode};
use crate::{Host, Thrown};

/// `intl_error`: a code and an optional message, already prefixed with
/// the function that set it.
#[derive(Default)]
pub(crate) struct IntlError {
    pub(crate) code: UErrorCode,
    pub(crate) msg: Option<String>,
}

impl IntlError {
    /// `intl_error_get_message`.
    pub(crate) fn message(&self) -> String {
        let name = u_error_name(self.code);
        match &self.msg {
            Some(m) => format!("{m}: {name}"),
            None => name,
        }
    }

    /// `intl_error_reset(err)`.
    pub(crate) fn reset(&mut self) {
        self.code = sys::U_ZERO_ERROR;
        self.msg = None;
    }
}

thread_local! {
    static ERR: RefCell<IntlError> = const { RefCell::new(IntlError { code: 0, msg: None }) };
    /// Set while a constructor runs: php's `__construct` methods force
    /// `use_exceptions` on and `error_level` off.
    static CTOR: Cell<bool> = const { Cell::new(false) };
}

/// ICU's `u_errorName`.
pub(crate) fn u_error_name(code: UErrorCode) -> String {
    // SAFETY: u_errorName returns a static string for every code.
    unsafe { CStr::from_ptr(sys::u_errorName(code)) }
        .to_string_lossy()
        .into_owned()
}

/// `intl_error_get_code(NULL)`.
pub(crate) fn error_code() -> UErrorCode {
    ERR.with(|e| e.borrow().code)
}

/// `intl_error_get_message(NULL)`.
pub(crate) fn error_message() -> String {
    ERR.with(|e| e.borrow().message())
}

/// `intl_error_reset(NULL)`.
pub(crate) fn error_reset() {
    ERR.with(|e| e.borrow_mut().reset());
}

/// `intl_error_set_code(NULL, code)`.
pub(crate) fn error_set_code(code: UErrorCode) {
    ERR.with(|e| e.borrow_mut().code = code);
}

/// The global message alone, without php's `intl.error_level` and
/// `intl.use_exceptions` reporting (two grapheme helpers that have no
/// host).
pub(crate) fn error_set_msg(fname: &str, msg: &str) {
    ERR.with(|e| e.borrow_mut().msg = Some(format!("{fname}(): {msg}")));
}

/// `intl_error_set(NULL, code, msg)` without the reporting.
pub(crate) fn error_set(fname: &str, code: UErrorCode, msg: &str) {
    error_set_code(code);
    error_set_msg(fname, msg);
}

/// Runs a constructor body with php's forced settings.
pub(crate) fn in_ctor<T>(f: impl FnOnce() -> T) -> T {
    let old = CTOR.with(|c| c.replace(true));
    let r = f();
    CTOR.with(|c| c.set(old));
    r
}

fn error_level() -> i64 {
    if CTOR.with(Cell::get) {
        return 0;
    }
    match crate::ini_value(b"intl.error_level") {
        Some(Some(v)) => crate::ini_parse_quantity(&v),
        _ => 0,
    }
}

fn use_exceptions() -> bool {
    CTOR.with(Cell::get)
        || matches!(crate::ini_value(b"intl.use_exceptions"), Some(Some(v)) if crate::ini_parse_bool(&v))
}

/// `intl_error_set_custom_msg(NULL, msg)`: the global message, raised as
/// php's `intl.error_level` diagnostic and thrown as an IntlException when
/// `intl.use_exceptions` is on. The exception is held for when the
/// builtin returns, as php's is; a user error handler's is returned.
pub(crate) fn set_custom_msg(host: &mut dyn Host, fname: &str, msg: &str) -> Result<(), Thrown> {
    let prefixed = format!("{fname}(): {msg}");
    let level = error_level();
    if level != 0 {
        host.emit_diagnostic(level, prefixed.as_bytes())?;
    }
    if use_exceptions() {
        host.set_pending_throw(Thrown::error("IntlException", prefixed.clone()));
    }
    ERR.with(|e| e.borrow_mut().msg = Some(prefixed));
    Ok(())
}

/// `intl_error_set(NULL, code, msg)`.
pub(crate) fn set(
    host: &mut dyn Host,
    fname: &str,
    code: UErrorCode,
    msg: &str,
) -> Result<(), Thrown> {
    error_set_code(code);
    set_custom_msg(host, fname, msg)
}

/// `intl_errors_set_custom_msg(err, msg)`: the object's message, then the
/// global one.
pub(crate) fn errors_set_custom_msg(
    host: &mut dyn Host,
    err: &RefCell<IntlError>,
    fname: &str,
    msg: &str,
) -> Result<(), Thrown> {
    err.borrow_mut().msg = Some(format!("{fname}(): {msg}"));
    set_custom_msg(host, fname, msg)
}

/// `intl_errors_set_code(err, code)`.
pub(crate) fn errors_set_code(err: &RefCell<IntlError>, code: UErrorCode) {
    err.borrow_mut().code = code;
    error_set_code(code);
}

/// `INTL_METHOD_CHECK_STATUS(obj, msg)`: copies the object's code to the
/// global error; on a failure sets both messages and answers true (the
/// caller returns false).
pub(crate) fn check_status(
    host: &mut dyn Host,
    err: &RefCell<IntlError>,
    fname: &str,
    msg: &str,
) -> Result<bool, Thrown> {
    let code = err.borrow().code;
    error_set_code(code);
    if sys::failure(code) {
        errors_set_custom_msg(host, err, fname, msg)?;
        return Ok(true);
    }
    Ok(false)
}

/// `INTL_CHECK_STATUS(status, msg)` (global only): true on a failure.
pub(crate) fn check_global(
    host: &mut dyn Host,
    status: UErrorCode,
    fname: &str,
    msg: &str,
) -> Result<bool, Thrown> {
    error_set_code(status);
    if sys::failure(status) {
        set_custom_msg(host, fname, msg)?;
        return Ok(true);
    }
    Ok(false)
}

/// The deprecation `OnUpdateErrorLevel` raises for a non-zero
/// `intl.error_level`.
pub(crate) const ERROR_LEVEL_DEPRECATION: &str = "Using a value different than 0 for intl.error_level is deprecated, \
     as the intl.error_level INI setting is deprecated. Instead the intl.use_exceptions INI setting \
     should be enabled to throw exceptions on errors or intl_get_error_code()/intl_get_error_message() \
     should be used to manually deal with errors";
