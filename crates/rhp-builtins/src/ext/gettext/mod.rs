//! php's `gettext` extension (php-src ext/gettext/gettext.c) on libintl:
//! Homebrew's gettext on macOS (linked dynamically, LGPL), glibc's on
//! Linux. shim.c makes the calls through libintl.h.

use std::ffi::{c_char, c_int, c_ulong, CStr};

use rhp_values::ops::deref;
use rhp_values::Value;

use crate::zend::zpp::{int_arg, path_param, str_arg};
use crate::{Host, Thrown};

extern "C" {
    fn rhp_textdomain(d: *const c_char) -> *mut c_char;
    fn rhp_gettext(m: *const c_char) -> *mut c_char;
    fn rhp_dgettext(d: *const c_char, m: *const c_char) -> *mut c_char;
    fn rhp_dcgettext(d: *const c_char, m: *const c_char, c: c_int) -> *mut c_char;
    fn rhp_bindtextdomain(d: *const c_char, dir: *const c_char) -> *mut c_char;
    fn rhp_ngettext(m1: *const c_char, m2: *const c_char, n: c_ulong) -> *mut c_char;
    fn rhp_dngettext(
        d: *const c_char,
        m1: *const c_char,
        m2: *const c_char,
        n: c_ulong,
    ) -> *mut c_char;
    fn rhp_dcngettext(
        d: *const c_char,
        m1: *const c_char,
        m2: *const c_char,
        n: c_ulong,
        c: c_int,
    ) -> *mut c_char;
    fn rhp_bind_textdomain_codeset(d: *const c_char, cs: *const c_char) -> *mut c_char;
}

/// `PHP_GETTEXT_MAX_DOMAIN_LENGTH`.
const MAX_DOMAIN: usize = 1024;
/// `PHP_GETTEXT_MAX_MSGID_LENGTH`.
const MAX_MSGID: usize = 4096;

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("textdomain", fn_textdomain),
            ("gettext", fn_gettext),
            ("_", fn_gettext_alias),
            ("dgettext", fn_dgettext),
            ("dcgettext", fn_dcgettext),
            ("bindtextdomain", fn_bindtextdomain),
            ("ngettext", fn_ngettext),
            ("dngettext", fn_dngettext),
            ("dcngettext", fn_dcngettext),
            ("bind_textdomain_codeset", fn_bind_textdomain_codeset),
        ],
    );
}

fn pname(f: &str, n: usize) -> &'static str {
    rhp_bytecode::signatures::function(f.as_bytes())
        .and_then(|s| s.params.get(n - 1))
        .map_or("", |p| p.name)
}

fn value_error(f: &str, n: usize, msg: &str) -> Thrown {
    Thrown::error(
        "ValueError",
        format!("{f}(): Argument #{n} (${}) {msg}", pname(f, n)),
    )
}

fn sarg(host: &mut dyn Host, args: &[Value], i: usize, f: &str) -> Result<Vec<u8>, Thrown> {
    match args.get(i).map(deref) {
        Some(Value::Str(s)) => Ok(s.as_bytes().to_vec()),
        _ => str_arg(host, args, i, f, pname(f, i + 1)),
    }
}

fn opt_sarg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
) -> Result<Option<Vec<u8>>, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(None),
        _ => sarg(host, args, i, f).map(Some),
    }
}

/// `PHP_GETTEXT_DOMAIN_LENGTH_CHECK`.
fn domain_check(f: &str, n: usize, d: &[u8]) -> Result<(), Thrown> {
    if d.len() > MAX_DOMAIN {
        return Err(value_error(f, n, "is too long"));
    }
    if d.is_empty() {
        return Err(value_error(f, n, "must not be empty"));
    }
    Ok(())
}

/// `PHP_GETTEXT_LENGTH_CHECK`.
fn length_check(f: &str, n: usize, m: &[u8]) -> Result<(), Thrown> {
    if m.len() > MAX_MSGID {
        return Err(value_error(f, n, "is too long"));
    }
    Ok(())
}

/// `PHP_DCGETTEXT_CATEGORY_CHECK`.
fn category_check(f: &str, n: usize, c: i64) -> Result<(), Thrown> {
    if c == libc::LC_ALL as i64 {
        return Err(value_error(f, n, "cannot be LC_ALL"));
    }
    Ok(())
}

/// The bytes as the NUL-terminated C string php hands the library (cut at
/// an embedded NUL).
fn c(v: &[u8]) -> Vec<u8> {
    let mut b = v.to_vec();
    b.push(0);
    b
}

fn ptr(v: &[u8]) -> *const c_char {
    v.as_ptr().cast()
}

fn owned(p: *const c_char) -> Value {
    Value::str(unsafe { CStr::from_ptr(p) }.to_bytes().to_vec())
}

/// A lookup's result: the translation, or (the library returned the msgid
/// it was given) the msgid string itself, past any NUL.
fn translated(r: *const c_char, msgid_c: &[u8], msgid: Vec<u8>) -> Value {
    if r == ptr(msgid_c) {
        Value::str(msgid)
    } else {
        owned(r)
    }
}

fn fn_textdomain(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "textdomain";
    let d = opt_sarg(host, args, 0, f)?;
    if let Some(d) = &d {
        domain_check(f, 1, d)?;
        if d == b"0" {
            return Err(value_error(f, 1, "cannot be zero"));
        }
    }
    let dc = d.as_deref().map(c);
    let r = unsafe { rhp_textdomain(dc.as_deref().map_or(std::ptr::null(), ptr)) };
    Ok(owned(r))
}

