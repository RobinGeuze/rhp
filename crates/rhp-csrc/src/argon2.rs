//! The argon2 reference C, release 20190702: the libargon2 php's password.c
//! calls (`HAVE_ARGON2LIB`). Only the four entry points php uses.

use std::ffi::{c_char, c_int, c_void, CStr};

/// `argon2_type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Type {
    I = 1,
    Id = 2,
}

/// `ARGON2_OK`.
pub const OK: i32 = 0;

extern "C" {
    static rhp_argon2_min_memory: u64;
    static rhp_argon2_max_memory: u64;
    static rhp_argon2_min_time: u64;
    static rhp_argon2_max_time: u64;
    static rhp_argon2_max_lanes: u64;
    static rhp_argon2_version_number: u32;

    fn argon2_hash(
        t_cost: u32,
        m_cost: u32,
        parallelism: u32,
        pwd: *const c_void,
        pwdlen: usize,
        salt: *const c_void,
        saltlen: usize,
        hash: *mut c_void,
        hashlen: usize,
        encoded: *mut c_char,
        encodedlen: usize,
        ty: c_int,
        version: u32,
    ) -> c_int;
    fn argon2_verify(encoded: *const c_char, pwd: *const c_void, pwdlen: usize, ty: c_int)
        -> c_int;
    fn argon2_error_message(error_code: c_int) -> *const c_char;
    fn argon2_encodedlen(
        t_cost: u32,
        m_cost: u32,
        parallelism: u32,
        saltlen: u32,
        hashlen: u32,
        ty: c_int,
    ) -> usize;
}

/// argon2.h's `ARGON2_MIN_MEMORY`, `ARGON2_MAX_MEMORY` (KiB).
pub fn memory_range() -> (u64, u64) {
    unsafe { (rhp_argon2_min_memory, rhp_argon2_max_memory) }
}

/// argon2.h's `ARGON2_MIN_TIME`, `ARGON2_MAX_TIME`.
pub fn time_range() -> (u64, u64) {
    unsafe { (rhp_argon2_min_time, rhp_argon2_max_time) }
}

/// argon2.h's `ARGON2_MAX_LANES`.
pub fn max_lanes() -> u64 {
    unsafe { rhp_argon2_max_lanes }
}

/// php_password_argon2_hash's `argon2_encodedlen` + `argon2_hash` with
/// `ARGON2_VERSION_NUMBER`: the encoded hash, or the argon2 error code.
pub fn hash(
    t_cost: u32,
    m_cost: u32,
    threads: u32,
    pwd: &[u8],
    salt: &[u8],
    hashlen: usize,
    ty: Type,
) -> Result<Vec<u8>, i32> {
    let encodedlen = unsafe {
        argon2_encodedlen(
            t_cost,
            m_cost,
            threads,
            salt.len() as u32,
            hashlen as u32,
            ty as c_int,
        )
    };
    let mut out = vec![0u8; hashlen];
    let mut encoded = vec![0u8; encodedlen];
    let status = unsafe {
        argon2_hash(
            t_cost,
            m_cost,
            threads,
            pwd.as_ptr().cast(),
            pwd.len(),
            salt.as_ptr().cast(),
            salt.len(),
            out.as_mut_ptr().cast(),
            out.len(),
            encoded.as_mut_ptr().cast(),
            encoded.len(),
            ty as c_int,
            rhp_argon2_version_number,
        )
    };
    if status != OK {
        return Err(status);
    }
    let encoded = CStr::from_bytes_until_nul(&encoded).expect("argon2_hash writes a C string");
    Ok(encoded.to_bytes().to_vec())
}

/// `argon2_verify(encoded, pwd, pwdlen, type) == ARGON2_OK`.
pub fn verify(encoded: &CStr, pwd: &[u8], ty: Type) -> bool {
    unsafe {
        argon2_verify(
            encoded.as_ptr(),
            pwd.as_ptr().cast(),
            pwd.len(),
            ty as c_int,
        ) == OK
    }
}

/// `argon2_error_message`.
pub fn error_message(code: i32) -> String {
    unsafe { CStr::from_ptr(argon2_error_message(code)) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify() {
        let h = hash(2, 1024, 1, b"password", b"somesalt", 32, Type::Id).unwrap();
        assert_eq!(
            h,
            b"$argon2id$v=19$m=1024,t=2,p=1$c29tZXNhbHQ$7FfsnA6vUe7qLpL/3Kqc3uR48ZJyFbUVt7jWZlf0Htk".to_vec()
        );
        let c = std::ffi::CString::new(h).unwrap();
        assert!(verify(&c, b"password", Type::Id));
        assert!(!verify(&c, b"Password", Type::Id));
        assert!(!verify(&c, b"password", Type::I));
    }

    #[test]
    fn errors() {
        assert_eq!(hash(1, 8, 2, b"p", b"somesalt", 32, Type::I), Err(-14));
        assert_eq!(error_message(-14), "Memory cost is too small");
        assert_eq!(memory_range(), (8, 0xFFFF_FFFF));
        assert_eq!(max_lanes(), 0xFF_FFFF);
    }
}
