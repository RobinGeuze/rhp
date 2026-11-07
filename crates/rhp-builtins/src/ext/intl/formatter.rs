//! `NumberFormatter` and `numfmt_*`, ported from php-src
//! ext/intl/formatter/ onto ICU's `unum`. A `NumberFormatter` object's
//! state (php's `NumberFormatter_object`) is its payload.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::OnceLock;

use rhp_values::{ArrayKey, ObjectHandle, Value};

use super::error::{self, IntlError};
use super::sys::{self, UChar, UErrorCode, UNumberFormat};
use super::{class_int, utf16_to_utf8, utf8_to_utf16};
use crate::zend::object_handlers::{self, CloneObj, ObjectHandlers};
use crate::zend::zpp::{self, deref};
use crate::{Host, Thrown};

pub(super) static HANDLERS: ObjectHandlers = ObjectHandlers {
    clone_obj: CloneObj::Custom(clone_obj),
    ..ObjectHandlers::STD
};

const TYPE_DEFAULT: i64 = 0;
const TYPE_INT32: i64 = 1;
const TYPE_INT64: i64 = 2;
const TYPE_DOUBLE: i64 = 3;
const TYPE_CURRENCY: i64 = 4;

/// `NumberFormatter_object`.
#[derive(Default)]
pub(super) struct NumFmt {
    err: RefCell<IntlError>,
    unum: Cell<usize>,
}

impl NumFmt {
    fn unum(&self) -> *mut UNumberFormat {
        self.unum.get() as *mut UNumberFormat
    }
    fn code(&self) -> UErrorCode {
        self.err.borrow().code
    }
    fn set_code(&self, c: UErrorCode) {
        self.err.borrow_mut().code = c;
    }
}

impl Drop for NumFmt {
    fn drop(&mut self) {
        let f = self.unum();
        if !f.is_null() {
            // SAFETY: the formatter this object opened.
            unsafe { sys::unum_close(f) };
        }
    }
}

fn payload(h: &ObjectHandle) -> Rc<NumFmt> {
    if let Some(o) = object_handlers::payload_of::<NumFmt>(h) {
        return o;
    }
    let o = Rc::new(NumFmt::default());
    object_handlers::set_payload(h, o.clone());
    o
}

fn object(host: &dyn Host, v: &Value) -> Option<Rc<NumFmt>> {
    if !object_handlers::has_handlers(host, v, &HANDLERS) {
        return None;
    }
    Some(payload(&object_handlers::object_of(v)?))
}

/// `NumberFormatter_object_clone`.
fn clone_obj(_host: &mut dyn Host, src: &ObjectHandle, dst: &ObjectHandle) -> Result<(), Thrown> {
    let from = payload(src);
    let to = Rc::new(NumFmt::default());
    object_handlers::set_payload(dst, to.clone());
    if from.unum().is_null() {
        return Err(Thrown::error(
            "Error",
            "Cannot clone uninitialized NumberFormatter",
        ));
    }
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a live formatter.
    let f = unsafe { sys::unum_clone(from.unum(), &mut status) };
    to.unum.set(f as usize);
    if sys::failure(status) {
        return Err(Thrown::error("Error", "Failed to clone NumberFormatter"));
    }
    Ok(())
}

/// The UNUM attribute groups numfmt_get_attribute/numfmt_set_attribute
/// accept: the integer ones, then UNUM_ROUNDING_INCREMENT.
fn attrs() -> &'static (Vec<i64>, i64) {
    static A: OnceLock<(Vec<i64>, i64)> = OnceLock::new();
    A.get_or_init(|| {
        let ints = [
            "PARSE_INT_ONLY",
            "GROUPING_USED",
            "DECIMAL_ALWAYS_SHOWN",
            "MAX_INTEGER_DIGITS",
            "MIN_INTEGER_DIGITS",
            "INTEGER_DIGITS",
            "MAX_FRACTION_DIGITS",
            "MIN_FRACTION_DIGITS",
            "FRACTION_DIGITS",
            "MULTIPLIER",
            "GROUPING_SIZE",
            "ROUNDING_MODE",
            "FORMAT_WIDTH",
            "PADDING_POSITION",
            "SECONDARY_GROUPING_SIZE",
            "SIGNIFICANT_DIGITS_USED",
            "MIN_SIGNIFICANT_DIGITS",
            "MAX_SIGNIFICANT_DIGITS",
            "LENIENT_PARSE",
        ]
        .iter()
        .map(|n| class_int("NumberFormatter", n))
        .collect();
        (ints, class_int("NumberFormatter", "ROUNDING_INCREMENT"))
    })
}

