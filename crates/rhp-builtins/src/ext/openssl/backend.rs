//! php-src's openssl_backend_common.c and openssl_backend_v3.c: the helpers
//! the functions share (certificate, key and request loading, names, time,
//! verification stores, request configuration).

use std::ffi::{c_char, c_int, c_long, c_void, CStr, CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::ptr;

use rhp_values::{ArrayKey, PhpArray, Value};

use super::sys;
use super::{
    bio_file, cert_of, check_path, cstr_bytes, cstring, errors_restore_mark, errors_set_mark,
    key_of, libctx, mem_buf, openssl_free, store_errors, try_string, Own, PROPQ,
};
use crate::zend::zpp::deref;
use crate::{warn, Host, Thrown};

pub(super) const READ: &CStr = c"r";
pub(super) const WRITE: &CStr = c"w";

pub(super) fn key(k: &str) -> ArrayKey {
    ArrayKey::from_bytes(k.as_bytes())
}

// ---------------------------------------------------------------------------
// names, strings, times
// ---------------------------------------------------------------------------

/// php_openssl_add_assoc_name_entry: `key => [field => value…]`, or the
/// fields straight into `val` when `key` is None.
pub(super) fn add_assoc_name_entry(
    val: &mut PhpArray,
    key_name: Option<&str>,
    name: *const sys::X509_NAME,
    shortname: bool,
) {
    let mut sub = PhpArray::new();
    let target: &mut PhpArray = if key_name.is_some() { &mut sub } else { val };
    let n = unsafe { sys::X509_NAME_entry_count(name) };
    for i in 0..n {
        unsafe {
            let ne = sys::X509_NAME_get_entry(name, i);
            let obj = sys::X509_NAME_ENTRY_get_object(ne);
            let nid = sys::OBJ_obj2nid(obj);
            let sname = if shortname {
                sys::OBJ_nid2sn(nid)
            } else {
                sys::OBJ_nid2ln(nid)
            };
            let sname = cstr_bytes(sname).to_vec();
            let s = sys::X509_NAME_ENTRY_get_data(ne);
            let mut buf: *mut u8 = ptr::null_mut();
            let data: Option<Vec<u8>> = if sys::ASN1_STRING_type(s) != sys::V_ASN1_UTF8STRING {
                let len = sys::ASN1_STRING_to_UTF8(&mut buf, s);
                (len != -1).then(|| std::slice::from_raw_parts(buf, len as usize).to_vec())
            } else {
                let p = sys::ASN1_STRING_get0_data(s);
                let len = sys::ASN1_STRING_length(s);
                Some(std::slice::from_raw_parts(p, len.max(0) as usize).to_vec())
            };
            match data {
                Some(d) => {
                    let k = ArrayKey::from_bytes(&sname);
                    match target.get(&k).cloned() {
                        Some(Value::Array(mut a)) => {
                            a.push(Value::str(d));
                            target.insert(k, Value::Array(a));
                        }
                        Some(Value::Str(old)) => {
                            let mut a = PhpArray::new();
                            a.push(Value::Str(old));
                            a.push(Value::str(d));
                            target.insert(k, Value::Array(a));
                        }
                        Some(_) => {}
                        None => {
                            target.insert(k, Value::str(d));
                        }
                    }
                }
                None => store_errors(),
            }
            openssl_free(buf);
        }
    }
    if let Some(k) = key_name {
        val.insert(key(k), Value::Array(sub));
    }
}

/// php_openssl_add_assoc_asn1_string.
pub(super) fn asn1_string_bytes(s: *const sys::ASN1_STRING) -> Vec<u8> {
    unsafe {
        let p = sys::ASN1_STRING_get0_data(s);
        let n = sys::ASN1_STRING_length(s);
        if p.is_null() || n <= 0 {
            return Vec::new();
        }
        std::slice::from_raw_parts(p, n as usize).to_vec()
    }
}

/// php_openssl_asn1_time_to_time_t.
pub(super) fn asn1_time_to_time_t(
    host: &mut dyn Host,
    fname: &str,
    t: *const sys::ASN1_TIME,
) -> Result<i64, Thrown> {
    let ty = unsafe { sys::ASN1_STRING_type(t as *const sys::ASN1_STRING) };
    if ty != sys::V_ASN1_UTCTIME && ty != sys::V_ASN1_GENERALIZEDTIME {
        warn(host, fname, "Illegal ASN1 data type for timestamp")?;
        return Ok(-1);
    }
    let data = asn1_string_bytes(t as *const sys::ASN1_STRING);
    let cstr_len = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    if data.len() != cstr_len {
        warn(host, fname, "Illegal length in timestamp")?;
        return Ok(-1);
    }
    if data.len() < 13 || (ty == sys::V_ASN1_GENERALIZEDTIME && data.len() < 15) {
        warn(
            host,
            fname,
            &format!(
                "Unable to parse time string {} correctly",
                String::from_utf8_lossy(&data)
            ),
        )?;
        return Ok(-1);
    }
    // php works backwards from the end with atoi.
    let atoi = |s: &[u8]| -> i32 {
        let mut i = 0;
        while i < s.len() && (s[i] == b' ' || (9..=13).contains(&s[i])) {
            i += 1;
        }
        let neg = i < s.len() && s[i] == b'-';
        if i < s.len() && (s[i] == b'-' || s[i] == b'+') {
            i += 1;
        }
        let mut n: i32 = 0;
        while i < s.len() && s[i].is_ascii_digit() {
            n = n.wrapping_mul(10).wrapping_add((s[i] - b'0') as i32);
            i += 1;
        }
        if neg {
            -n
        } else {
            n
        }
    };
    let mut end = data.len() - 3;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_sec = atoi(&data[end..data.len() - 1]);
    let field = |end: &mut usize, w: usize| -> i32 {
        let v = atoi(&data[*end - w..*end]);
        *end -= w;
        v
    };
    tm.tm_min = field(&mut end, 2);
    tm.tm_hour = field(&mut end, 2);
    tm.tm_mday = field(&mut end, 2);
    tm.tm_mon = field(&mut end, 2) - 1;
    if ty == sys::V_ASN1_UTCTIME {
        tm.tm_year = field(&mut end, 2);
        if tm.tm_year < 68 {
            tm.tm_year += 100;
        }
    } else {
        tm.tm_year = field(&mut end, 4) - 1900;
    }
    // mktime then tm_gmtoff: the fields read as UTC.
    tm.tm_isdst = -1;
    let ret = unsafe { libc::mktime(&mut tm) };
    Ok(ret as i64 + tm.tm_gmtoff as i64)
}

// ---------------------------------------------------------------------------
// digests and ciphers
// ---------------------------------------------------------------------------

/// php_openssl_get_evp_md_by_name. Release with [`release_md`].
pub(super) fn md_by_name(name: &CStr) -> *const sys::EVP_MD {
    unsafe {
        let dp = sys::OBJ_NAME_get(name.as_ptr(), sys::OBJ_NAME_TYPE_MD_METH);
        if !dp.is_null() {
            return dp as *const sys::EVP_MD;
        }
        sys::EVP_MD_fetch(libctx(), name.as_ptr(), PROPQ)
    }
}

/// php_openssl_get_evp_md_from_algo.
pub(super) fn md_from_algo(algo: i64) -> *const sys::EVP_MD {
    let name: &CStr = match algo {
        1 => c"SHA1",
        2 => c"MD5",
        3 => c"MD4",
        6 => c"SHA224",
        7 => c"SHA256",
        8 => c"SHA384",
        9 => c"SHA512",
        10 => c"RIPEMD160",
        _ => return ptr::null(),
    };
    md_by_name(name)
}

/// php_openssl_release_evp_md (a no-op for OpenSSL's static methods).
pub(super) fn release_md(md: *const sys::EVP_MD) {
    if !md.is_null() {
        unsafe { sys::EVP_MD_free(md as *mut sys::EVP_MD) };
    }
}

/// php_openssl_get_evp_cipher_by_name.
pub(super) fn cipher_by_name(name: &CStr) -> *const sys::EVP_CIPHER {
    unsafe {
        let cp = sys::OBJ_NAME_get(name.as_ptr(), sys::OBJ_NAME_TYPE_CIPHER_METH);
        if !cp.is_null() {
            return cp as *const sys::EVP_CIPHER;
        }
        sys::EVP_CIPHER_fetch(libctx(), name.as_ptr(), PROPQ)
    }
}

/// php_openssl_get_evp_cipher_from_algo.
pub(super) fn cipher_from_algo(algo: i64) -> *const sys::EVP_CIPHER {
    let name: &CStr = match algo {
        0 => c"RC2-40-CBC",
        1 => c"RC2-CBC",
        2 => c"RC2-64-CBC",
        3 => c"DES-CBC",
        4 => c"DES-EDE3-CBC",
        5 => c"AES-128-CBC",
        6 => c"AES-192-CBC",
        7 => c"AES-256-CBC",
        _ => return ptr::null(),
    };
    cipher_by_name(name)
}

pub(super) fn release_cipher(c: *const sys::EVP_CIPHER) {
    if !c.is_null() {
        unsafe { sys::EVP_CIPHER_free(c as *mut sys::EVP_CIPHER) };
    }
}

// ---------------------------------------------------------------------------
// certificates
// ---------------------------------------------------------------------------

/// php_openssl_pem_read_asn1_bio_x509: a PEM certificate (no DER).
fn pem_read_asn1_bio_x509(bio: *mut sys::BIO) -> *mut sys::X509 {
    unsafe extern "C" fn d2i(a: *mut *mut c_void, pp: *mut *const u8, len: c_long) -> *mut c_void {
        unsafe { sys::d2i_X509(a.cast(), pp, len).cast() }
    }
    unsafe {
        let mut x = sys::X509_new_ex(libctx(), PROPQ);
        if x.is_null() {
            return x;
        }
        let r = sys::PEM_ASN1_read_bio(
            d2i,
            c"CERTIFICATE".as_ptr(),
            bio,
            (&mut x as *mut *mut sys::X509).cast(),
            None,
            ptr::null_mut(),
        );
        if r.is_null() {
            sys::X509_free(x);
            return ptr::null_mut();
        }
        x
    }
}

/// php_openssl_pem_read_bio_x509.
pub(super) fn pem_read_bio_x509(bio: *mut sys::BIO) -> *mut sys::X509 {
    unsafe {
        let mut x = sys::X509_new_ex(libctx(), PROPQ);
        if x.is_null() {
            return x;
        }
        if sys::PEM_read_bio_X509(bio, &mut x, None, ptr::null_mut()).is_null() {
            sys::X509_free(x);
            return ptr::null_mut();
        }
        x
    }
}

/// php_openssl_pem_read_bio_x509_req.
pub(super) fn pem_read_bio_x509_req(bio: *mut sys::BIO) -> *mut sys::X509_REQ {
    unsafe {
        let mut x = sys::X509_REQ_new_ex(libctx(), PROPQ);
        if x.is_null() {
            return x;
        }
        if sys::PEM_read_bio_X509_REQ(bio, &mut x, None, ptr::null_mut()).is_null() {
            sys::X509_REQ_free(x);
            return ptr::null_mut();
        }
        x
    }
}

/// php_openssl_pem_read_bio_public_key.
pub(super) fn pem_read_bio_public_key(bio: *mut sys::BIO) -> *mut sys::EVP_PKEY {
    unsafe {
        sys::PEM_read_bio_PUBKEY_ex(bio, ptr::null_mut(), None, ptr::null_mut(), libctx(), PROPQ)
    }
}

pub(super) unsafe extern "C" fn x509_free(x: *mut sys::X509) {
    unsafe { sys::X509_free(x) };
}

/// php_openssl_x509_from_str: `file://` path or PEM/DER data. NULL after
/// storing errors.
pub(super) fn x509_from_str(
    host: &mut dyn Host,
    fname: &str,
    s: &[u8],
    arg_num: usize,
    from_array: bool,
    option: Option<&str>,
) -> Result<Own<sys::X509>, Thrown> {
    let bio = if s.len() > 7 && s.starts_with(b"file://") {
        let Some(path) = check_path(host, fname, s, arg_num, true, from_array, option)? else {
            return Ok(Own::null());
        };
        let b = bio_file(&path, READ);
        if b.is_null() {
            store_errors();
            return Ok(Own::null());
        }
        (b, true)
    } else {
        let b = mem_buf(s);
        if b.is_null() {
            store_errors();
            return Ok(Own::null());
        }
        (b, false)
    };
    let cert = if bio.1 {
        pem_read_bio_x509(bio.0.p)
    } else {
        pem_read_asn1_bio_x509(bio.0.p)
    };
    drop(bio);
    if cert.is_null() {
        store_errors();
        return Ok(Own::null());
    }
    Ok(Own::new(cert, x509_free))
}

/// php_openssl_x509_from_param: the object's certificate (borrowed) or a
/// parsed one (owned).
pub(super) fn x509_from_param(
    host: &mut dyn Host,
    fname: &str,
    a: &super::ObjOrStr,
    arg_num: usize,
) -> Result<Own<sys::X509>, Thrown> {
    match a {
        super::ObjOrStr::Obj(o) => Ok(Own::borrowed(cert_of(o).unwrap_or(ptr::null_mut()))),
        super::ObjOrStr::Str(s) => x509_from_str(host, fname, s, arg_num, false, None),
    }
}

/// php_openssl_x509_from_zval.
pub(super) fn x509_from_zval(
    host: &mut dyn Host,
    fname: &str,
    v: &Value,
    arg_num: usize,
    from_array: bool,
    option: Option<&str>,
) -> Result<Own<sys::X509>, Thrown> {
    let v = deref(v);
    if let Some(x) = cert_of(&v) {
        return Ok(Own::borrowed(x));
    }
    let s = try_string(host, &v)?;
    x509_from_str(host, fname, &s, arg_num, from_array, option)
}

/// php_openssl_x509_fingerprint.
pub(super) fn x509_fingerprint(
    host: &mut dyn Host,
    fname: &str,
    peer: *mut sys::X509,
    method: &[u8],
    raw: bool,
) -> Result<Option<Vec<u8>>, Thrown> {
    let md = md_by_name(&cstring(method));
    if md.is_null() {
        warn(host, fname, "Unknown digest algorithm")?;
        return Ok(None);
    }
    let mut buf = [0u8; sys::EVP_MAX_MD_SIZE as usize];
    let mut n: u32 = 0;
    if unsafe { sys::X509_digest(peer, md, buf.as_mut_ptr(), &mut n) } == 0 {
        release_md(md);
        store_errors();
        host.emit_diagnostic(
            crate::E_ERROR,
            format!("{fname}(): Could not generate signature").as_bytes(),
        )?;
        return Ok(None);
    }
    release_md(md);
    let d = &buf[..n as usize];
    Ok(Some(if raw { d.to_vec() } else { hex(d).into_bytes() }))
}

pub(super) fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// openssl_x509v3_subjectAltName (CVE-2013-4073).
pub(super) fn subject_alt_name(bio: *mut sys::BIO, ext: *mut sys::X509_EXTENSION) -> bool {
    unsafe {
        let names = sys::X509V3_EXT_d2i(ext) as *mut sys::stack_st_GENERAL_NAME;
        if names.is_null() {
            store_errors();
            return false;
        }
        let st = names as *mut sys::OPENSSL_STACK;
        let num = sys::OPENSSL_sk_num(st);
        for i in 0..num {
            let name = sys::OPENSSL_sk_value(st, i) as *mut sys::GENERAL_NAME;
            let prefix: Option<&CStr> = match (*name).type_ {
                sys::GEN_EMAIL => Some(c"email:"),
                sys::GEN_DNS => Some(c"DNS:"),
                sys::GEN_URI => Some(c"URI:"),
                _ => None,
            };
            match prefix {
                Some(p) => {
                    sys::BIO_puts(bio, p.as_ptr());
                    let s = (*name).d as *const sys::ASN1_STRING;
                    sys::BIO_write(
                        bio,
                        sys::ASN1_STRING_get0_data(s).cast(),
                        sys::ASN1_STRING_length(s),
                    );
                }
                None => {
                    sys::GENERAL_NAME_print(bio, name);
                }
            }
            if i < num - 1 {
                sys::BIO_puts(bio, c", ".as_ptr());
            }
        }
        sys::OPENSSL_sk_pop_free(st, general_name_free);
    }
    true
}

unsafe extern "C" fn general_name_free(p: *mut c_void) {
    unsafe { sys::GENERAL_NAME_free(p.cast()) };
}

pub(super) unsafe extern "C" fn x509_free_void(p: *mut c_void) {
    unsafe { sys::X509_free(p.cast()) };
}

unsafe extern "C" fn x509_info_free_void(p: *mut c_void) {
    unsafe { sys::X509_INFO_free(p.cast()) };
}

/// A `STACK_OF(X509)` freed with its certificates.
pub(super) struct CertStack(pub *mut sys::OPENSSL_STACK);

impl Drop for CertStack {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { sys::OPENSSL_sk_pop_free(self.0, x509_free_void) };
        }
    }
}

