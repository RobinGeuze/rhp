//! The TLS layer of socket streams (ssl://, tls://, tlsv1.x://, and
//! `stream_socket_enable_crypto` on tcp://): a port of ext/openssl/xp_ssl.c.
//! The session runs on the socket's own descriptor (`SSL_set_fd`), as php's
//! does, so errno-based errors read the same.
//!
//! These functions run on a stream the VM lends out, without a host: their
//! warnings go to the stream diagnostics queue (`fdstream::push_diag`) and
//! whoever ran the operation reports them. Peer certificates to capture are
//! handed back for the caller to wrap in `OpenSSLCertificate` objects.

use std::ffi::{c_char, c_int, c_long, c_uchar, c_uint, c_void, CStr, CString};
use std::os::fd::RawFd;
use std::ptr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use rhp_values::{conv, ArrayKey, PhpArray, Value};

use super::sys;
use crate::fdstream::{poll_fd, push_diag, FdStream, SockInfo};
use crate::streams::errno;
use crate::E_WARNING;

// STREAM_CRYPTO_METHOD_* bits
pub const IS_CLIENT: i64 = 1;
const SSLV3: i64 = 1 << 2;
const TLS1_0: i64 = 1 << 3;
const TLS1_1: i64 = 1 << 4;
const TLS1_2: i64 = 1 << 5;
const TLS1_3: i64 = 1 << 6;
/// `STREAM_CRYPTO_METHOD_TLS_ANY_CLIENT`.
pub const TLS_ANY_CLIENT: i64 = IS_CLIENT | TLS1_0 | TLS1_1 | TLS1_2 | TLS1_3;
pub const SSLV3_CLIENT: i64 = IS_CLIENT | SSLV3;
pub const TLS1_0_CLIENT: i64 = IS_CLIENT | TLS1_0;
pub const TLS1_1_CLIENT: i64 = IS_CLIENT | TLS1_1;
pub const TLS1_2_CLIENT: i64 = IS_CLIENT | TLS1_2;
pub const TLS1_3_CLIENT: i64 = IS_CLIENT | TLS1_3;
/// `STREAM_CRYPTO_METHOD_SSLv23_CLIENT` (the http wrapper's proxy tunnel).
pub const SSLV23_CLIENT: i64 = 57;

const DEFAULT_VERIFY_DEPTH: u64 = 9;
const DEFAULT_RENEG_LIMIT: i64 = 2;
const DEFAULT_RENEG_WINDOW: i64 = 300;
const DEFAULT_STREAM_CIPHERS: &CStr = c"ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-AES256-GCM-SHA384:DHE-RSA-AES128-GCM-SHA256:DHE-DSS-AES128-GCM-SHA256:kEDH+AESGCM:ECDHE-RSA-AES128-SHA256:ECDHE-ECDSA-AES128-SHA256:ECDHE-RSA-AES128-SHA:ECDHE-ECDSA-AES128-SHA:ECDHE-RSA-AES256-SHA384:ECDHE-ECDSA-AES256-SHA384:ECDHE-RSA-AES256-SHA:ECDHE-ECDSA-AES256-SHA:DHE-RSA-AES128-SHA256:DHE-RSA-AES128-SHA:DHE-DSS-AES128-SHA256:DHE-RSA-AES256-SHA256:DHE-DSS-AES256-SHA:DHE-RSA-AES256-SHA:AES128-GCM-SHA256:AES256-GCM-SHA384:AES128:AES256:HIGH:!SSLv2:!aNULL:!eNULL:!EXPORT:!DES:!MD5:!RC4:!ADH";

/// Whether the OpenSSL headers rhp was built against leave `feature` on
/// (php's `#ifndef OPENSSL_NO_*` checks; build.rs exports the defines).
fn have(no: &str) -> bool {
    !env!("RHP_OPENSSL_NO").split(',').any(|n| n == no)
}

pub fn have_ssl3() -> bool {
    have("OPENSSL_NO_SSL3")
}
pub fn have_tls11() -> bool {
    have("OPENSSL_NO_TLS1_1_METHOD")
}
pub fn have_tls12() -> bool {
    have("OPENSSL_NO_TLS1_2_METHOD")
}
pub fn have_tls13() -> bool {
    have("OPENSSL_NO_TLS1_3")
}
fn have_tlsext() -> bool {
    have("OPENSSL_NO_TLSEXT")
}

fn min_proto() -> i64 {
    if have_ssl3() {
        SSLV3
    } else {
        TLS1_0
    }
}
fn max_proto() -> i64 {
    if have_tls13() {
        TLS1_3
    } else {
        TLS1_2
    }
}

fn warn(msg: impl Into<String>) {
    push_diag(E_WARNING, true, msg);
}

/// `php_error(E_WARNING, ...)`: no function prefix.
fn warn_plain(msg: impl Into<String>) {
    push_diag(E_WARNING, false, msg);
}

fn cstr_bytes(p: *const c_char) -> Vec<u8> {
    if p.is_null() {
        return Vec::new();
    }
    unsafe { CStr::from_ptr(p) }.to_bytes().to_vec()
}

/// A C string of `b` up to its first NUL (what C code sees of a zend string).
fn cstring(b: &[u8]) -> CString {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    CString::new(&b[..end]).unwrap_or_default()
}

/// What `%s` prints of `b`.
fn shown(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

// ---------------------------------------------------------------------------
// context options
// ---------------------------------------------------------------------------

/// The stream's `ssl` context options, read when an operation starts
/// (`GET_VER_OPT`). `has_ctx` is false for a stream without a context
/// (`fsockopen`).
#[derive(Clone, Default)]
pub struct Opts {
    pub has_ctx: bool,
    ssl: Option<PhpArray>,
}

impl Opts {
    /// From `stream_context_get_options()` of the stream's context.
    pub fn new(has_ctx: bool, options: &Value) -> Opts {
        let ssl = match options {
            Value::Array(a) => match a
                .get(&ArrayKey::from_bytes(b"ssl"))
                .map(rhp_semantics::deref)
            {
                Some(Value::Array(s)) => Some(s),
                _ => None,
            },
            _ => None,
        };
        Opts { has_ctx, ssl }
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        if !self.has_ctx {
            return None;
        }
        self.ssl
            .as_ref()?
            .get(&ArrayKey::from_bytes(name.as_bytes()))
            .map(rhp_semantics::deref)
    }

    /// These options with `name` set (the http wrapper's proxy tunnel sets
    /// `peer_name` for the handshake).
    pub fn with(mut self, name: &str, v: Value) -> Opts {
        self.has_ctx = true;
        self.ssl
            .get_or_insert_with(PhpArray::new)
            .insert(ArrayKey::from_bytes(name.as_bytes()), v);
        self
    }

    fn is_true(&self, name: &str) -> Option<bool> {
        self.get(name).map(|v| conv::to_bool(&v))
    }

    fn long(&self, name: &str) -> Option<i64> {
        self.get(name).map(|v| conv::to_int(&v))
    }

    /// `GET_VER_OPT_STRING`: `try_convert_to_string`.
    pub fn string(&self, name: &str) -> Option<Vec<u8>> {
        string_of(&self.get(name)?)
    }
}

/// `try_convert_to_string` of an option value (objects are left out).
fn string_of(v: &Value) -> Option<Vec<u8>> {
    match v {
        Value::Array(_) => {
            push_diag(E_WARNING, false, "Array to string conversion");
            Some(b"Array".to_vec())
        }
        Value::Object(_) | Value::Resource(_) => None,
        v => Some(conv::to_string_bytes(v)),
    }
}

// ---------------------------------------------------------------------------
// the session
// ---------------------------------------------------------------------------

/// The renegotiation rate limit of a server session.
struct Reneg {
    prev_handshake: i64,
    limit: i64,
    window: i64,
    tokens: f32,
    should_close: bool,
    /// The context has a `reneg_limit_callback`: it decides (after the read
    /// that saw the limit exceeded) whether the connection closes.
    has_callback: bool,
    callback_due: bool,
}

/// What OpenSSL's callbacks read. Behind its own allocation: they run while
/// a handshake has the session borrowed.
#[derive(Default)]
struct Shared {
    allow_self_signed: bool,
    verify_depth: u64,
    passphrase: Option<Vec<u8>>,
    sni: Vec<(CString, *mut sys::SSL_CTX)>,
    alpn: Vec<u8>,
    reneg: Option<Reneg>,
}

/// php's `php_openssl_netstream_data_t` minus the socket half (that is the
/// stream's [`SockInfo`]).
pub struct SslSock {
    ssl: *mut sys::SSL,
    ctx: *mut sys::SSL_CTX,
    pub enable_on_connect: bool,
    pub is_client: bool,
    /// php's `ssl_active`.
    pub active: bool,
    pub method: i64,
    pub url_name: Option<Vec<u8>>,
    state_set: bool,
    /// The http wrapper's response came from IIS or Google's frontend.
    pub talking_to_iis: bool,
    shared: *mut Shared,
}

impl SslSock {
    pub fn new(enable_on_connect: bool, method: i64, url_name: Option<Vec<u8>>) -> SslSock {
        SslSock {
            ssl: ptr::null_mut(),
            ctx: ptr::null_mut(),
            enable_on_connect,
            is_client: false,
            active: false,
            method,
            url_name,
            state_set: false,
            talking_to_iis: false,
            shared: Box::into_raw(Box::new(Shared::default())),
        }
    }

    /// The SSL handle (null before setup), for a session stream.
    pub fn handle(&self) -> *mut sys::SSL {
        self.ssl
    }

    /// `SSL_pending`.
    pub fn pending(&self) -> usize {
        if self.ssl.is_null() || !self.active {
            return 0;
        }
        unsafe { sys::SSL_pending(self.ssl) }.max(0) as usize
    }

    fn shared(&mut self) -> &mut Shared {
        unsafe { &mut *self.shared }
    }
}

impl Drop for SslSock {
    fn drop(&mut self) {
        unsafe {
            if self.active {
                sys::SSL_shutdown(self.ssl);
                self.active = false;
            }
            if !self.ssl.is_null() {
                sys::SSL_free(self.ssl);
            }
            if !self.ctx.is_null() {
                sys::SSL_CTX_free(self.ctx);
            }
            let shared = Box::from_raw(self.shared);
            for (_, ctx) in &shared.sni {
                if !ctx.is_null() {
                    sys::SSL_CTX_free(*ctx);
                }
            }
        }
        // a panic in a callback of SSL_shutdown, once everything is freed
        if !std::thread::panicking() {
            super::with_callbacks(|| ());
        }
    }
}

/// php_openssl_get_ssl_stream_data_index.
fn data_index() -> c_int {
    static IDX: OnceLock<c_int> = OnceLock::new();
    *IDX.get_or_init(|| {
        super::init();
        unsafe {
            sys::SSL_get_ex_new_index(
                0,
                c"PHP stream index".as_ptr() as *mut c_void,
                None,
                None,
                None,
            )
        }
    })
}

fn shared_of(ssl: *const sys::SSL) -> Option<&'static mut Shared> {
    let p = unsafe { sys::SSL_get_ex_data(ssl, data_index()) } as *mut Shared;
    (!p.is_null()).then(|| unsafe { &mut *p })
}

