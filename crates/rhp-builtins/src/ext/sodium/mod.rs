//! php's `sodium` extension (php-src ext/sodium/libsodium.c) on libsodium,
//! linked statically. Constants and the optional parts php compiles in come
//! from sodium.h through shim.c. Password hashing (sodium_pwhash.c) is not
//! registered: php registers it only when the standard module has no
//! argon2, and rhp's has.

mod sys;

use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void};
use std::ptr::{null, null_mut};
use std::sync::OnceLock;

use rhp_values::ops::deref;
use rhp_values::{PhpArray, Value};

use crate::zend::zpp::{int_arg, set_ref, str_arg};
use crate::{Host, Thrown, E_WARNING};

use sys::{AeadDecrypt, AeadEncrypt};

pub const PRELUDE_PHP: &str = include_str!("prelude.php");

/// `ZSTR_MAX_LEN` on 64-bit.
const ZSTR_MAX_LEN: usize = usize::MAX - 32;

mod feat {
    pub const AESGCM: u32 = 1 << 0;
    pub const AEGIS: u32 = 1 << 1;
    pub const XCHACHA_AEAD: u32 = 1 << 3;
    pub const SECRETSTREAM: u32 = 1 << 4;
    pub const PWHASH: u32 = 1 << 5;
    pub const SCRYPT: u32 = 1 << 6;
    pub const XCHACHA_STREAM: u32 = 1 << 7;
    pub const BASE64: u32 = 1 << 8;
    pub const RISTRETTO: u32 = 1 << 9;
    pub const NEEDS_REHASH: u32 = 1 << 10;
    pub const ARGON2ID: u32 = 1 << 11;
    pub const KDF_PRIMITIVE: u32 = 1 << 12;
}

fn features() -> u32 {
    static F: OnceLock<u32> = OnceLock::new();
    *F.get_or_init(|| unsafe { sys::rhp_sodium_features() })
}

fn has(f: u32) -> bool {
    features() & f != 0
}

/// libsodium's constants by name: php's (`SODIUM_*`) and the internal
/// limits (`crypto_*`).
fn table() -> &'static HashMap<&'static str, i64> {
    static T: OnceLock<HashMap<&'static str, i64>> = OnceLock::new();
    T.get_or_init(|| {
        sys::consts()
            .into_iter()
            .chain(sys::internal())
            .filter_map(|(n, v)| v.map(|v| (n, v)))
            .collect()
    })
}

/// The value of constant `name` (0 if libsodium lacks it).
fn c(name: &str) -> usize {
    table().get(name).copied().unwrap_or(0) as usize
}

/// php's MINIT: sodium_init() once.
fn init() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        if unsafe { sys::sodium_init() } < 0 {
            panic!("sodium_init()");
        }
    });
}

pub fn constants() -> Vec<(&'static str, Value)> {
    let mut out = vec![
        ("SODIUM_LIBRARY_VERSION", Value::str(sys::library_version())),
        (
            "SODIUM_LIBRARY_MAJOR_VERSION",
            Value::Int(unsafe { sys::sodium_library_version_major() } as i64),
        ),
        (
            "SODIUM_LIBRARY_MINOR_VERSION",
            Value::Int(unsafe { sys::sodium_library_version_minor() } as i64),
        ),
    ];
    for (n, v) in sys::consts() {
        let v = match (n, v) {
            (_, Some(v)) => Value::Int(v),
            ("SODIUM_CRYPTO_PWHASH_STRPREFIX", None) => Value::str(sys::pwhash_strprefix()),
            (_, None) => Value::str(sys::scrypt_strprefix()),
        };
        out.push((n, v));
    }
    out
}

/// phpinfo's rows.
pub fn info_rows() -> Vec<(&'static str, String)> {
    let s = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    vec![
        ("sodium support", "enabled".to_string()),
        ("libsodium headers version", s(sys::header_version())),
        ("libsodium library version", s(sys::library_version())),
    ]
}

type F = fn(&mut dyn Host, &[Value]) -> Result<Value, Thrown>;

