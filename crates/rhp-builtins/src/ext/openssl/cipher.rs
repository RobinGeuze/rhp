//! Symmetric ciphers, digests, randomness and SPKAC: openssl_encrypt/decrypt,
//! openssl_cipher_{iv,key}_length, openssl_get_{cipher,md}_methods,
//! openssl_get_curve_names, openssl_digest, openssl_pbkdf2,
//! openssl_random_pseudo_bytes, openssl_spki_* (php-src openssl.c and
//! openssl_backend_common.c).

use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use rhp_values::{PhpArray, Value};

use super::backend::{
    cipher_by_name, hex, md_by_name, md_from_algo, pkey_free, pkey_from_zval, release_cipher,
    release_md,
};
use super::{
    bio_bytes, bool_arg, cstr_bytes, cstring, int_arg, key_arg, libctx, mem_bio, openssl_free,
    set_ref, store_errors, str_arg, sys, value_error, Own, Put,
};
use crate::ext::hash::primitives::{base64_decode, base64_encode};
use crate::ext::zpp::deref;
use crate::{warn, Host, Thrown};

const OPENSSL_RAW_DATA: i64 = 1;
const OPENSSL_ZERO_PADDING: i64 = 2;
const OPENSSL_DONT_ZERO_PAD_KEY: i64 = 4;

pub(super) fn register(put: &mut Put) {
    put(
        "openssl_get_md_methods",
        super::wrapped!(fn_get_md_methods),
        0,
        1,
    );
    put(
        "openssl_get_cipher_methods",
        super::wrapped!(fn_get_cipher_methods),
        0,
        1,
    );
    put(
        "openssl_get_curve_names",
        super::wrapped!(fn_get_curve_names),
        0,
        0,
    );
    put("openssl_digest", super::wrapped!(fn_digest), 2, 3);
    put("openssl_encrypt", super::wrapped!(fn_encrypt), 3, 8);
    put("openssl_decrypt", super::wrapped!(fn_decrypt), 3, 7);
    put(
        "openssl_cipher_iv_length",
        super::wrapped!(fn_cipher_iv_length),
        1,
        1,
    );
    put(
        "openssl_cipher_key_length",
        super::wrapped!(fn_cipher_key_length),
        1,
        1,
    );
    put(
        "openssl_random_pseudo_bytes",
        super::wrapped!(fn_random_pseudo_bytes),
        1,
        2,
    );
    put("openssl_pbkdf2", super::wrapped!(fn_pbkdf2), 4, 5);
    put("openssl_spki_new", super::wrapped!(fn_spki_new), 2, 3);
    put("openssl_spki_verify", super::wrapped!(fn_spki_verify), 1, 1);
    put("openssl_spki_export", super::wrapped!(fn_spki_export), 1, 1);
    put(
        "openssl_spki_export_challenge",
        fn_spki_export_challenge,
        1,
        1,
    );
}

// ---------------------------------------------------------------------------
// method lists
// ---------------------------------------------------------------------------

unsafe extern "C" fn add_md_or_alias(name: *const sys::OBJ_NAME, arg: *mut c_void) {
    let out = unsafe { &mut *(arg as *mut Vec<Vec<u8>>) };
    out.push(cstr_bytes(unsafe { (*name).name }).to_vec());
}

unsafe extern "C" fn add_md(name: *const sys::OBJ_NAME, arg: *mut c_void) {
    if unsafe { (*name).alias } == 0 {
        unsafe { add_md_or_alias(name, arg) };
    }
}

fn fn_get_md_methods(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let aliases = bool_arg(host, args, 0, "openssl_get_md_methods", false)?;
    super::init();
    let mut names: Vec<Vec<u8>> = Vec::new();
    unsafe {
        sys::OBJ_NAME_do_all_sorted(
            sys::OBJ_NAME_TYPE_MD_METH,
            if aliases { add_md_or_alias } else { add_md },
            (&mut names as *mut Vec<Vec<u8>>).cast(),
        );
    }
    let mut a = PhpArray::new();
    for n in names {
        a.push(Value::str(n));
    }
    Ok(Value::Array(a))
}

