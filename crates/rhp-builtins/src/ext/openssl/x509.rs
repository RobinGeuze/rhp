//! The openssl_x509_* functions (php-src ext/openssl/openssl.c).

use std::ffi::c_char;
use std::ptr;

use rhp_values::{ArrayKey, PhpArray, Value};

use super::backend::{
    self, add_assoc_name_entry, asn1_string_bytes, asn1_time_to_time_t, key, x509_from_param, WRITE,
};
use super::{
    arg, bio_bytes, bool_arg, check_path, cstr_bytes, int_arg, mem_bio, obj_or_str, openssl_free,
    opt_str_arg, set_ref, store_errors, str_arg, sys, type_error, ObjOrStr, CERT_CLASS,
};
use crate::{warn, Host, Thrown};

pub(super) fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            (
                "openssl_x509_export_to_file",
                super::wrapped!(fn_export_to_file),
            ),
            ("openssl_x509_export", super::wrapped!(fn_export)),
            ("openssl_x509_fingerprint", super::wrapped!(fn_fingerprint)),
            (
                "openssl_x509_check_private_key",
                super::wrapped!(fn_check_private_key),
            ),
            ("openssl_x509_verify", super::wrapped!(fn_verify)),
            ("openssl_x509_parse", super::wrapped!(fn_parse)),
            (
                "openssl_x509_checkpurpose",
                super::wrapped!(fn_checkpurpose),
            ),
            ("openssl_x509_read", super::wrapped!(fn_read)),
            ("openssl_x509_free", super::wrapped!(fn_free)),
        ],
    );
}

fn cert_arg(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<ObjOrStr, Thrown> {
    obj_or_str(host, args, 0, fname, CERT_CLASS)
}

fn fn_export_to_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_x509_export_to_file";
    let c = cert_arg(host, args, f)?;
    let filename = str_arg(host, args, 1, f)?;
    let notext = bool_arg(host, args, 2, f, true)?;
    let cert = x509_from_param(host, f, &c, 1)?;
    if cert.is_null() {
        warn(host, f, "X.509 Certificate cannot be retrieved")?;
        return Ok(Value::Bool(false));
    }
    let Some(path) = check_path(host, f, &filename, 2, false, false, None)? else {
        return Ok(Value::Bool(false));
    };
    let bio = super::bio_file(&path, WRITE);
    let mut ok = false;
    if !bio.is_null() {
        unsafe {
            if (notext || sys::X509_print(bio.p, cert.p) != 0)
                && sys::PEM_write_bio_X509(bio.p, cert.p) != 0
            {
                ok = true;
            } else {
                store_errors();
            }
        }
    } else {
        store_errors();
        warn(
            host,
            f,
            &format!("Error opening file {}", path.to_string_lossy()),
        )?;
    }
    if !bio.is_null() && unsafe { sys::BIO_free(bio.release()) } == 0 {
        store_errors();
    }
    Ok(Value::Bool(ok))
}

fn fn_export(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_x509_export";
    let c = cert_arg(host, args, f)?;
    let notext = bool_arg(host, args, 2, f, true)?;
    let cert = x509_from_param(host, f, &c, 1)?;
    if cert.is_null() {
        warn(host, f, "X.509 Certificate cannot be retrieved")?;
        return Ok(Value::Bool(false));
    }
    let bio = mem_bio();
    if bio.is_null() {
        store_errors();
        return Ok(Value::Bool(false));
    }
    unsafe {
        if !notext && sys::X509_print(bio.p, cert.p) == 0 {
            store_errors();
        } else if sys::PEM_write_bio_X509(bio.p, cert.p) != 0 {
            set_ref(args, 1, Value::str(bio_bytes(bio.p)));
            return Ok(Value::Bool(true));
        } else {
            store_errors();
        }
    }
    Ok(Value::Bool(false))
}

fn fn_fingerprint(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_x509_fingerprint";
    let c = cert_arg(host, args, f)?;
    let method = match args.get(1) {
        Some(_) => str_arg(host, args, 1, f)?,
        None => b"sha1".to_vec(),
    };
    let raw = bool_arg(host, args, 2, f, false)?;
    let cert = x509_from_param(host, f, &c, 1)?;
    if cert.is_null() {
        warn(host, f, "X.509 Certificate cannot be retrieved")?;
        return Ok(Value::Bool(false));
    }
    Ok(
        match backend::x509_fingerprint(host, f, cert.p, &method, raw)? {
            Some(s) => Value::str(s),
            None => Value::Bool(false),
        },
    )
}

