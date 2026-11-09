//! The libsodium functions ext/sodium calls (sodium.h), and the constant
//! table of shim.c.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_uchar, c_ulonglong, c_void, CStr};

type U8 = c_uchar;
type Ull = c_ulonglong;

/// An AEAD encrypt function (`crypto_aead_*_encrypt`).
pub type AeadEncrypt = unsafe extern "C" fn(
    c: *mut U8,
    clen: *mut Ull,
    m: *const U8,
    mlen: Ull,
    ad: *const U8,
    adlen: Ull,
    nsec: *const U8,
    npub: *const U8,
    k: *const U8,
) -> c_int;

/// An AEAD decrypt function (`crypto_aead_*_decrypt`).
pub type AeadDecrypt = unsafe extern "C" fn(
    m: *mut U8,
    mlen: *mut Ull,
    nsec: *mut U8,
    c: *const U8,
    clen: Ull,
    ad: *const U8,
    adlen: Ull,
    npub: *const U8,
    k: *const U8,
) -> c_int;

#[repr(C)]
pub struct Const {
    pub name: *const c_char,
    pub value: i64,
}

extern "C" {
    static rhp_sodium_consts: [Const; 0];
    static rhp_sodium_internal: [Const; 0];
    pub fn rhp_sodium_aegis128l_encrypt(
        c: *mut U8,
        clen: *mut Ull,
        m: *const U8,
        mlen: Ull,
        ad: *const U8,
        adlen: Ull,
        nsec: *const U8,
        npub: *const U8,
        k: *const U8,
    ) -> c_int;
    pub fn rhp_sodium_aegis128l_decrypt(
        m: *mut U8,
        mlen: *mut Ull,
        nsec: *mut U8,
        c: *const U8,
        clen: Ull,
        ad: *const U8,
        adlen: Ull,
        npub: *const U8,
        k: *const U8,
    ) -> c_int;
    pub fn rhp_sodium_aegis128l_keygen(k: *mut U8);
    pub fn rhp_sodium_aegis256_encrypt(
        c: *mut U8,
        clen: *mut Ull,
        m: *const U8,
        mlen: Ull,
        ad: *const U8,
        adlen: Ull,
        nsec: *const U8,
        npub: *const U8,
        k: *const U8,
    ) -> c_int;
    pub fn rhp_sodium_aegis256_decrypt(
        m: *mut U8,
        mlen: *mut Ull,
        nsec: *mut U8,
        c: *const U8,
        clen: Ull,
        ad: *const U8,
        adlen: Ull,
        npub: *const U8,
        k: *const U8,
    ) -> c_int;
    pub fn rhp_sodium_aegis256_keygen(k: *mut U8);
    fn rhp_sodium_pwhash_strprefix() -> *const c_char;
    fn rhp_sodium_scrypt_strprefix() -> *const c_char;
    fn rhp_sodium_header_version() -> *const c_char;
    pub fn rhp_sodium_features() -> u32;
    pub fn rhp_sodium_generichash_statebytes() -> usize;
    pub fn rhp_sodium_secretstream_statebytes() -> usize;

    pub fn sodium_init() -> c_int;
    pub fn sodium_version_string() -> *const c_char;
    pub fn sodium_library_version_major() -> c_int;
    pub fn sodium_library_version_minor() -> c_int;
    pub fn randombytes_buf(buf: *mut c_void, size: usize);

    pub fn sodium_memzero(pnt: *mut c_void, len: usize);
    pub fn sodium_increment(n: *mut U8, nlen: usize);
    pub fn sodium_add(a: *mut U8, b: *const U8, len: usize);
    pub fn sodium_memcmp(b1: *const c_void, b2: *const c_void, len: usize) -> c_int;
    pub fn sodium_compare(b1: *const U8, b2: *const U8, len: usize) -> c_int;
    pub fn sodium_bin2hex(
        hex: *mut c_char,
        hex_maxlen: usize,
        bin: *const U8,
        bin_len: usize,
    ) -> *mut c_char;
    pub fn sodium_hex2bin(
        bin: *mut U8,
        bin_maxlen: usize,
        hex: *const c_char,
        hex_len: usize,
        ignore: *const c_char,
        bin_len: *mut usize,
        hex_end: *mut *const c_char,
    ) -> c_int;
    pub fn sodium_bin2base64(
        b64: *mut c_char,
        b64_maxlen: usize,
        bin: *const U8,
        bin_len: usize,
        variant: c_int,
    ) -> *mut c_char;
    pub fn sodium_base642bin(
        bin: *mut U8,
        bin_maxlen: usize,
        b64: *const c_char,
        b64_len: usize,
        ignore: *const c_char,
        bin_len: *mut usize,
        b64_end: *mut *const c_char,
        variant: c_int,
    ) -> c_int;
    pub fn sodium_base64_encoded_len(bin_len: usize, variant: c_int) -> usize;
    pub fn sodium_pad(
        padded_buflen_p: *mut usize,
        buf: *mut U8,
        unpadded_buflen: usize,
        blocksize: usize,
        max_buflen: usize,
    ) -> c_int;
    pub fn sodium_unpad(
        unpadded_buflen_p: *mut usize,
        buf: *const U8,
        padded_buflen: usize,
        blocksize: usize,
    ) -> c_int;

    pub fn crypto_shorthash(out: *mut U8, m: *const U8, mlen: Ull, k: *const U8) -> c_int;

    pub fn crypto_secretbox_easy(
        c: *mut U8,
        m: *const U8,
        mlen: Ull,
        n: *const U8,
        k: *const U8,
    ) -> c_int;
    pub fn crypto_secretbox_open_easy(
        m: *mut U8,
        c: *const U8,
        clen: Ull,
        n: *const U8,
        k: *const U8,
    ) -> c_int;

    pub fn crypto_generichash(
        out: *mut U8,
        outlen: usize,
        m: *const U8,
        mlen: Ull,
        key: *const U8,
        keylen: usize,
    ) -> c_int;
    pub fn crypto_generichash_init(
        state: *mut c_void,
        key: *const U8,
        keylen: usize,
        outlen: usize,
    ) -> c_int;
    pub fn crypto_generichash_update(state: *mut c_void, m: *const U8, mlen: Ull) -> c_int;
    pub fn crypto_generichash_final(state: *mut c_void, out: *mut U8, outlen: usize) -> c_int;
    pub fn crypto_generichash_blake2b_salt_personal(
        out: *mut U8,
        outlen: usize,
        m: *const U8,
        mlen: Ull,
        key: *const U8,
        keylen: usize,
        salt: *const U8,
        personal: *const U8,
    ) -> c_int;
    pub fn crypto_kdf_derive_from_key(
        subkey: *mut U8,
        subkey_len: usize,
        subkey_id: u64,
        ctx: *const c_char,
        key: *const U8,
    ) -> c_int;

    pub fn crypto_box_keypair(pk: *mut U8, sk: *mut U8) -> c_int;
    pub fn crypto_box_seed_keypair(pk: *mut U8, sk: *mut U8, seed: *const U8) -> c_int;
    pub fn crypto_box_easy(
        c: *mut U8,
        m: *const U8,
        mlen: Ull,
        n: *const U8,
        pk: *const U8,
        sk: *const U8,
    ) -> c_int;
    pub fn crypto_box_open_easy(
        m: *mut U8,
        c: *const U8,
        clen: Ull,
        n: *const U8,
        pk: *const U8,
        sk: *const U8,
    ) -> c_int;
    pub fn crypto_box_seal(c: *mut U8, m: *const U8, mlen: Ull, pk: *const U8) -> c_int;
    pub fn crypto_box_seal_open(
        m: *mut U8,
        c: *const U8,
        clen: Ull,
        pk: *const U8,
        sk: *const U8,
    ) -> c_int;

    pub fn crypto_scalarmult_base(q: *mut U8, n: *const U8) -> c_int;
    pub fn crypto_scalarmult(q: *mut U8, n: *const U8, p: *const U8) -> c_int;
    pub fn crypto_scalarmult_ristretto255(q: *mut U8, n: *const U8, p: *const U8) -> c_int;
    pub fn crypto_scalarmult_ristretto255_base(q: *mut U8, n: *const U8) -> c_int;

    pub fn crypto_sign_keypair(pk: *mut U8, sk: *mut U8) -> c_int;
    pub fn crypto_sign_seed_keypair(pk: *mut U8, sk: *mut U8, seed: *const U8) -> c_int;
    pub fn crypto_sign_ed25519_sk_to_pk(pk: *mut U8, sk: *const U8) -> c_int;
    pub fn crypto_sign(
        sm: *mut U8,
        smlen: *mut Ull,
        m: *const U8,
        mlen: Ull,
        sk: *const U8,
    ) -> c_int;
    pub fn crypto_sign_open(
        m: *mut U8,
        mlen: *mut Ull,
        sm: *const U8,
        smlen: Ull,
        pk: *const U8,
    ) -> c_int;
    pub fn crypto_sign_detached(
        sig: *mut U8,
        siglen: *mut Ull,
        m: *const U8,
        mlen: Ull,
        sk: *const U8,
    ) -> c_int;
    pub fn crypto_sign_verify_detached(
        sig: *const U8,
        m: *const U8,
        mlen: Ull,
        pk: *const U8,
    ) -> c_int;
    pub fn crypto_sign_ed25519_sk_to_curve25519(curve: *mut U8, ed: *const U8) -> c_int;
    pub fn crypto_sign_ed25519_pk_to_curve25519(curve: *mut U8, ed: *const U8) -> c_int;

    pub fn crypto_stream(c: *mut U8, clen: Ull, n: *const U8, k: *const U8) -> c_int;
    pub fn crypto_stream_xor(
        c: *mut U8,
        m: *const U8,
        mlen: Ull,
        n: *const U8,
        k: *const U8,
    ) -> c_int;
    pub fn crypto_stream_xchacha20(c: *mut U8, clen: Ull, n: *const U8, k: *const U8) -> c_int;
    pub fn crypto_stream_xchacha20_xor(
        c: *mut U8,
        m: *const U8,
        mlen: Ull,
        n: *const U8,
        k: *const U8,
    ) -> c_int;
    pub fn crypto_stream_xchacha20_xor_ic(
        c: *mut U8,
        m: *const U8,
        mlen: Ull,
        n: *const U8,
        ic: u64,
        k: *const U8,
    ) -> c_int;

    pub fn crypto_pwhash(
        out: *mut U8,
        outlen: Ull,
        passwd: *const c_char,
        passwdlen: Ull,
        salt: *const U8,
        opslimit: Ull,
        memlimit: usize,
        alg: c_int,
    ) -> c_int;
    pub fn crypto_pwhash_argon2id(
        out: *mut U8,
        outlen: Ull,
        passwd: *const c_char,
        passwdlen: Ull,
        salt: *const U8,
        opslimit: Ull,
        memlimit: usize,
        alg: c_int,
    ) -> c_int;
    pub fn crypto_pwhash_str(
        out: *mut c_char,
        passwd: *const c_char,
        passwdlen: Ull,
        opslimit: Ull,
        memlimit: usize,
    ) -> c_int;
    pub fn crypto_pwhash_str_verify(s: *const c_char, passwd: *const c_char, len: Ull) -> c_int;
    pub fn crypto_pwhash_str_needs_rehash(
        s: *const c_char,
        opslimit: Ull,
        memlimit: usize,
    ) -> c_int;
    pub fn crypto_pwhash_scryptsalsa208sha256(
        out: *mut U8,
        outlen: Ull,
        passwd: *const c_char,
        passwdlen: Ull,
        salt: *const U8,
        opslimit: Ull,
        memlimit: usize,
    ) -> c_int;
    pub fn crypto_pwhash_scryptsalsa208sha256_str(
        out: *mut c_char,
        passwd: *const c_char,
        passwdlen: Ull,
        opslimit: Ull,
        memlimit: usize,
    ) -> c_int;
    pub fn crypto_pwhash_scryptsalsa208sha256_str_verify(
        s: *const c_char,
        passwd: *const c_char,
        len: Ull,
    ) -> c_int;

    pub fn crypto_aead_aes256gcm_is_available() -> c_int;
    pub fn crypto_aead_aes256gcm_encrypt(
        c: *mut U8,
        clen: *mut Ull,
        m: *const U8,
        mlen: Ull,
        ad: *const U8,
        adlen: Ull,
        nsec: *const U8,
        npub: *const U8,
        k: *const U8,
    ) -> c_int;
    pub fn crypto_aead_aes256gcm_decrypt(
        m: *mut U8,
        mlen: *mut Ull,
        nsec: *mut U8,
        c: *const U8,
        clen: Ull,
        ad: *const U8,
        adlen: Ull,
        npub: *const U8,
        k: *const U8,
    ) -> c_int;
    pub fn crypto_aead_chacha20poly1305_encrypt(
        c: *mut U8,
        clen: *mut Ull,
        m: *const U8,
        mlen: Ull,
        ad: *const U8,
        adlen: Ull,
        nsec: *const U8,
        npub: *const U8,
        k: *const U8,
    ) -> c_int;
    pub fn crypto_aead_chacha20poly1305_decrypt(
        m: *mut U8,
        mlen: *mut Ull,
        nsec: *mut U8,
        c: *const U8,
        clen: Ull,
        ad: *const U8,
        adlen: Ull,
        npub: *const U8,
        k: *const U8,
    ) -> c_int;
    pub fn crypto_aead_chacha20poly1305_ietf_encrypt(
        c: *mut U8,
        clen: *mut Ull,
        m: *const U8,
        mlen: Ull,
        ad: *const U8,
        adlen: Ull,
        nsec: *const U8,
        npub: *const U8,
        k: *const U8,
    ) -> c_int;
    pub fn crypto_aead_chacha20poly1305_ietf_decrypt(
        m: *mut U8,
        mlen: *mut Ull,
        nsec: *mut U8,
        c: *const U8,
        clen: Ull,
        ad: *const U8,
        adlen: Ull,
        npub: *const U8,
        k: *const U8,
    ) -> c_int;
    pub fn crypto_aead_xchacha20poly1305_ietf_encrypt(
        c: *mut U8,
        clen: *mut Ull,
        m: *const U8,
        mlen: Ull,
        ad: *const U8,
        adlen: Ull,
        nsec: *const U8,
        npub: *const U8,
        k: *const U8,
    ) -> c_int;
    pub fn crypto_aead_xchacha20poly1305_ietf_decrypt(
        m: *mut U8,
        mlen: *mut Ull,
        nsec: *mut U8,
        c: *const U8,
        clen: Ull,
        ad: *const U8,
        adlen: Ull,
        npub: *const U8,
        k: *const U8,
    ) -> c_int;

    pub fn crypto_auth(out: *mut U8, m: *const U8, mlen: Ull, k: *const U8) -> c_int;
    pub fn crypto_auth_verify(h: *const U8, m: *const U8, mlen: Ull, k: *const U8) -> c_int;

    pub fn crypto_secretstream_xchacha20poly1305_init_push(
        state: *mut c_void,
        header: *mut U8,
        k: *const U8,
    ) -> c_int;
    pub fn crypto_secretstream_xchacha20poly1305_push(
        state: *mut c_void,
        c: *mut U8,
        clen_p: *mut Ull,
        m: *const U8,
        mlen: Ull,
        ad: *const U8,
        adlen: Ull,
        tag: U8,
    ) -> c_int;
    pub fn crypto_secretstream_xchacha20poly1305_init_pull(
        state: *mut c_void,
        header: *const U8,
        k: *const U8,
    ) -> c_int;
    pub fn crypto_secretstream_xchacha20poly1305_pull(
        state: *mut c_void,
        m: *mut U8,
        mlen_p: *mut Ull,
        tag_p: *mut U8,
        c: *const U8,
        clen: Ull,
        ad: *const U8,
        adlen: Ull,
    ) -> c_int;
    pub fn crypto_secretstream_xchacha20poly1305_rekey(state: *mut c_void);

    pub fn crypto_core_ristretto255_add(r: *mut U8, p: *const U8, q: *const U8) -> c_int;
    pub fn crypto_core_ristretto255_sub(r: *mut U8, p: *const U8, q: *const U8) -> c_int;
    pub fn crypto_core_ristretto255_from_hash(p: *mut U8, r: *const U8) -> c_int;
    pub fn crypto_core_ristretto255_is_valid_point(p: *const U8) -> c_int;
    pub fn crypto_core_ristretto255_random(p: *mut U8);
    pub fn crypto_core_ristretto255_scalar_random(r: *mut U8);
    pub fn crypto_core_ristretto255_scalar_invert(recip: *mut U8, s: *const U8) -> c_int;
    pub fn crypto_core_ristretto255_scalar_negate(neg: *mut U8, s: *const U8);
    pub fn crypto_core_ristretto255_scalar_complement(comp: *mut U8, s: *const U8);
    pub fn crypto_core_ristretto255_scalar_add(z: *mut U8, x: *const U8, y: *const U8);
    pub fn crypto_core_ristretto255_scalar_sub(z: *mut U8, x: *const U8, y: *const U8);
    pub fn crypto_core_ristretto255_scalar_mul(z: *mut U8, x: *const U8, y: *const U8);
    pub fn crypto_core_ristretto255_scalar_reduce(r: *mut U8, s: *const U8);
}