// ---------------------------------------------------------------------------
// callbacks
// ---------------------------------------------------------------------------

/// `verify_callback`.
extern "C" fn verify_callback(preverify_ok: c_int, ctx: *mut sys::X509_STORE_CTX) -> c_int {
    super::guard(preverify_ok, || unsafe {
        let err = sys::X509_STORE_CTX_get_error(ctx);
        let depth = sys::X509_STORE_CTX_get_error_depth(ctx);
        let ssl = sys::X509_STORE_CTX_get_ex_data(ctx, sys::SSL_get_ex_data_X509_STORE_CTX_idx())
            as *const sys::SSL;
        let Some(sh) = shared_of(ssl) else {
            return preverify_ok;
        };
        let mut ret = preverify_ok;
        if err == sys::X509_V_ERR_DEPTH_ZERO_SELF_SIGNED_CERT && sh.allow_self_signed {
            ret = 1;
        }
        if depth as u64 > sh.verify_depth {
            ret = 0;
            sys::X509_STORE_CTX_set_error(ctx, sys::X509_V_ERR_CERT_CHAIN_TOO_LONG);
        }
        ret
    })
}

/// `php_openssl_passwd_callback`.
unsafe extern "C" fn passwd_callback(
    buf: *mut c_char,
    num: c_int,
    _verify: c_int,
    data: *mut c_void,
) -> c_int {
    super::guard(0, || {
        let sh = unsafe { &*(data as *const Shared) };
        match &sh.passphrase {
            Some(p) if p.len() < (num as usize).saturating_sub(1) => {
                unsafe {
                    ptr::copy_nonoverlapping(p.as_ptr(), buf as *mut u8, p.len());
                    *buf.add(p.len()) = 0;
                }
                p.len() as c_int
            }
            _ => 0,
        }
    })
}

/// `php_openssl_info_callback`: rate-limit client-initiated renegotiation.
unsafe extern "C" fn info_callback(ssl: *const sys::SSL, where_: c_int, _ret: c_int) {
    super::guard((), || {
        if where_ & sys::SSL_CB_HANDSHAKE_START == 0 {
            return;
        }
        let Some(reneg) = shared_of(ssl).and_then(|s| s.reneg.as_mut()) else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        if reneg.prev_handshake == 0 {
            reneg.prev_handshake = now;
            return;
        }
        let elapsed = now - reneg.prev_handshake;
        reneg.prev_handshake = now;
        let rate = reneg.limit.checked_div(reneg.window).unwrap_or(0);
        reneg.tokens -= (elapsed * rate) as f32;
        if reneg.tokens < 0.0 {
            reneg.tokens = 0.0;
        }
        reneg.tokens += 1.0;
        if reneg.tokens > reneg.limit as f32 {
            reneg.should_close = true;
            if reneg.has_callback {
                reneg.callback_due = true;
            } else {
                warn("SSL: client-initiated handshake rate limit exceeded by peer");
            }
        }
    })
}

/// `php_openssl_server_sni_callback`.
unsafe extern "C" fn sni_callback(ssl: *mut sys::SSL, _al: *mut c_int, _arg: *mut c_void) -> c_int {
    super::guard(sys::SSL_TLSEXT_ERR_NOACK, || unsafe {
        let name = sys::SSL_get_servername(ssl, sys::TLSEXT_NAMETYPE_host_name);
        if name.is_null() {
            return sys::SSL_TLSEXT_ERR_NOACK;
        }
        let Some(sh) = shared_of(ssl) else {
            return sys::SSL_TLSEXT_ERR_NOACK;
        };
        let name = CStr::from_ptr(name).to_bytes();
        for (cert_name, ctx) in &sh.sni {
            if matches_wildcard_name(name, cert_name.as_bytes()) {
                sys::SSL_set_SSL_CTX(ssl, *ctx);
                return sys::SSL_TLSEXT_ERR_OK;
            }
        }
        sys::SSL_TLSEXT_ERR_NOACK
    })
}

/// `php_openssl_server_alpn_callback`.
unsafe extern "C" fn alpn_callback(
    _ssl: *mut sys::SSL,
    out: *mut *const c_uchar,
    outlen: *mut c_uchar,
    inp: *const c_uchar,
    inlen: c_uint,
    arg: *mut c_void,
) -> c_int {
    super::guard(sys::SSL_TLSEXT_ERR_NOACK, || unsafe {
        let sh = &*(arg as *const Shared);
        let r = sys::SSL_select_next_proto(
            out as *mut *mut c_uchar,
            outlen,
            sh.alpn.as_ptr(),
            sh.alpn.len() as c_uint,
            inp,
            inlen,
        );
        if r != sys::OPENSSL_NPN_NEGOTIATED {
            return sys::SSL_TLSEXT_ERR_NOACK;
        }
        sys::SSL_TLSEXT_ERR_OK
    })
}

// ---------------------------------------------------------------------------
// errors
// ---------------------------------------------------------------------------

/// `php_openssl_handle_ssl_error`: whether to retry, and errno as php
/// leaves it (`err_no` is errno right after the SSL call).
fn handle_ssl_error(
    t: &SslSock,
    blocked: bool,
    eof: &mut bool,
    nr_bytes: c_int,
    is_init: bool,
    err_no: i32,
) -> (bool, i32) {
    let err = unsafe { sys::SSL_get_error(t.ssl, nr_bytes) };
    let mut errno_out = err_no;
    let retry;
    let syscall_clean = err == sys::SSL_ERROR_SYSCALL && unsafe { sys::ERR_peek_error() } == 0;
    match err {
        sys::SSL_ERROR_ZERO_RETURN => retry = false,
        sys::SSL_ERROR_WANT_READ | sys::SSL_ERROR_WANT_WRITE => {
            errno_out = libc::EAGAIN;
            retry = is_init || blocked;
        }
        _ if syscall_clean => {
            if nr_bytes == 0 {
                if !t.talking_to_iis && unsafe { sys::ERR_get_error() } != 0 {
                    warn("SSL: fatal protocol error");
                }
                unsafe {
                    sys::SSL_set_shutdown(
                        t.ssl,
                        sys::SSL_SENT_SHUTDOWN | sys::SSL_RECEIVED_SHUTDOWN,
                    )
                };
                *eof = true;
            } else {
                warn(format!("SSL: {}", crate::streams::strerror(err_no)));
            }
            retry = false;
        }
        _ => {
            let mut ecode = unsafe { sys::ERR_get_error() };
            if sys::ERR_GET_REASON(ecode) == sys::SSL_R_NO_SHARED_CIPHER {
                warn(
                    "SSL_R_NO_SHARED_CIPHER: no suitable shared cipher could be used.  \
                     This could be because the server is missing an SSL certificate \
                     (local_cert context option)",
                );
            } else {
                let mut msgs: Vec<String> = Vec::new();
                loop {
                    let mut buf = [0 as c_char; 512];
                    unsafe { sys::ERR_error_string_n(ecode, buf.as_mut_ptr(), buf.len()) };
                    msgs.push(String::from_utf8_lossy(&cstr_bytes(buf.as_ptr())).into_owned());
                    ecode = unsafe { sys::ERR_get_error() };
                    if ecode == 0 {
                        break;
                    }
                }
                warn(format!(
                    "SSL operation failed with code {err}. OpenSSL Error messages:\n{}",
                    msgs.join("\n")
                ));
            }
            retry = false;
            errno_out = 0;
        }
    }
    (retry, errno_out)
}

