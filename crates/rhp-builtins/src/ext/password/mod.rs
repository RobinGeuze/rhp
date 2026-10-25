//! The `password_*` extension: PHP 8.5's password hashing API.
//!
//! Reimplemented in Rust under the differential-test framework. The two
//! algorithms PHP 8.5 ships are backed by faithful pure-Rust crates:
//!   * **bcrypt** (`$2y$`) via the `bcrypt` crate (its `$2b$` identifier is
//!     rewritten to PHP's `$2y$`), and
//!   * **argon2i / argon2id** via the `argon2` crate (standard PHC strings).
//!
//! `password_hash()` uses a random salt, so it is proven correct by
//! *cross-verification* (rhp-hash ↔ php-verify both ways) rather than byte
//! comparison; the deterministic surface (`password_get_info`,
//! `password_needs_rehash`, `password_verify` of fixed hashes, `password_algos`)
//! is byte-identical to the C ext/standard implementation.

use std::collections::HashMap;

use argon2::{Algorithm, Argon2, Params, Version};
use password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use rhp_values::{conv, ArrayKey, PhpArray, Value};
use rand_core::{OsRng, RngCore};

use crate::{Builtin, Host, Thrown};

// PHP defaults (ext/standard/php_password.h).
const BCRYPT_DEFAULT_COST: i64 = 12;
const ARGON2_MEMORY_COST: i64 = 64 << 10; // 65536 KiB
const ARGON2_TIME_COST: i64 = 4;
const ARGON2_THREADS: i64 = 1;

// libargon2 bounds (argon2.h) as used by php's range checks.
const ARGON2_MIN_MEMORY: i64 = 8;
const ARGON2_MAX_MEMORY: i64 = u32::MAX as i64;
const ARGON2_MIN_TIME: i64 = 1;
const ARGON2_MAX_TIME: i64 = u32::MAX as i64;
const ARGON2_MAX_LANES: i64 = 0x00FF_FFFF;

// ---------------------------------------------------------------------------
// value helpers
// ---------------------------------------------------------------------------

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn bytes_of(v: &Value) -> Vec<u8> {
    conv::to_string_bytes(&deref(v))
}

/// Look up an array option by string key, applying PHP's `zval_get_long`.
fn opt_long(options: Option<&Value>, key: &[u8]) -> Option<i64> {
    let arr = match options.map(deref) {
        Some(Value::Array(a)) => a,
        _ => return None,
    };
    arr.get(&ArrayKey::from_bytes(key)).map(|v| conv::to_int(&deref(v)))
}

fn verr(msg: impl Into<String>) -> Thrown {
    Thrown::error("ValueError", msg)
}

// ---------------------------------------------------------------------------
// algorithm identity
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Algo {
    Bcrypt,
    Argon2i,
    Argon2id,
}

/// `php_password_algo_find`: resolve a registered identifier.
fn algo_find(ident: &[u8]) -> Option<Algo> {
    match ident {
        b"2y" => Some(Algo::Bcrypt),
        b"argon2i" => Some(Algo::Argon2i),
        b"argon2id" => Some(Algo::Argon2id),
        _ => None,
    }
}

/// `php_password_algo_find_zval`: resolve the `$algo` argument of
/// `password_hash` / `password_needs_rehash` (string | int | null).
fn algo_find_zval(v: Option<&Value>) -> Option<Algo> {
    match v.map(|v| deref(v)) {
        None | Some(Value::Null) => Some(Algo::Bcrypt), // default
        Some(Value::Str(s)) => algo_find(s.as_bytes()),
        Some(Value::Int(n)) => algo_from_long(n),
        Some(Value::Bool(b)) => algo_from_long(b as i64),
        Some(Value::Float(f)) => algo_from_long(f as i64),
        _ => None,
    }
}

fn algo_from_long(n: i64) -> Option<Algo> {
    match n {
        0 | 1 => Some(Algo::Bcrypt),
        2 => Some(Algo::Argon2i),
        3 => Some(Algo::Argon2id),
        _ => None,
    }
}

/// `php_password_algo_extract_ident`: the identifier run between the first two
/// `$` separators (not requiring a leading `$`, matching the C source).
fn extract_ident(hash: &[u8]) -> Option<&[u8]> {
    if hash.len() < 3 {
        return None;
    }
    let rest = &hash[1..];
    let end = rest.iter().position(|&b| b == b'$')?;
    Some(&rest[..end])
}