unsafe extern "C" fn add_cipher_name(name: *const c_char, arg: *mut c_void) {
    let out = unsafe { &mut *(arg as *mut Vec<Vec<u8>>) };
    out.push(cstr_bytes(name).to_ascii_lowercase());
}

unsafe extern "C" fn add_cipher_or_alias(cipher: *mut sys::EVP_CIPHER, arg: *mut c_void) {
    unsafe { sys::EVP_CIPHER_names_do_all(cipher, add_cipher_name, arg) };
}

unsafe extern "C" fn add_cipher(cipher: *mut sys::EVP_CIPHER, arg: *mut c_void) {
    unsafe { add_cipher_name(sys::EVP_CIPHER_get0_name(cipher), arg) };
}

fn fn_get_cipher_methods(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let aliases = bool_arg(host, args, 0, "openssl_get_cipher_methods", false)?;
    let mut names: Vec<Vec<u8>> = Vec::new();
    unsafe {
        sys::EVP_CIPHER_do_all_provided(
            libctx(),
            if aliases {
                add_cipher_or_alias
            } else {
                add_cipher
            },
            (&mut names as *mut Vec<Vec<u8>>).cast(),
        );
    }
    // zend_hash_sort with string_compare_function (a binary comparison of
    // lowercase names); php's sort is stable with renumbering.
    names.sort();
    let mut a = PhpArray::new();
    for n in names {
        a.push(Value::str(n));
    }
    Ok(Value::Array(a))
}

#[repr(C)]
struct EcBuiltinCurve {
    nid: c_int,
    comment: *const c_char,
}

extern "C" {
    fn EC_get_builtin_curves(r: *mut EcBuiltinCurve, nitems: usize) -> usize;
}

fn fn_get_curve_names(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    super::init();
    let len = unsafe { EC_get_builtin_curves(ptr::null_mut(), 0) };
    let mut curves: Vec<EcBuiltinCurve> = (0..len)
        .map(|_| EcBuiltinCurve {
            nid: 0,
            comment: ptr::null(),
        })
        .collect();
    if unsafe { EC_get_builtin_curves(curves.as_mut_ptr(), len) } == 0 {
        return Ok(Value::Bool(false));
    }
    let mut a = PhpArray::new();
    for c in &curves {
        let sn = unsafe { sys::OBJ_nid2sn(c.nid) };
        if !sn.is_null() {
            a.push(Value::str(cstr_bytes(sn).to_vec()));
        }
    }
    Ok(Value::Array(a))
}

// ---------------------------------------------------------------------------
// digests
// ---------------------------------------------------------------------------

fn fn_digest(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_digest";
    let data = str_arg(host, args, 0, f)?;
    let method = str_arg(host, args, 1, f)?;
    let raw = bool_arg(host, args, 2, f, false)?;
    let md = md_by_name(&cstring(&method));
    if md.is_null() {
        warn(host, f, "Unknown digest algorithm")?;
        return Ok(Value::Bool(false));
    }
    let mut siglen = unsafe { sys::EVP_MD_size(md) }.max(0) as u32;
    let mut sig = vec![0u8; siglen as usize];
    let ok = unsafe {
        let ctx = sys::EVP_MD_CTX_new();
        let ok = !ctx.is_null()
            && sys::EVP_DigestInit(ctx, md) != 0
            && sys::EVP_DigestUpdate(ctx, data.as_ptr().cast(), data.len()) != 0
            && sys::EVP_DigestFinal(ctx, sig.as_mut_ptr(), &mut siglen) != 0;
        sys::EVP_MD_CTX_free(ctx);
        ok
    };
    release_md(md);
    if !ok {
        store_errors();
        return Ok(Value::Bool(false));
    }
    sig.truncate(siglen as usize);
    Ok(Value::str(if raw { sig } else { hex(&sig).into_bytes() }))
}

