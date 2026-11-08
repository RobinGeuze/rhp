//! `IntlListFormatter`, ported from php-src
//! ext/intl/listformatter/listformatter_class.c onto ICU's `ulistfmt`.

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_void};

use rhp_values::Value;

use super::class_int;
use super::common::{self, Call};
use super::error::{self, IntlError};
use super::sys::{self, UChar, UErrorCode};
use crate::zend::object_handlers::ObjectHandlers;
use crate::{Host, Thrown};

pub(super) static HANDLERS: ObjectHandlers = ObjectHandlers::UNCLONEABLE;

pub(super) const SPEC: common::ClassSpec = common::ClassSpec {
    name: "IntlListFormatter",
    helper: "__listfmt",
    props: "",
};

/// `ListFormatter_object`.
#[derive(Default)]
pub(super) struct ListFmtObj {
    err: RefCell<IntlError>,
    fmt: Cell<usize>,
}

impl ListFmtObj {
    fn ptr(&self) -> *mut c_void {
        self.fmt.get() as *mut c_void
    }
}

impl Drop for ListFmtObj {
    fn drop(&mut self) {
        if !self.ptr().is_null() {
            // SAFETY: the formatter this object owns.
            unsafe { ulistfmt_close(self.ptr()) };
        }
    }
}

/// `intl_errors_reset(err)`: the object's and the global error.
fn errors_reset(err: &RefCell<IntlError>) {
    err.borrow_mut().reset();
    error::error_reset();
}

/// `__listfmt(method, argc, $this, ...args)`.
pub(super) fn fn_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("IntlListFormatter", args, &[]);
    let f = call.fname.clone();
    let this = call.this.clone().expect("$this");
    let lf = common::state_of::<ListFmtObj>(&this).expect("an object");
    match call.name {
        "__construct" => {
            errors_reset(&lf.err);
            construct(host, &call, &lf).map(|_| Value::Null)
        }
        "format" => {
            errors_reset(&lf.err);
            format(host, &call, &f, &lf)
        }
        "getErrorCode" => Ok(Value::Int(lf.err.borrow().code as i64)),
        "getErrorMessage" => Ok(Value::str(lf.err.borrow().message().into_bytes())),
        other => unreachable!("IntlListFormatter::{other}"),
    }
}

fn construct(host: &mut dyn Host, call: &Call, lf: &ListFmtObj) -> Result<(), Thrown> {
    let type_and = class_int("IntlListFormatter", "TYPE_AND");
    let width_wide = class_int("IntlListFormatter", "WIDTH_WIDE");
    let given = call.bytes(0);
    let ty = call.long(1, type_and);
    let width = call.long(2, width_wide);
    if !lf.ptr().is_null() {
        return Err(Thrown::error(
            "Error",
            "IntlListFormatter object is already constructed",
        ));
    }
    let locale = if given.is_empty() {
        super::locale_get_default()
    } else {
        given.clone()
    };
    let max = sys::ULOC_FULLNAME_CAPACITY - 1;
    if given.len() > max {
        return Err(call.value_error(
            0,
            &format!("must be less than or equal to {max} characters"),
        ));
    }
    let cloc = super::c_string(&locale);
    // SAFETY: a NUL-terminated locale; ICU returns a static string.
    if super::c_bytes(unsafe { sys::uloc_getISO3Language(cloc.as_ptr()) }).is_empty() {
        return Err(call.value_error(
            0,
            &format!("\"{}\" is invalid", String::from_utf8_lossy(&locale)),
        ));
    }
    let types = ["TYPE_AND", "TYPE_OR", "TYPE_UNITS"].map(|n| class_int("IntlListFormatter", n));
    if !types.contains(&ty) {
        return Err(call.value_error(
            1,
            "must be one of IntlListFormatter::TYPE_AND, IntlListFormatter::TYPE_OR, or IntlListFormatter::TYPE_UNITS",
        ));
    }
    let widths =
        ["WIDTH_WIDE", "WIDTH_SHORT", "WIDTH_NARROW"].map(|n| class_int("IntlListFormatter", n));
    if !widths.contains(&width) {
        return Err(call.value_error(
            2,
            "must be one of IntlListFormatter::WIDTH_WIDE, IntlListFormatter::WIDTH_SHORT, or IntlListFormatter::WIDTH_NARROW",
        ));
    }
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a NUL-terminated locale.
    let p = unsafe { ulistfmt_openForType(cloc.as_ptr(), ty as i32, width as i32, &mut status) };
    lf.fmt.set(p as usize);
    if sys::failure(status) {
        error::set(host, &call.fname, status, "Constructor failed")?;
        return Err(Thrown::error("IntlException", "Constructor failed"));
    }
    Ok(())
}

