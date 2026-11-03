//! `normalizer_*` — Unicode normalization, ported from php-src
//! `ext/intl/normalizer/normalizer_normalize.cpp` onto ICU's `unorm2`.

use rhp_values::Value;

use super::sys::{self, UChar, UErrorCode, UNormalizer2};
use super::{error_reset, error_set, error_set_code, error_set_msg, utf16_to_utf8, utf8_to_utf16};
use crate::ext::zpp::{int_arg_or, str_arg, value_error};
use crate::{Host, Thrown};

const FORM_D: i64 = 4;
const FORM_KD: i64 = 8;
const FORM_C: i64 = 16;
const FORM_KC: i64 = 32;
const FORM_KC_CF: i64 = 48;

/// `intl_get_normalizer`: ICU's singleton for `form`, or null with
/// `U_ILLEGAL_ARGUMENT_ERROR`.
fn get_normalizer(form: i64, status: &mut UErrorCode) -> *const UNormalizer2 {
    // SAFETY: the getters return ICU-owned singletons.
    unsafe {
        match form {
            FORM_C => sys::unorm2_getNFCInstance(status),
            FORM_D => sys::unorm2_getNFDInstance(status),
            FORM_KC => sys::unorm2_getNFKCInstance(status),
            FORM_KD => sys::unorm2_getNFKDInstance(status),
            FORM_KC_CF => sys::unorm2_getNFKCCasefoldInstance(status),
            _ => {
                *status = sys::U_ILLEGAL_ARGUMENT_ERROR;
                std::ptr::null()
            }
        }
    }
}

/// `intl_normalize` into `dst`; returns the length needed.
fn normalize_into(form: i64, src: &[UChar], dst: &mut [UChar], status: &mut UErrorCode) -> i32 {
    let norm = get_normalizer(form, status);
    if sys::failure(*status) {
        return -1;
    }
    // SAFETY: `dst` holds at least its length plus one unit (see callers).
    unsafe {
        sys::unorm2_normalize(
            norm,
            src.as_ptr(),
            src.len() as i32,
            dst.as_mut_ptr(),
            dst.len() as i32,
            status,
        )
    }
}

fn form_arg(host: &mut dyn Host, args: &[Value], f: &str) -> Result<(Vec<u8>, i64), Thrown> {
    let s = str_arg(host, args, 0, f, "string")?;
    let form = int_arg_or(host, args, 1, f, "form", FORM_C)?;
    Ok((s, form))
}

fn invalid_form(f: &str) -> Thrown {
    value_error(f, 2, "form", "must be a a valid normalization form")
}

pub(super) fn fn_normalize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    normalize(host, args, "normalizer_normalize")
}

pub(super) fn fn_is_normalized(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    is_normalized(host, args, "normalizer_is_normalized")
}

pub(super) fn fn_get_raw_decomposition(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    get_raw_decomposition(host, args, "normalizer_get_raw_decomposition")
}

/// `__normalizer($method, ...)`: the `Normalizer` class's static methods,
/// which php's errors name `Normalizer::<method>`.
pub(super) fn fn_method(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let method = match args.first() {
        Some(Value::Str(s)) => s.as_bytes().to_vec(),
        _ => Vec::new(),
    };
    let rest = args.get(1..).unwrap_or(&[]);
    match &method[..] {
        b"normalize" => normalize(host, rest, "Normalizer::normalize"),
        b"isNormalized" => is_normalized(host, rest, "Normalizer::isNormalized"),
        b"getRawDecomposition" => {
            get_raw_decomposition(host, rest, "Normalizer::getRawDecomposition")
        }
        _ => Ok(Value::Null),
    }
}

