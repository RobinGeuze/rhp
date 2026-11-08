//! `IntlChar`, ported from php-src ext/intl/uchar/uchar.cpp onto ICU's
//! `uchar.h`.

use std::ffi::{c_char, c_void};

use rhp_values::{PhpArray, Value};

use super::common::{self, Call};
use super::error;
use super::sys::{self, UBool, UChar32};
use crate::{Host, Thrown};

pub(super) const SPEC: common::ClassSpec = common::ClassSpec {
    name: "IntlChar",
    helper: "__intlchar",
    props: "",
};

/// `UCHAR_MAX_VALUE`.
const UCHAR_MAX_VALUE: i64 = 0x10ffff;
/// `U_UNICODE_CHAR_NAME`, `U_LONG_PROPERTY_NAME`, `U_FOLD_CASE_DEFAULT`.
const U_UNICODE_CHAR_NAME: i64 = 0;
const U_LONG_PROPERTY_NAME: i64 = 1;
const U_FOLD_CASE_DEFAULT: i64 = 0;

/// `convert_cp`: the code point of a one-character UTF-8 string or an int.
/// `None` after setting the global error.
fn convert_cp(host: &mut dyn Host, f: &str, v: &Value) -> Result<Option<UChar32>, Thrown> {
    let cp = match v {
        Value::Str(s) => {
            let s = s.as_bytes();
            if s.len() > i32::MAX as usize {
                error::error_set_code(sys::U_ILLEGAL_ARGUMENT_ERROR);
                error::set_custom_msg(host, f, "Input string is too long.")?;
                return Ok(None);
            }
            // U8_NEXT reads the NUL after an empty string, as php's does.
            let mut buf = s.to_vec();
            buf.push(0);
            let mut cp = 0;
            // SAFETY: `s.len()` bytes plus the NUL.
            let i = unsafe { rhp_u8_next(buf.as_ptr(), s.len() as i32, &mut cp) };
            if i as usize != s.len() {
                error::error_set_code(sys::U_ILLEGAL_ARGUMENT_ERROR);
                error::set_custom_msg(
                    host,
                    f,
                    "Passing a UTF-8 character for codepoint requires a string which is exactly one UTF-8 codepoint long.",
                )?;
                return Ok(None);
            }
            cp as i64
        }
        Value::Int(n) => *n,
        _ => 0,
    };
    if !(0..=UCHAR_MAX_VALUE).contains(&cp) {
        error::error_set_code(sys::U_ILLEGAL_ARGUMENT_ERROR);
        error::set_custom_msg(host, f, "Codepoint out of range")?;
        return Ok(None);
    }
    Ok(Some(cp as UChar32))
}

/// `U8_APPEND_UNSAFE`.
fn utf8(cp: UChar32) -> Vec<u8> {
    let mut b = [0u8; 4];
    match char::from_u32(cp as u32) {
        Some(c) => c.encode_utf8(&mut b).as_bytes().to_vec(),
        // a surrogate: its three-byte form
        None => {
            let c = cp as u32;
            vec![
                (0xe0 | (c >> 12)) as u8,
                (0x80 | ((c >> 6) & 0x3f)) as u8,
                (0x80 | (c & 0x3f)) as u8,
            ]
        }
    }
}

/// The result of a `IC_CHAR_METHOD_CHAR`: a string for a string argument.
fn char_result(arg: &Value, cp: UChar32) -> Value {
    match arg {
        Value::Str(_) => Value::str(utf8(cp)),
        _ => Value::Int(cp as i64),
    }
}

fn version_array(v: [u8; 4]) -> Value {
    let mut a = PhpArray::new();
    for n in v {
        a.push(Value::Int(n as i64));
    }
    Value::Array(a)
}

/// The context of an enumeration callback.
struct Enum<'a> {
    host: &'a mut dyn Host,
    callback: Value,
    thrown: Option<Thrown>,
}

extern "C" fn enum_types_cb(ctx: *const c_void, start: UChar32, limit: UChar32, ty: i32) -> UBool {
    // SAFETY: the `Enum` enumCharTypes passed, alive for the call.
    let e = unsafe { &mut *(ctx as *mut Enum) };
    let args = [
        Value::Int(start as i64),
        Value::Int(limit as i64),
        Value::Int(ty as i64),
    ];
    match e.host.call_callable(&e.callback, &args) {
        Ok(_) if !e.host.has_pending_throw() => 1,
        Ok(_) => 0,
        Err(t) => {
            e.thrown = Some(t);
            0
        }
    }
}

