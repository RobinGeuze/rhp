//! Keys, signatures and the RSA/seal primitives: openssl_pkey_*,
//! openssl_sign/verify, openssl_{public,private}_{encrypt,decrypt},
//! openssl_seal/open, openssl_dh_compute_key, openssl_pkey_derive (php-src
//! ext/openssl/openssl.c and openssl_backend_v3.c).

use std::ffi::{c_char, c_int, CStr};
use std::ptr;

use rhp_values::{PhpArray, Value};

use super::backend::{
    self, add_bn, bn_from, key, md_by_name, md_from_algo, pkey_ctx_from_name, pkey_ctx_from_pkey,
    pkey_free, pkey_from_zval, release_cipher, release_md, Bn, Req, WRITE,
};
use super::{
    arg, bio_bytes, check_path, cstring, int_arg, key_arg, key_object, mem_bio, mem_buf,
    openssl_free, opt_str_arg, set_ref, store_errors, str_arg, sys, type_error, value_error, Own,
    PROPQ,
};
use crate::zend::zpp::deref;
use crate::{warn, Host, Thrown};

pub(super) fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("openssl_pkey_new", super::wrapped!(fn_pkey_new)),
            (
                "openssl_pkey_export_to_file",
                super::wrapped!(fn_export_to_file),
            ),
            ("openssl_pkey_export", super::wrapped!(fn_export)),
            ("openssl_pkey_get_public", super::wrapped!(fn_get_public)),
            ("openssl_get_publickey", super::wrapped!(fn_get_public)),
            ("openssl_pkey_free", super::wrapped!(fn_pkey_free)),
            ("openssl_free_key", super::wrapped!(fn_free_key)),
            ("openssl_pkey_get_private", super::wrapped!(fn_get_private)),
            ("openssl_get_privatekey", super::wrapped!(fn_get_private)),
            ("openssl_pkey_get_details", super::wrapped!(fn_get_details)),
            ("openssl_dh_compute_key", super::wrapped!(fn_dh_compute_key)),
            ("openssl_pkey_derive", super::wrapped!(fn_pkey_derive)),
            (
                "openssl_private_encrypt",
                super::wrapped!(fn_private_encrypt),
            ),
            (
                "openssl_private_decrypt",
                super::wrapped!(fn_private_decrypt),
            ),
            ("openssl_public_encrypt", super::wrapped!(fn_public_encrypt)),
            ("openssl_public_decrypt", super::wrapped!(fn_public_decrypt)),
            ("openssl_sign", super::wrapped!(fn_sign)),
            ("openssl_verify", super::wrapped!(fn_verify)),
            ("openssl_seal", super::wrapped!(fn_seal)),
            ("openssl_open", super::wrapped!(fn_open)),
        ],
    );
}

/// A `p` (path) parameter: a string without NUL bytes.
pub(super) fn path_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
) -> Result<Vec<u8>, Thrown> {
    let s = str_arg(host, args, i, fname)?;
    if s.contains(&0) {
        return Err(value_error(fname, i, "must not contain any null bytes"));
    }
    Ok(s)
}

/// An optional `?array` parameter.
pub(super) fn opt_array(args: &[Value], i: usize) -> Option<PhpArray> {
    match args.get(i).map(deref) {
        Some(Value::Array(a)) => Some(a),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// openssl_pkey_new and its php_openssl_pkey_init_* helpers
// ---------------------------------------------------------------------------

unsafe extern "C" fn param_bld_free(b: *mut sys::OSSL_PARAM_BLD) {
    unsafe { sys::OSSL_PARAM_BLD_free(b) };
}

unsafe extern "C" fn param_free(p: *mut sys::OSSL_PARAM) {
    unsafe { sys::OSSL_PARAM_free(p) };
}

fn bld_new() -> Own<sys::OSSL_PARAM_BLD> {
    Own::new(unsafe { sys::OSSL_PARAM_BLD_new() }, param_bld_free)
}

fn push_bn(bld: &Own<sys::OSSL_PARAM_BLD>, name: &CStr, bn: &Bn) -> bool {
    unsafe { sys::OSSL_PARAM_BLD_push_BN(bld.p, name.as_ptr(), bn.0) != 0 }
}

fn to_param(bld: &Own<sys::OSSL_PARAM_BLD>) -> Own<sys::OSSL_PARAM> {
    Own::new(unsafe { sys::OSSL_PARAM_BLD_to_param(bld.p) }, param_free)
}

fn fromdata(ctx: &Own<sys::EVP_PKEY_CTX>, params: &Own<sys::OSSL_PARAM>) -> *mut sys::EVP_PKEY {
    let mut k: *mut sys::EVP_PKEY = ptr::null_mut();
    unsafe {
        if sys::EVP_PKEY_fromdata_init(ctx.p) <= 0
            || sys::EVP_PKEY_fromdata(ctx.p, &mut k, sys::EVP_PKEY_KEYPAIR, params.p) <= 0
        {
            return ptr::null_mut();
        }
    }
    k
}

fn keygen(ctx: &Own<sys::EVP_PKEY_CTX>) -> *mut sys::EVP_PKEY {
    let mut k: *mut sys::EVP_PKEY = ptr::null_mut();
    unsafe {
        if sys::EVP_PKEY_keygen_init(ctx.p) <= 0 || sys::EVP_PKEY_keygen(ctx.p, &mut k) <= 0 {
            return ptr::null_mut();
        }
    }
    k
}

/// php_openssl_pkey_init_rsa.
fn init_rsa(data: &PhpArray) -> *mut sys::EVP_PKEY {
    let ctx = pkey_ctx_from_name(c"RSA");
    let bld = bld_new();
    let (n, e, d) = (bn_from(data, "n"), bn_from(data, "e"), bn_from(data, "d"));
    let (p, q) = (bn_from(data, "p"), bn_from(data, "q"));
    let (dmp1, dmq1, iqmp) = (
        bn_from(data, "dmp1"),
        bn_from(data, "dmq1"),
        bn_from(data, "iqmp"),
    );
    let mut pkey = ptr::null_mut();
    if !ctx.is_null() && !bld.is_null() && n.is_some() && d.is_some() {
        push_bn(&bld, c"n", &n);
        push_bn(&bld, c"d", &d);
        for (name, bn) in [
            (c"e", &e),
            (c"rsa-factor1", &p),
            (c"rsa-factor2", &q),
            (c"rsa-exponent1", &dmp1),
            (c"rsa-exponent2", &dmq1),
            (c"rsa-coefficient1", &iqmp),
        ] {
            if bn.is_some() {
                push_bn(&bld, name, bn);
            }
        }
        let params = to_param(&bld);
        if !params.is_null() {
            pkey = fromdata(&ctx, &params);
        }
    }
    store_errors();
    pkey
}

/// php_openssl_pkey_init_dsa.
fn init_dsa(data: &PhpArray) -> (*mut sys::EVP_PKEY, bool) {
    let mut ctx = pkey_ctx_from_name(c"DSA");
    let bld = bld_new();
    let (p, q, g) = (bn_from(data, "p"), bn_from(data, "q"), bn_from(data, "g"));
    let (priv_key, pub_key) = (bn_from(data, "priv_key"), bn_from(data, "pub_key"));
    let mut out = (ptr::null_mut(), false);
    'done: {
        if ctx.is_null() || bld.is_null() || !p.is_some() || !q.is_some() || !g.is_some() {
            break 'done;
        }
        push_bn(&bld, c"p", &p);
        push_bn(&bld, c"q", &q);
        push_bn(&bld, c"g", &g);
        // php ignores priv_key without pub_key here (unlike DH).
        if pub_key.is_some() {
            push_bn(&bld, c"pub", &pub_key);
            if priv_key.is_some() {
                push_bn(&bld, c"priv", &priv_key);
            }
        }
        let params = to_param(&bld);
        if params.is_null() {
            break 'done;
        }
        let param_key = Own::new(fromdata(&ctx, &params), pkey_free);
        if param_key.is_null() {
            break 'done;
        }
        if pub_key.is_some() {
            out = (param_key.release(), priv_key.is_some());
        } else {
            out.1 = true;
            ctx = pkey_ctx_from_pkey(param_key.p);
            out.0 = keygen(&ctx);
        }
    }
    store_errors();
    out
}

/// php_openssl_dh_pub_from_priv.
fn dh_pub_from_priv(priv_key: &Bn, g: &Bn, p: &Bn) -> Bn {
    unsafe {
        let pub_key = Bn(sys::BN_new());
        if pub_key.0.is_null() {
            store_errors();
            return pub_key;
        }
        let ct = Bn(sys::BN_new());
        if ct.0.is_null() {
            store_errors();
            return Bn(ptr::null_mut());
        }
        let ctx = sys::BN_CTX_new();
        if ctx.is_null() {
            store_errors();
            return Bn(ptr::null_mut());
        }
        sys::BN_with_flags(ct.0, priv_key.0, sys::BN_FLG_CONSTTIME);
        let ok = sys::BN_mod_exp_mont(pub_key.0, g.0, ct.0, p.0, ctx, ptr::null_mut()) != 0;
        sys::BN_CTX_free(ctx);
        if !ok {
            store_errors();
            return Bn(ptr::null_mut());
        }
        pub_key
    }
}

/// php_openssl_pkey_init_dh.
fn init_dh(data: &PhpArray) -> (*mut sys::EVP_PKEY, bool) {
    let mut ctx = pkey_ctx_from_name(c"DH");
    let bld = bld_new();
    let (p, q, g) = (bn_from(data, "p"), bn_from(data, "q"), bn_from(data, "g"));
    let (priv_key, mut pub_key) = (bn_from(data, "priv_key"), bn_from(data, "pub_key"));
    let mut out = (ptr::null_mut(), false);
    'done: {
        if ctx.is_null() || bld.is_null() || !p.is_some() || !g.is_some() {
            break 'done;
        }
        push_bn(&bld, c"p", &p);
        push_bn(&bld, c"g", &g);
        if q.is_some() {
            push_bn(&bld, c"q", &q);
        }
        if priv_key.is_some() {
            push_bn(&bld, c"priv", &priv_key);
            if !pub_key.is_some() {
                pub_key = dh_pub_from_priv(&priv_key, &g, &p);
                if !pub_key.is_some() {
                    break 'done;
                }
            }
        }
        if pub_key.is_some() {
            push_bn(&bld, c"pub", &pub_key);
        }
        let params = to_param(&bld);
        if params.is_null() {
            break 'done;
        }
        let param_key = Own::new(fromdata(&ctx, &params), pkey_free);
        if param_key.is_null() {
            break 'done;
        }
        if pub_key.is_some() || priv_key.is_some() {
            out = (param_key.release(), priv_key.is_some());
        } else {
            out.1 = true;
            ctx = pkey_ctx_from_pkey(param_key.p);
            out.0 = keygen(&ctx);
        }
    }
    store_errors();
    out
}

