//! Certificate signing requests: openssl_csr_* (php-src openssl.c and
//! openssl_backend_common.c).

use std::ffi::{c_long, CStr, CString};
use std::ptr;

use rhp_values::{ArrayKey, PhpArray, Value};

use super::backend::{
    self, add_assoc_name_entry, conf_get_string, conf_section, pkey_free, pkey_from_zval,
    x509_from_param, Req, READ, WRITE,
};
use super::pkey::{opt_array, path_arg};
use super::{
    arg, bio_bytes, bio_file, bool_arg, check_path, cstring, int_arg, mem_bio, mem_buf, obj_or_str,
    opt_str_arg, set_ref, store_errors, sys, try_string, type_error, ObjOrStr, Own, CERT_CLASS,
    CSR_CLASS,
};
use crate::zend::zpp::deref;
use crate::{warn, Host, Thrown};

pub(super) fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            (
                "openssl_csr_export_to_file",
                super::wrapped!(fn_export_to_file),
            ),
            ("openssl_csr_export", super::wrapped!(fn_export)),
            ("openssl_csr_sign", super::wrapped!(fn_sign)),
            ("openssl_csr_new", super::wrapped!(fn_new)),
            ("openssl_csr_get_subject", super::wrapped!(fn_get_subject)),
            (
                "openssl_csr_get_public_key",
                super::wrapped!(fn_get_public_key),
            ),
        ],
    );
}

unsafe extern "C" fn req_free(r: *mut sys::X509_REQ) {
    unsafe { sys::X509_REQ_free(r) };
}

/// php_openssl_csr_from_str.
fn csr_from_str(
    host: &mut dyn Host,
    f: &str,
    s: &[u8],
    arg_num: usize,
) -> Result<Own<sys::X509_REQ>, Thrown> {
    let bio = if s.len() > 7 && s.starts_with(b"file://") {
        let Some(path) = check_path(host, f, s, arg_num, true, false, None)? else {
            return Ok(Own::null());
        };
        bio_file(&path, READ)
    } else {
        mem_buf(s)
    };
    if bio.is_null() {
        store_errors();
        return Ok(Own::null());
    }
    let r = backend::pem_read_bio_x509_req(bio.p);
    if r.is_null() {
        store_errors();
    }
    Ok(Own::new(r, req_free))
}

/// php_openssl_csr_from_param.
fn csr_from_param(
    host: &mut dyn Host,
    f: &str,
    a: &ObjOrStr,
    arg_num: usize,
) -> Result<Own<sys::X509_REQ>, Thrown> {
    match a {
        ObjOrStr::Obj(o) => Ok(Own::borrowed(super::csr_of(o).unwrap_or(ptr::null_mut()))),
        ObjOrStr::Str(s) => csr_from_str(host, f, s, arg_num),
    }
}

