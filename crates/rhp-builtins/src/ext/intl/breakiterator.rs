//! `IntlBreakIterator`, `IntlRuleBasedBreakIterator` and
//! `IntlCodePointBreakIterator`, ported from php-src
//! ext/intl/breakiterator/ onto ICU's BreakIterator (breakiter.cpp).

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_void};
use std::rc::Rc;

use rhp_values::{ObjectHandle, PhpArray, Value};

use super::common::{self, Call};
use super::error::{self, IntlError};
use super::iterator;
use super::sys::{self, UChar, UErrorCode, UParseError};
use crate::zend::internal_iterator::{self, IteratorFuncs};
use crate::zend::object_handlers::{self, CloneObj, ObjectHandlers, PropPurpose};
use crate::{Host, Thrown};

pub(super) static HANDLERS: ObjectHandlers = ObjectHandlers {
    clone_obj: CloneObj::Custom(clone_obj),
    compare: Some(compare),
    get_properties_for: Some(properties_for),
    get_iterator: Some(get_iterator),
    ..ObjectHandlers::STD
};

pub(super) const SPEC: common::ClassSpec = common::ClassSpec {
    name: "IntlBreakIterator",
    helper: "__breakiter",
    props: "",
};

pub(super) const RBBI_SPEC: common::ClassSpec = common::ClassSpec {
    name: "IntlRuleBasedBreakIterator",
    helper: "__rbbiter",
    props: "",
};

pub(super) const CPBI_SPEC: common::ClassSpec = common::ClassSpec {
    name: "IntlCodePointBreakIterator",
    helper: "__cpbiter",
    props: "",
};

/// The operations of `rhp_brk_op`.
pub(super) const OP_FIRST: i32 = 0;
const OP_LAST: i32 = 1;
const OP_PREVIOUS: i32 = 2;
pub(super) const OP_NEXT: i32 = 3;
pub(super) const OP_CURRENT: i32 = 4;
const OP_FOLLOWING: i32 = 5;
const OP_PRECEDING: i32 = 6;
const OP_NEXT_N: i32 = 7;
const OP_IS_BOUNDARY: i32 = 8;

/// `BreakIterator_object`.
#[derive(Default)]
pub(super) struct BiObj {
    err: RefCell<IntlError>,
    biter: Cell<usize>,
    /// The text ICU reads (its UText points into these bytes).
    text: RefCell<Option<Value>>,
    compiled_rules: RefCell<Option<Value>>,
}

impl BiObj {
    fn ptr(&self) -> *mut c_void {
        self.biter.get() as *mut c_void
    }

    /// `rhp_brk_op` on the live iterator.
    pub(super) fn op(&self, op: i32, n: i32) -> i32 {
        // SAFETY: a live iterator (callers checked).
        unsafe { rhp_brk_op(self.ptr(), op, n) }
    }

    /// The text's bytes (empty before `setText()`).
    pub(super) fn text_bytes(&self) -> Vec<u8> {
        match &*self.text.borrow() {
            Some(Value::Str(s)) => s.as_bytes().to_vec(),
            _ => Vec::new(),
        }
    }
}

impl Drop for BiObj {
    fn drop(&mut self) {
        if !self.ptr().is_null() {
            // SAFETY: the iterator this object owns.
            unsafe { rhp_brk_delete(self.ptr()) };
        }
    }
}

/// `breakiterator_object_create`: an object of the iterator's class.
fn object_create(host: &mut dyn Host, biter: *mut c_void) -> Result<Value, Thrown> {
    // SAFETY: a live iterator.
    let class: &[u8] = match unsafe { rhp_brk_kind(biter) } {
        1 => b"IntlRuleBasedBreakIterator",
        2 => b"IntlCodePointBreakIterator",
        _ => b"IntlBreakIterator",
    };
    let Some(obj) = host.instantiate(class, Vec::new()) else {
        // SAFETY: not adopted.
        unsafe { rhp_brk_delete(biter) };
        return Ok(Value::Null);
    };
    let bio = common::state_of::<BiObj>(&obj).expect("an object");
    bio.biter.set(biter as usize);
    Ok(obj)
}

/// `BreakIterator_clone_obj`.
fn clone_obj(_host: &mut dyn Host, src: &ObjectHandle, dst: &ObjectHandle) -> Result<(), Thrown> {
    let from = common::state::<BiObj>(src);
    let to = Rc::new(BiObj::default());
    object_handlers::set_payload(dst, to.clone());
    if from.ptr().is_null() {
        return Err(Thrown::error(
            "Error",
            "Cannot clone uninitialized BreakIterator",
        ));
    }
    // SAFETY: a live iterator.
    let c = unsafe { rhp_brk_clone(from.ptr()) };
    if c.is_null() {
        return Err(Thrown::error("Error", "Failed to clone BreakIterator"));
    }
    to.biter.set(c as usize);
    *to.text.borrow_mut() = from.text.borrow().clone();
    *to.compiled_rules.borrow_mut() = from.compiled_rules.borrow().clone();
    Ok(())
}

