//! S/MIME: openssl_pkcs7_* and openssl_cms_* (php-src openssl.c and
//! openssl_backend_v3.c).

use std::ffi::{c_int, c_long, c_uint, c_void};
use std::ptr;

use rhp_values::{ArrayKey, PhpArray, Value};

use super::backend::{
    self, cipher_by_name, cipher_from_algo, load_all_certs_from_file, pkey_from_zval,
    release_cipher, setup_verify, x509_from_param, x509_from_zval, CertStack, READ, WRITE,
};
use super::pkey::path_arg;
use super::{
    arg, bio_bytes, bio_new_file, int_arg, libctx, mem_bio, obj_or_str, set_ref, store_errors,
    str_arg, sys, try_string, type_error, value_error, Own, Put, CERT_CLASS, PROPQ,
};
use crate::ext::zpp::deref;
use crate::{warn, Host, Thrown};

const PKCS7_DETACHED: i64 = 0x40;
const CMS_DETACHED: i64 = 0x40;
const CMS_STREAM: i64 = 0x1000;
const CMS_PARTIAL: i64 = 0x4000;
const ENCODING_DER: i64 = 0;
const ENCODING_SMIME: i64 = 1;
const ENCODING_PEM: i64 = 2;
const NID_PKCS7_SIGNED: c_int = 22;
const NID_PKCS7_SIGNED_AND_ENVELOPED: c_int = 24;
const BIO_CTRL_RESET: c_int = 1;

pub(super) fn register(put: &mut Put) {
    put(
        "openssl_pkcs7_verify",
        super::wrapped!(fn_pkcs7_verify),
        2,
        7,
    );
    put(
        "openssl_pkcs7_encrypt",
        super::wrapped!(fn_pkcs7_encrypt),
        4,
        6,
    );
    put("openssl_pkcs7_read", super::wrapped!(fn_pkcs7_read), 2, 2);
    put("openssl_pkcs7_sign", super::wrapped!(fn_pkcs7_sign), 5, 7);
    put(
        "openssl_pkcs7_decrypt",
        super::wrapped!(fn_pkcs7_decrypt),
        3,
        4,
    );
    put("openssl_cms_verify", super::wrapped!(fn_cms_verify), 2, 9);
    put("openssl_cms_encrypt", super::wrapped!(fn_cms_encrypt), 4, 7);
    put("openssl_cms_read", super::wrapped!(fn_cms_read), 2, 2);
    put("openssl_cms_sign", super::wrapped!(fn_cms_sign), 5, 8);
    put("openssl_cms_decrypt", super::wrapped!(fn_cms_decrypt), 3, 5);
}

extern "C" {
    fn PKCS7_new_ex(libctx: *mut sys::OSSL_LIB_CTX, propq: *const i8) -> *mut sys::PKCS7;
    fn PEM_read_bio_PKCS7(
        bp: *mut sys::BIO,
        x: *mut *mut sys::PKCS7,
        cb: sys::pem_password_cb,
        u: *mut c_void,
    ) -> *mut sys::PKCS7;
    fn PEM_write_bio_PKCS7(bp: *mut sys::BIO, x: *const sys::PKCS7) -> c_int;
    fn PEM_write_bio_X509_CRL(bp: *mut sys::BIO, x: *const sys::X509_CRL) -> c_int;
    fn CMS_ContentInfo_new_ex(
        libctx: *mut sys::OSSL_LIB_CTX,
        propq: *const i8,
    ) -> *mut sys::CMS_ContentInfo;
    fn PEM_read_bio_CMS(
        bp: *mut sys::BIO,
        x: *mut *mut sys::CMS_ContentInfo,
        cb: sys::pem_password_cb,
        u: *mut c_void,
    ) -> *mut sys::CMS_ContentInfo;
    fn PEM_write_bio_CMS(bp: *mut sys::BIO, x: *const sys::CMS_ContentInfo) -> c_int;
    fn PEM_write_bio_CMS_stream(
        out: *mut sys::BIO,
        cms: *mut sys::CMS_ContentInfo,
        in_: *mut sys::BIO,
        flags: c_int,
    ) -> c_int;
    fn d2i_CMS_bio(
        bp: *mut sys::BIO,
        cms: *mut *mut sys::CMS_ContentInfo,
    ) -> *mut sys::CMS_ContentInfo;
    fn i2d_CMS_bio(bp: *mut sys::BIO, cms: *mut sys::CMS_ContentInfo) -> c_int;
    fn SMIME_read_CMS_ex(
        bio: *mut sys::BIO,
        flags: c_int,
        bcont: *mut *mut sys::BIO,
        cms: *mut *mut sys::CMS_ContentInfo,
    ) -> *mut sys::CMS_ContentInfo;
    fn SMIME_write_CMS(
        bio: *mut sys::BIO,
        cms: *mut sys::CMS_ContentInfo,
        data: *mut sys::BIO,
        flags: c_int,
    ) -> c_int;
    fn CMS_final(
        cms: *mut sys::CMS_ContentInfo,
        data: *mut sys::BIO,
        dcont: *mut sys::BIO,
        flags: c_uint,
    ) -> c_int;
    fn CMS_get0_signers(cms: *mut sys::CMS_ContentInfo) -> *mut sys::OPENSSL_STACK;
    fn CMS_get0_type(cms: *const sys::CMS_ContentInfo) -> *const sys::ASN1_OBJECT;
    fn CMS_get1_certs(cms: *mut sys::CMS_ContentInfo) -> *mut sys::OPENSSL_STACK;
    fn CMS_get1_crls(cms: *mut sys::CMS_ContentInfo) -> *mut sys::OPENSSL_STACK;
    fn X509_CRL_free(x: *mut sys::X509_CRL);
    fn BIO_ctrl(b: *mut sys::BIO, cmd: c_int, larg: c_long, parg: *mut c_void) -> c_long;
}

