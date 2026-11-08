//! `Transliterator` and `transliterator_*`, ported from php-src
//! ext/intl/transliterator/ onto ICU's `utrans`.

use std::cell::{Cell, RefCell};
use std::ptr;
use std::rc::Rc;

use rhp_values::{PhpArray, Value};

use super::common::{self, Call};
use super::error::{self, IntlError};
use super::sys::{self, UTransliterator};
use crate::zend::object_handlers::{self, CloneObj, ObjectHandlers};
use crate::{Host, Thrown};
use rhp_values::ObjectHandle;

pub(super) static HANDLERS: ObjectHandlers = ObjectHandlers {
    clone_obj: CloneObj::Custom(clone_obj),
    ..ObjectHandlers::STD
};

pub(super) const ALIASES: &[(&str, &str)] = &[
    ("transliterator_create", "create"),
    ("transliterator_create_from_rules", "createFromRules"),
    ("transliterator_list_ids", "listIDs"),
    ("transliterator_create_inverse", "createInverse"),
    ("transliterator_transliterate", "transliterate"),
    ("transliterator_get_error_code", "getErrorCode"),
    ("transliterator_get_error_message", "getErrorMessage"),
];

pub(super) const SPEC: common::ClassSpec = common::ClassSpec {
    name: "Transliterator",
    helper: "__translit",
    props: "    public readonly string $id;\n",
};

/// `Transliterator_object`.
#[derive(Default)]
pub(super) struct TransObj {
    err: RefCell<IntlError>,
    utrans: Cell<usize>,
}

impl TransObj {
    fn utrans(&self) -> *mut UTransliterator {
        self.utrans.get() as *mut UTransliterator
    }
}

impl Drop for TransObj {
    fn drop(&mut self) {
        if !self.utrans().is_null() {
            // SAFETY: the transliterator this object owns.
            unsafe { sys::utrans_close(self.utrans()) };
        }
    }
}

/// `Transliterator_clone_obj`.
fn clone_obj(_host: &mut dyn Host, src: &ObjectHandle, dst: &ObjectHandle) -> Result<(), Thrown> {
    let from = common::state::<TransObj>(src);
    let to = Rc::new(TransObj::default());
    object_handlers::set_payload(dst, to.clone());
    if from.utrans().is_null() {
        return Err(Thrown::error(
            "Error",
            "Cannot clone uninitialized Transliterator",
        ));
    }
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a live transliterator.
    let t = unsafe { sys::utrans_clone(from.utrans(), &mut status) };
    if sys::failure(status) {
        if !t.is_null() {
            // SAFETY: the clone ICU returned.
            unsafe { sys::utrans_close(t) };
        }
        return Err(Thrown::error("Error", "Failed to clone Transliterator"));
    }
    to.utrans.set(t as usize);
    Ok(())
}

/// `transliterator_object_construct`: a new object around `utrans`, its
/// `id` set. `None` with the status on a conversion failure.
fn construct(
    host: &mut dyn Host,
    utrans: *mut UTransliterator,
) -> Result<(Value, Rc<TransObj>, sys::UErrorCode), Thrown> {
    let mut len = 0i32;
    // SAFETY: a live transliterator; ICU returns its id.
    let id = unsafe { sys::utrans_getUnicodeID(utrans, &mut len) };
    // SAFETY: `len` units at `id`.
    let id = unsafe { std::slice::from_raw_parts(id, len.max(0) as usize) };
    let mut status = sys::U_ZERO_ERROR;
    let id8 = common::to_utf8(id, &mut status);
    let props = id8
        .map(|s| vec![(b"id".to_vec(), Value::str(s))])
        .unwrap_or_default();
    let obj = host
        .instantiate(b"Transliterator", props)
        .ok_or_else(|| Thrown::error("Error", "Class \"Transliterator\" not found"))?;
    let to = common::state_of::<TransObj>(&obj).expect("an object");
    to.utrans.set(utrans as usize);
    Ok((obj, to, status))
}

/// `create_transliterator`: `None` after a failure (the global error set).
fn create(
    host: &mut dyn Host,
    call: &Call,
    id: &[u8],
    direction: i64,
) -> Result<Option<Value>, Thrown> {
    error::error_reset();
    check_direction(call, direction)?;
    let f = &call.fname;
    let mut status = sys::U_ZERO_ERROR;
    let Some(uid) = common::to_utf16(id, &mut status) else {
        error::error_set_code(status);
        error::set_custom_msg(host, f, "String conversion of id to UTF-16 failed")?;
        return Ok(None);
    };
    let mut pe = sys::UParseError::default();
    // SAFETY: the id's units; no rules.
    let t = unsafe {
        sys::utrans_openU(
            uid.as_ptr(),
            uid.len() as i32,
            direction as i32,
            ptr::null(),
            -1,
            &mut pe,
            &mut status,
        )
    };
    if sys::failure(status) {
        error::error_set_code(status);
        let id = String::from_utf8_lossy(&super::c_string(id).into_bytes()).into_owned();
        error::set_custom_msg(
            host,
            f,
            &format!("unable to open ICU transliterator with id \"{id}\""),
        )?;
        return Ok(None);
    }
    let (obj, _, status) = construct(host, t)?;
    if sys::failure(status) {
        error::error_set_code(status);
        error::set_custom_msg(host, f, "internal constructor call failed")?;
        return Ok(None);
    }
    Ok(Some(obj))
}

