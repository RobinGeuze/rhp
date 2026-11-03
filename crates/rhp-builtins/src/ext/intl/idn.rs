//! `idn_to_ascii` / `idn_to_utf8`, ported from php-src
//! `ext/intl/idn/idn.cpp` onto ICU's UTS #46 (`uidna`).

use rhp_values::{ArrayKey, PhpArray, Value};

use super::sys::{self, UErrorCode};
use super::{error_reset, error_set_code, error_set_msg, int_const};
use crate::ext::zpp::{int_arg_or, set_ref, str_arg, value_error};
use crate::{Host, Thrown};

/// `INTL_IDN_VARIANT_UTS46`.
const VARIANT_UTS46: i64 = 1;

/// `php_intl_idn_check_status`: always records the code; fails on an error.
fn check_status(fname: &str, status: UErrorCode, msg: &str) -> bool {
    error_set_code(status);
    if sys::failure(status) {
        error_set_msg(fname, msg);
        return false;
    }
    true
}

/// The owner of a `uidna_openUTS46` instance.
struct Uts46(*mut sys::UIDNA);

impl Drop for Uts46 {
    fn drop(&mut self) {
        // SAFETY: opened by uidna_openUTS46, closed once.
        unsafe { sys::uidna_close(self.0) };
    }
}

fn handoff(host: &mut dyn Host, args: &[Value], to_ascii: bool) -> Result<Value, Thrown> {
    let f = if to_ascii {
        "idn_to_ascii"
    } else {
        "idn_to_utf8"
    };
    error_reset();
    let domain = str_arg(host, args, 0, f, "domain")?;
    let option = int_arg_or(host, args, 1, f, "flags", int_const("IDNA_DEFAULT"))?;
    let variant = int_arg_or(host, args, 2, f, "variant", VARIANT_UTS46)?;
    if domain.is_empty() {
        return Err(value_error(f, 1, "domain", "must not be empty"));
    }
    if domain.len() > i32::MAX as usize - 1 {
        // php's format string says "d" where it means PRId32.
        return Err(value_error(f, 1, "domain", "must be less than d bytes"));
    }
    if variant != VARIANT_UTS46 {
        // php names argument #2 here.
        return Err(value_error(
            f,
            2,
            "flags",
            "must be INTL_IDNA_VARIANT_UTS46",
        ));
    }
    let want_info = args.len() > 3;
    if want_info {
        set_ref(args, 3, Value::Array(PhpArray::new()));
    }

    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: plain open; php passes the flags as uint32_t.
    let uts46 = Uts46(unsafe { sys::uidna_openUTS46(option as u32, &mut status) });
    if !check_status(f, status, "failed to open UIDNA instance") {
        return Ok(Value::Bool(false));
    }
    let capac: i32 = if to_ascii { 255 } else { 252 * 4 };
    let mut buf = vec![0u8; capac as usize];
    let mut info = sys::UIDNAInfo::new();
    // SAFETY: `buf` holds `capac` bytes; the domain is passed with its length.
    let len = unsafe {
        let convert = if to_ascii {
            sys::uidna_nameToASCII_UTF8
        } else {
            sys::uidna_nameToUnicodeUTF8
        };
        convert(
            uts46.0,
            domain.as_ptr().cast(),
            domain.len() as i32,
            buf.as_mut_ptr().cast(),
            capac,
            &mut info,
            &mut status,
        )
    };
    if len >= capac || !check_status(f, status, "failed to convert name") {
        return Ok(Value::Bool(false));
    }
    buf.truncate(len.max(0) as usize);
    if want_info {
        let mut a = PhpArray::new();
        a.insert(ArrayKey::from_bytes(b"result"), Value::str(buf.clone()));
        a.insert(
            ArrayKey::from_bytes(b"isTransitionalDifferent"),
            Value::Bool(info.is_transitional_different != 0),
        );
        a.insert(
            ArrayKey::from_bytes(b"errors"),
            Value::Int(info.errors as i64),
        );
        set_ref(args, 3, Value::Array(a));
    }
    Ok(if info.errors == 0 {
        Value::str(buf)
    } else {
        Value::Bool(false)
    })
}

pub(super) fn fn_idn_to_ascii(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    handoff(host, args, true)
}

pub(super) fn fn_idn_to_utf8(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    handoff(host, args, false)
}