// ---------------------------------------------------------------------------
// peer verification
// ---------------------------------------------------------------------------

/// `php_openssl_matches_wildcard_name`.
fn matches_wildcard_name(subject: &[u8], cert: &[u8]) -> bool {
    if subject.eq_ignore_ascii_case(cert) {
        return true;
    }
    let Some(w) = cert.iter().position(|&c| c == b'*') else {
        return false;
    };
    if cert[..w].contains(&b'.') {
        return false;
    }
    let prefix = w;
    if prefix > 0
        && (subject.len() < prefix || !subject[..prefix].eq_ignore_ascii_case(&cert[..prefix]))
    {
        return false;
    }
    let suffix = &cert[w + 1..];
    if suffix.len() + prefix <= subject.len() {
        let tail = &subject[subject.len() - suffix.len()..];
        return tail.eq_ignore_ascii_case(suffix)
            && !subject[prefix..subject.len() - suffix.len()].contains(&b'.');
    }
    false
}

/// `EXPAND_IPV6_ADDRESS`.
fn expand_ipv6(b: &[u8]) -> String {
    (0..8)
        .map(|i| format!("{:X}", (b[2 * i] as u16) << 8 | b[2 * i + 1] as u16))
        .collect::<Vec<_>>()
        .join(":")
}

unsafe extern "C" fn free_general_name(p: *mut c_void) {
    unsafe { sys::GENERAL_NAME_free(p as *mut sys::GENERAL_NAME) };
}

/// `php_openssl_matches_san_list`: `(matched, has_service_id)`.
fn matches_san_list(peer: *mut sys::X509, subject: &[u8]) -> (bool, bool) {
    let names = unsafe {
        sys::X509_get_ext_d2i(
            peer,
            sys::NID_subject_alt_name,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    } as *mut sys::OPENSSL_STACK;
    let count = if names.is_null() {
        -1
    } else {
        unsafe { sys::OPENSSL_sk_num(names) }
    };
    let free = || {
        if !names.is_null() {
            unsafe { sys::OPENSSL_sk_pop_free(names, free_general_name) };
        }
    };
    let mut has_service_id = false;
    let subject_ipv6 = crate::inet::pton6(subject).map(|a| expand_ipv6(&a));
    for i in 0..count {
        let san = unsafe { &*(sys::OPENSSL_sk_value(names, i) as *const sys::GENERAL_NAME) };
        match san.type_ {
            sys::GEN_DNS => {
                has_service_id = true;
                let s = san.d as *const sys::ASN1_STRING;
                let mut out: *mut c_uchar = ptr::null_mut();
                if unsafe { sys::ASN1_STRING_to_UTF8(&mut out, s) } < 0 {
                    continue;
                }
                let mut name = cstr_bytes(out as *const c_char);
                let len = unsafe { sys::ASN1_STRING_length(s) } as usize;
                unsafe { super::openssl_free(out) };
                if len != name.len() {
                    // null-byte poisoning
                    continue;
                }
                if name.last() == Some(&b'.') {
                    name.pop();
                }
                if matches_wildcard_name(subject, &name) {
                    free();
                    return (true, has_service_id);
                }
            }
            sys::GEN_IPADD => {
                let s = san.d as *const sys::ASN1_STRING;
                let len = unsafe { sys::ASN1_STRING_length(s) };
                let data = unsafe { sys::ASN1_STRING_get0_data(s) };
                if len == 4 {
                    let b = unsafe { std::slice::from_raw_parts(data, 4) };
                    let ip = format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]);
                    if subject.eq_ignore_ascii_case(ip.as_bytes()) {
                        free();
                        return (true, has_service_id);
                    }
                } else if len == 16 {
                    if let Some(sub) = &subject_ipv6 {
                        let b = unsafe { std::slice::from_raw_parts(data, 16) };
                        if sub.eq_ignore_ascii_case(&expand_ipv6(b)) {
                            free();
                            return (true, has_service_id);
                        }
                    }
                }
            }
            sys::GEN_URI => has_service_id = true,
            sys::GEN_OTHERNAME => {
                // SRV-ID, matched by OID
                let other = unsafe { &*(san.d as *const sys::OTHERNAME) };
                let mut oid = [0 as c_char; 32];
                let n = unsafe { sys::OBJ_obj2txt(oid.as_mut_ptr(), 32, other.type_id, 1) };
                if n > 0 && cstr_bytes(oid.as_ptr()) == b"1.3.6.1.5.5.7.8.7" {
                    has_service_id = true;
                }
            }
            _ => {}
        }
    }
    free();
    (false, has_service_id)
}

/// `php_openssl_matches_common_name`.
fn matches_common_name(peer: *mut sys::X509, subject: &[u8]) -> bool {
    let mut buf = [0 as c_char; 1024];
    let name = unsafe { sys::X509_get_subject_name(peer) };
    let len = unsafe {
        sys::X509_NAME_get_text_by_NID(
            name,
            sys::NID_commonName,
            buf.as_mut_ptr(),
            buf.len() as c_int,
        )
    };
    let text = cstr_bytes(buf.as_ptr());
    if len == -1 {
        warn("Unable to locate peer certificate CN");
        false
    } else if len as usize != text.len() {
        let raw: Vec<u8> = buf[..len.max(0) as usize]
            .iter()
            .map(|&c| c as u8)
            .collect();
        warn(format!(
            "Peer certificate CN=`{}' is malformed",
            String::from_utf8_lossy(&raw)
        ));
        false
    } else if matches_wildcard_name(subject, &text) {
        true
    } else {
        warn(format!(
            "Peer certificate CN=`{}' did not match expected CN=`{}'",
            String::from_utf8_lossy(&text),
            shown(subject)
        ));
        false
    }
}

/// `php_openssl_x509_fingerprint_cmp`: 0 when equal.
fn fingerprint_cmp(peer: *mut sys::X509, method: &[u8], expected: &[u8]) -> i32 {
    let md = super::backend::md_by_name(&cstring(method));
    if md.is_null() {
        warn("Unknown digest algorithm");
        return -1;
    }
    let mut buf = [0u8; sys::EVP_MAX_MD_SIZE as usize];
    let mut n: c_uint = 0;
    let ok = unsafe { sys::X509_digest(peer, md, buf.as_mut_ptr(), &mut n) };
    super::backend::release_md(md);
    if ok == 0 {
        super::store_errors();
        push_diag(1, true, "Could not generate signature");
        return -1;
    }
    let hex = super::backend::hex(&buf[..n as usize]);
    if shown(expected).eq_ignore_ascii_case(&hex) {
        0
    } else {
        1
    }
}

/// `php_openssl_x509_fingerprint_match`.
fn fingerprint_match(peer: *mut sys::X509, val: &Value) -> bool {
    match val {
        Value::Str(s) => {
            let method: &[u8] = match s.len() {
                32 => b"md5",
                40 => b"sha1",
                _ => return false,
            };
            fingerprint_cmp(peer, method, s.as_bytes()) == 0
        }
        Value::Array(a) => {
            if a.is_empty() {
                warn("Invalid peer_fingerprint array; [algo => fingerprint] form required");
                return false;
            }
            for (k, v) in a.iter() {
                let (ArrayKey::Str(k), Value::Str(fp)) = (k, rhp_semantics::deref(v)) else {
                    warn("Invalid peer_fingerprint array; [algo => fingerprint] form required");
                    return false;
                };
                if fingerprint_cmp(peer, k.as_bytes(), fp.as_bytes()) != 0 {
                    return false;
                }
            }
            true
        }
        _ => {
            warn("Invalid peer_fingerprint value; fingerprint string or array of the form [algo => fingerprint] required");
            false
        }
    }
}

