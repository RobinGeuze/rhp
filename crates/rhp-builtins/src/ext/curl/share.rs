//! `CurlShareHandle` and `CurlSharePersistentHandle`: php-src's share.c.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_int, c_long};
use std::rc::Rc;

use rhp_values::{conv, PhpArray, Value};

use super::sys::{self, CURLSH};
use super::{cstr_bytes, global_init, long_arg, new_object, share_arg, zval_get_long, SHARE};
use crate::zend::zpp::deref;
use crate::{Host, Thrown};

/// `php_curlsh`.
pub(crate) struct Share {
    pub share: *mut CURLSH,
    pub errno: Cell<i32>,
    /// A CurlSharePersistentHandle: the handle belongs to the thread's
    /// persistent table, not to the object.
    pub persistent: bool,
}

impl Drop for Share {
    /// curl_share_free_obj.
    fn drop(&mut self) {
        if !self.persistent && !self.share.is_null() {
            unsafe { sys::curl_share_cleanup(self.share) };
        }
    }
}

thread_local! {
    /// `CURL_G(persistent_curlsh)`: per thread, as php's module globals
    /// under ZTS; kept across requests.
    static PERSISTENT: RefCell<HashMap<u64, usize>> = RefCell::new(HashMap::new());
}

fn register(id: u32, s: Share) {
    SHARE.with(|m| m.borrow_mut().insert(id, Rc::new(s)));
}

pub(super) fn fn_curl_share_init(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    global_init();
    let (v, h) = new_object(host, "CurlShareHandle")?;
    let share = unsafe { sys::curl_share_init() };
    register(
        h.borrow().id,
        Share {
            share,
            errno: Cell::new(0),
            persistent: false,
        },
    );
    Ok(v)
}

pub(super) fn fn_curl_share_close(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_share_close";
    crate::deprecated_fn(host, f, "8.5", "as it has no effect since PHP 8.0")?;
    share_arg(host, args, f)?;
    Ok(Value::Null)
}

pub(super) fn fn_curl_share_setopt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_share_setopt";
    let sh = share_arg(host, args, f)?;
    let option = long_arg(host, args, 1, f, "option")?;
    let zvalue = args.get(2).map(deref).unwrap_or(Value::Null);
    let error = if option == sys::php("CURLSHOPT_SHARE") || option == sys::php("CURLSHOPT_UNSHARE")
    {
        let v = zval_get_long(host, &zvalue)?;
        unsafe { sys::curl_share_setopt(sh.share, option as c_int, v as c_long) }
    } else {
        sh.errno.set(sys::internal("CURLSHE_BAD_OPTION") as i32);
        return Err(Thrown::error(
            "ValueError",
            format!("{f}(): Argument #2 ($option) is not a valid cURL share option"),
        ));
    };
    sh.errno.set(error);
    Ok(Value::Bool(error == 0))
}

pub(super) fn fn_curl_share_errno(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let sh = share_arg(host, args, "curl_share_errno")?;
    Ok(Value::Int(sh.errno.get() as i64))
}