pub fn register(m: &mut crate::Registry) {
    init();
    let mut fns: Vec<(&'static str, F)> = Vec::new();
    let mut add = |on: bool, list: &[(&'static str, F)]| {
        if on {
            fns.extend_from_slice(list);
        }
    };
    add(
        true,
        &[(
            "sodium_crypto_aead_aes256gcm_is_available",
            fn_aes256gcm_is_available,
        )],
    );
    add(
        has(feat::AESGCM),
        &[
            ("sodium_crypto_aead_aes256gcm_decrypt", fn_aes256gcm_decrypt),
            ("sodium_crypto_aead_aes256gcm_encrypt", fn_aes256gcm_encrypt),
            ("sodium_crypto_aead_aes256gcm_keygen", fn_aes256gcm_keygen),
        ],
    );
    add(
        has(feat::AEGIS),
        &[
            ("sodium_crypto_aead_aegis128l_decrypt", fn_aegis128l_decrypt),
            ("sodium_crypto_aead_aegis128l_encrypt", fn_aegis128l_encrypt),
            ("sodium_crypto_aead_aegis128l_keygen", fn_aegis128l_keygen),
            ("sodium_crypto_aead_aegis256_decrypt", fn_aegis256_decrypt),
            ("sodium_crypto_aead_aegis256_encrypt", fn_aegis256_encrypt),
            ("sodium_crypto_aead_aegis256_keygen", fn_aegis256_keygen),
        ],
    );
    add(
        true,
        &[
            (
                "sodium_crypto_aead_chacha20poly1305_decrypt",
                fn_chacha_decrypt,
            ),
            (
                "sodium_crypto_aead_chacha20poly1305_encrypt",
                fn_chacha_encrypt,
            ),
            (
                "sodium_crypto_aead_chacha20poly1305_keygen",
                fn_chacha_keygen,
            ),
            (
                "sodium_crypto_aead_chacha20poly1305_ietf_decrypt",
                fn_ietf_decrypt,
            ),
            (
                "sodium_crypto_aead_chacha20poly1305_ietf_encrypt",
                fn_ietf_encrypt,
            ),
            (
                "sodium_crypto_aead_chacha20poly1305_ietf_keygen",
                fn_ietf_keygen,
            ),
        ],
    );
    add(
        has(feat::XCHACHA_AEAD),
        &[
            (
                "sodium_crypto_aead_xchacha20poly1305_ietf_decrypt",
                fn_xchacha_decrypt,
            ),
            (
                "sodium_crypto_aead_xchacha20poly1305_ietf_keygen",
                fn_xchacha_keygen,
            ),
            (
                "sodium_crypto_aead_xchacha20poly1305_ietf_encrypt",
                fn_xchacha_encrypt,
            ),
        ],
    );
    add(
        true,
        &[
            ("sodium_crypto_auth", fn_crypto_auth),
            ("sodium_crypto_auth_keygen", fn_auth_keygen),
            ("sodium_crypto_auth_verify", fn_crypto_auth_verify),
            ("sodium_crypto_box", fn_box),
            ("sodium_crypto_box_keypair", fn_box_keypair),
            ("sodium_crypto_box_seed_keypair", fn_box_seed_keypair),
            (
                "sodium_crypto_box_keypair_from_secretkey_and_publickey",
                fn_box_keypair_from_sk_pk,
            ),
            ("sodium_crypto_box_open", fn_box_open),
            ("sodium_crypto_box_publickey", fn_box_publickey),
            (
                "sodium_crypto_box_publickey_from_secretkey",
                fn_box_publickey_from_secretkey,
            ),
            ("sodium_crypto_box_seal", fn_box_seal),
            ("sodium_crypto_box_seal_open", fn_box_seal_open),
            ("sodium_crypto_box_secretkey", fn_box_secretkey),
        ],
    );
    add(
        has(feat::RISTRETTO),
        &[
            ("sodium_crypto_core_ristretto255_add", fn_r255_add),
            (
                "sodium_crypto_core_ristretto255_from_hash",
                fn_r255_from_hash,
            ),
            (
                "sodium_crypto_core_ristretto255_is_valid_point",
                fn_r255_is_valid_point,
            ),
            ("sodium_crypto_core_ristretto255_random", fn_r255_random),
            (
                "sodium_crypto_core_ristretto255_scalar_add",
                fn_r255_scalar_add,
            ),
            (
                "sodium_crypto_core_ristretto255_scalar_complement",
                fn_r255_scalar_complement,
            ),
            (
                "sodium_crypto_core_ristretto255_scalar_invert",
                fn_r255_scalar_invert,
            ),
            (
                "sodium_crypto_core_ristretto255_scalar_mul",
                fn_r255_scalar_mul,
            ),
            (
                "sodium_crypto_core_ristretto255_scalar_negate",
                fn_r255_scalar_negate,
            ),
            (
                "sodium_crypto_core_ristretto255_scalar_random",
                fn_r255_scalar_random,
            ),
            (
                "sodium_crypto_core_ristretto255_scalar_reduce",
                fn_r255_scalar_reduce,
            ),
            (
                "sodium_crypto_core_ristretto255_scalar_sub",
                fn_r255_scalar_sub,
            ),
            ("sodium_crypto_core_ristretto255_sub", fn_r255_sub),
        ],
    );
    add(
        true,
        &[
            ("sodium_crypto_kx_keypair", fn_kx_keypair),
            ("sodium_crypto_kx_publickey", fn_kx_publickey),
            ("sodium_crypto_kx_secretkey", fn_kx_secretkey),
            ("sodium_crypto_kx_seed_keypair", fn_kx_seed_keypair),
            (
                "sodium_crypto_kx_client_session_keys",
                fn_kx_client_session_keys,
            ),
            (
                "sodium_crypto_kx_server_session_keys",
                fn_kx_server_session_keys,
            ),
            ("sodium_crypto_generichash", fn_generichash),
            ("sodium_crypto_generichash_keygen", fn_generichash_keygen),
            ("sodium_crypto_generichash_init", fn_generichash_init),
            ("sodium_crypto_generichash_update", fn_generichash_update),
            ("sodium_crypto_generichash_final", fn_generichash_final),
            ("sodium_crypto_kdf_derive_from_key", fn_kdf_derive_from_key),
            ("sodium_crypto_kdf_keygen", fn_kdf_keygen),
        ],
    );
    add(
        has(feat::PWHASH),
        &[
            ("sodium_crypto_pwhash", fn_pwhash),
            ("sodium_crypto_pwhash_str", fn_pwhash_str),
            ("sodium_crypto_pwhash_str_verify", fn_pwhash_str_verify),
        ],
    );
    add(
        has(feat::NEEDS_REHASH),
        &[(
            "sodium_crypto_pwhash_str_needs_rehash",
            fn_pwhash_str_needs_rehash,
        )],
    );
    add(
        has(feat::SCRYPT),
        &[
            ("sodium_crypto_pwhash_scryptsalsa208sha256", fn_scrypt),
            (
                "sodium_crypto_pwhash_scryptsalsa208sha256_str",
                fn_scrypt_str,
            ),
            (
                "sodium_crypto_pwhash_scryptsalsa208sha256_str_verify",
                fn_scrypt_str_verify,
            ),
        ],
    );
    add(true, &[("sodium_crypto_scalarmult", fn_scalarmult)]);
    add(
        has(feat::RISTRETTO),
        &[
            ("sodium_crypto_scalarmult_ristretto255", fn_scalarmult_r255),
            (
                "sodium_crypto_scalarmult_ristretto255_base",
                fn_scalarmult_r255_base,
            ),
        ],
    );
    add(
        true,
        &[
            ("sodium_crypto_secretbox", fn_secretbox),
            ("sodium_crypto_secretbox_keygen", fn_secretbox_keygen),
            ("sodium_crypto_secretbox_open", fn_secretbox_open),
        ],
    );
    add(
        has(feat::SECRETSTREAM),
        &[
            (
                "sodium_crypto_secretstream_xchacha20poly1305_keygen",
                fn_ss_keygen,
            ),
            (
                "sodium_crypto_secretstream_xchacha20poly1305_init_push",
                fn_ss_init_push,
            ),
            (
                "sodium_crypto_secretstream_xchacha20poly1305_push",
                fn_ss_push,
            ),
            (
                "sodium_crypto_secretstream_xchacha20poly1305_init_pull",
                fn_ss_init_pull,
            ),
            (
                "sodium_crypto_secretstream_xchacha20poly1305_pull",
                fn_ss_pull,
            ),
            (
                "sodium_crypto_secretstream_xchacha20poly1305_rekey",
                fn_ss_rekey,
            ),
        ],
    );
    add(
        true,
        &[
            ("sodium_crypto_shorthash", fn_shorthash),
            ("sodium_crypto_shorthash_keygen", fn_shorthash_keygen),
            ("sodium_crypto_sign", fn_sign),
            ("sodium_crypto_sign_detached", fn_sign_detached),
            (
                "sodium_crypto_sign_ed25519_pk_to_curve25519",
                fn_sign_pk_to_curve25519,
            ),
            (
                "sodium_crypto_sign_ed25519_sk_to_curve25519",
                fn_sign_sk_to_curve25519,
            ),
            ("sodium_crypto_sign_keypair", fn_sign_keypair),
            (
                "sodium_crypto_sign_keypair_from_secretkey_and_publickey",
                fn_sign_keypair_from_sk_pk,
            ),
            ("sodium_crypto_sign_open", fn_sign_open),
            ("sodium_crypto_sign_publickey", fn_sign_publickey),
            ("sodium_crypto_sign_secretkey", fn_sign_secretkey),
            (
                "sodium_crypto_sign_publickey_from_secretkey",
                fn_sign_publickey_from_secretkey,
            ),
            ("sodium_crypto_sign_seed_keypair", fn_sign_seed_keypair),
            (
                "sodium_crypto_sign_verify_detached",
                fn_sign_verify_detached,
            ),
            ("sodium_crypto_stream", fn_stream),
            ("sodium_crypto_stream_keygen", fn_stream_keygen),
            ("sodium_crypto_stream_xor", fn_stream_xor),
        ],
    );
    add(
        has(feat::XCHACHA_STREAM),
        &[
            ("sodium_crypto_stream_xchacha20", fn_stream_xchacha20),
            (
                "sodium_crypto_stream_xchacha20_keygen",
                fn_stream_xchacha20_keygen,
            ),
            (
                "sodium_crypto_stream_xchacha20_xor",
                fn_stream_xchacha20_xor,
            ),
            (
                "sodium_crypto_stream_xchacha20_xor_ic",
                fn_stream_xchacha20_xor_ic,
            ),
        ],
    );
    add(
        true,
        &[
            ("sodium_add", fn_add),
            ("sodium_compare", fn_compare),
            ("sodium_increment", fn_increment),
            ("sodium_memcmp", fn_memcmp),
            ("sodium_memzero", fn_memzero),
            ("sodium_pad", fn_pad),
            ("sodium_unpad", fn_unpad),
            ("sodium_bin2hex", fn_bin2hex),
            ("sodium_hex2bin", fn_hex2bin),
        ],
    );
    add(
        has(feat::BASE64),
        &[
            ("sodium_bin2base64", fn_bin2base64),
            ("sodium_base642bin", fn_base642bin),
        ],
    );
    add(
        true,
        &[(
            "sodium_crypto_scalarmult_base",
            fn_box_publickey_from_secretkey_alias,
        )],
    );
    crate::register(m, &fns);
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// `zend_throw_exception(sodium_exception_ce, msg, 0)`.
fn exc(msg: &str) -> Thrown {
    Thrown::error("SodiumException", msg)
}

/// The stub's name of parameter `n` (1-based) of `f`.
fn pname(f: &str, n: usize) -> &'static str {
    rhp_bytecode::signatures::function(f.as_bytes())
        .and_then(|s| s.params.get(n - 1))
        .map_or("", |p| p.name)
}

/// `zend_argument_error(sodium_exception_ce, n, msg)`.
fn arg_err(f: &str, n: usize, msg: &str) -> Thrown {
    exc(&format!("{f}(): Argument #{n} (${}) {msg}", pname(f, n)))
}

/// The SodiumException for an argument that is not `name` bytes long.
fn need(f: &str, n: usize, v: &[u8], name: &str) -> Result<(), Thrown> {
    if v.len() != c(name) {
        return Err(arg_err(f, n, &format!("must be {name} bytes long")));
    }
    Ok(())
}

/// String argument `i` (already coerced by the VM's zpp).
fn sarg(host: &mut dyn Host, args: &[Value], i: usize, f: &str) -> Result<Vec<u8>, Thrown> {
    match args.get(i).map(deref) {
        Some(Value::Str(s)) => Ok(s.as_bytes().to_vec()),
        _ => str_arg(host, args, i, f, pname(f, i + 1)),
    }
}

/// An optional string argument.
fn sarg_or(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
    default: &[u8],
) -> Result<Vec<u8>, Thrown> {
    if args.len() <= i {
        return Ok(default.to_vec());
    }
    sarg(host, args, i, f)
}

fn iarg(host: &mut dyn Host, args: &[Value], i: usize, f: &str) -> Result<i64, Thrown> {
    int_arg(host, args, i, f, pname(f, i + 1))
}

fn iarg_or(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
    default: i64,
) -> Result<i64, Thrown> {
    if args.len() <= i {
        return Ok(default);
    }
    iarg(host, args, i, f)
}

fn internal_error() -> Thrown {
    exc("internal error")
}

fn ok(r: c_int) -> Result<(), Thrown> {
    if r != 0 {
        return Err(internal_error());
    }
    Ok(())
}

fn bytes(v: Vec<u8>) -> Result<Value, Thrown> {
    Ok(Value::str(v))
}

fn random(n: usize) -> Vec<u8> {
    let mut k = vec![0u8; n];
    unsafe { sys::randombytes_buf(k.as_mut_ptr().cast(), n) };
    k
}

/// `zend_parse_parameters_none()` keygen: `n` random bytes.
fn keygen(n: &str) -> Result<Value, Thrown> {
    bytes(random(c(n)))
}

fn p(v: &[u8]) -> *const u8 {
    v.as_ptr()
}

fn cp(v: &[u8]) -> *const c_char {
    v.as_ptr().cast()
}

/// A by-reference state argument: its string, or the SodiumException.
fn state_arg(args: &[Value], f: &str) -> Result<Vec<u8>, Thrown> {
    match args.first().map(deref) {
        Some(Value::Str(s)) => Ok(s.as_bytes().to_vec()),
        _ => Err(arg_err(f, 1, "must be a reference to a state")),
    }
}

/// 64-byte aligned scratch for a libsodium state struct (php copies the
/// string into a `crypto_generichash_state` on the stack).
#[repr(C, align(64))]
struct Aligned([u8; 512]);

impl Aligned {
    fn from(b: &[u8]) -> Box<Aligned> {
        let mut a = Box::new(Aligned([0; 512]));
        a.0[..b.len()].copy_from_slice(b);
        a
    }
    fn ptr(&mut self) -> *mut c_void {
        self.0.as_mut_ptr().cast()
    }
}

impl Drop for Aligned {
    fn drop(&mut self) {
        unsafe { sys::sodium_memzero(self.0.as_mut_ptr().cast(), self.0.len()) };
    }
}

fn generichash_statebytes() -> usize {
    unsafe { sys::rhp_sodium_generichash_statebytes() }
}

fn secretstream_statebytes() -> usize {
    unsafe { sys::rhp_sodium_secretstream_statebytes() }
}

// ---------------------------------------------------------------------------
// utilities
// ---------------------------------------------------------------------------

fn fn_memzero(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if !matches!(args.first().map(deref), Some(Value::Str(_))) {
        return Err(exc("a PHP string is required"));
    }
    set_ref(args, 0, Value::Null);
    Ok(Value::Null)
}

fn fn_increment(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Some(Value::Str(s)) = args.first().map(deref) else {
        return Err(exc("a PHP string is required"));
    };
    let mut v = s.as_bytes().to_vec();
    unsafe { sys::sodium_increment(v.as_mut_ptr(), v.len()) };
    set_ref(args, 0, Value::str(v));
    Ok(Value::Null)
}

fn fn_add(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_add";
    let addv = sarg(host, args, 1, f)?;
    let Some(Value::Str(s)) = args.first().map(deref) else {
        return Err(exc("PHP strings are required"));
    };
    let mut v = s.as_bytes().to_vec();
    if v.len() != addv.len() {
        return Err(arg_err(
            f,
            1,
            "and argument #2 ($string_2) must have the same length",
        ));
    }
    unsafe { sys::sodium_add(v.as_mut_ptr(), p(&addv), v.len()) };
    set_ref(args, 0, Value::str(v));
    Ok(Value::Null)
}

fn same_len_pair(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
) -> Result<(Vec<u8>, Vec<u8>), Thrown> {
    let a = sarg(host, args, 0, f)?;
    let b = sarg(host, args, 1, f)?;
    if a.len() != b.len() {
        return Err(arg_err(
            f,
            1,
            "and argument #2 ($string_2) must have the same length",
        ));
    }
    Ok((a, b))
}

fn fn_memcmp(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (a, b) = same_len_pair(host, args, "sodium_memcmp")?;
    let r = unsafe { sys::sodium_memcmp(p(&a).cast(), p(&b).cast(), a.len()) };
    Ok(Value::Int(r as i64))
}

fn fn_compare(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (a, b) = same_len_pair(host, args, "sodium_compare")?;
    let r = unsafe { sys::sodium_compare(p(&a), p(&b), a.len()) };
    Ok(Value::Int(r as i64))
}

fn fn_bin2hex(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let bin = sarg(host, args, 0, "sodium_bin2hex")?;
    let mut hex = vec![0u8; bin.len() * 2 + 1];
    unsafe { sys::sodium_bin2hex(hex.as_mut_ptr().cast(), hex.len(), p(&bin), bin.len()) };
    hex.pop();
    bytes(hex)
}

fn fn_hex2bin(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_hex2bin";
    let hex = sarg(host, args, 0, f)?;
    let ignore = cstring(sarg_or(host, args, 1, f, b"")?);
    let bin_len = hex.len() / 2;
    let mut bin = vec![0u8; bin_len.max(1)];
    let mut real = 0usize;
    let mut end: *const c_char = null();
    let r = unsafe {
        sys::sodium_hex2bin(
            bin.as_mut_ptr(),
            bin_len,
            cp(&hex),
            hex.len(),
            ignore.as_ptr(),
            &mut real,
            &mut end,
        )
    };
    if r != 0 || end != unsafe { cp(&hex).add(hex.len()) } {
        return Err(arg_err(f, 1, "must be a valid hexadecimal string"));
    }
    bin.truncate(real);
    bytes(bin)
}

/// A C string of `v` up to its first NUL (php passes `char *` arguments
/// as C strings).
fn cstring(mut v: Vec<u8>) -> Vec<c_char> {
    if let Some(n) = v.iter().position(|&b| b == 0) {
        v.truncate(n);
    }
    v.push(0);
    v.into_iter().map(|b| b as c_char).collect()
}

fn variant_ok(v: i64) -> bool {
    ((v as u32) & !0x6) == 0x1
}

fn fn_bin2base64(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_bin2base64";
    let bin = sarg(host, args, 0, f)?;
    let variant = iarg(host, args, 1, f)?;
    if !variant_ok(variant) {
        return Err(arg_err(f, 2, "must be a valid base64 variant identifier"));
    }
    let n = unsafe { sys::sodium_base64_encoded_len(bin.len(), variant as c_int) };
    let mut out = vec![0u8; n];
    unsafe {
        sys::sodium_bin2base64(
            out.as_mut_ptr().cast(),
            n,
            p(&bin),
            bin.len(),
            variant as c_int,
        )
    };
    out.truncate(n - 1);
    bytes(out)
}

fn fn_base642bin(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_base642bin";
    let b64 = sarg(host, args, 0, f)?;
    let variant = iarg(host, args, 1, f)?;
    let ignore = cstring(sarg_or(host, args, 2, f, b"")?);
    if !variant_ok(variant) {
        return Err(arg_err(f, 2, "must be a valid base64 variant identifier"));
    }
    let bin_len = b64.len() / 4 * 3 + 2;
    let mut bin = vec![0u8; bin_len];
    let mut real = 0usize;
    let mut end: *const c_char = null();
    let r = unsafe {
        sys::sodium_base642bin(
            bin.as_mut_ptr(),
            bin_len,
            cp(&b64),
            b64.len(),
            ignore.as_ptr(),
            &mut real,
            &mut end,
            variant as c_int,
        )
    };
    if r != 0 || end != unsafe { cp(&b64).add(b64.len()) } {
        return Err(arg_err(f, 1, "must be a valid base64 string"));
    }
    bin.truncate(real);
    bytes(bin)
}

fn blocksize_arg(host: &mut dyn Host, args: &[Value], f: &str) -> Result<usize, Thrown> {
    let b = iarg(host, args, 1, f)?;
    if b <= 0 {
        return Err(arg_err(f, 2, "must be greater than 0"));
    }
    Ok(b as usize)
}

fn fn_pad(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_pad";
    let unpadded = sarg(host, args, 0, f)?;
    let blocksize = blocksize_arg(host, args, f)?;
    let mut xpadlen = blocksize - 1;
    if blocksize & (blocksize - 1) == 0 {
        xpadlen -= unpadded.len() & (blocksize - 1);
    } else {
        xpadlen -= unpadded.len() % blocksize;
    }
    if usize::MAX - unpadded.len() <= xpadlen {
        return Err(exc("input is too large"));
    }
    let xpadded_len = unpadded.len() + xpadlen;
    let mut padded = vec![0u8; xpadded_len + 1];
    padded[..unpadded.len()].copy_from_slice(&unpadded);
    let r = unsafe {
        sys::sodium_pad(
            null_mut(),
            padded.as_mut_ptr(),
            unpadded.len(),
            blocksize,
            xpadded_len + 1,
        )
    };
    ok(r)?;
    bytes(padded)
}

fn fn_unpad(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_unpad";
    let padded = sarg(host, args, 0, f)?;
    let blocksize = blocksize_arg(host, args, f)?;
    if padded.len() < blocksize {
        return Err(arg_err(f, 1, "must be at least as long as the block size"));
    }
    let mut len = 0usize;
    let r = unsafe { sys::sodium_unpad(&mut len, p(&padded), padded.len(), blocksize) };
    if r != 0 || len > i64::MAX as usize {
        return Err(exc("invalid padding"));
    }
    bytes(padded[..len].to_vec())
}

// ---------------------------------------------------------------------------
// AEAD
// ---------------------------------------------------------------------------

/// One AEAD construction of libsodium.c's encrypt/decrypt pairs.
struct Aead {
    /// The constants' infix: `SODIUM_CRYPTO_AEAD_<NAME>_KEYBYTES`.
    name: &'static str,
    enc: AeadEncrypt,
    dec: AeadDecrypt,
    limit: Limit,
}

/// The per-key length checks each pair makes.
#[derive(Clone, Copy, PartialEq)]
enum Limit {
    None,
    AesGcm,
    Ietf,
    XChaCha,
}

const AES: Aead = Aead {
    name: "AES256GCM",
    enc: sys::crypto_aead_aes256gcm_encrypt,
    dec: sys::crypto_aead_aes256gcm_decrypt,
    limit: Limit::AesGcm,
};
const AEGIS128L: Aead = Aead {
    name: "AEGIS128L",
    enc: sys::rhp_sodium_aegis128l_encrypt,
    dec: sys::rhp_sodium_aegis128l_decrypt,
    limit: Limit::None,
};
const AEGIS256: Aead = Aead {
    name: "AEGIS256",
    enc: sys::rhp_sodium_aegis256_encrypt,
    dec: sys::rhp_sodium_aegis256_decrypt,
    limit: Limit::None,
};
const CHACHA: Aead = Aead {
    name: "CHACHA20POLY1305",
    enc: sys::crypto_aead_chacha20poly1305_encrypt,
    dec: sys::crypto_aead_chacha20poly1305_decrypt,
    limit: Limit::None,
};
const IETF: Aead = Aead {
    name: "CHACHA20POLY1305_IETF",
    enc: sys::crypto_aead_chacha20poly1305_ietf_encrypt,
    dec: sys::crypto_aead_chacha20poly1305_ietf_decrypt,
    limit: Limit::Ietf,
};
const XCHACHA: Aead = Aead {
    name: "XCHACHA20POLY1305_IETF",
    enc: sys::crypto_aead_xchacha20poly1305_ietf_encrypt,
    dec: sys::crypto_aead_xchacha20poly1305_ietf_decrypt,
    limit: Limit::XChaCha,
};

/// 64 * 2^32 - 64: the IETF ChaCha20 limit for one key.
const IETF_MAX: u64 = 64 * (1 << 32) - 64;
/// 16 * (2^32 - 2): the AES-GCM limit for one key.
const AES_MAX: u64 = 16 * ((1 << 32) - 2);

impl Aead {
    fn k(&self, what: &str) -> String {
        format!("SODIUM_CRYPTO_AEAD_{}_{what}", self.name)
    }

    fn fname(&self, op: &str) -> String {
        format!("sodium_crypto_aead_{}_{op}", self.name.to_ascii_lowercase())
    }

    fn args(&self, host: &mut dyn Host, args: &[Value], f: &str) -> Result<[Vec<u8>; 4], Thrown> {
        let a = [
            sarg(host, args, 0, f)?,
            sarg(host, args, 1, f)?,
            sarg(host, args, 2, f)?,
            sarg(host, args, 3, f)?,
        ];
        need(f, 3, &a[2], &self.k("NPUBBYTES"))?;
        need(f, 4, &a[3], &self.k("KEYBYTES"))?;
        Ok(a)
    }

    fn encrypt(&self, host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
        let f = self.fname("encrypt");
        let [msg, ad, npub, key] = self.args(host, args, &f)?;
        let abytes = c(&self.k("ABYTES"));
        if ZSTR_MAX_LEN - msg.len() <= abytes {
            return Err(exc("arithmetic overflow"));
        }
        let too_long = match self.limit {
            Limit::AesGcm => msg.len() as u64 > AES_MAX - abytes as u64,
            Limit::Ietf => msg.len() as u64 > IETF_MAX,
            _ => false,
        };
        if too_long {
            return Err(exc("message too long for a single key"));
        }
        let clen = msg.len() + abytes;
        let mut out = vec![0u8; clen];
        let mut real: u64 = 0;
        let r = unsafe {
            (self.enc)(
                out.as_mut_ptr(),
                &mut real,
                p(&msg),
                msg.len() as u64,
                p(&ad),
                ad.len() as u64,
                null(),
                p(&npub),
                p(&key),
            )
        };
        ok(r)?;
        if real == 0 || real as usize > clen {
            return Err(exc("arithmetic overflow"));
        }
        out.truncate(real as usize);
        bytes(out)
    }

    fn decrypt(&self, host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
        let f = self.fname("decrypt");
        let [ct, ad, npub, key] = self.args(host, args, &f)?;
        let abytes = c(&self.k("ABYTES"));
        if ct.len() < abytes {
            return Ok(Value::Bool(false));
        }
        let over = (ct.len() - abytes) as u64;
        match self.limit {
            Limit::AesGcm if over > AES_MAX => return Err(arg_err(&f, 1, "is too long")),
            Limit::Ietf if over > IETF_MAX => return Err(exc("message too long for a single key")),
            Limit::XChaCha if over > IETF_MAX => return Err(exc("arithmetic overflow")),
            _ => {}
        }
        let mut out = vec![0u8; ct.len().max(1)];
        let mut real: u64 = 0;
        let r = unsafe {
            (self.dec)(
                out.as_mut_ptr(),
                &mut real,
                null_mut(),
                p(&ct),
                ct.len() as u64,
                p(&ad),
                ad.len() as u64,
                p(&npub),
                p(&key),
            )
        };
        if r != 0 {
            return Ok(Value::Bool(false));
        }
        if real as usize > ct.len() {
            return Err(exc("arithmetic overflow"));
        }
        out.truncate(real as usize);
        bytes(out)
    }
}

fn fn_aes256gcm_is_available(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(
        has(feat::AESGCM) && unsafe { sys::crypto_aead_aes256gcm_is_available() } != 0,
    ))
}
fn fn_aes256gcm_encrypt(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    AES.encrypt(h, a)
}
fn fn_aes256gcm_decrypt(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    AES.decrypt(h, a)
}
fn fn_aes256gcm_keygen(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    keygen("SODIUM_CRYPTO_AEAD_AES256GCM_KEYBYTES")
}
fn fn_aegis128l_encrypt(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    AEGIS128L.encrypt(h, a)
}
fn fn_aegis128l_decrypt(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    AEGIS128L.decrypt(h, a)
}
fn fn_aegis128l_keygen(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    let mut k = vec![0u8; c("SODIUM_CRYPTO_AEAD_AEGIS128L_KEYBYTES")];
    unsafe { sys::rhp_sodium_aegis128l_keygen(k.as_mut_ptr()) };
    bytes(k)
}
fn fn_aegis256_encrypt(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    AEGIS256.encrypt(h, a)
}
fn fn_aegis256_decrypt(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    AEGIS256.decrypt(h, a)
}
fn fn_aegis256_keygen(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    let mut k = vec![0u8; c("SODIUM_CRYPTO_AEAD_AEGIS256_KEYBYTES")];
    unsafe { sys::rhp_sodium_aegis256_keygen(k.as_mut_ptr()) };
    bytes(k)
}
fn fn_chacha_encrypt(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    CHACHA.encrypt(h, a)
}
fn fn_chacha_decrypt(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    CHACHA.decrypt(h, a)
}
fn fn_chacha_keygen(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    keygen("SODIUM_CRYPTO_AEAD_CHACHA20POLY1305_KEYBYTES")
}
fn fn_ietf_encrypt(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    IETF.encrypt(h, a)
}
fn fn_ietf_decrypt(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    IETF.decrypt(h, a)
}
fn fn_ietf_keygen(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    keygen("SODIUM_CRYPTO_AEAD_CHACHA20POLY1305_IETF_KEYBYTES")
}
fn fn_xchacha_encrypt(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    XCHACHA.encrypt(h, a)
}
fn fn_xchacha_decrypt(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    XCHACHA.decrypt(h, a)
}
fn fn_xchacha_keygen(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    keygen("SODIUM_CRYPTO_AEAD_XCHACHA20POLY1305_IETF_KEYBYTES")
}

// ---------------------------------------------------------------------------
// keygens
// ---------------------------------------------------------------------------

fn fn_auth_keygen(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    keygen("SODIUM_CRYPTO_AUTH_KEYBYTES")
}
fn fn_generichash_keygen(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    keygen("SODIUM_CRYPTO_GENERICHASH_KEYBYTES")
}
fn fn_kdf_keygen(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    keygen("SODIUM_CRYPTO_KDF_KEYBYTES")
}
fn fn_secretbox_keygen(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    keygen("SODIUM_CRYPTO_SECRETBOX_KEYBYTES")
}
fn fn_shorthash_keygen(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    keygen("SODIUM_CRYPTO_SHORTHASH_KEYBYTES")
}
fn fn_stream_keygen(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    keygen("SODIUM_CRYPTO_STREAM_KEYBYTES")
}
fn fn_stream_xchacha20_keygen(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    keygen("SODIUM_CRYPTO_STREAM_XCHACHA20_KEYBYTES")
}
fn fn_ss_keygen(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    keygen("SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_KEYBYTES")
}

// ---------------------------------------------------------------------------
// shorthash, secretbox, auth, generichash, kdf
// ---------------------------------------------------------------------------

fn fn_shorthash(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_shorthash";
    let msg = sarg(host, args, 0, f)?;
    let key = sarg(host, args, 1, f)?;
    need(f, 2, &key, "SODIUM_CRYPTO_SHORTHASH_KEYBYTES")?;
    let mut out = vec![0u8; c("SODIUM_CRYPTO_SHORTHASH_BYTES")];
    ok(unsafe { sys::crypto_shorthash(out.as_mut_ptr(), p(&msg), msg.len() as u64, p(&key)) })?;
    bytes(out)
}

fn nonce_key(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    nonce: &str,
    key: &str,
) -> Result<[Vec<u8>; 3], Thrown> {
    let a = [
        sarg(host, args, 0, f)?,
        sarg(host, args, 1, f)?,
        sarg(host, args, 2, f)?,
    ];
    need(f, 2, &a[1], nonce)?;
    need(f, 3, &a[2], key)?;
    Ok(a)
}

fn fn_secretbox(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let [msg, nonce, key] = nonce_key(
        host,
        args,
        "sodium_crypto_secretbox",
        "SODIUM_CRYPTO_SECRETBOX_NONCEBYTES",
        "SODIUM_CRYPTO_SECRETBOX_KEYBYTES",
    )?;
    let mut out = vec![0u8; msg.len() + c("SODIUM_CRYPTO_SECRETBOX_MACBYTES")];
    ok(unsafe {
        sys::crypto_secretbox_easy(
            out.as_mut_ptr(),
            p(&msg),
            msg.len() as u64,
            p(&nonce),
            p(&key),
        )
    })?;
    bytes(out)
}

fn fn_secretbox_open(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let [ct, nonce, key] = nonce_key(
        host,
        args,
        "sodium_crypto_secretbox_open",
        "SODIUM_CRYPTO_SECRETBOX_NONCEBYTES",
        "SODIUM_CRYPTO_SECRETBOX_KEYBYTES",
    )?;
    let mac = c("SODIUM_CRYPTO_SECRETBOX_MACBYTES");
    if ct.len() < mac {
        return Ok(Value::Bool(false));
    }
    let mut out = vec![0u8; (ct.len() - mac).max(1)];
    let r = unsafe {
        sys::crypto_secretbox_open_easy(
            out.as_mut_ptr(),
            p(&ct),
            ct.len() as u64,
            p(&nonce),
            p(&key),
        )
    };
    if r != 0 {
        return Ok(Value::Bool(false));
    }
    out.truncate(ct.len() - mac);
    bytes(out)
}

fn fn_crypto_auth(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_auth";
    let msg = sarg(host, args, 0, f)?;
    let key = sarg(host, args, 1, f)?;
    need(f, 2, &key, "SODIUM_CRYPTO_AUTH_KEYBYTES")?;
    let mut out = vec![0u8; c("SODIUM_CRYPTO_AUTH_BYTES")];
    ok(unsafe { sys::crypto_auth(out.as_mut_ptr(), p(&msg), msg.len() as u64, p(&key)) })?;
    bytes(out)
}

fn fn_crypto_auth_verify(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_auth_verify";
    let mac = sarg(host, args, 0, f)?;
    let msg = sarg(host, args, 1, f)?;
    let key = sarg(host, args, 2, f)?;
    need(f, 3, &key, "SODIUM_CRYPTO_AUTH_KEYBYTES")?;
    need(f, 1, &mac, "SODIUM_CRYPTO_AUTH_BYTES")?;
    let r = unsafe { sys::crypto_auth_verify(p(&mac), p(&msg), msg.len() as u64, p(&key)) };
    Ok(Value::Bool(r == 0))
}

fn hash_len_ok(n: i64) -> Result<(), Thrown> {
    let min = c("SODIUM_CRYPTO_GENERICHASH_BYTES_MIN") as i64;
    let max = c("SODIUM_CRYPTO_GENERICHASH_BYTES_MAX") as i64;
    if n < min || n > max {
        return Err(exc("unsupported output length"));
    }
    Ok(())
}

fn key_len_ok(key: &[u8]) -> Result<(), Thrown> {
    let (min, max) = (
        c("SODIUM_CRYPTO_GENERICHASH_KEYBYTES_MIN"),
        c("SODIUM_CRYPTO_GENERICHASH_KEYBYTES_MAX"),
    );
    if !key.is_empty() && (key.len() < min || key.len() > max) {
        return Err(exc("unsupported key length"));
    }
    Ok(())
}

fn key_ptr(key: &[u8]) -> *const u8 {
    if key.is_empty() {
        null()
    } else {
        p(key)
    }
}

fn fn_generichash(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_generichash";
    let msg = sarg(host, args, 0, f)?;
    let key = sarg_or(host, args, 1, f, b"")?;
    let len = iarg_or(
        host,
        args,
        2,
        f,
        c("SODIUM_CRYPTO_GENERICHASH_BYTES") as i64,
    )?;
    hash_len_ok(len)?;
    key_len_ok(&key)?;
    let mut out = vec![0u8; len as usize];
    ok(unsafe {
        sys::crypto_generichash(
            out.as_mut_ptr(),
            len as usize,
            p(&msg),
            msg.len() as u64,
            key_ptr(&key),
            key.len(),
        )
    })?;
    bytes(out)
}

fn fn_generichash_init(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_generichash_init";
    let key = sarg_or(host, args, 0, f, b"")?;
    let len = iarg_or(
        host,
        args,
        1,
        f,
        c("SODIUM_CRYPTO_GENERICHASH_BYTES") as i64,
    )?;
    hash_len_ok(len)?;
    key_len_ok(&key)?;
    let mut st = Aligned::from(&[]);
    ok(unsafe { sys::crypto_generichash_init(st.ptr(), key_ptr(&key), key.len(), len as usize) })?;
    bytes(st.0[..generichash_statebytes()].to_vec())
}

fn fn_generichash_update(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_generichash_update";
    let msg = sarg(host, args, 1, f)?;
    let state = state_arg(args, f)?;
    let n = generichash_statebytes();
    if state.len() != n {
        return Err(exc("incorrect state length"));
    }
    let mut st = Aligned::from(&state);
    ok(unsafe { sys::crypto_generichash_update(st.ptr(), p(&msg), msg.len() as u64) })?;
    set_ref(args, 0, Value::str(st.0[..n].to_vec()));
    Ok(Value::Bool(true))
}

fn fn_generichash_final(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_generichash_final";
    let len = iarg_or(
        host,
        args,
        1,
        f,
        c("SODIUM_CRYPTO_GENERICHASH_BYTES") as i64,
    )?;
    let state = state_arg(args, f)?;
    if state.len() != generichash_statebytes() {
        return Err(exc("incorrect state length"));
    }
    hash_len_ok(len)?;
    let mut st = Aligned::from(&state);
    let mut out = vec![0u8; len as usize];
    ok(unsafe { sys::crypto_generichash_final(st.ptr(), out.as_mut_ptr(), len as usize) })?;
    set_ref(args, 0, Value::Null);
    bytes(out)
}

fn fn_kdf_derive_from_key(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_kdf_derive_from_key";
    let subkey_len = iarg(host, args, 0, f)?;
    let subkey_id = iarg(host, args, 1, f)?;
    let ctx = sarg(host, args, 2, f)?;
    let key = sarg(host, args, 3, f)?;
    if subkey_len < c("SODIUM_CRYPTO_KDF_BYTES_MIN") as i64 {
        return Err(arg_err(
            f,
            1,
            "must be greater than or equal to SODIUM_CRYPTO_KDF_BYTES_MIN",
        ));
    }
    if subkey_len > c("SODIUM_CRYPTO_KDF_BYTES_MAX") as i64 {
        return Err(arg_err(
            f,
            1,
            "must be less than or equal to SODIUM_CRYPTO_KDF_BYTES_MAX",
        ));
    }
    if subkey_id < 0 {
        return Err(arg_err(f, 2, "must be greater than or equal to 0"));
    }
    need(f, 3, &ctx, "SODIUM_CRYPTO_KDF_CONTEXTBYTES")?;
    need(f, 4, &key, "SODIUM_CRYPTO_KDF_KEYBYTES")?;
    let mut out = vec![0u8; subkey_len as usize];
    if has(feat::KDF_PRIMITIVE) {
        unsafe {
            sys::crypto_kdf_derive_from_key(
                out.as_mut_ptr(),
                out.len(),
                subkey_id as u64,
                cp(&ctx),
                p(&key),
            )
        };
    } else {
        let mut ctx_padded = vec![0u8; c("crypto_generichash_blake2b_PERSONALBYTES")];
        ctx_padded[..ctx.len()].copy_from_slice(&ctx);
        let mut salt = vec![0u8; c("crypto_generichash_blake2b_SALTBYTES")];
        salt[..8].copy_from_slice(&(subkey_id as u64).to_le_bytes());
        unsafe {
            sys::crypto_generichash_blake2b_salt_personal(
                out.as_mut_ptr(),
                out.len(),
                null(),
                0,
                p(&key),
                key.len(),
                p(&salt),
                p(&ctx_padded),
            )
        };
    }
    bytes(out)
}

// ---------------------------------------------------------------------------
// box, kx, scalarmult
// ---------------------------------------------------------------------------

/// A keypair argument's (secret, public) halves, checked against
/// `<PREFIX>_KEYPAIRBYTES`.
fn keypair<'a>(
    f: &str,
    n: usize,
    kp: &'a [u8],
    prefix: &str,
) -> Result<(&'a [u8], &'a [u8]), Thrown> {
    need(f, n, kp, &format!("{prefix}_KEYPAIRBYTES"))?;
    Ok(kp.split_at(c(&format!("{prefix}_SECRETKEYBYTES"))))
}

