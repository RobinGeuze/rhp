//! The OpenSSL API ext/openssl calls: openssl-sys, plus the declarations it
//! lacks (no bindgen). Struct layouts declared here are the public ones of
//! OpenSSL 3 and 4 (`OBJ_NAME`, `CONF_VALUE`, the head of `X509_INFO`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code
)]

use std::ffi::{c_char, c_int, c_long, c_uchar, c_uint, c_ulong, c_void};

pub use openssl_sys::*;

#[repr(C)]
pub struct OBJ_NAME {
    pub type_: c_int,
    pub alias: c_int,
    pub name: *const c_char,
    pub data: *const c_char,
}

#[repr(C)]
pub struct CONF_VALUE {
    pub section: *mut c_char,
    pub name: *mut c_char,
    pub value: *mut c_char,
}

/// The leading fields of `X509_INFO`; only allocated by OpenSSL.
#[repr(C)]
pub struct X509_INFO {
    pub x509: *mut X509,
}

pub enum NETSCAPE_SPKI {}
pub enum X509_PUBKEY {}
pub enum ASN1_PCTX {}
pub enum OPENSSL_STACK {}

pub const OBJ_NAME_TYPE_MD_METH: c_int = 1;
pub const OBJ_NAME_TYPE_CIPHER_METH: c_int = 2;

pub const CONF_MFLAGS_IGNORE_RETURN_CODES: c_ulong = 0x2;
pub const CONF_MFLAGS_IGNORE_MISSING_FILE: c_ulong = 0x10;
pub const CONF_MFLAGS_DEFAULT_SECTION: c_ulong = 0x20;

pub const XN_FLAG_COMPAT: c_ulong = 0;

pub const X509_FILETYPE_PEM: c_int = 1;
pub const X509_FILETYPE_DEFAULT: c_int = 3;
pub const X509_L_FILE_LOAD: c_int = 1;
pub const X509_L_ADD_DIR: c_int = 2;

pub const EVP_CIPH_GCM_MODE: c_int = 0x6;
pub const EVP_CIPH_CCM_MODE: c_int = 0x7;
pub const EVP_CIPH_WRAP_MODE: c_int = 0x10002;
pub const EVP_CIPH_OCB_MODE: c_int = 0x10003;
pub const EVP_CIPH_MODE: c_ulong = 0xF0007;
pub const EVP_CTRL_AEAD_SET_IVLEN: c_int = 0x9;
pub const EVP_CTRL_AEAD_GET_TAG: c_int = 0x10;
pub const EVP_CTRL_AEAD_SET_TAG: c_int = 0x11;
pub const NID_chacha20_poly1305: c_int = 1018;
pub const NID_sm2: c_int = 1172;
pub const EVP_PKEY_KEYMGMT: c_int = -1;
pub const EVP_PKEY_KEYPAIR: c_int = 0x87;
pub const OPENSSL_EC_NAMED_CURVE: c_int = 0x001;
pub const POINT_CONVERSION_COMPRESSED: c_int = 2;
pub const BN_FLG_CONSTTIME: c_int = 0x04;
pub const OPENSSL_VERSION: c_int = 0;
pub const EVP_MAX_IV_LENGTH: usize = 16;
pub const OPENSSL_INIT_LOAD_CONFIG: u64 = 0x40;
pub const PKCS7_BINARY: c_int = 0x80;
pub const MBSTRING_UTF8: c_int = 0x1000;
pub const RSA_PKCS1_PADDING: c_int = 1;
pub const RSA_NO_PADDING: c_int = 3;
pub const RSA_PKCS1_OAEP_PADDING: c_int = 4;
pub const RSA_PKCS1_PSS_PADDING: c_int = 6;
pub const GEN_EMAIL: c_int = 1;
pub const GEN_DNS: c_int = 2;
pub const GEN_URI: c_int = 6;

pub type OBJ_NAME_cb = unsafe extern "C" fn(*const OBJ_NAME, *mut c_void);