fn fn_pbkdf2(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_pbkdf2";
    let password = str_arg(host, args, 0, f)?;
    let salt = str_arg(host, args, 1, f)?;
    let key_length = int_arg(host, args, 2, f, 0)?;
    let iterations = int_arg(host, args, 3, f, 0)?;
    let method = match args.get(4) {
        Some(_) => str_arg(host, args, 4, f)?,
        None => Vec::new(),
    };
    if key_length > c_int::MAX as i64 || key_length < c_int::MIN as i64 {
        return Err(value_error(f, 2, "key is too long"));
    }
    if iterations > c_int::MAX as i64 || iterations < c_int::MIN as i64 {
        return Err(value_error(f, 3, "iterations is too long"));
    }
    if key_length <= 0 {
        return Err(value_error(f, 2, "must be greater than 0"));
    }
    let md = if method.is_empty() {
        md_by_name(c"SHA1")
    } else {
        md_by_name(&cstring(&method))
    };
    if md.is_null() {
        warn(host, f, "Unknown digest algorithm")?;
        return Ok(Value::Bool(false));
    }
    let mut out = vec![0u8; key_length as usize];
    let ok = unsafe {
        sys::PKCS5_PBKDF2_HMAC(
            password.as_ptr().cast(),
            password.len() as c_int,
            salt.as_ptr(),
            salt.len() as c_int,
            iterations as c_int,
            md,
            key_length as c_int,
            out.as_mut_ptr(),
        )
    } == 1;
    release_md(md);
    if ok {
        Ok(Value::str(out))
    } else {
        store_errors();
        Ok(Value::Bool(false))
    }
}

fn fn_random_pseudo_bytes(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_random_pseudo_bytes";
    let len = int_arg(host, args, 0, f, 0)?;
    let has_strong = args.len() > 1;
    let fail = |args: &[Value]| {
        if has_strong {
            set_ref(args, 1, Value::Bool(false));
        }
    };
    if len <= 0 {
        fail(args);
        return Err(value_error(f, 0, "must be greater than 0"));
    }
    if len > c_int::MAX as i64 {
        fail(args);
        return Err(value_error(
            f,
            0,
            &format!("must be less than or equal to {}", c_int::MAX),
        ));
    }
    super::init();
    let mut buf = vec![0u8; len as usize];
    if unsafe { sys::RAND_bytes(buf.as_mut_ptr(), len as c_int) } <= 0 {
        store_errors();
        fail(args);
        return Err(Thrown::error(
            "Exception",
            "Error reading from source device",
        ));
    }
    if has_strong {
        set_ref(args, 1, Value::Bool(true));
    }
    Ok(Value::str(buf))
}

// ---------------------------------------------------------------------------
// openssl_encrypt / openssl_decrypt
// ---------------------------------------------------------------------------

/// php_openssl_cipher_mode.
#[derive(Default)]
struct Mode {
    is_aead: bool,
    is_single_run_aead: bool,
    set_tag_length_always: bool,
    set_tag_length_when_encrypting: bool,
}

/// php_openssl_load_cipher_mode.
fn load_cipher_mode(cipher: *const sys::EVP_CIPHER) -> Mode {
    let mut m = Mode::default();
    let mode = unsafe { sys::EVP_CIPHER_get_mode(cipher) };
    match mode {
        sys::EVP_CIPH_GCM_MODE | sys::EVP_CIPH_CCM_MODE | sys::EVP_CIPH_OCB_MODE => {
            m.set_tag_length_always = mode == sys::EVP_CIPH_OCB_MODE;
            m.is_aead = true;
            m.set_tag_length_when_encrypting = mode == sys::EVP_CIPH_CCM_MODE;
            m.is_single_run_aead = mode == sys::EVP_CIPH_CCM_MODE;
        }
        _ => {
            if unsafe { sys::EVP_CIPHER_nid(cipher) } == sys::NID_chacha20_poly1305 {
                m.is_aead = true;
            }
        }
    }
    m
}