/// `php_password_bcrypt_valid`: exactly 60 bytes beginning with `$2y$`.
fn bcrypt_valid(hash: &[u8]) -> bool {
    hash.len() == 60 && hash.starts_with(b"$2y$")
}

/// `php_password_algo_identify`: identify with a bcrypt default fallback.
/// Mirrors `php_password_algo_identify_ex(hash, bcrypt)` used by `password_verify`.
fn identify_default_bcrypt(hash: &[u8]) -> Algo {
    match extract_ident(hash).and_then(algo_find) {
        Some(Algo::Bcrypt) if bcrypt_valid(hash) => Algo::Bcrypt,
        Some(a @ (Algo::Argon2i | Algo::Argon2id)) => a, // argon2 has no `valid` callback
        _ => Algo::Bcrypt,
    }
}

/// `php_password_algo_identify_ex(hash, NULL)`: identify with no fallback
/// (used by `password_needs_rehash` and `password_get_info`).
fn identify_strict(hash: &[u8]) -> Option<Algo> {
    match extract_ident(hash).and_then(algo_find) {
        Some(Algo::Bcrypt) if bcrypt_valid(hash) => Some(Algo::Bcrypt),
        Some(a @ (Algo::Argon2i | Algo::Argon2id)) => Some(a),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// parameter extraction (C `sscanf` semantics)
// ---------------------------------------------------------------------------

/// Parse a C `long` at the start of `s` (optional ASCII whitespace, optional
/// sign, decimal digits). Returns the value and the number of bytes consumed,
/// or `None` if no digit was found.
fn scan_long(s: &[u8]) -> Option<(i64, usize)> {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let start = i;
    let mut val: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        val = val.saturating_mul(10).saturating_add((s[i] - b'0') as i64);
        i += 1;
    }
    if i == start {
        return None;
    }
    Some((if neg { -val } else { val }, i))
}

/// `php_password_bcrypt_get_cost`: `sscanf(hash, "$2y$%ld$")`, default `12`.
fn bcrypt_get_cost(hash: &[u8]) -> i64 {
    // The literal "$2y$" is guaranteed present by `bcrypt_valid` before this.
    match scan_long(&hash[4..]) {
        Some((cost, _)) => cost,
        None => BCRYPT_DEFAULT_COST,
    }
}

/// `extract_argon2_parameters`: `sscanf(p, "v=%ld$m=%ld,t=%ld,p=%ld")` after the
/// `$argon2i$` / `$argon2id$` prefix, leaving defaults for any field not parsed.
fn argon2_params(hash: &[u8]) -> (i64, i64, i64) {
    let mut m = ARGON2_MEMORY_COST;
    let mut t = ARGON2_TIME_COST;
    let mut p = ARGON2_THREADS;

    let rest = if let Some(r) = hash.strip_prefix(b"$argon2i$") {
        r
    } else if let Some(r) = hash.strip_prefix(b"$argon2id$") {
        r
    } else {
        return (m, t, p);
    };

    // The sscanf format is one contiguous string; a mismatch stops the scan and
    // leaves the remaining fields at their defaults.
    let step = |s: &[u8], lit: &[u8]| -> Option<(i64, usize)> {
        let s = s.strip_prefix(lit)?;
        let (v, used) = scan_long(s)?;
        Some((v, lit.len() + used))
    };

    let mut pos = 0usize;
    // v=%ld (value discarded)
    match step(&rest[pos..], b"v=") {
        Some((_, used)) => pos += used,
        None => return (m, t, p),
    }
    match step(&rest[pos..], b"$m=") {
        Some((v, used)) => {
            m = v;
            pos += used;
        }
        None => return (m, t, p),
    }
    match step(&rest[pos..], b",t=") {
        Some((v, used)) => {
            t = v;
            pos += used;
        }
        None => return (m, t, p),
    }
    if let Some((v, _)) = step(&rest[pos..], b",p=") {
        p = v;
    }
    (m, t, p)
}

// ---------------------------------------------------------------------------
// hashing
// ---------------------------------------------------------------------------

fn bcrypt_hash(password: &[u8], options: Option<&Value>) -> Result<Value, Thrown> {
    if password.contains(&0) {
        return Err(verr("Bcrypt password must not contain null character"));
    }
    let cost = opt_long(options, b"cost").unwrap_or(BCRYPT_DEFAULT_COST);
    if !(4..=31).contains(&cost) {
        return Err(verr(format!(
            "Invalid bcrypt cost parameter specified: {cost}"
        )));
    }
    // The `bcrypt` crate emits the `$2b$` identifier; PHP uses `$2y$` (an
    // equivalent variant producing identical digests).
    let hashed = bcrypt::hash(password, cost as u32)
        .map_err(|_| Thrown::error("Error", "Password hashing failed for unknown reason"))?;
    let out = if let Some(rest) = hashed.strip_prefix("$2b$") {
        format!("$2y${rest}")
    } else {
        hashed
    };
    Ok(Value::str(out))
}

fn argon2_hash(password: &[u8], options: Option<&Value>, ty: Algo) -> Result<Value, Thrown> {
    let m = opt_long(options, b"memory_cost").unwrap_or(ARGON2_MEMORY_COST);
    if !(ARGON2_MIN_MEMORY..=ARGON2_MAX_MEMORY).contains(&m) {
        return Err(verr("Memory cost is outside of allowed memory range"));
    }
    let t = opt_long(options, b"time_cost").unwrap_or(ARGON2_TIME_COST);
    if !(ARGON2_MIN_TIME..=ARGON2_MAX_TIME).contains(&t) {
        return Err(verr("Time cost is outside of allowed time range"));
    }
    let p = opt_long(options, b"threads").unwrap_or(ARGON2_THREADS);
    if p == 0 || p > ARGON2_MAX_LANES {
        return Err(verr("Invalid number of threads"));
    }
    // libargon2 rejects memory below 8 lanes (ARGON2_MEMORY_TOO_LITTLE).
    if m < 8 * p {
        return Err(verr("Memory cost is too small"));
    }

    let params = Params::new(m as u32, t as u32, p as u32, Some(Params::DEFAULT_OUTPUT_LEN))
        .map_err(|e| verr(e.to_string()))?;
    let alg = if ty == Algo::Argon2i {
        Algorithm::Argon2i
    } else {
        Algorithm::Argon2id
    };
    let argon = Argon2::new(alg, Version::V0x13, params);

    // PHP uses a 16-byte random salt.
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let salt = SaltString::encode_b64(&salt)
        .map_err(|e| verr(e.to_string()))?;

    let hash = argon
        .hash_password(password, &salt)
        .map_err(|e| verr(e.to_string()))?;
    Ok(Value::str(hash.to_string()))
}

// ---------------------------------------------------------------------------
// verification
// ---------------------------------------------------------------------------

fn bcrypt_verify(password: &[u8], hash: &[u8]) -> bool {
    let hash = match std::str::from_utf8(hash) {
        Ok(s) => s,
        Err(_) => return false,
    };
    // `crypt` (and PHP's verify) accept the `$2a$`, `$2b$`, `$2x$`, `$2y$`
    // variants, which all produce the same digest as the crate's `$2b$`.
    let normalized;
    let hash = if hash.len() >= 4
        && hash.is_char_boundary(4)
        && matches!(&hash[..4], "$2a$" | "$2x$" | "$2y$")
    {
        normalized = format!("$2b${}", &hash[4..]);
        normalized.as_str()
    } else {
        hash
    };
    bcrypt::verify(password, hash).unwrap_or(false)
}

fn argon2_verify(password: &[u8], hash: &[u8]) -> bool {
    let hash = match std::str::from_utf8(hash) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let parsed = match PasswordHash::new(hash) {
        Ok(p) => p,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password, &parsed)
        .is_ok()
}

// ---------------------------------------------------------------------------
// builtins
// ---------------------------------------------------------------------------

fn fn_password_hash(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let password = bytes_of(&args[0]);
    let algo = algo_find_zval(args.get(1)).ok_or_else(|| {
        verr("password_hash(): Argument #2 ($algo) must be a valid password hashing algorithm")
    })?;
    let options = args.get(2);
    match algo {
        Algo::Bcrypt => bcrypt_hash(&password, options),
        Algo::Argon2i => argon2_hash(&password, options, Algo::Argon2i),
        Algo::Argon2id => argon2_hash(&password, options, Algo::Argon2id),
    }
}

fn fn_password_verify(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let password = bytes_of(&args[0]);
    let hash = bytes_of(&args[1]);
    let ok = match identify_default_bcrypt(&hash) {
        Algo::Bcrypt => bcrypt_verify(&password, &hash),
        Algo::Argon2i | Algo::Argon2id => argon2_verify(&password, &hash),
    };
    Ok(Value::Bool(ok))
}

fn fn_password_needs_rehash(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let hash = bytes_of(&args[0]);
    let new_algo = match algo_find_zval(args.get(1)) {
        Some(a) => a,
        None => return Ok(Value::Bool(false)), // unknown new algorithm, never rehash
    };
    let old_algo = identify_strict(&hash);
    if old_algo != Some(new_algo) {
        return Ok(Value::Bool(true));
    }
    let options = args.get(2);
    let needs = match new_algo {
        Algo::Bcrypt => {
            let want = opt_long(options, b"cost").unwrap_or(BCRYPT_DEFAULT_COST);
            bcrypt_get_cost(&hash) != want
        }
        Algo::Argon2i | Algo::Argon2id => {
            let want_m = opt_long(options, b"memory_cost").unwrap_or(ARGON2_MEMORY_COST);
            let want_t = opt_long(options, b"time_cost").unwrap_or(ARGON2_TIME_COST);
            let want_p = opt_long(options, b"threads").unwrap_or(ARGON2_THREADS);
            let (m, t, p) = argon2_params(&hash);
            want_m != m || want_t != t || want_p != p
        }
    };
    Ok(Value::Bool(needs))
}

fn fn_password_get_info(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let hash = bytes_of(&args[0]);
    let mut out = PhpArray::new();

    let algo = identify_strict(&hash);
    match algo {
        None => {
            out.insert(ArrayKey::from_bytes(b"algo"), Value::Null);
            out.insert(ArrayKey::from_bytes(b"algoName"), Value::str("unknown"));
            out.insert(ArrayKey::from_bytes(b"options"), Value::Array(PhpArray::new()));
        }
        Some(a) => {
            // `algo` echoes the identifier extracted from the hash.
            let ident = extract_ident(&hash).unwrap_or(b"").to_vec();
            out.insert(ArrayKey::from_bytes(b"algo"), Value::str(ident));
            let name = match a {
                Algo::Bcrypt => "bcrypt",
                Algo::Argon2i => "argon2i",
                Algo::Argon2id => "argon2id",
            };
            out.insert(ArrayKey::from_bytes(b"algoName"), Value::str(name));

            let mut options = PhpArray::new();
            match a {
                Algo::Bcrypt => {
                    options.insert(ArrayKey::from_bytes(b"cost"), Value::Int(bcrypt_get_cost(&hash)));
                }
                Algo::Argon2i | Algo::Argon2id => {
                    let (m, t, p) = argon2_params(&hash);
                    options.insert(ArrayKey::from_bytes(b"memory_cost"), Value::Int(m));
                    options.insert(ArrayKey::from_bytes(b"time_cost"), Value::Int(t));
                    options.insert(ArrayKey::from_bytes(b"threads"), Value::Int(p));
                }
            }
            out.insert(ArrayKey::from_bytes(b"options"), Value::Array(options));
        }
    }
    Ok(Value::Array(out))
}

fn fn_password_algos(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut arr = PhpArray::new();
    for (i, name) in ["2y", "argon2i", "argon2id"].iter().enumerate() {
        arr.insert(ArrayKey::Int(i as i64), Value::str(*name));
    }
    Ok(Value::Array(arr))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(name, Builtin { name, func, min_args: min, max_args: max });
    }
    put(m, "password_hash", fn_password_hash, 2, Some(3));
    put(m, "password_verify", fn_password_verify, 2, Some(2));
    put(m, "password_needs_rehash", fn_password_needs_rehash, 2, Some(3));
    put(m, "password_get_info", fn_password_get_info, 1, Some(1));
    put(m, "password_algos", fn_password_algos, 0, Some(0));
}

/// `PASSWORD_*` constants (all algo identifiers are strings in PHP 8).
pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("PASSWORD_DEFAULT", Value::str("2y")),
        ("PASSWORD_BCRYPT", Value::str("2y")),
        ("PASSWORD_ARGON2I", Value::str("argon2i")),
        ("PASSWORD_ARGON2ID", Value::str("argon2id")),
        ("PASSWORD_BCRYPT_DEFAULT_COST", Value::Int(12)),
        ("PASSWORD_ARGON2_PROVIDER", Value::str("standard")),
        ("PASSWORD_ARGON2_DEFAULT_MEMORY_COST", Value::Int(65536)),
        ("PASSWORD_ARGON2_DEFAULT_TIME_COST", Value::Int(4)),
        ("PASSWORD_ARGON2_DEFAULT_THREADS", Value::Int(1)),
    ]
}