extern "C" {
    pub fn BIO_new_file(filename: *const c_char, mode: *const c_char) -> *mut BIO;
    pub fn BIO_free(a: *mut BIO) -> c_int;
    pub fn BIO_puts(bp: *mut BIO, buf: *const c_char) -> c_int;
    pub fn a2i_ASN1_INTEGER(
        bp: *mut BIO,
        bs: *mut ASN1_INTEGER,
        buf: *mut c_char,
        size: c_int,
    ) -> c_int;
    pub fn d2i_PKCS12_bio(bp: *mut BIO, p12: *mut *mut PKCS12) -> *mut PKCS12;
    pub fn OpenSSL_version(t: c_int) -> *const c_char;
    pub fn EVP_VerifyFinal(
        ctx: *mut EVP_MD_CTX,
        sigbuf: *const c_uchar,
        siglen: c_uint,
        pkey: *mut EVP_PKEY,
    ) -> c_int;
    pub fn ERR_error_string_n(e: c_ulong, buf: *mut c_char, len: usize);
    pub fn ERR_set_mark() -> c_int;
    pub fn ERR_pop_to_mark() -> c_int;

    pub fn OSSL_LIB_CTX_get0_global_default() -> *mut OSSL_LIB_CTX;
    pub fn CONF_modules_load_file_ex(
        libctx: *mut OSSL_LIB_CTX,
        filename: *const c_char,
        appname: *const c_char,
        flags: c_ulong,
    ) -> c_int;

    pub fn OBJ_NAME_do_all_sorted(type_: c_int, f: OBJ_NAME_cb, arg: *mut c_void);
    pub fn OBJ_NAME_get(name: *const c_char, type_: c_int) -> *const c_char;
    pub fn OBJ_txt2nid(s: *const c_char) -> c_int;
    pub fn OBJ_ln2nid(s: *const c_char) -> c_int;
    pub fn EVP_PKEY_type(type_: c_int) -> c_int;
    pub fn EVP_CIPHER_CTX_reset(ctx: *mut EVP_CIPHER_CTX) -> c_int;
    pub fn X509_PUBKEY_get(key: *const X509_PUBKEY) -> *mut EVP_PKEY;
    pub fn OBJ_create_objects(in_: *mut BIO) -> c_int;
    pub fn EVP_CIPHER_do_all_provided(
        libctx: *mut OSSL_LIB_CTX,
        f: unsafe extern "C" fn(*mut EVP_CIPHER, *mut c_void),
        arg: *mut c_void,
    );
    pub fn EVP_CIPHER_names_do_all(
        cipher: *const EVP_CIPHER,
        f: unsafe extern "C" fn(*const c_char, *mut c_void),
        data: *mut c_void,
    ) -> c_int;
    pub fn EVP_CIPHER_get0_name(cipher: *const EVP_CIPHER) -> *const c_char;
    pub fn EVP_CIPHER_get_mode(cipher: *const EVP_CIPHER) -> c_int;
    pub fn EVP_CipherFinal_ex(
        ctx: *mut EVP_CIPHER_CTX,
        out: *mut c_uchar,
        outl: *mut c_int,
    ) -> c_int;
    pub fn EVP_Digest(
        data: *const c_void,
        count: usize,
        md: *mut c_uchar,
        size: *mut c_uint,
        type_: *const EVP_MD,
        impl_: *mut ENGINE,
    ) -> c_int;

    pub fn PEM_read_bio_PrivateKey_ex(
        bp: *mut BIO,
        x: *mut *mut EVP_PKEY,
        cb: pem_password_cb,
        u: *mut c_void,
        libctx: *mut OSSL_LIB_CTX,
        propq: *const c_char,
    ) -> *mut EVP_PKEY;
    pub fn PEM_read_bio_PUBKEY_ex(
        bp: *mut BIO,
        x: *mut *mut EVP_PKEY,
        cb: pem_password_cb,
        u: *mut c_void,
        libctx: *mut OSSL_LIB_CTX,
        propq: *const c_char,
    ) -> *mut EVP_PKEY;
    pub fn PEM_ASN1_read_bio(
        d2i: unsafe extern "C" fn(*mut *mut c_void, *mut *const c_uchar, c_long) -> *mut c_void,
        name: *const c_char,
        bp: *mut BIO,
        x: *mut *mut c_void,
        cb: pem_password_cb,
        u: *mut c_void,
    ) -> *mut c_void;
    pub fn PEM_X509_INFO_read_bio_ex(
        bp: *mut BIO,
        sk: *mut OPENSSL_STACK,
        cb: pem_password_cb,
        u: *mut c_void,
        libctx: *mut OSSL_LIB_CTX,
        propq: *const c_char,
    ) -> *mut OPENSSL_STACK;
    pub fn PEM_write_bio_PrivateKey_traditional(
        bp: *mut BIO,
        x: *const EVP_PKEY,
        enc: *const EVP_CIPHER,
        kstr: *const c_uchar,
        klen: c_int,
        cb: pem_password_cb,
        u: *mut c_void,
    ) -> c_int;
    pub fn X509_INFO_free(a: *mut X509_INFO);

    pub fn X509_new_ex(libctx: *mut OSSL_LIB_CTX, propq: *const c_char) -> *mut X509;
    pub fn X509_REQ_new_ex(libctx: *mut OSSL_LIB_CTX, propq: *const c_char) -> *mut X509_REQ;
    pub fn X509_check_purpose(x: *mut X509, id: c_int, ca: c_int) -> c_int;
    pub fn X509_PURPOSE_get_count() -> c_int;
    pub fn X509_PURPOSE_get0(idx: c_int) -> *mut X509_PURPOSE;
    pub fn X509_PURPOSE_get_id(xp: *const X509_PURPOSE) -> c_int;
    pub fn X509_PURPOSE_get0_name(xp: *const X509_PURPOSE) -> *mut c_char;
    pub fn X509_PURPOSE_get0_sname(xp: *const X509_PURPOSE) -> *mut c_char;
    pub fn X509_NAME_oneline(a: *const X509_NAME, buf: *mut c_char, size: c_int) -> *mut c_char;
    pub fn X509_NAME_print_ex(
        out: *mut BIO,
        nm: *const X509_NAME,
        indent: c_int,
        flags: c_ulong,
    ) -> c_int;
    pub fn X509_NAME_hash_ex(
        x: *const X509_NAME,
        libctx: *mut OSSL_LIB_CTX,
        propq: *const c_char,
        ok: *mut c_int,
    ) -> c_ulong;
    pub fn X509_get0_notBefore(x: *const X509) -> *const ASN1_TIME;
    pub fn X509_get0_notAfter(x: *const X509) -> *const ASN1_TIME;
    pub fn X509_check_private_key(x: *const X509, k: *const EVP_PKEY) -> c_int;
    pub fn X509_REQ_check_private_key(x: *const X509_REQ, k: *const EVP_PKEY) -> c_int;
    pub fn X509_get_default_cert_area() -> *const c_char;
    pub fn X509_get_default_cert_dir() -> *const c_char;
    pub fn X509_get_default_cert_file() -> *const c_char;
    pub fn X509_get_default_cert_dir_env() -> *const c_char;
    pub fn X509_get_default_cert_file_env() -> *const c_char;
    pub fn X509_get_default_private_dir() -> *const c_char;
    pub fn X509_STORE_CTX_set_purpose(ctx: *mut X509_STORE_CTX, purpose: c_int) -> c_int;
    pub fn X509_LOOKUP_file() -> *mut X509_LOOKUP_METHOD;
    pub fn X509_LOOKUP_hash_dir() -> *mut X509_LOOKUP_METHOD;
    pub fn X509_LOOKUP_ctrl(
        ctx: *mut X509_LOOKUP,
        cmd: c_int,
        argc: *const c_char,
        argl: c_long,
        ret: *mut *mut c_char,
    ) -> c_int;
    pub fn X509_time_adj_ex(
        s: *mut ASN1_TIME,
        offset_day: c_int,
        offset_sec: c_long,
        t: *mut libc::time_t,
    ) -> *mut ASN1_TIME;
    pub fn X509_REQ_print(bp: *mut BIO, req: *mut X509_REQ) -> c_int;
    pub fn X509_REQ_add1_attr_by_NID(
        req: *mut X509_REQ,
        nid: c_int,
        type_: c_int,
        bytes: *const c_uchar,
        len: c_int,
    ) -> c_int;
    pub fn X509_REQ_add1_attr_by_txt(
        req: *mut X509_REQ,
        attrname: *const c_char,
        type_: c_int,
        bytes: *const c_uchar,
        len: c_int,
    ) -> c_int;
    pub fn X509_REQ_get_attr_by_NID(req: *const X509_REQ, nid: c_int, lastpos: c_int) -> c_int;
    pub fn X509_NAME_add_entry_by_txt(
        name: *mut X509_NAME,
        field: *const c_char,
        type_: c_int,
        bytes: *const c_uchar,
        len: c_int,
        loc: c_int,
        set: c_int,
    ) -> c_int;
    pub fn X509V3_EXT_d2i(ext: *mut X509_EXTENSION) -> *mut c_void;
    pub fn X509V3_EXT_add_nconf(
        conf: *mut CONF,
        ctx: *mut X509V3_CTX,
        section: *const c_char,
        cert: *mut X509,
    ) -> c_int;
    pub fn X509V3_EXT_REQ_add_nconf(
        conf: *mut CONF,
        ctx: *mut X509V3_CTX,
        section: *const c_char,
        req: *mut X509_REQ,
    ) -> c_int;
    pub fn X509V3_set_nconf(ctx: *mut X509V3_CTX, conf: *mut CONF);
    pub fn GENERAL_NAME_print(out: *mut BIO, gen: *mut GENERAL_NAME) -> c_int;

    pub fn NCONF_new_ex(libctx: *mut OSSL_LIB_CTX, meth: *mut c_void) -> *mut CONF;
    pub fn NCONF_free(conf: *mut CONF);
    pub fn NCONF_load(conf: *mut CONF, file: *const c_char, eline: *mut c_long) -> c_int;
    pub fn NCONF_get_string(
        conf: *const CONF,
        group: *const c_char,
        name: *const c_char,
    ) -> *mut c_char;
    pub fn NCONF_get_number_e(
        conf: *const CONF,
        group: *const c_char,
        name: *const c_char,
        result: *mut c_long,
    ) -> c_int;
    pub fn NCONF_get_section(conf: *const CONF, section: *const c_char) -> *mut OPENSSL_STACK;
    pub fn ASN1_STRING_set_default_mask_asc(p: *const c_char) -> c_int;
    pub fn ASN1_INTEGER_set_int64(a: *mut ASN1_INTEGER, r: i64) -> c_int;
    pub fn s2i_ASN1_INTEGER(meth: *mut c_void, value: *const c_char) -> *mut ASN1_INTEGER;
    pub fn i2s_ASN1_INTEGER(meth: *mut c_void, a: *const ASN1_INTEGER) -> *mut c_char;

    pub fn RAND_file_name(buf: *mut c_char, num: usize) -> *const c_char;
    pub fn RAND_load_file(file: *const c_char, max_bytes: c_long) -> c_int;
    pub fn RAND_write_file(file: *const c_char) -> c_int;
    pub fn RAND_status() -> c_int;

    pub fn EVP_PKEY_get_base_id(pkey: *const EVP_PKEY) -> c_int;
    pub fn EVP_PKEY_get0_type_name(key: *const EVP_PKEY) -> *const c_char;
    pub fn EVP_PKEY_CTX_new_from_pkey(
        libctx: *mut OSSL_LIB_CTX,
        pkey: *mut EVP_PKEY,
        propquery: *const c_char,
    ) -> *mut EVP_PKEY_CTX;
    pub fn EVP_PKEY_CTX_set_dsa_paramgen_bits(ctx: *mut EVP_PKEY_CTX, nbits: c_int) -> c_int;
    pub fn EVP_PKEY_CTX_set_dh_paramgen_prime_len(ctx: *mut EVP_PKEY_CTX, len: c_int) -> c_int;
    pub fn EVP_PKEY_CTX_set_ec_param_enc(ctx: *mut EVP_PKEY_CTX, param_enc: c_int) -> c_int;
    pub fn EVP_PKEY_set1_encoded_public_key(
        pkey: *mut EVP_PKEY,
        pub_: *const c_uchar,
        publen: usize,
    ) -> c_int;
    pub fn EVP_PKEY_check(ctx: *mut EVP_PKEY_CTX) -> c_int;
    pub fn EVP_PKEY_public_check_quick(ctx: *mut EVP_PKEY_CTX) -> c_int;
    pub fn EVP_PKEY_CTX_set_params(ctx: *mut EVP_PKEY_CTX, params: *const OSSL_PARAM) -> c_int;
    pub fn EVP_DigestSignInit_ex(
        ctx: *mut EVP_MD_CTX,
        pctx: *mut *mut EVP_PKEY_CTX,
        mdname: *const c_char,
        libctx: *mut OSSL_LIB_CTX,
        props: *const c_char,
        pkey: *mut EVP_PKEY,
        params: *const OSSL_PARAM,
    ) -> c_int;
    pub fn EVP_DigestVerifyInit_ex(
        ctx: *mut EVP_MD_CTX,
        pctx: *mut *mut EVP_PKEY_CTX,
        mdname: *const c_char,
        libctx: *mut OSSL_LIB_CTX,
        props: *const c_char,
        pkey: *mut EVP_PKEY,
        params: *const OSSL_PARAM,
    ) -> c_int;
    pub fn EVP_MD_get0_name(md: *const EVP_MD) -> *const c_char;
    pub fn OSSL_PARAM_BLD_push_utf8_string(
        bld: *mut OSSL_PARAM_BLD,
        key: *const c_char,
        buf: *const c_char,
        bsize: usize,
    ) -> c_int;
    pub fn OSSL_PARAM_BLD_push_octet_string(
        bld: *mut OSSL_PARAM_BLD,
        key: *const c_char,
        buf: *const c_void,
        bsize: usize,
    ) -> c_int;
    pub fn EC_GROUP_new_by_curve_name_ex(
        libctx: *mut OSSL_LIB_CTX,
        propq: *const c_char,
        nid: c_int,
    ) -> *mut EC_GROUP;
    pub fn EC_GROUP_set_seed(group: *mut EC_GROUP, seed: *const c_uchar, len: usize) -> usize;
    pub fn EC_GROUP_check_named_curve(
        group: *const EC_GROUP,
        nist_only: c_int,
        ctx: *mut BN_CTX,
    ) -> c_int;
    pub fn EC_POINT_point2buf(
        group: *const EC_GROUP,
        point: *const EC_POINT,
        form: c_int,
        pbuf: *mut *mut c_uchar,
        ctx: *mut BN_CTX,
    ) -> usize;
    pub fn EC_POINT_set_affine_coordinates(
        group: *const EC_GROUP,
        p: *mut EC_POINT,
        x: *const BIGNUM,
        y: *const BIGNUM,
        ctx: *mut BN_CTX,
    ) -> c_int;
    pub fn BN_mod_exp_mont(
        r: *mut BIGNUM,
        a: *const BIGNUM,
        p: *const BIGNUM,
        m: *const BIGNUM,
        ctx: *mut BN_CTX,
        m_ctx: *mut c_void,
    ) -> c_int;
    pub fn BN_with_flags(dest: *mut BIGNUM, b: *const BIGNUM, flags: c_int);
    pub fn EVP_SealInit(
        ctx: *mut EVP_CIPHER_CTX,
        type_: *const EVP_CIPHER,
        ek: *mut *mut c_uchar,
        ekl: *mut c_int,
        iv: *mut c_uchar,
        pubk: *mut *mut EVP_PKEY,
        npubk: c_int,
    ) -> c_int;
    pub fn EVP_OpenInit(
        ctx: *mut EVP_CIPHER_CTX,
        type_: *const EVP_CIPHER,
        ek: *const c_uchar,
        ekl: c_int,
        iv: *const c_uchar,
        priv_: *mut EVP_PKEY,
    ) -> c_int;

    pub fn NETSCAPE_SPKI_new() -> *mut NETSCAPE_SPKI;
    pub fn NETSCAPE_SPKI_free(a: *mut NETSCAPE_SPKI);
    pub fn NETSCAPE_SPKI_b64_decode(str: *const c_char, len: c_int) -> *mut NETSCAPE_SPKI;
    pub fn NETSCAPE_SPKI_b64_encode(x: *mut NETSCAPE_SPKI) -> *mut c_char;
    pub fn NETSCAPE_SPKI_set_pubkey(x: *mut NETSCAPE_SPKI, pkey: *mut EVP_PKEY) -> c_int;
    pub fn NETSCAPE_SPKI_get_pubkey(x: *const NETSCAPE_SPKI) -> *mut EVP_PKEY;
    pub fn NETSCAPE_SPKI_sign(
        x: *mut NETSCAPE_SPKI,
        pkey: *mut EVP_PKEY,
        md: *const EVP_MD,
    ) -> c_int;
    pub fn NETSCAPE_SPKI_verify(a: *mut NETSCAPE_SPKI, r: *mut EVP_PKEY) -> c_int;

    pub fn OPENSSL_sk_new_null() -> *mut OPENSSL_STACK;
    pub fn OPENSSL_sk_new_reserve(c: *mut c_void, n: c_int) -> *mut OPENSSL_STACK;
    pub fn OPENSSL_sk_num(st: *const OPENSSL_STACK) -> c_int;
    pub fn OPENSSL_sk_value(st: *const OPENSSL_STACK, i: c_int) -> *mut c_void;
    pub fn OPENSSL_sk_push(st: *mut OPENSSL_STACK, data: *const c_void) -> c_int;
    pub fn OPENSSL_sk_pop(st: *mut OPENSSL_STACK) -> *mut c_void;
    pub fn OPENSSL_sk_shift(st: *mut OPENSSL_STACK) -> *mut c_void;
    pub fn OPENSSL_sk_free(st: *mut OPENSSL_STACK);
    pub fn OPENSSL_sk_pop_free(st: *mut OPENSSL_STACK, func: unsafe extern "C" fn(*mut c_void));
}