/// php_openssl_validate_iv: the IV the cipher gets.
fn validate_iv(
    host: &mut dyn Host,
    f: &str,
    iv: Vec<u8>,
    required: usize,
    ctx: *mut sys::EVP_CIPHER_CTX,
    mode: &Mode,
) -> Result<Option<Vec<u8>>, Thrown> {
    if mode.is_aead {
        if unsafe {
            sys::EVP_CIPHER_CTX_ctrl(
                ctx,
                sys::EVP_CTRL_AEAD_SET_IVLEN,
                iv.len() as c_int,
                ptr::null_mut(),
            )
        } <= 0
        {
            warn(host, f, "Setting of IV length for AEAD mode failed")?;
            return Ok(None);
        }
        return Ok(Some(iv));
    }
    if iv.len() == required {
        return Ok(Some(iv));
    }
    let mut new = vec![0u8; required];
    if iv.is_empty() {
        return Ok(Some(new));
    }
    if iv.len() < required {
        warn(
            host,
            f,
            &format!(
                "IV passed is only {} bytes long, cipher expects an IV of precisely {} bytes, padding with \\0",
                iv.len(),
                required
            ),
        )?;
        new[..iv.len()].copy_from_slice(&iv);
        return Ok(Some(new));
    }
    warn(
        host,
        f,
        &format!(
            "IV passed is {} bytes long which is longer than the {} expected by selected cipher, truncating",
            iv.len(),
            required
        ),
    )?;
    new.copy_from_slice(&iv[..required]);
    Ok(Some(new))
}

/// php_openssl_cipher_init: `Ok(false)` is FAILURE.
#[allow(clippy::too_many_arguments)]
fn cipher_init(
    host: &mut dyn Host,
    f: &str,
    cipher: *const sys::EVP_CIPHER,
    ctx: *mut sys::EVP_CIPHER_CTX,
    mode: &Mode,
    password: &[u8],
    iv: Vec<u8>,
    tag: Option<&[u8]>,
    tag_len: c_int,
    options: i64,
    enc: bool,
) -> Result<bool, Thrown> {
    let max_iv_len = unsafe { sys::EVP_CIPHER_iv_length(cipher) }.max(0) as usize;
    if enc && iv.is_empty() && max_iv_len > 0 && !mode.is_aead {
        warn(
            host,
            f,
            "Using an empty Initialization Vector (iv) is potentially insecure and not recommended",
        )?;
    }
    let enc_i = enc as c_int;
    unsafe {
        if sys::EVP_CipherInit_ex(
            ctx,
            cipher,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            enc_i,
        ) == 0
        {
            store_errors();
            return Ok(false);
        }
    }
    let Some(iv) = validate_iv(host, f, iv, max_iv_len, ctx, mode)? else {
        return Ok(false);
    };
    unsafe {
        if (mode.set_tag_length_always || (enc && mode.set_tag_length_when_encrypting))
            && sys::EVP_CIPHER_CTX_ctrl(ctx, sys::EVP_CTRL_AEAD_SET_TAG, tag_len, ptr::null_mut())
                <= 0
        {
            warn(host, f, "Setting tag length for AEAD cipher failed")?;
            return Ok(false);
        }
        if let Some(t) = tag.filter(|_| !enc && tag_len > 0) {
            if !mode.is_aead {
                warn(
                    host,
                    f,
                    "The tag cannot be used because the cipher algorithm does not support AEAD",
                )?;
            } else if sys::EVP_CIPHER_CTX_ctrl(
                ctx,
                sys::EVP_CTRL_AEAD_SET_TAG,
                tag_len,
                t.as_ptr() as *mut c_void,
            ) <= 0
            {
                warn(host, f, "Setting tag for AEAD cipher decryption failed")?;
                return Ok(false);
            }
        }
    }
    // check and set key
    let password_len = password.len() as c_int;
    let key_len = unsafe { sys::EVP_CIPHER_key_length(cipher) };
    let key: Vec<u8> = if key_len > password_len {
        if options & OPENSSL_DONT_ZERO_PAD_KEY != 0
            && unsafe { sys::EVP_CIPHER_CTX_set_key_length(ctx, password_len) } == 0
        {
            store_errors();
            warn(host, f, "Key length cannot be set for the cipher algorithm")?;
            return Ok(false);
        }
        let mut k = vec![0u8; key_len as usize];
        k[..password.len()].copy_from_slice(password);
        k
    } else {
        if password_len > key_len
            && unsafe { sys::EVP_CIPHER_CTX_set_key_length(ctx, password_len) } == 0
        {
            store_errors();
        }
        password.to_vec()
    };
    unsafe {
        if sys::EVP_CipherInit_ex(
            ctx,
            ptr::null(),
            ptr::null_mut(),
            key.as_ptr(),
            if iv.is_empty() && max_iv_len == 0 && !mode.is_aead {
                ptr::null()
            } else {
                iv.as_ptr()
            },
            enc_i,
        ) == 0
        {
            store_errors();
            return Ok(false);
        }
        if options & OPENSSL_ZERO_PADDING != 0 {
            sys::EVP_CIPHER_CTX_set_padding(ctx, 0);
        }
    }
    Ok(true)
}