unsafe extern "C" fn ec_group_free(g: *mut sys::EC_GROUP) {
    unsafe { sys::EC_GROUP_free(g) };
}

unsafe extern "C" fn ec_point_free(p: *mut sys::EC_POINT) {
    unsafe { sys::EC_POINT_free(p) };
}

unsafe extern "C" fn bn_ctx_free(c: *mut sys::BN_CTX) {
    unsafe { sys::BN_CTX_free(c) };
}

/// php_openssl_pkey_init_ec.
#[allow(unused_assignments)]
fn init_ec(
    host: &mut dyn Host,
    fname: &str,
    data: &PhpArray,
) -> Result<(*mut sys::EVP_PKEY, bool), Thrown> {
    let mut ctx = pkey_ctx_from_name(c"EC");
    let bctx = Own::new(unsafe { sys::BN_CTX_new() }, bn_ctx_free);
    let bld = bld_new();
    let mut out = (ptr::null_mut(), false);
    let mut group: Own<sys::EC_GROUP> = Own::null();
    let mut point_g: Own<sys::EC_POINT> = Own::null();
    let mut point_q: Own<sys::EC_POINT> = Own::null();
    let mut bufs: Vec<*mut u8> = Vec::new();
    let mut bns: Vec<Bn> = Vec::new();
    // OSSL_PARAM_BLD holds strings by reference until OSSL_PARAM_BLD_to_param.
    let mut keep: Vec<Vec<u8>> = Vec::new();
    let (mut has_d, mut has_xy) = (false, false);
    let str_of = |name: &str| match data.get(&key(name)).map(deref) {
        Some(Value::Str(s)) if !s.is_empty() => Some(s.as_bytes().to_vec()),
        _ => None,
    };
    unsafe {
        'done: {
            if ctx.is_null() || bld.is_null() || bctx.is_null() {
                break 'done;
            }
            let mut nid = sys::NID_undef;
            if let Some(curve) = str_of("curve_name") {
                let c = cstring(&curve);
                nid = sys::OBJ_sn2nid(c.as_ptr());
                if nid == sys::NID_undef {
                    warn(
                        host,
                        fname,
                        &format!(
                            "Unknown elliptic curve (short) name {}",
                            c.to_string_lossy()
                        ),
                    )?;
                    break 'done;
                }
                group = Own::new(
                    sys::EC_GROUP_new_by_curve_name_ex(super::libctx(), PROPQ, nid),
                    ec_group_free,
                );
                keep.push(curve);
                let curve = keep.last().map(Vec::as_slice).unwrap_or_default();
                if group.is_null()
                    || sys::OSSL_PARAM_BLD_push_utf8_string(
                        bld.p,
                        c"group".as_ptr(),
                        curve.as_ptr().cast(),
                        curve.len(),
                    ) == 0
                {
                    break 'done;
                }
            } else {
                let (p, a, b, order) = (
                    bn_from(data, "p"),
                    bn_from(data, "a"),
                    bn_from(data, "b"),
                    bn_from(data, "order"),
                );
                if !(p.is_some() && a.is_some() && b.is_some() && order.is_some()) {
                    if !p.is_some() && !a.is_some() && !b.is_some() && !order.is_some() {
                        warn(host, fname, "Missing params: curve_name")?;
                    } else {
                        warn(host, fname, "Missing params: curve_name or p, a, b, order")?;
                    }
                    break 'done;
                }
                if !push_bn(&bld, c"p", &p)
                    || !push_bn(&bld, c"a", &a)
                    || !push_bn(&bld, c"b", &b)
                    || !push_bn(&bld, c"order", &order)
                    || sys::OSSL_PARAM_BLD_push_utf8_string(
                        bld.p,
                        c"field-type".as_ptr(),
                        c"prime-field".as_ptr(),
                        0,
                    ) == 0
                {
                    break 'done;
                }
                group = Own::new(
                    sys::EC_GROUP_new_curve_GFp(p.0, a.0, b.0, bctx.p),
                    ec_group_free,
                );
                if group.is_null() {
                    break 'done;
                }
                point_g = Own::new(sys::EC_POINT_new(group.p), ec_point_free);
                if point_g.is_null() {
                    break 'done;
                }
                if let Some(gen) = str_of("generator") {
                    keep.push(gen);
                    let gen = keep.last().map(Vec::as_slice).unwrap_or_default();
                    if sys::EC_POINT_oct2point(group.p, point_g.p, gen.as_ptr(), gen.len(), bctx.p)
                        == 0
                        || sys::OSSL_PARAM_BLD_push_octet_string(
                            bld.p,
                            c"generator".as_ptr(),
                            gen.as_ptr().cast(),
                            gen.len(),
                        ) == 0
                    {
                        break 'done;
                    }
                } else {
                    let (gx, gy) = (bn_from(data, "g_x"), bn_from(data, "g_y"));
                    if !gx.is_some() || !gy.is_some() {
                        warn(host, fname, "Missing params: generator or g_x and g_y")?;
                        break 'done;
                    }
                    if sys::EC_POINT_set_affine_coordinates(group.p, point_g.p, gx.0, gy.0, bctx.p)
                        == 0
                    {
                        break 'done;
                    }
                    let mut buf: *mut u8 = ptr::null_mut();
                    let n = sys::EC_POINT_point2buf(
                        group.p,
                        point_g.p,
                        sys::POINT_CONVERSION_COMPRESSED,
                        &mut buf,
                        bctx.p,
                    );
                    bufs.push(buf);
                    if n == 0
                        || sys::OSSL_PARAM_BLD_push_octet_string(
                            bld.p,
                            c"generator".as_ptr(),
                            buf.cast(),
                            n,
                        ) == 0
                    {
                        break 'done;
                    }
                }
                if let Some(seed) = str_of("seed") {
                    keep.push(seed);
                    let seed = keep.last().map(Vec::as_slice).unwrap_or_default();
                    if sys::EC_GROUP_set_seed(group.p, seed.as_ptr(), seed.len()) == 0
                        || sys::OSSL_PARAM_BLD_push_octet_string(
                            bld.p,
                            c"seed".as_ptr(),
                            seed.as_ptr().cast(),
                            seed.len(),
                        ) == 0
                    {
                        break 'done;
                    }
                }
                let cofactor = bn_from(data, "cofactor");
                if !push_bn(&bld, c"cofactor", &cofactor)
                    || sys::EC_GROUP_set_generator(group.p, point_g.p, order.0, cofactor.0) == 0
                {
                    break 'done;
                }
                nid = sys::EC_GROUP_check_named_curve(group.p, 0, bctx.p);
                // OSSL_PARAM_BLD keeps pointers to the BIGNUMs until to_param.
                bns.extend([p, a, b, order, cofactor]);
            }
            // custom params are not supported with SM2
            if nid != sys::NID_sm2 {
                let (d, x, y) = (bn_from(data, "d"), bn_from(data, "x"), bn_from(data, "y"));
                has_d = d.is_some();
                has_xy = x.is_some() && y.is_some();
                if has_d {
                    point_q = Own::new(sys::EC_POINT_new(group.p), ec_point_free);
                    if point_q.is_null()
                        || sys::EC_POINT_mul(
                            group.p,
                            point_q.p,
                            d.0,
                            ptr::null(),
                            ptr::null(),
                            bctx.p,
                        ) == 0
                        || !push_bn(&bld, c"priv", &d)
                    {
                        break 'done;
                    }
                } else if has_xy {
                    point_q = Own::new(sys::EC_POINT_new(group.p), ec_point_free);
                    if point_q.is_null()
                        || sys::EC_POINT_set_affine_coordinates(
                            group.p, point_q.p, x.0, y.0, bctx.p,
                        ) == 0
                    {
                        break 'done;
                    }
                }
                if !point_q.is_null() {
                    let mut buf: *mut u8 = ptr::null_mut();
                    let n = sys::EC_POINT_point2buf(
                        group.p,
                        point_q.p,
                        sys::POINT_CONVERSION_COMPRESSED,
                        &mut buf,
                        bctx.p,
                    );
                    bufs.push(buf);
                    if n == 0
                        || sys::OSSL_PARAM_BLD_push_octet_string(
                            bld.p,
                            c"pub".as_ptr(),
                            buf.cast(),
                            n,
                        ) == 0
                    {
                        break 'done;
                    }
                }
                bns.extend([d, x, y]);
            }
            let params = to_param(&bld);
            if params.is_null() {
                break 'done;
            }
            let mut param_key: Own<sys::EVP_PKEY> = Own::null();
            if has_d || has_xy {
                param_key = Own::new(fromdata(&ctx, &params), pkey_free);
                if param_key.is_null() {
                    break 'done;
                }
                ctx = Own::new(
                    sys::EVP_PKEY_CTX_new(param_key.p, ptr::null_mut()),
                    backend::pkey_ctx_free,
                );
                if ctx.is_null() {
                    break 'done;
                }
            }
            if sys::EVP_PKEY_check(ctx.p) != 0 || sys::EVP_PKEY_public_check_quick(ctx.p) != 0 {
                out.1 = has_d;
                if !param_key.is_null() {
                    sys::EVP_PKEY_up_ref(param_key.p);
                }
                out.0 = param_key.p;
            } else {
                out.1 = true;
                let mut k: *mut sys::EVP_PKEY = ptr::null_mut();
                if sys::EVP_PKEY_keygen_init(ctx.p) != 1
                    || sys::EVP_PKEY_CTX_set_params(ctx.p, params.p) != 1
                    || sys::EVP_PKEY_generate(ctx.p, &mut k) != 1
                {
                    break 'done;
                }
                out.0 = k;
            }
        }
    }
    store_errors();
    for b in bufs {
        unsafe { openssl_free(b) };
    }
    drop(bns);
    drop(keep);
    Ok(out)
}