unsafe extern "C" fn pkcs7_free(p: *mut sys::PKCS7) {
    unsafe { sys::PKCS7_free(p) };
}

unsafe extern "C" fn cms_free(p: *mut sys::CMS_ContentInfo) {
    unsafe { sys::CMS_ContentInfo_free(p) };
}

unsafe extern "C" fn x509_crl_free_void(p: *mut c_void) {
    unsafe { X509_CRL_free(p.cast()) };
}

fn bio_reset(b: *mut sys::BIO) -> c_long {
    unsafe { BIO_ctrl(b, BIO_CTRL_RESET, 0, ptr::null_mut()) }
}

/// An optional `?string` path parameter (`p!`).
fn opt_path(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
) -> Result<Option<Vec<u8>>, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => path_arg(host, args, i, f).map(Some),
    }
}

/// The extra headers both S/MIME writers put before the message:
/// `BIO_printf("%s: %s\n")` (C strings, so up to a NUL).
fn write_headers(
    host: &mut dyn Host,
    out: *mut sys::BIO,
    headers: &PhpArray,
) -> Result<bool, Thrown> {
    let cut = |b: &[u8]| -> Vec<u8> { b.iter().take_while(|&&c| c != 0).copied().collect() };
    for (k, v) in headers.iter() {
        let s = try_string(host, v)?;
        let mut line = Vec::new();
        if let ArrayKey::Str(name) = k {
            line.extend(cut(name.as_bytes()));
            line.extend_from_slice(b": ");
        }
        line.extend(cut(&s));
        line.push(b'\n');
        if unsafe { sys::BIO_write(out, line.as_ptr().cast(), line.len() as c_int) } < 0 {
            store_errors();
            return Ok(false);
        }
    }
    Ok(true)
}

/// php_openssl_pem_read_bio_pkcs7.
fn pem_read_bio_pkcs7(bio: *mut sys::BIO) -> *mut sys::PKCS7 {
    unsafe {
        let mut p = PKCS7_new_ex(libctx(), PROPQ);
        if p.is_null() {
            return p;
        }
        if PEM_read_bio_PKCS7(bio, &mut p, None, ptr::null_mut()).is_null() {
            sys::PKCS7_free(p);
            return ptr::null_mut();
        }
        p
    }
}

/// php_openssl_pem_read_bio_cms / d2i_bio_cms / smime_read_cms.
fn read_cms(bio: *mut sys::BIO, how: i64, bcont: *mut *mut sys::BIO) -> *mut sys::CMS_ContentInfo {
    unsafe {
        let mut ci = CMS_ContentInfo_new_ex(libctx(), PROPQ);
        if ci.is_null() {
            return ci;
        }
        let r = match how {
            ENCODING_PEM => PEM_read_bio_CMS(bio, &mut ci, None, ptr::null_mut()),
            ENCODING_DER => d2i_CMS_bio(bio, &mut ci),
            _ => SMIME_read_CMS_ex(bio, 0, bcont, &mut ci),
        };
        if r.is_null() {
            sys::CMS_ContentInfo_free(ci);
            return ptr::null_mut();
        }
        ci
    }
}

