//! `Collator` and `collator_*`, ported from php-src ext/intl/collator/
//! onto ICU's `ucol`. A `Collator` object's state (php's
//! `Collator_object`) is its payload.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rhp_values::{ArrayKey, ObjectHandle, PhpArray, Value};

use super::error::{self, IntlError};
use super::sys::{self, UChar, UCollator};
use super::{utf16_to_utf8, utf8_to_utf16};
use crate::zend::object_handlers::{self, CloneObj, ObjectHandlers};
use crate::zend::zpp::{self, deref};
use crate::{Host, Thrown};

/// No clone: php's `Collator_handlers.clone_obj` is NULL.
pub(super) static HANDLERS: ObjectHandlers = ObjectHandlers {
    clone_obj: CloneObj::None,
    ..ObjectHandlers::STD
};

const SORT_REGULAR: i64 = 0;
const SORT_STRING: i64 = 1;
const SORT_NUMERIC: i64 = 2;

/// `Collator_object`.
#[derive(Default)]
pub(super) struct CollatorObj {
    err: RefCell<IntlError>,
    ucoll: Cell<usize>,
}

impl CollatorObj {
    fn ucoll(&self) -> *mut UCollator {
        self.ucoll.get() as *mut UCollator
    }
}

impl Drop for CollatorObj {
    fn drop(&mut self) {
        let c = self.ucoll();
        if !c.is_null() {
            // SAFETY: the collator this object opened.
            unsafe { sys::ucol_close(c) };
        }
    }
}

/// The state of a `Collator` (or subclass) object, created on first use.
fn object(host: &dyn Host, v: &Value) -> Option<Rc<CollatorObj>> {
    if !object_handlers::has_handlers(host, v, &HANDLERS) {
        return None;
    }
    let h = object_handlers::object_of(v)?;
    Some(payload(&h))
}

fn payload(h: &ObjectHandle) -> Rc<CollatorObj> {
    if let Some(o) = object_handlers::payload_of::<CollatorObj>(h) {
        return o;
    }
    let o = Rc::new(CollatorObj::default());
    object_handlers::set_payload(h, o.clone());
    o
}

/// A call: the method's or function's name, and whether it runs as a
/// method (php's `hasThis()`, which shifts argument numbers).
struct Call<'a> {
    fname: &'a str,
    method: bool,
}

impl Call<'_> {
    /// The int parameter at `i` after the object (php numbers a
    /// procedural call's object #1).
    fn long(
        &self,
        host: &mut dyn Host,
        args: &[Value],
        i: usize,
        pname: &str,
        default: i64,
    ) -> Result<i64, Thrown> {
        let n = i + if self.method { 1 } else { 2 };
        super::long_param(host, args.get(i), n, self.fname, pname, default)
    }

    /// The string parameter at `i` after the object.
    fn str(
        &self,
        host: &mut dyn Host,
        args: &[Value],
        i: usize,
        pname: &str,
    ) -> Result<Vec<u8>, Thrown> {
        let n = i + if self.method { 1 } else { 2 };
        super::str_param(host, args.get(i), n, self.fname, pname)
    }
}

/// `zend_parse_method_parameters`' "O": the object, then the remaining
/// arguments. A procedural call's first argument must be a Collator.
fn this_and_args<'a>(
    host: &dyn Host,
    call: &Call,
    this: Option<&Value>,
    args: &'a [Value],
) -> Result<(Rc<CollatorObj>, &'a [Value]), Thrown> {
    if let Some(t) = this {
        let o = object(host, t).expect("a Collator");
        return Ok((o, args));
    }
    let v = args.first().map(deref).unwrap_or(Value::Null);
    match object(host, &v) {
        Some(o) => Ok((o, &args[1..])),
        None => Err(Thrown::type_error(format!(
            "{}(): Argument #1 ($object) must be of type Collator, {} given",
            call.fname,
            zpp::zval_value_name(host, &v)
        ))),
    }
}

