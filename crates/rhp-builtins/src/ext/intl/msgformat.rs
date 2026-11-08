//! `MessageFormatter` and `msgfmt_*`, ported from php-src
//! ext/intl/msgformat/ onto ICU's `umsg` and, for the argument types,
//! named arguments and parsing, MessageFormat itself (icu.cpp).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;

use rhp_values::{ArrayKey, ObjectHandle, PhpArray, Value};

use super::common::{self, Call};
use super::cpp;
use super::error::{self, IntlError};
use super::sys::{self, UChar};
use crate::zend::object_handlers::{self, CloneObj, ObjectHandlers};
use crate::zend::zpp::deref;
use crate::{Host, Thrown};

pub(super) static HANDLERS: ObjectHandlers = ObjectHandlers {
    clone_obj: CloneObj::Custom(clone_obj),
    ..ObjectHandlers::STD
};

pub(super) const SPEC: common::ClassSpec = common::ClassSpec {
    name: "MessageFormatter",
    helper: "__msgfmt",
    props: "",
};

pub(super) const ALIASES: &[(&str, &str)] = &[
    ("msgfmt_create", "create"),
    ("msgfmt_format", "format"),
    ("msgfmt_format_message", "formatMessage"),
    ("msgfmt_parse", "parse"),
    ("msgfmt_parse_message", "parseMessage"),
    ("msgfmt_set_pattern", "setPattern"),
    ("msgfmt_get_pattern", "getPattern"),
    ("msgfmt_get_locale", "getLocale"),
    ("msgfmt_get_error_code", "getErrorCode"),
    ("msgfmt_get_error_message", "getErrorMessage"),
];

/// An argument's key in the pattern's type table.
#[derive(PartialEq, Eq, Hash, Clone)]
enum Key {
    Num(u64),
    Name(Vec<UChar>),
}

/// `MessageFormatter_object` (`msgformat_data`).
#[derive(Default)]
pub(super) struct MsgObj {
    err: RefCell<IntlError>,
    umsgf: Cell<usize>,
    orig_format: RefCell<Option<Vec<u8>>>,
    arg_types: RefCell<Option<HashMap<Key, i32>>>,
    tz_set: Cell<bool>,
}

impl MsgObj {
    fn fmt(&self) -> *mut c_void {
        self.umsgf.get() as *mut c_void
    }
}

impl Drop for MsgObj {
    fn drop(&mut self) {
        if !self.fmt().is_null() {
            // SAFETY: the formatter this object owns.
            unsafe { sys::umsg_close(self.fmt()) };
        }
    }
}

fn clone_obj(_host: &mut dyn Host, src: &ObjectHandle, dst: &ObjectHandle) -> Result<(), Thrown> {
    let from = common::state::<MsgObj>(src);
    let to = Rc::new(MsgObj::default());
    object_handlers::set_payload(dst, to.clone());
    *to.orig_format.borrow_mut() = from.orig_format.borrow().clone();
    to.tz_set.set(from.tz_set.get());
    if from.fmt().is_null() {
        return Err(Thrown::error(
            "Error",
            "Cannot clone uninitialized MessageFormatter",
        ));
    }
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a live formatter.
    let c = unsafe { sys::umsg_clone(from.fmt(), &mut status) };
    to.umsgf.set(c as usize);
    if sys::failure(status) {
        return Err(Thrown::error("Error", "Failed to clone MessageFormatter"));
    }
    Ok(())
}

/// `umsg_open` for `pattern` in `locale`: the formatter, the status and
/// the parse error.
fn open(locale: &[u8], pattern: &[UChar]) -> (*mut c_void, sys::UErrorCode, sys::UParseError) {
    let loc = super::c_string(locale);
    let mut status = sys::U_ZERO_ERROR;
    let mut pe = sys::UParseError::default();
    // SAFETY: the pattern's units, a C locale.
    let f = unsafe {
        sys::umsg_open(
            if pattern.is_empty() {
                std::ptr::null()
            } else {
                pattern.as_ptr()
            },
            pattern.len() as i32,
            loc.as_ptr(),
            &mut pe,
            &mut status,
        )
    };
    (f, status, pe)
}