/// `crypto_*_keypair(pk, sk)` into "sk || pk".
fn make_keypair(
    prefix: &str,
    gen: impl FnOnce(*mut u8, *mut u8) -> c_int,
) -> Result<Value, Thrown> {
    let sk = c(&format!("{prefix}_SECRETKEYBYTES"));
    let pk = c(&format!("{prefix}_PUBLICKEYBYTES"));
    let mut kp = vec![0u8; sk + pk];
    let base = kp.as_mut_ptr();
    ok(gen(unsafe { base.add(sk) }, base))?;
    bytes(kp)
}

fn fn_box_keypair(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    make_keypair("SODIUM_CRYPTO_BOX", |pk, sk| unsafe {
        sys::crypto_box_keypair(pk, sk)
    })
}

fn fn_box_seed_keypair(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_box_seed_keypair";
    let seed = sarg(host, args, 0, f)?;
    need(f, 1, &seed, "SODIUM_CRYPTO_BOX_SEEDBYTES")?;
    make_keypair("SODIUM_CRYPTO_BOX", |pk, sk| unsafe {
        sys::crypto_box_seed_keypair(pk, sk, p(&seed))
    })
}

fn keypair_from(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    prefix: &str,
) -> Result<Value, Thrown> {
    let sk = sarg(host, args, 0, f)?;
    let pk = sarg(host, args, 1, f)?;
    need(f, 1, &sk, &format!("{prefix}_SECRETKEYBYTES"))?;
    need(f, 2, &pk, &format!("{prefix}_PUBLICKEYBYTES"))?;
    bytes([sk, pk].concat())
}