/// `collator_check_initialized`.
fn check_initialized(host: &mut dyn Host, co: &CollatorObj, call: &Call) -> Result<(), Thrown> {
    if co.ucoll().is_null() {
        let code = co.err.borrow().code;
        error::error_set_code(code);
        error::errors_set_custom_msg(host, &co.err, call.fname, "Object not initialized")?;
        return Err(Thrown::error("Error", "Object not initialized"));
    }
    Ok(())
}

/// The fetch every method starts with: global and object errors reset.
fn fetch(co: &CollatorObj) {
    co.err.borrow_mut().reset();
}

fn str_at(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
    p: &str,
) -> Result<Vec<u8>, Thrown> {
    zpp::str_arg(host, args, i, f, p)
}

// --- construction ------------------------------------------------------------

/// `collator_ctor`: false after a failure (the error, or the exception in
/// a constructor, is set).
fn ctor(host: &mut dyn Host, fname: &str, this: &Value, locale: &[u8]) -> Result<bool, Thrown> {
    error::error_reset();
    if super::locale_too_long(host, fname, locale.len())? {
        return Ok(false);
    }
    let co = object(host, this).expect("a Collator");
    fetch(&co);
    if !co.ucoll().is_null() {
        return Err(Thrown::error(
            "Error",
            "Collator object is already constructed",
        ));
    }
    let locale = if locale.is_empty() {
        super::locale_get_default()
    } else {
        locale.to_vec()
    };
    let loc = super::c_string(&locale);
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a NUL-terminated locale name.
    let c = unsafe { sys::ucol_open(loc.as_ptr(), &mut status) };
    co.ucoll.set(c as usize);
    co.err.borrow_mut().code = status;
    if error::check_status(host, &co.err, fname, "unable to open ICU collator")? {
        return Ok(false);
    }
    Ok(true)
}