fn check_direction(call: &Call, direction: i64) -> Result<(), Thrown> {
    if direction != 0 && direction != 1 {
        return Err(call.value_error(
            1,
            "must be either Transliterator::FORWARD or Transliterator::REVERSE",
        ));
    }
    Ok(())
}

/// `__translit(method, argc, $this, ...args)`.
pub(super) fn fn_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("Transliterator", args, ALIASES);
    let f = call.fname.clone();
    match call.name {
        "__construct" => Err(Thrown::error(
            "Exception",
            "An object of this type cannot be created with the new operator.",
        )),
        "create" => {
            let id = call.bytes(0);
            let dir = call.long(1, 0);
            Ok(create(host, &call, &id, dir)?.unwrap_or(Value::Null))
        }
        "createFromRules" => {
            error::error_reset();
            let rules = call.bytes(0);
            let dir = call.long(1, 0);
            check_direction(&call, dir)?;
            let tmp = TransObj::default();
            let mut status = sys::U_ZERO_ERROR;
            let Some(urules) = common::to_utf16(&rules, &mut status) else {
                error::errors_set_code(&tmp.err, status);
                error::errors_set_custom_msg(
                    host,
                    &tmp.err,
                    &f,
                    "String conversion of rules to UTF-16 failed",
                )?;
                return Ok(Value::Null);
            };
            let id: Vec<u16> = "RulesTransPHP".encode_utf16().collect();
            let mut pe = sys::UParseError::default();
            // SAFETY: the id's and the rules' units.
            let t = unsafe {
                sys::utrans_openU(
                    id.as_ptr(),
                    id.len() as i32,
                    dir as i32,
                    urules.as_ptr(),
                    urules.len() as i32,
                    &mut pe,
                    &mut status,
                )
            };
            error::error_set_code(status);
            if sys::failure(status) {
                tmp.err.borrow_mut().code = status;
                let msg = format!(
                    "unable to create ICU transliterator from rules ({})",
                    error::parse_error_to_string(&pe)
                );
                error::errors_set_custom_msg(host, &tmp.err, &f, &msg)?;
                return Ok(Value::Null);
            }
            let (obj, to, status) = construct(host, t)?;
            to.err.borrow_mut().code = status;
            if error::check_status(host, &to.err, &f, "internal constructor call failed")? {
                return Ok(Value::Null);
            }
            Ok(obj)
        }
        "listIDs" => {
            error::error_reset();
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: plain ICU call.
            let en = unsafe { sys::utrans_openIDs(&mut status) };
            if error::check_global(
                host,
                status,
                &f,
                "Failed to obtain registered transliterators",
            )? {
                return Ok(Value::Bool(false));
            }
            let mut out = PhpArray::new();
            loop {
                let mut len = 0i32;
                // SAFETY: a live enumeration.
                let p = unsafe { sys::uenum_unext(en, &mut len, &mut status) };
                if p.is_null() {
                    break;
                }
                // SAFETY: `len` units at `p`.
                let u = unsafe { std::slice::from_raw_parts(p, len.max(0) as usize) };
                match common::to_utf8(u, &mut status) {
                    Some(s) => out.push(Value::str(s)),
                    None => break,
                }
            }
            // SAFETY: the enumeration opened above.
            unsafe { sys::uenum_close(en) };
            error::error_set_code(status);
            if sys::failure(status) {
                error::set_custom_msg(
                    host,
                    &f,
                    "Failed to build array of registered transliterators",
                )?;
                return Ok(Value::Bool(false));
            }
            Ok(Value::Array(out))
        }
        _ => method(host, &call),
    }
}

fn method(host: &mut dyn Host, call: &Call) -> Result<Value, Thrown> {
    let f = call.fname.clone();
    if call.name == "transliterate" {
        return transliterate(host, call);
    }
    error::error_reset();
    let this = call.this.clone().expect("an object");
    let to = common::state_of::<TransObj>(&this).expect("an object");
    match call.name {
        "getErrorCode" => return Ok(Value::Int(to.err.borrow().code as i64)),
        "getErrorMessage" => return Ok(Value::str(to.err.borrow().message().into_bytes())),
        _ => {}
    }
    to.err.borrow_mut().reset();
    if to.utrans().is_null() {
        return Err(Thrown::error("Error", "Found unconstructed transliterator"));
    }
    match call.name {
        "createInverse" => {
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: a live transliterator.
            let t = unsafe { sys::utrans_openInverse(to.utrans(), &mut status) };
            let tmp = TransObj::default();
            tmp.err.borrow_mut().code = status;
            if error::check_status(
                host,
                &tmp.err,
                &f,
                "could not create inverse ICU transliterator",
            )? {
                return Ok(Value::Null);
            }
            let (obj, nto, status) = construct(host, t)?;
            nto.err.borrow_mut().code = status;
            if error::check_status(host, &nto.err, &f, "internal constructor call failed")? {
                return Ok(Value::Null);
            }
            Ok(obj)
        }
        other => unreachable!("Transliterator::{other}"),
    }
}