/// php_openssl_pkey_object_curve_25519_448.
fn curve_25519_448(host: &mut dyn Host, name: &CStr, data: &PhpArray) -> Result<Value, Thrown> {
    let bld = bld_new();
    let mut ret = Value::Bool(false);
    let mut pkey: *mut sys::EVP_PKEY = ptr::null_mut();
    let mut is_private = false;
    let str_of = |n: &str| match data.get(&key(n)).map(deref) {
        Some(Value::Str(s)) if !s.is_empty() => Some(s.as_bytes().to_vec()),
        _ => None,
    };
    unsafe {
        'done: {
            if bld.is_null() {
                break 'done;
            }
            let priv_key = str_of("priv_key");
            if let Some(k) = &priv_key {
                if sys::OSSL_PARAM_BLD_push_octet_string(
                    bld.p,
                    c"priv".as_ptr(),
                    k.as_ptr().cast(),
                    k.len(),
                ) == 0
                {
                    break 'done;
                }
            }
            let pub_key = str_of("pub_key");
            if let Some(k) = &pub_key {
                if sys::OSSL_PARAM_BLD_push_octet_string(
                    bld.p,
                    c"pub".as_ptr(),
                    k.as_ptr().cast(),
                    k.len(),
                ) == 0
                {
                    break 'done;
                }
            }
            let params = to_param(&bld);
            let ctx = pkey_ctx_from_name(name);
            if params.is_null() || ctx.is_null() {
                break 'done;
            }
            // php tests the zvals' presence, not their content.
            let given = |n: &str| data.get(&key(n)).is_some();
            if given("pub_key") || given("priv_key") {
                pkey = fromdata(&ctx, &params);
                if pkey.is_null() {
                    break 'done;
                }
                is_private = given("priv_key");
            } else {
                is_private = true;
                pkey = keygen(&ctx);
            }
        }
    }
    if !pkey.is_null() {
        ret = key_object(host, pkey, is_private)?;
    }
    store_errors();
    Ok(ret)
}