/// `php_openssl_apply_peer_verification_policy`.
fn apply_peer_verification_policy(t: &SslSock, peer: *mut sys::X509, opts: &Opts) -> bool {
    let must_verify_peer = opts.is_true("verify_peer").unwrap_or(t.is_client);
    let must_verify_peer_name = opts.is_true("verify_peer_name").unwrap_or(t.is_client);
    let fingerprint = opts.get("peer_fingerprint");
    if (must_verify_peer || must_verify_peer_name || fingerprint.is_some()) && peer.is_null() {
        warn("Could not get peer certificate");
        return false;
    }
    if must_verify_peer {
        let err = unsafe { sys::SSL_get_verify_result(t.ssl) } as c_int;
        let ok = err == sys::X509_V_OK
            || (err == sys::X509_V_ERR_DEPTH_ZERO_SELF_SIGNED_CERT
                && opts.is_true("allow_self_signed") == Some(true));
        if !ok {
            let text = cstr_bytes(unsafe { sys::X509_verify_cert_error_string(err as c_long) });
            warn(format!(
                "Could not verify peer: code:{err} {}",
                String::from_utf8_lossy(&text)
            ));
            return false;
        }
    }
    if let Some(fp) = fingerprint {
        if matches!(fp, Value::Str(_) | Value::Array(_)) {
            if !fingerprint_match(peer, &fp) {
                warn("peer_fingerprint match failure");
                return false;
            }
        } else {
            warn("Expected peer fingerprint must be a string or an array");
            return false;
        }
    }
    if must_verify_peer_name {
        let mut peer_name = opts.string("peer_name");
        if peer_name.is_none() && t.is_client {
            peer_name = t.url_name.clone();
        }
        let Some(name) = peer_name else {
            return false;
        };
        let name = &name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())];
        let (matched, has_service_id) = matches_san_list(peer, name);
        if matched {
            return true;
        } else if has_service_id {
            warn(format!(
                "Peer certificate subjectAltName did not match expected name `{}'",
                shown(name)
            ));
            return false;
        }
        return matches_common_name(peer, name);
    }
    true
}

// ---------------------------------------------------------------------------
// setup
// ---------------------------------------------------------------------------

/// `php_openssl_check_path_ex` for a context option: the absolute path, or
/// `None` after a warning.
fn check_path(path: &[u8], option: &str) -> Option<CString> {
    if path.is_empty() {
        return Some(CString::default());
    }
    let msg = if path.contains(&0) {
        "must not contain any null bytes"
    } else {
        let real = crate::ext::dom::expand_filepath(path);
        if path.len() < libc::PATH_MAX as usize && real.len() < libc::PATH_MAX as usize {
            return Some(CString::new(real).unwrap_or_default());
        }
        "must be a valid file path"
    };
    warn(format!("Path for {option} option {msg}"));
    None
}

fn ini_string(name: &[u8]) -> Option<Vec<u8>> {
    crate::runtime_fns::ini_value(name)
        .flatten()
        .filter(|v| !v.is_empty())
}

/// `php_openssl_load_stream_cafile`: the certificates added.
fn load_stream_cafile(store: *mut sys::X509_STORE, cafile: &[u8]) -> i64 {
    let (scheme, _) = crate::streams::split_scheme(cafile);
    let data = if scheme == b"file" {
        std::fs::read(&*String::from_utf8_lossy(crate::streams::file_path(cafile))).ok()
    } else {
        None
    };
    let remote = matches!(
        &scheme.to_ascii_lowercase()[..],
        b"http" | b"https" | b"ftp" | b"ftps" | b"data"
    );
    let Some(data) = data.filter(|_| !remote) else {
        if remote {
            warn_plain("remote cafile streams are disabled for security purposes");
        } else {
            warn_plain(format!("failed loading cafile stream: `{}'", shown(cafile)));
        }
        return 0;
    };
    let mut added = 0;
    let mut buffer: Option<Vec<u8>> = None;
    for line in data.split_inclusive(|&b| b == b'\n') {
        match &mut buffer {
            None => {
                if line == b"-----BEGIN CERTIFICATE-----\n"
                    || line == b"-----BEGIN CERTIFICATE-----\r\n"
                {
                    buffer = Some(line.to_vec());
                }
            }
            Some(buf) => {
                buf.extend_from_slice(line);
                if line == b"-----END CERTIFICATE-----"
                    || line == b"-----END CERTIFICATE-----\n"
                    || line == b"-----END CERTIFICATE-----\r\n"
                {
                    let pem = buffer.take().unwrap_or_default();
                    let bio = super::mem_buf(&pem);
                    let cert = super::backend::pem_read_bio_x509(bio.p);
                    if !cert.is_null() {
                        if unsafe { sys::X509_STORE_add_cert(store, cert) } != 0 {
                            added += 1;
                        }
                        unsafe { sys::X509_free(cert) };
                    }
                }
            }
        }
    }
    if added == 0 {
        warn_plain(format!(
            "no valid certs found cafile stream: `{}'",
            shown(cafile)
        ));
    }
    added
}

/// `php_openssl_enable_peer_verification`.
fn enable_peer_verification(ctx: *mut sys::SSL_CTX, is_client: bool, opts: &Opts) -> bool {
    let mut cafile = opts.string("cafile");
    let mut capath = opts.string("capath");
    match &cafile {
        None => cafile = ini_string(b"openssl.cafile"),
        Some(f) if !is_client => {
            let c = cstring(f);
            let names = unsafe { sys::SSL_load_client_CA_file(c.as_ptr()) };
            if names.is_null() {
                warn_plain("SSL: failed loading CA names from cafile");
                return false;
            }
            unsafe { sys::SSL_CTX_set_client_CA_list(ctx, names) };
        }
        Some(_) => {}
    }
    if capath.is_none() {
        capath = ini_string(b"openssl.capath");
    }
    if cafile.is_some() || capath.is_some() {
        let f = cafile.as_deref().map(cstring);
        let p = capath.as_deref().map(cstring);
        let ok = unsafe {
            sys::SSL_CTX_load_verify_locations(
                ctx,
                f.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
                p.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
            )
        };
        if ok == 0 {
            unsafe { sys::ERR_clear_error() };
            if let Some(f) = &cafile {
                if load_stream_cafile(unsafe { sys::SSL_CTX_get_cert_store(ctx) }, f) == 0 {
                    return false;
                }
            }
        }
    } else if is_client && unsafe { sys::SSL_CTX_set_default_verify_paths(ctx) } == 0 {
        warn("Unable to set default verify locations and no CA settings specified");
        return false;
    }
    unsafe { sys::SSL_CTX_set_verify(ctx, sys::SSL_VERIFY_PEER, Some(verify_callback)) };
    true
}

fn disable_peer_verification(ctx: *mut sys::SSL_CTX) {
    unsafe { sys::SSL_CTX_set_verify(ctx, sys::SSL_VERIFY_NONE, None) };
}

/// `php_openssl_set_local_cert`.
fn set_local_cert(ctx: *mut sys::SSL_CTX, opts: &Opts) -> bool {
    let Some(certfile) = opts.string("local_cert") else {
        return true;
    };
    let Some(mut resolved) = check_path(&certfile, "local_cert in ssl stream context") else {
        warn(format!(
            "Unable to get real path of certificate file `{}'",
            shown(&certfile)
        ));
        return false;
    };
    if super::with_callbacks(|| unsafe {
        sys::SSL_CTX_use_certificate_chain_file(ctx, resolved.as_ptr())
    }) != 1
    {
        warn(format!(
            "Unable to set local cert chain file `{}'; Check that your cafile/capath settings include details of your certificate and its issuer",
            shown(&certfile)
        ));
        return false;
    }
    if let Some(pk) = opts.string("local_pk") {
        match check_path(&pk, "local_pk in ssl stream context") {
            Some(p) => resolved = p,
            None => {
                warn(format!(
                    "Unable to get real path of private key file `{}'",
                    shown(&pk)
                ));
                return false;
            }
        }
    }
    if super::with_callbacks(|| unsafe {
        sys::SSL_CTX_use_PrivateKey_file(ctx, resolved.as_ptr(), sys::SSL_FILETYPE_PEM)
    }) != 1
    {
        warn(format!(
            "Unable to set private key file `{}'",
            String::from_utf8_lossy(resolved.as_bytes())
        ));
        return false;
    }
    if unsafe { sys::SSL_CTX_check_private_key(ctx) } == 0 {
        warn("Private key does not match certificate!");
    }
    true
}

fn min_proto_flag(flags: i64) -> i64 {
    let mut ver = min_proto();
    while ver <= max_proto() {
        if flags & ver != 0 {
            return ver;
        }
        ver <<= 1;
    }
    max_proto()
}

fn max_proto_flag(flags: i64) -> i64 {
    let mut ver = max_proto();
    while ver >= min_proto() {
        if flags & ver != 0 {
            return ver;
        }
        ver >>= 1;
    }
    TLS1_3
}

fn map_proto_version(flag: i64) -> c_int {
    match flag {
        TLS1_3 if have_tls13() => sys::TLS1_3_VERSION,
        TLS1_2 => sys::TLS1_2_VERSION,
        TLS1_1 => sys::TLS1_1_VERSION,
        TLS1_0 => sys::TLS1_VERSION,
        SSLV3 if have_ssl3() => sys::SSL3_VERSION,
        _ => sys::TLS1_2_VERSION,
    }
}

/// `php_openssl_get_proto_version_flags`.
fn proto_version_flags(mut flags: i64, min: i64, max: i64) -> i64 {
    let min = if min == 0 { min_proto_flag(flags) } else { min };
    let max = if max == 0 { max_proto_flag(flags) } else { max };
    let mut ver = min_proto();
    while ver <= max_proto() {
        if ver >= min && ver <= max {
            flags |= ver;
        } else {
            flags &= !ver;
        }
        ver <<= 1;
    }
    flags
}