/// php_openssl_cipher_update: the output buffer and the bytes written.
#[allow(clippy::too_many_arguments)]
fn cipher_update(
    host: &mut dyn Host,
    f: &str,
    cipher: *const sys::EVP_CIPHER,
    ctx: *mut sys::EVP_CIPHER_CTX,
    mode: &Mode,
    data: &[u8],
    aad: &[u8],
) -> Result<Option<(Vec<u8>, usize)>, Thrown> {
    let mut i: c_int = 0;
    let block = unsafe { sys::EVP_CIPHER_block_size(cipher) }.max(0) as usize;
    let mut outlen = data.len() + block;
    unsafe {
        if mode.is_single_run_aead
            && sys::EVP_CipherUpdate(
                ctx,
                ptr::null_mut(),
                &mut i,
                ptr::null(),
                data.len() as c_int,
            ) == 0
        {
            store_errors();
            warn(host, f, "Setting of data length failed")?;
            return Ok(None);
        }
        if mode.is_aead
            && sys::EVP_CipherUpdate(
                ctx,
                ptr::null_mut(),
                &mut i,
                aad.as_ptr(),
                aad.len() as c_int,
            ) == 0
        {
            store_errors();
            warn(host, f, "Setting of additional application data failed")?;
            return Ok(None);
        }
        if sys::EVP_CIPHER_get_mode(cipher) == sys::EVP_CIPH_WRAP_MODE {
            outlen += block;
        }
        // one more byte: php's zend_string carries a NUL after the data
        let mut out = vec![0u8; outlen + 1];
        if sys::EVP_CipherUpdate(
            ctx,
            out.as_mut_ptr(),
            &mut i,
            data.as_ptr(),
            data.len() as c_int,
        ) == 0
        {
            store_errors();
            return Ok(None);
        }
        Ok(Some((out, i.max(0) as usize)))
    }
}

struct CipherCtx(*mut sys::EVP_CIPHER_CTX, *const sys::EVP_CIPHER);

impl Drop for CipherCtx {
    fn drop(&mut self) {
        unsafe {
            sys::EVP_CIPHER_CTX_reset(self.0);
            sys::EVP_CIPHER_CTX_free(self.0);
        }
        release_cipher(self.1);
    }
}

/// The cipher and a fresh context, or the warning php gives.
fn cipher_ctx(host: &mut dyn Host, f: &str, method: &[u8]) -> Result<Option<CipherCtx>, Thrown> {
    let cipher = cipher_by_name(&cstring(method));
    if cipher.is_null() {
        warn(host, f, "Unknown cipher algorithm")?;
        return Ok(None);
    }
    let ctx = unsafe { sys::EVP_CIPHER_CTX_new() };
    if ctx.is_null() {
        release_cipher(cipher);
        warn(host, f, "Failed to create cipher context")?;
        return Ok(None);
    }
    Ok(Some(CipherCtx(ctx, cipher)))
}

