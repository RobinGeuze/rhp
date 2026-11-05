//! `CurlMultiHandle`: php-src's multi.c. Transfers run on the VM thread,
//! inside `curl_multi_exec` (curl_multi_perform).

use std::cell::RefCell;
use std::ffi::{c_int, c_long, c_void};
use std::rc::Rc;

use rhp_values::{conv, PhpArray, Value};

use super::easy::{self, drive, Easy, Method};
use super::sys::{self, CURL, CURLM};
use super::{
    akey, cstr_bytes, easy_arg, easy_of, global_init, long_arg, multi_arg, new_object,
    zval_get_long, MULTI,
};
use crate::ext::libxml::ffi;
use crate::zend::zpp::deref;
use crate::{Host, Thrown};

/// `php_curlm`.
pub(crate) struct Multi {
    pub multi: *mut CURLM,
    /// The added CurlHandles (`easyh`), which the multi keeps alive.
    pub easyh: Vec<Value>,
    pub errno: i32,
    push: Option<Value>,
}

impl Drop for Multi {
    /// curl_multi_free_obj.
    fn drop(&mut self) {
        if !self.multi.is_null() {
            unsafe { sys::curl_multi_cleanup(self.multi) };
        }
        let handles = std::mem::take(&mut self.easyh);
        drop(handles);
    }
}

fn same_object(a: &Value, b: &Value) -> bool {
    match (deref(a), deref(b)) {
        (Value::Object(x), Value::Object(y)) => Rc::ptr_eq(&x, &y),
        _ => false,
    }
}

/// `_php_curl_multi_find_easy_handle`.
fn find_easy(m: &Multi, cp: *mut CURL) -> Option<(Value, Rc<RefCell<Easy>>)> {
    m.easyh.iter().find_map(|v| {
        let e = easy_of(v)?;
        let found = e.try_borrow().is_ok_and(|e| e.cp == cp);
        found.then(|| (v.clone(), e))
    })
}

pub(super) fn fn_curl_multi_init(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_multi_init";
    global_init();
    let multi = unsafe { sys::curl_multi_init() };
    if multi.is_null() {
        return Err(Thrown::error(
            "Error",
            format!("{f}(): Could not initialize a new cURL multi handle"),
        ));
    }
    let (v, h) = match new_object(host, "CurlMultiHandle") {
        Ok(x) => x,
        Err(t) => {
            unsafe { sys::curl_multi_cleanup(multi) };
            return Err(t);
        }
    };
    let m = Multi {
        multi,
        easyh: Vec::new(),
        errno: 0,
        push: None,
    };
    let id = h.borrow().id;
    MULTI.with(|map| map.borrow_mut().insert(id, Rc::new(RefCell::new(m))));
    Ok(v)
}

pub(super) fn fn_curl_multi_add_handle(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let f = "curl_multi_add_handle";
    let (_, m) = multi_arg(host, args, f)?;
    let (z_ch, e) = easy_arg(host, args, 1, f, "handle")?;
    easy::verify_handlers(host, f, &e, true)?;
    e.borrow_mut().cleanup_handle();
    let (multi, cp) = (m.borrow().multi, e.borrow().cp);
    let (error, thrown) = drive(host, f, || unsafe { sys::curl_multi_add_handle(multi, cp) });
    {
        let mut mh = m.borrow_mut();
        mh.errno = error;
        if error == 0 {
            mh.easyh.push(z_ch);
        }
    }
    thrown?;
    Ok(Value::Int(error as i64))
}

pub(super) fn fn_curl_multi_remove_handle(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let f = "curl_multi_remove_handle";
    let (_, m) = multi_arg(host, args, f)?;
    let (z_ch, e) = easy_arg(host, args, 1, f, "handle")?;
    let (multi, cp) = (m.borrow().multi, e.borrow().cp);
    // A transfer cut short logs through the debug callback.
    let (error, thrown) = drive(host, f, || unsafe {
        sys::curl_multi_remove_handle(multi, cp)
    });
    let removed = {
        let mut mh = m.borrow_mut();
        mh.errno = error;
        if error == 0 {
            mh.easyh
                .iter()
                .position(|v| same_object(v, &z_ch))
                .map(|i| mh.easyh.remove(i))
        } else {
            None
        }
    };
    drop(removed);
    thrown?;
    Ok(Value::Int(error as i64))
}

pub(super) fn fn_curl_multi_get_handles(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let (_, m) = multi_arg(host, args, "curl_multi_get_handles")?;
    let mut a = PhpArray::new();
    for v in &m.borrow().easyh {
        a.push(v.clone());
    }
    Ok(Value::Array(a))
}