fn fn_box_keypair_from_sk_pk(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    keypair_from(
        host,
        args,
        "sodium_crypto_box_keypair_from_secretkey_and_publickey",
        "SODIUM_CRYPTO_BOX",
    )
}

/// `<x>_secretkey` / `<x>_publickey`: one half of a keypair.
fn half(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    prefix: &str,
    public: bool,
) -> Result<Value, Thrown> {
    let kp = sarg(host, args, 0, f)?;
    let (sk, pk) = keypair(f, 1, &kp, prefix)?;
    bytes(if public { pk.to_vec() } else { sk.to_vec() })
}

fn fn_box_secretkey(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    half(
        h,
        a,
        "sodium_crypto_box_secretkey",
        "SODIUM_CRYPTO_BOX",
        false,
    )
}
fn fn_box_publickey(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    half(
        h,
        a,
        "sodium_crypto_box_publickey",
        "SODIUM_CRYPTO_BOX",
        true,
    )
}
fn fn_kx_secretkey(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    half(
        h,
        a,
        "sodium_crypto_kx_secretkey",
        "SODIUM_CRYPTO_KX",
        false,
    )
}
fn fn_kx_publickey(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    half(h, a, "sodium_crypto_kx_publickey", "SODIUM_CRYPTO_KX", true)
}
fn fn_sign_secretkey(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    half(
        h,
        a,
        "sodium_crypto_sign_secretkey",
        "SODIUM_CRYPTO_SIGN",
        false,
    )
}
fn fn_sign_publickey(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    half(
        h,
        a,
        "sodium_crypto_sign_publickey",
        "SODIUM_CRYPTO_SIGN",
        true,
    )
}

