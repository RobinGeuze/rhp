//! `ResourceBundle` and `resourcebundle_*`, ported from php-src
//! ext/intl/resourcebundle/ onto ICU's `ures`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rhp_values::{ArrayKey, ObjectHandle, PhpArray, Value};

use super::common::{self, Call};
use super::error::{self, IntlError};
use super::sys::{self, UResourceBundle};
use crate::zend::internal_iterator::{self, IteratorFuncs};
use crate::zend::object_handlers::{self, DimType, DimensionHandlers, ObjectHandlers};
use crate::zend::zpp::deref;
use crate::{Host, Thrown};

pub(super) static HANDLERS: ObjectHandlers = ObjectHandlers {
    dimension: Some(&DIMENSION),
    get_iterator: Some(get_iterator),
    ..ObjectHandlers::UNCLONEABLE
};

static DIMENSION: DimensionHandlers = DimensionHandlers {
    read: read_dimension,
    write: |_, _, _, _| Err(not_array()),
    has: |_, _, _, _| Err(not_array()),
    unset: |_, _, _| Err(not_array()),
};

pub(super) const SPEC: common::ClassSpec = common::ClassSpec {
    name: "ResourceBundle",
    helper: "__resbundle",
    props: "",
};

pub(super) const ALIASES: &[(&str, &str)] = &[
    ("resourcebundle_create", "create"),
    ("resourcebundle_get", "get"),
    ("resourcebundle_count", "count"),
    ("resourcebundle_locales", "getLocales"),
    ("resourcebundle_get_error_code", "getErrorCode"),
    ("resourcebundle_get_error_message", "getErrorMessage"),
];

/// php's MAXPATHLEN.
#[cfg(target_os = "linux")]
const MAXPATHLEN: usize = 4096;
#[cfg(not(target_os = "linux"))]
const MAXPATHLEN: usize = 1024;

/// `ResourceBundle_object`.
#[derive(Default)]
pub(super) struct RbObj {
    err: RefCell<IntlError>,
    me: Cell<usize>,
    child: Cell<usize>,
}

impl RbObj {
    fn me(&self) -> *mut UResourceBundle {
        self.me.get() as *mut UResourceBundle
    }
    fn child(&self) -> *mut UResourceBundle {
        self.child.get() as *mut UResourceBundle
    }
}

impl Drop for RbObj {
    fn drop(&mut self) {
        // SAFETY: what this object owns.
        unsafe {
            if !self.me().is_null() {
                sys::ures_close(self.me());
            }
            if !self.child().is_null() {
                sys::ures_close(self.child());
            }
        }
    }
}

fn not_array() -> Thrown {
    Thrown::error("Error", "Cannot use object of type ResourceBundle as array")
}

