//! `Spoofchecker`, ported from php-src ext/intl/spoofchecker/ onto ICU's
//! `uspoof`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rhp_values::Value;

use super::common::{self, Call};
use super::error::{self, IntlError};
use super::sys::{self, USpoofCheckResult, USpoofChecker};
use super::{class_int, u_error_name};
use crate::zend::object_handlers::{self, CloneObj, ObjectHandlers};
use crate::{Host, Thrown};
use rhp_values::ObjectHandle;

pub(super) static HANDLERS: ObjectHandlers = ObjectHandlers {
    clone_obj: CloneObj::Custom(clone_obj),
    ..ObjectHandlers::STD
};

pub(super) const SPEC: common::ClassSpec = common::ClassSpec {
    name: "Spoofchecker",
    helper: "__spoof",
    props: "",
};

/// `Spoofchecker_object`.
#[derive(Default)]
pub(super) struct SpoofObj {
    err: RefCell<IntlError>,
    uspoof: Cell<usize>,
    res: Cell<usize>,
}

impl SpoofObj {
    fn uspoof(&self) -> *mut USpoofChecker {
        self.uspoof.get() as *mut USpoofChecker
    }
    fn res(&self) -> *mut USpoofCheckResult {
        self.res.get() as *mut USpoofCheckResult
    }
}

impl Drop for SpoofObj {
    fn drop(&mut self) {
        // SAFETY: what this object owns.
        unsafe {
            if !self.uspoof().is_null() {
                sys::uspoof_close(self.uspoof());
            }
            if !self.res().is_null() {
                sys::uspoof_closeCheckResult(self.res());
            }
        }
    }
}

/// `spoofchecker_clone_obj`.
fn clone_obj(_host: &mut dyn Host, src: &ObjectHandle, dst: &ObjectHandle) -> Result<(), Thrown> {
    let from = common::state::<SpoofObj>(src);
    let to = Rc::new(SpoofObj::default());
    object_handlers::set_payload(dst, to.clone());
    if from.uspoof().is_null() {
        return Err(Thrown::error(
            "Error",
            "Cannot clone uninitialized SpoofChecker",
        ));
    }
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a live checker.
    let c = unsafe { sys::uspoof_clone(from.uspoof(), &mut status) };
    if sys::failure(status) {
        return Err(Thrown::error("Error", "Failed to clone SpoofChecker"));
    }
    to.uspoof.set(c as usize);
    Ok(())
}

fn warn_status(host: &mut dyn Host, f: &str, code: sys::UErrorCode) -> Result<(), Thrown> {
    host.emit_diagnostic(
        crate::E_WARNING,
        format!("{f}(): ({code}) {}", u_error_name(code)).as_bytes(),
    )
}