fn create(host: &mut dyn Host, fname: &str, args: &[Value]) -> Result<Value, Thrown> {
    let locale = str_at(host, args, 0, fname, "locale")?;
    let Some(obj) = host.instantiate(b"Collator", Vec::new()) else {
        return Ok(Value::Null);
    };
    if ctor(host, fname, &obj, &locale)? {
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
    if name == "create" {
        return create(host, fname, args);
    }
    if name == "__construct" {
        let this = this.expect("$this");
        let locale = str_at(host, args, 0, fname, "locale")?;
        error::in_ctor(|| ctor(host, fname, this, &locale))?;
        return Ok(Value::Null);
    }
    error::error_reset();
    let (co, args) = this_and_args(host, &call, this, args)?;
    let f = fname;
    match name {
        "getErrorCode" => return Ok(Value::Int(co.err.borrow().code as i64)),
        "getErrorMessage" => return Ok(Value::str(co.err.borrow().message())),
        _ => {}
    }
    fetch(&co);
    if matches!(name, "sort" | "asort") {
        if co.ucoll().is_null() {
            return Err(Thrown::error("Error", "Object not initialized"));
        }
    } else {
        check_initialized(host, &co, &call)?;
    }
    let c = co.ucoll();
    match name {
        "getAttribute" => {
            let attr = call.long(host, args, 0, "attribute", 0)?;
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: a live collator.
            let v = unsafe { sys::ucol_getAttribute(c, attr as i32, &mut status) };
            co.err.borrow_mut().code = status;
            if error::check_status(host, &co.err, f, "Error getting attribute value")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Int(v as i64))
        }
        "setAttribute" => {
            let attr = call.long(host, args, 0, "attribute", 0)?;
            let value = call.long(host, args, 1, "value", 0)?;
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: a live collator.
            unsafe { sys::ucol_setAttribute(c, attr as i32, value as i32, &mut status) };
            co.err.borrow_mut().code = status;
            if error::check_status(host, &co.err, f, "Error setting attribute value")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(true))
        }
        // SAFETY: a live collator.
        "getStrength" => Ok(Value::Int(unsafe { sys::ucol_getStrength(c) } as i64)),
        "setStrength" => {
            let strength = call.long(host, args, 0, "strength", 0)?;
            // SAFETY: a live collator.
            unsafe { sys::ucol_setStrength(c, strength as i32) };
            Ok(Value::Bool(true))
        }
        "getLocale" => {
            let kind = call.long(host, args, 0, "type", 0)?;
            let mut status = sys::U_ZERO_ERROR;
            // SAFETY: a live collator.
            let p = unsafe { sys::ucol_getLocaleByType(c, kind as i32, &mut status) };
            co.err.borrow_mut().code = status;
            if error::check_status(host, &co.err, f, "Error getting locale by type")? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::str(super::c_bytes(p)))
        }
        "compare" => {
            let s1 = call.str(host, args, 0, "string1")?;
            let s2 = call.str(host, args, 1, "string2")?;
            let u1 = match utf8_to_utf16(&s1) {
                Ok(u) => u,
                Err(st) => {
                    error::errors_set_code(&co.err, st);
                    error::errors_set_custom_msg(
                        host,
                        &co.err,
                        f,
                        "Error converting first argument to UTF-16",
                    )?;
                    return Ok(Value::Bool(false));
                }
            };
            let u2 = match utf8_to_utf16(&s2) {
                Ok(u) => u,
                Err(st) => {
                    error::errors_set_code(&co.err, st);
                    error::errors_set_custom_msg(
                        host,
                        &co.err,
                        f,
                        "Error converting second argument to UTF-16",
                    )?;
                    return Ok(Value::Bool(false));
                }
            };
            Ok(Value::Int(strcoll(c, &u1, &u2) as i64))
        }
        "getSortKey" => {
            let s = call.str(host, args, 0, "string")?;
            let u = match utf8_to_utf16(&s) {
                Ok(u) => u,
                Err(st) => {
                    error::errors_set_code(&co.err, st);
                    error::errors_set_custom_msg(
                        host,
                        &co.err,
                        f,
                        "Error converting first argument to UTF-16",
                    )?;
                    return Ok(Value::Bool(false));
                }
            };
            match sort_key(c, &u) {
                Some(mut k) => {
                    k.pop();
                    Ok(Value::str(k))
                }
                None => Ok(Value::Bool(false)),
            }
        }
        "sort" | "asort" => {
            let flags = call.long(host, args, 1, "flags", SORT_REGULAR)?;
            sort_internal(host, &co, f, args.first(), name == "sort", flags)
        }
        "sortWithSortKeys" => sort_with_sort_keys(host, &co, f, args.first()),
        _ => Ok(Value::Null),
    }
}

fn strcoll(c: *const UCollator, a: &[UChar], b: &[UChar]) -> i32 {
    // SAFETY: a live collator over two UTF-16 slices.
    unsafe { sys::ucol_strcoll(c, a.as_ptr(), a.len() as i32, b.as_ptr(), b.len() as i32) }
}

/// `ucol_getSortKey` with its terminating NUL; `None` for an empty key.
fn sort_key(c: *const UCollator, u: &[UChar]) -> Option<Vec<u8>> {
    // SAFETY: a preflight, then a buffer of the size ICU asked for.
    let len =
        unsafe { sys::ucol_getSortKey(c, u.as_ptr(), u.len() as i32, std::ptr::null_mut(), 0) };
    if len <= 0 {
        return None;
    }
    let mut k = vec![0u8; len as usize];
    let len = unsafe { sys::ucol_getSortKey(c, u.as_ptr(), u.len() as i32, k.as_mut_ptr(), len) };
    if len <= 0 {
        return None;
    }
    k.truncate(len as usize);
    Some(k)
}

// --- sorting (collator_sort.c, collator_convert.c, collator_is_numeric.c) --------

/// The reference a by-reference array argument arrives in: the procedural
/// function's own argument, or the prelude's `[&$array]`.
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