/// `resourcebundle_ctor`.
fn ctor(host: &mut dyn Host, call: &Call, rb: &RbObj) -> Result<bool, Thrown> {
    let f = &call.fname;
    error::error_reset();
    let locale = call.opt_bytes(0);
    let bundle = call.opt_bytes(1);
    let fallback = call.bool(2, true);
    if !rb.me().is_null() {
        return Err(Thrown::error(
            "Error",
            "ResourceBundle object is already constructed",
        ));
    }
    if let Some(l) = &locale {
        if super::locale_too_long(host, f, l.len())? {
            return Ok(false);
        }
    }
    let locale = locale.unwrap_or_else(super::locale_get_default);
    if bundle.as_ref().is_some_and(|b| b.len() >= MAXPATHLEN) {
        return Err(call.value_error(1, "is too long"));
    }
    let cloc = super::c_string(&locale);
    let cbundle = bundle.as_deref().map(super::c_string);
    let bptr = cbundle.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: NUL-terminated strings (or NULL for ICU's own data).
    let me = unsafe {
        if fallback {
            sys::ures_open(bptr, cloc.as_ptr(), &mut status)
        } else {
            sys::ures_openDirect(bptr, cloc.as_ptr(), &mut status)
        }
    };
    rb.me.set(me as usize);
    rb.err.borrow_mut().code = status;
    if error::check_status(host, &rb.err, f, "Cannot load libICU resource bundle")? {
        return Ok(false);
    }
    if !fallback
        && (status == sys::U_USING_FALLBACK_WARNING || status == sys::U_USING_DEFAULT_WARNING)
    {
        error::error_set_code(status);
        let mut st = status;
        // SAFETY: a live bundle.
        let actual = unsafe { sys::ures_getLocaleByType(me, sys::ULOC_ACTUAL_LOCALE, &mut st) };
        let msg = format!(
            "Cannot load libICU resource '{}' without fallback from {} to {}",
            bundle
                .as_deref()
                .map_or("(default data)".into(), String::from_utf8_lossy),
            String::from_utf8_lossy(&locale),
            String::from_utf8_lossy(&super::c_bytes(actual)),
        );
        rb.err.borrow_mut().code = st;
        error::errors_set_custom_msg(host, &rb.err, f, &msg)?;
        return Ok(false);
    }
    Ok(true)
}

/// `resourcebundle_extract_value`: the value of `rb.child` (a nested
/// table or array becomes a ResourceBundle that takes the child over).
fn extract_value(host: &mut dyn Host, f: &str, rb: &RbObj) -> Result<Value, Thrown> {
    let child = rb.child();
    let mut status = sys::U_ZERO_ERROR;
    let mut len = 0i32;
    let fail = |host: &mut dyn Host,
                status: sys::UErrorCode,
                msg: &str|
     -> Result<Option<Value>, Thrown> {
        rb.err.borrow_mut().code = status;
        Ok(error::check_status(host, &rb.err, f, msg)?.then_some(Value::Bool(false)))
    };
    // SAFETY: `child` is the bundle's live child resource.
    let ty = unsafe { sys::ures_getType(child) };
    Ok(match ty {
        sys::URES_STRING => {
            let p = unsafe { sys::ures_getString(child, &mut len, &mut status) };
            if let Some(v) = fail(host, status, "Failed to retrieve string value")? {
                return Ok(v);
            }
            // SAFETY: `len` units at `p`.
            let u = unsafe { std::slice::from_raw_parts(p, len as usize) };
            match super::utf16_to_utf8(u) {
                Ok(s) => Value::str(s),
                Err(st) => {
                    fail(host, st, "Error converting value to UTF-8")?;
                    Value::Bool(false)
                }
            }
        }
        sys::URES_BINARY => {
            let p = unsafe { sys::ures_getBinary(child, &mut len, &mut status) };
            if let Some(v) = fail(host, status, "Failed to retrieve binary value")? {
                return Ok(v);
            }
            // SAFETY: `len` bytes at `p`.
            Value::str(unsafe { std::slice::from_raw_parts(p, len as usize) }.to_vec())
        }
        sys::URES_INT => {
            let n = unsafe { sys::ures_getInt(child, &mut status) };
            if let Some(v) = fail(host, status, "Failed to retrieve integer value")? {
                return Ok(v);
            }
            Value::Int(n as i64)
        }
        sys::URES_INT_VECTOR => {
            let p = unsafe { sys::ures_getIntVector(child, &mut len, &mut status) };
            if let Some(v) = fail(host, status, "Failed to retrieve vector value")? {
                return Ok(v);
            }
            let mut a = PhpArray::new();
            // SAFETY: `len` ints at `p`.
            for &n in unsafe { std::slice::from_raw_parts(p, len as usize) } {
                a.push(Value::Int(n as i64));
            }
            Value::Array(a)
        }
        _ => {
            let Some(obj) = host.instantiate(b"ResourceBundle", Vec::new()) else {
                return Ok(Value::Null);
            };
            let n = common::state_of::<RbObj>(&obj).expect("an object");
            n.me.set(rb.child.replace(0));
            rb.err.borrow_mut().reset();
            obj
        }
    })
}

/// The offset of `$rb[...]` or `get()`.
enum Offset {
    Key(Vec<u8>),
    Index(i64),
}

/// `resource_bundle_array_fetch`; `arg` is the argument number for
/// `get()` (0 for `$rb[...]`).
fn fetch(
    host: &mut dyn Host,
    f: &str,
    rb: &RbObj,
    offset: Offset,
    fallback: bool,
    arg: Option<&Call>,
) -> Result<Value, Thrown> {
    error::error_reset();
    rb.err.borrow_mut().reset();
    let mut status = sys::U_ZERO_ERROR;
    let what = match &offset {
        Offset::Key(k) => {
            if k.is_empty() {
                return Err(match arg {
                    Some(c) => c.value_error(0, "must not be empty"),
                    None => Thrown::error("ValueError", "Offset must not be empty"),
                });
            }
            let ck = super::c_string(k);
            // SAFETY: a live bundle; `child` is reused or NULL.
            let c = unsafe { sys::ures_getByKey(rb.me(), ck.as_ptr(), rb.child(), &mut status) };
            rb.child.set(c as usize);
            format!("'{}'", String::from_utf8_lossy(k))
        }
        Offset::Index(i) => {
            if *i < i32::MIN as i64 || *i > i32::MAX as i64 {
                return Err(match arg {
                    Some(c) => c.value_error(
                        0,
                        &format!("index must be between {} and {}", i32::MIN, i32::MAX),
                    ),
                    None => Thrown::error(
                        "ValueError",
                        format!("Index must be between {} and {}", i32::MIN, i32::MAX),
                    ),
                });
            }
            // SAFETY: as above.
            let c = unsafe { sys::ures_getByIndex(rb.me(), *i as i32, rb.child(), &mut status) };
            rb.child.set(c as usize);
            i.to_string()
        }
    };
    rb.err.borrow_mut().code = status;
    error::error_set_code(status);
    if sys::failure(status) {
        error::errors_set_custom_msg(
            host,
            &rb.err,
            f,
            &format!("Cannot load resource element {what}"),
        )?;
        return Ok(Value::Null);
    }
    if !fallback
        && (status == sys::U_USING_FALLBACK_WARNING || status == sys::U_USING_DEFAULT_WARNING)
    {
        let mut st = sys::U_ZERO_ERROR;
        // SAFETY: a live bundle.
        let actual =
            unsafe { sys::ures_getLocaleByType(rb.me(), sys::ULOC_ACTUAL_LOCALE, &mut st) };
        let msg = format!(
            "Cannot load element {what} without fallback from to {}",
            String::from_utf8_lossy(&super::c_bytes(actual))
        );
        error::errors_set_custom_msg(host, &rb.err, f, &msg)?;
        return Ok(Value::Null);
    }
    extract_value(host, f, rb)
}

/// `resourcebundle_array_get`.
fn read_dimension(
    host: &mut dyn Host,
    h: &ObjectHandle,
    offset: Option<&Value>,
    ty: DimType,
) -> Result<Option<Value>, Thrown> {
    let Some(offset) = offset else {
        return Err(Thrown::error(
            "Error",
            "Cannot apply [] to ResourceBundle object",
        ));
    };
    if matches!(ty, DimType::W | DimType::Rw | DimType::Unset) {
        return Err(not_array());
    }
    let rb = common::state::<RbObj>(h);
    let off = match deref(offset) {
        Value::Int(n) => Offset::Index(n),
        Value::Str(s) => Offset::Key(s.as_bytes().to_vec()),
        v => {
            let tn = crate::zend::zpp::zval_type_name(host, &v);
            return Err(Thrown::type_error(format!(
                "Cannot access offset of type {tn} on ResourceBundle"
            )));
        }
    };
    // php names the running function in the message.
    let f = host.active_function_name();
    let v = fetch(host, &f, &rb, off, true, None)?;
    Ok(match (ty, v) {
        (DimType::Is, Value::Null) => None,
        (_, v) => Some(v),
    })
}

/// `resourcebundle_get_iterator`.
fn get_iterator(
    host: &mut dyn Host,
    h: &ObjectHandle,
    by_ref: bool,
) -> Result<ObjectHandle, Thrown> {
    if by_ref {
        return Err(Thrown::error(
            "Error",
            "An iterator cannot be used with foreach by reference",
        ));
    }
    let rb = common::state::<RbObj>(h);
    // SAFETY: a live bundle (an unconstructed one has no resources).
    let (is_table, length) = if rb.me().is_null() {
        (false, 0)
    } else {
        unsafe {
            (
                sys::ures_getType(rb.me()) == sys::URES_TABLE,
                sys::ures_getSize(rb.me()),
            )
        }
    };
    internal_iterator::create(
        host,
        Box::new(RbIter {
            _obj: h.clone(),
            rb,
            is_table,
            length,
            i: Cell::new(0),
            current: RefCell::new(None),
            key: RefCell::new(None),
        }),
    )
}

/// `ResourceBundle_iterator`.
struct RbIter {
    _obj: ObjectHandle,
    rb: Rc<RbObj>,
    is_table: bool,
    length: i32,
    i: Cell<i32>,
    current: RefCell<Option<Value>>,
    key: RefCell<Option<Vec<u8>>>,
}

impl RbIter {
    /// `resourcebundle_iterator_read`.
    fn read(&self, host: &mut dyn Host) -> Result<(), Thrown> {
        if self.current.borrow().is_some() {
            return Ok(());
        }
        let rb = &self.rb;
        let mut status = sys::U_ZERO_ERROR;
        // SAFETY: a live bundle.
        let c = unsafe { sys::ures_getByIndex(rb.me(), self.i.get(), rb.child(), &mut status) };
        rb.child.set(c as usize);
        if !sys::failure(status) {
            if self.is_table {
                // SAFETY: the child just read.
                *self.key.borrow_mut() = Some(super::c_bytes(unsafe { sys::ures_getKey(c) }));
            }
            let f = host.active_function_name();
            let v = extract_value(host, &f, rb)?;
            *self.current.borrow_mut() = Some(v);
        }
        Ok(())
    }

    fn invalidate(&self) {
        *self.current.borrow_mut() = None;
        *self.key.borrow_mut() = None;
    }
}

impl IteratorFuncs for RbIter {
    fn valid(&self, _host: &mut dyn Host) -> Result<bool, Thrown> {
        Ok(self.i.get() < self.length)
    }
    fn current(&self, host: &mut dyn Host) -> Result<Option<Value>, Thrown> {
        self.read(host)?;
        Ok(self.current.borrow().clone())
    }
    fn key(&self, host: &mut dyn Host) -> Result<Option<Value>, Thrown> {
        self.read(host)?;
        Ok(Some(if self.is_table {
            Value::str(self.key.borrow().clone().unwrap_or_default())
        } else {
            Value::Int(self.i.get() as i64)
        }))
    }
    fn move_forward(&self, _host: &mut dyn Host) -> Result<(), Thrown> {
        self.i.set(self.i.get() + 1);
        self.invalidate();
        Ok(())
    }
    fn rewind(&self, _host: &mut dyn Host) -> Result<(), Thrown> {
        self.i.set(0);
        self.invalidate();
        Ok(())
    }
}

/// `__resbundle(method, argc, $this, ...args)`.
pub(super) fn fn_helper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let call = Call::new("ResourceBundle", args, ALIASES);
    let f = call.fname.clone();
    match call.name {
        "__construct" => {
            let this = call.this.clone().expect("$this");
            let rb = common::state_of::<RbObj>(&this).expect("an object");
            error::in_ctor(|| ctor(host, &call, &rb))?;
            return Ok(Value::Null);
        }
        "create" => {
            let Some(obj) = host.instantiate(b"ResourceBundle", Vec::new()) else {
                return Ok(Value::Null);
            };
            let rb = common::state_of::<RbObj>(&obj).expect("an object");
            return Ok(if ctor(host, &call, &rb)? {
                obj
            } else {
                Value::Null
            });
        }
        "getLocales" => return locales(host, &call),
        _ => {}
    }
    let this = call.this.clone().expect("an object");
    let rb = common::state_of::<RbObj>(&this).expect("an object");
    match call.name {
        "getErrorCode" => return Ok(Value::Int(rb.err.borrow().code as i64)),
        "getErrorMessage" => return Ok(Value::str(rb.err.borrow().message().into_bytes())),
        "getIterator" => {
            let h = object_handlers::object_of(&this).expect("an object");
            return Ok(Value::Object(get_iterator(host, &h, false)?));
        }
        _ => {}
    }
    if call.name == "get" {
        let off = match call.arg(0) {
            Some(Value::Int(n)) => Offset::Index(n),
            Some(Value::Str(s)) => Offset::Key(s.as_bytes().to_vec()),
            _ => Offset::Index(0),
        };
        return fetch(host, &f, &rb, off, call.bool(1, true), Some(&call));
    }
    // RESOURCEBUNDLE_METHOD_FETCH_OBJECT
    if rb.me().is_null() {
        return Err(Thrown::error("Error", "Found unconstructed ResourceBundle"));
    }
    match call.name {
        // SAFETY: a live bundle.
        "count" => Ok(Value::Int(unsafe { sys::ures_getSize(rb.me()) } as i64)),
        other => unreachable!("ResourceBundle::{other}"),
    }
}