impl CertStack {
    pub fn null() -> CertStack {
        CertStack(ptr::null_mut())
    }
    pub fn len(&self) -> i32 {
        if self.0.is_null() {
            0
        } else {
            unsafe { sys::OPENSSL_sk_num(self.0) }
        }
    }
}

/// php_openssl_load_all_certs_from_file.
pub(super) fn load_all_certs_from_file(
    host: &mut dyn Host,
    fname: &str,
    file: &[u8],
    arg_num: usize,
) -> Result<CertStack, Thrown> {
    let Some(path) = check_path(host, fname, file, arg_num, false, false, None)? else {
        return Ok(CertStack::null());
    };
    let shown = String::from_utf8_lossy(path.to_bytes()).into_owned();
    let bio = bio_file(&path, READ);
    if bio.is_null() {
        store_errors();
        warn(host, fname, &format!("Error opening the file, {shown}"))?;
        return Ok(CertStack::null());
    }
    let sk = unsafe {
        sys::PEM_X509_INFO_read_bio_ex(
            bio.p,
            ptr::null_mut(),
            None,
            ptr::null_mut(),
            libctx(),
            PROPQ,
        )
    };
    if sk.is_null() {
        store_errors();
        warn(host, fname, &format!("Error reading the file, {shown}"))?;
        return Ok(CertStack::null());
    }
    let stack = unsafe { sys::OPENSSL_sk_new_reserve(ptr::null_mut(), sys::OPENSSL_sk_num(sk)) };
    if stack.is_null() {
        store_errors();
        unsafe { sys::OPENSSL_sk_pop_free(sk, x509_info_free_void) };
        return Ok(CertStack::null());
    }
    unsafe {
        while sys::OPENSSL_sk_num(sk) > 0 {
            let xi = sys::OPENSSL_sk_shift(sk) as *mut sys::X509_INFO;
            if !(*xi).x509.is_null() {
                sys::OPENSSL_sk_push(stack, (*xi).x509.cast());
                (*xi).x509 = ptr::null_mut();
            }
            sys::X509_INFO_free(xi);
        }
        sys::OPENSSL_sk_free(sk);
    }
    let stack = CertStack(stack);
    if stack.len() == 0 {
        warn(host, fname, &format!("No certificates in file, {shown}"))?;
        return Ok(CertStack::null());
    }
    Ok(stack)
}

