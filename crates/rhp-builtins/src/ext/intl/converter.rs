//! `UConverter`, ported from php-src ext/intl/converter/converter.c onto
//! ICU's `ucnv`.
//!
//! A subclass's `toUCallback()`/`fromUCallback()` run from inside ICU's
//! conversion calls. The object's [`Ctx`] (boxed, so ICU can hold its
//! address) carries the host while rhp is in such a call; outside one (a
//! converter closed when the object is freed without a host) the
//! callbacks do nothing.

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_void, CString};
use std::rc::{Rc, Weak};

use rhp_values::{ObjectData, ObjectHandle, PhpArray, Value};

use super::common::{self, Call};
use super::error::{self, IntlError};
use super::sys::{self, UBool, UChar, UChar32, UErrorCode};
use crate::zend::object_handlers::{self, CloneObj, ObjectHandlers};
use crate::zend::zpp::deref;
use crate::{Host, Thrown};

pub(super) static HANDLERS: ObjectHandlers = ObjectHandlers {
    clone_obj: CloneObj::Custom(clone_obj),
    free_obj_host: Some(free_obj),
    ..ObjectHandlers::STD
};

pub(super) const SPEC: common::ClassSpec = common::ClassSpec {
    name: "UConverter",
    helper: "__uconv",
    props: "",
};

/// `UConverterCallbackReason`.
const UCNV_UNASSIGNED: i32 = 0;
const UCNV_ILLEGAL: i32 = 1;
const UCNV_IRREGULAR: i32 = 2;
const UCNV_RESET: i32 = 3;
const UCNV_CLOSE: i32 = 4;
const UCNV_CLONE: i32 = 5;
/// `U_AMBIGUOUS_ALIAS_WARNING`.
const U_AMBIGUOUS_ALIAS_WARNING: UErrorCode = -122;