/// `php_openssl_set_server_specific_opts`.
fn set_server_specific_opts(ctx: *mut sys::SSL_CTX, opts: &Opts) -> bool {
    if opts.get("rsa_key_size").is_some() {
        warn("rsa_key_size context option has been removed");
    }
    if let Some(dh) = opts.get("dh_param") {
        let Some(path) = string_of(&dh) else {
            return false;
        };
        let c = cstring(&path);
        let bio = unsafe { sys::BIO_new_file(c.as_ptr(), c"r".as_ptr()) };
        if bio.is_null() {
            warn("Invalid dh_param");
            return false;
        }
        let pkey = unsafe { sys::PEM_read_bio_Parameters(bio, ptr::null_mut()) };
        unsafe { sys::BIO_free(bio) };
        if pkey.is_null() {
            warn("Failed reading DH params");
            return false;
        }
        if unsafe { sys::SSL_CTX_set0_tmp_dh_pkey(ctx, pkey) } == 0 {
            warn("Failed assigning DH params");
            unsafe { sys::EVP_PKEY_free(pkey) };
            return false;
        }
    }
    let mut options = unsafe { sys::SSL_CTX_get_options(ctx) };
    if opts.is_true("single_dh_use") != Some(false) {
        options |= sys::SSL_OP_SINGLE_DH_USE;
    }
    if opts.is_true("honor_cipher_order") != Some(false) {
        options |= sys::SSL_OP_CIPHER_SERVER_PREFERENCE;
    }
    unsafe { sys::SSL_CTX_set_options(ctx, options) };
    true
}

/// `php_openssl_create_sni_server_ctx`.
fn create_sni_server_ctx(cert: &CStr, key: &CStr) -> *mut sys::SSL_CTX {
    let ctx = unsafe { sys::SSL_CTX_new(sys::TLS_server_method()) };
    if ctx.is_null() {
        warn("Failed to create the SSL context");
        return ptr::null_mut();
    }
    if super::with_callbacks(|| unsafe {
        sys::SSL_CTX_use_certificate_chain_file(ctx, cert.as_ptr())
    }) != 1
    {
        warn(format!(
            "Failed setting local cert chain file `{}'; check that your cafile/capath settings include details of your certificate and its issuer",
            String::from_utf8_lossy(cert.to_bytes())
        ));
        unsafe { sys::SSL_CTX_free(ctx) };
        return ptr::null_mut();
    }
    if super::with_callbacks(|| unsafe {
        sys::SSL_CTX_use_PrivateKey_file(ctx, key.as_ptr(), sys::SSL_FILETYPE_PEM)
    }) != 1
    {
        warn(format!(
            "Failed setting private key from file `{}'",
            String::from_utf8_lossy(key.to_bytes())
        ));
        unsafe { sys::SSL_CTX_free(ctx) };
        return ptr::null_mut();
    }
    ctx
}

/// `php_openssl_enable_server_sni`.
fn enable_server_sni(t: &mut SslSock, opts: &Opts, verify_peer: bool) -> bool {
    if opts.is_true("SNI_enabled") == Some(false) {
        return true;
    }
    let Some(certs) = opts.get("SNI_server_certs") else {
        return true;
    };
    let Value::Array(certs) = certs else {
        warn("SNI_server_certs requires an array mapping host names to cert paths");
        return false;
    };
    if certs.is_empty() {
        warn("SNI_server_certs host cert array must not be empty");
        return false;
    }
    for (key, current) in certs.iter() {
        let ArrayKey::Str(key) = key else {
            warn("SNI_server_certs array requires string host name keys");
            return false;
        };
        let ctx = match rhp_semantics::deref(current) {
            Value::Array(a) => {
                let get = |n: &[u8]| a.get(&ArrayKey::from_bytes(n)).map(rhp_semantics::deref);
                let Some(cert) = get(b"local_cert") else {
                    warn("local_cert not present in the array");
                    return false;
                };
                let Some(cert) = string_of(&cert) else {
                    return false;
                };
                let Some(cert_path) =
                    check_path(&cert, "SNI_server_certs local_cert in ssl stream context")
                else {
                    warn(format!(
                        "Failed setting local cert chain file `{}'; could not open file",
                        shown(&cert)
                    ));
                    return false;
                };
                let Some(pk) = get(b"local_pk") else {
                    warn("local_pk not present in the array");
                    return false;
                };
                let Some(pk) = string_of(&pk) else {
                    return false;
                };
                let Some(pk_path) =
                    check_path(&pk, "SNI_server_certs local_pk in ssl stream context")
                else {
                    warn(format!(
                        "Failed setting local private key file `{}';  could not open file",
                        shown(&pk)
                    ));
                    return false;
                };
                create_sni_server_ctx(&cert_path, &pk_path)
            }
            Value::Str(s) => {
                match check_path(s.as_bytes(), "SNI_server_certs in ssl stream context") {
                    Some(p) => create_sni_server_ctx(&p, &p),
                    None => {
                        warn(format!(
                            "Failed setting local cert chain file `{}'; file not found",
                            shown(s.as_bytes())
                        ));
                        ptr::null_mut()
                    }
                }
            }
            _ => {
                warn("SNI_server_certs options values must be of type array|string");
                ptr::null_mut()
            }
        };
        if ctx.is_null() {
            return false;
        }
        t.shared().sni.push((cstring(key.as_bytes()), ctx));
        if !verify_peer {
            disable_peer_verification(ctx);
        } else if !enable_peer_verification(ctx, t.is_client, opts) {
            return false;
        }
    }
    unsafe { sys::SSL_CTX_set_servername_cb(t.ctx, sni_callback) };
    true
}

/// `php_openssl_alpn_protos_parse`.
fn alpn_protos_parse(input: &[u8]) -> Option<Vec<u8>> {
    let input = &input[..input.iter().position(|&c| c == 0).unwrap_or(input.len())];
    if input.len() >= 65535 {
        return None;
    }
    let mut out = vec![0u8; input.len() + 1];
    let mut start = 0;
    for i in 0..=input.len() {
        if i == input.len() || input[i] == b',' {
            if i - start > 255 {
                return None;
            }
            out[start] = (i - start) as u8;
            start = i + 1;
        } else {
            out[i + 1] = input[i];
        }
    }
    Some(out)
}

/// A session stream for `stream_socket_enable_crypto`'s fourth argument.
pub enum Session {
    /// Not a stream with the TLS ops.
    NotSsl,
    Ssl(*mut sys::SSL),
}