fn fn_encrypt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_encrypt";
    let data = str_arg(host, args, 0, f)?;
    let method = str_arg(host, args, 1, f)?;
    let password = str_arg(host, args, 2, f)?;
    let options = int_arg(host, args, 3, f, 0)?;
    let iv = match args.get(4) {
        Some(_) => str_arg(host, args, 4, f)?,
        None => Vec::new(),
    };
    let has_tag = args.len() > 5;
    let aad = match args.get(6) {
        Some(_) => str_arg(host, args, 6, f)?,
        None => Vec::new(),
    };
    let tag_len = int_arg(host, args, 7, f, 16)?;
    if tag_len > c_int::MAX as i64 || tag_len < c_int::MIN as i64 {
        return Err(Thrown::error("ValueError", "tag_len is too long"));
    }
    let Some(cc) = cipher_ctx(host, f, &method)? else {
        return Ok(Value::Bool(false));
    };
    let mode = load_cipher_mode(cc.1);
    if !cipher_init(
        host,
        f,
        cc.1,
        cc.0,
        &mode,
        &password,
        iv,
        None,
        tag_len as c_int,
        options,
        true,
    )? {
        return Ok(Value::Bool(false));
    }
    let Some((mut out, mut outlen)) = cipher_update(host, f, cc.1, cc.0, &mode, &data, &aad)?
    else {
        return Ok(Value::Bool(false));
    };
    let mut i: c_int = 0;
    if unsafe { sys::EVP_EncryptFinal_ex(cc.0, out.as_mut_ptr().add(outlen), &mut i) } == 0 {
        store_errors();
        return Ok(Value::Bool(false));
    }
    outlen += i.max(0) as usize;
    out.truncate(outlen);
    let result = if options & OPENSSL_RAW_DATA != 0 {
        out
    } else {
        base64_encode(&out)
    };
    if mode.is_aead && has_tag {
        let mut tag = vec![0u8; tag_len.max(0) as usize];
        if unsafe {
            sys::EVP_CIPHER_CTX_ctrl(
                cc.0,
                sys::EVP_CTRL_AEAD_GET_TAG,
                tag_len as c_int,
                tag.as_mut_ptr().cast(),
            )
        } > 0
        {
            set_ref(args, 5, Value::str(tag));
        } else {
            warn(host, f, "Retrieving verification tag failed")?;
            return Ok(Value::Bool(false));
        }
    } else if has_tag {
        set_ref(args, 5, Value::Null);
    } else if mode.is_aead {
        warn(host, f, "A tag should be provided when using AEAD mode")?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::str(result))
}

fn fn_decrypt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_decrypt";
    let data = str_arg(host, args, 0, f)?;
    let method = str_arg(host, args, 1, f)?;
    let password = str_arg(host, args, 2, f)?;
    let options = int_arg(host, args, 3, f, 0)?;
    let iv = match args.get(4) {
        Some(_) => str_arg(host, args, 4, f)?,
        None => Vec::new(),
    };
    let tag = match args.get(5).map(deref) {
        None | Some(Value::Null) => None,
        Some(_) => Some(str_arg(host, args, 5, f)?),
    };
    let aad = match args.get(6) {
        Some(_) => str_arg(host, args, 6, f)?,
        None => Vec::new(),
    };
    if method.is_empty() {
        return Err(value_error(f, 1, "must not be empty"));
    }
    let Some(cc) = cipher_ctx(host, f, &method)? else {
        return Ok(Value::Bool(false));
    };
    let mode = load_cipher_mode(cc.1);
    let data = if options & OPENSSL_RAW_DATA == 0 {
        match base64_decode(&data, false) {
            Some(d) => d,
            None => {
                warn(host, f, "Failed to base64 decode the input")?;
                return Ok(Value::Bool(false));
            }
        }
    } else {
        data
    };
    let tag_len = tag.as_ref().map_or(0, |t| t.len()) as c_int;
    if !cipher_init(
        host,
        f,
        cc.1,
        cc.0,
        &mode,
        &password,
        iv,
        tag.as_deref(),
        tag_len,
        options,
        false,
    )? {
        return Ok(Value::Bool(false));
    }
    let Some((mut out, mut outlen)) = cipher_update(host, f, cc.1, cc.0, &mode, &data, &aad)?
    else {
        return Ok(Value::Bool(false));
    };
    let mut i: c_int = 0;
    if mode.is_single_run_aead
        || unsafe { sys::EVP_DecryptFinal_ex(cc.0, out.as_mut_ptr().add(outlen), &mut i) } != 0
    {
        outlen += i.max(0) as usize;
        out.truncate(outlen);
        Ok(Value::str(out))
    } else {
        store_errors();
        Ok(Value::Bool(false))
    }
}