/// php_openssl_check_cert.
pub(super) fn check_cert(
    host: &mut dyn Host,
    fname: &str,
    store: *mut sys::X509_STORE,
    x: *mut sys::X509,
    untrusted: *mut sys::OPENSSL_STACK,
    purpose: c_int,
) -> Result<c_int, Thrown> {
    unsafe {
        let csc = sys::X509_STORE_CTX_new();
        if csc.is_null() {
            store_errors();
            host.emit_diagnostic(
                crate::E_ERROR,
                format!("{fname}(): Memory allocation failure").as_bytes(),
            )?;
            return Ok(0);
        }
        if sys::X509_STORE_CTX_init(csc, store, x, untrusted.cast()) == 0 {
            sys::X509_STORE_CTX_free(csc);
            store_errors();
            warn(host, fname, "Certificate store initialization failed")?;
            return Ok(0);
        }
        if purpose >= 0 && sys::X509_STORE_CTX_set_purpose(csc, purpose) == 0 {
            store_errors();
        }
        let ret = sys::X509_verify_cert(csc);
        if ret < 0 {
            store_errors();
        }
        sys::X509_STORE_CTX_free(csc);
        Ok(ret)
    }
}

pub(super) unsafe extern "C" fn x509_store_free(s: *mut sys::X509_STORE) {
    unsafe { sys::X509_STORE_free(s) };
}