/// `php_openssl_setup_crypto`: 0 or -1.
pub fn setup_crypto(
    stream: &mut FdStream,
    opts: &Opts,
    method: i64,
    session: Option<Session>,
) -> i32 {
    let Some(raw) = stream.raw_fd() else {
        return -1;
    };
    let blocked = stream.sock.as_ref().is_none_or(|s| s.blocked);
    let Some(t) = stream.tls.as_deref_mut() else {
        return -1;
    };
    if !t.ssl.is_null() {
        if blocked {
            warn("SSL/TLS already set-up for this stream");
            return -1;
        }
        return 0;
    }
    super::init();
    unsafe { sys::ERR_clear_error() };
    t.is_client = method & IS_CLIENT != 0;
    let method_flags = method & !IS_CLIENT;
    let m = unsafe {
        if t.is_client {
            sys::TLS_client_method()
        } else {
            sys::TLS_server_method()
        }
    };
    if !t.ctx.is_null() {
        // left by a setup that failed after creating it
        unsafe { sys::SSL_CTX_free(t.ctx) };
    }
    t.ctx = unsafe { sys::SSL_CTX_new(m) };
    if t.ctx.is_null() {
        warn("SSL context creation failure");
        return -1;
    }
    let min_version = opts.long("min_proto_version").unwrap_or(0);
    let max_version = opts.long("max_proto_version").unwrap_or(0);
    let method_flags = proto_version_flags(method_flags, min_version, max_version);
    let mut options = sys::SSL_OP_ALL;
    if opts.is_true("no_ticket") == Some(true) {
        options |= sys::SSL_OP_NO_TICKET;
    }
    options &= !sys::SSL_OP_DONT_INSERT_EMPTY_FRAGMENTS;
    options |= sys::SSL_OP_IGNORE_UNEXPECTED_EOF;
    if opts.is_true("disable_compression") != Some(false) {
        options |= sys::SSL_OP_NO_COMPRESSION;
    }
    let mut verify_peer = false;
    if opts.is_true("verify_peer") == Some(false) {
        disable_peer_verification(t.ctx);
    } else {
        verify_peer = true;
        if !enable_peer_verification(t.ctx, t.is_client, opts) {
            return -1;
        }
    }
    if opts.get("passphrase").is_some() {
        t.shared().passphrase = opts.string("passphrase");
        unsafe {
            sys::SSL_CTX_set_default_passwd_cb_userdata(t.ctx, t.shared as *mut c_void);
            sys::SSL_CTX_set_default_passwd_cb(t.ctx, Some(passwd_callback));
        }
    }
    let ciphers = opts
        .string("ciphers")
        .map(|c| cstring(&c))
        .unwrap_or_else(|| DEFAULT_STREAM_CIPHERS.to_owned());
    if unsafe { sys::SSL_CTX_set_cipher_list(t.ctx, ciphers.as_ptr()) } != 1 {
        return -1;
    }
    if let Some(level) = opts.long("security_level") {
        if !(0..=5).contains(&level) {
            warn("Security level must be between 0 and 5");
        }
        unsafe { sys::SSL_CTX_set_security_level(t.ctx, level as c_int) };
    }
    if let Some(protos) = opts.string("alpn_protocols") {
        if !have_tlsext() {
            warn("alpn_protocols support is not compiled into the OpenSSL library against which PHP is linked");
        } else {
            let Some(alpn) = alpn_protos_parse(&protos) else {
                warn("Failed parsing comma-separated TLS ALPN protocol string");
                unsafe { sys::SSL_CTX_free(t.ctx) };
                t.ctx = ptr::null_mut();
                return -1;
            };
            if t.is_client {
                if unsafe {
                    sys::SSL_CTX_set_alpn_protos(t.ctx, alpn.as_ptr(), alpn.len() as c_uint)
                } != 0
                {
                    warn("Failed setting TLS ALPN protocols, protocol names must not be empty");
                    unsafe { sys::SSL_CTX_free(t.ctx) };
                    t.ctx = ptr::null_mut();
                    return -1;
                }
            } else {
                t.shared().alpn = alpn;
                unsafe {
                    sys::SSL_CTX_set_alpn_select_cb_php(
                        t.ctx,
                        Some(alpn_callback),
                        t.shared as *mut c_void,
                    )
                };
            }
        }
    }
    if !set_local_cert(t.ctx, opts) {
        return -1;
    }
    unsafe {
        sys::SSL_CTX_set_options(t.ctx, options);
        sys::SSL_CTX_set_min_proto_version(t.ctx, map_proto_version(min_proto_flag(method_flags)));
        sys::SSL_CTX_set_max_proto_version(t.ctx, map_proto_version(max_proto_flag(method_flags)));
    }
    if !t.is_client && opts.has_ctx && !set_server_specific_opts(t.ctx, opts) {
        return -1;
    }
    t.ssl = unsafe { sys::SSL_new(t.ctx) };
    if t.ssl.is_null()
        || unsafe { sys::SSL_set_ex_data(t.ssl, data_index(), t.shared as *mut c_void) } == 0
    {
        warn("SSL handle creation failure");
        unsafe { sys::SSL_CTX_free(t.ctx) };
        t.ctx = ptr::null_mut();
        t.shared().alpn.clear();
        return -1;
    }
    if unsafe { sys::SSL_set_fd(t.ssl, raw) } == 0 {
        let mut eof = stream.eof;
        let t = stream.tls.as_deref().expect("tls");
        handle_ssl_error(t, blocked, &mut eof, 0, true, errno());
        stream.eof = eof;
    }
    let t = stream.tls.as_deref_mut().expect("tls");
    if !t.is_client && have_tlsext() && !enable_server_sni(t, opts, verify_peer) {
        return -1;
    }
    if !t.is_client {
        // php_openssl_init_server_reneg_limit
        let limit = opts.long("reneg_limit").unwrap_or(DEFAULT_RENEG_LIMIT);
        if limit >= 0 {
            let window = opts.long("reneg_window").unwrap_or(DEFAULT_RENEG_WINDOW);
            t.shared().reneg = Some(Reneg {
                prev_handshake: 0,
                limit,
                window,
                tokens: 0.0,
                should_close: false,
                has_callback: opts.get("reneg_limit_callback").is_some(),
                callback_due: false,
            });
            unsafe { sys::SSL_set_info_callback(t.ssl, Some(info_callback)) };
        }
    }
    unsafe { sys::SSL_set_mode(t.ssl, sys::SSL_MODE_RELEASE_BUFFERS) };
    match session {
        None => {}
        Some(Session::NotSsl) => warn("Supplied session stream must be an SSL enabled stream"),
        Some(Session::Ssl(s)) if s.is_null() => {
            warn("Supplied SSL session stream is not initialized")
        }
        Some(Session::Ssl(s)) => unsafe {
            sys::SSL_copy_session_id(t.ssl, s);
        },
    }
    0
}

// ---------------------------------------------------------------------------
// the handshake
// ---------------------------------------------------------------------------

/// Peer certificates a handshake read for `capture_peer_cert` /
/// `capture_peer_cert_chain`, owned by the receiver.
#[derive(Default)]
pub struct Captured {
    pub cert: Option<*mut sys::X509>,
    /// `Some(None)`: the chain option was set but there was no chain.
    pub chain: Option<Option<Vec<*mut sys::X509>>>,
}

impl Drop for Captured {
    fn drop(&mut self) {
        unsafe {
            if let Some(c) = self.cert.take() {
                sys::X509_free(c);
            }
            if let Some(Some(chain)) = self.chain.take() {
                for c in chain {
                    sys::X509_free(c);
                }
            }
        }
    }
}

/// `php_openssl_set_blocking`.
fn set_blocking(raw: RawFd, sock: &mut SockInfo, block: bool) -> bool {
    let ok = crate::fdstream::set_fd_blocking(raw, block);
    if ok {
        sock.blocked = block;
    }
    ok
}

/// A timeval as php compares it.
fn tv_duration((s, us): (i64, i64)) -> Duration {
    Duration::from_secs(s.max(0) as u64) + Duration::from_micros(us.max(0) as u64)
}

fn tv_positive((s, us): (i64, i64)) -> bool {
    s > 0 || (s == 0 && us > 0)
}

fn tv_left(timeout: (i64, i64), elapsed: Duration) -> (i64, i64) {
    let left = tv_duration(timeout).saturating_sub(elapsed);
    (left.as_secs() as i64, left.subsec_micros() as i64)
}

/// `php_openssl_capture_peer_certs`, the OpenSSL half.
fn capture_peer_certs(t: &SslSock, peer: *mut sys::X509, opts: &Opts, out: &mut Captured) -> bool {
    let mut captured = false;
    if opts.is_true("capture_peer_cert") == Some(true) {
        out.cert = Some(peer);
        captured = true;
    }
    if opts.is_true("capture_peer_cert_chain") == Some(true) {
        let chain = unsafe { sys::SSL_get_peer_cert_chain(t.ssl) } as *const sys::OPENSSL_STACK;
        let n = if chain.is_null() {
            0
        } else {
            unsafe { sys::OPENSSL_sk_num(chain) }
        };
        out.chain = Some((n > 0).then(|| {
            (0..n)
                .map(|i| unsafe {
                    sys::X509_dup(sys::OPENSSL_sk_value(chain, i) as *mut sys::X509)
                })
                .collect()
        }));
    }
    captured
}

/// `php_openssl_enable_crypto`: 1 done, 0 would block, -1 failed.
pub fn enable_crypto(stream: &mut FdStream, opts: &Opts, activate: bool) -> (i32, Captured) {
    let mut cap = Captured::default();
    let Some(raw) = stream.raw_fd() else {
        return (-1, cap);
    };
    let active = stream.tls.as_ref().is_some_and(|t| t.active);
    if activate && !active {
        let (Some(t), Some(sock)) = (stream.tls.as_deref_mut(), stream.sock.as_mut()) else {
            return (-1, cap);
        };
        if t.ssl.is_null() {
            return (-1, cap);
        }
        let blocked = sock.blocked;
        if t.is_client && have_tlsext() && opts.is_true("SNI_enabled") != Some(false) {
            // php_openssl_enable_client_sni
            let name = opts.string("peer_name").or_else(|| t.url_name.clone());
            if let Some(name) = name {
                let c = cstring(&name);
                unsafe { sys::SSL_set_tlsext_host_name(t.ssl, c.as_ptr() as *mut c_char) };
            }
        }
        if !t.state_set {
            unsafe {
                if t.is_client {
                    sys::SSL_set_connect_state(t.ssl);
                } else {
                    sys::SSL_set_accept_state(t.ssl);
                }
            }
            t.state_set = true;
        }
        if set_blocking(raw, sock, false) {
            unsafe {
                sys::SSL_set_mode(
                    t.ssl,
                    sys::SSL_MODE_ENABLE_PARTIAL_WRITE | sys::SSL_MODE_ACCEPT_MOVING_WRITE_BUFFER,
                )
            };
        }
        // What the verify callback reads during the handshake.
        let sh = t.shared();
        sh.allow_self_signed = opts.is_true("allow_self_signed") == Some(true);
        sh.verify_depth = opts
            .long("verify_depth")
            .map_or(DEFAULT_VERIFY_DEPTH, |d| d as u64);

        let timeout = if t.is_client {
            sock.connect_timeout
        } else {
            sock.timeout.unwrap_or((-1, 0))
        };
        let has_timeout = !sock.blocked && tv_positive(timeout);
        let start = Instant::now();
        let mut n;
        let mut last_errno;
        let mut eof = stream.eof;
        loop {
            unsafe { sys::ERR_clear_error() };
            n = super::with_callbacks(|| unsafe {
                if t.is_client {
                    sys::SSL_connect(t.ssl)
                } else {
                    sys::SSL_accept(t.ssl)
                }
            });
            last_errno = errno();
            let elapsed = start.elapsed();
            if has_timeout && elapsed > tv_duration(timeout) {
                set_blocking(raw, sock, blocked);
                warn("SSL: Handshake timed out");
                stream.eof = eof;
                return (-1, cap);
            }
            if n <= 0 {
                let (retry, e) =
                    handle_ssl_error(t, sock.blocked, &mut eof, n, blocked, last_errno);
                last_errno = e;
                if !retry {
                    break;
                }
                let err = unsafe { sys::SSL_get_error(t.ssl, n) };
                let events = if err == sys::SSL_ERROR_WANT_READ {
                    libc::POLLIN | libc::POLLPRI
                } else {
                    libc::POLLOUT
                };
                poll_fd(raw, events, has_timeout.then(|| tv_left(timeout, elapsed)));
            } else {
                break;
            }
        }
        if sock.blocked != blocked {
            set_blocking(raw, sock, blocked);
        }
        let mut captured = false;
        let mut peer = ptr::null_mut();
        if n == 1 {
            peer = unsafe { sys::SSL_get1_peer_certificate(t.ssl) };
            if !peer.is_null() && opts.has_ctx {
                captured = capture_peer_certs(t, peer, opts, &mut cap);
            }
            if !apply_peer_verification_policy(t, peer, opts) {
                super::with_callbacks(|| unsafe { sys::SSL_shutdown(t.ssl) });
                n = -1;
            } else {
                t.active = true;
            }
        } else if last_errno == libc::EAGAIN {
            n = 0;
        } else {
            n = -1;
            peer = unsafe { sys::SSL_get1_peer_certificate(t.ssl) };
            if !peer.is_null() && opts.has_ctx {
                captured = capture_peer_certs(t, peer, opts, &mut cap);
            }
        }
        if !peer.is_null() && !captured {
            unsafe { sys::X509_free(peer) };
        }
        stream.eof = eof;
        return (n, cap);
    } else if !activate && active {
        if let Some(t) = stream.tls.as_deref_mut() {
            super::with_callbacks(|| unsafe { sys::SSL_shutdown(t.ssl) });
            t.active = false;
        }
    }
    (-1, cap)
}