fn store_ref(r: &Value, v: Value) {
    if let Value::Ref(rc) = r {
        *rc.borrow_mut() = v;
    }
}

/// A value of the array being sorted: php converts its strings to UTF-16
/// in place first.
#[derive(Clone)]
enum Item {
    U16(Rc<Vec<UChar>>),
    Other(Value),
}

/// `collator_is_numeric`'s results.
enum Numeric {
    Long(i64),
    Double(f64),
}

fn at(s: &[UChar], i: usize) -> UChar {
    s.get(i).copied().unwrap_or(0)
}

fn is_space(c: UChar) -> bool {
    // SAFETY: a plain property lookup.
    unsafe { sys::u_isspace(c as i32) != 0 }
}

/// `collator_u_strtol(nptr, &end, 10)`: the value, the end index and
/// whether it overflowed (php's ERANGE).
fn u_strtol(s: &[UChar]) -> (i64, usize, bool) {
    let mut i = 0;
    let mut c;
    loop {
        c = at(s, i);
        i += 1;
        if !is_space(c) {
            break;
        }
    }
    let mut neg = false;
    if c == b'-' as UChar {
        neg = true;
        c = at(s, i);
        i += 1;
    } else if c == b'+' as UChar {
        c = at(s, i);
        i += 1;
    }
    let base: u64 = 10;
    let cutoff: u64 = if neg {
        (i64::MIN as u64).wrapping_neg()
    } else {
        i64::MAX as u64
    };
    let cutlim = cutoff % base;
    let cutoff = cutoff / base;
    let mut acc: u64 = 0;
    let mut any: i32 = 0;
    loop {
        let d = if (b'0' as UChar..=b'9' as UChar).contains(&c) {
            (c - b'0' as UChar) as u64
        } else if (b'A' as UChar..=b'Z' as UChar).contains(&c) {
            (c - b'A' as UChar + 10) as u64
        } else if (b'a' as UChar..=b'z' as UChar).contains(&c) {
            (c - b'a' as UChar + 10) as u64
        } else {
            break;
        };
        if d >= base {
            break;
        }
        if any < 0 || acc > cutoff || (acc == cutoff && d > cutlim) {
            any = -1;
        } else {
            any = 1;
            acc = acc * base + d;
        }
        c = at(s, i);
        i += 1;
    }
    let mut erange = false;
    let val = if any < 0 {
        erange = true;
        if neg {
            i64::MIN
        } else {
            i64::MAX
        }
    } else if neg {
        (acc as i64).wrapping_neg()
    } else {
        acc as i64
    };
    let end = if any != 0 { i - 1 } else { 0 };
    (val, end, erange)
}

/// `collator_u_strtod`: the value and the end index (0 when nothing
/// numeric was read).
fn u_strtod(s: &[UChar]) -> (f64, usize) {
    let digit = |c: UChar| (b'0' as UChar..=b'9' as UChar).contains(&c);
    let mut u = 0;
    while is_space(at(s, u)) {
        u += 1;
    }
    let nstart = u;
    let mut c = at(s, u);
    if c == b'-' as UChar || c == b'+' as UChar {
        u += 1;
        c = at(s, u);
    }
    let mut any = false;
    while digit(c) {
        any = true;
        u += 1;
        c = at(s, u);
    }
    if c == b'.' as UChar {
        u += 1;
        c = at(s, u);
        while digit(c) {
            any = true;
            u += 1;
            c = at(s, u);
        }
    }
    if (c == b'e' as UChar || c == b'E' as UChar) && any {
        let e = u;
        let mut any_exp = false;
        u += 1;
        c = at(s, u);
        if c == b'-' as UChar || c == b'+' as UChar {
            u += 1;
            c = at(s, u);
        }
        while digit(c) {
            any_exp = true;
            u += 1;
            c = at(s, u);
        }
        if !any_exp {
            u = e;
        }
    }
    if any {
        let text: String = s[nstart..u].iter().map(|&c| c as u8 as char).collect();
        return (text.parse::<f64>().unwrap_or(0.0), u);
    }
    (0.0, 0)
}