fn fn_pkey_new(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_pkey_new";
    let a = opt_array(args, 0);
    if let Some(a) = &a {
        let sub = |n: &str| match a.get(&key(n)).map(deref) {
            Some(Value::Array(d)) => Some(d),
            _ => None,
        };
        if let Some(d) = sub("rsa") {
            let k = init_rsa(&d);
            return if k.is_null() {
                Ok(Value::Bool(false))
            } else {
                key_object(host, k, true)
            };
        }
        let made = if let Some(d) = sub("dsa") {
            Some(init_dsa(&d))
        } else if let Some(d) = sub("dh") {
            Some(init_dh(&d))
        } else if let Some(d) = sub("ec") {
            Some(init_ec(host, f, &d)?)
        } else {
            None
        };
        if let Some((k, private)) = made {
            return if k.is_null() {
                Ok(Value::Bool(false))
            } else {
                key_object(host, k, private)
            };
        }
        for (n, name) in [
            ("x25519", c"X25519"),
            ("ed25519", c"ED25519"),
            ("x448", c"X448"),
            ("ed448", c"ED448"),
        ] {
            if let Some(d) = sub(n) {
                return curve_25519_448(host, name, &d);
            }
        }
    }
    let mut req = Req::new();
    if backend::parse_config(host, f, &mut req, a.as_ref())? {
        let k = backend::generate_private_key(host, f, &mut req)?;
        if !k.is_null() {
            let k = std::mem::replace(&mut req.priv_key, Own::null()).release();
            return key_object(host, k, true);
        }
    }
    Ok(Value::Bool(false))
}

/// The PEM `openssl_pkey_export*` writes.
fn export_pem(
    bio: *mut sys::BIO,
    k: *mut sys::EVP_PKEY,
    passphrase: Option<&[u8]>,
    req: &Req,
) -> bool {
    let cipher = match passphrase {
        Some(_) if req.priv_key_encrypt => {
            if req.priv_key_encrypt_cipher.is_null() {
                unsafe { sys::EVP_des_ede3_cbc() }
            } else {
                req.priv_key_encrypt_cipher
            }
        }
        _ => ptr::null(),
    };
    let (p, n) = match passphrase {
        Some(s) => (s.as_ptr(), s.len() as c_int),
        None => (ptr::null(), 0),
    };
    unsafe {
        sys::PEM_write_bio_PrivateKey(bio, k, cipher, p as *mut u8, n, None, ptr::null_mut()) != 0
    }
}

fn fn_export_to_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_pkey_export_to_file";
    let filename = path_arg(host, args, 1, f)?;
    let passphrase = opt_str_arg(host, args, 2, f)?;
    let opts = opt_array(args, 3);
    let k = pkey_from_zval(host, f, &arg(args, 0), false, passphrase.as_deref(), 1)?;
    if k.is_null() {
        super::warn_unless_thrown(host, f, "Cannot get key from parameter 1")?;
        return Ok(Value::Bool(false));
    }
    let Some(path) = check_path(host, f, &filename, 2, false, false, None)? else {
        return Ok(Value::Bool(false));
    };
    let mut req = Req::new();
    if backend::parse_config(host, f, &mut req, opts.as_ref())? {
        let bio = super::bio_file(&path, WRITE);
        if bio.is_null() {
            store_errors();
            return Ok(Value::Bool(false));
        }
        if export_pem(bio.p, k.p, passphrase.as_deref(), &req) {
            return Ok(Value::Bool(true));
        }
        store_errors();
    }
    Ok(Value::Bool(false))
}

fn fn_export(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_pkey_export";
    let passphrase = opt_str_arg(host, args, 2, f)?;
    let opts = opt_array(args, 3);
    let k = pkey_from_zval(host, f, &arg(args, 0), false, passphrase.as_deref(), 1)?;
    if k.is_null() {
        super::warn_unless_thrown(host, f, "Cannot get key from parameter 1")?;
        return Ok(Value::Bool(false));
    }
    let mut req = Req::new();
    if backend::parse_config(host, f, &mut req, opts.as_ref())? {
        let bio = mem_bio();
        if bio.is_null() {
            store_errors();
            return Ok(Value::Bool(false));
        }
        if export_pem(bio.p, k.p, passphrase.as_deref(), &req) {
            set_ref(args, 1, Value::str(bio_bytes(bio.p)));
            return Ok(Value::Bool(true));
        }
        store_errors();
    }
    Ok(Value::Bool(false))
}

fn fn_get_public(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_pkey_get_public";
    let k = pkey_from_zval(host, f, &arg(args, 0), true, None, 1)?;
    if k.is_null() {
        return Ok(Value::Bool(false));
    }
    key_object(host, k.release(), false)
}

fn fn_pkey_free(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    key_arg(host, args, 0, "openssl_pkey_free")?;
    Ok(Value::Null)
}

fn fn_free_key(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    key_arg(host, args, 0, "openssl_free_key")?;
    Ok(Value::Null)
}

fn fn_get_private(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_pkey_get_private";
    let passphrase = match args.get(1) {
        None => Some(Vec::new()),
        Some(_) => opt_str_arg(host, args, 1, f)?,
    };
    let k = pkey_from_zval(host, f, &arg(args, 0), false, passphrase.as_deref(), 1)?;
    if k.is_null() {
        return Ok(Value::Bool(false));
    }
    key_object(host, k.release(), true)
}

fn copy_bn_param(a: &mut PhpArray, k: *mut sys::EVP_PKEY, param: &CStr, name: &str) {
    let mut bn: *mut sys::BIGNUM = ptr::null_mut();
    if unsafe { sys::EVP_PKEY_get_bn_param(k, param.as_ptr(), &mut bn) } > 0 {
        add_bn(a, bn, name);
        unsafe { sys::BN_free(bn) };
    }
}