// ---------------------------------------------------------------------------
// I/O
// ---------------------------------------------------------------------------

/// `php_openssl_sockop_read` with the session active: the bytes read,
/// `-1` on a timeout.
pub fn sockop_read(stream: &mut FdStream, buf: &mut [u8], supplemental: bool) -> isize {
    sockop_io(
        stream,
        true,
        buf.as_mut_ptr() as *mut c_void,
        buf.len(),
        supplemental,
    )
}

/// `php_openssl_sockop_write` with the session active: the bytes written,
/// `-1` on a timeout.
pub fn sockop_write(stream: &mut FdStream, buf: &[u8]) -> isize {
    // SSL_write only reads from the buffer
    sockop_io(stream, false, buf.as_ptr() as *mut c_void, buf.len(), false)
}

/// `php_openssl_sockop_io`: `len` bytes at `buf`, written to when `read`.
fn sockop_io(
    stream: &mut FdStream,
    read: bool,
    buf: *mut c_void,
    len: usize,
    supplemental: bool,
) -> isize {
    let Some(raw) = stream.raw_fd() else {
        return -1;
    };
    let mut eof = stream.eof;
    let (Some(t), Some(sock)) = (stream.tls.as_deref_mut(), stream.sock.as_mut()) else {
        return -1;
    };
    let mut retry = !supplemental;
    let began_blocked = sock.blocked;
    let count = len.min(c_int::MAX as usize) as c_int;
    let timeout = (began_blocked && !supplemental).then_some(sock.timeout.unwrap_or((-1, 0)));
    if timeout.is_some() || supplemental {
        set_blocking(raw, sock, false);
    }
    let has_timeout = !sock.blocked && timeout.is_some_and(tv_positive);
    let start = Instant::now();
    let mut nr_bytes: c_int;
    loop {
        let elapsed = start.elapsed();
        if has_timeout && elapsed > tv_duration(timeout.unwrap_or_default()) {
            if began_blocked {
                set_blocking(raw, sock, true);
            }
            sock.timed_out = true;
            stream.eof = eof;
            return -1;
        }
        unsafe { sys::ERR_clear_error() };
        let err_no;
        if read {
            nr_bytes = super::with_callbacks(|| unsafe { sys::SSL_read(t.ssl, buf, count) });
            err_no = errno();
            if let Some(r) = t.shared().reneg.as_mut().filter(|r| r.should_close) {
                if r.callback_due {
                    // php calls the callback from the handshake; rhp has no
                    // host here, so the code that ran the read calls it
                    r.callback_due = false;
                    stream.events.push(crate::fdstream::NotifyEvent::RenegLimit);
                    break;
                }
                unsafe { libc::shutdown(raw, libc::SHUT_RDWR) };
                nr_bytes = 0;
                eof = true;
                break;
            }
        } else {
            nr_bytes = super::with_callbacks(|| unsafe { sys::SSL_write(t.ssl, buf, count) });
            err_no = errno();
        }
        let left = has_timeout.then(|| tv_left(timeout.unwrap_or_default(), elapsed));
        if nr_bytes <= 0 {
            let err = unsafe { sys::SSL_get_error(t.ssl, nr_bytes) };
            let (r, e) = handle_ssl_error(t, sock.blocked, &mut eof, nr_bytes, false, err_no);
            retry = r;
            if e == libc::EAGAIN && err == sys::SSL_ERROR_WANT_READ && read {
                retry = true;
            }
            if e == libc::EAGAIN && err == sys::SSL_ERROR_WANT_WRITE && !read {
                retry = true;
            }
            if read {
                eof = !retry
                    && err != sys::SSL_ERROR_WANT_READ
                    && err != sys::SSL_ERROR_WANT_WRITE
                    && unsafe { sys::SSL_pending(t.ssl) } == 0;
            }
            if !began_blocked || supplemental {
                break;
            }
            if retry {
                let want_write = err == sys::SSL_ERROR_WANT_WRITE;
                let want_read = err == sys::SSL_ERROR_WANT_READ;
                let events = if (read && want_write) || (!read && !want_read) {
                    libc::POLLOUT | libc::POLLPRI
                } else {
                    libc::POLLIN | libc::POLLPRI
                };
                poll_fd(raw, events, left);
            }
        } else {
            let err = unsafe { sys::SSL_get_error(t.ssl, nr_bytes) };
            if err == sys::SSL_ERROR_NONE {
                break;
            }
            if began_blocked {
                let events = if (read && err == sys::SSL_ERROR_WANT_WRITE)
                    || (!read && err != sys::SSL_ERROR_WANT_READ)
                {
                    libc::POLLOUT | libc::POLLPRI
                } else {
                    libc::POLLIN | libc::POLLPRI
                };
                poll_fd(raw, events, left);
            }
        }
        if !retry {
            break;
        }
    }
    if began_blocked {
        set_blocking(raw, sock, true);
    }
    stream.eof = eof;
    nr_bytes.max(0) as isize
}

impl SslSock {
    /// The `reneg_limit_callback` answered: keep the connection (true), or
    /// close it as php does when the rate limit is exceeded.
    pub fn reneg_answered(&mut self, keep: bool) -> bool {
        let Some(r) = self.shared().reneg.as_mut() else {
            return true;
        };
        r.should_close = !keep;
        keep
    }

    /// `PHP_STREAM_OPTION_CHECK_LIVENESS` with a zero timeout (what `feof`
    /// asks) on an active session.
    pub fn alive(&self, raw: RawFd, sock: &SockInfo) -> bool {
        if poll_fd(raw, libc::POLLIN | libc::POLLPRI, Some((0, 0))) <= 0 {
            return true;
        }
        let began_blocked = sock.blocked;
        if began_blocked {
            crate::fdstream::set_fd_blocking(raw, false);
        }
        let mut b = 0u8;
        let n = super::with_callbacks(|| unsafe {
            sys::SSL_peek(self.ssl, &mut b as *mut u8 as *mut c_void, 1)
        });
        let alive = if n <= 0 {
            match unsafe { sys::SSL_get_error(self.ssl, n) } {
                sys::SSL_ERROR_SYSCALL => errno() == libc::EAGAIN,
                sys::SSL_ERROR_WANT_READ | sys::SSL_ERROR_WANT_WRITE => true,
                _ => false,
            }
        } else {
            true
        };
        if began_blocked {
            crate::fdstream::set_fd_blocking(raw, true);
        }
        alive
    }

