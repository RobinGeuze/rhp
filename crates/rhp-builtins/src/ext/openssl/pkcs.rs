//! PKCS#12: openssl_pkcs12_export, openssl_pkcs12_export_to_file,
//! openssl_pkcs12_read (php-src openssl.c).

use std::ptr;

use rhp_values::{ArrayKey, PhpArray, Value};

use super::backend::{
    self, array_to_x509_sk, key, pkey_free, pkey_from_zval, x509_from_param, CertStack, WRITE,
};
use super::pkey::path_arg;
use super::{
    arg, bio_bytes, check_path, cstring, mem_bio, obj_or_str, set_ref, store_errors, str_arg, sys,
    type_error, Own, CERT_CLASS,
};
use crate::zend::zpp::deref;
use crate::{warn, Host, Thrown};

pub(super) fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            (
                "openssl_pkcs12_export_to_file",
                super::wrapped!(fn_export_to_file),
            ),
            ("openssl_pkcs12_export", super::wrapped!(fn_export)),
            ("openssl_pkcs12_read", super::wrapped!(fn_read)),
        ],
    );
}

unsafe extern "C" fn pkcs12_free(p: *mut sys::PKCS12) {
    unsafe { sys::PKCS12_free(p) };
}

/// The PKCS12 both export functions build, or `None` after php's warnings.
fn create(
    host: &mut dyn Host,
    f: &str,
    args: &[Value],
    out_arg: usize,
) -> Result<Option<Own<sys::PKCS12>>, Thrown> {
    let c = obj_or_str(host, args, 0, f, CERT_CLASS)?;
    let filename = if out_arg == 1 {
        Some(path_arg(host, args, 1, f)?)
    } else {
        None
    };
    let pass = str_arg(host, args, 3, f)?;
    let opts = match args.get(4).map(deref) {
        None => None,
        Some(Value::Array(a)) => Some(a),
        Some(v) => return Err(type_error(host, f, 4, &v)),
    };
    let cert = x509_from_param(host, f, &c, 1)?;
    if cert.is_null() {
        warn(host, f, "X.509 Certificate cannot be retrieved")?;
        return Ok(None);
    }
    let priv_key = pkey_from_zval(host, f, &arg(args, 2), false, Some(b""), 3)?;
    if priv_key.is_null() {
        super::warn_unless_thrown(host, f, "Cannot get private key from parameter 3")?;
        return Ok(None);
    }
    if unsafe { sys::X509_check_private_key(cert.p, priv_key.p) } == 0 {
        if out_arg == 1 {
            store_errors();
        }
        warn(host, f, "Private key does not correspond to cert")?;
        return Ok(None);
    }
    if let Some(name) = &filename {
        if check_path(host, f, name, 2, false, false, None)?.is_none() {
            return Ok(None);
        }
    }
    let friendly = match opts
        .as_ref()
        .and_then(|a| a.get(&key("friendly_name")))
        .map(deref)
    {
        Some(Value::Str(s)) => Some(cstring(s.as_bytes())),
        _ => None,
    };
    let mut ca = CertStack::null();
    if let Some(item) = opts.as_ref().and_then(|a| a.get(&key("extracerts"))) {
        ca = array_to_x509_sk(host, f, item, 5, Some("extracerts"))?;
        if ca.0.is_null() {
            return Ok(None);
        }
    }
    let pass = cstring(&pass);
    let p12 = unsafe {
        sys::PKCS12_create(
            pass.as_ptr(),
            friendly.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
            priv_key.p,
            cert.p,
            ca.0.cast(),
            0,
            0,
            0,
            0,
            0,
        )
    };
    if p12.is_null() {
        store_errors();
        return Ok(None);
    }
    Ok(Some(Own::new(p12, pkcs12_free)))
}