fn copy_octet_param(a: &mut PhpArray, k: *mut sys::EVP_PKEY, param: &CStr, name: &str) {
    let mut buf = [0u8; 64];
    let mut len = 0usize;
    if unsafe {
        sys::EVP_PKEY_get_octet_string_param(k, param.as_ptr(), buf.as_mut_ptr(), 64, &mut len)
    } > 0
    {
        a.insert(key(name), Value::str(&buf[..len]));
    }
}

/// php_openssl_pkey_get_details: the type-specific part; returns the
/// `OPENSSL_KEYTYPE_*` (-1 if none).
fn pkey_details(out: &mut PhpArray, k: *mut sys::EVP_PKEY) -> i64 {
    let mut base_id = 0;
    unsafe {
        if sys::EVP_PKEY_id(k) != sys::EVP_PKEY_KEYMGMT {
            base_id = sys::EVP_PKEY_get_base_id(k);
        } else {
            let tn = sys::EVP_PKEY_get0_type_name(k);
            if !tn.is_null() {
                let nid = sys::OBJ_txt2nid(tn);
                if nid != sys::NID_undef {
                    base_id = sys::EVP_PKEY_type(nid);
                }
            }
        }
    }
    let mut ary = PhpArray::new();
    let (ktype, name) = match base_id {
        sys::EVP_PKEY_RSA => {
            for (p, n) in [
                (c"n", "n"),
                (c"e", "e"),
                (c"d", "d"),
                (c"rsa-factor1", "p"),
                (c"rsa-factor2", "q"),
                (c"rsa-exponent1", "dmp1"),
                (c"rsa-exponent2", "dmq1"),
                (c"rsa-coefficient1", "iqmp"),
            ] {
                copy_bn_param(&mut ary, k, p, n);
            }
            (0, "rsa")
        }
        sys::EVP_PKEY_DSA => {
            for (p, n) in [
                (c"p", "p"),
                (c"q", "q"),
                (c"g", "g"),
                (c"priv", "priv_key"),
                (c"pub", "pub_key"),
            ] {
                copy_bn_param(&mut ary, k, p, n);
            }
            (1, "dsa")
        }
        sys::EVP_PKEY_DH => {
            for (p, n) in [
                (c"p", "p"),
                (c"g", "g"),
                (c"priv", "priv_key"),
                (c"pub", "pub_key"),
            ] {
                copy_bn_param(&mut ary, k, p, n);
            }
            (2, "dh")
        }
        sys::EVP_PKEY_EC => {
            let mut buf = [0 as c_char; 64];
            let mut len = 0usize;
            if unsafe {
                sys::EVP_PKEY_get_utf8_string_param(
                    k,
                    c"group".as_ptr(),
                    buf.as_mut_ptr(),
                    64,
                    &mut len,
                )
            } > 0
            {
                let curve: Vec<u8> = buf[..len].iter().map(|&c| c.to_ne_bytes()[0]).collect();
                ary.insert(key("curve_name"), Value::str(curve.clone()));
                let c = cstring(&curve);
                let nid = unsafe { sys::OBJ_sn2nid(c.as_ptr()) };
                if nid != sys::NID_undef {
                    let obj = unsafe { sys::OBJ_nid2obj(nid) };
                    if !obj.is_null() {
                        let mut oid = [0 as c_char; 80];
                        let n = unsafe { sys::OBJ_obj2txt(oid.as_mut_ptr(), 80, obj, 1) };
                        let n = (n.max(0) as usize).min(79);
                        let oid: Vec<u8> = oid[..n].iter().map(|&c| c.to_ne_bytes()[0]).collect();
                        ary.insert(key("curve_oid"), Value::str(oid));
                        unsafe { sys::ASN1_OBJECT_free(obj) };
                    }
                }
            }
            copy_bn_param(&mut ary, k, c"qx", "x");
            copy_bn_param(&mut ary, k, c"qy", "y");
            copy_bn_param(&mut ary, k, c"priv", "d");
            (3, "ec")
        }
        sys::EVP_PKEY_X25519 | sys::EVP_PKEY_ED25519 | sys::EVP_PKEY_X448 | sys::EVP_PKEY_ED448 => {
            copy_octet_param(&mut ary, k, c"priv", "priv_key");
            copy_octet_param(&mut ary, k, c"pub", "pub_key");
            match base_id {
                sys::EVP_PKEY_X25519 => (4, "x25519"),
                sys::EVP_PKEY_ED25519 => (5, "ed25519"),
                sys::EVP_PKEY_X448 => (6, "x448"),
                _ => (7, "ed448"),
            }
        }
        _ => return -1,
    };
    out.insert(key(name), Value::Array(ary));
    ktype
}

fn fn_get_details(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, k, _) = key_arg(host, args, 0, "openssl_pkey_get_details")?;
    let bio = mem_bio();
    if bio.is_null() || unsafe { sys::PEM_write_bio_PUBKEY(bio.p, k) } == 0 {
        store_errors();
        return Ok(Value::Bool(false));
    }
    let mut out = PhpArray::new();
    out.insert(
        key("bits"),
        Value::Int(unsafe { sys::EVP_PKEY_bits(k) } as i64),
    );
    // php takes the length as an unsigned int.
    out.insert(key("key"), Value::str(bio_bytes(bio.p)));
    let ktype = pkey_details(&mut out, k);
    out.insert(key("type"), Value::Int(ktype));
    Ok(Value::Array(out))
}

/// php_openssl_pkey_derive.
fn derive(k: *mut sys::EVP_PKEY, peer: *mut sys::EVP_PKEY, requested: usize) -> Option<Vec<u8>> {
    let mut size = requested;
    unsafe {
        let ctx = Own::new(
            sys::EVP_PKEY_CTX_new(k, ptr::null_mut()),
            backend::pkey_ctx_free,
        );
        if ctx.is_null() {
            return None;
        }
        if sys::EVP_PKEY_derive_init(ctx.p) <= 0
            || sys::EVP_PKEY_derive_set_peer(ctx.p, peer) <= 0
            || (size == 0 && sys::EVP_PKEY_derive(ctx.p, ptr::null_mut(), &mut size) <= 0)
        {
            store_errors();
            return None;
        }
        let mut out = vec![0u8; size];
        if sys::EVP_PKEY_derive(ctx.p, out.as_mut_ptr(), &mut size) <= 0 {
            store_errors();
            return None;
        }
        out.truncate(size);
        Some(out)
    }
}

fn fn_dh_compute_key(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_dh_compute_key";
    let pub_str = str_arg(host, args, 0, f)?;
    let (_, k, _) = key_arg(host, args, 1, f)?;
    if unsafe { sys::EVP_PKEY_get_base_id(k) } != sys::EVP_PKEY_DH {
        return Ok(Value::Bool(false));
    }
    // php_openssl_dh_compute_key
    unsafe {
        let peer = Own::new(sys::EVP_PKEY_new(), pkey_free);
        if peer.is_null()
            || sys::EVP_PKEY_copy_parameters(peer.p, k) <= 0
            || sys::EVP_PKEY_set1_encoded_public_key(peer.p, pub_str.as_ptr(), pub_str.len()) <= 0
        {
            store_errors();
            return Ok(Value::Bool(false));
        }
        Ok(match derive(k, peer.p, 0) {
            Some(s) => Value::str(s),
            None => Value::Bool(false),
        })
    }
}