type HostPtr = *mut (dyn Host + 'static);

/// What a callback needs: the host during a call into ICU, the object,
/// its error, and the running method's name for messages.
#[derive(Default)]
struct Ctx {
    host: Cell<Option<HostPtr>>,
    obj: RefCell<Weak<RefCell<ObjectData>>>,
    err: RefCell<IntlError>,
    fname: RefCell<String>,
    thrown: RefCell<Option<Thrown>>,
}

/// `php_converter_object`.
#[derive(Default)]
pub(super) struct ConvObj {
    src: Cell<usize>,
    dest: Cell<usize>,
    ctx: Box<Ctx>,
}

impl ConvObj {
    fn src(&self) -> *mut c_void {
        self.src.get() as *mut c_void
    }
    fn dest(&self) -> *mut c_void {
        self.dest.get() as *mut c_void
    }
    fn ctx_ptr(&self) -> *const c_void {
        &*self.ctx as *const Ctx as *const c_void
    }

    /// Runs `f` (ICU calls that may call back) with the host available to
    /// the callbacks; an exception a callback threw comes back after.
    fn with_host<R>(
        &self,
        host: &mut dyn Host,
        fname: &str,
        f: impl FnOnce() -> R,
    ) -> Result<R, Thrown> {
        // SAFETY: the pointer is only used while `f` runs, during which
        // the caller does not touch `host`.
        let p: HostPtr = unsafe { std::mem::transmute::<*mut (dyn Host + '_), HostPtr>(host) };
        let old_host = self.ctx.host.replace(Some(p));
        let old_name = self.ctx.fname.replace(fname.to_string());
        let r = f();
        self.ctx.host.set(old_host);
        *self.ctx.fname.borrow_mut() = old_name;
        match self.ctx.thrown.borrow_mut().take() {
            Some(t) => Err(t),
            None => Ok(r),
        }
    }
}

impl Drop for ConvObj {
    fn drop(&mut self) {
        // SAFETY: the converters this object owns; with no host the
        // callbacks ICU makes while closing do nothing.
        unsafe {
            if !self.src().is_null() {
                ucnv_close(self.src());
            }
            if !self.dest().is_null() {
                ucnv_close(self.dest());
            }
        }
    }
}

/// `php_converter_throw_failure`: `intl_errors_set` on the object's error
/// (or only the global one).
fn throw_failure(
    host: &mut dyn Host,
    ctx: Option<&Ctx>,
    fname: &str,
    code: UErrorCode,
    msg: &str,
) -> Result<(), Thrown> {
    match ctx {
        Some(c) => error::errors_set(host, &c.err, fname, code, msg),
        None => error::set(host, fname, code, msg),
    }
}

fn ufailure_msg(code: UErrorCode) -> String {
    format!("returned error {code}: {}", super::u_error_name(code))
}

#[repr(C)]
pub(super) struct ToUArgs {
    size: u16,
    flush: UBool,
    converter: *mut c_void,
    source: *const c_char,
    source_limit: *const c_char,
    target: *mut UChar,
    target_limit: *const UChar,
    offsets: *mut i32,
}

#[repr(C)]
pub(super) struct FromUArgs {
    size: u16,
    flush: UBool,
    converter: *mut c_void,
    source: *const UChar,
    source_limit: *const UChar,
    target: *mut c_char,
    target_limit: *const c_char,
    offsets: *mut i32,
}

/// A callback's view of its context: the object, the host and the name.
fn callback_env(ctx: &Ctx) -> Option<(Value, HostPtr, String)> {
    let hp = ctx.host.get()?;
    if ctx.thrown.borrow().is_some() {
        // php's zend_call_function skips the call while an exception is out
        return None;
    }
    let obj = ctx.obj.borrow().upgrade()?;
    Some((Value::Object(obj), hp, ctx.fname.borrow().clone()))
}

/// Calls the object's `method` with php's four arguments, the last a
/// reference to the error code; the return value, and the code written
/// back.
fn call_user(
    host: &mut dyn Host,
    obj: &Value,
    method: &str,
    args: [Value; 3],
    code: &mut UErrorCode,
) -> Result<Value, Thrown> {
    let err = Rc::new(RefCell::new(Value::Int(*code as i64)));
    let mut cb = PhpArray::new();
    cb.push(obj.clone());
    cb.push(Value::str(method.as_bytes().to_vec()));
    let [a, b, c] = args;
    let r = host.call_callable(&Value::Array(cb), &[a, b, c, Value::Ref(err.clone())]);
    if let Value::Int(n) = &*err.borrow() {
        *code = *n as UErrorCode;
    }
    r
}

/// `TARGET_CHECK`.
fn target_check(
    host: &mut dyn Host,
    ctx: &Ctx,
    fname: &str,
    available: isize,
    needed: isize,
) -> Result<bool, Thrown> {
    if available < needed {
        throw_failure(
            host,
            Some(ctx),
            fname,
            sys::U_BUFFER_OVERFLOW_ERROR,
            &format!("Buffer overrun {needed} bytes needed, {available} available"),
        )?;
        return Ok(false);
    }
    Ok(true)
}

/// `php_converter_append_toUnicode_target`.
fn append_to_unicode(
    host: &mut dyn Host,
    ctx: &Ctx,
    fname: &str,
    v: &Value,
    args: &mut ToUArgs,
) -> Result<(), Thrown> {
    let avail = |a: &ToUArgs| (a.target_limit as isize - a.target as isize) / 2;
    match v {
        Value::Null => {}
        Value::Int(l) => {
            let l = *l;
            if !(0..=0x10ffff).contains(&l) {
                return throw_failure(
                    host,
                    Some(ctx),
                    fname,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    &format!("Invalid codepoint U+{l:04x}"),
                );
            }
            if l > 0xffff {
                if target_check(host, ctx, fname, avail(args), 2)? {
                    // SAFETY: two units checked free.
                    unsafe {
                        *args.target = (((l - 0x10000) >> 10) | 0xd800) as UChar;
                        *args.target.add(1) = (((l - 0x10000) & 0x3ff) | 0xdc00) as UChar;
                        args.target = args.target.add(2);
                    }
                }
                return Ok(());
            }
            if target_check(host, ctx, fname, avail(args), 1)? {
                // SAFETY: one unit checked free.
                unsafe {
                    *args.target = l as UChar;
                    args.target = args.target.add(1);
                }
            }
        }
        Value::Str(s) => {
            let b = s.as_bytes();
            let mut i = 0i32;
            while i as usize != b.len() && target_check(host, ctx, fname, avail(args), 1)? {
                let mut c = 0;
                // SAFETY: `b.len()` bytes.
                i = unsafe {
                    super::uchar::rhp_u8_next_from(b.as_ptr(), i, b.len() as i32, &mut c)
                };
                // SAFETY: one unit checked free (php stores the UChar32 in a UChar).
                unsafe {
                    *args.target = c as UChar;
                    args.target = args.target.add(1);
                }
            }
        }
        Value::Array(a) => {
            for v in a.values() {
                append_to_unicode(host, ctx, fname, v, args)?;
            }
        }
        _ => {
            return throw_failure(
                host,
                Some(ctx),
                fname,
                sys::U_ILLEGAL_ARGUMENT_ERROR,
                "toUCallback() specified illegal type for substitution character",
            )
        }
    }
    Ok(())
}

/// `php_converter_append_fromUnicode_target`.
fn append_from_unicode(
    host: &mut dyn Host,
    ctx: &Ctx,
    fname: &str,
    v: &Value,
    args: &mut FromUArgs,
) -> Result<(), Thrown> {
    let avail = |a: &FromUArgs| a.target_limit as isize - a.target as isize;
    match v {
        Value::Null => {}
        Value::Int(l) => {
            if target_check(host, ctx, fname, avail(args), 1)? {
                // SAFETY: one byte checked free.
                unsafe {
                    *args.target = *l as c_char;
                    args.target = args.target.add(1);
                }
            }
        }
        Value::Str(s) => {
            let b = s.as_bytes();
            if target_check(host, ctx, fname, avail(args), b.len() as isize)? {
                // SAFETY: `b.len()` bytes checked free.
                unsafe {
                    std::ptr::copy_nonoverlapping(b.as_ptr(), args.target as *mut u8, b.len());
                    args.target = args.target.add(b.len());
                }
            }
        }
        Value::Array(a) => {
            for v in a.values() {
                append_from_unicode(host, ctx, fname, v, args)?;
            }
        }
        _ => {
            return throw_failure(
                host,
                Some(ctx),
                fname,
                sys::U_ILLEGAL_ARGUMENT_ERROR,
                "fromUCallback() specified illegal type for substitution character",
            )
        }
    }
    Ok(())
}

/// `php_converter_to_u_callback`.
extern "C" fn to_u_callback(
    context: *const c_void,
    args: *mut ToUArgs,
    code_units: *const c_char,
    length: i32,
    reason: i32,
    code: *mut UErrorCode,
) {
    // SAFETY: the object's Ctx, alive while its converters are.
    let ctx = unsafe { &*(context as *const Ctx) };
    let Some((obj, hp, fname)) = callback_env(ctx) else {
        return;
    };
    // SAFETY: set by `with_host` for the duration of the ICU call.
    let host = unsafe { &mut *hp };
    // SAFETY: ICU's arguments for this call.
    let (args, code) = unsafe { (&mut *args, &mut *code) };
    let bytes = |p: *const c_char, n: isize| -> Value {
        if p.is_null() || n <= 0 {
            Value::str(Vec::new())
        } else {
            // SAFETY: `n` bytes ICU handed over.
            Value::str(unsafe { std::slice::from_raw_parts(p as *const u8, n as usize) }.to_vec())
        }
    };
    let source = bytes(
        args.source,
        args.source_limit as isize - args.source as isize,
    );
    let units = bytes(code_units, length as isize);
    let r = call_user(
        host,
        &obj,
        "toUCallback",
        [Value::Int(reason as i64), source, units],
        code,
    )
    .and_then(|v| append_to_unicode(host, ctx, &fname, &deref(&v), args));
    if let Err(t) = r {
        *ctx.thrown.borrow_mut() = Some(t);
    }
}

/// `php_converter_from_u_callback`.
extern "C" fn from_u_callback(
    context: *const c_void,
    args: *mut FromUArgs,
    code_units: *const UChar,
    length: i32,
    code_point: UChar32,
    reason: i32,
    code: *mut UErrorCode,
) {
    // SAFETY: as above.
    let ctx = unsafe { &*(context as *const Ctx) };
    let Some((obj, hp, fname)) = callback_env(ctx) else {
        return;
    };
    // SAFETY: set by `with_host` for the duration of the ICU call.
    let host = unsafe { &mut *hp };
    // SAFETY: ICU's arguments for this call.
    let (args, code) = unsafe { (&mut *args, &mut *code) };
    let units: &[UChar] = if code_units.is_null() || length <= 0 {
        &[]
    } else {
        // SAFETY: `length` units ICU handed over.
        unsafe { std::slice::from_raw_parts(code_units, length as usize) }
    };
    // U16_NEXT: pairs joined, lone surrogates as they are
    let mut cps = PhpArray::new();
    let mut i = 0;
    while i < units.len() {
        let c = units[i] as u32;
        i += 1;
        if (0xd800..0xdc00).contains(&c)
            && i < units.len()
            && (0xdc00..0xe000).contains(&(units[i] as u32))
        {
            let lo = units[i] as u32;
            i += 1;
            cps.push(Value::Int(
                (((c - 0xd800) << 10) + (lo - 0xdc00) + 0x10000) as i64,
            ));
        } else {
            cps.push(Value::Int(c as i64));
        }
    }
    let r = call_user(
        host,
        &obj,
        "fromUCallback",
        [
            Value::Int(reason as i64),
            Value::Array(cps),
            Value::Int(code_point as i64),
        ],
        code,
    )
    .and_then(|v| append_from_unicode(host, ctx, &fname, &deref(&v), args));
    if let Err(t) = r {
        *ctx.thrown.borrow_mut() = Some(t);
    }
}

/// `php_converter_set_callbacks`: only a subclass's converters call back.
fn set_callbacks(
    host: &mut dyn Host,
    co: &ConvObj,
    this: &Value,
    cnv: *mut c_void,
    fname: &str,
) -> Result<bool, Thrown> {
    if host.object_class_name(this).as_deref() == Some(b"UConverter".as_slice()) {
        return Ok(true);
    }
    let mut ok = true;
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a live converter; the context outlives it.
    unsafe {
        ucnv_setToUCallBack(
            cnv,
            to_u_callback,
            co.ctx_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut status,
        )
    };
    if sys::failure(status) {
        throw_failure(host, Some(&co.ctx), fname, status, &ufailure_msg(status))?;
        ok = false;
    }
    status = sys::U_ZERO_ERROR;
    // SAFETY: as above.
    unsafe {
        ucnv_setFromUCallBack(
            cnv,
            from_u_callback,
            co.ctx_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut status,
        )
    };
    if sys::failure(status) {
        throw_failure(host, Some(&co.ctx), fname, status, &ufailure_msg(status))?;
        ok = false;
    }
    Ok(ok)
}

/// `php_converter_set_encoding`: a new converter for `enc` (NULL: ICU's
/// default), or `None` after reporting the failure.
fn open_converter(
    host: &mut dyn Host,
    obj: Option<(&ConvObj, &Value)>,
    enc: Option<&[u8]>,
    fname: &str,
) -> Result<Option<*mut c_void>, Thrown> {
    let c = enc.map(|e| {
        let end = e.iter().position(|&b| b == 0).unwrap_or(e.len());
        CString::new(&e[..end]).unwrap_or_default()
    });
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a NUL-terminated name or NULL.
    let cnv = unsafe {
        ucnv_open(
            c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
            &mut status,
        )
    };
    if status == U_AMBIGUOUS_ALIAS_WARNING {
        let mut st = sys::U_ZERO_ERROR;
        // SAFETY: the converter just opened.
        let name = unsafe { ucnv_getName(cnv, &mut st) };
        let actual = if sys::failure(st) {
            b"(unknown)".to_vec()
        } else {
            super::c_bytes(name)
        };
        let msg = format!(
            "Ambiguous encoding specified, using {}",
            String::from_utf8_lossy(&actual)
        );
        error::set(host, fname, status, &msg)?;
    } else if sys::failure(status) {
        match obj {
            Some((co, _)) => {
                throw_failure(host, Some(&co.ctx), fname, status, &ufailure_msg(status))?
            }
            None => {
                let msg = format!(
                    "Error setting encoding: {status} - {}",
                    super::u_error_name(status)
                );
                error::set(host, fname, status, &msg)?;
            }
        }
        return Ok(None);
    }
    if let Some((co, this)) = obj {
        if !set_callbacks(host, co, this, cnv, fname)? {
            // SAFETY: not kept.
            unsafe { ucnv_close(cnv) };
            return Ok(None);
        }
    }
    Ok(Some(cnv))
}

/// Replaces `slot`'s converter (closing the old one, which may call back).
fn set_encoding(
    host: &mut dyn Host,
    co: &ConvObj,
    this: &Value,
    slot: &Cell<usize>,
    enc: Option<&[u8]>,
    fname: &str,
) -> Result<bool, Thrown> {
    let Some(cnv) = open_converter(host, Some((co, this)), enc, fname)? else {
        return Ok(false);
    };
    let old = slot.replace(cnv as usize) as *mut c_void;
    if !old.is_null() {
        // SAFETY: the converter replaced.
        co.with_host(host, fname, || unsafe { ucnv_close(old) })?;
    }
    Ok(true)
}

/// `php_converter_do_convert`.
fn do_convert(
    host: &mut dyn Host,
    co: Option<&ConvObj>,
    dest: *mut c_void,
    src: *mut c_void,
    s: &[u8],
    fname: &str,
) -> Result<Option<Vec<u8>>, Thrown> {
    let ctx = co.map(|c| &*c.ctx);
    if src.is_null() || dest.is_null() {
        throw_failure(
            host,
            ctx,
            fname,
            sys::U_INVALID_STATE_ERROR,
            "Internal converters not initialized",
        )?;
        return Ok(None);
    }
    let run = |host: &mut dyn Host, f: &mut dyn FnMut() -> i32| -> Result<i32, Thrown> {
        match co {
            Some(c) => c.with_host(host, fname, f),
            None => Ok(f()),
        }
    };
    let mut error = sys::U_ZERO_ERROR;
    // SAFETY (below): live converters and buffers of the given sizes.
    let mut temp_len = 1 + run(host, &mut || unsafe {
        ucnv_toUChars(
            src,
            std::ptr::null_mut(),
            0,
            s.as_ptr().cast(),
            s.len() as i32,
            &mut error,
        )
    })?;
    if sys::failure(error) && error != sys::U_BUFFER_OVERFLOW_ERROR {
        throw_failure(host, ctx, fname, error, &ufailure_msg(error))?;
        return Ok(None);
    }
    let mut temp = vec![0 as UChar; temp_len.max(1) as usize];
    error = sys::U_ZERO_ERROR;
    temp_len = run(host, &mut || unsafe {
        ucnv_toUChars(
            src,
            temp.as_mut_ptr(),
            temp_len,
            s.as_ptr().cast(),
            s.len() as i32,
            &mut error,
        )
    })?;
    if sys::failure(error) {
        throw_failure(host, ctx, fname, error, &ufailure_msg(error))?;
        return Ok(None);
    }
    let temp_len = temp_len.max(0);
    let ret_len = run(host, &mut || unsafe {
        ucnv_fromUChars(
            dest,
            std::ptr::null_mut(),
            0,
            temp.as_ptr(),
            temp_len,
            &mut error,
        )
    })?;
    if sys::failure(error) && error != sys::U_BUFFER_OVERFLOW_ERROR {
        throw_failure(host, ctx, fname, error, &ufailure_msg(error))?;
        return Ok(None);
    }
    let mut out = vec![0u8; ret_len.max(0) as usize + 1];
    error = sys::U_ZERO_ERROR;
    let n = run(host, &mut || unsafe {
        ucnv_fromUChars(
            dest,
            out.as_mut_ptr().cast(),
            ret_len + 1,
            temp.as_ptr(),
            temp_len,
            &mut error,
        )
    })?;
    if sys::failure(error) {
        throw_failure(host, ctx, fname, error, &ufailure_msg(error))?;
        return Ok(None);
    }
    out.truncate(n.max(0) as usize);
    Ok(Some(out))
}

/// `ucnv_getSubstChars`: up to 127 bytes.
fn subst_chars(cnv: *mut c_void, status: &mut UErrorCode) -> Vec<u8> {
    let mut buf = [0 as c_char; 127];
    let mut len: i8 = 127;
    // SAFETY: a live converter, a 127-byte buffer.
    unsafe { ucnv_getSubstChars(cnv, buf.as_mut_ptr(), &mut len, status) };
    buf[..len.max(0) as usize]
        .iter()
        .map(|&c| c as u8)
        .collect()
}

/// `php_converter_clone_object`.
fn clone_obj(host: &mut dyn Host, src: &ObjectHandle, dst: &ObjectHandle) -> Result<(), Thrown> {
    let from = common::state::<ConvObj>(src);
    let to = Rc::new(ConvObj::default());
    *to.ctx.obj.borrow_mut() = Rc::downgrade(dst);
    object_handlers::set_payload(dst, to.clone());
    let fname = host.active_function_name();
    let mut error = sys::U_ZERO_ERROR;
    // SAFETY: the original's converters; cloning calls its callbacks.
    let s = from.with_host(host, &fname, || unsafe {
        ucnv_clone(from.src(), &mut error)
    })?;
    to.src.set(s as usize);
    if !sys::failure(error) {
        error = sys::U_ZERO_ERROR;
        let d = from.with_host(host, &fname, || unsafe {
            ucnv_clone(from.dest(), &mut error)
        })?;
        to.dest.set(d as usize);
    }
    if sys::failure(error) {
        return Err(Thrown::error("Error", "Failed to clone UConverter"));
    }
    let this = Value::Object(dst.clone());
    set_callbacks(host, &to, &this, to.src(), &fname)?;
    set_callbacks(host, &to, &this, to.dest(), &fname)?;
    Ok(())
}

/// `php_converter_free_object`: closing calls a subclass's callbacks.
fn free_obj(host: &mut dyn Host, h: &ObjectHandle) -> Result<(), Thrown> {
    let Some(co) = object_handlers::payload_of::<ConvObj>(h) else {
        return Ok(());
    };
    let fname = host.active_function_name();
    let (s, d) = (
        co.src.replace(0) as *mut c_void,
        co.dest.replace(0) as *mut c_void,
    );
    co.with_host(host, &fname, || unsafe {
        // SAFETY: the converters this object owned.
        if !s.is_null() {
            ucnv_close(s);
        }
        if !d.is_null() {
            ucnv_close(d);
        }
    })
}

/// `php_converter_default_callback`: `toUCallback()`/`fromUCallback()`.
fn default_callback(host: &mut dyn Host, call: &Call, co: &ConvObj) -> Result<Value, Thrown> {
    let reason = call.long(0, 0) as i32;
    if !matches!(reason, UCNV_UNASSIGNED | UCNV_ILLEGAL | UCNV_IRREGULAR) {
        return Ok(Value::Null);
    }
    let err = call.reference(3);
    let assign = |host: &mut dyn Host, code: UErrorCode| -> Result<(), Thrown> {
        if let Some(r) = &err {
            common::assign_ref(host, r, Value::Int(code as i64))?;
        }
        Ok(())
    };
    if co.src().is_null() {
        throw_failure(
            host,
            Some(&co.ctx),
            &call.fname,
            sys::U_INVALID_STATE_ERROR,
            "Source Converter has not been initialized yet",
        )?;
        assign(host, sys::U_INVALID_STATE_ERROR)?;
        return Ok(Value::str(vec![0x1a]));
    }
    let mut status = sys::U_ZERO_ERROR;
    let mut chars = subst_chars(co.src(), &mut status);
    if sys::failure(status) {
        throw_failure(
            host,
            Some(&co.ctx),
            &call.fname,
            status,
            &ufailure_msg(status),
        )?;
        chars = vec![0x1a];
    }
    assign(host, status)?;
    Ok(Value::str(chars))
}

/// `__uconv(method, argc, $this, ...args)`.
pub(super) fn fn_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("UConverter", args, &[]);
    let f = call.fname.clone();
    match call.name {
        "reasonText" => {
            error::error_reset();
            let name = match call.long(0, 0) as i32 {
                UCNV_UNASSIGNED => "REASON_UNASSIGNED",
                UCNV_ILLEGAL => "REASON_ILLEGAL",
                UCNV_IRREGULAR => "REASON_IRREGULAR",
                UCNV_RESET => "REASON_RESET",
                UCNV_CLOSE => "REASON_CLOSE",
                UCNV_CLONE => "REASON_CLONE",
                _ => return Err(call.value_error(0, "must be a UConverter::REASON_* constant")),
            };
            return Ok(Value::str(name.as_bytes().to_vec()));
        }
        "transcode" => return transcode(host, &call),
        "getAvailable" => {
            error::error_reset();
            let mut a = PhpArray::new();
            // SAFETY: ICU's static converter list.
            for i in 0..unsafe { ucnv_countAvailable() } {
                a.push(Value::str(super::c_bytes(unsafe {
                    ucnv_getAvailableName(i)
                })));
            }
            return Ok(Value::Array(a));
        }
        "getAliases" => {
            error::error_reset();
            let name = super::c_string(&call.bytes(0));
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: a NUL-terminated name.
            let count = unsafe { ucnv_countAliases(name.as_ptr(), &mut status) };
            if sys::failure(status) {
                throw_failure(host, None, &f, status, &ufailure_msg(status))?;
                return Ok(Value::Bool(false));
            }
            let mut a = PhpArray::new();
            for i in 0..count {
                let mut st = sys::U_ZERO_ERROR;
                // SAFETY: as above.
                let alias = unsafe { ucnv_getAlias(name.as_ptr(), i, &mut st) };
                if sys::failure(st) {
                    throw_failure(host, None, &f, st, &ufailure_msg(st))?;
                    return Ok(Value::Null);
                }
                a.push(Value::str(super::c_bytes(alias)));
            }
            return Ok(Value::Array(a));
        }
        "getStandards" => {
            error::error_reset();
            let mut a = PhpArray::new();
            // SAFETY: ICU's static standard list.
            for i in 0..unsafe { ucnv_countStandards() } {
                let mut st = sys::U_ZERO_ERROR;
                let name = unsafe { ucnv_getStandard(i, &mut st) };
                if sys::failure(st) {
                    throw_failure(host, None, &f, st, &ufailure_msg(st))?;
                    return Ok(Value::Null);
                }
                a.push(Value::str(super::c_bytes(name)));
            }
            return Ok(Value::Array(a));
        }
        _ => {}
    }
    let this = call.this.clone().expect("$this");
    let co = common::state_of::<ConvObj>(&this).expect("an object");
    if let Some(h) = object_handlers::object_of(&this) {
        *co.ctx.obj.borrow_mut() = Rc::downgrade(&h);
    }
    let reset_all = || {
        co.ctx.err.borrow_mut().reset();
        error::error_reset();
    };
    Ok(match call.name {
        "__construct" => {
            error::error_reset();
            let enc = |i: usize| -> Option<Vec<u8>> {
                match call.arg(i) {
                    None => Some(b"utf-8".to_vec()),
                    Some(Value::Null) => None,
                    Some(Value::Str(s)) => Some(s.as_bytes().to_vec()),
                    Some(_) => None,
                }
            };
            let (dest, src) = (enc(0), enc(1));
            error::in_ctor(|| -> Result<(), Thrown> {
                if set_encoding(host, &co, &this, &co.src, src.as_deref(), &f)? {
                    set_encoding(host, &co, &this, &co.dest, dest.as_deref(), &f)?;
                }
                Ok(())
            })?;
            Value::Null
        }
        "toUCallback" | "fromUCallback" => default_callback(host, &call, &co)?,
        "setSourceEncoding" | "setDestinationEncoding" => {
            reset_all();
            let slot = if call.name == "setSourceEncoding" {
                &co.src
            } else {
                &co.dest
            };
            Value::Bool(set_encoding(
                host,
                &co,
                &this,
                slot,
                Some(&call.bytes(0)),
                &f,
            )?)
        }
        "getSourceEncoding" | "getDestinationEncoding" => {
            reset_all();
            let cnv = if call.name == "getSourceEncoding" {
                co.src()
            } else {
                co.dest()
            };
            if cnv.is_null() {
                return Ok(Value::Null);
            }
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: a live converter.
            let name = unsafe { ucnv_getName(cnv, &mut status) };
            co.ctx.err.borrow_mut().code = status;
            if sys::failure(status) {
                throw_failure(host, Some(&co.ctx), &f, status, &ufailure_msg(status))?;
                return Ok(Value::Bool(false));
            }
            Value::str(super::c_bytes(name))
        }
        "getSourceType" | "getDestinationType" => {
            reset_all();
            let cnv = if call.name == "getSourceType" {
                co.src()
            } else {
                co.dest()
            };
            if cnv.is_null() {
                return Ok(Value::Null);
            }
            // SAFETY: a live converter.
            Value::Int(unsafe { ucnv_getType(cnv) } as i64)
        }
        "setSubstChars" => {
            reset_all();
            let chars = call.bytes(0);
            let mut ret = true;
            for (cnv, which) in [(co.src(), "Source"), (co.dest(), "Destination")] {
                if cnv.is_null() {
                    throw_failure(
                        host,
                        Some(&co.ctx),
                        &f,
                        sys::U_INVALID_STATE_ERROR,
                        &format!("{which} Converter has not been initialized yet"),
                    )?;
                    ret = false;
                    continue;
                }
                let mut status = sys::U_ZERO_ERROR;
                // SAFETY: a live converter; the bytes.
                unsafe {
                    ucnv_setSubstChars(cnv, chars.as_ptr().cast(), chars.len() as i8, &mut status)
                };
                if sys::failure(status) {
                    throw_failure(host, Some(&co.ctx), &f, status, &ufailure_msg(status))?;
                    ret = false;
                }
            }
            Value::Bool(ret)
        }
        "getSubstChars" => {
            reset_all();
            if co.src().is_null() {
                return Ok(Value::Null);
            }
            let mut status = sys::U_ZERO_ERROR;
            let chars = subst_chars(co.src(), &mut status);
            if sys::failure(status) {
                throw_failure(host, Some(&co.ctx), &f, status, &ufailure_msg(status))?;
                return Ok(Value::Bool(false));
            }
            Value::str(chars)
        }
        "convert" => {
            reset_all();
            let s = call.bytes(0);
            let reverse = call.bool(1, false);
            let (dest, src) = if reverse {
                (co.src(), co.dest())
            } else {
                (co.dest(), co.src())
            };
            match do_convert(host, Some(&co), dest, src, &s, &f)? {
                Some(r) => Value::str(r),
                None => Value::Bool(false),
            }
        }
        "getErrorCode" => Value::Int(co.ctx.err.borrow().code as i64),
        "getErrorMessage" => Value::str(co.ctx.err.borrow().message().into_bytes()),
        other => unreachable!("UConverter::{other}"),
    })
}

/// `UConverter::transcode`.
fn transcode(host: &mut dyn Host, call: &Call) -> Result<Value, Thrown> {
    let f = &call.fname;
    let (s, dest, src) = (call.bytes(0), call.bytes(1), call.bytes(2));
    let options = match call.arg(3) {
        Some(Value::Array(a)) => Some(a),
        _ => None,
    };
    error::error_reset();
    let src_cnv = open_converter(host, None, Some(&src), f)?;
    let dest_cnv = match src_cnv {
        Some(_) => open_converter(host, None, Some(&dest), f)?,
        None => None,
    };
    let close = |c: Option<*mut c_void>| {
        if let Some(c) = c {
            // SAFETY: a converter opened here.
            unsafe { ucnv_close(c) };
        }
    };
    let (Some(sc), Some(dc)) = (src_cnv, dest_cnv) else {
        close(src_cnv);
        close(dest_cnv);
        return Ok(Value::Bool(false));
    };
    let mut error = sys::U_ZERO_ERROR;
    if let Some(o) = options.filter(|o| !o.is_empty()) {
        for (key, cnv) in [("from_subst", sc), ("to_subst", dc)] {
            if sys::failure(error) {
                break;
            }
            if let Some(Value::Str(v)) = o
                .get(&rhp_values::ArrayKey::from_bytes(key.as_bytes()))
                .map(deref)
            {
                error = sys::U_ZERO_ERROR;
                // SAFETY: a live converter; at most 127 bytes.
                unsafe {
                    ucnv_setSubstChars(
                        cnv,
                        v.as_bytes().as_ptr().cast(),
                        (v.len() & 0x7f) as i8,
                        &mut error,
                    )
                };
            }
        }
    }
    let mut ret = Value::Null;
    if !sys::failure(error) {
        if let Some(r) = do_convert(host, None, dc, sc, &s, f)? {
            ret = Value::str(r);
        }
    }
    if sys::failure(error) {
        throw_failure(host, None, f, error, &ufailure_msg(error))?;
        ret = Value::Bool(false);
    }
    close(Some(sc));
    close(Some(dc));
    // php leaves the return value null when the conversion failed
    Ok(ret)
}

type ToUCallback =
    extern "C" fn(*const c_void, *mut ToUArgs, *const c_char, i32, i32, *mut UErrorCode);
type FromUCallback =
    extern "C" fn(*const c_void, *mut FromUArgs, *const UChar, i32, UChar32, i32, *mut UErrorCode);

super::sys::icu! {
    pub fn ucnv_open(name: *const c_char, status: *mut UErrorCode) -> *mut c_void;
    pub fn ucnv_close(c: *mut c_void);
    pub fn ucnv_clone(c: *const c_void, status: *mut UErrorCode) -> *mut c_void;
    pub fn ucnv_getName(c: *const c_void, status: *mut UErrorCode) -> *const c_char;
    pub fn ucnv_getType(c: *const c_void) -> i32;
    pub fn ucnv_setToUCallBack(
        c: *mut c_void, f: ToUCallback, ctx: *const c_void, old: *mut c_void, old_ctx: *mut c_void,
        status: *mut UErrorCode,
    );
    pub fn ucnv_setFromUCallBack(
        c: *mut c_void, f: FromUCallback, ctx: *const c_void, old: *mut c_void, old_ctx: *mut c_void,
        status: *mut UErrorCode,
    );
    pub fn ucnv_getSubstChars(c: *const c_void, buf: *mut c_char, len: *mut i8, status: *mut UErrorCode);
    pub fn ucnv_setSubstChars(c: *mut c_void, s: *const c_char, len: i8, status: *mut UErrorCode);
    pub fn ucnv_toUChars(
        c: *mut c_void, dest: *mut UChar, cap: i32, src: *const c_char, len: i32, status: *mut UErrorCode,
    ) -> i32;
    pub fn ucnv_fromUChars(
        c: *mut c_void, dest: *mut c_char, cap: i32, src: *const UChar, len: i32, status: *mut UErrorCode,
    ) -> i32;
    pub fn ucnv_countAvailable() -> i32;
    pub fn ucnv_getAvailableName(i: i32) -> *const c_char;
    pub fn ucnv_countAliases(name: *const c_char, status: *mut UErrorCode) -> u16;
    pub fn ucnv_getAlias(name: *const c_char, i: u16, status: *mut UErrorCode) -> *const c_char;
    pub fn ucnv_countStandards() -> u16;
    pub fn ucnv_getStandard(i: u16, status: *mut UErrorCode) -> *const c_char;
}