struct Call<'a> {
    fname: &'a str,
    method: bool,
}

/// The formatter and the arguments after it (`"O"`).
fn this_and_args<'a>(
    host: &dyn Host,
    call: &Call,
    this: Option<&Value>,
    args: &'a [Value],
) -> Result<(Rc<NumFmt>, &'a [Value]), Thrown> {
    if let Some(t) = this {
        return Ok((object(host, t).expect("a NumberFormatter"), args));
    }
    let v = args.first().map(deref).unwrap_or(Value::Null);
    match object(host, &v) {
        Some(o) => Ok((o, &args[1..])),
        None => Err(Thrown::type_error(format!(
            "{}(): Argument #1 ($formatter) must be of type NumberFormatter, {} given",
            call.fname,
            zpp::zval_value_name(host, &v)
        ))),
    }
}

fn int_at(args: &[Value], i: usize, default: i64) -> i64 {
    args.get(i)
        .map(|v| rhp_values::conv::to_int(&deref(v)))
        .unwrap_or(default)
}

/// `INTL_METHOD_RETVAL_UTF8`.
fn retval_utf8(host: &mut dyn Host, nf: &NumFmt, f: &str, u: &[UChar]) -> Result<Value, Thrown> {
    match utf16_to_utf8(u) {
        Ok(s) => {
            nf.set_code(sys::U_ZERO_ERROR);
            error::error_set_code(sys::U_ZERO_ERROR);
            Ok(Value::str(s))
        }
        Err(st) => {
            nf.set_code(st);
            error::check_status(host, &nf.err, f, "Error converting value to UTF-8")?;
            Ok(Value::Bool(false))
        }
    }
}

/// A UTF-16 copy of `s` with php's terminating NUL unit.
fn utf16_z(s: &[u8]) -> Result<Vec<UChar>, UErrorCode> {
    let mut u = utf8_to_utf16(s)?;
    u.push(0);
    Ok(u)
}

/// php's format calls: a 32-unit buffer, then one of the length ICU asked
/// for.
fn format_with(
    nf: &NumFmt,
    mut call: impl FnMut(*mut UChar, i32, &mut UErrorCode) -> i32,
) -> Vec<UChar> {
    let mut buf: Vec<UChar> = vec![0; 32];
    let mut st = sys::U_ZERO_ERROR;
    let mut len = call(buf.as_mut_ptr(), 32, &mut st);
    if st == sys::U_BUFFER_OVERFLOW_ERROR {
        st = sys::U_ZERO_ERROR;
        buf = vec![0; len.max(0) as usize];
        let n = call(buf.as_mut_ptr(), len, &mut st);
        if !sys::failure(st) {
            len = n;
        }
    }
    nf.set_code(st);
    buf.truncate(len.clamp(0, buf.len() as i32) as usize);
    buf
}

// --- construction ------------------------------------------------------------