fn fn_pkey_derive(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_pkey_derive";
    let key_len = int_arg(host, args, 2, f, 0)?;
    if args.len() == 3 {
        crate::deprecated(
            host,
            f,
            "the $key_length parameter is deprecated as it is either ignored or truncates the key",
        )?;
    }
    if key_len < 0 {
        return Err(value_error(f, 2, "must be greater than or equal to 0"));
    }
    let k = pkey_from_zval(host, f, &arg(args, 1), false, Some(b""), 2)?;
    if k.is_null() {
        return Ok(Value::Bool(false));
    }
    let peer = pkey_from_zval(host, f, &arg(args, 0), true, None, 1)?;
    if peer.is_null() {
        return Ok(Value::Bool(false));
    }
    Ok(match derive(k.p, peer.p, key_len as usize) {
        Some(s) => Value::str(s),
        None => Value::Bool(false),
    })
}

// ---------------------------------------------------------------------------
// RSA primitives
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Op {
    Sign,
    Decrypt,
    Encrypt,
    VerifyRecover,
}

/// The shared body of openssl_{private,public}_{encrypt,decrypt}.
fn rsa_op(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    op: Op,
    public: bool,
    bad_key: &str,
) -> Result<Value, Thrown> {
    let data = str_arg(host, args, 0, f)?;
    let padding = int_arg(host, args, 3, f, sys::RSA_PKCS1_PADDING as i64)?;
    let digest = match op {
        Op::Decrypt | Op::Encrypt => match args.get(4).map(deref) {
            None | Some(Value::Null) => None,
            Some(_) => Some(super::pkey::path_arg(host, args, 4, f)?),
        },
        _ => None,
    };
    let k = if public {
        pkey_from_zval(host, f, &arg(args, 2), true, None, 3)?
    } else {
        pkey_from_zval(host, f, &arg(args, 2), false, Some(b""), 3)?
    };
    if k.is_null() {
        super::warn_unless_thrown(host, f, bad_key)?;
        return Ok(Value::Bool(false));
    }
    let mut md: *const sys::EVP_MD = ptr::null();
    let r = rsa_op_run(
        host,
        f,
        k.p,
        op,
        padding as c_int,
        digest.as_deref(),
        &mut md,
        &data,
    );
    release_md(md);
    match r? {
        Some(out) => {
            set_ref(args, 1, Value::str(out));
            Ok(Value::Bool(true))
        }
        None => Ok(Value::Bool(false)),
    }
}

#[allow(clippy::too_many_arguments)]
fn rsa_op_run(
    host: &mut dyn Host,
    f: &str,
    k: *mut sys::EVP_PKEY,
    op: Op,
    padding: c_int,
    digest: Option<&[u8]>,
    md: &mut *const sys::EVP_MD,
    data: &[u8],
) -> Result<Option<Vec<u8>>, Thrown> {
    unsafe {
        let ctx = Own::new(
            sys::EVP_PKEY_CTX_new(k, ptr::null_mut()),
            backend::pkey_ctx_free,
        );
        let run = |out: *mut u8, len: &mut usize| match op {
            Op::Sign => sys::EVP_PKEY_sign(ctx.p, out, len, data.as_ptr(), data.len()),
            Op::Decrypt => sys::EVP_PKEY_decrypt(ctx.p, out, len, data.as_ptr(), data.len()),
            Op::Encrypt => sys::EVP_PKEY_encrypt(ctx.p, out, len, data.as_ptr(), data.len()),
            Op::VerifyRecover => {
                sys::EVP_PKEY_verify_recover(ctx.p, out, len, data.as_ptr(), data.len())
            }
        };
        let mut ok = !ctx.is_null()
            && match op {
                Op::Sign => sys::EVP_PKEY_sign_init(ctx.p),
                Op::Decrypt => sys::EVP_PKEY_decrypt_init(ctx.p),
                Op::Encrypt => sys::EVP_PKEY_encrypt_init(ctx.p),
                Op::VerifyRecover => sys::EVP_PKEY_verify_recover_init(ctx.p),
            } > 0;
        if ok {
            ok = match op {
                Op::Decrypt | Op::Encrypt => {
                    set_rsa_padding_and_digest(host, f, ctx.p, padding, digest, md)?
                }
                _ => sys::EVP_PKEY_CTX_set_rsa_padding(ctx.p, padding) > 0,
            };
        }
        let mut len = 0usize;
        if !ok || run(ptr::null_mut(), &mut len) <= 0 {
            store_errors();
            return Ok(None);
        }
        let mut out = vec![0u8; len];
        if run(out.as_mut_ptr(), &mut len) <= 0 {
            store_errors();
            return Ok(None);
        }
        out.truncate(len);
        Ok(Some(out))
    }
}

/// php_openssl_set_rsa_padding_and_digest.
fn set_rsa_padding_and_digest(
    host: &mut dyn Host,
    f: &str,
    ctx: *mut sys::EVP_PKEY_CTX,
    padding: c_int,
    digest: Option<&[u8]>,
    pmd: &mut *const sys::EVP_MD,
) -> Result<bool, Thrown> {
    if unsafe { sys::EVP_PKEY_CTX_set_rsa_padding(ctx, padding) } <= 0 {
        return Ok(false);
    }
    if let Some(d) = digest {
        let md = md_by_name(&cstring(d));
        if md.is_null() {
            warn(
                host,
                f,
                &format!("Unknown digest algorithm: {}", String::from_utf8_lossy(d)),
            )?;
            return Ok(false);
        }
        *pmd = md;
        if padding == sys::RSA_PKCS1_OAEP_PADDING
            && unsafe { sys::EVP_PKEY_CTX_set_rsa_oaep_md(ctx, md as *mut sys::EVP_MD) } <= 0
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn fn_private_encrypt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    rsa_op(
        host,
        args,
        "openssl_private_encrypt",
        Op::Sign,
        false,
        "key param is not a valid private key",
    )
}

fn fn_private_decrypt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    rsa_op(
        host,
        args,
        "openssl_private_decrypt",
        Op::Decrypt,
        false,
        "key parameter is not a valid private key",
    )
}

fn fn_public_encrypt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    rsa_op(
        host,
        args,
        "openssl_public_encrypt",
        Op::Encrypt,
        true,
        "key parameter is not a valid public key",
    )
}

fn fn_public_decrypt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    rsa_op(
        host,
        args,
        "openssl_public_decrypt",
        Op::VerifyRecover,
        true,
        "key parameter is not a valid public key",
    )
}

// ---------------------------------------------------------------------------
// signatures
// ---------------------------------------------------------------------------

/// php_openssl_setup_rsa_padding.
fn setup_rsa_padding(
    host: &mut dyn Host,
    f: &str,
    pctx: *mut sys::EVP_PKEY_CTX,
    k: *mut sys::EVP_PKEY,
    padding: i64,
) -> Result<bool, Thrown> {
    let key_type = unsafe { sys::EVP_PKEY_type(sys::EVP_PKEY_id(k)) };
    if padding != 0 {
        if key_type != sys::EVP_PKEY_RSA {
            warn(host, f, "Padding parameter is only supported for RSA keys")?;
            return Ok(false);
        }
        if padding == sys::RSA_PKCS1_PSS_PADDING as i64 {
            if unsafe { sys::EVP_PKEY_CTX_set_rsa_padding(pctx, sys::RSA_PKCS1_PSS_PADDING) } <= 0 {
                store_errors();
                return Ok(false);
            }
        } else if padding != sys::RSA_PKCS1_PADDING as i64 {
            warn(host, f, "Unknown padding type")?;
            return Ok(false);
        }
    }
    Ok(true)
}