/// `BreakIterator_compare_objects`.
fn compare(host: &mut dyn Host, a: &Value, b: &Value) -> Result<Option<i32>, Thrown> {
    let is_bi = |v: &Value| super::timezone::is_a(host, v, b"IntlBreakIterator");
    if !is_bi(a) || !is_bi(b) {
        return Ok(None);
    }
    let (x, y) = (
        common::state_of::<BiObj>(a).expect("an object"),
        common::state_of::<BiObj>(b).expect("an object"),
    );
    if x.ptr().is_null() || y.ptr().is_null() {
        return Ok(Some(if x.ptr() == y.ptr() { 0 } else { 1 }));
    }
    // SAFETY: live iterators.
    Ok(Some(if unsafe { rhp_brk_equal(x.ptr(), y.ptr()) } != 0 {
        0
    } else {
        1
    }))
}

/// `BreakIterator_get_debug_info`.
fn properties_for(
    _host: &mut dyn Host,
    h: &ObjectHandle,
    purpose: PropPurpose,
) -> Result<Option<PhpArray>, Thrown> {
    if purpose != PropPurpose::Debug {
        return Ok(None);
    }
    let bio = common::state::<BiObj>(h);
    let mut a = PhpArray::new();
    if bio.ptr().is_null() {
        common::put(&mut a, "valid", Value::Bool(false));
        return Ok(Some(a));
    }
    common::put(&mut a, "valid", Value::Bool(true));
    common::put(
        &mut a,
        "text",
        bio.text.borrow().clone().unwrap_or(Value::Null),
    );
    // SAFETY: a live iterator; typeid's name is static.
    let ty = super::c_bytes(unsafe { rhp_brk_type_name(bio.ptr()) });
    common::put(&mut a, "type", Value::str(ty));
    Ok(Some(a))
}

/// `_breakiterator_get_iterator`: the boundaries, from `first()`.
fn get_iterator(
    host: &mut dyn Host,
    h: &ObjectHandle,
    by_ref: bool,
) -> Result<ObjectHandle, Thrown> {
    if by_ref {
        return Err(Thrown::error(
            "Exception",
            "Iteration by reference is not supported",
        ));
    }
    let bio = common::state::<BiObj>(h);
    if bio.ptr().is_null() {
        return Err(Thrown::error(
            "Exception",
            "The BreakIterator is not properly constructed",
        ));
    }
    internal_iterator::create(
        host,
        Box::new(Boundaries {
            _obj: h.clone(),
            bio,
            current: Cell::new(None),
        }),
    )
}

struct Boundaries {
    _obj: ObjectHandle,
    bio: Rc<BiObj>,
    current: Cell<Option<i32>>,
}

impl Boundaries {
    /// `_breakiter_prolog`: false (and the object's error) when the
    /// iterator went away.
    fn prolog(&self) -> bool {
        self.bio.err.borrow_mut().reset();
        if self.bio.ptr().is_null() {
            let mut e = self.bio.err.borrow_mut();
            e.code = sys::U_INVALID_STATE_ERROR;
            e.msg = Some(
                "The BreakIterator object backing the PHP iterator is not properly constructed"
                    .into(),
            );
            return false;
        }
        true
    }
}

impl IteratorFuncs for Boundaries {
    fn valid(&self, _host: &mut dyn Host) -> Result<bool, Thrown> {
        Ok(self.current.get().is_some())
    }
    fn current(&self, _host: &mut dyn Host) -> Result<Option<Value>, Thrown> {
        Ok(self.current.get().map(|p| Value::Int(p as i64)))
    }
    fn key(&self, _host: &mut dyn Host) -> Result<Option<Value>, Thrown> {
        Ok(None)
    }
    fn move_forward(&self, _host: &mut dyn Host) -> Result<(), Thrown> {
        self.current.set(None);
        if !self.prolog() {
            return Ok(());
        }
        let pos = self.bio.op(OP_NEXT, 0);
        if pos != sys::UBRK_DONE {
            self.current.set(Some(pos));
        }
        Ok(())
    }
    fn rewind(&self, _host: &mut dyn Host) -> Result<(), Thrown> {
        if self.prolog() {
            self.current.set(Some(self.bio.op(OP_FIRST, 0)));
        }
        Ok(())
    }
}