fn scalarmult_base(f: &str, sk: &[u8]) -> Result<Value, Thrown> {
    need(f, 1, sk, "SODIUM_CRYPTO_BOX_SECRETKEYBYTES")?;
    let mut pk = vec![0u8; c("SODIUM_CRYPTO_BOX_PUBLICKEYBYTES")];
    unsafe { sys::crypto_scalarmult_base(pk.as_mut_ptr(), p(sk)) };
    bytes(pk)
}

fn fn_box_publickey_from_secretkey(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_box_publickey_from_secretkey";
    let sk = sarg(host, args, 0, f)?;
    scalarmult_base(f, &sk)
}

/// `sodium_crypto_scalarmult_base`, php's alias of
/// sodium_crypto_box_publickey_from_secretkey (its messages name the alias's
/// parameter).
fn fn_box_publickey_from_secretkey_alias(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let f = "sodium_crypto_scalarmult_base";
    let sk = sarg(host, args, 0, f)?;
    scalarmult_base(f, &sk)
}

fn fn_box(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_box";
    let msg = sarg(host, args, 0, f)?;
    let nonce = sarg(host, args, 1, f)?;
    let kp = sarg(host, args, 2, f)?;
    need(f, 2, &nonce, "SODIUM_CRYPTO_BOX_NONCEBYTES")?;
    let (sk, pk) = keypair(f, 3, &kp, "SODIUM_CRYPTO_BOX")?;
    let mut out = vec![0u8; msg.len() + c("SODIUM_CRYPTO_BOX_MACBYTES")];
    ok(unsafe {
        sys::crypto_box_easy(
            out.as_mut_ptr(),
            p(&msg),
            msg.len() as u64,
            p(&nonce),
            p(pk),
            p(sk),
        )
    })?;
    bytes(out)
}

fn fn_box_open(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_box_open";
    let ct = sarg(host, args, 0, f)?;
    let nonce = sarg(host, args, 1, f)?;
    let kp = sarg(host, args, 2, f)?;
    need(f, 2, &nonce, "SODIUM_CRYPTO_BOX_NONCEBYTES")?;
    let (sk, pk) = keypair(f, 3, &kp, "SODIUM_CRYPTO_BOX")?;
    let mac = c("SODIUM_CRYPTO_BOX_MACBYTES");
    if ct.len() < mac {
        return Ok(Value::Bool(false));
    }
    let mut out = vec![0u8; (ct.len() - mac).max(1)];
    let r = unsafe {
        sys::crypto_box_open_easy(
            out.as_mut_ptr(),
            p(&ct),
            ct.len() as u64,
            p(&nonce),
            p(pk),
            p(sk),
        )
    };
    if r != 0 {
        return Ok(Value::Bool(false));
    }
    out.truncate(ct.len() - mac);
    bytes(out)
}

fn fn_box_seal(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_box_seal";
    let msg = sarg(host, args, 0, f)?;
    let pk = sarg(host, args, 1, f)?;
    need(f, 2, &pk, "SODIUM_CRYPTO_BOX_PUBLICKEYBYTES")?;
    let mut out = vec![0u8; msg.len() + c("SODIUM_CRYPTO_BOX_SEALBYTES")];
    ok(unsafe { sys::crypto_box_seal(out.as_mut_ptr(), p(&msg), msg.len() as u64, p(&pk)) })?;
    bytes(out)
}

fn fn_box_seal_open(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_box_seal_open";
    let ct = sarg(host, args, 0, f)?;
    let kp = sarg(host, args, 1, f)?;
    let (sk, pk) = keypair(f, 2, &kp, "SODIUM_CRYPTO_BOX")?;
    let seal = c("SODIUM_CRYPTO_BOX_SEALBYTES");
    if ct.len() < seal {
        return Ok(Value::Bool(false));
    }
    let mut out = vec![0u8; (ct.len() - seal).max(1)];
    let r = unsafe {
        sys::crypto_box_seal_open(out.as_mut_ptr(), p(&ct), ct.len() as u64, p(pk), p(sk))
    };
    if r != 0 {
        return Ok(Value::Bool(false));
    }
    out.truncate(ct.len() - seal);
    bytes(out)
}

fn kx_from_sk(sk: Vec<u8>) -> Result<Value, Thrown> {
    let mut kp = sk;
    let n = kp.len();
    kp.resize(n + c("SODIUM_CRYPTO_KX_PUBLICKEYBYTES"), 0);
    let (s, pk) = kp.split_at_mut(n);
    ok(unsafe { sys::crypto_scalarmult_base(pk.as_mut_ptr(), p(s)) })?;
    bytes(kp)
}

fn fn_kx_keypair(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    kx_from_sk(random(c("SODIUM_CRYPTO_KX_SECRETKEYBYTES")))
}

fn fn_kx_seed_keypair(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_kx_seed_keypair";
    let seed = sarg(host, args, 0, f)?;
    need(f, 1, &seed, "SODIUM_CRYPTO_KX_SEEDBYTES")?;
    let mut sk = vec![0u8; c("SODIUM_CRYPTO_KX_SECRETKEYBYTES")];
    unsafe {
        sys::crypto_generichash(
            sk.as_mut_ptr(),
            sk.len(),
            p(&seed),
            seed.len() as u64,
            null(),
            0,
        )
    };
    kx_from_sk(sk)
}