/// `transliterator_transliterate`.
fn transliterate(host: &mut dyn Host, call: &Call) -> Result<Value, Thrown> {
    error::error_reset();
    let f = call.fname.clone();
    let method = call.is_method();
    let (obj, s, start, limit) = if method {
        (
            call.this.clone().expect("$this"),
            call.bytes(0),
            call.long(1, 0),
            call.long(2, -1),
        )
    } else {
        // The procedural form takes a transliterator or an id.
        let first = call.this.clone().unwrap_or(Value::Null);
        let s = call.bytes(0);
        let start = call.long(1, 0);
        let limit = call.long(2, -1);
        let obj = match &first {
            Value::Str(id) => {
                let id = id.as_bytes().to_vec();
                let fake = Call {
                    fname: f.clone(),
                    name: "create",
                    shifted: false,
                    this: None,
                    argc: 1,
                    args: &[],
                };
                match create(host, &fake, &id, 0)? {
                    Some(o) => o,
                    None => {
                        if !host.has_pending_throw() {
                            let msg = error::error_message();
                            let id = String::from_utf8_lossy(&super::c_string(&id).into_bytes())
                                .into_owned();
                            host.emit_diagnostic(
                                crate::E_WARNING,
                                format!(
                                    "{f}(): Could not create transliterator with ID \"{id}\" ({msg})"
                                )
                                .as_bytes(),
                            )?;
                        }
                        return Ok(Value::Bool(false));
                    }
                }
            }
            v => v.clone(),
        };
        (obj, s, start, limit)
    };
    let (n_start, n_end) = if method { (1, 2) } else { (2, 3) };
    let arg_err = |n: usize, msg: &str| {
        let pname = if n == n_start { "start" } else { "end" };
        Thrown::error(
            "ValueError",
            format!("{f}(): Argument #{} (${pname}) {msg}", n + 1),
        )
    };
    if limit < -1 {
        return Err(arg_err(n_end, "must be greater than or equal to -1"));
    }
    if start < 0 {
        return Err(arg_err(n_start, "must be greater than or equal to 0"));
    }
    if limit != -1 && start > limit {
        return Err(arg_err(
            n_start,
            &format!(
                "must be less than or equal to argument #{} ($end)",
                n_end + 1
            ),
        ));
    }
    let to = common::state_of::<TransObj>(&obj).expect("an object");
    to.err.borrow_mut().reset();
    if to.utrans().is_null() {
        return Err(Thrown::error("Error", "Found unconstructed transliterator"));
    }
    let mut status = sys::U_ZERO_ERROR;
    let Some(u) = common::to_utf16(&s, &mut status) else {
        to.err.borrow_mut().code = status;
        error::check_status(
            host,
            &to.err,
            &f,
            "String conversion of string to UTF-16 failed",
        )?;
        return Ok(Value::Bool(false));
    };
    let ulen = u.len() as i64;
    if start > ulen || (limit != -1 && limit > ulen) {
        let msg = format!(
            "Neither \"start\" nor the \"end\" arguments can exceed the number of UTF-16 code units (in this case, {ulen})"
        );
        error::errors_set(host, &to.err, &f, sys::U_ILLEGAL_ARGUMENT_ERROR, &msg)?;
        return Ok(Value::Bool(false));
    }
    let mut cap = u.len() + 1;
    let out = loop {
        let mut buf: Vec<u16> = vec![0; cap];
        buf[..u.len()].copy_from_slice(&u);
        let mut len = u.len() as i32;
        let mut temp_limit = if limit == -1 { len } else { limit as i32 };
        let mut st = sys::U_ZERO_ERROR;
        // SAFETY: `buf` holds `cap` units, the text in its first `len`.
        unsafe {
            sys::utrans_transUChars(
                to.utrans(),
                buf.as_mut_ptr(),
                &mut len,
                cap as i32,
                start as i32,
                &mut temp_limit,
                &mut st,
            )
        };
        if st == sys::U_BUFFER_OVERFLOW_ERROR {
            cap = len as usize + 1;
            continue;
        }
        if sys::failure(st) {
            error::error_set_code(st);
            to.err.borrow_mut().code = st;
            error::errors_set_custom_msg(host, &to.err, &f, "transliteration failed")?;
            return Ok(Value::Bool(false));
        }
        buf.truncate(len as usize);
        break buf;
    };
    let mut status = sys::U_ZERO_ERROR;
    match common::to_utf8(&out, &mut status) {
        Some(s) => Ok(Value::str(s)),
        None => {
            to.err.borrow_mut().code = status;
            error::check_status(host, &to.err, &f, "Error converting value to UTF-8")?;
            Ok(Value::Bool(false))
        }
    }
}