/// Write the certificates and CRLs of a stack pair as PEM strings, at the
/// same indexes (a CRL overwrites the certificate at its index, as in php).
fn certs_and_crls(certs: *mut sys::OPENSSL_STACK, crls: *mut sys::OPENSSL_STACK) -> PhpArray {
    let mut out = PhpArray::new();
    unsafe {
        if !certs.is_null() {
            for i in 0..sys::OPENSSL_sk_num(certs) {
                let x = sys::OPENSSL_sk_value(certs, i) as *mut sys::X509;
                let b = mem_bio();
                if !b.is_null() && sys::PEM_write_bio_X509(b.p, x) != 0 {
                    out.insert(ArrayKey::Int(i as i64), Value::str(bio_bytes(b.p)));
                }
            }
        }
        if !crls.is_null() {
            for i in 0..sys::OPENSSL_sk_num(crls) {
                let c = sys::OPENSSL_sk_value(crls, i) as *mut sys::X509_CRL;
                let b = mem_bio();
                if !b.is_null() && PEM_write_bio_X509_CRL(b.p, c) != 0 {
                    out.insert(ArrayKey::Int(i as i64), Value::str(bio_bytes(b.p)));
                }
            }
        }
    }
    out
}

/// Write each signer certificate as PEM; `false` on a write failure.
fn write_signers(
    host: &mut dyn Host,
    f: &str,
    certout: *mut sys::BIO,
    signers: *mut sys::OPENSSL_STACK,
) -> Result<bool, Thrown> {
    let mut ok = true;
    for i in 0..unsafe { sys::OPENSSL_sk_num(signers) } {
        let x = unsafe { sys::OPENSSL_sk_value(signers, i) } as *mut sys::X509;
        if unsafe { sys::PEM_write_bio_X509(certout, x) } == 0 {
            store_errors();
            ok = false;
            warn(host, f, &format!("Failed to write signer {i}"))?;
        }
    }
    Ok(ok)
}

fn array_or_null(
    host: &dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
) -> Result<Option<PhpArray>, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => Ok(Some(a)),
        Some(v) => Err(type_error(host, f, i, &v)),
    }
}