/// php_openssl_cipher_{iv,key}_length.
fn cipher_length(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    get: unsafe extern "C" fn(*const sys::EVP_CIPHER) -> c_int,
) -> Result<Value, Thrown> {
    let method = str_arg(host, args, 0, f)?;
    if method.is_empty() {
        return Err(value_error(f, 0, "must not be empty"));
    }
    let cipher = cipher_by_name(&cstring(&method));
    if cipher.is_null() {
        warn(host, f, "Unknown cipher algorithm")?;
        return Ok(Value::Bool(false));
    }
    let n = unsafe { get(cipher) };
    release_cipher(cipher);
    if n == -1 {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Int(n as i64))
}

fn fn_cipher_iv_length(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    cipher_length(
        host,
        args,
        "openssl_cipher_iv_length",
        sys::EVP_CIPHER_get_iv_length,
    )
}

fn fn_cipher_key_length(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    cipher_length(
        host,
        args,
        "openssl_cipher_key_length",
        sys::EVP_CIPHER_get_key_length,
    )
}

// ---------------------------------------------------------------------------
// SPKAC
// ---------------------------------------------------------------------------

unsafe extern "C" fn spki_free(s: *mut sys::NETSCAPE_SPKI) {
    unsafe { sys::NETSCAPE_SPKI_free(s) };
}

/// php_openssl_spki_cleanup: drop CR and LF.
fn spki_cleanup(s: &[u8]) -> Vec<u8> {
    // php works on the C string: it stops at the first NUL.
    s.iter()
        .take_while(|&&b| b != 0)
        .copied()
        .filter(|&b| b != b'\n' && b != b'\r')
        .collect()
}

fn fn_spki_new(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_spki_new";
    let (zkey, _, _) = key_arg(host, args, 0, f)?;
    let challenge = str_arg(host, args, 1, f)?;
    let algo = int_arg(host, args, 2, f, 2)?;
    let k = pkey_from_zval(host, f, &zkey, false, Some(&challenge), 1)?;
    if k.is_null() {
        super::warn_unless_thrown(host, f, "Unable to use supplied private key")?;
        return Ok(Value::Bool(false));
    }
    let md = md_from_algo(algo);
    let r = spki_new(host, f, k.p, md, &challenge);
    release_md(md);
    r
}

fn spki_new(
    host: &mut dyn Host,
    f: &str,
    k: *mut sys::EVP_PKEY,
    md: *const sys::EVP_MD,
    challenge: &[u8],
) -> Result<Value, Thrown> {
    if md.is_null() {
        warn(host, f, "Unknown digest algorithm")?;
        return Ok(Value::Bool(false));
    }
    let spki = Own::new(unsafe { sys::NETSCAPE_SPKI_new() }, spki_free);
    if spki.is_null() {
        store_errors();
        warn(host, f, "Unable to create new SPKAC")?;
        return Ok(Value::Bool(false));
    }
    unsafe {
        if !spki_set_challenge(spki.p, challenge) {
            store_errors();
            warn(host, f, "Unable to set challenge data")?;
            return Ok(Value::Bool(false));
        }
        if sys::NETSCAPE_SPKI_set_pubkey(spki.p, k) == 0 {
            store_errors();
            warn(host, f, "Unable to embed public key")?;
            return Ok(Value::Bool(false));
        }
        if sys::NETSCAPE_SPKI_sign(spki.p, k, md) == 0 {
            store_errors();
            warn(host, f, "Unable to sign with specified digest algorithm")?;
            return Ok(Value::Bool(false));
        }
        let s = sys::NETSCAPE_SPKI_b64_encode(spki.p);
        if s.is_null() {
            store_errors();
            warn(host, f, "Unable to encode SPKAC")?;
            return Ok(Value::Bool(false));
        }
        let mut out = b"SPKAC=".to_vec();
        out.extend_from_slice(cstr_bytes(s));
        openssl_free(s);
        Ok(Value::str(out))
    }
}