fn fn_check_private_key(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_x509_check_private_key";
    let c = cert_arg(host, args, f)?;
    let cert = x509_from_param(host, f, &c, 1)?;
    if cert.is_null() {
        return Ok(Value::Bool(false));
    }
    let k = backend::pkey_from_zval(host, f, &arg(args, 1), false, Some(b""), 2)?;
    if k.is_null() {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(
        unsafe { sys::X509_check_private_key(cert.p, k.p) } != 0,
    ))
}

fn fn_verify(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_x509_verify";
    let c = cert_arg(host, args, f)?;
    let cert = x509_from_param(host, f, &c, 1)?;
    if cert.is_null() {
        return Ok(Value::Int(-1));
    }
    let k = backend::pkey_from_zval(host, f, &arg(args, 1), true, None, 2)?;
    let mut err = -1;
    if !k.is_null() {
        err = unsafe { sys::X509_verify(cert.p, k.p) };
        if err < 0 {
            store_errors();
        }
    }
    Ok(Value::Int(err as i64))
}

fn fn_parse(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_x509_parse";
    let c = cert_arg(host, args, f)?;
    let short = bool_arg(host, args, 1, f, true)?;
    let cert = x509_from_param(host, f, &c, 1)?;
    if cert.is_null() {
        return Ok(Value::Bool(false));
    }
    let x = cert.p;
    let mut out = PhpArray::new();
    unsafe {
        let subject = sys::X509_get_subject_name(x);
        let name = sys::X509_NAME_oneline(subject, ptr::null_mut(), 0);
        if name.is_null() {
            store_errors();
            return Ok(Value::Bool(false));
        }
        out.insert(key("name"), Value::str(cstr_bytes(name).to_vec()));
        openssl_free(name);
        add_assoc_name_entry(&mut out, Some("subject"), subject, short);
        out.insert(
            key("hash"),
            Value::str(format!("{:08x}", sys::X509_subject_name_hash(x))),
        );
        add_assoc_name_entry(
            &mut out,
            Some("issuer"),
            sys::X509_get_issuer_name(x),
            short,
        );
        out.insert(key("version"), Value::Int(sys::X509_get_version(x) as i64));

        let serial = sys::X509_get_serialNumber(x);
        let bn = sys::ASN1_INTEGER_to_BN(serial, ptr::null_mut());
        if bn.is_null() {
            store_errors();
            return Ok(Value::Bool(false));
        }
        let hex = sys::BN_bn2hex(bn);
        sys::BN_free(bn);
        if hex.is_null() {
            store_errors();
            return Ok(Value::Bool(false));
        }
        let dec = sys::i2s_ASN1_INTEGER(ptr::null_mut(), serial);
        if dec.is_null() {
            openssl_free(hex);
            store_errors();
            return Ok(Value::Bool(false));
        }
        out.insert(key("serialNumber"), Value::str(cstr_bytes(dec).to_vec()));
        openssl_free(dec);
        out.insert(key("serialNumberHex"), Value::str(cstr_bytes(hex).to_vec()));
        openssl_free(hex);

        let nb = sys::X509_get0_notBefore(x);
        let na = sys::X509_get0_notAfter(x);
        out.insert(key("validFrom"), Value::str(asn1_string_bytes(nb.cast())));
        out.insert(key("validTo"), Value::str(asn1_string_bytes(na.cast())));
        let from = asn1_time_to_time_t(host, f, nb)?;
        out.insert(key("validFrom_time_t"), Value::Int(from));
        let to = asn1_time_to_time_t(host, f, na)?;
        out.insert(key("validTo_time_t"), Value::Int(to));

        let alias = sys::X509_alias_get0(x, ptr::null_mut());
        if !alias.is_null() {
            out.insert(key("alias"), Value::str(cstr_bytes(alias.cast()).to_vec()));
        }
        let sig_nid = sys::X509_get_signature_nid(x);
        out.insert(
            key("signatureTypeSN"),
            Value::str(cstr_bytes(sys::OBJ_nid2sn(sig_nid)).to_vec()),
        );
        out.insert(
            key("signatureTypeLN"),
            Value::str(cstr_bytes(sys::OBJ_nid2ln(sig_nid)).to_vec()),
        );
        out.insert(key("signatureTypeNID"), Value::Int(sig_nid as i64));

        let mut purposes = PhpArray::new();
        for i in 0..sys::X509_PURPOSE_get_count() {
            let purp = sys::X509_PURPOSE_get0(i);
            let id = sys::X509_PURPOSE_get_id(purp);
            let mut sub = PhpArray::new();
            sub.insert(
                ArrayKey::Int(0),
                Value::Bool(sys::X509_check_purpose(x, id, 0) != 0),
            );
            sub.insert(
                ArrayKey::Int(1),
                Value::Bool(sys::X509_check_purpose(x, id, 1) != 0),
            );
            let pname = if short {
                sys::X509_PURPOSE_get0_sname(purp)
            } else {
                sys::X509_PURPOSE_get0_name(purp)
            };
            sub.insert(ArrayKey::Int(2), Value::str(cstr_bytes(pname).to_vec()));
            purposes.insert(ArrayKey::Int(id as i64), Value::Array(sub));
        }
        out.insert(key("purposes"), Value::Array(purposes));

        let mut exts = PhpArray::new();
        for i in 0..sys::X509_get_ext_count(x) {
            let ext = sys::X509_get_ext(x, i);
            let obj = sys::X509_EXTENSION_get_object(ext);
            let nid = sys::OBJ_obj2nid(obj);
            let extname: Vec<u8> = if nid != sys::NID_undef {
                cstr_bytes(sys::OBJ_nid2sn(nid)).to_vec()
            } else {
                let mut buf = [0 as c_char; 256];
                sys::OBJ_obj2txt(buf.as_mut_ptr(), 255, obj, 1);
                cstr_bytes(buf.as_ptr()).to_vec()
            };
            let bio = mem_bio();
            if bio.is_null() {
                store_errors();
                return Ok(Value::Bool(false));
            }
            let k = ArrayKey::from_bytes(&extname);
            if nid == sys::NID_subject_alt_name {
                if backend::subject_alt_name(bio.p, ext) {
                    exts.insert(k, Value::str(bio_bytes(bio.p)));
                } else {
                    return Ok(Value::Bool(false));
                }
            } else if sys::X509V3_EXT_print(bio.p, ext, 0, 0) > 0 {
                exts.insert(k, Value::str(bio_bytes(bio.p)));
            } else {
                exts.insert(
                    k,
                    Value::str(asn1_string_bytes(
                        sys::X509_EXTENSION_get_data(ext) as *const sys::ASN1_STRING
                    )),
                );
            }
        }
        out.insert(key("extensions"), Value::Array(exts));
    }
    Ok(Value::Array(out))
}