fn fn_export_to_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_csr_export_to_file";
    let a = obj_or_str(host, args, 0, f, CSR_CLASS)?;
    let filename = path_arg(host, args, 1, f)?;
    let notext = bool_arg(host, args, 2, f, true)?;
    let csr = csr_from_param(host, f, &a, 1)?;
    if csr.is_null() {
        warn(
            host,
            f,
            "X.509 Certificate Signing Request cannot be retrieved",
        )?;
        return Ok(Value::Bool(false));
    }
    let Some(path) = check_path(host, f, &filename, 2, false, false, None)? else {
        return Ok(Value::Bool(false));
    };
    let shown = path.to_string_lossy().into_owned();
    let bio = bio_file(&path, WRITE);
    if bio.is_null() {
        store_errors();
        warn(host, f, &format!("Error opening file {shown}"))?;
        return Ok(Value::Bool(false));
    }
    unsafe {
        if !notext && sys::X509_REQ_print(bio.p, csr.p) == 0 {
            store_errors();
        } else if sys::PEM_write_bio_X509_REQ(bio.p, csr.p) == 0 {
            warn(host, f, &format!("Error writing PEM to file {shown}"))?;
            store_errors();
        } else {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn fn_export(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_csr_export";
    let a = obj_or_str(host, args, 0, f, CSR_CLASS)?;
    let notext = bool_arg(host, args, 2, f, true)?;
    let csr = csr_from_param(host, f, &a, 1)?;
    if csr.is_null() {
        warn(
            host,
            f,
            "X.509 Certificate Signing Request cannot be retrieved",
        )?;
        return Ok(Value::Bool(false));
    }
    let bio = mem_bio();
    unsafe {
        if !notext && sys::X509_REQ_print(bio.p, csr.p) == 0 {
            store_errors();
        } else if sys::PEM_write_bio_X509_REQ(bio.p, csr.p) != 0 {
            set_ref(args, 1, Value::str(bio_bytes(bio.p)));
            return Ok(Value::Bool(true));
        } else {
            store_errors();
        }
    }
    Ok(Value::Bool(false))
}

fn fn_sign(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_csr_sign";
    let a = obj_or_str(host, args, 0, f, CSR_CLASS)?;
    let ca = match arg(args, 1) {
        Value::Null => None,
        _ => Some(obj_or_str(host, args, 1, f, CERT_CLASS)?),
    };
    let num_days = int_arg(host, args, 3, f, 0)?;
    let opts = match args.get(4).map(deref) {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => Some(a),
        Some(v) => return Err(type_error(host, f, 4, &v)),
    };
    let serial = int_arg(host, args, 5, f, 0)?;
    let serial_hex = opt_str_arg(host, args, 6, f)?;

    let csr = csr_from_param(host, f, &a, 1)?;
    if csr.is_null() {
        warn(
            host,
            f,
            "X.509 Certificate Signing Request cannot be retrieved",
        )?;
        return Ok(Value::Bool(false));
    }
    let mut req = Req::new();
    let mut cert: Own<sys::X509> = Own::null();
    if let Some(ca) = &ca {
        cert = x509_from_param(host, f, ca, 2)?;
        if cert.is_null() {
            warn(host, f, "X.509 Certificate cannot be retrieved")?;
            return Ok(Value::Bool(false));
        }
    }
    let priv_key = pkey_from_zval(host, f, &arg(args, 2), false, Some(b""), 3)?;
    if priv_key.is_null() {
        super::warn_unless_thrown(host, f, "Cannot get private key from parameter 3")?;
        return Ok(Value::Bool(false));
    }
    if !cert.is_null() && unsafe { sys::X509_check_private_key(cert.p, priv_key.p) } == 0 {
        store_errors();
        warn(host, f, "Private key does not correspond to signing cert")?;
        return Ok(Value::Bool(false));
    }
    // LONG_MAX / 86400 (LP64)
    let max_days = i64::MAX / 86400;
    if num_days < 0 || num_days > max_days {
        warn(host, f, &format!("Days must be between 0 and {max_days}"))?;
        return Ok(Value::Bool(false));
    }
    if !backend::parse_config(host, f, &mut req, opts.as_ref())? {
        return Ok(Value::Bool(false));
    }
    let key = Own::new(unsafe { sys::X509_REQ_get_pubkey(csr.p) }, pkey_free);
    if key.is_null() {
        store_errors();
        warn(host, f, "Error unpacking public key")?;
        return Ok(Value::Bool(false));
    }
    let i = unsafe { sys::X509_REQ_verify(csr.p, key.p) };
    if i < 0 {
        store_errors();
        warn(host, f, "Signature verification problems")?;
        return Ok(Value::Bool(false));
    } else if i == 0 {
        warn(host, f, "Signature did not match the certificate request")?;
        return Ok(Value::Bool(false));
    }
    let new_cert = Own::new(unsafe { sys::X509_new() }, backend::x509_free);
    if new_cert.is_null() {
        store_errors();
        warn(host, f, "No memory")?;
        return Ok(Value::Bool(false));
    }
    unsafe {
        if sys::X509_set_version(new_cert.p, 2) == 0 {
            return Ok(Value::Bool(false));
        }
        if let Some(hex) = &serial_hex {
            if hex.len() > 200 {
                warn(
                    host,
                    f,
                    "Error parsing serial number because it is too long",
                )?;
                return Ok(Value::Bool(false));
            }
            let bio = mem_buf(hex);
            if bio.is_null() {
                store_errors();
                warn(
                    host,
                    f,
                    "Error parsing serial number because memory allocation failed",
                )?;
                return Ok(Value::Bool(false));
            }
            let mut buf = [0 as std::ffi::c_char; 256];
            let ok = sys::a2i_ASN1_INTEGER(
                bio.p,
                sys::X509_get_serialNumber(new_cert.p),
                buf.as_mut_ptr(),
                buf.len() as i32,
            );
            drop(bio);
            if ok == 0 {
                store_errors();
                warn(host, f, "Error parsing serial number")?;
                return Ok(Value::Bool(false));
            }
        } else if sys::ASN1_INTEGER_set_int64(sys::X509_get_serialNumber(new_cert.p), serial) == 0 {
            store_errors();
            warn(host, f, "Error setting serial number")?;
            return Ok(Value::Bool(false));
        }
        if sys::X509_set_subject_name(new_cert.p, sys::X509_REQ_get_subject_name(csr.p)) == 0 {
            store_errors();
            return Ok(Value::Bool(false));
        }
        let issuer = if cert.is_null() { new_cert.p } else { cert.p };
        if sys::X509_set_issuer_name(new_cert.p, sys::X509_get_subject_name(issuer)) == 0 {
            store_errors();
            return Ok(Value::Bool(false));
        }
        if sys::X509_gmtime_adj(sys::X509_getm_notBefore(new_cert.p), 0).is_null()
            || sys::X509_gmtime_adj(
                sys::X509_getm_notAfter(new_cert.p),
                (60 * 60 * 24 * num_days) as c_long,
            )
            .is_null()
        {
            store_errors();
            return Ok(Value::Bool(false));
        }
        if sys::X509_set_pubkey(new_cert.p, key.p) == 0 {
            store_errors();
            return Ok(Value::Bool(false));
        }
        if let Some(section) = &req.extensions_section {
            let mut ctx: sys::X509V3_CTX = std::mem::zeroed();
            sys::X509V3_set_ctx(&mut ctx, issuer, new_cert.p, csr.p, ptr::null_mut(), 0);
            sys::X509V3_set_nconf(&mut ctx, req.req_config.p);
            if sys::X509V3_EXT_add_nconf(req.req_config.p, &mut ctx, section.as_ptr(), new_cert.p)
                == 0
            {
                store_errors();
                return Ok(Value::Bool(false));
            }
        }
        if sys::X509_sign(new_cert.p, priv_key.p, req.md_alg) == 0 {
            store_errors();
            warn(host, f, "Failed to sign it")?;
            return Ok(Value::Bool(false));
        }
    }
    super::cert_object(host, new_cert.release())
}

/// php_openssl_csr_add_subj_entry: `Ok(false)` is FAILURE.
fn add_subj_entry(
    host: &mut dyn Host,
    f: &str,
    item: &Value,
    subj: *mut sys::X509_NAME,
    nid: i32,
) -> Result<bool, Thrown> {
    let s = try_string(host, item)?;
    let c = cstring(&s);
    if unsafe {
        sys::X509_NAME_add_entry_by_NID(
            subj,
            nid,
            sys::MBSTRING_UTF8,
            c.as_ptr() as *mut u8,
            -1,
            -1,
            0,
        )
    } == 0
    {
        store_errors();
        warn(
            host,
            f,
            &format!(
                "dn: add_entry_by_NID {nid} -> {} (failed; check error queue and value of string_mask OpenSSL option if illegal characters are reported)",
                c.to_string_lossy()
            ),
        )?;
        return Ok(false);
    }
    Ok(true)
}

/// php_openssl_csr_make: `Ok(false)` is FAILURE.
fn csr_make(
    host: &mut dyn Host,
    f: &str,
    req: &Req,
    csr: *mut sys::X509_REQ,
    dn: &PhpArray,
    attribs: Option<&PhpArray>,
) -> Result<bool, Thrown> {
    let cfg = req.req_config.p;
    let dn_sect = unsafe {
        let s = sys::NCONF_get_string(
            cfg,
            req.section_name.as_ptr(),
            c"distinguished_name".as_ptr(),
        );
        if s.is_null() {
            store_errors();
            return Ok(false);
        }
        CStr::from_ptr(s).to_owned()
    };
    let Some(dn_sk) = conf_section(cfg, &dn_sect) else {
        store_errors();
        return Ok(false);
    };
    let attr_sk = match conf_get_string(cfg, Some(&req.section_name), c"attributes") {
        None => None,
        Some(s) => match conf_section(cfg, &s) {
            Some(v) => Some(v),
            None => {
                store_errors();
                return Ok(false);
            }
        },
    };
    if unsafe { sys::X509_REQ_set_version(csr, 0) } == 0 {
        store_errors();
        return Ok(false);
    }
    let subj = unsafe { sys::X509_REQ_get_subject_name(csr) };
    // the values from the dn hash
    for (k, item) in dn.iter() {
        let ArrayKey::Str(name) = k else { continue };
        let cname = cstring(name.as_bytes());
        let nid = unsafe { sys::OBJ_txt2nid(cname.as_ptr()) };
        if nid != sys::NID_undef {
            if let Value::Array(sub) = deref(item) {
                for subitem in sub.values() {
                    if !add_subj_entry(host, f, subitem, subj, nid)? {
                        return Ok(false);
                    }
                }
            } else if !add_subj_entry(host, f, item, subj, nid)? {
                return Ok(false);
            }
        } else {
            warn(
                host,
                f,
                &format!(
                    "dn: {} is not a recognized name",
                    String::from_utf8_lossy(name.as_bytes())
                ),
            )?;
        }
    }
    // then the defaults from the config file
    for (name, value) in &dn_sk {
        let n = name.to_bytes();
        if n.len() < 9 || !n.ends_with(b"_default") {
            continue;
        }
        let mut ty = &n[..(n.len() - 8).min(200)];
        // skip a leading "X.", "X:" or "X," (several instances)
        if let Some(p) = ty.iter().position(|&c| c == b':' || c == b',' || c == b'.') {
            if p + 1 < ty.len() {
                ty = &ty[p + 1..];
            }
        }
        let cty = CString::new(ty).unwrap_or_default();
        unsafe {
            let nid = sys::OBJ_txt2nid(cty.as_ptr());
            if sys::X509_NAME_get_index_by_NID(subj, nid, -1) >= 0 {
                continue;
            }
            if sys::X509_NAME_add_entry_by_txt(
                subj,
                cty.as_ptr(),
                sys::MBSTRING_UTF8,
                value.as_ptr().cast(),
                -1,
                -1,
                0,
            ) == 0
            {
                store_errors();
                warn(
                    host,
                    f,
                    &format!(
                        "add_entry_by_txt {} -> {} (failed)",
                        cty.to_string_lossy(),
                        value.to_string_lossy()
                    ),
                )?;
                return Ok(false);
            }
            if sys::X509_NAME_entry_count(subj) == 0 {
                warn(host, f, "No objects specified in config file")?;
                return Ok(false);
            }
        }
    }
    if let Some(attribs) = attribs {
        for (k, item) in attribs.iter() {
            let ArrayKey::Str(name) = k else {
                warn(host, f, "attributes: numeric fild names are not supported")?;
                continue;
            };
            let cname = cstring(name.as_bytes());
            let nid = unsafe { sys::OBJ_txt2nid(cname.as_ptr()) };
            if nid != sys::NID_undef {
                let s = try_string(host, item)?;
                if unsafe {
                    sys::X509_REQ_add1_attr_by_NID(
                        csr,
                        nid,
                        sys::MBSTRING_UTF8,
                        s.as_ptr(),
                        s.len() as i32,
                    )
                } == 0
                {
                    store_errors();
                    warn(
                        host,
                        f,
                        &format!(
                            "attributes: add_attr_by_NID {nid} -> {} (failed)",
                            cstring(&s).to_string_lossy()
                        ),
                    )?;
                    return Ok(false);
                }
            } else {
                warn(
                    host,
                    f,
                    &format!(
                        "attributes: {} is not a recognized attribute name",
                        cname.to_string_lossy()
                    ),
                )?;
            }
        }
        for (name, value) in attr_sk.iter().flatten() {
            unsafe {
                let nid = sys::OBJ_txt2nid(name.as_ptr());
                if sys::X509_REQ_get_attr_by_NID(csr, nid, -1) >= 0 {
                    continue;
                }
                if sys::X509_REQ_add1_attr_by_txt(
                    csr,
                    name.as_ptr(),
                    sys::MBSTRING_UTF8,
                    value.as_ptr().cast(),
                    -1,
                ) == 0
                {
                    store_errors();
                    warn(
                        host,
                        f,
                        &format!(
                            "add1_attr_by_txt {} -> {} (failed; check error queue and value of string_mask OpenSSL option if illegal characters are reported)",
                            name.to_string_lossy(),
                            value.to_string_lossy()
                        ),
                    )?;
                    return Ok(false);
                }
            }
        }
    }
    if unsafe { sys::X509_REQ_set_pubkey(csr, req.priv_key.p) } == 0 {
        store_errors();
    }
    Ok(true)
}

fn fn_new(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_csr_new";
    let dn = match arg(args, 0) {
        Value::Array(a) => a,
        v => return Err(type_error(host, f, 0, &v)),
    };
    let opts = opt_array(args, 2);
    let attribs = opt_array(args, 3);
    let mut req = Req::new();
    let mut ret = Value::Bool(false);
    if !backend::parse_config(host, f, &mut req, opts.as_ref())? {
        return Ok(ret);
    }
    let out_pkey = arg(args, 1);
    let mut we_made_the_key = false;
    if !matches!(out_pkey, Value::Null) {
        req.priv_key = pkey_from_zval(host, f, &out_pkey, false, None, 2)?;
    }
    if req.priv_key.is_null() {
        backend::generate_private_key(host, f, &mut req)?;
        we_made_the_key = true;
    }
    if req.priv_key.is_null() {
        warn(host, f, "Unable to generate a private key")?;
        return Ok(ret);
    }
    let csr = Own::new(unsafe { sys::X509_REQ_new() }, req_free);
    if csr.is_null() {
        store_errors();
        return Ok(ret);
    }
    if !csr_make(host, f, &req, csr.p, &dn, attribs.as_ref())? {
        return Ok(ret);
    }
    unsafe {
        let mut ctx: sys::X509V3_CTX = std::mem::zeroed();
        sys::X509V3_set_ctx(
            &mut ctx,
            ptr::null_mut(),
            ptr::null_mut(),
            csr.p,
            ptr::null_mut(),
            0,
        );
        sys::X509V3_set_nconf(&mut ctx, req.req_config.p);
        if let Some(section) = &req.request_extensions_section {
            if sys::X509V3_EXT_REQ_add_nconf(req.req_config.p, &mut ctx, section.as_ptr(), csr.p)
                == 0
            {
                store_errors();
                warn(
                    host,
                    f,
                    &format!(
                        "Error loading extension section {}",
                        section.to_string_lossy()
                    ),
                )?;
                return Ok(ret);
            }
        }
        ret = Value::Bool(true);
        if sys::X509_REQ_sign(csr.p, req.priv_key.p, req.md_alg) != 0 {
            ret = super::csr_object(host, csr.release())?;
        } else {
            store_errors();
            warn(host, f, "Error signing request")?;
        }
    }
    if we_made_the_key {
        let k = std::mem::replace(&mut req.priv_key, Own::null()).release();
        let obj = super::key_object(host, k, true)?;
        set_ref(args, 1, obj);
    }
    Ok(ret)
}

fn fn_get_subject(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_csr_get_subject";
    let a = obj_or_str(host, args, 0, f, CSR_CLASS)?;
    let short = bool_arg(host, args, 1, f, true)?;
    let csr = csr_from_param(host, f, &a, 1)?;
    if csr.is_null() {
        return Ok(Value::Bool(false));
    }
    let mut out = PhpArray::new();
    add_assoc_name_entry(
        &mut out,
        None,
        unsafe { sys::X509_REQ_get_subject_name(csr.p) },
        short,
    );
    Ok(Value::Array(out))
}

fn fn_get_public_key(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_csr_get_public_key";
    let a = obj_or_str(host, args, 0, f, CSR_CLASS)?;
    bool_arg(host, args, 1, f, true)?;
    let csr = csr_from_param(host, f, &a, 1)?;
    if csr.is_null() {
        return Ok(Value::Bool(false));
    }
    let orig = Own::new(unsafe { sys::X509_REQ_get_pubkey(csr.p) }, pkey_free);
    let pubkey = backend::extract_public_key(orig.p);
    drop(orig);
    drop(csr);
    if pubkey.is_null() {
        store_errors();
        return Ok(Value::Bool(false));
    }
    super::key_object(host, pubkey, false)
}