fn cstr(p: *const c_char) -> &'static [u8] {
    unsafe { CStr::from_ptr(p) }.to_bytes()
}

/// shim.c's constant table: (name, value), `None` for a string constant.
pub fn consts() -> Vec<(&'static str, Option<i64>)> {
    table(std::ptr::addr_of!(rhp_sodium_consts) as *const Const)
}

/// shim.c's internal limits (php's macros that are no constants).
pub fn internal() -> Vec<(&'static str, Option<i64>)> {
    table(std::ptr::addr_of!(rhp_sodium_internal) as *const Const)
}

fn table(base: *const Const) -> Vec<(&'static str, Option<i64>)> {
    let mut out = Vec::new();
    for i in 0.. {
        let c = unsafe { &*base.add(i) };
        if c.name.is_null() {
            break;
        }
        let name = std::str::from_utf8(cstr(c.name)).unwrap_or("");
        out.push((name, (c.value != -1).then_some(c.value)));
    }
    out
}

pub fn pwhash_strprefix() -> &'static [u8] {
    cstr(unsafe { rhp_sodium_pwhash_strprefix() })
}

pub fn scrypt_strprefix() -> &'static [u8] {
    cstr(unsafe { rhp_sodium_scrypt_strprefix() })
}

pub fn header_version() -> &'static [u8] {
    cstr(unsafe { rhp_sodium_header_version() })
}

pub fn library_version() -> &'static [u8] {
    cstr(unsafe { sodium_version_string() })
}
