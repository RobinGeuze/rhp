//! The `password_*` functions: ext/standard/password.c. bcrypt goes through
//! `php_crypt` (crypt_blowfish.c), so `password_verify` also accepts every
//! other `crypt()` hash as php does; argon2i and argon2id go through the
//! argon2 reference C php links. Both C libraries come from rhp-csrc.

use std::ffi::CString;

use rhp_csrc::argon2;
use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::ext::hash::primitives::base64_encode as base64;
use crate::ext::standard::crypt::php_crypt;
use crate::zend::zpp::{str_param, zval_value_name};
use crate::{Host, Thrown, E_DEPRECATED};

// php_password.h
const BCRYPT_COST: i64 = 12;
const ARGON2_MEMORY_COST: i64 = 64 << 10;
const ARGON2_TIME_COST: i64 = 4;
const ARGON2_THREADS: i64 = 1;

fn value_error(msg: impl Into<String>) -> Thrown {
    Thrown::error("ValueError", msg)
}

/// The C string `s` points at: everything before the first NUL.
fn c_str(s: &[u8]) -> &[u8] {
    s.iter().position(|&b| b == 0).map_or(s, |n| &s[..n])
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Algo {
    Bcrypt,
    Argon2i,
    Argon2id,
}

/// The registered algorithms, in registration order (`password_algos()`).
const ALGOS: [(&[u8], Algo); 3] = [
    (b"2y", Algo::Bcrypt),
    (b"argon2i", Algo::Argon2i),
    (b"argon2id", Algo::Argon2id),
];

impl Algo {
    fn name(self) -> &'static str {
        match self {
            Algo::Bcrypt => "bcrypt",
            Algo::Argon2i => "argon2i",
            Algo::Argon2id => "argon2id",
        }
    }

    /// The algorithm's `valid` callback; argon2 has none.
    fn valid(self, hash: &[u8]) -> bool {
        match self {
            Algo::Bcrypt => bcrypt_valid(hash),
            Algo::Argon2i | Algo::Argon2id => true,
        }
    }
}

/// `php_password_algo_find`.
fn algo_find(ident: &[u8]) -> Option<Algo> {
    ALGOS.iter().find(|(i, _)| *i == ident).map(|&(_, a)| a)
}

/// `php_password_algo_find_zval` on a `string|int|null $algo` argument
/// (`Z_PARAM_STR_OR_LONG_OR_NULL`).
fn algo_find_zval(
    host: &mut dyn Host,
    func: &str,
    arg: Option<&Value>,
) -> Result<Option<Algo>, Thrown> {
    let long = match arg.map(deref) {
        None | Some(Value::Null) => return Ok(Some(Algo::Bcrypt)),
        Some(Value::Str(s)) => return Ok(algo_find(s.as_bytes())),
        Some(Value::Int(n)) => n,
        Some(Value::Bool(b)) => b as i64,
        Some(Value::Float(f)) => {
            // zend_parse_arg_long_weak; a float that can't be an int is
            // taken as a string, which names no algorithm.
            const TWO_63: f64 = 9_223_372_036_854_775_808.0;
            if !(f.is_finite() && (-TWO_63..TWO_63).contains(&f)) {
                return Ok(None);
            }
            if f.fract() != 0.0 {
                let msg = format!(
                    "Implicit conversion from float {} to int loses precision",
                    String::from_utf8_lossy(&rhp_values::fmt::float_serialize(f))
                );
                host.emit_diagnostic(E_DEPRECATED, &msg)?;
            }
            f as i64
        }
        Some(v) => {
            return Err(Thrown::type_error(format!(
                "{func}(): Argument #2 ($algo) must be of type string|int|null, {} given",
                zval_value_name(host, &v)
            )))
        }
    };
    Ok(match long {
        0 | 1 => Some(Algo::Bcrypt),
        2 => Some(Algo::Argon2i),
        3 => Some(Algo::Argon2id),
        _ => None,
    })
}

/// `php_password_algo_extract_ident`: from after the first byte up to the
/// next `$` (strchr, so a NUL first means none).
fn extract_ident(hash: &[u8]) -> Option<&[u8]> {
    if hash.len() < 3 {
        return None;
    }
    let ident = c_str(&hash[1..]);
    let end = ident.iter().position(|&b| b == b'$')?;
    Some(&ident[..end])
}