extern "C" fn enum_names_cb(
    ctx: *mut c_void,
    code: UChar32,
    choice: i32,
    name: *const c_char,
    len: i32,
) -> UBool {
    // SAFETY: as above; `len` bytes at `name`.
    let e = unsafe { &mut *(ctx as *mut Enum) };
    let name = unsafe { std::slice::from_raw_parts(name as *const u8, len.max(0) as usize) };
    let args = [
        Value::Int(code as i64),
        Value::Int(choice as i64),
        Value::str(name.to_vec()),
    ];
    match e.host.call_callable(&e.callback, &args) {
        Ok(_) if !e.host.has_pending_throw() => 1,
        Ok(_) => 0,
        Err(t) => {
            e.thrown = Some(t);
            0
        }
    }
}

/// `__intlchar(method, argc, null, ...args)`.
pub(super) fn fn_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("IntlChar", args, &[]);
    let f = call.fname.clone();
    error::error_reset();
    let arg0 = call.arg(0).unwrap_or(Value::Null);
    macro_rules! cp {
        () => {
            match convert_cp(host, &f, &arg0)? {
                Some(cp) => cp,
                None => return Ok(Value::Null),
            }
        };
    }
    // SAFETY (below): plain ICU property lookups on a valid code point.
    let b = |r: UBool| Value::Bool(r != 0);
    Ok(match call.name {
        "chr" => Value::str(utf8(cp!())),
        "ord" => Value::Int(cp!() as i64),
        "hasBinaryProperty" => {
            let c = cp!();
            b(unsafe { u_hasBinaryProperty(c, call.long(1, 0) as i32) })
        }
        "getIntPropertyValue" => {
            let c = cp!();
            Value::Int(unsafe { u_getIntPropertyValue(c, call.long(1, 0) as i32) } as i64)
        }
        "getIntPropertyMinValue" => {
            Value::Int(unsafe { u_getIntPropertyMinValue(call.long(0, 0) as i32) } as i64)
        }
        "getIntPropertyMaxValue" => {
            Value::Int(unsafe { u_getIntPropertyMaxValue(call.long(0, 0) as i32) } as i64)
        }
        "getNumericValue" => Value::Float(unsafe { u_getNumericValue(cp!()) }),
        "enumCharTypes" => {
            let mut e = Enum {
                host,
                callback: arg0,
                thrown: None,
            };
            unsafe { u_enumCharTypes(enum_types_cb, &mut e as *mut Enum as *const c_void) };
            if let Some(t) = e.thrown {
                return Err(t);
            }
            Value::Null
        }
        "getBlockCode" => Value::Int(unsafe { ublock_getCode(cp!()) } as i64),
        "charName" => {
            let c = cp!();
            let choice = call.long(1, U_UNICODE_CHAR_NAME) as i32;
            let mut status = sys::U_ZERO_ERROR;
            let len = unsafe { u_charName(c, choice, std::ptr::null_mut(), 0, &mut status) };
            let mut buf = vec![0u8; len.max(0) as usize + 1];
            status = sys::U_ZERO_ERROR;
            let len = unsafe {
                u_charName(
                    c,
                    choice,
                    buf.as_mut_ptr() as *mut c_char,
                    buf.len() as i32,
                    &mut status,
                )
            };
            if sys::failure(status) {
                error::check_global(host, status, &f, "Failure getting character name")?;
                return Ok(Value::Null);
            }
            buf.truncate(len.max(0) as usize);
            Value::str(buf)
        }
        "charFromName" => {
            let name = super::c_string(&call.bytes(0));
            let mut status = sys::U_ZERO_ERROR;
            let r = unsafe {
                u_charFromName(
                    call.long(1, U_UNICODE_CHAR_NAME) as i32,
                    name.as_ptr(),
                    &mut status,
                )
            };
            error::error_set_code(status);
            if sys::failure(status) {
                return Ok(Value::Null);
            }
            Value::Int(r as i64)
        }
        "enumCharNames" => {
            let start = match convert_cp(host, &f, &arg0)? {
                Some(c) => c,
                None => return Ok(Value::Bool(false)),
            };
            let limit = match convert_cp(host, &f, &call.arg(1).unwrap_or(Value::Null))? {
                Some(c) => c,
                None => return Ok(Value::Bool(false)),
            };
            let choice = call.long(3, U_UNICODE_CHAR_NAME) as i32;
            let callback = call.arg(2).unwrap_or(Value::Null);
            let mut status = sys::U_ZERO_ERROR;
            let mut e = Enum {
                host,
                callback,
                thrown: None,
            };
            unsafe {
                u_enumCharNames(
                    start,
                    limit,
                    enum_names_cb,
                    &mut e as *mut Enum as *mut c_void,
                    choice,
                    &mut status,
                )
            };
            if let Some(t) = e.thrown {
                return Err(t);
            }
            error::error_set_code(status);
            if sys::failure(status) {
                return Ok(Value::Bool(false));
            }
            Value::Bool(true)
        }
        "getPropertyName" | "getPropertyValueName" => {
            let p = call.long(0, 0) as i32;
            let r = if call.name == "getPropertyName" {
                unsafe { u_getPropertyName(p, call.long(1, U_LONG_PROPERTY_NAME) as i32) }
            } else {
                unsafe {
                    u_getPropertyValueName(
                        p,
                        call.long(1, 0) as i32,
                        call.long(2, U_LONG_PROPERTY_NAME) as i32,
                    )
                }
            };
            if r.is_null() {
                error::error_set_code(sys::U_ILLEGAL_ARGUMENT_ERROR);
                error::set_custom_msg(host, &f, "Failed to get property name")?;
                return Ok(Value::Bool(false));
            }
            Value::str(super::c_bytes(r))
        }
        "getPropertyEnum" => {
            let a = super::c_string(&call.bytes(0));
            Value::Int(unsafe { u_getPropertyEnum(a.as_ptr()) } as i64)
        }
        "getPropertyValueEnum" => {
            let n = super::c_string(&call.bytes(1));
            Value::Int(unsafe { u_getPropertyValueEnum(call.long(0, 0) as i32, n.as_ptr()) } as i64)
        }
        "foldCase" => {
            let c = cp!();
            char_result(&arg0, unsafe {
                u_foldCase(c, call.long(1, U_FOLD_CASE_DEFAULT) as u32)
            })
        }
        "digit" => {
            let c = cp!();
            let r = unsafe { u_digit(c, call.long(1, 10) as i8) };
            if r < 0 {
                error::error_set_code(sys::U_ILLEGAL_ARGUMENT_ERROR);
                error::set_custom_msg(host, &f, "Invalid digit")?;
                return Ok(Value::Bool(false));
            }
            Value::Int(r as i64)
        }
        "forDigit" => {
            Value::Int(unsafe { u_forDigit(call.long(0, 0) as i32, call.long(1, 10) as i8) } as i64)
        }
        "charAge" => {
            let c = cp!();
            let mut v = [0u8; 4];
            unsafe { u_charAge(c, v.as_mut_ptr()) };
            version_array(v)
        }
        "getUnicodeVersion" => {
            let mut v = [0u8; 4];
            unsafe { u_getUnicodeVersion(v.as_mut_ptr()) };
            version_array(v)
        }
        "getFC_NFKC_Closure" => {
            let c = cp!();
            let mut status = sys::U_ZERO_ERROR;
            let len = unsafe { u_getFC_NFKC_Closure(c, std::ptr::null_mut(), 0, &mut status) };
            if len == 0 {
                return Ok(Value::str(Vec::new()));
            }
            let mut buf = vec![0u16; len as usize + 1];
            status = sys::U_ZERO_ERROR;
            let len = unsafe { u_getFC_NFKC_Closure(c, buf.as_mut_ptr(), len, &mut status) };
            if error::check_global(host, status, &f, "Failed getting closure")? {
                return Ok(Value::Bool(false));
            }
            match super::utf16_to_utf8(&buf[..len as usize]) {
                Ok(s) => Value::str(s),
                Err(st) => {
                    error::check_global(host, st, &f, "Failed converting output to UTF8")?;
                    Value::Bool(false)
                }
            }
        }
        "charDirection" => Value::Int(unsafe { u_charDirection(cp!()) } as i64),
        "charType" => Value::Int(unsafe { u_charType(cp!()) } as i64),
        "getCombiningClass" => Value::Int(unsafe { u_getCombiningClass(cp!()) } as i64),
        "charDigitValue" => Value::Int(unsafe { u_charDigitValue(cp!()) } as i64),
        "charMirror" => char_result(&arg0, unsafe { u_charMirror(cp!()) }),
        "tolower" => char_result(&arg0, unsafe { u_tolower(cp!()) }),
        "toupper" => char_result(&arg0, unsafe { u_toupper(cp!()) }),
        "totitle" => char_result(&arg0, unsafe { u_totitle(cp!()) }),
        "getBidiPairedBracket" => char_result(&arg0, unsafe { u_getBidiPairedBracket(cp!()) }),
        name => {
            let c = cp!();
            let f: unsafe extern "C" fn(UChar32) -> UBool = match name {
                "isUAlphabetic" => u_isUAlphabetic,
                "isULowercase" => u_isULowercase,
                "isUUppercase" => u_isUUppercase,
                "isUWhiteSpace" => u_isUWhiteSpace,
                "islower" => u_islower,
                "isupper" => u_isupper,
                "istitle" => u_istitle,
                "isdigit" => u_isdigit,
                "isalpha" => u_isalpha,
                "isalnum" => u_isalnum,
                "isxdigit" => u_isxdigit,
                "ispunct" => u_ispunct,
                "isgraph" => u_isgraph,
                "isblank" => u_isblank,
                "isdefined" => u_isdefined,
                "isspace" => u_isspace,
                "isJavaSpaceChar" => u_isJavaSpaceChar,
                "isWhitespace" => u_isWhitespace,
                "iscntrl" => u_iscntrl,
                "isISOControl" => u_isISOControl,
                "isprint" => u_isprint,
                "isbase" => u_isbase,
                "isMirrored" => u_isMirrored,
                "isIDStart" => u_isIDStart,
                "isIDPart" => u_isIDPart,
                "isIDIgnorable" => u_isIDIgnorable,
                "isJavaIDStart" => u_isJavaIDStart,
                "isJavaIDPart" => u_isJavaIDPart,
                other => unreachable!("IntlChar::{other}"),
            };
            b(unsafe { f(c) })
        }
    })
}