fn fn_export_to_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_pkcs12_export_to_file";
    let Some(p12) = create(host, f, args, 1)? else {
        return Ok(Value::Bool(false));
    };
    let filename = str_arg(host, args, 1, f)?;
    let Some(path) = check_path(host, f, &filename, 2, false, false, None)? else {
        return Ok(Value::Bool(false));
    };
    let shown = path.to_string_lossy().into_owned();
    let bio = super::bio_file(&path, WRITE);
    if bio.is_null() {
        store_errors();
        warn(host, f, &format!("Error opening file {shown}"))?;
        return Ok(Value::Bool(false));
    }
    if unsafe { sys::i2d_PKCS12_bio(bio.p, p12.p) } == 0 {
        store_errors();
        warn(host, f, &format!("Error writing to file {shown}"))?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(true))
}

fn fn_export(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_pkcs12_export";
    let Some(p12) = create(host, f, args, 2)? else {
        return Ok(Value::Bool(false));
    };
    let bio = mem_bio();
    if !bio.is_null() && unsafe { sys::i2d_PKCS12_bio(bio.p, p12.p) } != 0 {
        set_ref(args, 1, Value::str(bio_bytes(bio.p)));
        return Ok(Value::Bool(true));
    }
    store_errors();
    Ok(Value::Bool(false))
}

fn pem_of(write: impl FnOnce(*mut sys::BIO) -> bool) -> Option<Vec<u8>> {
    let bio = mem_bio();
    if !bio.is_null() && write(bio.p) {
        Some(bio_bytes(bio.p))
    } else {
        None
    }
}

fn fn_read(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_pkcs12_read";
    let data = str_arg(host, args, 0, f)?;
    let pass = str_arg(host, args, 2, f)?;
    let bio_in = mem_bio();
    if unsafe { sys::BIO_write(bio_in.p, data.as_ptr().cast(), data.len() as i32) } <= 0 {
        store_errors();
        return Ok(Value::Bool(false));
    }
    let mut p12: *mut sys::PKCS12 = ptr::null_mut();
    let mut pkey: *mut sys::EVP_PKEY = ptr::null_mut();
    let mut cert: *mut sys::X509 = ptr::null_mut();
    let mut ca: *mut sys::stack_st_X509 = ptr::null_mut();
    let pass = cstring(&pass);
    let ok = unsafe {
        !sys::d2i_PKCS12_bio(bio_in.p, &mut p12).is_null()
            && sys::PKCS12_parse(p12, pass.as_ptr(), &mut pkey, &mut cert, &mut ca) != 0
    };
    let _p12 = Own::new(p12, pkcs12_free);
    let _pkey = Own::new(pkey, pkey_free);
    let _cert = Own::new(cert, backend::x509_free);
    let ca = CertStack(ca.cast());
    if !ok {
        store_errors();
        return Ok(Value::Bool(false));
    }
    let mut out = PhpArray::new();
    if !cert.is_null() {
        match pem_of(|b| unsafe { sys::PEM_write_bio_X509(b, cert) } != 0) {
            Some(p) => {
                out.insert(key("cert"), Value::str(p));
            }
            None => store_errors(),
        }
    }
    if !pkey.is_null() {
        match pem_of(|b| unsafe {
            sys::PEM_write_bio_PrivateKey(
                b,
                pkey,
                ptr::null(),
                ptr::null_mut(),
                0,
                None,
                ptr::null_mut(),
            ) != 0
        }) {
            Some(p) => {
                out.insert(key("pkey"), Value::str(p));
            }
            None => store_errors(),
        }
    }
    let n = ca.len();
    if n > 0 {
        let mut extra = PhpArray::new();
        for i in 0..n {
            let x = unsafe { sys::OPENSSL_sk_pop(ca.0) } as *mut sys::X509;
            if x.is_null() {
                break;
            }
            if let Some(p) = pem_of(|b| unsafe { sys::PEM_write_bio_X509(b, x) } != 0) {
                extra.insert(ArrayKey::Int(i as i64), Value::str(p));
            }
            unsafe { sys::X509_free(x) };
        }
        out.insert(key("extracerts"), Value::Array(extra));
    }
    set_ref(args, 1, Value::Array(out));
    Ok(Value::Bool(true))
}