/// `Z_PARAM_STR_OR_LONG(method_str, method_long)` then the digest it names:
/// `(md, method_long)`.
fn digest_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
) -> Result<(*const sys::EVP_MD, i64), Thrown> {
    match args.get(i).map(deref) {
        None => Ok((md_from_algo(1), 1)),
        Some(Value::Str(s)) => Ok((md_by_name(&cstring(s.as_bytes())), 1)),
        Some(Value::Int(n)) => Ok((md_from_algo(n), n)),
        Some(Value::Null) => {
            let (pname, _) = super::param(f, i);
            host.emit_diagnostic(
                crate::E_DEPRECATED,
                format!(
                    "{f}(): Passing null to parameter #{} (${pname}) of type string|int is deprecated",
                    i + 1
                ).as_bytes(),
            )?;
            Ok((md_from_algo(0), 0))
        }
        Some(v @ (Value::Float(_) | Value::Bool(_))) => {
            // zpp: a float or bool becomes an int when it is integral, else a string
            match v {
                Value::Bool(b) => Ok((md_from_algo(b as i64), b as i64)),
                Value::Float(x) if x.fract() == 0.0 && x.is_finite() => {
                    Ok((md_from_algo(x as i64), x as i64))
                }
                _ => {
                    let s = rhp_values::conv::to_string_bytes(&v);
                    Ok((md_by_name(&cstring(&s)), 1))
                }
            }
        }
        Some(v) => {
            let s = super::try_string(host, &v);
            match (&v, s) {
                (Value::Object(_), Ok(s)) => Ok((md_by_name(&cstring(&s)), 1)),
                _ => Err(type_error(host, f, i, &v)),
            }
        }
    }
}

fn fn_sign(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_sign";
    let data = str_arg(host, args, 0, f)?;
    let k = pkey_from_zval(host, f, &arg(args, 2), false, Some(b""), 3)?;
    if k.is_null() {
        super::warn_unless_thrown(
            host,
            f,
            "Supplied key param cannot be coerced into a private key",
        )?;
        return Ok(Value::Bool(false));
    }
    let (md, method_long) = digest_arg(host, args, 3, f)?;
    let padding = int_arg(host, args, 4, f, 0)?;
    if md.is_null() && method_long != 0 {
        warn(host, f, "Unknown digest algorithm")?;
        return Ok(Value::Bool(false));
    }
    let r = (|| -> Result<Option<Vec<u8>>, Thrown> {
        unsafe {
            let ctx = sys::EVP_MD_CTX_new();
            if ctx.is_null() {
                return Ok(None);
            }
            let mut pctx: *mut sys::EVP_PKEY_CTX = ptr::null_mut();
            let mut siglen = 0usize;
            let mut out = None;
            if sys::EVP_DigestSignInit(ctx, &mut pctx, md, ptr::null_mut(), k.p) != 0
                && setup_rsa_padding(host, f, pctx, k.p, padding)
                    .inspect_err(|_| sys::EVP_MD_CTX_free(ctx))?
                && sys::EVP_DigestSign(ctx, ptr::null_mut(), &mut siglen, data.as_ptr(), data.len())
                    != 0
            {
                let mut sig = vec![0u8; siglen];
                if sys::EVP_DigestSign(
                    ctx,
                    sig.as_mut_ptr(),
                    &mut siglen,
                    data.as_ptr(),
                    data.len(),
                ) != 0
                {
                    sig.truncate(siglen);
                    out = Some(sig);
                }
            }
            sys::EVP_MD_CTX_free(ctx);
            Ok(out)
        }
    })();
    release_md(md);
    match r? {
        Some(sig) => {
            set_ref(args, 1, Value::str(sig));
            Ok(Value::Bool(true))
        }
        None => {
            store_errors();
            Ok(Value::Bool(false))
        }
    }
}

fn fn_verify(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_verify";
    let data = str_arg(host, args, 0, f)?;
    let sig = str_arg(host, args, 1, f)?;
    let (md, method_long) = digest_arg(host, args, 3, f)?;
    let padding = int_arg(host, args, 4, f, 0);
    let padding = match padding {
        Ok(p) => p,
        Err(e) => {
            release_md(md);
            return Err(e);
        }
    };
    if md.is_null() && method_long != 0 {
        warn(host, f, "Unknown digest algorithm")?;
        return Ok(Value::Bool(false));
    }
    let k = pkey_from_zval(host, f, &arg(args, 2), true, None, 3);
    let k = match k {
        Ok(k) => k,
        Err(e) => {
            release_md(md);
            return Err(e);
        }
    };
    if k.is_null() {
        let r = super::warn_unless_thrown(
            host,
            f,
            "Supplied key param cannot be coerced into a public key",
        );
        release_md(md);
        r?;
        return Ok(Value::Bool(false));
    }
    let r = (|| -> Result<c_int, Thrown> {
        unsafe {
            let ctx = sys::EVP_MD_CTX_new();
            if ctx.is_null() {
                store_errors();
                return Ok(-1);
            }
            let mut pctx: *mut sys::EVP_PKEY_CTX = ptr::null_mut();
            let init = sys::EVP_DigestVerifyInit(ctx, &mut pctx, md, ptr::null_mut(), k.p) != 0;
            let padded = init
                && setup_rsa_padding(host, f, pctx, k.p, padding)
                    .inspect_err(|_| sys::EVP_MD_CTX_free(ctx))?;
            let err = if !padded {
                store_errors();
                -1
            } else {
                let e =
                    sys::EVP_DigestVerify(ctx, sig.as_ptr(), sig.len(), data.as_ptr(), data.len());
                if e < 0 {
                    store_errors();
                }
                e
            };
            sys::EVP_MD_CTX_free(ctx);
            Ok(err)
        }
    })();
    release_md(md);
    Ok(Value::Int(r? as i64))
}

/// phar's OpenSSL signature check (phar_verify_signature): `None` when
/// the public key cannot be read, else whether `sig` verifies over `data`
/// with `digest`.
pub(crate) fn verify_with_pem(pem: &[u8], digest: &str, data: &[u8], sig: &[u8]) -> Option<bool> {
    super::init();
    let bio = mem_buf(pem);
    if bio.is_null() {
        return None;
    }
    let k = Own::new(
        unsafe { sys::PEM_read_bio_PUBKEY(bio.p, ptr::null_mut(), None, ptr::null_mut()) },
        pkey_free,
    );
    if k.is_null() {
        return None;
    }
    // EVP_VerifyInit / EVP_VerifyUpdate / EVP_VerifyFinal
    let md = md_by_name(&cstring(digest.as_bytes()));
    let ok = unsafe {
        let ctx = sys::EVP_MD_CTX_new();
        let ok = !ctx.is_null()
            && sys::EVP_DigestInit_ex(ctx, md, ptr::null_mut()) != 0
            && sys::EVP_DigestUpdate(ctx, data.as_ptr().cast(), data.len()) != 0
            && sys::EVP_VerifyFinal(ctx, sig.as_ptr(), sig.len() as u32, k.p) == 1;
        sys::EVP_MD_CTX_free(ctx);
        ok
    };
    release_md(md);
    Some(ok)
}