/// `msgfmt_ctor`: false after a failure.
fn ctor(
    host: &mut dyn Host,
    f: &str,
    mfo: &MsgObj,
    locale: &[u8],
    pattern: &[u8],
) -> Result<bool, Thrown> {
    error::error_reset();
    if super::locale_too_long(host, f, locale.len())? {
        return Ok(false);
    }
    mfo.err.borrow_mut().reset();
    let mut spattern = Vec::new();
    if !pattern.is_empty() {
        match super::utf8_to_utf16(pattern) {
            Ok(u) => spattern = u,
            Err(st) => {
                mfo.err.borrow_mut().code = st;
                error::check_status(host, &mfo.err, f, "error converting pattern to UTF-16")?;
                return Ok(false);
            }
        }
    }
    let locale = if locale.is_empty() {
        super::locale_get_default()
    } else {
        locale.to_vec()
    };
    *mfo.arg_types.borrow_mut() = None;
    *mfo.orig_format.borrow_mut() = Some(pattern.to_vec());
    let (fmt, status, pe) = open(&locale, &spattern);
    if !mfo.fmt().is_null() {
        // SAFETY: the formatter this object owned.
        unsafe { sys::umsg_close(mfo.fmt()) };
    }
    mfo.umsgf.set(fmt as usize);
    mfo.err.borrow_mut().code = status;
    if status == sys::U_PATTERN_SYNTAX_ERROR {
        let msg = format!(
            "pattern syntax error ({})",
            error::parse_error_to_string(&pe)
        );
        error::error_set_code(status);
        error::errors_set_custom_msg(host, &mfo.err, f, &msg)?;
        return Ok(false);
    }
    if error::check_status(host, &mfo.err, f, "message formatter creation failed")? {
        return Ok(false);
    }
    Ok(true)
}

/// `umsg_get_types`.
fn types(host: &mut dyn Host, mfo: &MsgObj, f: &str) -> Result<Option<HashMap<Key, i32>>, Thrown> {
    if sys::failure(mfo.err.borrow().code) {
        return Ok(None);
    }
    if let Some(t) = mfo.arg_types.borrow().as_ref() {
        return Ok(Some(t.clone()));
    }
    let mut out: *mut cpp::MsgType = std::ptr::null_mut();
    let mut code = 0;
    let mut msg: *const std::ffi::c_char = std::ptr::null();
    // SAFETY: a live formatter; the shim fills `out`.
    let n = unsafe { cpp::rhp_msgfmt_arg_types(mfo.fmt(), &mut out, &mut code, &mut msg) };
    let mut map = HashMap::new();
    let count = if n < 0 { 0 } else { n };
    for i in 0..count as usize {
        // SAFETY: `n` entries at `out`.
        let t = unsafe { &*out.add(i) };
        let key = if t.name.is_null() {
            Key::Num(t.number as u64)
        } else {
            // SAFETY: `name_len` units.
            Key::Name(unsafe { std::slice::from_raw_parts(t.name, t.name_len as usize) }.to_vec())
        };
        map.insert(key, t.ty);
    }
    // SAFETY: the shim's array, its names included (all `n` entries, or
    // none was kept on failure... the shim copies them all).
    unsafe { cpp::rhp_msgfmt_free_types(out, count) };
    if n < 0 {
        let m = String::from_utf8_lossy(&super::c_bytes(msg)).into_owned();
        error::errors_set(host, &mfo.err, f, code, &m)?;
        return Ok(None);
    }
    *mfo.arg_types.borrow_mut() = Some(map.clone());
    Ok(Some(map))
}

/// `umsg_set_timezone`.
fn set_timezone(host: &mut dyn Host, mfo: &MsgObj, f: &str) -> Result<(), Thrown> {
    if mfo.tz_set.get() {
        return Ok(());
    }
    // SAFETY: a live formatter.
    let has = unsafe { cpp::rhp_msgfmt_has_date_formats(mfo.fmt()) };
    if has < 0 {
        error::errors_set(
            host,
            &mfo.err,
            f,
            sys::U_MEMORY_ALLOCATION_ERROR,
            "Out of memory retrieving subformats",
        )?;
    }
    if has > 0 && !sys::failure(mfo.err.borrow().code) {
        if let Some(z) = super::timezone::process_argument(host, None, Some(&mfo.err), f)? {
            // SAFETY: a live formatter and zone (cloned into each subformat).
            unsafe { cpp::rhp_msgfmt_set_timezone(mfo.fmt(), z.as_ptr()) };
        }
    }
    if !sys::failure(mfo.err.borrow().code) {
        mfo.tz_set.set(true);
    }
    Ok(())
}