fn skip_ws(s: &[UChar], mut i: usize) -> usize {
    while is_space(at(s, i)) {
        i += 1;
    }
    i
}

/// `collator_is_numeric(str, len, ..., allow_errors=false)`.
fn is_numeric(s: &[UChar]) -> Option<Numeric> {
    if s.is_empty() {
        return None;
    }
    let (lval, end, erange) = u_strtol(s);
    if !erange {
        let end = skip_ws(s, end);
        if end == s.len() {
            return Some(Numeric::Long(lval));
        } else if end == 0
            && at(s, 0) != 0
            && at(s, 0) != b'.' as UChar
            && at(s, 0) != b'-' as UChar
        {
            return None;
        }
    }
    let (dval, end) = u_strtod(s);
    if !(dval == 0.0 && end == 0) {
        let end = skip_ws(s, end);
        if end == s.len() {
            if !dval.is_finite() {
                return None;
            }
            return Some(Numeric::Double(dval));
        }
    }
    None
}

fn num_value(n: Numeric) -> Value {
    match n {
        Numeric::Long(l) => Value::Int(l),
        Numeric::Double(d) => Value::Float(d),
    }
}

/// `intl_convert_utf8_to_utf16` with php's warning-and-empty fallback
/// (collator_convert_zstr_utf8_to_utf16).
fn to_u16_warn(host: &mut dyn Host, s: &[u8], what: &str) -> Result<Vec<UChar>, Thrown> {
    match utf8_to_utf16(s) {
        Ok(u) => Ok(u),
        Err(_) => {
            host.emit_diagnostic(
                crate::E_WARNING,
                format!("Error casting object to string in {what}()").as_bytes(),
            )?;
            Ok(Vec::new())
        }
    }
}

/// `collator_convert_zstr_utf16_to_utf8`.
fn u16_to_value(host: &mut dyn Host, u: &[UChar]) -> Result<Value, Thrown> {
    match utf16_to_utf8(u) {
        Ok(s) => Ok(Value::str(s)),
        Err(_) => {
            host.emit_diagnostic(
                crate::E_WARNING,
                b"Error converting utf16 to utf8 in collator_convert_zval_utf16_to_utf8()",
            )?;
            Ok(Value::str(""))
        }
    }
}

/// `collator_convert_object_to_string`: an object with a string cast as
/// UTF-16; anything else as it is.
fn object_to_string(host: &mut dyn Host, it: &Item) -> Result<Item, Thrown> {
    if let Item::Other(v @ Value::Object(_)) = it {
        if let Some(s) = object_handlers::to_string(host, v)? {
            let u = to_u16_warn(host, &s, "collator_convert_object_to_string")?;
            return Ok(Item::U16(Rc::new(u)));
        }
    }
    Ok(it.clone())
}

/// `collator_normalize_sort_argument`: a UTF-16 string as a number when
/// numeric, else as UTF-8; anything else as it is.
fn normalize_arg(host: &mut dyn Host, it: &Item) -> Result<Value, Thrown> {
    match it {
        Item::U16(u) => match is_numeric(u) {
            Some(n) => Ok(num_value(n)),
            None => u16_to_value(host, u),
        },
        Item::Other(v) => Ok(v.clone()),
    }
}

fn ord(o: std::cmp::Ordering) -> i32 {
    o as i32
}

/// `collator_regular_compare_function`.
fn regular_compare(
    host: &mut dyn Host,
    c: *const UCollator,
    a: &Item,
    b: &Item,
) -> Result<i32, Thrown> {
    let s1 = object_to_string(host, a)?;
    let s2 = object_to_string(host, b)?;
    if let (Item::U16(u1), Item::U16(u2)) = (&s1, &s2) {
        let n1 = is_numeric(u1);
        let n2 = if n1.is_some() { is_numeric(u2) } else { None };
        match (n1, n2) {
            (Some(n1), Some(n2)) => {
                let r = crate::host::host_compare(host, &num_value(n1), &num_value(n2))?;
                Ok(ord(r))
            }
            _ => Ok(strcoll(c, u1, u2)),
        }
    } else {
        let n1 = normalize_arg(host, &s1)?;
        let n2 = normalize_arg(host, &s2)?;
        Ok(ord(crate::host::host_compare(host, &n1, &n2)?))
    }
}