// ---------------------------------------------------------------------------
// seal / open
// ---------------------------------------------------------------------------

fn fn_seal(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_seal";
    let data = str_arg(host, args, 0, f)?;
    let pubkeys = match arg(args, 3) {
        Value::Array(a) => a,
        v => return Err(type_error(host, f, 3, &v)),
    };
    let method = str_arg(host, args, 4, f)?;
    let has_iv = args.len() > 5;
    if pubkeys.is_empty() {
        return Err(value_error(f, 3, "must not be empty"));
    }
    let cipher = backend::cipher_by_name(&cstring(&method));
    if cipher.is_null() {
        warn(host, f, "Unknown cipher algorithm")?;
        return Ok(Value::Bool(false));
    }
    let r = seal(host, f, args, &data, &pubkeys, cipher, has_iv);
    release_cipher(cipher);
    r
}

fn seal(
    host: &mut dyn Host,
    f: &str,
    args: &[Value],
    data: &[u8],
    pubkeys: &PhpArray,
    cipher: *const sys::EVP_CIPHER,
    has_iv: bool,
) -> Result<Value, Thrown> {
    let iv_len = unsafe { sys::EVP_CIPHER_iv_length(cipher) };
    if !has_iv && iv_len > 0 {
        return Err(value_error(
            f,
            5,
            "cannot be null for the chosen cipher algorithm",
        ));
    }
    let mut keys: Vec<Own<sys::EVP_PKEY>> = Vec::new();
    for (i, (_, v)) in pubkeys.iter().enumerate() {
        let k = pkey_from_zval(host, f, v, true, None, 4)?;
        if k.is_null() {
            super::warn_unless_thrown(
                host,
                f,
                &format!("Not a public key ({}th member of pubkeys)", i + 1),
            )?;
            return Ok(Value::Bool(false));
        }
        keys.push(k);
    }
    unsafe {
        let ctx = sys::EVP_CIPHER_CTX_new();
        if ctx.is_null()
            || sys::EVP_EncryptInit_ex(ctx, cipher, ptr::null_mut(), ptr::null(), ptr::null()) == 0
        {
            sys::EVP_CIPHER_CTX_free(ctx);
            store_errors();
            return Ok(Value::Bool(false));
        }
        let block = sys::EVP_CIPHER_CTX_block_size(ctx);
        let mut buf = vec![0u8; data.len() + block.max(0) as usize];
        sys::EVP_CIPHER_CTX_reset(ctx);
        let mut eks: Vec<Vec<u8>> = keys
            .iter()
            .map(|k| vec![0u8; sys::EVP_PKEY_size(k.p).max(0) as usize + 1])
            .collect();
        let mut ek_ptrs: Vec<*mut u8> = eks.iter_mut().map(|e| e.as_mut_ptr()).collect();
        let mut eksl: Vec<c_int> = vec![0; keys.len()];
        let mut pkeys: Vec<*mut sys::EVP_PKEY> = keys.iter().map(|k| k.p).collect();
        let mut iv_buf = [0u8; sys::EVP_MAX_IV_LENGTH + 1];
        let (mut len1, mut len2) = (0, 0);
        if sys::EVP_SealInit(
            ctx,
            cipher,
            ek_ptrs.as_mut_ptr(),
            eksl.as_mut_ptr(),
            iv_buf.as_mut_ptr(),
            pkeys.as_mut_ptr(),
            keys.len() as c_int,
        ) <= 0
            || sys::EVP_EncryptUpdate(
                ctx,
                buf.as_mut_ptr(),
                &mut len1,
                data.as_ptr(),
                data.len() as c_int,
            ) == 0
            || sys::EVP_SealFinal(ctx, buf.as_mut_ptr().add(len1 as usize), &mut len2) == 0
        {
            sys::EVP_CIPHER_CTX_free(ctx);
            store_errors();
            return Ok(Value::Bool(false));
        }
        let total = len1 + len2;
        if total > 0 {
            buf.truncate(total as usize);
            set_ref(args, 1, Value::str(buf));
            let mut ekeys = PhpArray::new();
            for (e, l) in eks.iter().zip(&eksl) {
                ekeys.push(Value::str(&e[..*l as usize]));
            }
            set_ref(args, 2, Value::Array(ekeys));
            if has_iv {
                set_ref(args, 5, Value::str(&iv_buf[..iv_len.max(0) as usize]));
            }
        }
        sys::EVP_CIPHER_CTX_free(ctx);
        Ok(Value::Int(total as i64))
    }
}

fn fn_open(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_open";
    let data = str_arg(host, args, 0, f)?;
    let ekey = str_arg(host, args, 2, f)?;
    let method = str_arg(host, args, 4, f)?;
    let iv = opt_str_arg(host, args, 5, f)?;
    let k = pkey_from_zval(host, f, &arg(args, 3), false, Some(b""), 4)?;
    if k.is_null() {
        super::warn_unless_thrown(host, f, "Unable to coerce parameter 4 into a private key")?;
        return Ok(Value::Bool(false));
    }
    let cipher = backend::cipher_by_name(&cstring(&method));
    if cipher.is_null() {
        warn(host, f, "Unknown cipher algorithm")?;
        return Ok(Value::Bool(false));
    }
    let r = (|| -> Result<Value, Thrown> {
        let iv_len = unsafe { sys::EVP_CIPHER_iv_length(cipher) };
        let iv_ptr = if iv_len > 0 {
            let Some(iv) = &iv else {
                return Err(value_error(
                    f,
                    5,
                    "cannot be null for the chosen cipher algorithm",
                ));
            };
            if iv_len as usize != iv.len() {
                warn(host, f, "IV length is invalid")?;
                return Ok(Value::Bool(false));
            }
            iv.as_ptr()
        } else {
            ptr::null()
        };
        let mut buf = vec![0u8; data.len() + 1];
        unsafe {
            let ctx = sys::EVP_CIPHER_CTX_new();
            let (mut len1, mut len2) = (0, 0);
            let ok = !ctx.is_null()
                && sys::EVP_OpenInit(ctx, cipher, ekey.as_ptr(), ekey.len() as c_int, iv_ptr, k.p)
                    != 0
                && sys::EVP_DecryptUpdate(
                    ctx,
                    buf.as_mut_ptr(),
                    &mut len1,
                    data.as_ptr(),
                    data.len() as c_int,
                ) != 0
                && sys::EVP_OpenFinal(ctx, buf.as_mut_ptr().add(len1 as usize), &mut len2) != 0
                && len1 + len2 > 0;
            sys::EVP_CIPHER_CTX_free(ctx);
            if ok {
                buf.truncate((len1 + len2) as usize);
                set_ref(args, 1, Value::str(buf));
                Ok(Value::Bool(true))
            } else {
                store_errors();
                Ok(Value::Bool(false))
            }
        }
    })();
    release_cipher(cipher);
    r
}