/// php's kx session keys: BLAKE2b(q || client_pk || server_pk), split in
/// two; the server gets the halves swapped.
fn kx_session(host: &mut dyn Host, args: &[Value], f: &str, server: bool) -> Result<Value, Thrown> {
    let kp = sarg(host, args, 0, f)?;
    let other = sarg(host, args, 1, f)?;
    let (sk, pk) = keypair(f, 1, &kp, "SODIUM_CRYPTO_KX")?;
    need(f, 2, &other, "SODIUM_CRYPTO_KX_PUBLICKEYBYTES")?;
    let mut q = vec![0u8; c("SODIUM_CRYPTO_SCALARMULT_BYTES")];
    ok(unsafe { sys::crypto_scalarmult(q.as_mut_ptr(), p(sk), p(&other)) })?;
    let (client_pk, server_pk) = if server {
        (&other[..], pk)
    } else {
        (pk, &other[..])
    };
    let n = c("SODIUM_CRYPTO_KX_SESSIONKEYBYTES");
    let mut keys = vec![0u8; 2 * n];
    let mut h = Aligned::from(&[]);
    unsafe {
        sys::crypto_generichash_init(h.ptr(), null(), 0, 2 * n);
        sys::crypto_generichash_update(h.ptr(), p(&q), q.len() as u64);
        sys::sodium_memzero(q.as_mut_ptr().cast(), q.len());
        sys::crypto_generichash_update(h.ptr(), p(client_pk), client_pk.len() as u64);
        sys::crypto_generichash_update(h.ptr(), p(server_pk), server_pk.len() as u64);
        sys::crypto_generichash_final(h.ptr(), keys.as_mut_ptr(), 2 * n);
    }
    let (a, b) = keys.split_at(n);
    let mut out = PhpArray::new();
    let (first, second) = if server { (b, a) } else { (a, b) };
    out.push(Value::str(first.to_vec()));
    out.push(Value::str(second.to_vec()));
    Ok(Value::Array(out))
}

fn fn_kx_client_session_keys(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    kx_session(h, a, "sodium_crypto_kx_client_session_keys", false)
}

fn fn_kx_server_session_keys(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    kx_session(h, a, "sodium_crypto_kx_server_session_keys", true)
}

fn fn_scalarmult(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_scalarmult";
    let n = sarg(host, args, 0, f)?;
    let pt = sarg(host, args, 1, f)?;
    need(f, 1, &n, "SODIUM_CRYPTO_SCALARMULT_SCALARBYTES")?;
    need(f, 2, &pt, "SODIUM_CRYPTO_SCALARMULT_BYTES")?;
    let mut q = vec![0u8; c("SODIUM_CRYPTO_SCALARMULT_BYTES")];
    ok(unsafe { sys::crypto_scalarmult(q.as_mut_ptr(), p(&n), p(&pt)) })?;
    bytes(q)
}

fn fn_scalarmult_r255(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_scalarmult_ristretto255";
    let n = sarg(host, args, 0, f)?;
    let pt = sarg(host, args, 1, f)?;
    need(
        f,
        1,
        &n,
        "SODIUM_CRYPTO_SCALARMULT_RISTRETTO255_SCALARBYTES",
    )?;
    need(f, 2, &pt, "SODIUM_CRYPTO_SCALARMULT_RISTRETTO255_BYTES")?;
    let mut q = vec![0u8; c("SODIUM_CRYPTO_SCALARMULT_RISTRETTO255_BYTES")];
    if unsafe { sys::crypto_scalarmult_ristretto255(q.as_mut_ptr(), p(&n), p(&pt)) } != 0 {
        return Err(exc("Result is identity element"));
    }
    bytes(q)
}

fn fn_scalarmult_r255_base(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_scalarmult_ristretto255_base";
    let n = sarg(host, args, 0, f)?;
    need(
        f,
        1,
        &n,
        "SODIUM_CRYPTO_SCALARMULT_RISTRETTO255_SCALARBYTES",
    )?;
    let mut q = vec![0u8; c("SODIUM_CRYPTO_SCALARMULT_RISTRETTO255_BYTES")];
    if unsafe { sys::crypto_scalarmult_ristretto255_base(q.as_mut_ptr(), p(&n)) } != 0 {
        return Err(arg_err(f, 1, "must not be zero"));
    }
    bytes(q)
}

// ---------------------------------------------------------------------------
// sign
// ---------------------------------------------------------------------------

fn fn_sign_keypair(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    make_keypair("SODIUM_CRYPTO_SIGN", |pk, sk| unsafe {
        sys::crypto_sign_keypair(pk, sk)
    })
}

fn fn_sign_seed_keypair(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_sign_seed_keypair";
    let seed = sarg(host, args, 0, f)?;
    need(f, 1, &seed, "SODIUM_CRYPTO_SIGN_SEEDBYTES")?;
    make_keypair("SODIUM_CRYPTO_SIGN", |pk, sk| unsafe {
        sys::crypto_sign_seed_keypair(pk, sk, p(&seed))
    })
}

fn fn_sign_keypair_from_sk_pk(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    keypair_from(
        host,
        args,
        "sodium_crypto_sign_keypair_from_secretkey_and_publickey",
        "SODIUM_CRYPTO_SIGN",
    )
}

fn fn_sign_publickey_from_secretkey(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_sign_publickey_from_secretkey";
    let sk = sarg(host, args, 0, f)?;
    need(f, 1, &sk, "SODIUM_CRYPTO_SIGN_SECRETKEYBYTES")?;
    let mut pk = vec![0u8; c("SODIUM_CRYPTO_SIGN_PUBLICKEYBYTES")];
    ok(unsafe { sys::crypto_sign_ed25519_sk_to_pk(pk.as_mut_ptr(), p(&sk)) })?;
    bytes(pk)
}

fn fn_sign(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_sign";
    let msg = sarg(host, args, 0, f)?;
    let sk = sarg(host, args, 1, f)?;
    need(f, 2, &sk, "SODIUM_CRYPTO_SIGN_SECRETKEYBYTES")?;
    let n = msg.len() + c("SODIUM_CRYPTO_SIGN_BYTES");
    let mut out = vec![0u8; n];
    let mut real: u64 = 0;
    ok(unsafe {
        sys::crypto_sign(
            out.as_mut_ptr(),
            &mut real,
            p(&msg),
            msg.len() as u64,
            p(&sk),
        )
    })?;
    if real as usize > n {
        return Err(exc("arithmetic overflow"));
    }
    out.truncate(real as usize);
    bytes(out)
}

fn fn_sign_open(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_sign_open";
    let sm = sarg(host, args, 0, f)?;
    let pk = sarg(host, args, 1, f)?;
    need(f, 2, &pk, "SODIUM_CRYPTO_SIGN_PUBLICKEYBYTES")?;
    let mut out = vec![0u8; sm.len().max(1)];
    let mut real: u64 = 0;
    let r = unsafe {
        sys::crypto_sign_open(out.as_mut_ptr(), &mut real, p(&sm), sm.len() as u64, p(&pk))
    };
    if r != 0 {
        return Ok(Value::Bool(false));
    }
    if real as usize > sm.len() {
        return Err(exc("arithmetic overflow"));
    }
    out.truncate(real as usize);
    bytes(out)
}

fn fn_sign_detached(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_sign_detached";
    let msg = sarg(host, args, 0, f)?;
    let sk = sarg(host, args, 1, f)?;
    need(f, 2, &sk, "SODIUM_CRYPTO_SIGN_SECRETKEYBYTES")?;
    let n = c("SODIUM_CRYPTO_SIGN_BYTES");
    let mut sig = vec![0u8; n];
    let mut real: u64 = 0;
    let r = unsafe {
        sys::crypto_sign_detached(
            sig.as_mut_ptr(),
            &mut real,
            p(&msg),
            msg.len() as u64,
            p(&sk),
        )
    };
    if r != 0 {
        return Err(exc("signature creation failed"));
    }
    if real == 0 || real as usize > n {
        return Err(exc("signature has a bogus size"));
    }
    sig.truncate(real as usize);
    bytes(sig)
}

fn fn_sign_verify_detached(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_sign_verify_detached";
    let sig = sarg(host, args, 0, f)?;
    let msg = sarg(host, args, 1, f)?;
    let pk = sarg(host, args, 2, f)?;
    need(f, 1, &sig, "SODIUM_CRYPTO_SIGN_BYTES")?;
    need(f, 3, &pk, "SODIUM_CRYPTO_SIGN_PUBLICKEYBYTES")?;
    let r = unsafe { sys::crypto_sign_verify_detached(p(&sig), p(&msg), msg.len() as u64, p(&pk)) };
    Ok(Value::Bool(r == 0))
}

fn to_curve(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    len_name: &str,
    out_len: &str,
    conv: unsafe extern "C" fn(*mut u8, *const u8) -> c_int,
) -> Result<Value, Thrown> {
    let k = sarg(host, args, 0, f)?;
    need(f, 1, &k, len_name)?;
    let mut out = vec![0u8; c(out_len)];
    if unsafe { conv(out.as_mut_ptr(), p(&k)) } != 0 {
        return Err(exc("conversion failed"));
    }
    bytes(out)
}

fn fn_sign_sk_to_curve25519(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    to_curve(
        h,
        a,
        "sodium_crypto_sign_ed25519_sk_to_curve25519",
        "SODIUM_CRYPTO_SIGN_SECRETKEYBYTES",
        "SODIUM_CRYPTO_BOX_SECRETKEYBYTES",
        sys::crypto_sign_ed25519_sk_to_curve25519,
    )
}

fn fn_sign_pk_to_curve25519(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    to_curve(
        h,
        a,
        "sodium_crypto_sign_ed25519_pk_to_curve25519",
        "SODIUM_CRYPTO_SIGN_PUBLICKEYBYTES",
        "SODIUM_CRYPTO_SIGN_PUBLICKEYBYTES",
        sys::crypto_sign_ed25519_pk_to_curve25519,
    )
}

// ---------------------------------------------------------------------------
// stream
// ---------------------------------------------------------------------------

type StreamFn = unsafe extern "C" fn(*mut u8, u64, *const u8, *const u8) -> c_int;
type StreamXorFn = unsafe extern "C" fn(*mut u8, *const u8, u64, *const u8, *const u8) -> c_int;

fn stream(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    pre: &str,
    gen: StreamFn,
) -> Result<Value, Thrown> {
    let len = iarg(host, args, 0, f)?;
    let nonce = sarg(host, args, 1, f)?;
    let key = sarg(host, args, 2, f)?;
    if len <= 0 {
        return Err(arg_err(f, 1, "must be greater than 0"));
    }
    need(f, 2, &nonce, &format!("{pre}_NONCEBYTES"))?;
    need(f, 3, &key, &format!("{pre}_KEYBYTES"))?;
    let mut out = vec![0u8; len as usize];
    ok(unsafe { gen(out.as_mut_ptr(), len as u64, p(&nonce), p(&key)) })?;
    bytes(out)
}

fn stream_xor(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    pre: &str,
    xor: StreamXorFn,
) -> Result<Value, Thrown> {
    let [msg, nonce, key] = nonce_key(
        host,
        args,
        f,
        &format!("{pre}_NONCEBYTES"),
        &format!("{pre}_KEYBYTES"),
    )?;
    let mut out = vec![0u8; msg.len().max(1)];
    ok(unsafe {
        xor(
            out.as_mut_ptr(),
            p(&msg),
            msg.len() as u64,
            p(&nonce),
            p(&key),
        )
    })?;
    out.truncate(msg.len());
    bytes(out)
}