/// `numfmt_ctor`: false after a failure.
fn ctor(host: &mut dyn Host, fname: &str, this: &Value, args: &[Value]) -> Result<bool, Thrown> {
    error::error_reset();
    let locale = zpp::str_arg(host, args, 0, fname, "locale")?;
    let style = zpp::int_arg(host, args, 1, fname, "style")?;
    let pattern = zpp::opt_str_arg(host, args, 2, fname, "pattern")?;
    if super::locale_too_long(host, fname, locale.len())? {
        return Ok(false);
    }
    let nf = object(host, this).expect("a NumberFormatter");
    nf.err.borrow_mut().reset();
    if !nf.unum().is_null() {
        return Err(Thrown::error(
            "Error",
            "NumberFormatter object is already constructed",
        ));
    }
    let mut spattern: Vec<UChar> = Vec::new();
    if let Some(p) = pattern.filter(|p| !p.is_empty()) {
        match utf8_to_utf16(&p) {
            Ok(u) => spattern = u,
            Err(st) => {
                nf.set_code(st);
                error::check_status(host, &nf.err, fname, "error converting pattern to UTF-16")?;
                return Ok(false);
            }
        }
    }
    let locale = if locale.is_empty() {
        super::locale_get_default()
    } else {
        locale
    };
    let loc = super::c_string(&locale);
    // SAFETY: a NUL-terminated locale name; ICU returns a static string.
    let iso3 = super::c_bytes(unsafe { sys::uloc_getISO3Language(loc.as_ptr()) });
    if iso3.is_empty() {
        return Err(zpp::value_error(
            fname,
            1,
            "locale",
            &format!("\"{}\" is invalid", String::from_utf8_lossy(loc.as_bytes())),
        ));
    }
    let canon = super::canonicalize_locale_string(&loc);
    let final_loc = canon.as_deref().unwrap_or(&loc);
    let mut status = sys::U_ZERO_ERROR;
    let pat = if spattern.is_empty() {
        std::ptr::null()
    } else {
        spattern.as_ptr()
    };
    // SAFETY: the pattern (or null) and a NUL-terminated locale.
    let f = unsafe {
        sys::unum_open(
            style as i32,
            pat,
            spattern.len() as i32,
            final_loc.as_ptr(),
            std::ptr::null_mut(),
            &mut status,
        )
    };
    nf.unum.set(f as usize);
    nf.set_code(status);
    if error::check_status(host, &nf.err, fname, "number formatter creation failed")? {
        return Ok(false);
    }
    Ok(true)
}

fn create(host: &mut dyn Host, fname: &str, args: &[Value]) -> Result<Value, Thrown> {
    let Some(obj) = host.instantiate(b"NumberFormatter", Vec::new()) else {
        return Ok(Value::Null);
    };
    if ctor(host, fname, &obj, args)? {
        Ok(obj)
    } else {
        Ok(Value::Null)
    }
}

// --- the functions -------------------------------------------------------------