/// php_openssl_setup_verify: a store with the files and directories of
/// `calist`, else OpenSSL's defaults.
pub(super) fn setup_verify(
    host: &mut dyn Host,
    fname: &str,
    calist: Option<&Value>,
    arg_num: usize,
) -> Result<Own<sys::X509_STORE>, Thrown> {
    let store = Own::new(unsafe { sys::X509_STORE_new() }, x509_store_free);
    if store.is_null() {
        store_errors();
        return Ok(store);
    }
    let (mut ndirs, mut nfiles) = (0, 0);
    if let Some(Value::Array(a)) = calist.map(deref) {
        for item in a.values() {
            let s = try_string(host, item)?;
            let Some(path) = check_path(host, fname, &s, arg_num, false, true, None)? else {
                continue;
            };
            let shown = path.to_bytes();
            let meta = std::fs::metadata(OsStr::from_bytes(shown));
            let Ok(meta) = meta else {
                warn(host, fname, &[b"Unable to stat ", shown].concat())?;
                continue;
            };
            unsafe {
                if meta.is_file() {
                    let l = sys::X509_STORE_add_lookup(store.p, sys::X509_LOOKUP_file());
                    if l.is_null()
                        || sys::X509_LOOKUP_ctrl(
                            l,
                            sys::X509_L_FILE_LOAD,
                            path.as_ptr(),
                            sys::X509_FILETYPE_PEM as c_long,
                            ptr::null_mut(),
                        ) == 0
                    {
                        store_errors();
                        warn(host, fname, &[b"Error loading file ", shown].concat())?;
                    } else {
                        nfiles += 1;
                    }
                } else {
                    let l = sys::X509_STORE_add_lookup(store.p, sys::X509_LOOKUP_hash_dir());
                    if l.is_null()
                        || sys::X509_LOOKUP_ctrl(
                            l,
                            sys::X509_L_ADD_DIR,
                            path.as_ptr(),
                            sys::X509_FILETYPE_PEM as c_long,
                            ptr::null_mut(),
                        ) == 0
                    {
                        store_errors();
                        warn(host, fname, &[b"Error loading directory ", shown].concat())?;
                    } else {
                        ndirs += 1;
                    }
                }
            }
        }
    }
    unsafe {
        if nfiles == 0 {
            let l = sys::X509_STORE_add_lookup(store.p, sys::X509_LOOKUP_file());
            if l.is_null()
                || sys::X509_LOOKUP_ctrl(
                    l,
                    sys::X509_L_FILE_LOAD,
                    ptr::null(),
                    sys::X509_FILETYPE_DEFAULT as c_long,
                    ptr::null_mut(),
                ) == 0
            {
                store_errors();
            }
        }
        if ndirs == 0 {
            let l = sys::X509_STORE_add_lookup(store.p, sys::X509_LOOKUP_hash_dir());
            if l.is_null()
                || sys::X509_LOOKUP_ctrl(
                    l,
                    sys::X509_L_ADD_DIR,
                    ptr::null(),
                    sys::X509_FILETYPE_DEFAULT as c_long,
                    ptr::null_mut(),
                ) == 0
            {
                store_errors();
            }
        }
    }
    Ok(store)
}

/// php_openssl_array_to_X509_sk.
pub(super) fn array_to_x509_sk(
    host: &mut dyn Host,
    fname: &str,
    certs: &Value,
    arg_num: usize,
    option: Option<&str>,
) -> Result<CertStack, Thrown> {
    let sk = CertStack(unsafe { sys::OPENSSL_sk_new_null() });
    if sk.0.is_null() {
        return Ok(sk);
    }
    let certs = deref(certs);
    let items: Vec<(Value, bool)> = match &certs {
        Value::Array(a) => a.values().map(|v| (v.clone(), true)).collect(),
        v => vec![(v.clone(), false)],
    };
    for (v, from_array) in items {
        let cert = x509_from_zval(host, fname, &v, arg_num, from_array, option)?;
        if cert.is_null() {
            return Ok(sk);
        }
        let x = if cert.is_owned() {
            cert.release()
        } else {
            let d = unsafe { sys::X509_dup(cert.p) };
            if d.is_null() {
                store_errors();
                return Ok(sk);
            }
            d
        };
        if unsafe { sys::OPENSSL_sk_push(sk.0, x.cast()) } <= 0 {
            unsafe { sys::X509_free(x) };
            store_errors();
            return Ok(CertStack::null());
        }
    }
    Ok(sk)
}