/// `Z_PARAM_DOUBLE` in the caller's typing mode (a null was reported by
/// the VM's stub-signature check).
fn double_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
    p: &str,
) -> Result<f64, Thrown> {
    let v = args.get(i).map(deref).unwrap_or(Value::Null);
    let wrong = |host: &dyn Host, v: &Value| {
        Thrown::type_error(format!(
            "{f}(): Argument #{} (${p}) must be of type float, {} given",
            i + 1,
            crate::zend::zpp::zval_value_name(host, v)
        ))
    };
    match &v {
        Value::Float(x) => return Ok(*x),
        Value::Int(n) => return Ok(*n as f64),
        _ => {}
    }
    if host.caller_strict_types() {
        return Err(wrong(host, &v));
    }
    match &v {
        Value::Bool(b) => Ok(*b as i64 as f64),
        Value::Null => Ok(0.0),
        Value::Str(s) => match conv::numeric_string(s.as_bytes()) {
            Some(conv::Num::Int(n)) => Ok(n as f64),
            Some(conv::Num::Float(x)) => Ok(x),
            None => Err(wrong(host, &v)),
        },
        _ => Err(wrong(host, &v)),
    }
}

pub(super) fn fn_curl_multi_select(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_multi_select";
    let (_, m) = multi_arg(host, args, f)?;
    let timeout = if args.len() > 1 {
        double_arg(host, args, 1, f, "timeout")?
    } else {
        1.0
    };
    let max = i32::MAX as f64 / 1000.0;
    if !(timeout >= 0.0 && timeout <= max) {
        return Err(Thrown::error(
            "ValueError",
            format!("{f}(): Argument #2 ($timeout) must be between 0 and {max:.6}"),
        ));
    }
    let multi = m.borrow().multi;
    let mut numfds: c_int = 0;
    let error = unsafe {
        sys::curl_multi_wait(
            multi,
            std::ptr::null_mut(),
            0,
            (timeout * 1000.0) as c_int,
            &mut numfds,
        )
    };
    if error != 0 {
        m.borrow_mut().errno = error;
        return Ok(Value::Int(-1));
    }
    Ok(Value::Int(numfds as i64))
}

pub(super) fn fn_curl_multi_exec(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_multi_exec";
    let (_obj, m) = multi_arg(host, args, f)?;
    // The handles stay alive while callbacks run, even if one removes itself.
    let multi = m.borrow().multi;
    let handles: Vec<Rc<RefCell<Easy>>> = m.borrow().easyh.iter().filter_map(easy_of).collect();
    for e in &handles {
        easy::verify_handlers(host, f, e, true)?;
    }
    let slot = args.get(1).cloned().unwrap_or(Value::Null);
    let mut still_running = zval_get_long(host, &slot)? as c_int;
    let running: *mut c_int = &mut still_running;
    let (error, thrown) = drive(host, f, || unsafe {
        sys::curl_multi_perform(multi, running)
    });
    crate::zend::zpp::set_ref(args, 1, Value::Int(still_running as i64));
    m.borrow_mut().errno = error;
    drop(handles);
    thrown?;
    Ok(Value::Int(error as i64))
}

pub(super) fn fn_curl_multi_getcontent(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let (_, e) = easy_arg(host, args, 0, "curl_multi_getcontent", "handle")?;
    let ch = e.borrow();
    if ch.write.method == Method::Return {
        return Ok(Value::str(ch.buf.clone().unwrap_or_default()));
    }
    Ok(Value::Null)
}

pub(super) fn fn_curl_multi_info_read(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let (_, m) = multi_arg(host, args, "curl_multi_info_read")?;
    let multi = m.borrow().multi;
    let mut queued: c_int = 0;
    let msg = unsafe { sys::curl_multi_info_read(multi, &mut queued) };
    if msg.is_null() {
        return Ok(Value::Bool(false));
    }
    let msg = unsafe { &*msg };
    if args.len() > 1 {
        crate::zend::zpp::set_ref(args, 1, Value::Int(queued as i64));
    }
    let mut a = PhpArray::new();
    a.insert(akey("msg"), Value::Int(msg.msg as i64));
    a.insert(akey("result"), Value::Int(msg.result() as i64));
    let found = find_easy(&m.borrow(), msg.easy_handle);
    if let Some((v, e)) = found {
        // Saved so curl_error()/curl_errno() report it.
        e.borrow_mut().errno = msg.result();
        a.insert(akey("handle"), v);
    }
    Ok(Value::Array(a))
}

pub(super) fn fn_curl_multi_close(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_multi_close";
    let (_, m) = multi_arg(host, args, f)?;
    let (multi, handles) = {
        let mh = m.borrow();
        (mh.multi, mh.easyh.clone())
    };
    let mut thrown = Ok(());
    for v in &handles {
        if let Some(e) = easy_of(v) {
            easy::verify_handlers(host, f, &e, true)?;
            let cp = e.borrow().cp;
            let (_, r) = drive(host, f, || unsafe {
                sys::curl_multi_remove_handle(multi, cp)
            });
            if thrown.is_ok() {
                thrown = r;
            }
        }
    }
    let old = std::mem::take(&mut m.borrow_mut().easyh);
    drop(old);
    thrown?;
    Ok(Value::Null)
}

