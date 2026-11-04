//! The `intl` extension — the subset Composer and its bundled
//! `symfony/string` reach for: grapheme-cluster string functions
//! (`grapheme_*`), Unicode normalization (`normalizer_*` / `Normalizer`) and
//! IDNA (`idn_to_ascii` / `idn_to_utf8`), plus the global error state
//! (`intl_get_error_code` / `intl_get_error_message`).
//!
//! Like php, rhp calls ICU4C (the build php links, see
//! docs/design/libraries.md); each function is a port of php-src's
//! ext/intl C. The global constants are php's (`consts.h`, generated from
//! the stubs) with the linked ICU's values.
//!
//! `extension_loaded('intl')` stays false: the rest of the extension
//! (Collator, NumberFormatter, Locale, …) is not implemented, and callers that
//! gate on the extension would otherwise assume it.

use std::cell::RefCell;
use std::ffi::CStr;
use std::ptr;
use std::sync::OnceLock;

use rhp_values::Value;

use crate::{Host, Thrown};

mod grapheme;
mod idn;
mod normalizer;
mod sys;

use sys::{UChar, UErrorCode};

// ---------------------------------------------------------------------------
// constants (consts.h through shim.c)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum ConstValue {
    Int(i64),
    Str(&'static str),
}

/// The shim's table, read once.
fn const_table() -> &'static [(&'static str, ConstValue)] {
    static TABLE: OnceLock<Vec<(&'static str, ConstValue)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut out = Vec::new();
        // SAFETY: the shim returns a static array ended by a null name; its
        // strings are C literals.
        unsafe {
            let mut p = sys::rhp_intl_constants();
            while !(*p).name.is_null() {
                let name = CStr::from_ptr((*p).name).to_str().expect("ASCII name");
                let value = if (*p).str.is_null() {
                    ConstValue::Int((*p).value)
                } else {
                    ConstValue::Str(CStr::from_ptr((*p).str).to_str().expect("ASCII value"))
                };
                out.push((name, value));
                p = p.add(1);
            }
        }
        out
    })
}

/// The value of one of php's integer intl constants.
pub(crate) fn int_const(name: &str) -> i64 {
    match const_table().iter().find(|(n, _)| *n == name) {
        Some((_, ConstValue::Int(v))) => *v,
        _ => panic!("consts.h has no integer {name}"),
    }
}