// ---------------------------------------------------------------------------
// keys
// ---------------------------------------------------------------------------

pub(super) unsafe extern "C" fn pkey_free(k: *mut sys::EVP_PKEY) {
    unsafe { sys::EVP_PKEY_free(k) };
}

/// php_openssl_extract_public_key: through PEM.
pub(super) fn extract_public_key(k: *mut sys::EVP_PKEY) -> *mut sys::EVP_PKEY {
    let bio = super::mem_bio();
    if bio.is_null() || unsafe { sys::PEM_write_bio_PUBKEY(bio.p, k) } == 0 {
        return ptr::null_mut();
    }
    pem_read_bio_public_key(bio.p)
}

struct Password<'a>(Option<&'a [u8]>);

unsafe extern "C" fn pem_password_cb(
    buf: *mut c_char,
    size: c_int,
    _rwflag: c_int,
    userdata: *mut c_void,
) -> c_int {
    super::guard(0, || {
        let pw = userdata as *const Password;
        let Some(key) = (unsafe { pw.as_ref() }).and_then(|p| p.0) else {
            return -1;
        };
        // php's struct holds the length as an int.
        let len = key.len() as c_int;
        let n = len.min(size);
        if n > 0 {
            unsafe { ptr::copy_nonoverlapping(key.as_ptr(), buf.cast::<u8>(), n as usize) };
        }
        n
    })
}

/// php_openssl_pem_read_bio_private_key, with php_openssl_pem_password_cb
/// when a passphrase is given.
pub(super) fn pem_read_bio_private_key(
    bio: *mut sys::BIO,
    passphrase: Option<&[u8]>,
) -> *mut sys::EVP_PKEY {
    let pw = Password(passphrase);
    super::with_callbacks(|| unsafe {
        match passphrase {
            None => sys::PEM_read_bio_PrivateKey_ex(
                bio,
                ptr::null_mut(),
                None,
                ptr::null_mut(),
                libctx(),
                PROPQ,
            ),
            Some(_) => sys::PEM_read_bio_PrivateKey_ex(
                bio,
                ptr::null_mut(),
                Some(pem_password_cb),
                &pw as *const Password as *mut c_void,
                libctx(),
                PROPQ,
            ),
        }
    })
}

/// php_openssl_pkey_from_zval: a new reference to the key `val` names (an
/// object, `[key, passphrase]`, PEM data or a `file://` path; for a public
/// key also a certificate). NULL after storing errors.
pub(super) fn pkey_from_zval(
    host: &mut dyn Host,
    fname: &str,
    val: &Value,
    public_key: bool,
    passphrase: Option<&[u8]>,
    arg_num: usize,
) -> Result<Own<sys::EVP_PKEY>, Thrown> {
    let mut val = deref(val);
    let mut phrase: Option<Vec<u8>> = passphrase.map(<[u8]>::to_vec);
    let null = || Ok(Own::null());
    if let Value::Array(a) = &val {
        let Some(zphrase) = a.get(&ArrayKey::Int(1)).map(deref) else {
            super::throw(Thrown::error(
                "ValueError",
                "Key array must be of the form array(0 => key, 1 => phrase)",
            ));
            return null();
        };
        phrase = Some(match &zphrase {
            Value::Str(s) => s.as_bytes().to_vec(),
            other => try_string(host, other)?,
        });
        let Some(k) = a.get(&ArrayKey::Int(0)).map(deref) else {
            super::throw(Thrown::error(
                "ValueError",
                "Key array must be of the form array(0 => key, 1 => phrase)",
            ));
            return null();
        };
        val = k;
    }
    let mut cert: Own<sys::X509> = Own::null();
    let mut key: *mut sys::EVP_PKEY = ptr::null_mut();
    if let Some((k, is_priv)) = key_of(&val) {
        if !public_key && !is_priv {
            warn(host, fname, "Supplied key param is a public key")?;
            return null();
        }
        if public_key && is_priv {
            warn(
                host,
                fname,
                "Don't know how to get public key from this private key",
            )?;
            return null();
        }
        unsafe { sys::EVP_PKEY_up_ref(k) };
        return Ok(Own::new(k, pkey_free));
    } else if let Some(x) = cert_of(&val) {
        cert = Own::borrowed(x);
    } else {
        if !matches!(val, Value::Str(_) | Value::Object(_)) {
            return null();
        }
        let s = try_string(host, &val)?;
        let mut file: Option<CString> = None;
        if s.len() > 7 && s.starts_with(b"file://") {
            match check_path(host, fname, &s, arg_num, true, false, None)? {
                Some(p) => file = Some(p),
                None => return null(),
            }
        }
        let open = |file: &Option<CString>| match file {
            Some(p) => bio_file(p, READ),
            None => mem_buf(&s),
        };
        if public_key {
            errors_set_mark();
            cert = x509_from_str(host, fname, &s, arg_num, false, None)?;
            if cert.is_null() {
                errors_restore_mark();
                let bio = open(&file);
                if bio.is_null() {
                    store_errors();
                    return null();
                }
                key = pem_read_bio_public_key(bio.p);
            }
        } else {
            let bio = open(&file);
            if bio.is_null() {
                return null();
            }
            key = pem_read_bio_private_key(bio.p, phrase.as_deref());
        }
    }
    if key.is_null() {
        store_errors();
        if public_key && !cert.is_null() {
            key = unsafe { sys::X509_get_pubkey(cert.p) };
            if key.is_null() {
                store_errors();
            }
        }
    }
    Ok(Own::new(key, pkey_free))
}

/// php_openssl_add_bn_to_array.
pub(super) fn add_bn(a: &mut PhpArray, bn: *const sys::BIGNUM, name: &str) {
    if bn.is_null() {
        return;
    }
    let len = (unsafe { sys::BN_num_bits(bn) } + 7) / 8;
    let mut buf = vec![0u8; len.max(0) as usize];
    unsafe { sys::BN_bn2bin(bn, buf.as_mut_ptr()) };
    a.insert(key(name), Value::str(buf));
}