fn int32_arg(call: &Call, i: usize) -> Result<i32, Thrown> {
    let n = call.long(i, 0);
    if n < i32::MIN as i64 || n > i32::MAX as i64 {
        return Err(call.value_error(i, &format!("must be between {} and {}", i32::MIN, i32::MAX)));
    }
    Ok(n as i32)
}

/// `_breakiter_factory` and `createCodePointInstance`.
fn factory(host: &mut dyn Host, call: &Call) -> Result<Value, Thrown> {
    error::error_reset();
    if call.name == "createCodePointInstance" {
        // SAFETY: a new iterator, adopted by the object.
        return object_create(host, unsafe { rhp_brk_codepoint_new() });
    }
    let kind = match call.name {
        "createWordInstance" => 0,
        "createLineInstance" => 1,
        "createCharacterInstance" => 2,
        "createSentenceInstance" => 3,
        _ => 4,
    };
    let locale = super::c_string(&call.opt_bytes(0).unwrap_or_else(super::locale_get_default));
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a NUL-terminated locale.
    let b = unsafe { rhp_brk_create(kind, locale.as_ptr(), &mut status) };
    error::error_set_code(status);
    if sys::failure(status) {
        if !b.is_null() {
            // SAFETY: not adopted.
            unsafe { rhp_brk_delete(b) };
        }
        error::set_custom_msg(host, &call.fname, "error creating BreakIterator")?;
        return Ok(Value::Null);
    }
    object_create(host, b)
}

/// `__breakiter(method, argc, $this, ...args)`.
pub(super) fn fn_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("IntlBreakIterator", args, &[]);
    let f = call.fname.clone();
    match call.name {
        "__construct" => {
            return Err(Thrown::error(
                "Exception",
                "An object of this type cannot be created with the new operator",
            ))
        }
        "createWordInstance"
        | "createLineInstance"
        | "createCharacterInstance"
        | "createSentenceInstance"
        | "createTitleInstance"
        | "createCodePointInstance" => return factory(host, &call),
        _ => {}
    }
    let this = call.this.clone().expect("$this");
    let bio = common::state_of::<BiObj>(&this).expect("an object");
    match call.name {
        "getErrorCode" => return Ok(Value::Int(bio.err.borrow().code as i64)),
        "getErrorMessage" => return Ok(Value::str(bio.err.borrow().message())),
        "getIterator" => {
            let h = object_handlers::object_of(&this).expect("an object");
            return Ok(Value::Object(get_iterator(host, &h, false)?));
        }
        _ => {}
    }
    // Argument checks php makes before fetching the object.
    match call.name {
        "isBoundary" => {
            int32_arg(&call, 0)?;
        }
        "getLocale" => {
            let t = call.long(0, 0);
            if t != sys::ULOC_ACTUAL_LOCALE as i64 && t != sys::ULOC_VALID_LOCALE as i64 {
                error::set(
                    host,
                    &f,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "invalid locale type",
                )?;
                return Ok(Value::Bool(false));
            }
        }
        "getPartsIterator" => {
            let t = call.long(0, iterator::KEY_SEQUENTIAL);
            if ![
                iterator::KEY_SEQUENTIAL,
                iterator::KEY_LEFT,
                iterator::KEY_RIGHT,
            ]
            .contains(&t)
            {
                return Err(call.value_error(
                    0,
                    "must be one of IntlPartsIterator::KEY_SEQUENTIAL, IntlPartsIterator::KEY_LEFT, or IntlPartsIterator::KEY_RIGHT",
                ));
            }
        }
        _ => {}
    }
    // BREAKITER_METHOD_FETCH_OBJECT
    error::error_reset();
    bio.err.borrow_mut().reset();
    if bio.ptr().is_null() {
        return Err(Thrown::error("Error", "Found unconstructed BreakIterator"));
    }
    let int = |n: i32| Value::Int(n as i64);
    Ok(match call.name {
        "getText" => bio.text.borrow().clone().unwrap_or(Value::Null),
        "setText" => {
            let text = call.arg(0).unwrap_or(Value::str(Vec::new()));
            let Value::Str(s) = &text else {
                unreachable!("a string")
            };
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: the bytes stay alive in `bio.text`.
            unsafe {
                rhp_brk_set_text(
                    bio.ptr(),
                    s.as_bytes().as_ptr().cast(),
                    s.len() as i64,
                    &mut status,
                )
            };
            bio.err.borrow_mut().code = status;
            if error::check_status(host, &bio.err, &f, "error calling BreakIterator::setText()")? {
                return Ok(Value::Bool(false));
            }
            *bio.text.borrow_mut() = Some(text);
            Value::Bool(true)
        }
        "first" => int(bio.op(OP_FIRST, 0)),
        "last" => int(bio.op(OP_LAST, 0)),
        "previous" => int(bio.op(OP_PREVIOUS, 0)),
        "current" => int(bio.op(OP_CURRENT, 0)),
        "next" => match call.arg(0) {
            None | Some(Value::Null) => int(bio.op(OP_NEXT, 0)),
            Some(_) => int(bio.op(OP_NEXT_N, int32_arg(&call, 0)?)),
        },
        "following" => int(bio.op(OP_FOLLOWING, int32_arg(&call, 0)?)),
        "preceding" => int(bio.op(OP_PRECEDING, int32_arg(&call, 0)?)),
        "isBoundary" => Value::Bool(bio.op(OP_IS_BOUNDARY, int32_arg(&call, 0)?) != 0),
        "getLocale" => {
            let mut buf = [0 as c_char; 160];
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: a live iterator; a buffer of 160.
            unsafe {
                rhp_brk_locale(
                    bio.ptr(),
                    call.long(0, 0) as i32,
                    buf.as_mut_ptr(),
                    160,
                    &mut status,
                )
            };
            bio.err.borrow_mut().code = status;
            if error::check_status(host, &bio.err, &f, "Call to ICU method has failed")? {
                return Ok(Value::Bool(false));
            }
            Value::str(super::c_bytes(buf.as_ptr()))
        }
        "getPartsIterator" => {
            let t = call.long(0, iterator::KEY_SEQUENTIAL);
            iterator::from_parts(host, this, bio, t)?
        }
        other => unreachable!("IntlBreakIterator::{other}"),
    })
}