pub fn constants() -> Vec<(&'static str, Value)> {
    const_table()
        .iter()
        .map(|&(n, v)| {
            let v = match v {
                ConstValue::Int(i) => Value::Int(i),
                ConstValue::Str(s) => Value::str(s),
            };
            (n, v)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// global error state (intl_error.c)
// ---------------------------------------------------------------------------

/// ICU's `u_errorName`.
fn u_error_name(code: UErrorCode) -> String {
    // SAFETY: u_errorName returns a static string for every code.
    unsafe { CStr::from_ptr(sys::u_errorName(code)) }
        .to_string_lossy()
        .into_owned()
}

struct IntlError {
    code: UErrorCode,
    msg: Option<String>,
}

thread_local! {
    static ERR: RefCell<IntlError> = const { RefCell::new(IntlError { code: 0, msg: None }) };
}

/// `intl_error_reset(NULL)`.
pub(crate) fn error_reset() {
    ERR.with(|e| {
        let mut e = e.borrow_mut();
        e.code = sys::U_ZERO_ERROR;
        e.msg = None;
    });
}

/// `intl_error_set_code(NULL, code)`.
pub(crate) fn error_set_code(code: UErrorCode) {
    ERR.with(|e| e.borrow_mut().code = code);
}

/// `intl_error_set_custom_msg(NULL, msg)`: the message is prefixed with the
/// calling function's name, as php does for the global error.
pub(crate) fn error_set_msg(fname: &str, msg: &str) {
    ERR.with(|e| e.borrow_mut().msg = Some(format!("{fname}(): {msg}")));
}

/// `intl_error_set(NULL, code, msg)`.
pub(crate) fn error_set(fname: &str, code: UErrorCode, msg: &str) {
    error_set_code(code);
    error_set_msg(fname, msg);
}

fn fn_intl_get_error_code(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(ERR.with(|e| e.borrow().code) as i64))
}

fn fn_intl_get_error_message(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let s = ERR.with(|e| {
        let e = e.borrow();
        let name = u_error_name(e.code);
        match &e.msg {
            Some(m) => format!("{m}: {name}"),
            None => name,
        }
    });
    Ok(Value::str(s.into_bytes()))
}

fn fn_intl_is_failure(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = super::zpp::int_arg(host, args, 0, "intl_is_failure", "errorCode")?;
    Ok(Value::Bool(code > sys::U_ZERO_ERROR as i64))
}

fn fn_intl_error_name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = super::zpp::int_arg(host, args, 0, "intl_error_name", "errorCode")?;
    // php passes the zend_long to u_errorName's UErrorCode (an int).
    Ok(Value::str(u_error_name(code as i32)))
}

// ---------------------------------------------------------------------------
// shared helpers (intl_convert.c)
// ---------------------------------------------------------------------------

/// `intl_convert_utf8_to_utf16`: ICU's strict UTF-8 → UTF-16.
pub(crate) fn utf8_to_utf16(src: &[u8]) -> Result<Vec<UChar>, UErrorCode> {
    if src.len() > i32::MAX as usize {
        return Err(sys::U_BUFFER_OVERFLOW_ERROR);
    }
    let mut status = sys::U_ZERO_ERROR;
    let mut len = 0i32;
    // SAFETY: a preflight (null buffer, zero capacity) over `src`.
    unsafe {
        sys::u_strFromUTF8(
            ptr::null_mut(),
            0,
            &mut len,
            src.as_ptr().cast(),
            src.len() as i32,
            &mut status,
        );
    }
    if status != sys::U_BUFFER_OVERFLOW_ERROR && status != sys::U_STRING_NOT_TERMINATED_WARNING {
        return Err(status);
    }
    let mut buf: Vec<UChar> = vec![0; len as usize + 1];
    status = sys::U_ZERO_ERROR;
    // SAFETY: `buf` holds `len + 1` units.
    unsafe {
        sys::u_strFromUTF8(
            buf.as_mut_ptr(),
            len + 1,
            ptr::null_mut(),
            src.as_ptr().cast(),
            src.len() as i32,
            &mut status,
        );
    }
    if sys::failure(status) {
        return Err(status);
    }
    buf.truncate(len as usize);
    Ok(buf)
}

/// `intl_convert_utf16_to_utf8`.
pub(crate) fn utf16_to_utf8(src: &[UChar]) -> Result<Vec<u8>, UErrorCode> {
    let mut status = sys::U_ZERO_ERROR;
    let mut len = 0i32;
    // SAFETY: a preflight over `src`.
    unsafe {
        sys::u_strToUTF8(
            ptr::null_mut(),
            0,
            &mut len,
            src.as_ptr(),
            src.len() as i32,
            &mut status,
        );
    }
    if status != sys::U_BUFFER_OVERFLOW_ERROR && status != sys::U_STRING_NOT_TERMINATED_WARNING {
        return Err(status);
    }
    let mut buf: Vec<u8> = vec![0; len as usize];
    status = sys::U_ZERO_ERROR;
    // SAFETY: `buf` holds `len` bytes.
    unsafe {
        sys::u_strToUTF8(
            buf.as_mut_ptr().cast(),
            len,
            ptr::null_mut(),
            src.as_ptr(),
            src.len() as i32,
            &mut status,
        );
    }
    if sys::failure(status) {
        return Err(status);
    }
    Ok(buf)
}

/// php's `ZEND_HAS_OOM()`: an allocation crossed memory_limit, and the VM
/// raises the error once the builtin returns.
pub(crate) fn has_oom() -> bool {
    rhp_values::mem::state().attention()
}

/// `Z_PARAM_PATH`: a string that must not contain NUL bytes.
pub(crate) fn path_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Vec<u8>, Thrown> {
    if args.len() <= i {
        return Ok(Vec::new());
    }
    let s = super::zpp::str_arg(host, args, i, fname, pname)?;
    if s.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{fname}(): Argument #{} (${pname}) must not contain any null bytes",
                i + 1
            ),
        ));
    }
    Ok(s)
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("intl_get_error_code", fn_intl_get_error_code),
            ("intl_get_error_message", fn_intl_get_error_message),
            ("intl_is_failure", fn_intl_is_failure),
            ("intl_error_name", fn_intl_error_name),
            ("grapheme_strlen", grapheme::fn_strlen),
            ("grapheme_strpos", grapheme::fn_strpos),
            ("grapheme_stripos", grapheme::fn_stripos),
            ("grapheme_strrpos", grapheme::fn_strrpos),
            ("grapheme_strripos", grapheme::fn_strripos),
            ("grapheme_substr", grapheme::fn_substr),
            ("grapheme_strstr", grapheme::fn_strstr),
            ("grapheme_stristr", grapheme::fn_stristr),
            ("grapheme_str_split", grapheme::fn_str_split),
            ("grapheme_extract", grapheme::fn_extract),
            ("grapheme_levenshtein", grapheme::fn_levenshtein),
            ("normalizer_normalize", normalizer::fn_normalize),
            ("normalizer_is_normalized", normalizer::fn_is_normalized),
            (
                "normalizer_get_raw_decomposition",
                normalizer::fn_get_raw_decomposition,
            ),
            ("idn_to_ascii", idn::fn_idn_to_ascii),
            ("idn_to_utf8", idn::fn_idn_to_utf8),
        ],
    );
    crate::register_helpers(m, &[("__normalizer", normalizer::fn_method, 1, None)]);
}

/// The `Normalizer` class: its static methods share the procedural
/// functions' code but name themselves in errors.
pub const PRELUDE_PHP: &str = include_str!("prelude.php");

#[cfg(test)]
mod tests;