/// `OPENSSL_PKEY_SET_BN`: the binary string `data[name]` as a BIGNUM.
pub(super) fn bn_from(data: &PhpArray, name: &str) -> Bn {
    match data.get(&key(name)).map(deref) {
        Some(Value::Str(s)) => Bn(unsafe {
            sys::BN_bin2bn(
                s.as_bytes().as_ptr(),
                s.len().min(c_int::MAX as usize) as c_int,
                ptr::null_mut(),
            )
        }),
        _ => Bn(ptr::null_mut()),
    }
}

/// A BIGNUM freed on drop.
pub(super) struct Bn(pub *mut sys::BIGNUM);

impl Drop for Bn {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { sys::BN_free(self.0) };
        }
    }
}

impl Bn {
    pub fn is_some(&self) -> bool {
        !self.0.is_null()
    }
}

// ---------------------------------------------------------------------------
// request configuration (php_x509_request)
// ---------------------------------------------------------------------------

pub(super) unsafe extern "C" fn conf_free(c: *mut sys::CONF) {
    unsafe { sys::NCONF_free(c) };
}

/// php_openssl_nconf_new.
pub(super) fn nconf_new() -> Own<sys::CONF> {
    Own::new(
        unsafe { sys::NCONF_new_ex(libctx(), ptr::null_mut()) },
        conf_free,
    )
}

/// php_openssl_conf_get_string: errors for a missing value are dropped.
pub(super) fn conf_get_string(
    conf: *mut sys::CONF,
    group: Option<&CStr>,
    name: &CStr,
) -> Option<CString> {
    unsafe {
        sys::ERR_set_mark();
        let s = sys::NCONF_get_string(conf, group.map_or(ptr::null(), CStr::as_ptr), name.as_ptr());
        sys::ERR_pop_to_mark();
        (!s.is_null()).then(|| CStr::from_ptr(s).to_owned())
    }
}

/// php_openssl_conf_get_number.
pub(super) fn conf_get_number(conf: *mut sys::CONF, group: &CStr, name: &CStr) -> c_long {
    let mut res: c_long = 0;
    unsafe {
        sys::ERR_set_mark();
        sys::NCONF_get_number_e(conf, group.as_ptr(), name.as_ptr(), &mut res);
        sys::ERR_pop_to_mark();
    }
    res
}

/// The `CONF_VALUE`s of a configuration section.
pub(super) fn conf_section(
    conf: *mut sys::CONF,
    section: &CStr,
) -> Option<Vec<(CString, CString)>> {
    unsafe {
        let sk = sys::NCONF_get_section(conf, section.as_ptr());
        if sk.is_null() {
            return None;
        }
        let mut out = Vec::new();
        for i in 0..sys::OPENSSL_sk_num(sk) {
            let v = sys::OPENSSL_sk_value(sk, i) as *const sys::CONF_VALUE;
            out.push((
                CStr::from_ptr((*v).name).to_owned(),
                CStr::from_ptr((*v).value).to_owned(),
            ));
        }
        Some(out)
    }
}

pub(super) struct Req {
    pub global_config: Own<sys::CONF>,
    pub req_config: Own<sys::CONF>,
    pub md_alg: *const sys::EVP_MD,
    pub section_name: CString,
    pub config_filename: CString,
    pub digest_name: Option<CString>,
    pub extensions_section: Option<CString>,
    pub request_extensions_section: Option<CString>,
    pub priv_key_bits: c_int,
    pub priv_key_type: c_int,
    pub priv_key_encrypt: bool,
    pub curve_name: c_int,
    pub priv_key: Own<sys::EVP_PKEY>,
    pub priv_key_encrypt_cipher: *const sys::EVP_CIPHER,
}

impl Req {
    /// PHP_SSL_REQ_INIT.
    pub fn new() -> Req {
        Req {
            global_config: Own::null(),
            req_config: Own::null(),
            md_alg: ptr::null(),
            section_name: CString::default(),
            config_filename: CString::default(),
            digest_name: None,
            extensions_section: None,
            request_extensions_section: None,
            priv_key_bits: 0,
            priv_key_type: 0,
            priv_key_encrypt: false,
            curve_name: 0,
            priv_key: Own::null(),
            priv_key_encrypt_cipher: ptr::null(),
        }
    }
}

impl Drop for Req {
    /// php_openssl_dispose_config.
    fn drop(&mut self) {
        if !self.md_alg.is_null() && self.md_alg != unsafe { sys::EVP_md_null() } {
            release_md(self.md_alg);
        }
        release_cipher(self.priv_key_encrypt_cipher);
    }
}

/// `SET_OPTIONAL_STRING_ARG`.
fn optional_string(args: Option<&PhpArray>, name: &str, def: Option<CString>) -> Option<CString> {
    if let Some(Value::Str(s)) = args.and_then(|a| a.get(&key(name))).map(deref) {
        return Some(cstring(s.as_bytes()));
    }
    if def.is_none() {
        store_errors();
    }
    def
}

/// `SET_OPTIONAL_LONG_ARG`.
fn optional_long(args: Option<&PhpArray>, name: &str, def: c_int) -> c_int {
    match args.and_then(|a| a.get(&key(name))).map(deref) {
        Some(Value::Int(n)) => n as c_int,
        _ => def,
    }
}

/// php_openssl_config_check_syntax.
fn config_check_syntax(
    host: &mut dyn Host,
    fname: &str,
    label: &str,
    config_filename: &CStr,
    section: &CStr,
    config: *mut sys::CONF,
) -> Result<bool, Thrown> {
    let mut ctx: sys::X509V3_CTX = unsafe { std::mem::zeroed() };
    unsafe {
        // X509V3_set_ctx_test
        sys::X509V3_set_ctx(
            &mut ctx,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            1,
        );
        sys::X509V3_set_nconf(&mut ctx, config);
        if sys::X509V3_EXT_add_nconf(config, &mut ctx, section.as_ptr(), ptr::null_mut()) == 0 {
            store_errors();
            warn(
                host,
                fname,
                &format!(
                    "Error loading {label} section {} of {}",
                    section.to_string_lossy(),
                    config_filename.to_string_lossy()
                ),
            )?;
            return Ok(false);
        }
    }
    Ok(true)
}