unsafe extern "C" {
    /// icu.cpp: ICU's `U8_NEXT` over `len` bytes from 0; the index after.
    fn rhp_u8_next(s: *const u8, len: i32, cp: *mut UChar32) -> i32;
}

type EnumTypesFn = extern "C" fn(*const c_void, UChar32, UChar32, i32) -> UBool;
type EnumNamesFn = extern "C" fn(*mut c_void, UChar32, i32, *const c_char, i32) -> UBool;

macro_rules! icu_bool {
    ($($name:ident),*) => {
        super::sys::icu! { $( pub fn $name(c: UChar32) -> UBool; )* }
    };
}

icu_bool!(
    u_isUAlphabetic,
    u_isULowercase,
    u_isUUppercase,
    u_isUWhiteSpace,
    u_islower,
    u_isupper,
    u_istitle,
    u_isdigit,
    u_isalpha,
    u_isalnum,
    u_isxdigit,
    u_ispunct,
    u_isgraph,
    u_isblank,
    u_isdefined,
    u_isspace,
    u_isJavaSpaceChar,
    u_isWhitespace,
    u_iscntrl,
    u_isISOControl,
    u_isprint,
    u_isbase,
    u_isMirrored,
    u_isIDStart,
    u_isIDPart,
    u_isIDIgnorable,
    u_isJavaIDStart,
    u_isJavaIDPart
);