/// `php_password_algo_identify_ex`.
fn identify_ex(hash: &[u8], default: Option<Algo>) -> Option<Algo> {
    let Some(ident) = extract_ident(hash) else {
        return default;
    };
    match algo_find(ident) {
        Some(a) if a.valid(hash) => Some(a),
        _ => default,
    }
}

/// `zval_get_long` of `options[key]`, if set.
fn option_long(options: Option<&PhpArray>, key: &[u8]) -> Option<i64> {
    options?
        .get(&ArrayKey::from_bytes(key))
        .map(|v| conv::to_int(&deref(v)))
}

/// A C `sscanf` `%ld`: optional whitespace and sign, then decimal digits.
/// The value and the bytes consumed, or `None` without a digit.
fn scan_long(s: &[u8]) -> Option<(i64, usize)> {
    let mut i = s
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
        .unwrap_or(s.len());
    let neg = match s.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let start = i;
    let mut val: i64 = 0;
    while let Some(d) = s.get(i).filter(|b| b.is_ascii_digit()) {
        let d = (d - b'0') as i64;
        val = if neg {
            val.saturating_mul(10).saturating_sub(d)
        } else {
            val.saturating_mul(10).saturating_add(d)
        };
        i += 1;
    }
    (i > start).then_some((val, i))
}