/// php's `zval_try_get_long`: `None` when the value cannot be an int. A
/// float that is not an int warns or is deprecated as zend_dval_to_lval_safe
/// does, a leading-numeric string warns.
fn try_get_long(host: &mut dyn Host, v: &Value) -> Result<Option<i64>, Thrown> {
    // ZEND_DOUBLE_FITS_LONG (true for NaN)
    const TWO_63: f64 = 9_223_372_036_854_775_808.0;
    let fits = |f: f64| !(f >= TWO_63 || f < -TWO_63);
    let shown = |f: f64| String::from_utf8_lossy(&rhp_values::fmt::float_serialize(f)).into_owned();
    Ok(Some(match v {
        Value::Null | Value::Bool(false) => 0,
        Value::Bool(true) => 1,
        Value::Int(n) => *n,
        Value::Float(f) => {
            let f = *f;
            let l = if f.is_finite() && fits(f) {
                f as i64
            } else {
                host.emit_diagnostic(
                    crate::E_WARNING,
                    &format!(
                        "The float {} is not representable as an int, cast occurred",
                        shown(f)
                    ),
                )?;
                if f.is_finite() {
                    conv::to_int(v)
                } else {
                    0
                }
            };
            if l as f64 != f && fits(f) {
                host.emit_diagnostic(
                    crate::E_DEPRECATED,
                    &format!(
                        "Implicit conversion from float {} to int loses precision",
                        shown(f)
                    ),
                )?;
            }
            l
        }
        Value::Str(s) => {
            let b = s.as_bytes();
            let Some((num, end)) = conv::scan_number(b) else {
                return Ok(None);
            };
            if !b[end..]
                .iter()
                .all(|&c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
            {
                host.emit_diagnostic(crate::E_WARNING, "A non-numeric value encountered")?;
            }
            match num {
                conv::Num::Int(n) => n,
                conv::Num::Float(f) => {
                    // zend_dval_to_lval_cap
                    let l = if f.is_nan() {
                        0
                    } else if f.is_finite() && fits(f) {
                        f as i64
                    } else if f > 0.0 {
                        i64::MAX
                    } else {
                        i64::MIN
                    };
                    if l as f64 != f {
                        host.emit_diagnostic(
                            crate::E_DEPRECATED,
                            &format!(
                                "Implicit conversion from float-string \"{}\" to int loses precision",
                                String::from_utf8_lossy(b)
                            ),
                        )?;
                    }
                    l
                }
            }
        }
        _ => return Ok(None),
    }))
}

/// `curl_share_init_persistent()`: a share per set of options, reused by
/// later calls (and requests) on the same thread.
pub(super) fn fn_curl_share_init_persistent(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let f = "curl_share_init_persistent";
    let opts = crate::zend::zpp::array_param(host, args, 0, f)?;
    let value_err = |msg: &str| {
        Thrown::error(
            "ValueError",
            format!("{f}(): Argument #1 ($share_options) {msg}"),
        )
    };
    if opts.is_empty() {
        return Err(value_err("must not be empty"));
    }
    let locks = [
        sys::php("CURL_LOCK_DATA_DNS"),
        sys::php("CURL_LOCK_DATA_SSL_SESSION"),
        sys::php("CURL_LOCK_DATA_CONNECT"),
        sys::php("CURL_LOCK_DATA_PSL"),
    ];
    let mut persistent_id: u64 = 0;
    for (_, entry) in opts.iter() {
        let entry = deref(entry);
        let Some(option) = try_get_long(host, &entry)? else {
            return Err(Thrown::type_error(format!(
                "{f}(): Argument #1 ($share_options) must contain only int values, {} given",
                crate::zend::zpp::zval_value_name(host, &entry)
            )));
        };
        if option == sys::php("CURL_LOCK_DATA_COOKIE") {
            return Err(value_err(
                "must not contain CURL_LOCK_DATA_COOKIE because sharing cookies across PHP \
                 requests is unsafe",
            ));
        }
        match locks.iter().position(|&l| l == option) {
            Some(bit) => persistent_id |= 1 << bit,
            None => return Err(value_err("must contain only CURL_LOCK_DATA_* constants")),
        }
    }
    let mut list = PhpArray::new();
    for (bit, &l) in locks.iter().enumerate() {
        if persistent_id & (1 << bit) != 0 {
            list.push(Value::Int(l));
        }
    }
    global_init();
    let existing = PERSISTENT.with(|p| p.borrow().get(&persistent_id).copied());
    let share = match existing {
        Some(p) => p as *mut CURLSH,
        None => {
            let share = unsafe { sys::curl_share_init() };
            for (_, l) in list.iter() {
                let l = conv::to_int(l);
                let r = unsafe {
                    sys::curl_share_setopt(share, sys::php("CURLSHOPT_SHARE") as c_int, l as c_long)
                };
                if r != 0 {
                    unsafe { sys::curl_share_cleanup(share) };
                    let why = String::from_utf8_lossy(&cstr_bytes(unsafe {
                        sys::curl_share_strerror(r)
                    }))
                    .into_owned();
                    return Err(Thrown::error(
                        "Exception",
                        format!("Could not construct persistent cURL share handle: {why}"),
                    ));
                }
            }
            PERSISTENT.with(|p| p.borrow_mut().insert(persistent_id, share as usize));
            share
        }
    };
    let v = host
        .instantiate(
            b"CurlSharePersistentHandle",
            vec![(b"options".to_vec(), Value::Array(list))],
        )
        .ok_or_else(|| {
            Thrown::unsupported("curl", "cannot instantiate CurlSharePersistentHandle")
        })?;
    if let Value::Object(h) = &v {
        register(
            h.borrow().id,
            Share {
                share,
                errno: Cell::new(0),
                persistent: true,
            },
        );
    }
    Ok(v)
}