fn fn_stream(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    stream(
        h,
        a,
        "sodium_crypto_stream",
        "SODIUM_CRYPTO_STREAM",
        sys::crypto_stream,
    )
}
fn fn_stream_xor(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    stream_xor(
        h,
        a,
        "sodium_crypto_stream_xor",
        "SODIUM_CRYPTO_STREAM",
        sys::crypto_stream_xor,
    )
}
fn fn_stream_xchacha20(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    stream(
        h,
        a,
        "sodium_crypto_stream_xchacha20",
        "SODIUM_CRYPTO_STREAM_XCHACHA20",
        sys::crypto_stream_xchacha20,
    )
}
fn fn_stream_xchacha20_xor(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    stream_xor(
        h,
        a,
        "sodium_crypto_stream_xchacha20_xor",
        "SODIUM_CRYPTO_STREAM_XCHACHA20",
        sys::crypto_stream_xchacha20_xor,
    )
}

fn fn_stream_xchacha20_xor_ic(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_stream_xchacha20_xor_ic";
    let msg = sarg(host, args, 0, f)?;
    let nonce = sarg(host, args, 1, f)?;
    let ic = iarg(host, args, 2, f)?;
    let key = sarg(host, args, 3, f)?;
    need(f, 2, &nonce, "SODIUM_CRYPTO_STREAM_XCHACHA20_NONCEBYTES")?;
    need(f, 3, &key, "SODIUM_CRYPTO_STREAM_XCHACHA20_KEYBYTES")?;
    let mut out = vec![0u8; msg.len().max(1)];
    ok(unsafe {
        sys::crypto_stream_xchacha20_xor_ic(
            out.as_mut_ptr(),
            p(&msg),
            msg.len() as u64,
            p(&nonce),
            ic as u64,
            p(&key),
        )
    })?;
    out.truncate(msg.len());
    bytes(out)
}

// ---------------------------------------------------------------------------
// pwhash
// ---------------------------------------------------------------------------

fn empty_password(host: &mut dyn Host, pw: &[u8]) -> Result<(), Thrown> {
    if pw.is_empty() {
        host.emit_diagnostic(E_WARNING, b"empty password")?;
    }
    Ok(())
}

fn fn_pwhash(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_pwhash";
    let hash_len = iarg(host, args, 0, f)?;
    let pw = sarg(host, args, 1, f)?;
    let salt = sarg(host, args, 2, f)?;
    let ops = iarg(host, args, 3, f)?;
    let mem = iarg(host, args, 4, f)?;
    let alg_default = table()
        .get("SODIUM_CRYPTO_PWHASH_ALG_DEFAULT")
        .copied()
        .unwrap_or(0);
    let alg = iarg_or(host, args, 5, f, alg_default)?;
    if hash_len <= 0 {
        return Err(arg_err(f, 1, "must be greater than 0"));
    }
    if hash_len >= 0xffff_ffff {
        return Err(arg_err(f, 1, "is too large"));
    }
    if pw.len() >= 0xffff_ffff {
        return Err(arg_err(f, 2, "is too long"));
    }
    if ops <= 0 {
        return Err(arg_err(f, 4, "must be greater than 0"));
    }
    if mem <= 0 {
        return Err(arg_err(f, 5, "must be greater than 0"));
    }
    let i13 = c("SODIUM_CRYPTO_PWHASH_ALG_ARGON2I13") as i64;
    let id13 = table().get("SODIUM_CRYPTO_PWHASH_ALG_ARGON2ID13").copied();
    if alg != i13 && Some(alg) != id13 && alg != alg_default {
        return Err(exc("unsupported password hashing algorithm"));
    }
    empty_password(host, &pw)?;
    need(f, 3, &salt, "SODIUM_CRYPTO_PWHASH_SALTBYTES")?;
    let ops_min = c("crypto_pwhash_OPSLIMIT_MIN") as i64;
    if ops < ops_min {
        return Err(arg_err(
            f,
            4,
            &format!("must be greater than or equal to {ops_min}"),
        ));
    }
    let mem_min = c("crypto_pwhash_MEMLIMIT_MIN") as i64;
    if mem < mem_min {
        return Err(arg_err(
            f,
            5,
            &format!("must be greater than or equal to {mem_min}"),
        ));
    }
    let mut out = vec![0u8; hash_len as usize];
    let call = |g: unsafe extern "C" fn(
        *mut u8,
        u64,
        *const c_char,
        u64,
        *const u8,
        u64,
        usize,
        c_int,
    ) -> c_int,
                out: &mut Vec<u8>| unsafe {
        g(
            out.as_mut_ptr(),
            hash_len as u64,
            cp(&pw),
            pw.len() as u64,
            p(&salt),
            ops as u64,
            mem as usize,
            alg as c_int,
        )
    };
    let mut r = -1;
    if has(feat::ARGON2ID) && Some(alg) == id13 {
        r = call(sys::crypto_pwhash_argon2id, &mut out);
    }
    if r == -1 {
        r = call(sys::crypto_pwhash, &mut out);
    }
    ok(r)?;
    bytes(out)
}

/// The (password, opslimit, memlimit) checks of the `*_str` functions;
/// `argon`: the Argon2 limits (else scrypt's).
fn str_limits(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    argon: bool,
) -> Result<(Vec<u8>, i64, i64), Thrown> {
    let pw = sarg(host, args, 0, f)?;
    let ops = iarg(host, args, 1, f)?;
    let mem = iarg(host, args, 2, f)?;
    if ops <= 0 {
        return Err(arg_err(f, 2, "must be greater than 0"));
    }
    if mem <= 0 {
        return Err(arg_err(f, 3, "must be greater than 0"));
    }
    if argon && pw.len() >= 0xffff_ffff {
        return Err(arg_err(f, 1, "is too long"));
    }
    empty_password(host, &pw)?;
    let (ops_min, mem_min) = if argon {
        (
            c("crypto_pwhash_OPSLIMIT_MIN") as i64,
            c("crypto_pwhash_MEMLIMIT_MIN") as i64,
        )
    } else {
        (
            c("SODIUM_CRYPTO_PWHASH_SCRYPTSALSA208SHA256_OPSLIMIT_INTERACTIVE") as i64,
            c("SODIUM_CRYPTO_PWHASH_SCRYPTSALSA208SHA256_MEMLIMIT_INTERACTIVE") as i64,
        )
    };
    if ops < ops_min {
        return Err(arg_err(
            f,
            2,
            &format!("must be greater than or equal to {ops_min}"),
        ));
    }
    if mem < mem_min {
        return Err(arg_err(
            f,
            3,
            &format!("must be greater than or equal to {mem_min}"),
        ));
    }
    Ok((pw, ops, mem))
}

/// A NUL-terminated hash string out of a `*_str` call.
fn nul_terminated(mut buf: Vec<u8>) -> Vec<u8> {
    if let Some(n) = buf.iter().position(|&b| b == 0) {
        buf.truncate(n);
    }
    buf
}

fn fn_pwhash_str(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_pwhash_str";
    let (pw, ops, mem) = str_limits(host, args, f, true)?;
    let mut out = vec![0u8; c("crypto_pwhash_STRBYTES")];
    ok(unsafe {
        sys::crypto_pwhash_str(
            out.as_mut_ptr().cast(),
            cp(&pw),
            pw.len() as u64,
            ops as u64,
            mem as usize,
        )
    })?;
    bytes(nul_terminated(out))
}

fn fn_pwhash_str_needs_rehash(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_pwhash_str_needs_rehash";
    let hash = cstring(sarg(host, args, 0, f)?);
    let ops = iarg(host, args, 1, f)?;
    let mem = iarg(host, args, 2, f)?;
    let r = unsafe { sys::crypto_pwhash_str_needs_rehash(hash.as_ptr(), ops as u64, mem as usize) };
    Ok(Value::Bool(r != 0))
}

fn fn_pwhash_str_verify(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_pwhash_str_verify";
    let hash = cstring(sarg(host, args, 0, f)?);
    let pw = sarg(host, args, 1, f)?;
    if pw.len() >= 0xffff_ffff {
        return Err(arg_err(f, 2, "is too long"));
    }
    empty_password(host, &pw)?;
    let r = unsafe { sys::crypto_pwhash_str_verify(hash.as_ptr(), cp(&pw), pw.len() as u64) };
    Ok(Value::Bool(r == 0))
}

fn fn_scrypt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_pwhash_scryptsalsa208sha256";
    let hash_len = iarg(host, args, 0, f)?;
    let pw = sarg(host, args, 1, f)?;
    let salt = sarg(host, args, 2, f)?;
    let ops = iarg(host, args, 3, f)?;
    let mem = iarg(host, args, 4, f)?;
    if hash_len <= 0 || hash_len > 0x1f_ffff_ffe0 {
        return Err(arg_err(f, 1, "must be greater than 0"));
    }
    if ops <= 0 {
        return Err(arg_err(f, 4, "must be greater than 0"));
    }
    if mem <= 0 {
        return Err(arg_err(f, 5, "must be greater than 0"));
    }
    empty_password(host, &pw)?;
    need(
        f,
        3,
        &salt,
        "SODIUM_CRYPTO_PWHASH_SCRYPTSALSA208SHA256_SALTBYTES",
    )?;
    let ops_min = c("SODIUM_CRYPTO_PWHASH_SCRYPTSALSA208SHA256_OPSLIMIT_INTERACTIVE") as i64;
    if ops < ops_min {
        return Err(arg_err(
            f,
            4,
            &format!("must be greater than or equal to {ops_min}"),
        ));
    }
    let mem_min = c("SODIUM_CRYPTO_PWHASH_SCRYPTSALSA208SHA256_MEMLIMIT_INTERACTIVE") as i64;
    if mem < mem_min {
        return Err(arg_err(
            f,
            5,
            &format!("must be greater than or equal to {mem_min}"),
        ));
    }
    let mut out = vec![0u8; hash_len as usize];
    ok(unsafe {
        sys::crypto_pwhash_scryptsalsa208sha256(
            out.as_mut_ptr(),
            hash_len as u64,
            cp(&pw),
            pw.len() as u64,
            p(&salt),
            ops as u64,
            mem as usize,
        )
    })?;
    bytes(out)
}

fn fn_scrypt_str(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_pwhash_scryptsalsa208sha256_str";
    let (pw, ops, mem) = str_limits(host, args, f, false)?;
    let mut out = vec![0u8; c("crypto_pwhash_scryptsalsa208sha256_STRBYTES")];
    ok(unsafe {
        sys::crypto_pwhash_scryptsalsa208sha256_str(
            out.as_mut_ptr().cast(),
            cp(&pw),
            pw.len() as u64,
            ops as u64,
            mem as usize,
        )
    })?;
    // php keeps STRBYTES - 1 bytes, the NUL padding included.
    out.pop();
    bytes(out)
}

fn fn_scrypt_str_verify(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_pwhash_scryptsalsa208sha256_str_verify";
    let hash = sarg(host, args, 0, f)?;
    let pw = sarg(host, args, 1, f)?;
    empty_password(host, &pw)?;
    if hash.len() != c("crypto_pwhash_scryptsalsa208sha256_STRBYTES") - 1 {
        host.emit_diagnostic(E_WARNING, b"wrong size for the hashed password")?;
        return Ok(Value::Bool(false));
    }
    let hash = cstring(hash);
    let r = unsafe {
        sys::crypto_pwhash_scryptsalsa208sha256_str_verify(hash.as_ptr(), cp(&pw), pw.len() as u64)
    };
    Ok(Value::Bool(r == 0))
}