/// `__rbbiter(method, argc, $this, ...args)`.
pub(super) fn fn_rbbi_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("IntlRuleBasedBreakIterator", args, &[]);
    let f = call.fname.clone();
    let this = call.this.clone().expect("$this");
    let bio = common::state_of::<BiObj>(&this).expect("an object");
    if call.name == "__construct" {
        return construct(&call, &bio).map(|_| Value::Null);
    }
    error::error_reset();
    bio.err.borrow_mut().reset();
    if bio.ptr().is_null() {
        return Err(Thrown::error("Error", "Found unconstructed BreakIterator"));
    }
    let b = bio.ptr();
    // SAFETY (below): a live rule-based iterator.
    Ok(match call.name {
        "getRules" => {
            let mut len = 0;
            let p = unsafe { rhp_rbbi_rules(b, &mut len) };
            let u = super::cpp::take_ustr(p, len).unwrap_or_default();
            match super::utf16_to_utf8(&u) {
                Ok(s) => Value::str(s),
                Err(st) => {
                    error::errors_set(
                        host,
                        &bio.err,
                        &f,
                        st,
                        "Error converting result to UTF-8 string",
                    )?;
                    Value::Bool(false)
                }
            }
        }
        "getRuleStatus" => Value::Int(unsafe { rhp_rbbi_rule_status(b) } as i64),
        "getRuleStatusVec" => {
            let mut status = sys::U_ZERO_ERROR;
            let n = unsafe { rhp_rbbi_rule_status_vec(b, std::ptr::null_mut(), 0, &mut status) };
            let mut v = vec![0i32; n.max(0) as usize];
            status = sys::U_ZERO_ERROR;
            let n = unsafe { rhp_rbbi_rule_status_vec(b, v.as_mut_ptr(), n, &mut status) };
            if sys::failure(status) {
                error::errors_set(
                    host,
                    &bio.err,
                    &f,
                    status,
                    "failed obtaining the status values",
                )?;
                return Ok(Value::Bool(false));
            }
            let mut a = PhpArray::new();
            for r in &v[..n.max(0) as usize] {
                a.push(Value::Int(*r as i64));
            }
            Value::Array(a)
        }
        "getBinaryRules" => {
            let mut len = 0u32;
            let p = unsafe { rhp_rbbi_binary_rules(b, &mut len) };
            if len > i32::MAX as u32 - 1 {
                let code = bio.err.borrow().code;
                error::errors_set(host, &bio.err, &f, code, "the rules are too large")?;
                return Ok(Value::Bool(false));
            }
            if p.is_null() {
                Value::str(Vec::new())
            } else {
                // SAFETY: `len` bytes at `p`.
                Value::str(unsafe { std::slice::from_raw_parts(p, len as usize) }.to_vec())
            }
        }
        other => unreachable!("IntlRuleBasedBreakIterator::{other}"),
    })
}