/// `ASN1_STRING_set(spki->spkac->challenge, …)`: NETSCAPE_SPKI is
/// `{ NETSCAPE_SPKAC *spkac; … }` and NETSCAPE_SPKAC is
/// `{ X509_PUBKEY *pubkey; ASN1_IA5STRING *challenge; }`.
unsafe fn spki_challenge(spki: *mut sys::NETSCAPE_SPKI) -> *mut sys::ASN1_STRING {
    unsafe {
        let spkac = *(spki as *mut *mut *mut c_void);
        *(spkac.add(1) as *mut *mut sys::ASN1_STRING)
    }
}

unsafe fn spki_set_challenge(spki: *mut sys::NETSCAPE_SPKI, challenge: &[u8]) -> bool {
    unsafe {
        sys::ASN1_STRING_set(
            spki_challenge(spki),
            challenge.as_ptr().cast(),
            challenge.len() as c_int,
        ) != 0
    }
}

/// Decode an SPKAC string, with php's warnings.
fn spki_decode(
    host: &mut dyn Host,
    f: &str,
    s: &[u8],
    decode_msg: &str,
) -> Result<Option<Own<sys::NETSCAPE_SPKI>>, Thrown> {
    let cleaned = spki_cleanup(s);
    if cleaned.is_empty() {
        warn(host, f, "Invalid SPKAC")?;
        return Ok(None);
    }
    let spki = Own::new(
        unsafe { sys::NETSCAPE_SPKI_b64_decode(cleaned.as_ptr().cast(), cleaned.len() as c_int) },
        spki_free,
    );
    if spki.is_null() {
        store_errors();
        warn(host, f, decode_msg)?;
        return Ok(None);
    }
    Ok(Some(spki))
}

fn fn_spki_verify(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_spki_verify";
    let s = str_arg(host, args, 0, f)?;
    let mut i = 0;
    if let Some(spki) = spki_decode(host, f, &s, "Unable to decode supplied SPKAC")? {
        let k = Own::new(unsafe { sys::NETSCAPE_SPKI_get_pubkey(spki.p) }, pkey_free);
        if k.is_null() {
            store_errors();
            warn(host, f, "Unable to acquire signed public key")?;
        } else {
            i = unsafe { sys::NETSCAPE_SPKI_verify(spki.p, k.p) };
        }
    }
    if i > 0 {
        Ok(Value::Bool(true))
    } else {
        store_errors();
        Ok(Value::Bool(false))
    }
}

fn fn_spki_export(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_spki_export";
    let s = str_arg(host, args, 0, f)?;
    let Some(spki) = spki_decode(host, f, &s, "Unable to decode supplied SPKAC")? else {
        return Ok(Value::Bool(false));
    };
    let k = Own::new(unsafe { sys::NETSCAPE_SPKI_get_pubkey(spki.p) }, pkey_free);
    if k.is_null() {
        store_errors();
        warn(host, f, "Unable to acquire signed public key")?;
        return Ok(Value::Bool(false));
    }
    let bio = mem_bio();
    if !bio.is_null() && unsafe { sys::PEM_write_bio_PUBKEY(bio.p, k.p) } != 0 {
        return Ok(Value::str(bio_bytes(bio.p)));
    }
    store_errors();
    Ok(Value::Bool(false))
}

fn fn_spki_export_challenge(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "openssl_spki_export_challenge";
    let s = str_arg(host, args, 0, f)?;
    let Some(spki) = spki_decode(host, f, &s, "Unable to decode SPKAC")? else {
        return Ok(Value::Bool(false));
    };
    let ch = unsafe { sys::ASN1_STRING_get0_data(spki_challenge(spki.p)) };
    Ok(Value::str(cstr_bytes(ch.cast()).to_vec()))
}