fn gettext_impl(host: &mut dyn Host, args: &[Value], f: &str) -> Result<Value, Thrown> {
    let m = sarg(host, args, 0, f)?;
    length_check(f, 1, &m)?;
    let mc = c(&m);
    let r = unsafe { rhp_gettext(ptr(&mc)) };
    Ok(translated(r, &mc, m))
}

fn fn_gettext(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    gettext_impl(host, args, "gettext")
}

/// `_()`, gettext's alias: messages name `_`.
fn fn_gettext_alias(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    gettext_impl(host, args, "_")
}

fn fn_dgettext(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "dgettext";
    let d = sarg(host, args, 0, f)?;
    let m = sarg(host, args, 1, f)?;
    domain_check(f, 1, &d)?;
    length_check(f, 2, &m)?;
    let (dc, mc) = (c(&d), c(&m));
    let r = unsafe { rhp_dgettext(ptr(&dc), ptr(&mc)) };
    Ok(translated(r, &mc, m))
}

fn fn_dcgettext(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "dcgettext";
    let d = sarg(host, args, 0, f)?;
    let m = sarg(host, args, 1, f)?;
    let cat = int_arg(host, args, 2, f, pname(f, 3))?;
    domain_check(f, 1, &d)?;
    length_check(f, 2, &m)?;
    category_check(f, 3, cat)?;
    let (dc, mc) = (c(&d), c(&m));
    let r = unsafe { rhp_dcgettext(ptr(&dc), ptr(&mc), cat as c_int) };
    Ok(translated(r, &mc, m))
}

fn fn_bindtextdomain(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "bindtextdomain";
    let d = path_param(host, args, 0, f, pname(f, 1))?;
    let dir = match args.get(1).map(deref) {
        None | Some(Value::Null) => None,
        _ => Some(path_param(host, args, 1, f, pname(f, 2))?),
    };
    domain_check(f, 1, &d)?;
    let dc = c(&d);
    let Some(dir) = dir else {
        let r = unsafe { rhp_bindtextdomain(ptr(&dc), std::ptr::null()) };
        if r.is_null() {
            return Ok(Value::Bool(false));
        }
        return Ok(owned(r));
    };
    let resolved = if !dir.is_empty() && dir != b"0" {
        crate::zend::virtual_cwd::realpath_strict(&dir)
    } else {
        crate::zend::virtual_cwd::getcwd()
    };
    let Some(resolved) = resolved else {
        return Ok(Value::Bool(false));
    };
    let rc = c(&resolved);
    let r = unsafe { rhp_bindtextdomain(ptr(&dc), ptr(&rc)) };
    Ok(owned(r))
}

fn fn_ngettext(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "ngettext";
    let m1 = sarg(host, args, 0, f)?;
    let m2 = sarg(host, args, 1, f)?;
    let n = int_arg(host, args, 2, f, pname(f, 3))?;
    length_check(f, 1, &m1)?;
    length_check(f, 2, &m2)?;
    let (c1, c2) = (c(&m1), c(&m2));
    Ok(owned(unsafe {
        rhp_ngettext(ptr(&c1), ptr(&c2), n as c_ulong)
    }))
}

fn fn_dngettext(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "dngettext";
    let d = sarg(host, args, 0, f)?;
    let m1 = sarg(host, args, 1, f)?;
    let m2 = sarg(host, args, 2, f)?;
    let n = int_arg(host, args, 3, f, pname(f, 4))?;
    domain_check(f, 1, &d)?;
    length_check(f, 2, &m1)?;
    length_check(f, 3, &m2)?;
    let (dc, c1, c2) = (c(&d), c(&m1), c(&m2));
    Ok(owned(unsafe {
        rhp_dngettext(ptr(&dc), ptr(&c1), ptr(&c2), n as c_ulong)
    }))
}

fn fn_dcngettext(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "dcngettext";
    let d = sarg(host, args, 0, f)?;
    let m1 = sarg(host, args, 1, f)?;
    let m2 = sarg(host, args, 2, f)?;
    let n = int_arg(host, args, 3, f, pname(f, 4))?;
    let cat = int_arg(host, args, 4, f, pname(f, 5))?;
    domain_check(f, 1, &d)?;
    length_check(f, 2, &m1)?;
    length_check(f, 3, &m2)?;
    category_check(f, 5, cat)?;
    let (dc, c1, c2) = (c(&d), c(&m1), c(&m2));
    Ok(owned(unsafe {
        rhp_dcngettext(ptr(&dc), ptr(&c1), ptr(&c2), n as c_ulong, cat as c_int)
    }))
}

fn fn_bind_textdomain_codeset(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "bind_textdomain_codeset";
    let d = sarg(host, args, 0, f)?;
    let cs = opt_sarg(host, args, 1, f)?;
    domain_check(f, 1, &d)?;
    let dc = c(&d);
    let csc = cs.as_deref().map(c);
    let r = unsafe {
        rhp_bind_textdomain_codeset(ptr(&dc), csc.as_deref().map_or(std::ptr::null(), ptr))
    };
    if r.is_null() {
        return Ok(Value::Bool(false));
    }
    Ok(owned(r))
}