fn normalize(host: &mut dyn Host, args: &[Value], f: &str) -> Result<Value, Thrown> {
    error_reset();
    let (input, form) = form_arg(host, args, f)?;
    let expansion = match form {
        FORM_D | FORM_KD => 3,
        FORM_C | FORM_KC | FORM_KC_CF => 1,
        _ => return Err(invalid_form(f)),
    };
    let uinput = match utf8_to_utf16(&input) {
        Ok(u) => u,
        Err(st) => {
            error_set(f, st, "Error converting input string to UTF-16");
            return Ok(Value::Bool(false));
        }
    };
    let mut cap = uinput.len() * expansion;
    // safe_femalloc(cap + 1, sizeof(UChar), 0, fallible): one unit more than
    // the capacity ICU is told.
    if !rhp_values::mem::reserve_safe(cap + 1, 2, 0) {
        return Ok(Value::Bool(false));
    }
    let mut buf: Vec<UChar> = vec![0; cap + 1];
    let mut status = sys::U_ZERO_ERROR;
    let mut needed = normalize_into(form, &uinput, &mut buf[..cap], &mut status);
    if sys::failure(status)
        && status != sys::U_BUFFER_OVERFLOW_ERROR
        && status != sys::U_STRING_NOT_TERMINATED_WARNING
    {
        error_set_msg(f, "Error normalizing string");
        return Ok(Value::Bool(false));
    }
    if needed as i64 > cap as i64 {
        cap = needed as usize;
        drop(buf);
        if !rhp_values::mem::reserve_safe(cap + 1, 2, 0) {
            return Ok(Value::Bool(false));
        }
        buf = vec![0; cap + 1];
        status = sys::U_ZERO_ERROR;
        needed = normalize_into(form, &uinput, &mut buf[..cap], &mut status);
        if sys::failure(status) {
            error_set_msg(f, "Error normalizing string");
            return Ok(Value::Bool(false));
        }
    }
    match utf16_to_utf8(&buf[..needed as usize]) {
        Ok(s) => Ok(Value::str(s)),
        Err(st) => {
            error_set(f, st, "error converting normalized text UTF-8");
            Ok(Value::Bool(false))
        }
    }
}

fn is_normalized(host: &mut dyn Host, args: &[Value], f: &str) -> Result<Value, Thrown> {
    error_reset();
    let (input, form) = form_arg(host, args, f)?;
    if !matches!(form, FORM_D | FORM_KD | FORM_C | FORM_KC | FORM_KC_CF) {
        return Err(invalid_form(f));
    }
    let uinput = match utf8_to_utf16(&input) {
        Ok(u) => u,
        Err(st) => {
            error_set(f, st, "Error converting string to UTF-16.");
            return Ok(Value::Bool(false));
        }
    };
    let mut status = sys::U_ZERO_ERROR;
    let norm = get_normalizer(form, &mut status);
    let normalized = !sys::failure(status)
        // SAFETY: a live normalizer over `uinput`.
        && unsafe {
            sys::unorm2_isNormalized(norm, uinput.as_ptr(), uinput.len() as i32, &mut status)
        } != 0;
    if sys::failure(status) {
        error_set_msg(
            f,
            "Error testing if string is the given normalization form.",
        );
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(normalized))
}

fn get_raw_decomposition(host: &mut dyn Host, args: &[Value], f: &str) -> Result<Value, Thrown> {
    error_reset();
    let (input, form) = form_arg(host, args, f)?;
    let mut status = sys::U_ZERO_ERROR;
    let norm = get_normalizer(form, &mut status);
    // U8_NEXT: one (possibly ill-formed) sequence must span the input; an
    // empty input reads its terminating NUL, one byte past the end.
    if input.is_empty() || super::grapheme::u8_fwd_1(&input, 0) != input.len() {
        error_set_code(sys::U_ILLEGAL_ARGUMENT_ERROR);
        error_set_msg(
            f,
            "Input string must be exactly one UTF-8 encoded code point long.",
        );
        return Ok(Value::Null);
    }
    // An ill-formed sequence decodes to U_SENTINEL (-1).
    let Some(c) = std::str::from_utf8(&input)
        .ok()
        .and_then(|t| t.chars().next())
    else {
        error_set_code(sys::U_ILLEGAL_ARGUMENT_ERROR);
        error_set_msg(f, "Code point out of range");
        return Ok(Value::Null);
    };
    let mut buf: [UChar; 32] = [0; 32];
    // SAFETY: a 32-unit buffer; with a failed `status` (an invalid form,
    // null `norm`) ICU returns 0 without touching it.
    let len = unsafe {
        sys::unorm2_getRawDecomposition(norm, c as i32, buf.as_mut_ptr(), 32, &mut status)
    };
    if len == -1 {
        return Ok(Value::Null);
    }
    // php converts without checking the result (its converter resets the
    // status first, so an invalid form gives "").
    Ok(match utf16_to_utf8(&buf[..len.clamp(0, 32) as usize]) {
        Ok(s) => Value::str(s),
        Err(_) => Value::Null,
    })
}