/// One converted argument.
struct Arg {
    name: Vec<UChar>,
    ty: i32,
    d: f64,
    i: i64,
    s: Vec<UChar>,
}

/// `"...'%s'"` of php's message: the bytes up to a NUL.
fn quoted(what: &[u8], s: &[u8]) -> Vec<u8> {
    [what, b": '", super::c_string(s).as_bytes(), b"'"].concat()
}

/// `umsg_format_helper`: the UTF-16 result, or `None` with the error set.
fn format_helper(
    host: &mut dyn Host,
    mfo: &MsgObj,
    f: &str,
    args: &PhpArray,
) -> Result<Option<Vec<UChar>>, Thrown> {
    if sys::failure(mfo.err.borrow().code) {
        return Ok(None);
    }
    let types = types(host, mfo, f)?;
    set_timezone(host, mfo, f)?;
    let mut fargs: Vec<Arg> = Vec::new();
    let failed = |mfo: &MsgObj| sys::failure(mfo.err.borrow().code);
    for (k, elem) in args.iter() {
        if failed(mfo) {
            break;
        }
        let elem = deref(elem);
        let (key, stored) = match k {
            ArrayKey::Int(n) => {
                if n < 0 || n > i32::MAX as i64 {
                    error::errors_set(
                        host,
                        &mfo.err,
                        f,
                        sys::U_ILLEGAL_ARGUMENT_ERROR,
                        "Found negative or too large array key",
                    )?;
                    continue;
                }
                let key: Vec<UChar> = n.to_string().encode_utf16().collect();
                let st = types
                    .as_ref()
                    .and_then(|t| t.get(&Key::Num(n as u64)).copied());
                (key, st)
            }
            ArrayKey::Str(s) => match super::utf8_to_utf16(s.as_bytes()) {
                Ok(u) => {
                    let st = types
                        .as_ref()
                        .and_then(|t| t.get(&Key::Name(u.clone())).copied());
                    (u, st)
                }
                Err(st) => {
                    let msg = quoted(b"Invalid UTF-8 data in argument key", s.as_bytes());
                    error::errors_set(host, &mfo.err, f, st, &msg)?;
                    continue;
                }
            },
        };
        let ty = stored.unwrap_or(cpp::K_OBJECT);
        let mut a = Arg {
            name: key,
            ty,
            d: 0.0,
            i: 0,
            s: Vec::new(),
        };
        let as_string = |host: &mut dyn Host, a: &mut Arg| -> Result<bool, Thrown> {
            let s = crate::zend::zpp::zval_get_string(host, &elem)?;
            match super::utf8_to_utf16(&s) {
                Ok(u) => {
                    a.ty = cpp::K_STRING;
                    a.s = u;
                    Ok(true)
                }
                Err(st) => {
                    let msg = quoted(b"Invalid UTF-8 data in string argument", &s);
                    error::errors_set(host, &mfo.err, f, st, &msg)?;
                    Ok(false)
                }
            }
        };
        if ty != cpp::K_OBJECT {
            match ty {
                cpp::K_STRING => {
                    if !as_string(host, &mut a)? {
                        continue;
                    }
                }
                cpp::K_DOUBLE => a.d = rhp_values::conv::to_float(&elem),
                cpp::K_LONG => {
                    let mut v = 0i32;
                    match &elem {
                        Value::Float(d) => {
                            if *d > i32::MAX as f64 || *d < i32::MIN as f64 {
                                error::errors_set(host, &mfo.err, f, sys::U_ILLEGAL_ARGUMENT_ERROR,
                                    "Found PHP float with absolute value too large for 32 bit integer argument")?;
                            } else {
                                v = *d as i32;
                            }
                        }
                        Value::Int(n) => {
                            if *n > i32::MAX as i64 || *n < i32::MIN as i64 {
                                error::errors_set(host, &mfo.err, f, sys::U_ILLEGAL_ARGUMENT_ERROR,
                                    "Found PHP integer with absolute value too large for 32 bit integer argument")?;
                            } else {
                                v = *n as i32;
                            }
                        }
                        other => v = crate::zend::zpp::zval_get_long(host, other)? as i32,
                    }
                    a.i = v as i64;
                }
                cpp::K_INT64 => {
                    a.i = match &elem {
                        Value::Float(d) => {
                            if *d > i64::MAX as f64 || *d < i64::MIN as f64 {
                                error::errors_set(host, &mfo.err, f, sys::U_ILLEGAL_ARGUMENT_ERROR,
                                    "Found PHP float with absolute value too large for 64 bit integer argument")?;
                                0
                            } else {
                                *d as i64
                            }
                        }
                        Value::Int(n) => *n,
                        other => crate::zend::zpp::zval_get_long(host, other)?,
                    };
                }
                cpp::K_DATE => {
                    let dd = super::dates::zval_to_millis(host, &elem, Some(&mfo.err), f)?;
                    if failed(mfo) {
                        if let Ok(k) = super::utf16_to_utf8(&a.name) {
                            let msg = [
                                b"The argument for key '".as_slice(),
                                super::c_string(&k).as_bytes(),
                                b"' cannot be used as a date or time",
                            ]
                            .concat();
                            let code = mfo.err.borrow().code;
                            error::errors_set(host, &mfo.err, f, code, &msg)?;
                        }
                        continue;
                    }
                    a.d = dd;
                }
                _ => {
                    error::errors_set(
                        host,
                        &mfo.err,
                        f,
                        sys::U_ILLEGAL_ARGUMENT_ERROR,
                        "Found unsupported argument type",
                    )?;
                }
            }
        } else {
            match &elem {
                Value::Float(d) => {
                    a.ty = cpp::K_DOUBLE;
                    a.d = *d;
                }
                Value::Int(n) => {
                    a.ty = cpp::K_INT64;
                    a.i = *n;
                }
                Value::Null | Value::Bool(false) => {
                    a.ty = cpp::K_INT64;
                    a.i = 0;
                }
                Value::Bool(true) => {
                    a.ty = cpp::K_INT64;
                    a.i = 1;
                }
                Value::Str(_) | Value::Object(_) | Value::Closure(_) => {
                    if !as_string(host, &mut a)? {
                        continue;
                    }
                }
                _ => {
                    if let Ok(k) = super::utf16_to_utf8(&a.name) {
                        let msg = [
                            b"No strategy to convert the value given for the argument with key '"
                                .as_slice(),
                            super::c_string(&k).as_bytes(),
                            b"' is available",
                        ]
                        .concat();
                        error::errors_set(host, &mfo.err, f, sys::U_ILLEGAL_ARGUMENT_ERROR, &msg)?;
                    }
                }
            }
        }
        fargs.push(a);
    }
    if failed(mfo) {
        return Ok(None);
    }
    let raw: Vec<cpp::MsgArg> = fargs
        .iter()
        .map(|a| cpp::MsgArg {
            name: a.name.as_ptr(),
            name_len: a.name.len() as i32,
            ty: a.ty,
            d: a.d,
            i: a.i,
            s: a.s.as_ptr(),
            s_len: a.s.len() as i32,
        })
        .collect();
    let (mut len, mut status) = (0, sys::U_ZERO_ERROR);
    // SAFETY: a live formatter; `raw` borrows `fargs`.
    let p = unsafe {
        cpp::rhp_msgfmt_format(
            mfo.fmt(),
            raw.as_ptr(),
            raw.len() as i32,
            &mut len,
            &mut status,
        )
    };
    if sys::failure(status) {
        error::errors_set(
            host,
            &mfo.err,
            f,
            status,
            "Call to ICU MessageFormat::format() has failed",
        )?;
        return Ok(None);
    }
    Ok(cpp::take_ustr(p, len))
}