/// `IntlRuleBasedBreakIterator::__construct`.
fn construct(call: &Call, bio: &BiObj) -> Result<(), Thrown> {
    let rules_v = call.arg(0).unwrap_or(Value::str(Vec::new()));
    let rules = match &rules_v {
        Value::Str(s) => s.as_bytes(),
        _ => &[],
    };
    let compiled = call.bool(1, false);
    if !bio.ptr().is_null() {
        return Err(Thrown::error(
            "Error",
            "IntlRuleBasedBreakIterator object is already constructed",
        ));
    }
    let mut status = sys::U_ZERO_ERROR;
    let b = if !compiled {
        let Ok(u) = super::utf8_to_utf16(rules) else {
            return Err(Thrown::error(
                "IntlException",
                "IntlRuleBasedBreakIterator::__construct(): rules were not a valid UTF-8 string",
            ));
        };
        let mut pe = UParseError::default();
        // SAFETY: the rules' units.
        let b = unsafe { rhp_rbbi_new(u.as_ptr(), u.len() as i32, &mut pe, &mut status) };
        error::error_set_code(status);
        if sys::failure(status) {
            return Err(Thrown::error(
                "IntlException",
                format!(
                    "IntlRuleBasedBreakIterator::__construct(): unable to create RuleBasedBreakIterator from rules ({})",
                    error::parse_error_to_string(&pe)
                ),
            ));
        }
        b
    } else {
        // SAFETY: the rules' bytes, which ICU aliases: the object keeps
        // the string.
        let b = unsafe { rhp_rbbi_new_compiled(rules.as_ptr(), rules.len() as u32, &mut status) };
        if sys::failure(status) {
            return Err(Thrown::error(
                "IntlException",
                "IntlRuleBasedBreakIterator::__construct(): unable to create instance from compiled rules",
            ));
        }
        b
    };
    bio.biter.set(b as usize);
    if compiled {
        *bio.compiled_rules.borrow_mut() = Some(rules_v.clone());
    }
    Ok(())
}

/// `__cpbiter(method, argc, $this)`.
pub(super) fn fn_cpbi_helper(_host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("IntlCodePointBreakIterator", args, &[]);
    let this = call.this.clone().expect("$this");
    let bio = common::state_of::<BiObj>(&this).expect("an object");
    error::error_reset();
    bio.err.borrow_mut().reset();
    if bio.ptr().is_null() {
        return Err(Thrown::error("Error", "Found unconstructed BreakIterator"));
    }
    match call.name {
        // SAFETY: a live code point iterator.
        "getLastCodePoint" => Ok(Value::Int(
            unsafe { rhp_cpbi_last_code_point(bio.ptr()) } as i64
        )),
        other => unreachable!("IntlCodePointBreakIterator::{other}"),
    }
}

unsafe extern "C" {
    fn rhp_brk_create(kind: i32, locale: *const c_char, status: *mut UErrorCode) -> *mut c_void;
    fn rhp_brk_codepoint_new() -> *mut c_void;
    fn rhp_brk_kind(b: *const c_void) -> i32;
    fn rhp_brk_type_name(b: *const c_void) -> *const c_char;
    fn rhp_brk_equal(a: *const c_void, b: *const c_void) -> i32;
    fn rhp_brk_clone(b: *const c_void) -> *mut c_void;
    fn rhp_brk_delete(b: *mut c_void);
    fn rhp_brk_set_text(b: *mut c_void, s: *const c_char, len: i64, status: *mut UErrorCode);
    fn rhp_brk_op(b: *mut c_void, op: i32, n: i32) -> i32;
    fn rhp_brk_locale(
        b: *const c_void,
        ty: i32,
        buf: *mut c_char,
        cap: i32,
        status: *mut UErrorCode,
    );
    fn rhp_rbbi_new(
        rules: *const UChar,
        len: i32,
        pe: *mut UParseError,
        status: *mut UErrorCode,
    ) -> *mut c_void;
    fn rhp_rbbi_new_compiled(rules: *const u8, len: u32, status: *mut UErrorCode) -> *mut c_void;
    fn rhp_rbbi_rules(b: *const c_void, len: *mut i32) -> *mut UChar;
    fn rhp_rbbi_rule_status(b: *const c_void) -> i32;
    fn rhp_rbbi_rule_status_vec(
        b: *mut c_void,
        out: *mut i32,
        cap: i32,
        status: *mut UErrorCode,
    ) -> i32;
    fn rhp_rbbi_binary_rules(b: *mut c_void, len: *mut u32) -> *const u8;
    fn rhp_cpbi_last_code_point(b: *mut c_void) -> i32;
}