fn array_arg(args: &[Value], i: usize) -> Option<Value> {
    match args.get(i).map(deref) {
        Some(v @ Value::Array(_)) => Some(v),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// PKCS#7
// ---------------------------------------------------------------------------

fn fn_pkcs7_verify(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_pkcs7_verify";
    let filename = path_arg(host, args, 0, f)?;
    let mut flags = int_arg(host, args, 1, f, 0)?;
    let signers_file = opt_path(host, args, 2, f)?;
    if let Some(v) = args.get(3).map(deref) {
        if !matches!(v, Value::Array(_)) {
            return Err(type_error(host, f, 3, &v));
        }
    }
    let cainfo = array_arg(args, 3);
    let extracerts = opt_path(host, args, 4, f)?;
    let datafile = opt_path(host, args, 5, f)?;
    let p7bfile = opt_path(host, args, 6, f)?;

    let mut others = CertStack::null();
    if let Some(e) = &extracerts {
        others = load_all_certs_from_file(host, f, e, 5)?;
        if others.0.is_null() {
            return Ok(Value::Int(-1));
        }
    }
    flags &= !PKCS7_DETACHED;
    let store = setup_verify(host, f, cainfo.as_ref(), 4)?;
    if store.is_null() {
        return Ok(Value::Int(-1));
    }
    let input = bio_new_file(host, f, &filename, 1, READ)?;
    if input.is_null() {
        return Ok(Value::Int(-1));
    }
    let mut datain: *mut sys::BIO = ptr::null_mut();
    let p7 = Own::new(
        unsafe { sys::SMIME_read_PKCS7(input.p, &mut datain) },
        pkcs7_free,
    );
    let datain = Own::new(datain, super::free_bio);
    if p7.is_null() {
        store_errors();
        return Ok(Value::Int(-1));
    }
    let mut dataout = Own::null();
    if let Some(d) = &datafile {
        dataout = bio_new_file(host, f, d, 6, WRITE)?;
        if dataout.is_null() {
            return Ok(Value::Int(-1));
        }
    }
    let mut p7bout = Own::null();
    if let Some(p) = &p7bfile {
        p7bout = bio_new_file(host, f, p, 7, WRITE)?;
        if p7bout.is_null() {
            return Ok(Value::Int(-1));
        }
    }
    if unsafe {
        sys::PKCS7_verify(
            p7.p,
            others.0.cast(),
            store.p,
            datain.p,
            dataout.p,
            flags as c_int,
        )
    } == 0
    {
        store_errors();
        return Ok(Value::Bool(false));
    }
    let mut ret = Value::Bool(true);
    if let Some(sf) = &signers_file {
        let certout = bio_new_file(host, f, sf, 3, WRITE)?;
        if !certout.is_null() {
            let signers = unsafe { sys::PKCS7_get0_signers(p7.p, others.0.cast(), flags as c_int) };
            if !signers.is_null() {
                if !write_signers(host, f, certout.p, signers.cast())? {
                    ret = Value::Int(-1);
                }
                unsafe { sys::OPENSSL_sk_free(signers.cast()) };
            } else {
                ret = Value::Int(-1);
                store_errors();
            }
        } else {
            warn(
                host,
                f,
                &format!(
                    "Signature OK, but cannot open {} for writing",
                    String::from_utf8_lossy(sf)
                ),
            )?;
            ret = Value::Int(-1);
        }
        if !p7bout.is_null() && unsafe { PEM_write_bio_PKCS7(p7bout.p, p7.p) } == 0 {
            warn(host, f, "Failed to write PKCS7 to file")?;
            store_errors();
            ret = Value::Bool(false);
        }
    }
    Ok(ret)
}

/// The recipient certificates of openssl_{pkcs7,cms}_encrypt; a null stack
/// when one cannot be read.
fn recipient_certs(host: &mut dyn Host, f: &str, v: &Value) -> Result<CertStack, Thrown> {
    let sk = CertStack(unsafe { sys::OPENSSL_sk_new_null() });
    if sk.0.is_null() {
        return Ok(sk);
    }
    let v = deref(v);
    let items: Vec<(Value, bool)> = match &v {
        Value::Array(a) => a.iter().map(|(_, c)| (c.clone(), true)).collect(),
        c => vec![(c.clone(), false)],
    };
    for (c, from_array) in items {
        let cert = x509_from_zval(host, f, &c, 3, from_array, None)?;
        if cert.is_null() {
            return Ok(CertStack::null());
        }
        let x = if cert.is_owned() {
            cert.release()
        } else {
            let d = unsafe { sys::X509_dup(cert.p) };
            if d.is_null() {
                store_errors();
                return Ok(CertStack::null());
            }
            d
        };
        if unsafe { sys::OPENSSL_sk_push(sk.0, x.cast()) } <= 0 {
            unsafe { sys::X509_free(x) };
            return Ok(CertStack::null());
        }
    }
    Ok(sk)
}

fn fn_pkcs7_encrypt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_pkcs7_encrypt";
    let infile = path_arg(host, args, 0, f)?;
    let outfile = path_arg(host, args, 1, f)?;
    let headers = array_or_null(host, args, 3, f)?;
    let flags = int_arg(host, args, 4, f, 0)?;
    let cipherid = int_arg(host, args, 5, f, 5)?;
    let input = bio_new_file(host, f, &infile, 1, READ)?;
    if input.is_null() {
        return Ok(Value::Bool(false));
    }
    let out = bio_new_file(host, f, &outfile, 2, WRITE)?;
    if out.is_null() {
        return Ok(Value::Bool(false));
    }
    let certs = recipient_certs(host, f, &arg(args, 2))?;
    if certs.0.is_null() {
        return Ok(Value::Bool(false));
    }
    let cipher = cipher_from_algo(cipherid);
    if cipher.is_null() {
        warn(host, f, "Failed to get cipher")?;
        return Ok(Value::Bool(false));
    }
    let r = (|| -> Result<Value, Thrown> {
        let p7 = Own::new(
            unsafe { sys::PKCS7_encrypt(certs.0.cast(), input.p, cipher, flags as c_int) },
            pkcs7_free,
        );
        if p7.is_null() {
            store_errors();
            return Ok(Value::Bool(false));
        }
        if let Some(h) = &headers {
            if !write_headers(host, out.p, h)? {
                return Ok(Value::Bool(false));
            }
        }
        bio_reset(input.p);
        if unsafe { sys::SMIME_write_PKCS7(out.p, p7.p, input.p, flags as c_int) } == 0 {
            store_errors();
            return Ok(Value::Bool(false));
        }
        Ok(Value::Bool(true))
    })();
    release_cipher(cipher);
    r
}

fn fn_pkcs7_read(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_pkcs7_read";
    let data = str_arg(host, args, 0, f)?;
    let bio = mem_bio();
    if bio.is_null() {
        return Ok(Value::Bool(false));
    }
    if unsafe { sys::BIO_write(bio.p, data.as_ptr().cast(), data.len() as c_int) } <= 0 {
        store_errors();
        return Ok(Value::Bool(false));
    }
    let p7 = Own::new(pem_read_bio_pkcs7(bio.p), pkcs7_free);
    if p7.is_null() {
        store_errors();
        return Ok(Value::Bool(false));
    }
    let (mut certs, mut crls) = (ptr::null_mut(), ptr::null_mut());
    unsafe {
        match sys::OBJ_obj2nid((*p7.p).type_) {
            NID_PKCS7_SIGNED => {
                let s = (*p7.p).d.sign;
                if !s.is_null() {
                    certs = (*s).cert as *mut sys::OPENSSL_STACK;
                    crls = (*s).crl as *mut sys::OPENSSL_STACK;
                }
            }
            NID_PKCS7_SIGNED_AND_ENVELOPED => {
                let s = (*p7.p).d.signed_and_enveloped;
                if !s.is_null() {
                    certs = (*s).cert as *mut sys::OPENSSL_STACK;
                    crls = (*s).crl as *mut sys::OPENSSL_STACK;
                }
            }
            _ => {}
        }
    }
    set_ref(args, 1, Value::Array(certs_and_crls(certs, crls)));
    Ok(Value::Bool(true))
}

fn fn_pkcs7_sign(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_pkcs7_sign";
    let infile = path_arg(host, args, 0, f)?;
    let outfile = path_arg(host, args, 1, f)?;
    let c = obj_or_str(host, args, 2, f, CERT_CLASS)?;
    let headers = array_or_null(host, args, 4, f)?;
    let flags = int_arg(host, args, 5, f, PKCS7_DETACHED)?;
    let extra = opt_path(host, args, 6, f)?;
    let mut others = CertStack::null();
    if let Some(e) = &extra {
        others = load_all_certs_from_file(host, f, e, 7)?;
        if others.0.is_null() {
            return Ok(Value::Bool(false));
        }
    }
    let k = pkey_from_zval(host, f, &arg(args, 3), false, Some(b""), 4)?;
    if k.is_null() {
        super::warn_unless_thrown(host, f, "Error getting private key")?;
        return Ok(Value::Bool(false));
    }
    let cert = x509_from_param(host, f, &c, 3)?;
    if cert.is_null() {
        warn(host, f, "X.509 Certificate cannot be retrieved")?;
        return Ok(Value::Bool(false));
    }
    let input = bio_new_file(host, f, &infile, 1, READ)?;
    if input.is_null() {
        warn(
            host,
            f,
            &format!(
                "Error opening input file {}!",
                String::from_utf8_lossy(&infile)
            ),
        )?;
        return Ok(Value::Bool(false));
    }
    let out = bio_new_file(host, f, &outfile, 2, WRITE)?;
    if out.is_null() {
        warn(
            host,
            f,
            &format!(
                "Error opening output file {}!",
                String::from_utf8_lossy(&outfile)
            ),
        )?;
        return Ok(Value::Bool(false));
    }
    let p7 = Own::new(
        unsafe { sys::PKCS7_sign(cert.p, k.p, others.0.cast(), input.p, flags as c_int) },
        pkcs7_free,
    );
    if p7.is_null() {
        store_errors();
        warn(host, f, "Error creating PKCS7 structure!")?;
        return Ok(Value::Bool(false));
    }
    bio_reset(input.p);
    if let Some(h) = &headers {
        if !write_headers(host, out.p, h)? {
            return Ok(Value::Bool(false));
        }
    }
    if unsafe { sys::SMIME_write_PKCS7(out.p, p7.p, input.p, flags as c_int) } == 0 {
        store_errors();
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

fn fn_pkcs7_decrypt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_pkcs7_decrypt";
    let infile = path_arg(host, args, 0, f)?;
    let outfile = path_arg(host, args, 1, f)?;
    let recipcert = arg(args, 2);
    let recipkey = match args.get(3).map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(v),
    };
    let cert = x509_from_zval(host, f, &recipcert, 3, false, None)?;
    if cert.is_null() {
        warn(host, f, "X.509 Certificate cannot be retrieved")?;
        return Ok(Value::Bool(false));
    }
    let k = pkey_from_zval(
        host,
        f,
        recipkey.as_ref().unwrap_or(&recipcert),
        false,
        Some(b""),
        4,
    )?;
    if k.is_null() {
        super::warn_unless_thrown(host, f, "Unable to get private key")?;
        return Ok(Value::Bool(false));
    }
    let input = bio_new_file(host, f, &infile, 1, READ)?;
    if input.is_null() {
        return Ok(Value::Bool(false));
    }
    let out = bio_new_file(host, f, &outfile, 2, WRITE)?;
    if out.is_null() {
        return Ok(Value::Bool(false));
    }
    let mut datain: *mut sys::BIO = ptr::null_mut();
    let p7 = Own::new(
        unsafe { sys::SMIME_read_PKCS7(input.p, &mut datain) },
        pkcs7_free,
    );
    let _datain = Own::new(datain, super::free_bio);
    if p7.is_null() {
        store_errors();
        return Ok(Value::Bool(false));
    }
    if unsafe { sys::PKCS7_decrypt(p7.p, k.p, cert.p, out.p, PKCS7_DETACHED as c_int) } != 0 {
        Ok(Value::Bool(true))
    } else {
        store_errors();
        Ok(Value::Bool(false))
    }
}

// ---------------------------------------------------------------------------
// CMS
// ---------------------------------------------------------------------------

fn fn_cms_verify(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_cms_verify";
    let filename = path_arg(host, args, 0, f)?;
    let flags = int_arg(host, args, 1, f, 0)?;
    let signers_file = opt_path(host, args, 2, f)?;
    if let Some(v) = args.get(3).map(deref) {
        if !matches!(v, Value::Array(_)) {
            return Err(type_error(host, f, 3, &v));
        }
    }
    let cainfo = array_arg(args, 3);
    let extracerts = opt_path(host, args, 4, f)?;
    let datafile = opt_path(host, args, 5, f)?;
    let p7bfile = opt_path(host, args, 6, f)?;
    let sigfile = opt_path(host, args, 7, f)?;
    let encoding = int_arg(host, args, 8, f, ENCODING_SMIME)?;

    let input = bio_new_file(host, f, &filename, 1, READ)?;
    if input.is_null() {
        return Ok(Value::Bool(false));
    }
    let mut sigbio: Own<sys::BIO> = Own::borrowed(input.p);
    if let (Some(s), true) = (&sigfile, flags & CMS_DETACHED != 0) {
        if encoding == ENCODING_SMIME {
            warn(
                host,
                f,
                "Detached signatures not possible with S/MIME encoding",
            )?;
            return Ok(Value::Bool(false));
        }
        sigbio = bio_new_file(host, f, s, 1, READ)?;
        if sigbio.is_null() {
            return Ok(Value::Bool(false));
        }
    }
    let mut datain: *mut sys::BIO = ptr::null_mut();
    let cms = match encoding {
        ENCODING_PEM | ENCODING_DER => {
            datain = input.p;
            read_cms(sigbio.p, encoding, ptr::null_mut())
        }
        ENCODING_SMIME => read_cms(sigbio.p, ENCODING_SMIME, &mut datain),
        _ => {
            warn(host, f, "Unknown encoding")?;
            return Ok(Value::Bool(false));
        }
    };
    let cms = Own::new(cms, cms_free);
    let _datain_owned = if encoding == ENCODING_SMIME {
        Own::new(datain, super::free_bio)
    } else {
        Own::borrowed(datain)
    };
    if cms.is_null() {
        store_errors();
        return Ok(Value::Bool(false));
    }
    if encoding != ENCODING_SMIME && flags & CMS_DETACHED == 0 {
        datain = ptr::null_mut();
    }
    let mut others = CertStack::null();
    if let Some(e) = &extracerts {
        others = load_all_certs_from_file(host, f, e, 5)?;
        if others.0.is_null() {
            return Ok(Value::Bool(false));
        }
    }
    let store = setup_verify(host, f, cainfo.as_ref(), 4)?;
    if store.is_null() {
        return Ok(Value::Bool(false));
    }
    let mut dataout = Own::null();
    if let Some(d) = &datafile {
        dataout = bio_new_file(host, f, d, 6, WRITE)?;
        if dataout.is_null() {
            return Ok(Value::Bool(false));
        }
    }
    let mut p7bout = Own::null();
    if let Some(p) = &p7bfile {
        p7bout = bio_new_file(host, f, p, 7, WRITE)?;
        if p7bout.is_null() {
            return Ok(Value::Bool(false));
        }
    }
    if unsafe {
        sys::CMS_verify(
            cms.p,
            others.0.cast(),
            store.p,
            datain,
            dataout.p,
            flags as c_uint,
        )
    } == 0
    {
        store_errors();
        return Ok(Value::Bool(false));
    }
    let mut ret = true;
    if let Some(sf) = &signers_file {
        let certout = bio_new_file(host, f, sf, 3, WRITE)?;
        if !certout.is_null() {
            let signers = unsafe { CMS_get0_signers(cms.p) };
            if !signers.is_null() {
                if !write_signers(host, f, certout.p, signers)? {
                    ret = false;
                }
                unsafe { sys::OPENSSL_sk_free(signers) };
            } else {
                ret = false;
                store_errors();
            }
        } else {
            warn(
                host,
                f,
                &format!(
                    "Signature OK, but cannot open {} for writing",
                    String::from_utf8_lossy(sf)
                ),
            )?;
            ret = false;
        }
        if !p7bout.is_null() && unsafe { PEM_write_bio_CMS(p7bout.p, cms.p) } == 0 {
            warn(host, f, "Failed to write CMS to file")?;
            store_errors();
            ret = false;
        }
    }
    Ok(Value::Bool(ret))
}

/// Write a finished CMS structure in `encoding`; `Ok(false)` on failure.
fn write_cms(
    host: &mut dyn Host,
    f: &str,
    out: *mut sys::BIO,
    cms: *mut sys::CMS_ContentInfo,
    input: *mut sys::BIO,
    flags: i64,
    encoding: i64,
) -> Result<bool, Thrown> {
    let need_final = flags & CMS_PARTIAL != 0 && flags & CMS_STREAM == 0;
    unsafe {
        match encoding {
            ENCODING_SMIME => {
                if SMIME_write_CMS(out, cms, input, flags as c_int) == 0 {
                    store_errors();
                    return Ok(false);
                }
            }
            ENCODING_DER | ENCODING_PEM => {
                if need_final && CMS_final(cms, input, ptr::null_mut(), flags as c_uint) != 1 {
                    store_errors();
                    return Ok(false);
                }
                let ok = if encoding == ENCODING_DER {
                    i2d_CMS_bio(out, cms) == 1
                } else if flags & CMS_STREAM != 0 {
                    PEM_write_bio_CMS_stream(out, cms, input, flags as c_int) != 0
                } else {
                    PEM_write_bio_CMS(out, cms) != 0
                };
                if !ok {
                    store_errors();
                    return Ok(false);
                }
            }
            _ => {
                warn(host, f, "Unknown OPENSSL encoding")?;
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn fn_cms_encrypt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_cms_encrypt";
    let infile = path_arg(host, args, 0, f)?;
    let outfile = path_arg(host, args, 1, f)?;
    let headers = array_or_null(host, args, 3, f)?;
    let flags = int_arg(host, args, 4, f, 0)?;
    let encoding = int_arg(host, args, 5, f, ENCODING_SMIME)?;
    let cipher_arg = args.get(6).map(deref);
    let input = bio_new_file(host, f, &infile, 1, READ)?;
    if input.is_null() {
        return Ok(Value::Bool(false));
    }
    let out = bio_new_file(host, f, &outfile, 2, WRITE)?;
    if out.is_null() {
        return Ok(Value::Bool(false));
    }
    let certs = recipient_certs(host, f, &arg(args, 2))?;
    if certs.0.is_null() {
        return Ok(Value::Bool(false));
    }
    let cipher = match &cipher_arg {
        None => cipher_from_algo(5),
        Some(Value::Int(n)) => cipher_from_algo(*n),
        Some(Value::Str(s)) => cipher_by_name(&super::cstring(s.as_bytes())),
        Some(v) => match int_arg(host, std::slice::from_ref(v), 0, f, 5) {
            Ok(n) => cipher_from_algo(n),
            Err(_) => cipher_by_name(&super::cstring(&try_string(host, v)?)),
        },
    };
    if cipher.is_null() {
        warn(host, f, "Failed to get cipher")?;
        return Ok(Value::Bool(false));
    }
    let r = (|| -> Result<Value, Thrown> {
        let cms = Own::new(
            unsafe { sys::CMS_encrypt(certs.0.cast(), input.p, cipher, flags as c_uint) },
            cms_free,
        );
        if cms.is_null() {
            store_errors();
            return Ok(Value::Bool(false));
        }
        if let (Some(h), ENCODING_SMIME) = (&headers, encoding) {
            if !write_headers(host, out.p, h)? {
                return Ok(Value::Bool(false));
            }
        }
        bio_reset(input.p);
        Ok(Value::Bool(write_cms(
            host, f, out.p, cms.p, input.p, flags, encoding,
        )?))
    })();
    release_cipher(cipher);
    r
}

fn fn_cms_read(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_cms_read";
    let data = str_arg(host, args, 0, f)?;
    let bio = mem_bio();
    if bio.is_null() {
        return Ok(Value::Bool(false));
    }
    if unsafe { sys::BIO_write(bio.p, data.as_ptr().cast(), data.len() as c_int) } <= 0 {
        store_errors();
        return Ok(Value::Bool(false));
    }
    let cms = Own::new(read_cms(bio.p, ENCODING_PEM, ptr::null_mut()), cms_free);
    if cms.is_null() {
        store_errors();
        return Ok(Value::Bool(false));
    }
    let (mut certs, mut crls) = (ptr::null_mut(), ptr::null_mut());
    unsafe {
        match sys::OBJ_obj2nid(CMS_get0_type(cms.p)) {
            NID_PKCS7_SIGNED | NID_PKCS7_SIGNED_AND_ENVELOPED => {
                certs = CMS_get1_certs(cms.p);
                crls = CMS_get1_crls(cms.p);
            }
            _ => {}
        }
    }
    let out = certs_and_crls(certs, crls);
    unsafe {
        if !certs.is_null() {
            sys::OPENSSL_sk_pop_free(certs, backend::x509_free_void);
        }
        if !crls.is_null() {
            sys::OPENSSL_sk_pop_free(crls, x509_crl_free_void);
        }
    }
    set_ref(args, 1, Value::Array(out));
    Ok(Value::Bool(true))
}

fn fn_cms_sign(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_cms_sign";
    let infile = path_arg(host, args, 0, f)?;
    let outfile = path_arg(host, args, 1, f)?;
    let c = obj_or_str(host, args, 2, f, CERT_CLASS)?;
    let headers = array_or_null(host, args, 4, f)?;
    let flags = int_arg(host, args, 5, f, 0)?;
    let encoding = int_arg(host, args, 6, f, ENCODING_SMIME)?;
    let extra = opt_path(host, args, 7, f)?;
    let mut others = CertStack::null();
    if let Some(e) = &extra {
        others = load_all_certs_from_file(host, f, e, 8)?;
        if others.0.is_null() {
            return Ok(Value::Bool(false));
        }
    }
    let k = pkey_from_zval(host, f, &arg(args, 3), false, Some(b""), 4)?;
    if k.is_null() {
        super::warn_unless_thrown(host, f, "Error getting private key")?;
        return Ok(Value::Bool(false));
    }
    let cert = x509_from_param(host, f, &c, 3)?;
    if cert.is_null() {
        warn(host, f, "X.509 Certificate cannot be retrieved")?;
        return Ok(Value::Bool(false));
    }
    if encoding & ENCODING_SMIME != 0 && flags & CMS_DETACHED != 0 {
        warn(
            host,
            f,
            "Detached signatures not possible with S/MIME encoding",
        )?;
        return Ok(Value::Bool(false));
    }
    let input = bio_new_file(host, f, &infile, 1, READ)?;
    if input.is_null() {
        warn(
            host,
            f,
            &format!(
                "Error opening input file {}!",
                String::from_utf8_lossy(&infile)
            ),
        )?;
        return Ok(Value::Bool(false));
    }
    let out = bio_new_file(host, f, &outfile, 2, WRITE)?;
    if out.is_null() {
        warn(
            host,
            f,
            &format!(
                "Error opening output file {}!",
                String::from_utf8_lossy(&outfile)
            ),
        )?;
        return Ok(Value::Bool(false));
    }
    let cms = Own::new(
        unsafe { sys::CMS_sign(cert.p, k.p, others.0.cast(), input.p, flags as c_uint) },
        cms_free,
    );
    if cms.is_null() {
        store_errors();
        warn(host, f, "Error creating CMS structure!")?;
        return Ok(Value::Bool(false));
    }
    if bio_reset(input.p) != 0 {
        store_errors();
        return Ok(Value::Bool(false));
    }
    if let (Some(h), ENCODING_SMIME) = (&headers, encoding) {
        if !write_headers(host, out.p, h)? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(write_cms(
        host, f, out.p, cms.p, input.p, flags, encoding,
    )?))
}

fn fn_cms_decrypt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_cms_decrypt";
    let infile = path_arg(host, args, 0, f)?;
    let outfile = path_arg(host, args, 1, f)?;
    let recipcert = arg(args, 2);
    let recipkey = match args.get(3).map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(v),
    };
    let encoding = int_arg(host, args, 4, f, ENCODING_SMIME)?;
    let cert = x509_from_zval(host, f, &recipcert, 3, false, None)?;
    if cert.is_null() {
        warn(host, f, "X.509 Certificate cannot be retrieved")?;
        return Ok(Value::Bool(false));
    }
    let k = pkey_from_zval(
        host,
        f,
        recipkey.as_ref().unwrap_or(&recipcert),
        false,
        Some(b""),
        if recipkey.is_some() { 4 } else { 3 },
    )?;
    if k.is_null() {
        super::warn_unless_thrown(host, f, "Unable to get private key")?;
        return Ok(Value::Bool(false));
    }
    let input = bio_new_file(host, f, &infile, 1, READ)?;
    if input.is_null() {
        return Ok(Value::Bool(false));
    }
    let out = bio_new_file(host, f, &outfile, 2, WRITE)?;
    if out.is_null() {
        return Ok(Value::Bool(false));
    }
    let mut datain: *mut sys::BIO = ptr::null_mut();
    let cms = match encoding {
        ENCODING_DER | ENCODING_PEM => read_cms(input.p, encoding, ptr::null_mut()),
        ENCODING_SMIME => read_cms(input.p, ENCODING_SMIME, &mut datain),
        _ => {
            return Err(value_error(f, 4, "must be an OPENSSL_ENCODING_* constant"));
        }
    };
    let cms = Own::new(cms, cms_free);
    let _datain = Own::new(datain, super::free_bio);
    if cms.is_null() {
        store_errors();
        return Ok(Value::Bool(false));
    }
    if unsafe { sys::CMS_decrypt(cms.p, k.p, cert.p, ptr::null_mut(), out.p, 0) } != 0 {
        Ok(Value::Bool(true))
    } else {
        store_errors();
        Ok(Value::Bool(false))
    }
}