fn format(host: &mut dyn Host, call: &Call, f: &str, lf: &ListFmtObj) -> Result<Value, Thrown> {
    let Some(Value::Array(a)) = call.arg(0) else {
        return Ok(Value::Bool(false));
    };
    if a.is_empty() {
        return Ok(Value::str(Vec::new()));
    }
    if lf.ptr().is_null() {
        return Err(Thrown::error(
            "Error",
            "Found unconstructed IntlListFormatter",
        ));
    }
    let mut items: Vec<Vec<UChar>> = Vec::with_capacity(a.len());
    for v in a.values() {
        let s = crate::zend::zpp::zval_get_string(host, &crate::zend::zpp::deref(v))?;
        match super::utf8_to_utf16(&s) {
            Ok(u) => items.push(u),
            Err(st) => {
                error::errors_set(host, &lf.err, f, st, "Failed to convert string to UTF-16")?;
                return Ok(Value::Bool(false));
            }
        }
    }
    let ptrs: Vec<*const UChar> = items.iter().map(|u| u.as_ptr()).collect();
    let lens: Vec<i32> = items.iter().map(|u| u.len() as i32).collect();
    let n = items.len() as i32;
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: `n` items with their lengths; a pre-flight call.
    let len = unsafe {
        ulistfmt_format(
            lf.ptr(),
            ptrs.as_ptr(),
            lens.as_ptr(),
            n,
            std::ptr::null_mut(),
            0,
            &mut status,
        )
    };
    if sys::failure(status) && status != sys::U_BUFFER_OVERFLOW_ERROR {
        error::errors_set(host, &lf.err, f, status, "Failed to format list")?;
        return Ok(Value::Bool(false));
    }
    let mut out = vec![0 as UChar; len.max(0) as usize + 1];
    status = sys::U_ZERO_ERROR;
    // SAFETY: as above, into a buffer of `len` + 1 units.
    unsafe {
        ulistfmt_format(
            lf.ptr(),
            ptrs.as_ptr(),
            lens.as_ptr(),
            n,
            out.as_mut_ptr(),
            len,
            &mut status,
        )
    };
    if sys::failure(status) {
        error::errors_set(host, &lf.err, f, status, "Failed to format list")?;
        return Ok(Value::Bool(false));
    }
    match super::utf16_to_utf8(&out[..len.max(0) as usize]) {
        Ok(s) => Ok(Value::str(s)),
        Err(st) => {
            error::errors_set(host, &lf.err, f, st, "Failed to convert result to UTF-8")?;
            Ok(Value::Bool(false))
        }
    }
}

super::sys::icu! {
    pub fn ulistfmt_openForType(locale: *const c_char, ty: i32, width: i32, status: *mut UErrorCode) -> *mut c_void;
    pub fn ulistfmt_close(f: *mut c_void);
    pub fn ulistfmt_format(
        f: *const c_void, items: *const *const UChar, lens: *const i32, count: i32, out: *mut UChar,
        cap: i32, status: *mut UErrorCode,
    ) -> i32;
}