// ---------------------------------------------------------------------------
// secretstream
// ---------------------------------------------------------------------------

fn fn_ss_init_push(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_secretstream_xchacha20poly1305_init_push";
    let key = sarg(host, args, 0, f)?;
    need(
        f,
        1,
        &key,
        "SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_KEYBYTES",
    )?;
    let mut st = Aligned::from(&[]);
    let mut header = vec![0u8; c("SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_HEADERBYTES")];
    ok(unsafe {
        sys::crypto_secretstream_xchacha20poly1305_init_push(st.ptr(), header.as_mut_ptr(), p(&key))
    })?;
    let mut out = PhpArray::new();
    out.push(Value::str(st.0[..secretstream_statebytes()].to_vec()));
    out.push(Value::str(header));
    Ok(Value::Array(out))
}

/// The secretstream state behind argument 1, length-checked.
fn ss_state(
    args: &[Value],
    f: &str,
    bad_len: impl FnOnce() -> Thrown,
) -> Result<Box<Aligned>, Thrown> {
    let state = state_arg(args, f)?;
    if state.len() != secretstream_statebytes() {
        return Err(bad_len());
    }
    Ok(Aligned::from(&state))
}

fn ss_store(args: &[Value], st: &Aligned) {
    set_ref(
        args,
        0,
        Value::str(st.0[..secretstream_statebytes()].to_vec()),
    );
}

fn fn_ss_push(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_secretstream_xchacha20poly1305_push";
    let msg = sarg(host, args, 1, f)?;
    let ad = sarg_or(host, args, 2, f, b"")?;
    let tag = iarg_or(
        host,
        args,
        3,
        f,
        c("SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_TAG_MESSAGE") as i64,
    )?;
    let mut st = ss_state(args, f, || arg_err(f, 1, "must have a correct length"))?;
    let abytes = c("SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_ABYTES");
    if msg.len() > c("SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_MESSAGEBYTES_MAX")
        || msg.len() > ZSTR_MAX_LEN - abytes
    {
        return Err(arg_err(
            f,
            2,
            "must be at most SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_MESSAGEBYTES_MAX bytes long",
        ));
    }
    if !(0..=255).contains(&tag) {
        return Err(arg_err(f, 4, "must be in the range of 0-255"));
    }
    let clen = msg.len() + abytes;
    let mut out = vec![0u8; clen];
    let mut real: u64 = 0;
    ok(unsafe {
        sys::crypto_secretstream_xchacha20poly1305_push(
            st.ptr(),
            out.as_mut_ptr(),
            &mut real,
            p(&msg),
            msg.len() as u64,
            p(&ad),
            ad.len() as u64,
            tag as u8,
        )
    })?;
    ss_store(args, &st);
    if real == 0 || real as usize > clen {
        return Err(exc("arithmetic overflow"));
    }
    out.truncate(real as usize);
    bytes(out)
}

fn fn_ss_init_pull(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_secretstream_xchacha20poly1305_init_pull";
    let header = sarg(host, args, 0, f)?;
    let key = sarg(host, args, 1, f)?;
    need(
        f,
        1,
        &header,
        "SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_HEADERBYTES",
    )?;
    need(
        f,
        2,
        &key,
        "SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_KEYBYTES",
    )?;
    let mut st = Aligned::from(&[]);
    ok(unsafe {
        sys::crypto_secretstream_xchacha20poly1305_init_pull(st.ptr(), p(&header), p(&key))
    })?;
    bytes(st.0[..secretstream_statebytes()].to_vec())
}

fn fn_ss_pull(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_secretstream_xchacha20poly1305_pull";
    let ct = sarg(host, args, 1, f)?;
    let ad = sarg_or(host, args, 2, f, b"")?;
    let mut st = ss_state(args, f, || exc("incorrect state length"))?;
    let abytes = c("SODIUM_CRYPTO_SECRETSTREAM_XCHACHA20POLY1305_ABYTES");
    if ct.len() < abytes {
        return Ok(Value::Bool(false));
    }
    let mlen = ct.len() - abytes;
    let mut out = vec![0u8; mlen.max(1)];
    let mut real: u64 = 0;
    let mut tag: u8 = 0;
    let r = unsafe {
        sys::crypto_secretstream_xchacha20poly1305_pull(
            st.ptr(),
            out.as_mut_ptr(),
            &mut real,
            &mut tag,
            p(&ct),
            ct.len() as u64,
            p(&ad),
            ad.len() as u64,
        )
    };
    // php pulls on the string in place: a failed pull leaves the state as
    // libsodium left it.
    ss_store(args, &st);
    if r != 0 {
        return Ok(Value::Bool(false));
    }
    if real as usize > mlen {
        return Err(exc("arithmetic overflow"));
    }
    out.truncate(real as usize);
    let mut a = PhpArray::new();
    a.push(Value::str(out));
    a.push(Value::Int(tag as i64));
    Ok(Value::Array(a))
}

fn fn_ss_rekey(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_secretstream_xchacha20poly1305_rekey";
    let mut st = ss_state(args, f, || exc("incorrect state length"))?;
    unsafe { sys::crypto_secretstream_xchacha20poly1305_rekey(st.ptr()) };
    ss_store(args, &st);
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// ristretto255
// ---------------------------------------------------------------------------

const R_BYTES: &str = "SODIUM_CRYPTO_CORE_RISTRETTO255_BYTES";
const R_SCALAR: &str = "SODIUM_CRYPTO_CORE_RISTRETTO255_SCALARBYTES";

type R2 = unsafe extern "C" fn(*mut u8, *const u8, *const u8) -> c_int;
type R2v = unsafe extern "C" fn(*mut u8, *const u8, *const u8);
type R1 = unsafe extern "C" fn(*mut u8, *const u8) -> c_int;
type R1v = unsafe extern "C" fn(*mut u8, *const u8);

fn r_two(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    in_len: &str,
    run: impl FnOnce(*mut u8, *const u8, *const u8) -> c_int,
) -> Result<Value, Thrown> {
    let a = sarg(host, args, 0, f)?;
    let b = sarg(host, args, 1, f)?;
    need(f, 1, &a, in_len)?;
    need(f, 2, &b, in_len)?;
    let mut r = vec![0u8; c(R_BYTES)];
    ok(run(r.as_mut_ptr(), p(&a), p(&b)))?;
    bytes(r)
}

fn r_one(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    in_len: &str,
    out_len: &str,
    run: impl FnOnce(*mut u8, *const u8) -> c_int,
) -> Result<Value, Thrown> {
    let s = sarg(host, args, 0, f)?;
    need(f, 1, &s, in_len)?;
    let mut r = vec![0u8; c(out_len)];
    ok(run(r.as_mut_ptr(), p(&s)))?;
    bytes(r)
}

fn r2(g: R2) -> impl FnOnce(*mut u8, *const u8, *const u8) -> c_int {
    move |r, a, b| unsafe { g(r, a, b) }
}
fn r2v(g: R2v) -> impl FnOnce(*mut u8, *const u8, *const u8) -> c_int {
    move |r, a, b| {
        unsafe { g(r, a, b) };
        0
    }
}
fn r1(g: R1) -> impl FnOnce(*mut u8, *const u8) -> c_int {
    move |r, a| unsafe { g(r, a) }
}
fn r1v(g: R1v) -> impl FnOnce(*mut u8, *const u8) -> c_int {
    move |r, a| {
        unsafe { g(r, a) };
        0
    }
}

fn fn_r255_add(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    r_two(
        h,
        a,
        "sodium_crypto_core_ristretto255_add",
        R_BYTES,
        r2(sys::crypto_core_ristretto255_add),
    )
}
fn fn_r255_sub(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    r_two(
        h,
        a,
        "sodium_crypto_core_ristretto255_sub",
        R_BYTES,
        r2(sys::crypto_core_ristretto255_sub),
    )
}
fn fn_r255_scalar_add(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    r_two(
        h,
        a,
        "sodium_crypto_core_ristretto255_scalar_add",
        R_SCALAR,
        r2v(sys::crypto_core_ristretto255_scalar_add),
    )
}
fn fn_r255_scalar_sub(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    r_two(
        h,
        a,
        "sodium_crypto_core_ristretto255_scalar_sub",
        R_SCALAR,
        r2v(sys::crypto_core_ristretto255_scalar_sub),
    )
}
fn fn_r255_scalar_mul(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    r_two(
        h,
        a,
        "sodium_crypto_core_ristretto255_scalar_mul",
        R_SCALAR,
        r2v(sys::crypto_core_ristretto255_scalar_mul),
    )
}
fn fn_r255_from_hash(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    r_one(
        h,
        a,
        "sodium_crypto_core_ristretto255_from_hash",
        "SODIUM_CRYPTO_CORE_RISTRETTO255_HASHBYTES",
        R_SCALAR,
        r1(sys::crypto_core_ristretto255_from_hash),
    )
}
fn fn_r255_scalar_complement(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    r_one(
        h,
        a,
        "sodium_crypto_core_ristretto255_scalar_complement",
        R_SCALAR,
        R_SCALAR,
        r1v(sys::crypto_core_ristretto255_scalar_complement),
    )
}
fn fn_r255_scalar_invert(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    r_one(
        h,
        a,
        "sodium_crypto_core_ristretto255_scalar_invert",
        R_SCALAR,
        R_SCALAR,
        r1(sys::crypto_core_ristretto255_scalar_invert),
    )
}
fn fn_r255_scalar_negate(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    r_one(
        h,
        a,
        "sodium_crypto_core_ristretto255_scalar_negate",
        R_SCALAR,
        R_SCALAR,
        r1v(sys::crypto_core_ristretto255_scalar_negate),
    )
}
fn fn_r255_scalar_reduce(h: &mut dyn Host, a: &[Value]) -> Result<Value, Thrown> {
    r_one(
        h,
        a,
        "sodium_crypto_core_ristretto255_scalar_reduce",
        "SODIUM_CRYPTO_CORE_RISTRETTO255_NONREDUCEDSCALARBYTES",
        R_SCALAR,
        r1v(sys::crypto_core_ristretto255_scalar_reduce),
    )
}

fn fn_r255_is_valid_point(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "sodium_crypto_core_ristretto255_is_valid_point";
    let s = sarg(host, args, 0, f)?;
    need(f, 1, &s, R_BYTES)?;
    Ok(Value::Bool(
        unsafe { sys::crypto_core_ristretto255_is_valid_point(p(&s)) } != 0,
    ))
}

fn fn_r255_random(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    let mut r = vec![0u8; c(R_BYTES)];
    unsafe { sys::crypto_core_ristretto255_random(r.as_mut_ptr()) };
    bytes(r)
}

fn fn_r255_scalar_random(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    let mut r = vec![0u8; c(R_SCALAR)];
    unsafe { sys::crypto_core_ristretto255_scalar_random(r.as_mut_ptr()) };
    bytes(r)
}