pub(super) fn fn_curl_multi_errno(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, m) = multi_arg(host, args, "curl_multi_errno")?;
    let n = m.borrow().errno;
    Ok(Value::Int(n as i64))
}

/// `_php_server_push_callback`.
unsafe extern "C" fn push_cb(
    parent: *mut CURL,
    easy: *mut CURL,
    num_headers: usize,
    headers: *mut sys::curl_pushheaders,
    userp: *mut c_void,
) -> c_int {
    let deny = sys::php("CURL_PUSH_DENY") as c_int;
    let ok = sys::php("CURL_PUSH_OK") as c_int;
    ffi::guard(deny, || {
        if userp.is_null() || ffi::aborted() {
            return deny;
        }
        let cell = unsafe { &*(userp as *const RefCell<Multi>) };
        let Ok((found, fcc)) = cell
            .try_borrow()
            .map(|m| (find_easy(&m, parent), m.push.clone()))
        else {
            return deny;
        };
        let (Some((parent_v, parent_e)), Some(fcc)) = (found, fcc) else {
            return deny;
        };
        let Some((child_v, child)) = ffi::call_host(|h| easy::wrap_pushed(h, easy, &parent_e))
        else {
            return deny;
        };
        let mut list = PhpArray::new();
        for i in 0..num_headers {
            list.push(Value::str(cstr_bytes(unsafe {
                sys::curl_pushheader_bynum(headers, i)
            })));
        }
        let args = [parent_v, child_v.clone(), Value::Array(list)];
        let r = ffi::call_host(|h| {
            let r = h.call_callable(&fcc, &args)?;
            zval_get_long(h, &r)
        });
        match r {
            Some(n) if n != deny as i64 => {
                if let Ok(mut m) = cell.try_borrow_mut() {
                    m.easyh.push(child_v);
                }
                ok
            }
            _ => {
                // libcurl frees the declined handle.
                easy::forget_cp(&child);
                deny
            }
        }
    })
}

pub(super) fn fn_curl_multi_setopt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "curl_multi_setopt";
    let (_, m) = multi_arg(host, args, f)?;
    let option = long_arg(host, args, 1, f, "option")?;
    let zvalue = args.get(2).map(deref).unwrap_or(Value::Null);
    let multi = m.borrow().multi;
    const LONGS: &[&str] = &[
        "CURLMOPT_PIPELINING",
        "CURLMOPT_MAXCONNECTS",
        "CURLMOPT_CHUNK_LENGTH_PENALTY_SIZE",
        "CURLMOPT_CONTENT_LENGTH_PENALTY_SIZE",
        "CURLMOPT_MAX_HOST_CONNECTIONS",
        "CURLMOPT_MAX_PIPELINE_LENGTH",
        "CURLMOPT_MAX_TOTAL_CONNECTIONS",
        "CURLMOPT_MAX_CONCURRENT_STREAMS",
    ];
    let is = |n: &str| sys::php_defined(n) && sys::php(n) == option;
    let error = if LONGS.iter().any(|n| is(n)) {
        let lval = zval_get_long(host, &zvalue)?;
        if is("CURLMOPT_PIPELINING") && lval & 1 != 0 {
            crate::warn(host, f, "CURLPIPE_HTTP1 is no longer supported")?;
        }
        unsafe { sys::curl_multi_setopt(multi, option as c_int, lval as c_long) }
    } else if is("CURLMOPT_PUSHFUNCTION") {
        m.borrow_mut().push = None;
        if let Some(why) = host.check_callable(&zvalue)? {
            return Err(Thrown::type_error(format!(
                "{f}(): Argument #2 ($option) must be a valid callback for option \
                 CURLMOPT_PUSHFUNCTION, {why}"
            )));
        }
        m.borrow_mut().push = Some(easy::bind_callable(host, &zvalue));
        let r = unsafe {
            sys::curl_multi_setopt(
                multi,
                option as c_int,
                push_cb as sys::PushFn as *const c_void,
            )
        };
        if r != 0 {
            return Ok(Value::Bool(false));
        }
        unsafe {
            sys::curl_multi_setopt(
                multi,
                sys::internal("CURLMOPT_PUSHDATA") as c_int,
                Rc::as_ptr(&m) as *const c_void,
            )
        }
    } else {
        m.borrow_mut().errno = sys::internal("CURLM_UNKNOWN_OPTION") as i32;
        return Err(Thrown::error(
            "ValueError",
            format!("{f}(): Argument #2 ($option) is not a valid cURL multi option"),
        ));
    };
    m.borrow_mut().errno = error;
    Ok(Value::Bool(error == 0))
}