fn dispatch(
    host: &mut dyn Host,
    name: &str,
    fname: &str,
    this: Option<&Value>,
    args: &[Value],
) -> Result<Value, Thrown> {
    let call = Call {
        fname,
        method: this.is_some(),
    };
    match name {
        "create" => return create(host, fname, args),
        "__construct" => {
            let this = this.expect("$this");
            error::in_ctor(|| ctor(host, fname, this, args))?;
            return Ok(Value::Null);
        }
        _ => {}
    }
    error::error_reset();
    let (nf, args) = this_and_args(host, &call, this, args)?;
    let f = fname;
    match name {
        "getErrorCode" => return Ok(Value::Int(nf.code() as i64)),
        "getErrorMessage" => return Ok(Value::str(nf.err.borrow().message().into_bytes())),
        "getSymbol" | "setSymbol" => {
            let sym = int_at(args, 0, 0);
            // SAFETY: a plain getter.
            let count = unsafe { sys::rhp_intl_unum_format_symbol_count() } as i64;
            if sym >= count || sym < 0 {
                error::set(
                    host,
                    f,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "invalid symbol value",
                )?;
                return Ok(Value::Bool(false));
            }
        }
        _ => {}
    }
    // FORMATTER_METHOD_FETCH_OBJECT
    nf.err.borrow_mut().reset();
    if nf.unum().is_null() {
        return Err(Thrown::error(
            "Error",
            "Found unconstructed NumberFormatter",
        ));
    }
    let u = nf.unum();
    match name {
        "format" => format(host, &nf, &call, u, args),
        "formatCurrency" => {
            let amount = rhp_values::conv::to_float(&deref(&args[0]));
            let currency = zpp::str_arg(host, args, 1, f, "currency")?;
            let mut cur = match utf16_z(&currency) {
                Ok(c) => c,
                Err(st) => {
                    nf.set_code(st);
                    error::check_status(host, &nf.err, f, "Currency conversion to UTF-16 failed")?;
                    return Ok(Value::Bool(false));
                }
            };
            // SAFETY: a live formatter, a NUL-terminated currency.
            let out = format_with(&nf, |b, n, st| unsafe {
                sys::unum_formatDoubleCurrency(
                    u,
                    amount,
                    cur.as_mut_ptr(),
                    b,
                    n,
                    std::ptr::null_mut(),
                    st,
                )
            });
            if sys::failure(nf.code()) {
                error::error_set_code(nf.code());
                error::errors_set_custom_msg(host, &nf.err, f, "Number formatting failed")?;
                return Ok(Value::Bool(false));
            }
            retval_utf8(host, &nf, f, &out)
        }
        "parse" => parse(host, &nf, &call, u, args),
        "parseCurrency" => parse_currency(host, &nf, f, u, args),
        "getAttribute" => {
            let attr = int_at(args, 0, 0);
            let (ints, incr) = attrs();
            let mut ret = Value::Null;
            if ints.contains(&attr) {
                // SAFETY: a live formatter.
                let v = unsafe { sys::unum_getAttribute(u, attr as i32) };
                if v == -1 {
                    nf.set_code(sys::U_UNSUPPORTED_ERROR);
                } else {
                    ret = Value::Int(v as i64);
                }
            } else if attr == *incr {
                // SAFETY: a live formatter.
                let v = unsafe { sys::unum_getDoubleAttribute(u, attr as i32) };
                if v == -1.0 {
                    nf.set_code(sys::U_UNSUPPORTED_ERROR);
                } else {
                    ret = Value::Float(v);
                }
            } else {
                nf.set_code(sys::U_UNSUPPORTED_ERROR);
            }
            if error::check_status(host, &nf.err, f, "Error getting attribute value")? {
                return Ok(Value::Bool(false));
            }
            Ok(ret)
        }
        "setAttribute" => {
            let attr = int_at(args, 0, 0);
            let value = deref(args.get(1).unwrap_or(&Value::Null));
            let (ints, incr) = attrs();
            if ints.contains(&attr) {
                let v = zpp::zval_get_long(host, &value)?;
                // SAFETY: a live formatter.
                unsafe { sys::unum_setAttribute(u, attr as i32, v as i32) };
            } else if attr == *incr {
                let v = rhp_values::conv::to_float(&value);
                // SAFETY: a live formatter.
                unsafe { sys::unum_setDoubleAttribute(u, attr as i32, v) };
            } else {
                nf.set_code(sys::U_UNSUPPORTED_ERROR);
            }
            if error::check_status(host, &nf.err, f, "Error setting attribute value")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(true))
        }
        "getTextAttribute" => {
            let attr = int_at(args, 0, 0) as i32;
            let mut st = sys::U_ZERO_ERROR;
            // SAFETY: a live formatter.
            let v = super::icu_uchars(64, &mut st, |b, n, s| unsafe {
                sys::unum_getTextAttribute(u, attr, b, n, s)
            });
            nf.set_code(st);
            if error::check_status(host, &nf.err, f, "Error getting attribute value")? {
                return Ok(Value::Bool(false));
            }
            retval_utf8(host, &nf, f, &v)
        }
        "setTextAttribute" => {
            let attr = int_at(args, 0, 0) as i32;
            let value = zpp::str_arg(host, args, 1, f, "value")?;
            let v = match utf8_to_utf16(&value) {
                Ok(v) => v,
                Err(st) => {
                    nf.set_code(st);
                    error::check_status(
                        host,
                        &nf.err,
                        f,
                        "Error converting attribute value to UTF-16",
                    )?;
                    return Ok(Value::Bool(false));
                }
            };
            let mut st = sys::U_ZERO_ERROR;
            // SAFETY: a live formatter and a UTF-16 slice.
            unsafe { sys::unum_setTextAttribute(u, attr, v.as_ptr(), v.len() as i32, &mut st) };
            nf.set_code(st);
            if error::check_status(host, &nf.err, f, "Error setting text attribute")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(true))
        }
        "getSymbol" => {
            let sym = int_at(args, 0, 0) as i32;
            let mut st = sys::U_ZERO_ERROR;
            // SAFETY: a live formatter.
            let v = super::icu_uchars(4, &mut st, |b, n, s| unsafe {
                sys::unum_getSymbol(u, sym, b, n, s)
            });
            nf.set_code(st);
            if error::check_status(host, &nf.err, f, "Error getting symbol value")? {
                return Ok(Value::Bool(false));
            }
            retval_utf8(host, &nf, f, &v)
        }
        "setSymbol" => {
            let sym = int_at(args, 0, 0) as i32;
            let value = zpp::str_arg(host, args, 1, f, "value")?;
            let v = match utf8_to_utf16(&value) {
                Ok(v) => v,
                Err(st) => {
                    nf.set_code(st);
                    error::check_status(
                        host,
                        &nf.err,
                        f,
                        "Error converting symbol value to UTF-16",
                    )?;
                    return Ok(Value::Bool(false));
                }
            };
            let mut st = sys::U_ZERO_ERROR;
            // SAFETY: a live formatter and a UTF-16 slice.
            unsafe { sys::unum_setSymbol(u, sym, v.as_ptr(), v.len() as i32, &mut st) };
            nf.set_code(st);
            if error::check_status(host, &nf.err, f, "Error setting symbol value")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(true))
        }
        "getPattern" => {
            let mut st = sys::U_ZERO_ERROR;
            // SAFETY: a live formatter.
            let v = super::icu_uchars(64, &mut st, |b, n, s| unsafe {
                sys::unum_toPattern(u, 0, b, n, s)
            });
            nf.set_code(st);
            if error::check_status(host, &nf.err, f, "Error getting formatter pattern")? {
                return Ok(Value::Bool(false));
            }
            retval_utf8(host, &nf, f, &v)
        }
        "setPattern" => {
            let value = zpp::str_arg(host, args, 0, f, "pattern")?;
            let v = match utf8_to_utf16(&value) {
                Ok(v) => v,
                Err(st) => {
                    nf.set_code(st);
                    error::check_status(host, &nf.err, f, "Error converting pattern to UTF-16")?;
                    return Ok(Value::Bool(false));
                }
            };
            let mut perr = sys::UParseError::default();
            let mut st = sys::U_ZERO_ERROR;
            // SAFETY: a live formatter and a UTF-16 slice.
            unsafe { sys::unum_applyPattern(u, 0, v.as_ptr(), v.len() as i32, &mut perr, &mut st) };
            nf.set_code(st);
            if sys::failure(st) {
                let msg = format!(
                    "Error setting pattern value at line {}, offset {}",
                    perr.line, perr.offset
                );
                error::errors_set_custom_msg(host, &nf.err, f, &msg)?;
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(true))
        }
        "getLocale" => {
            let kind = int_at(args, 0, 0) as i32;
            let mut st = sys::U_ZERO_ERROR;
            // SAFETY: a live formatter.
            let p = unsafe { sys::unum_getLocaleByType(u, kind, &mut st) };
            nf.set_code(st);
            if error::check_status(host, &nf.err, f, "Error getting locale")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::str(super::c_bytes(p)))
        }
        _ => Ok(Value::Null),
    }
}

