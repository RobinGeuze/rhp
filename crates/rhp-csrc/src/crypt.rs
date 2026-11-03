//! php-src's crypt C: standard and extended DES (crypt_freesec.c), blowfish
//! (crypt_blowfish.c), SHA-256 and SHA-512 crypt (crypt_sha256.c,
//! crypt_sha512.c). Key and setting are C strings, as php passes them. Each
//! function returns the hash, or `None` where the C returns NULL.

use std::ffi::{c_char, c_int, CStr};
use std::sync::Once;

/// `PHP_MAX_SALT_LEN` (php_crypt.h).
pub const MAX_SALT_LEN: usize = 123;

extern "C" {
    fn _crypt_extended_init();
    fn rhp_crypt_des(key: *const c_char, setting: *const c_char, out: *mut c_char) -> c_int;
    fn php_crypt_blowfish_rn(
        key: *const c_char,
        setting: *const c_char,
        output: *mut c_char,
        size: c_int,
    ) -> *mut c_char;
    fn php_sha256_crypt_r(
        key: *const c_char,
        salt: *const c_char,
        buffer: *mut c_char,
        buflen: c_int,
    ) -> *mut c_char;
    fn php_sha512_crypt_r(
        key: *const c_char,
        salt: *const c_char,
        buffer: *mut c_char,
        buflen: c_int,
    ) -> *mut c_char;
}

/// The NUL-terminated string in `buf`, then `buf` wiped (php's
/// `ZEND_SECURE_ZERO` of its output buffers).
fn take(buf: &mut [c_char]) -> Vec<u8> {
    let out = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_bytes().to_vec();
    for b in buf.iter_mut() {
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    out
}

/// `_crypt_extended_r`: DES (2-character salt) or extended DES (`_` + 8).
pub fn des(key: &CStr, setting: &CStr) -> Option<Vec<u8>> {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe { _crypt_extended_init() });
    let mut out = [0 as c_char; 21];
    let ok = unsafe { rhp_crypt_des(key.as_ptr(), setting.as_ptr(), out.as_mut_ptr()) };
    let hash = take(&mut out);
    (ok != 0).then_some(hash)
}

/// `php_crypt_blowfish_rn` with php_crypt()'s `PHP_MAX_SALT_LEN + 1` buffer.
pub fn blowfish(key: &CStr, setting: &CStr) -> Option<Vec<u8>> {
    let mut out = [0 as c_char; MAX_SALT_LEN + 1];
    let res = unsafe {
        php_crypt_blowfish_rn(
            key.as_ptr(),
            setting.as_ptr(),
            out.as_mut_ptr(),
            out.len() as c_int,
        )
    };
    let hash = take(&mut out);
    (!res.is_null()).then_some(hash)
}

/// `php_sha256_crypt_r` with php_crypt()'s `PHP_MAX_SALT_LEN` buffer.
pub fn sha256(key: &CStr, salt: &CStr) -> Option<Vec<u8>> {
    let mut out = [0 as c_char; MAX_SALT_LEN];
    let res = unsafe {
        php_sha256_crypt_r(
            key.as_ptr(),
            salt.as_ptr(),
            out.as_mut_ptr(),
            out.len() as c_int,
        )
    };
    let hash = take(&mut out);
    (!res.is_null()).then_some(hash)
}

/// `php_sha512_crypt_r` with php_crypt()'s `PHP_MAX_SALT_LEN` buffer. A
/// 9-digit `rounds=` with a 16-character salt doesn't fit it, so that fails
/// as it does in php.
pub fn sha512(key: &CStr, salt: &CStr) -> Option<Vec<u8>> {
    let mut out = [0 as c_char; MAX_SALT_LEN];
    let res = unsafe {
        php_sha512_crypt_r(
            key.as_ptr(),
            salt.as_ptr(),
            out.as_mut_ptr(),
            out.len() as c_int,
        )
    };
    let hash = take(&mut out);
    (!res.is_null()).then_some(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &[u8]) -> std::ffi::CString {
        std::ffi::CString::new(s).unwrap()
    }

    // php-src ext/standard/config.m4's crypt checks and crypt_*.c self-tests.
    #[test]
    fn known_vectors() {
        let k = c(b"rasmuslerdorf");
        assert_eq!(des(&k, &c(b"rl")).as_deref(), Some(&b"rl.3StKT.4T8M"[..]));
        assert_eq!(
            des(&k, &c(b"_J9..rasm")).as_deref(),
            Some(&b"_J9..rasmBYk8r9AiWNc"[..])
        );
        assert_eq!(
            blowfish(&k, &c(b"$2a$07$rasmuslerd............")).as_deref(),
            Some(&b"$2a$07$rasmuslerd............nIdrcHdxcUxWomQX9j6kvERCFjTg7Ra"[..])
        );
        assert_eq!(
            sha256(&k, &c(b"$5$rounds=5000$usesomesillystri$")).as_deref(),
            Some(
                &b"$5$rounds=5000$usesomesillystri$KqJWpanXZHKq2BOB43TSaYhEWsQ1Lr5QNyPCDH/Tp.6"[..]
            )
        );
        assert_eq!(
            sha512(&k, &c(b"$6$rounds=5000$usesomesillystri$")).as_deref(),
            Some(&b"$6$rounds=5000$usesomesillystri$D4IrlXatmP7rx3P3InaxBeoomnAihCKRVQP22JZ6EY47Wc6BkroIuUUBOov1i.S5KPgErtP/EN5mcO.ChWQW21"[..])
        );
    }

    #[test]
    fn failures_are_none() {
        let k = c(b"x");
        assert_eq!(des(&k, &c(b"_........")), None);
        assert_eq!(blowfish(&k, &c(b"$2a$03$1234567891234567891234567")), None);
        assert_eq!(sha512(&k, &c(b"$6$rounds=999$abc$")), None);
        assert_eq!(sha256(&k, &c(b"$5$rounds=1000000000$abc$")), None);
    }

    #[test]
    fn concurrent_des() {
        let threads: Vec<_> = (0..8)
            .map(|i| {
                std::thread::spawn(move || {
                    for _ in 0..200 {
                        let k = c(format!("pw{i}").as_bytes());
                        let a = des(&k, &c(b"_J9..rasm")).unwrap();
                        let b = des(&k, &c(b"_J9..rasm")).unwrap();
                        assert_eq!(a, b);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
    }
}