/// php_openssl_add_oid_section.
fn add_oid_section(host: &mut dyn Host, fname: &str, req: &Req) -> Result<bool, Thrown> {
    let Some(sect) = conf_get_string(req.req_config.p, None, c"oid_section") else {
        return Ok(true);
    };
    let Some(values) = conf_section(req.req_config.p, &sect) else {
        store_errors();
        warn(
            host,
            fname,
            &format!("Problem loading oid section {}", sect.to_string_lossy()),
        )?;
        return Ok(false);
    };
    for (name, value) in values {
        unsafe {
            if sys::OBJ_sn2nid(name.as_ptr()) == sys::NID_undef
                && sys::OBJ_ln2nid(name.as_ptr()) == sys::NID_undef
                && sys::OBJ_create(value.as_ptr(), name.as_ptr(), name.as_ptr()) == sys::NID_undef
            {
                store_errors();
                warn(
                    host,
                    fname,
                    &format!(
                        "Problem creating object {}={}",
                        name.to_string_lossy(),
                        value.to_string_lossy()
                    ),
                )?;
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// php_openssl_parse_config: `Ok(false)` is FAILURE.
pub(super) fn parse_config(
    host: &mut dyn Host,
    fname: &str,
    req: &mut Req,
    args: Option<&PhpArray>,
) -> Result<bool, Thrown> {
    let default_conf = super::conf_filename().to_owned();
    req.config_filename =
        optional_string(args, "config", Some(default_conf.clone())).unwrap_or_default();
    req.section_name =
        optional_string(args, "config_section_name", Some(c"req".to_owned())).unwrap_or_default();
    req.global_config = nconf_new();
    if unsafe { sys::NCONF_load(req.global_config.p, default_conf.as_ptr(), ptr::null_mut()) } == 0
    {
        store_errors();
    }
    req.req_config = nconf_new();
    if unsafe {
        sys::NCONF_load(
            req.req_config.p,
            req.config_filename.as_ptr(),
            ptr::null_mut(),
        )
    } == 0
    {
        return Ok(false);
    }
    // read in the oids
    if let Some(s) = conf_get_string(req.req_config.p, None, c"oid_file") {
        if let Some(path) =
            check_path(host, fname, s.to_bytes(), 0, false, false, Some("oid_file"))?
        {
            let bio = bio_file(&path, READ);
            if !bio.is_null() {
                unsafe { sys::OBJ_create_objects(bio.p) };
                drop(bio);
                store_errors();
            }
        }
    }
    if !add_oid_section(host, fname, req)? {
        return Ok(false);
    }
    let section = req.section_name.clone();
    let cfg = req.req_config.p;
    req.digest_name = optional_string(
        args,
        "digest_alg",
        conf_get_string(cfg, Some(&section), c"default_md"),
    );
    req.extensions_section = optional_string(
        args,
        "x509_extensions",
        conf_get_string(cfg, Some(&section), c"x509_extensions"),
    );
    req.request_extensions_section = optional_string(
        args,
        "req_extensions",
        conf_get_string(cfg, Some(&section), c"req_extensions"),
    );
    req.priv_key_bits = optional_long(
        args,
        "private_key_bits",
        conf_get_number(cfg, &section, c"default_bits") as c_int,
    );
    req.priv_key_type = optional_long(args, "private_key_type", 0);

    if let Some(item) = args.and_then(|a| a.get(&key("encrypt_key"))).map(deref) {
        req.priv_key_encrypt = matches!(item, Value::Bool(true));
    } else {
        let s = conf_get_string(cfg, Some(&section), c"encrypt_rsa_key")
            .or_else(|| conf_get_string(cfg, Some(&section), c"encrypt_key"));
        req.priv_key_encrypt = s.is_none_or(|s| s.to_bytes() != b"no");
    }
    match args
        .and_then(|a| a.get(&key("encrypt_key_cipher")))
        .map(deref)
    {
        Some(Value::Int(algo)) if req.priv_key_encrypt => {
            let c = cipher_from_algo(algo);
            if c.is_null() {
                warn(host, fname, "Unknown cipher algorithm for private key")?;
                return Ok(false);
            }
            req.priv_key_encrypt_cipher = c;
        }
        _ => req.priv_key_encrypt_cipher = ptr::null(),
    }

    // digest alg
    if req.digest_name.is_none() {
        req.digest_name = conf_get_string(cfg, Some(&section), c"default_md");
    }
    if let Some(d) = &req.digest_name {
        req.md_alg = if d.to_bytes() == b"null" {
            unsafe { sys::EVP_md_null() }
        } else {
            md_by_name(d)
        };
    }
    if req.md_alg.is_null() {
        req.md_alg = md_by_name(c"sha1");
        store_errors();
    }

    if let Some(s) = req.extensions_section.clone() {
        if !config_check_syntax(
            host,
            fname,
            "extensions_section",
            &req.config_filename.clone(),
            &s,
            cfg,
        )? {
            return Ok(false);
        }
    }
    // the ec group curve name
    req.curve_name = sys::NID_undef;
    if let Some(Value::Str(c)) = args.and_then(|a| a.get(&key("curve_name"))).map(deref) {
        let name = cstring(c.as_bytes());
        req.curve_name = unsafe { sys::OBJ_sn2nid(name.as_ptr()) };
        if req.curve_name == sys::NID_undef {
            warn(
                host,
                fname,
                &format!(
                    "Unknown elliptic curve (short) name {}",
                    name.to_string_lossy()
                ),
            )?;
            return Ok(false);
        }
    }
    // the string mask
    if let Some(s) = conf_get_string(cfg, Some(&section), c"string_mask") {
        if unsafe { sys::ASN1_STRING_set_default_mask_asc(s.as_ptr()) } == 0 {
            warn(
                host,
                fname,
                &format!("Invalid global string mask setting {}", s.to_string_lossy()),
            )?;
            return Ok(false);
        }
    }
    if let Some(s) = req.request_extensions_section.clone() {
        if !config_check_syntax(
            host,
            fname,
            "request_extensions_section",
            &req.config_filename.clone(),
            &s,
            cfg,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// php_openssl_load_rand_file: `(egdsocket, seeded)`.
fn load_rand_file(host: &mut dyn Host, fname: &str, file: Option<&CStr>) -> Result<bool, Thrown> {
    let mut buf = [0 as c_char; 1024];
    let file = match file {
        Some(f) => f.as_ptr(),
        None => unsafe { sys::RAND_file_name(buf.as_mut_ptr(), buf.len()) },
    };
    if file.is_null() || unsafe { sys::RAND_load_file(file, -1) } < 0 {
        if unsafe { sys::RAND_status() } == 0 {
            store_errors();
            warn(
                host,
                fname,
                "Unable to load random state; not enough random data!",
            )?;
        }
        return Ok(false);
    }
    Ok(true)
}

/// php_openssl_write_rand_file.
fn write_rand_file(
    host: &mut dyn Host,
    fname: &str,
    file: Option<&CStr>,
    seeded: bool,
) -> Result<(), Thrown> {
    if !seeded {
        return Ok(());
    }
    let mut buf = [0 as c_char; 1024];
    let file = match file {
        Some(f) => f.as_ptr(),
        None => unsafe { sys::RAND_file_name(buf.as_mut_ptr(), buf.len()) },
    };
    if file.is_null() || unsafe { sys::RAND_write_file(file) } < 0 {
        store_errors();
        warn(host, fname, "Unable to write random state")?;
    }
    Ok(())
}

/// The OpenSSL name and id of an `OPENSSL_KEYTYPE_*`.
fn evp_pkey_type(key_type: c_int) -> Option<(&'static CStr, c_int)> {
    Some(match key_type {
        0 => (c"RSA", sys::EVP_PKEY_RSA),
        1 => (c"DSA", sys::EVP_PKEY_DSA),
        2 => (c"DH", sys::EVP_PKEY_DH),
        3 => (c"EC", sys::EVP_PKEY_EC),
        4 => (c"X25519", sys::EVP_PKEY_X25519),
        5 => (c"ED25519", sys::EVP_PKEY_ED25519),
        6 => (c"X448", sys::EVP_PKEY_X448),
        7 => (c"ED448", sys::EVP_PKEY_ED448),
        _ => return None,
    })
}

pub(super) unsafe extern "C" fn pkey_ctx_free(c: *mut sys::EVP_PKEY_CTX) {
    unsafe { sys::EVP_PKEY_CTX_free(c) };
}

/// php_openssl_pkey_new_from_name.
pub(super) fn pkey_ctx_from_name(name: &CStr) -> Own<sys::EVP_PKEY_CTX> {
    Own::new(
        unsafe { sys::EVP_PKEY_CTX_new_from_name(libctx(), name.as_ptr(), PROPQ) },
        pkey_ctx_free,
    )
}

/// php_openssl_pkey_new_from_pkey.
pub(super) fn pkey_ctx_from_pkey(k: *mut sys::EVP_PKEY) -> Own<sys::EVP_PKEY_CTX> {
    Own::new(
        unsafe { sys::EVP_PKEY_CTX_new_from_pkey(libctx(), k, PROPQ) },
        pkey_ctx_free,
    )
}

/// php_openssl_generate_private_key: stored in `req.priv_key`; returns it
/// (borrowed).
pub(super) fn generate_private_key(
    host: &mut dyn Host,
    fname: &str,
    req: &mut Req,
) -> Result<*mut sys::EVP_PKEY, Thrown> {
    if matches!(req.priv_key_type, 0..=2) && req.priv_key_bits < 384 {
        warn(
            host,
            fname,
            &format!(
                "Private key length must be at least 384 bits, configured to {}",
                req.priv_key_bits
            ),
        )?;
        return Ok(ptr::null_mut());
    }
    let Some((name, ty)) = evp_pkey_type(req.priv_key_type) else {
        warn(host, fname, "Unsupported private key type")?;
        return Ok(ptr::null_mut());
    };
    let randfile = conf_get_string(req.req_config.p, Some(&req.section_name), c"RANDFILE");
    let seeded = load_rand_file(host, fname, randfile.as_deref())?;
    let key = keygen(host, fname, req, name, ty);
    write_rand_file(host, fname, randfile.as_deref(), seeded)?;
    let key = key?;
    if !key.is_null() {
        req.priv_key = Own::new(key, pkey_free);
    }
    Ok(key)
}

fn keygen(
    host: &mut dyn Host,
    fname: &str,
    req: &Req,
    name: &CStr,
    ty: c_int,
) -> Result<*mut sys::EVP_PKEY, Thrown> {
    let fail = || {
        store_errors();
        Ok(ptr::null_mut())
    };
    let mut ctx = pkey_ctx_from_name(name);
    if ctx.is_null() {
        return fail();
    }
    unsafe {
        if ty != sys::EVP_PKEY_RSA {
            if sys::EVP_PKEY_paramgen_init(ctx.p) <= 0 {
                return fail();
            }
            match ty {
                sys::EVP_PKEY_DSA => {
                    if sys::EVP_PKEY_CTX_set_dsa_paramgen_bits(ctx.p, req.priv_key_bits) <= 0 {
                        return fail();
                    }
                }
                sys::EVP_PKEY_DH => {
                    if sys::EVP_PKEY_CTX_set_dh_paramgen_prime_len(ctx.p, req.priv_key_bits) <= 0 {
                        return fail();
                    }
                }
                sys::EVP_PKEY_EC => {
                    if req.curve_name == sys::NID_undef {
                        warn(
                            host,
                            fname,
                            "Missing configuration value: \"curve_name\" not set",
                        )?;
                        return Ok(ptr::null_mut());
                    }
                    if sys::EVP_PKEY_CTX_set_ec_paramgen_curve_nid(ctx.p, req.curve_name) <= 0
                        || sys::EVP_PKEY_CTX_set_ec_param_enc(ctx.p, sys::OPENSSL_EC_NAMED_CURVE)
                            <= 0
                    {
                        return fail();
                    }
                }
                _ => {}
            }
            let mut params: *mut sys::EVP_PKEY = ptr::null_mut();
            if sys::EVP_PKEY_paramgen(ctx.p, &mut params) <= 0 {
                return fail();
            }
            let params = Own::new(params, pkey_free);
            ctx = pkey_ctx_from_pkey(params.p);
            if ctx.is_null() {
                return fail();
            }
        }
        if sys::EVP_PKEY_keygen_init(ctx.p) <= 0 {
            return fail();
        }
        if ty == sys::EVP_PKEY_RSA
            && sys::EVP_PKEY_CTX_set_rsa_keygen_bits(ctx.p, req.priv_key_bits) <= 0
        {
            return fail();
        }
        let mut key: *mut sys::EVP_PKEY = ptr::null_mut();
        if sys::EVP_PKEY_keygen(ctx.p, &mut key) <= 0 {
            return fail();
        }
        Ok(key)
    }
}