/// `collator_numeric_compare_function`.
fn numeric_compare(a: &Item, b: &Item) -> i32 {
    let d = |it: &Item| match it {
        Item::U16(u) => match is_numeric(u) {
            Some(Numeric::Long(l)) => l as f64,
            Some(Numeric::Double(d)) => d,
            None => 0.0,
        },
        Item::Other(v) => rhp_values::conv::to_float(&deref(v)),
    };
    let (x, y) = (d(a), d(b));
    if x < y {
        -1
    } else if x > y {
        1
    } else {
        0
    }
}

/// `collator_zval_to_string`: strings are UTF-16 already.
fn zval_to_u16(host: &mut dyn Host, it: &Item) -> Result<Rc<Vec<UChar>>, Thrown> {
    match it {
        Item::U16(u) => Ok(u.clone()),
        Item::Other(v) => {
            let s = zpp::zval_get_string(host, &deref(v))?;
            Ok(Rc::new(to_u16_warn(
                host,
                &s,
                "collator_convert_zstr_utf8_to_utf16",
            )?))
        }
    }
}

/// `collator_icu_compare_function`.
fn icu_compare(
    host: &mut dyn Host,
    c: *const UCollator,
    a: &Item,
    b: &Item,
) -> Result<i32, Thrown> {
    let u1 = zval_to_u16(host, a)?;
    let u2 = zval_to_u16(host, b)?;
    Ok(strcoll(c, &u1, &u2))
}

/// `collator_sort_internal`.
fn sort_internal(
    host: &mut dyn Host,
    co: &CollatorObj,
    f: &str,
    arg: Option<&Value>,
    renumber: bool,
    flags: i64,
) -> Result<Value, Thrown> {
    let Some(r) = ref_arg(arg) else {
        return Ok(Value::Bool(false));
    };
    let Value::Array(arr) = deref(&r) else {
        return Ok(Value::Bool(false));
    };
    let next_free = arr.next_free();
    // zend_array_dup, then the strings to UTF-16.
    let mut items: Vec<(ArrayKey, Item)> = Vec::with_capacity(arr.len());
    for (k, v) in arr.iter() {
        let it = match v {
            Value::Str(s) => match utf8_to_utf16(s.as_bytes()) {
                Ok(u) => Item::U16(Rc::new(u)),
                Err(st) => {
                    co.err.borrow_mut().code = st;
                    error::check_status(
                        host,
                        &co.err,
                        f,
                        "Error converting hash from UTF-8 to UTF-16",
                    )?;
                    return Ok(Value::Bool(false));
                }
            },
            _ => Item::Other(v.clone()),
        };
        items.push((k.clone(), it));
    }
    drop(arr);
    let c = co.ucoll() as *const UCollator;
    let mut thrown: Option<Thrown> = None;
    let mut cmp = |host: &mut dyn Host, a: &Item, b: &Item| -> i32 {
        if thrown.is_some() {
            return 0;
        }
        let r = match flags {
            SORT_NUMERIC => Ok(numeric_compare(a, b)),
            SORT_STRING => icu_compare(host, c, a, b),
            _ => regular_compare(host, c, a, b),
        };
        match r {
            Ok(r) => r.signum(),
            Err(t) => {
                thrown = Some(t);
                0
            }
        }
    };
    crate::ext::standard::array::zend_sort(&mut items, &mut |x, y| cmp(host, &x.1, &y.1));
    if let Some(t) = thrown {
        return Err(t);
    }
    let mut out = PhpArray::new();
    for (k, it) in items {
        let v = match it {
            Item::U16(u) => match utf16_to_utf8(&u) {
                Ok(s) => Value::str(s),
                Err(st) => {
                    co.err.borrow_mut().code = st;
                    error::check_status(
                        host,
                        &co.err,
                        f,
                        "Error converting hash from UTF-16 to UTF-8",
                    )?;
                    return Ok(Value::Bool(false));
                }
            },
            Item::Other(v) => v,
        };
        if renumber {
            out.push(v);
        } else {
            out.insert(k, v);
        }
    }
    if !renumber {
        out.to_hash();
        out.make_mut().set_next_free(next_free);
    }
    co.err.borrow_mut().code = sys::U_ZERO_ERROR;
    error::error_set_code(sys::U_ZERO_ERROR);
    store_ref(&r, Value::Array(out));
    Ok(Value::Bool(true))
}