fn fn_checkpurpose(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_x509_checkpurpose";
    let c = cert_arg(host, args, f)?;
    let purpose = int_arg(host, args, 1, f, 0)?;
    let cainfo = match args.get(2).map(crate::zend::zpp::deref) {
        None => None,
        Some(v @ Value::Array(_)) => Some(v),
        Some(v) => return Err(type_error(host, f, 2, &v)),
    };
    let untrusted = opt_str_arg(host, args, 3, f)?;
    let mut chain = backend::CertStack::null();
    if let Some(u) = &untrusted {
        chain = backend::load_all_certs_from_file(host, f, u, 4)?;
        if chain.0.is_null() {
            return Ok(Value::Int(-1));
        }
    }
    let store = backend::setup_verify(host, f, cainfo.as_ref(), 3)?;
    if store.is_null() {
        return Ok(Value::Int(-1));
    }
    let cert = x509_from_param(host, f, &c, 1)?;
    if cert.is_null() {
        return Ok(Value::Int(-1));
    }
    let ret = backend::check_cert(host, f, store.p, cert.p, chain.0, purpose as i32)?;
    Ok(if ret != 0 && ret != 1 {
        Value::Int(ret as i64)
    } else {
        Value::Bool(ret == 1)
    })
}

fn fn_read(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_x509_read";
    let c = cert_arg(host, args, f)?;
    let cert = x509_from_param(host, f, &c, 1)?;
    if cert.is_null() {
        warn(host, f, "X.509 Certificate cannot be retrieved")?;
        return Ok(Value::Bool(false));
    }
    let x = if cert.is_owned() {
        cert.release()
    } else {
        let d = unsafe { sys::X509_dup(cert.p) };
        if d.is_null() {
            warn(host, f, "X.509 Certificate could not be duplicated")?;
            return Ok(Value::Bool(false));
        }
        d
    };
    super::cert_object(host, x)
}

fn fn_free(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_x509_free";
    let v = arg(args, 0);
    if super::cert_of(&v).is_none() {
        return Err(type_error(host, f, 0, &v));
    }
    Ok(Value::Null)
}