/// `msgfmt_do_format`.
fn do_format(host: &mut dyn Host, mfo: &MsgObj, f: &str, args: &PhpArray) -> Result<Value, Thrown> {
    let Some(u) = format_helper(host, mfo, f, args)? else {
        return Ok(Value::Bool(false));
    };
    match super::utf16_to_utf8(&u) {
        Ok(s) => Ok(Value::str(s)),
        Err(st) => {
            mfo.err.borrow_mut().code = st;
            error::check_status(host, &mfo.err, f, "Error converting value to UTF-8")?;
            Ok(Value::Bool(false))
        }
    }
}

/// `msgfmt_do_parse`.
fn do_parse(host: &mut dyn Host, mfo: &MsgObj, f: &str, source: &[u8]) -> Result<Value, Thrown> {
    let u = match super::utf8_to_utf16(source) {
        Ok(u) => u,
        Err(st) => {
            mfo.err.borrow_mut().code = st;
            error::check_status(host, &mfo.err, f, "Converting parse string failed")?;
            return Ok(Value::Bool(false));
        }
    };
    let (mut count, mut status) = (0, sys::U_ZERO_ERROR);
    // SAFETY: a live formatter, the source's units.
    let p = unsafe {
        cpp::rhp_msgfmt_parse(
            mfo.fmt(),
            u.as_ptr(),
            u.len() as i32,
            &mut count,
            &mut status,
        )
    };
    let mut out = PhpArray::new();
    if !p.is_null() {
        for k in 0..count.max(0) as usize {
            // SAFETY: `count` entries.
            let e = unsafe { &*p.add(k) };
            let v = match e.ty {
                cpp::K_DATE => Value::Float(e.d / 1000.0),
                cpp::K_DOUBLE => Value::Float(e.d),
                cpp::K_LONG => Value::Int(e.i),
                cpp::K_INT64 => {
                    if e.i < -i64::MAX {
                        Value::Float(e.i as f64)
                    } else {
                        Value::Int(e.i)
                    }
                }
                cpp::K_STRING => {
                    let s = cpp::take_ustr(e.s, e.s_len).unwrap_or_default();
                    match super::utf16_to_utf8(&s) {
                        Ok(s) => Value::str(s),
                        Err(st) => {
                            status = st;
                            Value::Null
                        }
                    }
                }
                _ => {
                    status = sys::U_ILLEGAL_ARGUMENT_ERROR;
                    Value::Null
                }
            };
            out.push(v);
        }
        // SAFETY: the shim's array.
        unsafe { cpp::rhp_intl_free(p.cast()) };
    }
    mfo.err.borrow_mut().code = status;
    if error::check_status(host, &mfo.err, f, "Parsing failed")? {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Array(out))
}

