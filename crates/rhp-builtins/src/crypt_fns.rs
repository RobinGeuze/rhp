//! `crypt()`: ext/standard/crypt.c on php-src's own crypt C (rhp-csrc).
//! MD5-crypt is ported here (php_crypt_r.c's `php_md5_crypt_r`) instead of
//! vendoring php_crypt_r.c and md5.c.

use std::ffi::CString;

use md5::{Digest, Md5};
use rhp_csrc::crypt as c;
use rhp_values::Value;

use crate::argparse::str_param;
use crate::{Host, Thrown};

/// The C string `s` points at: everything before the first NUL.
fn c_str(s: &[u8]) -> &[u8] {
    s.iter().position(|&b| b == 0).map_or(s, |n| &s[..n])
}

/// `IS_VALID_SALT_CHARACTER`: a DES salt character.
fn is_valid_salt_char(c: u8) -> bool {
    matches!(c, b'.'..=b'9' | b'A'..=b'Z' | b'a'..=b'z')
}

/// `php_crypt()`: the hash, or `None` where php returns NULL. `password` and
/// `salt` are read as C strings, as php does.
pub(crate) fn php_crypt(password: &[u8], salt: &[u8]) -> Option<Vec<u8>> {
    let password = c_str(password);
    let salt = c_str(salt);
    // salt[i], with the terminating NUL (and nothing past it is ever read).
    let at = |i: usize| salt.get(i).copied().unwrap_or(0);

    if at(0) == b'*' && (at(1) == b'0' || at(1) == b'1') {
        return None;
    }
    if salt.starts_with(b"$1$") {
        return Some(md5_crypt(password, salt));
    }
    // Neither has an interior NUL any more.
    let key = CString::new(password).ok()?;
    let setting = CString::new(salt).ok()?;
    if salt.starts_with(b"$6$") {
        c::sha512(&key, &setting)
    } else if salt.starts_with(b"$5$") {
        c::sha256(&key, &setting)
    } else if at(0) == b'$' && at(1) == b'2' && at(2) != 0 && at(3) == b'$' {
        c::blowfish(&key, &setting)
    } else if at(0) == b'_' || (is_valid_salt_char(at(0)) && is_valid_salt_char(at(1))) {
        c::des(&key, &setting)
    } else {
        None
    }
}

const MD5_MAGIC: &[u8] = b"$1$";
const ITOA64: &[u8; 64] = b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// `php_md5_crypt_r` (php_crypt_r.c), the FreeBSD md5crypt:
///
/// "THE BEER-WARE LICENSE" (Revision 42): <phk@login.dknet.dk> wrote this
/// file. As long as you retain this notice you can do whatever you want with
/// this stuff. If we meet some day, and you think this stuff is worth it, you
/// can buy me a beer in return. Poul-Henning Kamp
fn md5_crypt(pw: &[u8], salt: &[u8]) -> Vec<u8> {
    let sp = salt.strip_prefix(MD5_MAGIC).unwrap_or(salt);
    // The salt stops at the first '$', and is at most 8 characters.
    let sl = sp
        .iter()
        .take(8)
        .position(|&b| b == b'$')
        .unwrap_or(sp.len().min(8));
    let sp = &sp[..sl];

    let mut ctx = Md5::new();
    ctx.update(pw);
    ctx.update(MD5_MAGIC);
    ctx.update(sp);

    let mut ctx1 = Md5::new();
    ctx1.update(pw);
    ctx1.update(sp);
    ctx1.update(pw);
    let mut fin: [u8; 16] = ctx1.finalize().into();
    for chunk in pw.chunks(16) {
        ctx.update(&fin[..chunk.len()]);
    }
    fin = [0; 16];

    let mut i = pw.len();
    while i != 0 {
        if i & 1 != 0 {
            ctx.update(&fin[..1]);
        } else {
            ctx.update(&pw[..1]);
        }
        i >>= 1;
    }
    fin = ctx.finalize().into();

    for i in 0..1000 {
        let mut ctx1 = Md5::new();
        if i & 1 != 0 {
            ctx1.update(pw);
        } else {
            ctx1.update(fin);
        }
        if i % 3 != 0 {
            ctx1.update(sp);
        }
        if i % 7 != 0 {
            ctx1.update(pw);
        }
        if i & 1 != 0 {
            ctx1.update(fin);
        } else {
            ctx1.update(pw);
        }
        fin = ctx1.finalize().into();
    }

    let mut out = Vec::with_capacity(MD5_MAGIC.len() + sl + 1 + 22);
    out.extend_from_slice(MD5_MAGIC);
    out.extend_from_slice(sp);
    out.push(b'$');
    let mut to64 = |mut v: u32, n: usize| {
        for _ in 0..n {
            out.push(ITOA64[(v & 0x3f) as usize]);
            v >>= 6;
        }
    };
    let f = |i: usize| fin[i] as u32;
    to64((f(0) << 16) | (f(6) << 8) | f(12), 4);
    to64((f(1) << 16) | (f(7) << 8) | f(13), 4);
    to64((f(2) << 16) | (f(8) << 8) | f(14), 4);
    to64((f(3) << 16) | (f(9) << 8) | f(15), 4);
    to64((f(4) << 16) | (f(10) << 8) | f(5), 4);
    to64(f(11), 2);
    out
}

/// `crypt(string $string, string $salt): string`.
fn fn_crypt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let password = str_param(host, args, 0, "crypt", "string")?;
    let salt_in = str_param(host, args, 1, "crypt", "salt")?;
    // php copies at most PHP_MAX_SALT_LEN bytes into a NUL-terminated buffer.
    let salt = &salt_in[..salt_in.len().min(c::MAX_SALT_LEN)];
    Ok(Value::str(match php_crypt(&password, salt) {
        Some(hash) => hash,
        None if salt.starts_with(b"*0") => b"*1".to_vec(),
        None => b"*0".to_vec(),
    }))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("crypt", fn_crypt)]);
}

/// `CRYPT_*` (basic_functions.stub.php): every algorithm is always there.
pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("CRYPT_SALT_LENGTH", Value::Int(c::MAX_SALT_LEN as i64)),
        ("CRYPT_STD_DES", Value::Int(1)),
        ("CRYPT_EXT_DES", Value::Int(1)),
        ("CRYPT_MD5", Value::Int(1)),
        ("CRYPT_BLOWFISH", Value::Int(1)),
        ("CRYPT_SHA256", Value::Int(1)),
        ("CRYPT_SHA512", Value::Int(1)),
    ]
}