    /// The `crypto` array of `stream_get_meta_data`.
    pub fn meta(&self) -> Value {
        let mut a = PhpArray::new();
        let put = |a: &mut PhpArray, k: &str, v: Value| {
            a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
        };
        let proto = match unsafe { sys::SSL_version(self.ssl) } {
            sys::TLS1_3_VERSION if have_tls13() => "TLSv1.3",
            sys::TLS1_2_VERSION if have_tls12() => "TLSv1.2",
            sys::TLS1_1_VERSION if have_tls11() => "TLSv1.1",
            sys::TLS1_VERSION => "TLSv1",
            sys::SSL3_VERSION if have_ssl3() => "SSLv3",
            _ => "UNKNOWN",
        };
        let cipher = unsafe { sys::SSL_get_current_cipher(self.ssl) };
        put(&mut a, "protocol", Value::str(proto));
        put(
            &mut a,
            "cipher_name",
            Value::str(cstr_bytes(unsafe { sys::SSL_CIPHER_get_name(cipher) })),
        );
        put(
            &mut a,
            "cipher_bits",
            Value::Int(unsafe { sys::SSL_CIPHER_get_bits(cipher, ptr::null_mut()) } as i64),
        );
        put(
            &mut a,
            "cipher_version",
            Value::str(cstr_bytes(unsafe { sys::SSL_CIPHER_get_version(cipher) })),
        );
        if have_tlsext() {
            let mut p: *const c_uchar = ptr::null();
            let mut len: c_uint = 0;
            unsafe { sys::SSL_get0_alpn_selected(self.ssl, &mut p, &mut len) };
            if !p.is_null() {
                let proto = unsafe { std::slice::from_raw_parts(p, len as usize) };
                put(&mut a, "alpn_protocol", Value::str(proto.to_vec()));
            }
        }
        Value::Array(a)
    }
}

/// `php_openssl_get_url_name`: the host of the transport address (after
/// `proto://`), trailing dots dropped.
pub fn url_name(resource: &[u8]) -> Option<Vec<u8>> {
    let u = crate::url::parse(resource)?;
    let mut host = u.host?;
    while host.last() == Some(&b'.') {
        host.pop();
    }
    (!host.is_empty()).then_some(host)
}

// ---------------------------------------------------------------------------
// the host side
// ---------------------------------------------------------------------------

/// The `ssl` options of the context `ctx` (none: a stream without one).
pub(crate) fn stream_opts(host: &dyn crate::Host, ctx: Option<&Value>) -> Opts {
    match ctx {
        Some(c) => Opts::new(true, &host.stream_context_options(c)),
        None => Opts::default(),
    }
}

/// `php_openssl_capture_peer_certs`, the php half: what the handshake
/// captured becomes `peer_certificate` / `peer_certificate_chain` options of
/// the context.
pub(crate) fn store_captured(
    host: &mut dyn crate::Host,
    ctx: Option<&Value>,
    mut cap: Captured,
) -> Result<(), crate::Thrown> {
    let Some(ctx) = ctx else {
        return Ok(());
    };
    if let Some(c) = cap.cert.take() {
        let obj = super::cert_object(host, c)?;
        host.stream_context_set_option(ctx, b"ssl", b"peer_certificate", obj);
    }
    if let Some(chain) = cap.chain.take() {
        let v = match chain {
            Some(list) => {
                let mut a = PhpArray::new();
                let mut rest = list.into_iter();
                while let Some(c) = rest.next() {
                    match super::cert_object(host, c) {
                        Ok(o) => a.push(o),
                        Err(e) => {
                            for c in rest {
                                unsafe { sys::X509_free(c) };
                            }
                            return Err(e);
                        }
                    }
                }
                Value::Array(a)
            }
            None => Value::Null,
        };
        host.stream_context_set_option(ctx, b"ssl", b"peer_certificate_chain", v);
    }
    Ok(())
}

/// A stream with php's openssl socket ops: tcp and the TLS transports.
pub(crate) fn has_tls_ops(f: &FdStream) -> bool {
    f.sock().is_some_and(|s| s.stream_type == "tcp_socket/ssl")
}

/// `php_stream_xport_crypto_setup` on a stream resource: 0, -1, or -2 when
/// the stream has no TLS ops.
pub(crate) fn crypto_setup(
    host: &mut dyn crate::Host,
    fname: &str,
    res: &Value,
    method: i64,
    session: Option<&Value>,
) -> Result<i32, crate::Thrown> {
    let ctx = host.stream_context_of(res);
    let opts = stream_opts(host, ctx.as_ref());
    let session = session.map(|s| match host.stream_fd_mut(s) {
        Some(f) if has_tls_ops(f) => {
            Session::Ssl(f.tls.as_ref().map_or(ptr::null_mut(), |t| t.handle()))
        }
        _ => Session::NotSsl,
    });
    let r = match host.stream_fd_mut(res) {
        Some(f) if has_tls_ops(f) => {
            if f.tls.is_none() {
                f.tls = Some(Box::new(SslSock::new(false, 0, None)));
            }
            Some(setup_crypto(f, &opts, method, session))
        }
        _ => None,
    };
    crate::flush_stream_diags(host, fname)?;
    match r {
        Some(r) => Ok(r),
        None => {
            crate::warn(host, fname, "This stream does not support SSL/crypto")?;
            Ok(-2)
        }
    }
}

/// `php_stream_xport_crypto_enable` on a stream resource.
pub(crate) fn crypto_enable(
    host: &mut dyn crate::Host,
    fname: &str,
    res: &Value,
    activate: bool,
) -> Result<i32, crate::Thrown> {
    let ctx = host.stream_context_of(res);
    let opts = stream_opts(host, ctx.as_ref());
    let r = match host.stream_fd_mut(res) {
        Some(f) if has_tls_ops(f) => {
            if f.tls.is_none() {
                f.tls = Some(Box::new(SslSock::new(false, 0, None)));
            }
            Some(enable_crypto(f, &opts, activate))
        }
        _ => None,
    };
    match r {
        Some((n, cap)) => {
            store_captured(host, ctx.as_ref(), cap)?;
            crate::flush_stream_diags(host, fname)?;
            Ok(n)
        }
        None => {
            crate::warn(host, fname, "This stream does not support SSL/crypto")?;
            Ok(-2)
        }
    }
}

/// Setup and enable on a stream not yet registered (a connect with
/// `enable_on_connect`, an accepted client): whether both worked.
pub(crate) fn crypto_on_connect(
    host: &mut dyn crate::Host,
    stream: &mut FdStream,
    ctx: Option<&Value>,
    method: i64,
) -> Result<bool, crate::Thrown> {
    let opts = stream_opts(host, ctx);
    let mut ok = setup_crypto(stream, &opts, method, None) >= 0;
    if ok {
        let (n, cap) = enable_crypto(stream, &opts, true);
        store_captured(host, ctx, cap)?;
        ok = n >= 0;
    }
    Ok(ok)
}

/// Setup and enable with options given rather than read from the context
/// (captured certificates still go to `ctx`).
pub(crate) fn crypto_with_opts(
    host: &mut dyn crate::Host,
    stream: &mut FdStream,
    ctx: &Value,
    opts: &Opts,
    method: i64,
) -> Result<bool, crate::Thrown> {
    if stream.tls.is_none() {
        stream.tls = Some(Box::new(SslSock::new(false, 0, None)));
    }
    let mut ok = setup_crypto(stream, opts, method, None) >= 0;
    if ok {
        let (n, cap) = enable_crypto(stream, opts, true);
        let ctx = (!matches!(ctx, Value::Null)).then_some(ctx);
        store_captured(host, ctx, cap)?;
        ok = n >= 0;
    }
    Ok(ok)
}

/// What php's openssl socket factory makes of transport `proto`:
/// `(enable_on_connect, crypto method)`, or `Err(())` once it warned (the
/// factory fails). `None` for a transport it does not register.
pub(crate) fn transport(proto: &[u8], opts: &Opts) -> Option<Result<(bool, i64), ()>> {
    // php_openssl_get_crypto_method
    let ctx_method = |default: i64| {
        opts.long("crypto_method")
            .map_or(default, |m| m | IS_CLIENT)
    };
    let missing = |name: &str| {
        warn(format!(
            "{name} support is not compiled into the OpenSSL library against which PHP is linked"
        ));
        Some(Err(()))
    };
    match proto {
        b"tcp" => Some(Ok((false, 0))),
        b"ssl" | b"tls" => Some(Ok((true, ctx_method(TLS_ANY_CLIENT)))),
        b"sslv3" if have_ssl3() => Some(Ok((true, SSLV3_CLIENT))),
        b"tlsv1.0" => Some(Ok((true, TLS1_0_CLIENT))),
        b"tlsv1.1" if have_tls11() => Some(Ok((true, TLS1_1_CLIENT))),
        b"tlsv1.1" => missing("TLSv1.1"),
        b"tlsv1.2" if have_tls12() => Some(Ok((true, TLS1_2_CLIENT))),
        b"tlsv1.2" => missing("TLSv1.2"),
        b"tlsv1.3" if have_tls13() => Some(Ok((true, TLS1_3_CLIENT))),
        b"tlsv1.3" => missing("TLSv1.3"),
        _ => None,
    }
}

/// `stream_get_transports()` past the plain socket ones.
pub(crate) fn transport_names() -> Vec<&'static str> {
    let mut v = vec!["ssl"];
    if have_ssl3() {
        v.push("sslv3");
    }
    v.extend(["tls", "tlsv1.0", "tlsv1.1", "tlsv1.2", "tlsv1.3"]);
    v
}

#[cfg(test)]
mod tests;