/// `numfmt_format`.
fn format(
    host: &mut dyn Host,
    nf: &NumFmt,
    call: &Call,
    u: *mut UNumberFormat,
    args: &[Value],
) -> Result<Value, Thrown> {
    let f = call.fname;
    let number = deref(args.first().unwrap_or(&Value::Null));
    let mut ty = int_at(args, 1, TYPE_DEFAULT);
    if ty == TYPE_DEFAULT {
        ty = if matches!(number, Value::Float(_)) {
            TYPE_DOUBLE
        } else {
            TYPE_INT64
        };
    }
    let out = match ty {
        TYPE_INT32 => {
            let n = zpp::zval_get_long(host, &number)? as i32;
            // SAFETY: a live formatter.
            format_with(nf, |b, c, st| unsafe {
                sys::unum_format(u, n, b, c, std::ptr::null_mut(), st)
            })
        }
        TYPE_INT64 => {
            let n = match number {
                Value::Float(d) => d as i64,
                ref v => rhp_values::conv::to_int(v),
            };
            // SAFETY: a live formatter.
            format_with(nf, |b, c, st| unsafe {
                sys::unum_formatInt64(u, n, b, c, std::ptr::null_mut(), st)
            })
        }
        TYPE_DOUBLE => {
            let n = rhp_values::conv::to_float(&number);
            // SAFETY: a live formatter.
            format_with(nf, |b, c, st| unsafe {
                sys::unum_formatDouble(u, n, b, c, std::ptr::null_mut(), st)
            })
        }
        TYPE_CURRENCY => {
            return Err(if call.method {
                zpp::value_error(
                    f,
                    2,
                    "type",
                    "cannot be NumberFormatter::TYPE_CURRENCY constant, use NumberFormatter::formatCurrency() method instead",
                )
            } else {
                zpp::value_error(
                    f,
                    3,
                    "type",
                    "cannot be NumberFormatter::TYPE_CURRENCY constant, use numfmt_format_currency() function instead",
                )
            })
        }
        _ => {
            return Err(zpp::value_error(
                f,
                if call.method { 2 } else { 3 },
                "type",
                "must be a NumberFormatter::TYPE_* constant",
            ))
        }
    };
    if error::check_status(host, &nf.err, f, "Number formatting failed")? {
        return Ok(Value::Bool(false));
    }
    retval_utf8(host, nf, f, &out)
}