/// `collator_sort_with_sort_keys`.
fn sort_with_sort_keys(
    host: &mut dyn Host,
    co: &CollatorObj,
    f: &str,
    arg: Option<&Value>,
) -> Result<Value, Thrown> {
    let Some(r) = ref_arg(arg) else {
        return Ok(Value::Bool(false));
    };
    let Value::Array(arr) = deref(&r) else {
        return Ok(Value::Bool(false));
    };
    if arr.is_empty() {
        return Ok(Value::Bool(true));
    }
    let c = co.ucoll() as *const UCollator;
    let mut keyed: Vec<(Vec<u8>, Value)> = Vec::with_capacity(arr.len());
    for v in arr.values() {
        let u = match v {
            Value::Str(s) => match utf8_to_utf16(s.as_bytes()) {
                Ok(u) => u,
                Err(st) => {
                    error::error_set_code(st);
                    co.err.borrow_mut().code = st;
                    error::errors_set_custom_msg(host, &co.err, f, "Sort with sort keys failed")?;
                    return Ok(Value::Bool(false));
                }
            },
            _ => Vec::new(),
        };
        let mut k = sort_key(c, &u).unwrap_or_default();
        // strcmp stops at the key's NUL.
        if let Some(z) = k.iter().position(|&b| b == 0) {
            k.truncate(z);
        }
        keyed.push((k, v.clone()));
    }
    drop(arr);
    crate::ext::standard::array::zend_sort(&mut keyed, &mut |a, b| ord(a.0.cmp(&b.0)));
    let mut out = PhpArray::new();
    for (_, v) in keyed {
        out.push(v);
    }
    store_ref(&r, Value::Array(out));
    Ok(Value::Bool(true))
}

// --- registration --------------------------------------------------------------

const FUNCS: &[(&str, &str)] = &[
    ("collator_create", "create"),
    ("collator_compare", "compare"),
    ("collator_get_attribute", "getAttribute"),
    ("collator_set_attribute", "setAttribute"),
    ("collator_get_strength", "getStrength"),
    ("collator_set_strength", "setStrength"),
    ("collator_sort", "sort"),
    ("collator_sort_with_sort_keys", "sortWithSortKeys"),
    ("collator_asort", "asort"),
    ("collator_get_locale", "getLocale"),
    ("collator_get_error_code", "getErrorCode"),
    ("collator_get_error_message", "getErrorMessage"),
    ("collator_get_sort_key", "getSortKey"),
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
    f9 => 9, f10 => 10, f11 => 11, f12 => 12
);

/// `__collator($method, $this, ...$args)`: the `Collator` methods.
pub(super) fn fn_method(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let method = rhp_values::conv::to_string_bytes(&deref(&args[0]));
    let method = String::from_utf8_lossy(&method).into_owned();
    let this = deref(&args[1]);
    let this = (!matches!(this, Value::Null)).then_some(this);
    let fname = format!("Collator::{method}");
    dispatch(host, &method, &fname, this.as_ref(), &args[2..])
}