/// `__spoof(method, argc, $this, ...args)`.
pub(super) fn fn_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("Spoofchecker", args, &[]);
    error::error_reset();
    let this = call.this.clone().expect("$this");
    let co = common::state_of::<SpoofObj>(&this).expect("an object");
    co.err.borrow_mut().reset();
    if call.name == "__construct" {
        if !co.uspoof().is_null() {
            return Err(Thrown::error(
                "Error",
                "Spoofchecker object is already constructed",
            ));
        }
        let mut status = sys::U_ZERO_ERROR;
        // SAFETY: plain ICU calls on the new checker.
        unsafe {
            let c = sys::uspoof_open(&mut status);
            co.uspoof.set(c as usize);
            if sys::failure(status) {
                co.err.borrow_mut().code = status;
                return Err(Thrown::error(
                    "IntlException",
                    "Spoofchecker::__construct(): unable to open ICU Spoof Checker",
                ));
            }
            sys::uspoof_setRestrictionLevel(
                c,
                class_int("Spoofchecker", "HIGHLY_RESTRICTIVE") as i32,
            );
            let r = sys::uspoof_openCheckResult(&mut status);
            co.res.set(r as usize);
        }
        co.err.borrow_mut().code = status;
        return Ok(Value::Null);
    }
    if co.uspoof().is_null() {
        return Err(Thrown::error("Error", "Found unconstructed Spoofchecker"));
    }
    let c = co.uspoof();
    let mut status = sys::U_ZERO_ERROR;
    match call.name {
        "isSuspicious" => {
            let text = call.bytes(0);
            // SAFETY: a live checker and result; the text's bytes.
            let ret = unsafe {
                sys::uspoof_check2UTF8(
                    c,
                    text.as_ptr().cast(),
                    text.len() as i32,
                    co.res(),
                    &mut status,
                )
            };
            if sys::failure(status) {
                co.err.borrow_mut().code = status;
                warn_status(host, &call.fname, status)?;
                // SAFETY: the live result.
                let errmask = unsafe { sys::uspoof_getCheckResultChecks(co.res(), &mut status) };
                if errmask != ret {
                    host.emit_diagnostic(
                        crate::E_WARNING,
                        format!(
                            "{}(): unexpected error ({ret}), does not relate to the flags passed to setChecks ({errmask})",
                            call.fname
                        )
                        .as_bytes(),
                    )?;
                }
                return Ok(Value::Bool(true));
            }
            if let Some(r) = call.reference(1) {
                common::assign_ref(host, &r, Value::Int(ret as i64))?;
            }
            Ok(Value::Bool(ret != 0))
        }
        "areConfusable" => {
            let s1 = call.bytes(0);
            let s2 = call.bytes(1);
            let mut ret = 0;
            if s1.len() > i32::MAX as usize || s2.len() > i32::MAX as usize {
                status = sys::U_BUFFER_OVERFLOW_ERROR;
            } else {
                // SAFETY: a live checker; both strings' bytes.
                ret = unsafe {
                    sys::uspoof_areConfusableUTF8(
                        c,
                        s1.as_ptr().cast(),
                        s1.len() as i32,
                        s2.as_ptr().cast(),
                        s2.len() as i32,
                        &mut status,
                    )
                };
            }
            if sys::failure(status) {
                co.err.borrow_mut().code = status;
                warn_status(host, &call.fname, status)?;
                return Ok(Value::Bool(true));
            }
            if let Some(r) = call.reference(2) {
                common::assign_ref(host, &r, Value::Int(ret as i64))?;
            }
            Ok(Value::Bool(ret != 0))
        }
        "setAllowedLocales" => {
            let locales = super::c_string(&call.bytes(0));
            // SAFETY: a live checker, a C string.
            unsafe { sys::uspoof_setAllowedLocales(c, locales.as_ptr(), &mut status) };
            if sys::failure(status) {
                co.err.borrow_mut().code = status;
                warn_status(host, &call.fname, status)?;
            }
            Ok(Value::Null)
        }
        "setChecks" => {
            let checks = call.long(0, 0);
            // SAFETY: a live checker.
            unsafe { sys::uspoof_setChecks(c, checks as i32, &mut status) };
            if sys::failure(status) {
                co.err.borrow_mut().code = status;
                warn_status(host, &call.fname, status)?;
            }
            Ok(Value::Null)
        }
        "setRestrictionLevel" => {
            let level = call.long(0, 0);
            let ok = [
                "ASCII",
                "SINGLE_SCRIPT_RESTRICTIVE",
                "HIGHLY_RESTRICTIVE",
                "MODERATELY_RESTRICTIVE",
                "MINIMALLY_RESTRICTIVE",
                "UNRESTRICTIVE",
            ]
            .iter()
            .any(|n| class_int("Spoofchecker", n) == level);
            if !ok {
                return Err(call.value_error(
                    0,
                    "must be one of Spoofchecker::ASCII, Spoofchecker::SINGLE_SCRIPT_RESTRICTIVE, \
                     Spoofchecker::HIGHLY_RESTRICTIVE, Spoofchecker::MODERATELY_RESTRICTIVE, \
                     Spoofchecker::MINIMALLY_RESTRICTIVE, or Spoofchecker::UNRESTRICTIVE",
                ));
            }
            // SAFETY: a live checker.
            unsafe { sys::uspoof_setRestrictionLevel(c, level as i32) };
            Ok(Value::Null)
        }
        "setAllowedChars" => {
            let pattern = call.bytes(0);
            let option = call.long(1, 0);
            if pattern.len() > i32::MAX as usize {
                return Err(call.value_error(
                    0,
                    &format!("must be less than or equal to {} bytes long", i32::MAX),
                ));
            }
            if pattern.first() != Some(&b'[') || pattern.last() != Some(&b']') {
                return Err(call.value_error(
                    0,
                    "must be a valid regular expression character set pattern",
                ));
            }
            let Some(up) = common::to_utf16(&pattern, &mut status) else {
                co.err.borrow_mut().code = status;
                return Err(call.value_error(
                    0,
                    &format!(
                        "string conversion to unicode encoding failed ({status}) {}",
                        u_error_name(status)
                    ),
                ));
            };
            let ignore = class_int("Spoofchecker", "IGNORE_SPACE");
            let ci = class_int("Spoofchecker", "CASE_INSENSITIVE");
            let acm = class_int("Spoofchecker", "ADD_CASE_MAPPINGS");
            let sci = class_int("Spoofchecker", "SIMPLE_CASE_INSENSITIVE");
            if option != 0
                && option != ignore
                && option != (ignore | sci)
                && option != (ignore | ci)
                && option != (ignore | acm)
            {
                return Err(call.value_error(
                    1,
                    "must be a valid pattern option, 0 or (SpoofChecker::IGNORE_SPACE|(<none> or \
                     SpoofChecker::CASE_INSENSITIVE or SpoofChecker::ADD_CASE_MAPPINGS or \
                     SpoofChecker::SIMPLE_CASE_INSENSITIVE))",
                ));
            }
            // SAFETY: a fresh set, the pattern's units, a live checker.
            unsafe {
                let set = sys::uset_openEmpty();
                sys::uset_applyPattern(
                    set,
                    up.as_ptr(),
                    up.len() as i32,
                    option as u32,
                    &mut status,
                );
                if sys::failure(status) {
                    sys::uset_close(set);
                    co.err.borrow_mut().code = status;
                    return Err(call.value_error(
                        0,
                        &format!(
                            "must be a valid regular expression character set pattern ({status}) {}",
                            u_error_name(status)
                        ),
                    ));
                }
                sys::uset_compact(set);
                sys::uspoof_setAllowedChars(c, set, &mut status);
                sys::uset_close(set);
            }
            if sys::failure(status) {
                co.err.borrow_mut().code = status;
                warn_status(host, &call.fname, status)?;
            }
            Ok(Value::Null)
        }
        other => unreachable!("Spoofchecker::{other}"),
    }
}
