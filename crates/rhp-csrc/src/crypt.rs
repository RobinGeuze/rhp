//! php-src's crypt C: standard and extended DES (crypt_freesec.c), blowfish
//! (crypt_blowfish.c), SHA-256 and SHA-512 crypt (crypt_sha256.c,
//! crypt_sha512.c). Key and setting are C strings, as php passes them. Each
//! function returns the hash, or `None` where the C returns NULL.

use std::ffi::{c_char, c_int, CStr};
use std::sync::Once;

/// `PHP_MAX_SALT_LEN` (php_crypt.h).
pub const MAX_SALT_LEN: usize = 123;

/// The `*_crypt_r`/`_rn` signature: key, setting, output buffer, its size.
type CryptR = unsafe extern "C" fn(*const c_char, *const c_char, *mut c_char, c_int) -> *mut c_char;

// The C symbols carry glue/prefix_crypt.h's rhp_ prefix.
extern "C" {
    #[link_name = "rhp_crypt_extended_init"]
    fn _crypt_extended_init();
    fn rhp_crypt_des(key: *const c_char, setting: *const c_char, out: *mut c_char) -> c_int;
    #[link_name = "rhp_php_crypt_blowfish_rn"]
    fn php_crypt_blowfish_rn(
        key: *const c_char,
        setting: *const c_char,
        output: *mut c_char,
        size: c_int,
    ) -> *mut c_char;
    #[link_name = "rhp_php_sha256_crypt_r"]
    fn php_sha256_crypt_r(
        key: *const c_char,
        salt: *const c_char,
        buffer: *mut c_char,
        buflen: c_int,
    ) -> *mut c_char;
    #[link_name = "rhp_php_sha512_crypt_r"]
    fn php_sha512_crypt_r(
        key: *const c_char,
        salt: *const c_char,
        buffer: *mut c_char,
        buflen: c_int,
    ) -> *mut c_char;
}

/// The hash in `buf` if the C succeeded, then `buf` wiped (php's
/// `ZEND_SECURE_ZERO` of its output buffers). A failed call may leave `buf`
/// full with no NUL, so it is read only on success, and never past its end.
fn take(buf: &mut [u8], ok: bool) -> Option<Vec<u8>> {
    let hash = ok
        .then(|| CStr::from_bytes_until_nul(buf).ok())
        .flatten()
        .map(|s| s.to_bytes().to_vec());
    for b in buf.iter_mut() {
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    hash
}

/// Calls `f` with `buf` as its output buffer.
fn crypt_r(f: CryptR, key: &CStr, setting: &CStr, buf: &mut [u8]) -> Option<Vec<u8>> {
    let size = c_int::try_from(buf.len()).ok()?;
    let res = unsafe {
        f(
            key.as_ptr(),
            setting.as_ptr(),
            buf.as_mut_ptr().cast(),
            size,
        )
    };
    take(buf, !res.is_null())
}

/// `_crypt_extended_r`: DES (2-character salt) or extended DES (`_` + 8).
pub fn des(key: &CStr, setting: &CStr) -> Option<Vec<u8>> {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe { _crypt_extended_init() });
    let mut out = [0u8; 21];
    let ok = unsafe { rhp_crypt_des(key.as_ptr(), setting.as_ptr(), out.as_mut_ptr().cast()) };
    take(&mut out, ok != 0)
}

/// `php_crypt_blowfish_rn` with php_crypt()'s `PHP_MAX_SALT_LEN + 1` buffer.
pub fn blowfish(key: &CStr, setting: &CStr) -> Option<Vec<u8>> {
    crypt_r(
        php_crypt_blowfish_rn,
        key,
        setting,
        &mut [0; MAX_SALT_LEN + 1],
    )
}

/// `php_sha256_crypt_r` with php_crypt()'s `PHP_MAX_SALT_LEN` buffer.
pub fn sha256(key: &CStr, salt: &CStr) -> Option<Vec<u8>> {
    crypt_r(php_sha256_crypt_r, key, salt, &mut [0; MAX_SALT_LEN])
}

/// `php_sha512_crypt_r` with php_crypt()'s `PHP_MAX_SALT_LEN` buffer. A
/// 9-digit `rounds=` with a 16-character salt doesn't fit it, so that fails
/// as it does in php.
pub fn sha512(key: &CStr, salt: &CStr) -> Option<Vec<u8>> {
    crypt_r(php_sha512_crypt_r, key, salt, &mut [0; MAX_SALT_LEN])
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

    // "$6$rounds=1000$" + 16-char salt + "$" + 86 = 118 bytes. php's 123-byte
    // buffer hits the same edge with a 9-digit rounds=.
    #[test]
    fn sha512_result_that_does_not_fit() {
        let (k, s) = (c(b"x"), c(b"$6$rounds=1000$abcdefghijklmnop"));
        let mut buf = [b'X'; 200];
        assert_eq!(crypt_r(php_sha512_crypt_r, &k, &s, &mut buf[..118]), None);
        assert!(buf[..118].iter().all(|&b| b == 0), "wiped");
        assert!(buf[118..].iter().all(|&b| b == b'X'), "C stayed in bounds");

        let hash = crypt_r(php_sha512_crypt_r, &k, &s, &mut [0; 119]).unwrap();
        assert_eq!(hash.len(), 118);
        assert_eq!(sha512(&k, &s), Some(hash));
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