/// The reference a by-reference argument arrives in: the function's own
/// argument, or the prelude's `[&$x]`.
fn ref_arg(v: Option<&Value>) -> Option<Value> {
    match v? {
        r @ Value::Ref(_) => Some(r.clone()),
        Value::Array(a) => match a.get(&ArrayKey::Int(0)) {
            Some(r @ Value::Ref(_)) => Some(r.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// `intl_convert_utf8_offset_to_utf16`: false for an invalid offset.
fn utf8_offset_to_utf16(s: &[u8], pos: &mut i32, status: &mut UErrorCode) -> bool {
    if *pos < 0 || *pos as usize > s.len() {
        return true;
    }
    *status = sys::U_ZERO_ERROR;
    let mut n = 0i32;
    // SAFETY: a preflight over the first `pos` bytes.
    unsafe {
        sys::u_strFromUTF8(
            std::ptr::null_mut(),
            0,
            &mut n,
            s.as_ptr().cast(),
            *pos,
            status,
        );
    }
    if *status != sys::U_BUFFER_OVERFLOW_ERROR && sys::failure(*status) {
        return false;
    }
    *status = sys::U_ZERO_ERROR;
    *pos = n;
    true
}

/// `intl_convert_utf16_offset_to_utf8`.
fn utf16_offset_to_utf8(s: &[UChar], mut pos: i32) -> i32 {
    if pos < 0 || pos as usize > s.len() {
        return pos;
    }
    let lead = |c: UChar| (c & 0xfc00) == 0xd800;
    let trail = |c: UChar| (c & 0xfc00) == 0xdc00;
    if pos > 0 && (pos as usize) < s.len() && lead(s[pos as usize - 1]) && trail(s[pos as usize]) {
        pos -= 1;
    }
    let mut n = 0i32;
    let mut st = sys::U_ZERO_ERROR;
    // SAFETY: a preflight over the first `pos` units.
    unsafe { sys::u_strToUTF8(std::ptr::null_mut(), 0, &mut n, s.as_ptr(), pos, &mut st) };
    if st != sys::U_BUFFER_OVERFLOW_ERROR && sys::failure(st) {
        return pos;
    }
    n
}

/// The `z!` position argument: `None` for null.
fn position_arg(host: &mut dyn Host, r: &Option<Value>) -> Result<Option<i32>, Thrown> {
    match r.as_ref().map(deref) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => Ok(Some(zpp::zval_get_long(host, &v)? as i32)),
    }
}

/// `numfmt_parse`.
fn parse(
    host: &mut dyn Host,
    nf: &NumFmt,
    call: &Call,
    u: *mut UNumberFormat,
    args: &[Value],
) -> Result<Value, Thrown> {
    let f = call.fname;
    let s = zpp::str_arg(host, args, 0, f, "string")?;
    let ty = int_at(args, 1, TYPE_DOUBLE);
    let zpos = ref_arg(args.get(2));
    let pos_in = position_arg(host, &zpos)?;
    let us = match utf8_to_utf16(&s) {
        Ok(v) => v,
        Err(st) => {
            nf.set_code(st);
            error::check_status(host, &nf.err, f, "String conversion to UTF-16 failed")?;
            return Ok(Value::Bool(false));
        }
    };
    let mut position = pos_in.unwrap_or(0);
    if pos_in.is_some() {
        let mut st = sys::U_ZERO_ERROR;
        let ok = utf8_offset_to_utf16(&s, &mut position, &mut st);
        nf.set_code(st);
        if !ok {
            error::check_status(host, &nf.err, f, "Invalid UTF-8 offset")?;
            return Ok(Value::Bool(false));
        }
    }
    let pp: *mut i32 = if pos_in.is_some() {
        &mut position
    } else {
        std::ptr::null_mut()
    };
    let mut st = sys::U_ZERO_ERROR;
    let ret = match ty {
        // SAFETY: a live formatter over a UTF-16 slice.
        TYPE_INT32 => {
            Value::Int(
                unsafe { sys::unum_parse(u, us.as_ptr(), us.len() as i32, pp, &mut st) } as i64,
            )
        }
        TYPE_INT64 => Value::Int(unsafe {
            sys::unum_parseInt64(u, us.as_ptr(), us.len() as i32, pp, &mut st)
        }),
        TYPE_DOUBLE => Value::Float(unsafe {
            sys::unum_parseDouble(u, us.as_ptr(), us.len() as i32, pp, &mut st)
        }),
        TYPE_CURRENCY => {
            return Err(if call.method {
                zpp::value_error(
                    f,
                    2,
                    "type",
                    "cannot be NumberFormatter::TYPE_CURRENCY constant, use NumberFormatter::parseCurrency() method instead",
                )
            } else {
                zpp::value_error(
                    f,
                    3,
                    "type",
                    "cannot be NumberFormatter::TYPE_CURRENCY constant, use numfmt_parse_currency() function instead",
                )
            })
        }
        _ => {
            return Err(zpp::value_error(
                f,
                if call.method { 2 } else { 3 },
                "type",
                "must be a NumberFormatter::TYPE_* constant",
            ))
        }
    };
    nf.set_code(st);
    if let (Some(r), Some(_)) = (&zpos, pos_in) {
        let p = utf16_offset_to_utf8(&us, position);
        zpp::try_assign_ref(host, std::slice::from_ref(r), 0, Value::Int(p as i64))?;
    }
    if error::check_status(host, &nf.err, f, "Number parsing failed")? {
        return Ok(Value::Bool(false));
    }
    Ok(ret)
}

/// `numfmt_parse_currency`.
fn parse_currency(
    host: &mut dyn Host,
    nf: &NumFmt,
    f: &str,
    u: *mut UNumberFormat,
    args: &[Value],
) -> Result<Value, Thrown> {
    let s = zpp::str_arg(host, args, 0, f, "string")?;
    let zcur = ref_arg(args.get(1));
    let zpos = ref_arg(args.get(2));
    let us = match utf8_to_utf16(&s) {
        Ok(v) => v,
        Err(st) => {
            nf.set_code(st);
            error::check_status(host, &nf.err, f, "String conversion to UTF-16 failed")?;
            return Ok(Value::Bool(false));
        }
    };
    let pos_in = position_arg(host, &zpos)?;
    let mut position = pos_in.unwrap_or(0);
    if pos_in.is_some() {
        let mut st = sys::U_ZERO_ERROR;
        let ok = utf8_offset_to_utf16(&s, &mut position, &mut st);
        nf.set_code(st);
        if !ok {
            error::check_status(host, &nf.err, f, "Invalid UTF-8 offset")?;
            return Ok(Value::Bool(false));
        }
    }
    let pp: *mut i32 = if pos_in.is_some() {
        &mut position
    } else {
        std::ptr::null_mut()
    };
    let mut currency: [UChar; 5] = [0; 5];
    let mut st = sys::U_ZERO_ERROR;
    // SAFETY: a live formatter over a UTF-16 slice; ICU writes at most
    // four units and a NUL.
    let number = unsafe {
        sys::unum_parseDoubleCurrency(
            u,
            us.as_ptr(),
            us.len() as i32,
            pp,
            currency.as_mut_ptr(),
            &mut st,
        )
    };
    nf.set_code(st);
    if let (Some(r), Some(_)) = (&zpos, pos_in) {
        let p = utf16_offset_to_utf8(&us, position);
        zpp::try_assign_ref(host, std::slice::from_ref(r), 0, Value::Int(p as i64))?;
    }
    if error::check_status(host, &nf.err, f, "Number parsing failed")? {
        return Ok(Value::Bool(false));
    }
    let len = currency.iter().position(|&c| c == 0).unwrap_or(5);
    let cur = match utf16_to_utf8(&currency[..len]) {
        Ok(c) => c,
        Err(st) => {
            nf.set_code(st);
            error::check_status(host, &nf.err, f, "Currency conversion to UTF-8 failed")?;
            return Ok(Value::Bool(false));
        }
    };
    if let Some(r) = &zcur {
        zpp::try_assign_ref(host, std::slice::from_ref(r), 0, Value::str(cur))?;
    }
    Ok(Value::Float(number))
}

// --- registration --------------------------------------------------------------

const FUNCS: &[(&str, &str)] = &[
    ("numfmt_create", "create"),
    ("numfmt_format", "format"),
    ("numfmt_parse", "parse"),
    ("numfmt_format_currency", "formatCurrency"),
    ("numfmt_parse_currency", "parseCurrency"),
    ("numfmt_set_attribute", "setAttribute"),
    ("numfmt_get_attribute", "getAttribute"),
    ("numfmt_set_text_attribute", "setTextAttribute"),
    ("numfmt_get_text_attribute", "getTextAttribute"),
    ("numfmt_set_symbol", "setSymbol"),
    ("numfmt_get_symbol", "getSymbol"),
    ("numfmt_set_pattern", "setPattern"),
    ("numfmt_get_pattern", "getPattern"),
    ("numfmt_get_locale", "getLocale"),
    ("numfmt_get_error_code", "getErrorCode"),
    ("numfmt_get_error_message", "getErrorMessage"),
];

macro_rules! procedural {
    ($($f:ident => $i:expr),*) => {
        $(fn $f(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            let (fname, name) = FUNCS[$i];
            dispatch(host, name, fname, None, args)
        })*
        pub(super) fn register(m: &mut crate::Registry) {
            let list: Vec<(&'static str, crate::NativeFn)> = vec![$((FUNCS[$i].0, $f as crate::NativeFn)),*];
            crate::register(m, &list);
        }
    };
}

procedural!(
    f0 => 0, f1 => 1, f2 => 2, f3 => 3, f4 => 4, f5 => 5, f6 => 6, f7 => 7, f8 => 8,
    f9 => 9, f10 => 10, f11 => 11, f12 => 12, f13 => 13, f14 => 14, f15 => 15
);

/// `__numfmt($method, $this, ...$args)`: the `NumberFormatter` methods.
pub(super) fn fn_method(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let method = rhp_values::conv::to_string_bytes(&deref(&args[0]));
    let method = String::from_utf8_lossy(&method).into_owned();
    let this = deref(&args[1]);
    let this = (!matches!(this, Value::Null)).then_some(this);
    let fname = format!("NumberFormatter::{method}");
    dispatch(host, &method, &fname, this.as_ref(), &args[2..])
}