/// `sscanf(s, fmt, ...)` for a format of literals and `%ld`s, `None`
/// standing for each `%ld`: fills `out` in order until the first mismatch.
fn sscanf_longs(s: &[u8], fmt: &[Option<&[u8]>], out: &mut [&mut i64]) {
    let s = c_str(s);
    let mut pos = 0;
    let mut n = 0;
    for part in fmt {
        match part {
            Some(lit) => {
                if !s[pos..].starts_with(lit) {
                    return;
                }
                pos += lit.len();
            }
            None => {
                let Some((v, used)) = scan_long(&s[pos..]) else {
                    return;
                };
                *out[n] = v;
                n += 1;
                pos += used;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// salts
// ---------------------------------------------------------------------------

/// `php_password_get_salt`: warns about a `salt` option, then
/// `php_password_make_salt(length)`: `length` characters of the base64 of
/// `length * 3 / 4 + 1` random bytes, `+` replaced by `.`.
fn get_salt(
    host: &mut dyn Host,
    length: usize,
    options: Option<&PhpArray>,
) -> Result<Vec<u8>, Thrown> {
    if options.is_some_and(|o| o.get(&ArrayKey::from_bytes(b"salt")).is_some()) {
        crate::warn(
            host,
            "password_hash",
            "The \"salt\" option has been ignored, since providing a custom salt is no longer supported",
        )?;
    }
    let mut raw = vec![0u8; length * 3 / 4 + 1];
    getrandom::getrandom(&mut raw).map_err(|_| value_error("Unable to generate salt"))?;
    let enc = base64(&raw);
    let salt = &enc[..length];
    if salt.contains(&b'=') {
        return Err(value_error("Generated salt too short"));
    }
    Ok(salt
        .iter()
        .map(|&b| if b == b'+' { b'.' } else { b })
        .collect())
}

// ---------------------------------------------------------------------------
// bcrypt
// ---------------------------------------------------------------------------

/// `php_password_bcrypt_valid`.
fn bcrypt_valid(hash: &[u8]) -> bool {
    hash.len() == 60 && hash.starts_with(b"$2y")
}

/// `sscanf(hash, "$2y$%ld$", &cost)`.
fn bcrypt_cost(hash: &[u8]) -> i64 {
    let mut cost = BCRYPT_COST;
    sscanf_longs(hash, &[Some(b"$2y$"), None], &mut [&mut cost]);
    cost
}

fn bcrypt_hash(
    host: &mut dyn Host,
    password: &[u8],
    options: Option<&PhpArray>,
) -> Result<Option<Vec<u8>>, Thrown> {
    if password.contains(&0) {
        return Err(value_error(
            "Bcrypt password must not contain null character",
        ));
    }
    let cost = option_long(options, b"cost").unwrap_or(BCRYPT_COST);
    if !(4..=31).contains(&cost) {
        return Err(value_error(format!(
            "Invalid bcrypt cost parameter specified: {cost}"
        )));
    }
    let mut setting = format!("$2y${cost:02}$").into_bytes();
    setting.extend(get_salt(host, 22, options)?);
    Ok(php_crypt(password, &setting).filter(|h| h.len() >= 13))
}

fn bcrypt_verify(password: &[u8], hash: &[u8]) -> bool {
    match php_crypt(password, hash) {
        Some(ret) => hash.len() >= 13 && constant_time_eq(&ret, hash),
        None => false,
    }
}

/// `php_safe_bcmp() == 0`.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---------------------------------------------------------------------------
// argon2
// ---------------------------------------------------------------------------

fn argon2_type(algo: Algo) -> argon2::Type {
    if algo == Algo::Argon2i {
        argon2::Type::I
    } else {
        argon2::Type::Id
    }
}

/// `extract_argon2_parameters`: `(memory_cost, time_cost, threads)`,
/// starting from `init`, overwritten by what
/// `sscanf("v=%ld$m=%ld,t=%ld,p=%ld")` reads after the prefix.
fn argon2_params(hash: &[u8], init: (i64, i64, i64)) -> (i64, i64, i64) {
    let (mut m, mut t, mut p) = init;
    if hash.len() < b"$argon2id$".len() + 1 {
        return (m, t, p);
    }
    let Some(rest) = hash
        .strip_prefix(b"$argon2i$")
        .or_else(|| hash.strip_prefix(b"$argon2id$"))
    else {
        return (m, t, p);
    };
    let mut v = 0;
    sscanf_longs(
        rest,
        &[
            Some(b"v="),
            None,
            Some(b"$m="),
            None,
            Some(b",t="),
            None,
            Some(b",p="),
            None,
        ],
        &mut [&mut v, &mut m, &mut t, &mut p],
    );
    (m, t, p)
}

fn argon2_hash(
    host: &mut dyn Host,
    password: &[u8],
    options: Option<&PhpArray>,
    algo: Algo,
) -> Result<Option<Vec<u8>>, Thrown> {
    // php holds these in size_t: a negative value is out of range.
    let in_range = |v: i64, (lo, hi): (u64, u64)| v >= 0 && (lo..=hi).contains(&(v as u64));

    let memory = option_long(options, b"memory_cost").unwrap_or(ARGON2_MEMORY_COST);
    if !in_range(memory, argon2::memory_range()) {
        return Err(value_error(
            "Memory cost is outside of allowed memory range",
        ));
    }
    let time = option_long(options, b"time_cost").unwrap_or(ARGON2_TIME_COST);
    if !in_range(time, argon2::time_range()) {
        return Err(value_error("Time cost is outside of allowed time range"));
    }
    let threads = option_long(options, b"threads").unwrap_or(ARGON2_THREADS);
    if !in_range(threads, (1, argon2::max_lanes())) {
        return Err(value_error("Invalid number of threads"));
    }
    let salt = get_salt(host, 16, options)?;
    argon2::hash(
        time as u32,
        memory as u32,
        threads as u32,
        password,
        &salt,
        32,
        argon2_type(algo),
    )
    .map(Some)
    .map_err(|status| value_error(argon2::error_message(status)))
}

fn argon2_verify(password: &[u8], hash: &[u8], algo: Algo) -> bool {
    match CString::new(c_str(hash)) {
        Ok(encoded) => argon2::verify(&encoded, password, argon2_type(algo)),
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// builtins
// ---------------------------------------------------------------------------

fn options_arg(args: &[Value], i: usize) -> Option<PhpArray> {
    match args.get(i).map(deref) {
        Some(Value::Array(a)) => Some(a),
        _ => None,
    }
}

fn fn_password_hash(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let password = str_param(host, args, 0, "password_hash", "password")?;
    let Some(algo) = algo_find_zval(host, "password_hash", args.get(1))? else {
        return Err(value_error(
            "password_hash(): Argument #2 ($algo) must be a valid password hashing algorithm",
        ));
    };
    let options = options_arg(args, 2);
    let digest = match algo {
        Algo::Bcrypt => bcrypt_hash(host, &password, options.as_ref())?,
        Algo::Argon2i | Algo::Argon2id => argon2_hash(host, &password, options.as_ref(), algo)?,
    };
    digest
        .map(Value::str)
        .ok_or_else(|| Thrown::error("Error", "Password hashing failed for unknown reason"))
}

fn fn_password_verify(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let password = str_param(host, args, 0, "password_verify", "password")?;
    let hash = str_param(host, args, 1, "password_verify", "hash")?;
    let ok = match identify_ex(&hash, Some(Algo::Bcrypt)) {
        Some(Algo::Bcrypt) | None => bcrypt_verify(&password, &hash),
        Some(algo) => argon2_verify(&password, &hash, algo),
    };
    Ok(Value::Bool(ok))
}

fn fn_password_needs_rehash(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let hash = str_param(host, args, 0, "password_needs_rehash", "hash")?;
    let Some(new_algo) = algo_find_zval(host, "password_needs_rehash", args.get(1))? else {
        // Unknown new algorithm: never prompt to rehash.
        return Ok(Value::Bool(false));
    };
    if identify_ex(&hash, None) != Some(new_algo) {
        return Ok(Value::Bool(true));
    }
    let options = options_arg(args, 2);
    let options = options.as_ref();
    let needs = match new_algo {
        Algo::Bcrypt => bcrypt_cost(&hash) != option_long(options, b"cost").unwrap_or(BCRYPT_COST),
        Algo::Argon2i | Algo::Argon2id => {
            let want = (
                option_long(options, b"memory_cost").unwrap_or(ARGON2_MEMORY_COST),
                option_long(options, b"time_cost").unwrap_or(ARGON2_TIME_COST),
                option_long(options, b"threads").unwrap_or(ARGON2_THREADS),
            );
            argon2_params(&hash, (0, 0, 0)) != want
        }
    };
    Ok(Value::Bool(needs))
}

fn fn_password_get_info(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let hash = str_param(host, args, 0, "password_get_info", "hash")?;
    let mut out = PhpArray::new();
    let mut options = PhpArray::new();
    let ident = extract_ident(&hash);
    let algo = ident.and_then(algo_find).filter(|a| a.valid(&hash));
    let (Some(ident), Some(algo)) = (ident, algo) else {
        out.insert(ArrayKey::from_bytes(b"algo"), Value::Null);
        out.insert(ArrayKey::from_bytes(b"algoName"), Value::str("unknown"));
        out.insert(ArrayKey::from_bytes(b"options"), Value::Array(options));
        return Ok(Value::Array(out));
    };
    out.insert(ArrayKey::from_bytes(b"algo"), Value::str(ident.to_vec()));
    out.insert(ArrayKey::from_bytes(b"algoName"), Value::str(algo.name()));
    match algo {
        Algo::Bcrypt => {
            options.insert(
                ArrayKey::from_bytes(b"cost"),
                Value::Int(bcrypt_cost(&hash)),
            );
        }
        Algo::Argon2i | Algo::Argon2id => {
            let (m, t, p) = argon2_params(
                &hash,
                (ARGON2_MEMORY_COST, ARGON2_TIME_COST, ARGON2_THREADS),
            );
            options.insert(ArrayKey::from_bytes(b"memory_cost"), Value::Int(m));
            options.insert(ArrayKey::from_bytes(b"time_cost"), Value::Int(t));
            options.insert(ArrayKey::from_bytes(b"threads"), Value::Int(p));
        }
    }
    out.insert(ArrayKey::from_bytes(b"options"), Value::Array(options));
    Ok(Value::Array(out))
}

fn fn_password_algos(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut arr = PhpArray::new();
    for (ident, _) in ALGOS {
        arr.push(Value::str(ident.to_vec()));
    }
    Ok(Value::Array(arr))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("password_hash", fn_password_hash),
            ("password_verify", fn_password_verify),
            ("password_needs_rehash", fn_password_needs_rehash),
            ("password_get_info", fn_password_get_info),
            ("password_algos", fn_password_algos),
        ],
    );
}

/// `PASSWORD_*` (password.stub.php).
pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("PASSWORD_DEFAULT", Value::str("2y")),
        ("PASSWORD_BCRYPT", Value::str("2y")),
        ("PASSWORD_ARGON2I", Value::str("argon2i")),
        ("PASSWORD_ARGON2ID", Value::str("argon2id")),
        ("PASSWORD_BCRYPT_DEFAULT_COST", Value::Int(BCRYPT_COST)),
        ("PASSWORD_ARGON2_PROVIDER", Value::str("standard")),
        (
            "PASSWORD_ARGON2_DEFAULT_MEMORY_COST",
            Value::Int(ARGON2_MEMORY_COST),
        ),
        (
            "PASSWORD_ARGON2_DEFAULT_TIME_COST",
            Value::Int(ARGON2_TIME_COST),
        ),
        (
            "PASSWORD_ARGON2_DEFAULT_THREADS",
            Value::Int(ARGON2_THREADS),
        ),
    ]
}

#[cfg(test)]
mod tests;