super::sys::icu! {
    pub fn u_hasBinaryProperty(c: UChar32, p: i32) -> UBool;
    pub fn u_getIntPropertyValue(c: UChar32, p: i32) -> i32;
    pub fn u_getIntPropertyMinValue(p: i32) -> i32;
    pub fn u_getIntPropertyMaxValue(p: i32) -> i32;
    pub fn u_getNumericValue(c: UChar32) -> f64;
    pub fn u_enumCharTypes(f: EnumTypesFn, ctx: *const c_void);
    pub fn ublock_getCode(c: UChar32) -> i32;
    pub fn u_charName(c: UChar32, choice: i32, buf: *mut c_char, cap: i32, status: *mut sys::UErrorCode) -> i32;
    pub fn u_charFromName(choice: i32, name: *const c_char, status: *mut sys::UErrorCode) -> UChar32;
    pub fn u_enumCharNames(
        start: UChar32, limit: UChar32, f: EnumNamesFn, ctx: *mut c_void, choice: i32, status: *mut sys::UErrorCode,
    );
    pub fn u_getPropertyName(p: i32, choice: i32) -> *const c_char;
    pub fn u_getPropertyEnum(alias: *const c_char) -> i32;
    pub fn u_getPropertyValueName(p: i32, v: i32, choice: i32) -> *const c_char;
    pub fn u_getPropertyValueEnum(p: i32, alias: *const c_char) -> i32;
    pub fn u_foldCase(c: UChar32, options: u32) -> UChar32;
    pub fn u_digit(c: UChar32, radix: i8) -> i32;
    pub fn u_forDigit(digit: i32, radix: i8) -> UChar32;
    pub fn u_charAge(c: UChar32, v: *mut u8);
    pub fn u_getUnicodeVersion(v: *mut u8);
    pub fn u_getFC_NFKC_Closure(c: UChar32, buf: *mut u16, cap: i32, status: *mut sys::UErrorCode) -> i32;
    pub fn u_charDirection(c: UChar32) -> i32;
    pub fn u_charType(c: UChar32) -> i8;
    pub fn u_getCombiningClass(c: UChar32) -> u8;
    pub fn u_charDigitValue(c: UChar32) -> i32;
    pub fn u_charMirror(c: UChar32) -> UChar32;
    pub fn u_tolower(c: UChar32) -> UChar32;
    pub fn u_toupper(c: UChar32) -> UChar32;
    pub fn u_totitle(c: UChar32) -> UChar32;
    pub fn u_getBidiPairedBracket(c: UChar32) -> UChar32;
}