/// A temporary formatter for formatMessage/parseMessage: `None` after
/// the error was set.
fn temp(
    host: &mut dyn Host,
    f: &str,
    locale: &[u8],
    pattern: &[u8],
    parse: bool,
) -> Result<Option<MsgObj>, Thrown> {
    if super::locale_too_long(host, f, locale.len())? {
        return Ok(None);
    }
    let mfo = MsgObj::default();
    let mut spattern = Vec::new();
    if !pattern.is_empty() {
        match super::utf8_to_utf16(pattern) {
            Ok(u) => spattern = u,
            Err(_) => {
                error::set(
                    host,
                    f,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "error converting pattern to UTF-16",
                )?;
                return Ok(None);
            }
        }
    }
    let locale = if locale.is_empty() {
        super::locale_get_default()
    } else {
        locale.to_vec()
    };
    let (fmt, status, pe) = open(&locale, &spattern);
    mfo.umsgf.set(fmt as usize);
    mfo.err.borrow_mut().code = status;
    if parse {
        if error::check_status(host, &mfo.err, f, "Creating message formatter failed")? {
            return Ok(None);
        }
    } else if sys::failure(status) {
        if status == sys::U_PATTERN_SYNTAX_ERROR {
            let msg = format!(
                "pattern syntax error ({})",
                error::parse_error_to_string(&pe)
            );
            error::error_set_code(status);
            error::set_custom_msg(host, f, &msg)?;
        } else {
            error::set_custom_msg(host, f, "Creating message formatter failed")?;
        }
        return Ok(None);
    }
    Ok(Some(mfo))
}