/// `resourcebundle_locales`.
// c_char is i8 or u8 depending on the target.
#[allow(clippy::unnecessary_cast)]
fn locales(host: &mut dyn Host, call: &Call) -> Result<Value, Thrown> {
    let f = &call.fname;
    error::error_reset();
    let bundle = call.bytes(0);
    if bundle.len() >= MAXPATHLEN {
        return Err(call.value_error(0, "is too long"));
    }
    let cb = super::c_string(&bundle);
    let bptr = if bundle.is_empty() {
        std::ptr::null()
    } else {
        cb.as_ptr()
    };
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a NUL-terminated name or NULL.
    let e = unsafe { sys::ures_openAvailableLocales(bptr, &mut status) };
    if error::check_global(host, status, f, "Cannot fetch locales list")? {
        return Ok(Value::Bool(false));
    }
    // SAFETY: the enumeration just opened.
    unsafe { sys::uenum_reset(e, &mut status) };
    if error::check_global(host, status, f, "Cannot iterate locales list")? {
        unsafe { sys::uenum_close(e) };
        return Ok(Value::Bool(false));
    }
    let mut a = PhpArray::new();
    loop {
        let mut len = 0;
        // SAFETY: as above.
        let p = unsafe { sys::uenum_next(e, &mut len, &mut status) };
        if p.is_null() {
            break;
        }
        // SAFETY: `len` bytes at `p`.
        let s = unsafe { std::slice::from_raw_parts(p as *const u8, len as usize) };
        a.insert(ArrayKey::Int(a.len() as i64), Value::str(s.to_vec()));
    }
    unsafe { sys::uenum_close(e) };
    Ok(Value::Array(a))
}