/// `__msgfmt(method, argc, $this, ...args)`.
pub(super) fn fn_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("MessageFormatter", args, ALIASES);
    let f = call.fname.clone();
    let array = |i: usize| match call.arg(i) {
        Some(Value::Array(a)) => a,
        _ => PhpArray::new(),
    };
    match call.name {
        "__construct" => {
            let this = call.this.clone().expect("$this");
            let mfo = common::state_of::<MsgObj>(&this).expect("an object");
            let (locale, pattern) = (call.bytes(0), call.bytes(1));
            error::in_ctor(|| ctor(host, &f, &mfo, &locale, &pattern))?;
            return Ok(Value::Null);
        }
        "create" => {
            let (locale, pattern) = (call.bytes(0), call.bytes(1));
            let Some(obj) = host.instantiate(b"MessageFormatter", Vec::new()) else {
                return Ok(Value::Null);
            };
            let mfo = common::state_of::<MsgObj>(&obj).expect("an object");
            return Ok(if ctor(host, &f, &mfo, &locale, &pattern)? {
                obj
            } else {
                Value::Null
            });
        }
        "formatMessage" => {
            let (locale, pattern) = (call.bytes(0), call.bytes(1));
            let Some(mfo) = temp(host, &f, &locale, &pattern, false)? else {
                return Ok(if super::locale_too_long_len(locale.len()) {
                    Value::Null
                } else {
                    Value::Bool(false)
                });
            };
            return do_format(host, &mfo, &f, &array(2));
        }
        "parseMessage" => {
            let (locale, pattern, source) = (call.bytes(0), call.bytes(1), call.bytes(2));
            let Some(mfo) = temp(host, &f, &locale, &pattern, true)? else {
                return Ok(if super::locale_too_long_len(locale.len()) {
                    Value::Null
                } else {
                    Value::Bool(false)
                });
            };
            return do_parse(host, &mfo, &f, &source);
        }
        _ => {}
    }
    error::error_reset();
    let this = call.this.clone().expect("an object");
    let mfo = common::state_of::<MsgObj>(&this).expect("an object");
    match call.name {
        "getErrorCode" => return Ok(Value::Int(mfo.err.borrow().code as i64)),
        "getErrorMessage" => return Ok(Value::str(mfo.err.borrow().message())),
        _ => {}
    }
    mfo.err.borrow_mut().reset();
    if mfo.fmt().is_null() {
        return Err(Thrown::error(
            "Error",
            "Found unconstructed MessageFormatter",
        ));
    }
    match call.name {
        "format" => do_format(host, &mfo, &f, &array(0)),
        "parse" => do_parse(host, &mfo, &f, &call.bytes(0)),
        "getPattern" => Ok(match mfo.orig_format.borrow().as_ref() {
            Some(p) => Value::str(p.clone()),
            None => Value::Bool(false),
        }),
        "getLocale" => {
            // SAFETY: a live formatter.
            let p = unsafe { sys::umsg_getLocale(mfo.fmt()) };
            Ok(Value::str(super::c_bytes(p)))
        }
        "setPattern" => {
            let value = call.bytes(0);
            let u = match super::utf8_to_utf16(&value) {
                Ok(u) => u,
                Err(st) => {
                    mfo.err.borrow_mut().code = st;
                    error::check_status(host, &mfo.err, &f, "Error converting pattern to UTF-16")?;
                    return Ok(Value::Bool(false));
                }
            };
            let mut pe = sys::UParseError::default();
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: a live formatter, the pattern's units.
            unsafe {
                sys::umsg_applyPattern(mfo.fmt(), u.as_ptr(), u.len() as i32, &mut pe, &mut status)
            };
            if sys::failure(status) {
                mfo.err.borrow_mut().code = status;
                let msg = format!(
                    "Error setting symbol value at line {}, offset {}",
                    pe.line, pe.offset
                );
                error::errors_set_custom_msg(host, &mfo.err, &f, &msg)?;
                return Ok(Value::Bool(false));
            }
            *mfo.orig_format.borrow_mut() = Some(value);
            *mfo.arg_types.borrow_mut() = None;
            Ok(Value::Bool(true))
        }
        other => unreachable!("MessageFormatter::{other}"),
    }
}
