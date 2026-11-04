//! The `hash` extension: the generic `hash()` / `hash_hmac()` / `hash_pbkdf2()`
//! surface over the full PHP 8.5 algorithm set.
//!
//! This is rhp's first *extension reimplemented in Rust* under the
//! differential-test framework (`crates/rhp-difftest`): every algorithm here
//! is proven byte-identical to the C `ext/hash` by sweeping it against the real
//! `php` binary over a broad generated corpus.
//!
//! Where a faithful pure-Rust crate exists we use it (RustCrypto's `md2`, `md4`,
//! `md-5`, `sha1`, `sha2`, `sha3`, `ripemd`, `whirlpool`, `tiger`, `gost94`;
//! `xxhash-rust`); crc32b and adler32 go through the linked zlib. The oddballs
//! with no 1:1 implementation are hand-rolled: the crc32/crc32c, FNV, joaat and
//! murmur variants below, and the table-heavy `snefru`, `haval` and 4-pass
//! `tiger` in their own files.

use std::collections::HashMap;

use digest::Digest;
use rhp_values::{conv, Value};

mod haval;
mod primitives;
mod snefru;
mod tiger;

use crate::ext::zlib::zraw as zlib;
use crate::{Builtin, Host, Thrown};
use primitives::to_hex;
pub(crate) use primitives::{base64_decode, base64_encode};

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

fn as_bool(v: Option<&Value>) -> bool {
    v.map(|v| conv::to_bool(&deref(v))).unwrap_or(false)
}

fn as_int(v: &Value) -> i64 {
    conv::to_int(&deref(v))
}

/// Run a RustCrypto `Digest` over `data`.
fn dg<D: Digest>(data: &[u8]) -> Vec<u8> {
    D::digest(data).to_vec()
}

// ---------------------------------------------------------------------------
// the algorithm dispatch — raw (binary) digest for a lowercased algo name
// ---------------------------------------------------------------------------

/// Compute the raw binary digest of `data` under `algo` (already lowercased).
/// Returns `None` for an unknown algorithm name.
pub(crate) fn digest(algo: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    Some(match algo {
        b"md2" => dg::<md2::Md2>(data),
        b"md4" => dg::<md4::Md4>(data),
        b"md5" => dg::<::md5::Md5>(data),
        b"sha1" => dg::<::sha1::Sha1>(data),
        b"sha224" => dg::<sha2::Sha224>(data),
        b"sha256" => dg::<sha2::Sha256>(data),
        b"sha384" => dg::<sha2::Sha384>(data),
        b"sha512/224" => dg::<sha2::Sha512_224>(data),
        b"sha512/256" => dg::<sha2::Sha512_256>(data),
        b"sha512" => dg::<sha2::Sha512>(data),
        b"sha3-224" => dg::<sha3::Sha3_224>(data),
        b"sha3-256" => dg::<sha3::Sha3_256>(data),
        b"sha3-384" => dg::<sha3::Sha3_384>(data),
        b"sha3-512" => dg::<sha3::Sha3_512>(data),
        b"ripemd128" => dg::<ripemd::Ripemd128>(data),
        b"ripemd160" => dg::<ripemd::Ripemd160>(data),
        b"ripemd256" => dg::<ripemd::Ripemd256>(data),
        b"ripemd320" => dg::<ripemd::Ripemd320>(data),
        b"whirlpool" => dg::<whirlpool::Whirlpool>(data),
        b"tiger128,3" => tiger3(data, 16),
        b"tiger160,3" => tiger3(data, 20),
        b"tiger192,3" => tiger3(data, 24),
        b"tiger128,4" => tiger::tiger4(data, 16),
        b"tiger160,4" => tiger::tiger4(data, 20),
        b"tiger192,4" => tiger::tiger4(data, 24),
        b"snefru" | b"snefru256" => snefru::snefru256(data).to_vec(),
        b"gost" => dg::<gost94::Gost94Test>(data),
        b"gost-crypto" => dg::<gost94::Gost94CryptoPro>(data),
        b"adler32" => zlib::adler32(data).to_be_bytes().to_vec(),
        // PHP emits the non-reflected "crc32" little-endian (unlike crc32b/crc32c)
        b"crc32" => crc32_msb(data).to_le_bytes().to_vec(),
        b"crc32b" => zlib::crc32(data).to_be_bytes().to_vec(),
        b"crc32c" => crc32c(data).to_be_bytes().to_vec(),
        b"fnv132" => fnv1_32(data).to_be_bytes().to_vec(),
        b"fnv1a32" => fnv1a_32(data).to_be_bytes().to_vec(),
        b"fnv164" => fnv1_64(data).to_be_bytes().to_vec(),
        b"fnv1a64" => fnv1a_64(data).to_be_bytes().to_vec(),
        b"joaat" => joaat(data).to_be_bytes().to_vec(),
        b"murmur3a" | b"murmur3c" | b"murmur3f" | b"xxh32" | b"xxh64" | b"xxh3" | b"xxh128" => {
            return seeded(algo, data, &Init::Seed(0))
        }
        _ => {
            if let Some((bits, passes)) = parse_haval(algo) {
                haval::haval(data, bits, passes)
            } else {
                return None;
            }
        }
    })
}

/// How a seeded algorithm (murmur3*, xxh*) starts: what its `hash_init`
/// takes from `$options`.
#[derive(Clone)]
enum Init {
    Seed(i64),
    Secret(Vec<u8>),
}

/// The digest of a seeded algorithm.
fn seeded(algo: &[u8], data: &[u8], init: &Init) -> Option<Vec<u8>> {
    use xxhash_rust::{xxh3, xxh32, xxh64};
    let seed = match init {
        Init::Seed(n) => *n,
        Init::Secret(_) => 0,
    };
    Some(match (algo, init) {
        (b"xxh3", Init::Secret(k)) => xxh3::xxh3_64_with_secret(data, k).to_be_bytes().to_vec(),
        (b"xxh128", Init::Secret(k)) => xxh3::xxh3_128_with_secret(data, k).to_be_bytes().to_vec(),
        (b"murmur3a", _) => murmur3a(data, seed as u32).to_be_bytes().to_vec(),
        (b"murmur3c", _) => murmur3c(data, seed as u32),
        (b"murmur3f", _) => murmur3f(data, seed as u64),
        (b"xxh32", _) => xxh32::xxh32(data, seed as u32).to_be_bytes().to_vec(),
        (b"xxh64", _) => xxh64::xxh64(data, seed as u64).to_be_bytes().to_vec(),
        (b"xxh3", _) => xxh3::xxh3_64_with_seed(data, seed as u64)
            .to_be_bytes()
            .to_vec(),
        (b"xxh128", _) => xxh3::xxh3_128_with_seed(data, seed as u64)
            .to_be_bytes()
            .to_vec(),
        _ => return None,
    })
}

/// The `$options` php's hash_init reads (PHP_MURMUR3AInit, PHP_XXH32Init,
/// _PHP_XXH3_Init, …), with their diagnostics from `fname`. `None`: the
/// default start. Other algorithms ignore the options.
fn hash_options(
    host: &mut dyn Host,
    fname: &str,
    algo: &[u8],
    opts: Option<&Value>,
) -> Result<Option<Init>, Thrown> {
    let Some(Value::Array(o)) = opts.map(crate::deref) else {
        return Ok(None);
    };
    let get = |k: &[u8]| {
        o.get(&rhp_values::ArrayKey::from_bytes(k))
            .map(crate::deref)
    };
    let seed = get(b"seed");
    const SEED_ZERO: &str =
        "Passing a seed of a type other than int is deprecated because it is the same as setting the seed to 0";
    Ok(match algo {
        b"murmur3a" | b"murmur3c" | b"murmur3f" | b"xxh32" => match seed {
            Some(Value::Int(n)) => Some(Init::Seed(n)),
            Some(_) => {
                crate::deprecated(host, fname, SEED_ZERO)?;
                None
            }
            None => None,
        },
        // php's xxh64 also warns when the options have no seed at all.
        b"xxh64" => match seed {
            Some(Value::Int(n)) => Some(Init::Seed(n)),
            _ => {
                crate::deprecated(host, fname, SEED_ZERO)?;
                None
            }
        },
        b"xxh3" | b"xxh128" => {
            let name = String::from_utf8_lossy(algo);
            let secret = get(b"secret");
            if seed.is_some() && secret.is_some() {
                return Err(Thrown::error(
                    "Error",
                    format!(
                        "{name}: Only one of seed or secret is to be passed for initialization"
                    ),
                ));
            }
            match (seed, secret) {
                (Some(Value::Int(n)), _) => Some(Init::Seed(n)),
                (Some(_), _) => {
                    crate::deprecated(
                        host,
                        fname,
                        "Passing a seed of a type other than int is deprecated because it is ignored",
                    )?;
                    None
                }
                (None, Some(k)) => {
                    if !matches!(k, Value::Str(_)) {
                        crate::deprecated(
                            host,
                            fname,
                            "Passing a secret of a type other than string is deprecated because it implicitly converts to a string, potentially hiding bugs",
                        )?;
                    }
                    let mut k = conv::to_string_bytes(&k);
                    // XXH3_SECRET_SIZE_MIN; php keeps a 256-byte buffer.
                    if k.len() < 136 {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "{name}: Secret length must be >= 136 bytes, {} bytes passed",
                                k.len()
                            ),
                        ));
                    }
                    if k.len() > 256 {
                        k.truncate(256);
                        crate::warn(
                            host,
                            fname,
                            &format!("{name}: Secret content exceeding 256 bytes discarded"),
                        )?;
                    }
                    Some(Init::Secret(k))
                }
                (None, None) => None,
            }
        }
        _ => None,
    })
}

/// [`digest`] started from `init`.
fn digest_init(algo: &[u8], data: &[u8], init: Option<&Init>) -> Option<Vec<u8>> {
    match init {
        Some(i) => seeded(algo, data, i),
        None => digest(algo, data),
    }
}

/// The internal block size (bytes) used by `hash_hmac` for a crypto algorithm,
/// or `None` for algorithms PHP does not allow in HMAC (crc/adler/fnv/…).
fn hmac_block_size(algo: &[u8]) -> Option<usize> {
    Some(match algo {
        b"md2" => 16,
        b"md4" | b"md5" => 64,
        b"sha1" | b"sha224" | b"sha256" => 64,
        b"sha384" | b"sha512" | b"sha512/224" | b"sha512/256" => 128,
        b"sha3-224" => 144,
        b"sha3-256" => 136,
        b"sha3-384" => 104,
        b"sha3-512" => 72,
        b"ripemd128" | b"ripemd160" | b"ripemd256" | b"ripemd320" => 64,
        b"whirlpool" => 64,
        b"tiger128,3" | b"tiger160,3" | b"tiger192,3" | b"tiger128,4" | b"tiger160,4"
        | b"tiger192,4" => 64,
        b"snefru" | b"snefru256" => 32,
        b"gost" | b"gost-crypto" => 32,
        _ => {
            if parse_haval(algo).is_some() {
                128
            } else {
                return None;
            }
        }
    })
}

/// Parse a `haval<bits>,<passes>` algorithm name into `(bits, passes)`.
fn parse_haval(algo: &[u8]) -> Option<(usize, usize)> {
    let rest = algo.strip_prefix(b"haval")?;
    let comma = rest.iter().position(|&b| b == b',')?;
    let bits: usize = std::str::from_utf8(&rest[..comma]).ok()?.parse().ok()?;
    let passes: usize = std::str::from_utf8(&rest[comma + 1..]).ok()?.parse().ok()?;
    if matches!(bits, 128 | 160 | 192 | 224 | 256) && matches!(passes, 3..=5) {
        Some((bits, passes))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// tiger (3-pass via the `tiger` crate, truncated to width)
// ---------------------------------------------------------------------------

fn tiger3(data: &[u8], width: usize) -> Vec<u8> {
    // `::tiger` = the external crate (the local `tiger` module shadows the name).
    let full = dg::<::tiger::Tiger>(data);
    full[..width].to_vec()
}

// ---------------------------------------------------------------------------
// CRC-32 variants zlib doesn't have (crc32b and adler32 go through zlib)
//   crc32  = MSB-first, poly 0x04C11DB7 (PHP's "crc32", non-reflected)
//   crc32c = reflected Castagnoli, poly 0x82F63B78
// ---------------------------------------------------------------------------

fn crc32_msb(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= (byte as u32) << 24;
        for _ in 0..8 {
            if crc & 0x8000_0000 != 0 {
                crc = (crc << 1) ^ 0x04C1_1DB7;
            } else {
                crc <<= 1;
            }
        }
    }
    !crc
}

fn crc32c(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0x82F6_3B78 & mask);
        }
    }
    !crc
}

// ---------------------------------------------------------------------------
// FNV-1 / FNV-1a (32- and 64-bit)
// ---------------------------------------------------------------------------

fn fnv1_32(data: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in data {
        h = h.wrapping_mul(0x0100_0193);
        h ^= b as u32;
    }
    h
}

fn fnv1a_32(data: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in data {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn fnv1_64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
        h ^= b as u64;
    }
    h
}

fn fnv1a_64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

// ---------------------------------------------------------------------------
// Jenkins one-at-a-time (joaat), seed 0
// ---------------------------------------------------------------------------

fn joaat(data: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &b in data {
        h = h.wrapping_add(b as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h = h.wrapping_add(h << 15);
    h
}

// ---------------------------------------------------------------------------
// MurmurHash3: murmur3a = x86_32, murmur3c = x86_128, murmur3f = x64_128
// PHP emits each hash word big-endian.
// ---------------------------------------------------------------------------

fn murmur3a(data: &[u8], seed: u32) -> u32 {
    const C1: u32 = 0xcc9e_2d51;
    const C2: u32 = 0x1b87_3593;
    let mut h: u32 = seed;
    let nblocks = data.len() / 4;
    for i in 0..nblocks {
        let mut k = u32::from_le_bytes([
            data[i * 4],
            data[i * 4 + 1],
            data[i * 4 + 2],
            data[i * 4 + 3],
        ]);
        k = k.wrapping_mul(C1);
        k = k.rotate_left(15);
        k = k.wrapping_mul(C2);
        h ^= k;
        h = h.rotate_left(13);
        h = h.wrapping_mul(5).wrapping_add(0xe654_6b64);
    }
    let tail = &data[nblocks * 4..];
    let mut k: u32 = 0;
    for (i, &b) in tail.iter().enumerate() {
        k ^= (b as u32) << (8 * i);
    }
    if !tail.is_empty() {
        k = k.wrapping_mul(C1);
        k = k.rotate_left(15);
        k = k.wrapping_mul(C2);
        h ^= k;
    }
    h ^= data.len() as u32;
    fmix32(h)
}

fn fmix32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h
}

fn fmix64(mut k: u64) -> u64 {
    k ^= k >> 33;
    k = k.wrapping_mul(0xff51_afd7_ed55_8ccd);
    k ^= k >> 33;
    k = k.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    k ^= k >> 33;
    k
}

fn murmur3c(data: &[u8], seed: u32) -> Vec<u8> {
    const C1: u32 = 0x239b_961b;
    const C2: u32 = 0xab0e_9789;
    const C3: u32 = 0x38b3_4ae5;
    const C4: u32 = 0xa1e3_8b93;
    let (mut h1, mut h2, mut h3, mut h4) = (seed, seed, seed, seed);
    let nblocks = data.len() / 16;
    for i in 0..nblocks {
        let base = i * 16;
        let rd = |o: usize| {
            u32::from_le_bytes([
                data[base + o],
                data[base + o + 1],
                data[base + o + 2],
                data[base + o + 3],
            ])
        };
        let (mut k1, mut k2, mut k3, mut k4) = (rd(0), rd(4), rd(8), rd(12));
        k1 = k1.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
        h1 ^= k1;
        h1 = h1
            .rotate_left(19)
            .wrapping_add(h2)
            .wrapping_mul(5)
            .wrapping_add(0x561c_cd1b);
        k2 = k2.wrapping_mul(C2).rotate_left(16).wrapping_mul(C3);
        h2 ^= k2;
        h2 = h2
            .rotate_left(17)
            .wrapping_add(h3)
            .wrapping_mul(5)
            .wrapping_add(0x0bca_a747);
        k3 = k3.wrapping_mul(C3).rotate_left(17).wrapping_mul(C4);
        h3 ^= k3;
        h3 = h3
            .rotate_left(15)
            .wrapping_add(h4)
            .wrapping_mul(5)
            .wrapping_add(0x96cd_1c35);
        k4 = k4.wrapping_mul(C4).rotate_left(18).wrapping_mul(C1);
        h4 ^= k4;
        h4 = h4
            .rotate_left(13)
            .wrapping_add(h1)
            .wrapping_mul(5)
            .wrapping_add(0x32ac_3b17);
    }
    let tail = &data[nblocks * 16..];
    let (mut k1, mut k2, mut k3, mut k4) = (0u32, 0u32, 0u32, 0u32);
    let get = |n: usize| *tail.get(n).unwrap_or(&0) as u32;
    let l = tail.len();
    if l >= 15 {
        k4 ^= get(14) << 16;
    }
    if l >= 14 {
        k4 ^= get(13) << 8;
    }
    if l >= 13 {
        k4 ^= get(12);
        k4 = k4.wrapping_mul(C4).rotate_left(18).wrapping_mul(C1);
        h4 ^= k4;
    }
    if l >= 12 {
        k3 ^= get(11) << 24;
    }
    if l >= 11 {
        k3 ^= get(10) << 16;
    }
    if l >= 10 {
        k3 ^= get(9) << 8;
    }
    if l >= 9 {
        k3 ^= get(8);
        k3 = k3.wrapping_mul(C3).rotate_left(17).wrapping_mul(C4);
        h3 ^= k3;
    }
    if l >= 8 {
        k2 ^= get(7) << 24;
    }
    if l >= 7 {
        k2 ^= get(6) << 16;
    }
    if l >= 6 {
        k2 ^= get(5) << 8;
    }
    if l >= 5 {
        k2 ^= get(4);
        k2 = k2.wrapping_mul(C2).rotate_left(16).wrapping_mul(C3);
        h2 ^= k2;
    }
    if l >= 4 {
        k1 ^= get(3) << 24;
    }
    if l >= 3 {
        k1 ^= get(2) << 16;
    }
    if l >= 2 {
        k1 ^= get(1) << 8;
    }
    if l >= 1 {
        k1 ^= get(0);
        k1 = k1.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
        h1 ^= k1;
    }

    let len = data.len() as u32;
    h1 ^= len;
    h2 ^= len;
    h3 ^= len;
    h4 ^= len;
    h1 = h1.wrapping_add(h2).wrapping_add(h3).wrapping_add(h4);
    h2 = h2.wrapping_add(h1);
    h3 = h3.wrapping_add(h1);
    h4 = h4.wrapping_add(h1);
    h1 = fmix32(h1);
    h2 = fmix32(h2);
    h3 = fmix32(h3);
    h4 = fmix32(h4);
    h1 = h1.wrapping_add(h2).wrapping_add(h3).wrapping_add(h4);
    h2 = h2.wrapping_add(h1);
    h3 = h3.wrapping_add(h1);
    h4 = h4.wrapping_add(h1);

    let mut out = Vec::with_capacity(16);
    for h in [h1, h2, h3, h4] {
        out.extend_from_slice(&h.to_be_bytes());
    }
    out
}

fn murmur3f(data: &[u8], seed: u64) -> Vec<u8> {
    const C1: u64 = 0x87c3_7b91_1142_53d5;
    const C2: u64 = 0x4cf5_ad43_2745_937f;
    let (mut h1, mut h2) = (seed, seed);
    let nblocks = data.len() / 16;
    for i in 0..nblocks {
        let base = i * 16;
        let rd = |o: usize| {
            let mut b = [0u8; 8];
            b.copy_from_slice(&data[base + o..base + o + 8]);
            u64::from_le_bytes(b)
        };
        let (mut k1, mut k2) = (rd(0), rd(8));
        k1 = k1.wrapping_mul(C1).rotate_left(31).wrapping_mul(C2);
        h1 ^= k1;
        h1 = h1
            .rotate_left(27)
            .wrapping_add(h2)
            .wrapping_mul(5)
            .wrapping_add(0x52dc_e729);
        k2 = k2.wrapping_mul(C2).rotate_left(33).wrapping_mul(C1);
        h2 ^= k2;
        h2 = h2
            .rotate_left(31)
            .wrapping_add(h1)
            .wrapping_mul(5)
            .wrapping_add(0x3849_5ab5);
    }
    let tail = &data[nblocks * 16..];
    let (mut k1, mut k2) = (0u64, 0u64);
    let get = |n: usize| *tail.get(n).unwrap_or(&0) as u64;
    let l = tail.len();
    if l >= 15 {
        k2 ^= get(14) << 48;
    }
    if l >= 14 {
        k2 ^= get(13) << 40;
    }
    if l >= 13 {
        k2 ^= get(12) << 32;
    }
    if l >= 12 {
        k2 ^= get(11) << 24;
    }
    if l >= 11 {
        k2 ^= get(10) << 16;
    }
    if l >= 10 {
        k2 ^= get(9) << 8;
    }
    if l >= 9 {
        k2 ^= get(8);
        k2 = k2.wrapping_mul(C2).rotate_left(33).wrapping_mul(C1);
        h2 ^= k2;
    }
    if l >= 8 {
        k1 ^= get(7) << 56;
    }
    if l >= 7 {
        k1 ^= get(6) << 48;
    }
    if l >= 6 {
        k1 ^= get(5) << 40;
    }
    if l >= 5 {
        k1 ^= get(4) << 32;
    }
    if l >= 4 {
        k1 ^= get(3) << 24;
    }
    if l >= 3 {
        k1 ^= get(2) << 16;
    }
    if l >= 2 {
        k1 ^= get(1) << 8;
    }
    if l >= 1 {
        k1 ^= get(0);
        k1 = k1.wrapping_mul(C1).rotate_left(31).wrapping_mul(C2);
        h1 ^= k1;
    }

    let len = data.len() as u64;
    h1 ^= len;
    h2 ^= len;
    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);
    h1 = fmix64(h1);
    h2 = fmix64(h2);
    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);

    let mut out = Vec::with_capacity(16);
    out.extend_from_slice(&h1.to_be_bytes());
    out.extend_from_slice(&h2.to_be_bytes());
    out
}

// ---------------------------------------------------------------------------
// HMAC (generic, over any block-based crypto algorithm)
// ---------------------------------------------------------------------------

fn hmac(algo: &[u8], key: &[u8], msg: &[u8]) -> Option<Vec<u8>> {
    let block = hmac_block_size(algo)?;
    // keys longer than the block size are hashed first
    let mut k = if key.len() > block {
        digest(algo, key)?
    } else {
        key.to_vec()
    };
    k.resize(block, 0);
    let ipad: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    let mut inner = ipad;
    inner.extend_from_slice(msg);
    let inner_digest = digest(algo, &inner)?;
    let mut outer = opad;
    outer.extend_from_slice(&inner_digest);
    digest(algo, &outer)
}

// ---------------------------------------------------------------------------
// PBKDF2 (RFC 2898) over HMAC
// ---------------------------------------------------------------------------

/// PBKDF2 over a RustCrypto digest, the way php's `hash_pbkdf2` runs it: the
/// HMAC key's ipad/opad blocks are hashed once and each HMAC resumes from a
/// copy of those two states. Same bytes as the generic path below, at half the
/// compressions and without per-block allocations (a 32 MiB key took 6x php's
/// time before).
fn pbkdf2_with<D: Digest + Clone>(
    block: usize,
    pass: &[u8],
    salt: &[u8],
    iterations: i64,
    out_len: usize,
) -> Vec<u8> {
    let mut k = if pass.len() > block {
        D::digest(pass).to_vec()
    } else {
        pass.to_vec()
    };
    k.resize(block, 0);
    let mut inner = D::new();
    inner.update(k.iter().map(|b| b ^ 0x36).collect::<Vec<u8>>());
    let mut outer = D::new();
    outer.update(k.iter().map(|b| b ^ 0x5c).collect::<Vec<u8>>());
    let prf = |parts: &[&[u8]]| {
        let mut c = inner.clone();
        for p in parts {
            c.update(p);
        }
        let ih = c.finalize();
        let mut c = outer.clone();
        c.update(&ih);
        c.finalize()
    };
    let h_len = <D as Digest>::output_size();
    let blocks = out_len.div_ceil(h_len);
    let mut out = Vec::with_capacity(blocks * h_len);
    for i in 1..=blocks as u32 {
        let mut u = prf(&[salt, &i.to_be_bytes()]);
        let mut t = u.clone();
        for _ in 1..iterations {
            u = prf(&[&u]);
            for (ti, ui) in t.iter_mut().zip(u.iter()) {
                *ti ^= *ui;
            }
        }
        out.extend_from_slice(&t);
    }
    out.truncate(out_len);
    out
}

/// [`pbkdf2_with`] for the algorithms backed by a RustCrypto digest; `None`
/// sends the rest (tiger, snefru, haval) down the generic path.
fn pbkdf2_fast(
    algo: &[u8],
    pass: &[u8],
    salt: &[u8],
    iterations: i64,
    out_len: usize,
) -> Option<Vec<u8>> {
    let b = hmac_block_size(algo)?;
    let (p, s, n, l) = (pass, salt, iterations, out_len);
    Some(match algo {
        b"md2" => pbkdf2_with::<md2::Md2>(b, p, s, n, l),
        b"md4" => pbkdf2_with::<md4::Md4>(b, p, s, n, l),
        b"md5" => pbkdf2_with::<::md5::Md5>(b, p, s, n, l),
        b"sha1" => pbkdf2_with::<::sha1::Sha1>(b, p, s, n, l),
        b"sha224" => pbkdf2_with::<sha2::Sha224>(b, p, s, n, l),
        b"sha256" => pbkdf2_with::<sha2::Sha256>(b, p, s, n, l),
        b"sha384" => pbkdf2_with::<sha2::Sha384>(b, p, s, n, l),
        b"sha512/224" => pbkdf2_with::<sha2::Sha512_224>(b, p, s, n, l),
        b"sha512/256" => pbkdf2_with::<sha2::Sha512_256>(b, p, s, n, l),
        b"sha512" => pbkdf2_with::<sha2::Sha512>(b, p, s, n, l),
        b"sha3-224" => pbkdf2_with::<sha3::Sha3_224>(b, p, s, n, l),
        b"sha3-256" => pbkdf2_with::<sha3::Sha3_256>(b, p, s, n, l),
        b"sha3-384" => pbkdf2_with::<sha3::Sha3_384>(b, p, s, n, l),
        b"sha3-512" => pbkdf2_with::<sha3::Sha3_512>(b, p, s, n, l),
        b"ripemd128" => pbkdf2_with::<ripemd::Ripemd128>(b, p, s, n, l),
        b"ripemd160" => pbkdf2_with::<ripemd::Ripemd160>(b, p, s, n, l),
        b"ripemd256" => pbkdf2_with::<ripemd::Ripemd256>(b, p, s, n, l),
        b"ripemd320" => pbkdf2_with::<ripemd::Ripemd320>(b, p, s, n, l),
        b"whirlpool" => pbkdf2_with::<whirlpool::Whirlpool>(b, p, s, n, l),
        b"gost" => pbkdf2_with::<gost94::Gost94Test>(b, p, s, n, l),
        b"gost-crypto" => pbkdf2_with::<gost94::Gost94CryptoPro>(b, p, s, n, l),
        _ => return None,
    })
}

fn pbkdf2(
    algo: &[u8],
    pass: &[u8],
    salt: &[u8],
    iterations: i64,
    out_len: usize,
) -> Option<Vec<u8>> {
    if let Some(out) = pbkdf2_fast(algo, pass, salt, iterations, out_len) {
        return Some(out);
    }
    pbkdf2_generic(algo, pass, salt, iterations, out_len)
}

/// PBKDF2 for the digests without a RustCrypto implementation, the HMAC key
/// padded once: H((K ^ opad) || H((K ^ ipad) || m)).
fn pbkdf2_generic(
    algo: &[u8],
    pass: &[u8],
    salt: &[u8],
    iterations: i64,
    out_len: usize,
) -> Option<Vec<u8>> {
    let h_len = digest(algo, b"")?.len();
    let block = hmac_block_size(algo)?;
    let mut k = if pass.len() > block {
        digest(algo, pass)?
    } else {
        pass.to_vec()
    };
    k.resize(block, 0);
    let mut inner: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    let mut outer: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    let mut mac = |msg: &[u8]| -> Option<Vec<u8>> {
        inner.truncate(block);
        inner.extend_from_slice(msg);
        let d = digest(algo, &inner)?;
        outer.truncate(block);
        outer.extend_from_slice(&d);
        digest(algo, &outer)
    };
    let blocks = out_len.div_ceil(h_len);
    let mut out = Vec::with_capacity(blocks * h_len);
    let mut salt_block = salt.to_vec();
    for i in 1..=blocks as u32 {
        salt_block.truncate(salt.len());
        salt_block.extend_from_slice(&i.to_be_bytes());
        let mut u = mac(&salt_block)?;
        let mut t = u.clone();
        for _ in 1..iterations {
            u = mac(&u)?;
            for (ti, ui) in t.iter_mut().zip(u.iter()) {
                *ti ^= *ui;
            }
        }
        out.extend_from_slice(&t);
    }
    out.truncate(out_len);
    Some(out)
}

// ---------------------------------------------------------------------------
// builtin wrappers
// ---------------------------------------------------------------------------

fn invalid_algo(func: &str) -> Thrown {
    Thrown::error(
        "ValueError",
        format!("{func}(): Argument #1 ($algo) must be a valid hashing algorithm"),
    )
}

fn fn_hash(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let algo = bytes_of(&args[0]).to_ascii_lowercase();
    let data = bytes_of(&args[1]);
    let binary = as_bool(args.get(2));
    if digest(&algo, b"").is_none() {
        return Err(invalid_algo("hash"));
    }
    let init = hash_options(host, "hash", &algo, args.get(3))?;
    let raw = digest_init(&algo, &data, init.as_ref()).unwrap_or_default();
    Ok(Value::str(if binary { raw } else { to_hex(&raw) }))
}

fn fn_hash_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let algo = bytes_of(&args[0]).to_ascii_lowercase();
    let path = bytes_of(&args[1]);
    let binary = as_bool(args.get(2));
    if digest(&algo, b"").is_none() {
        return Err(invalid_algo("hash_file"));
    }
    let Some(data) = host.stream_get_contents(&path) else {
        return Ok(Value::Bool(false));
    };
    host.consume_resource_ids(1);
    let init = hash_options(host, "hash_file", &algo, args.get(3))?;
    let raw = digest_init(&algo, &data, init.as_ref()).unwrap_or_default();
    Ok(Value::str(if binary { raw } else { to_hex(&raw) }))
}

/// `__hash_init_options($algo, $options)`: hash_init's reading of
/// `$options`, as `[seed]`, `['secret' => k]` or null.
fn fn_hash_init_options(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let algo = bytes_of(&args[0]).to_ascii_lowercase();
    Ok(match hash_options(host, "hash_init", &algo, args.get(1))? {
        Some(Init::Seed(n)) => Value::Int(n),
        Some(Init::Secret(k)) => Value::str(k),
        None => Value::Null,
    })
}

/// `__hash_digest($algo, $data, $binary, $init)`: a HashContext's digest,
/// `$init` from `__hash_init_options`.
fn fn_hash_digest(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let algo = bytes_of(&args[0]).to_ascii_lowercase();
    let data = bytes_of(&args[1]);
    let binary = as_bool(args.get(2));
    let init = match args.get(3).map(crate::deref) {
        Some(Value::Int(n)) => Some(Init::Seed(n)),
        Some(Value::Str(k)) => Some(Init::Secret(k.as_bytes().to_vec())),
        _ => None,
    };
    let raw = digest_init(&algo, &data, init.as_ref()).ok_or_else(|| invalid_algo("hash_final"))?;
    Ok(Value::str(if binary { raw } else { to_hex(&raw) }))
}

fn fn_md5_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = bytes_of(&args[0]);
    let binary = as_bool(args.get(1));
    let Some(data) = host.stream_get_contents(&path) else {
        return Ok(Value::Bool(false));
    };
    host.consume_resource_ids(1);
    let raw = digest(b"md5", &data).unwrap();
    Ok(Value::str(if binary { raw } else { to_hex(&raw) }))
}

fn fn_sha1_file(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let path = bytes_of(&args[0]);
    let binary = as_bool(args.get(1));
    let Some(data) = host.stream_get_contents(&path) else {
        return Ok(Value::Bool(false));
    };
    host.consume_resource_ids(1);
    let raw = digest(b"sha1", &data).unwrap();
    Ok(Value::str(if binary { raw } else { to_hex(&raw) }))
}

fn fn_hash_hmac(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let algo = bytes_of(&args[0]).to_ascii_lowercase();
    let data = bytes_of(&args[1]);
    let key = bytes_of(&args[2]);
    let binary = as_bool(args.get(3));
    // hash_hmac rejects non-crypto algorithms (crc32/adler/fnv/…)
    match hmac(&algo, &key, &data) {
        Some(raw) => Ok(Value::str(if binary { raw } else { to_hex(&raw) })),
        None => Err(Thrown::error(
            "ValueError",
            "hash_hmac(): Argument #1 ($algo) must be a valid cryptographic hashing algorithm",
        )),
    }
}

/// `$options` (arg 6) goes to the hash's init, and only the seeded
/// non-cryptographic algorithms read it, which pbkdf2 refuses.
fn fn_hash_pbkdf2(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let algo = bytes_of(&args[0]).to_ascii_lowercase();
    let pass = bytes_of(&args[1]);
    let salt = bytes_of(&args[2]);
    let iterations = as_int(&args[3]);
    let length = args.get(4).map(as_int).unwrap_or(0);
    let binary = as_bool(args.get(5));
    if hmac_block_size(&algo).is_none() {
        return Err(Thrown::error(
            "ValueError",
            "hash_pbkdf2(): Argument #1 ($algo) must be a valid cryptographic hashing algorithm",
        ));
    }
    if iterations <= 0 {
        return Err(Thrown::error(
            "ValueError",
            "hash_pbkdf2(): Argument #4 ($iterations) must be greater than 0",
        ));
    }
    if length < 0 {
        return Err(Thrown::error(
            "ValueError",
            "hash_pbkdf2(): Argument #5 ($length) must be greater than or equal to 0",
        ));
    }
    let key_len = length as usize;
    // $length counts the output: bytes when raw, hex digits otherwise (0:
    // the digest's full length), so only ceil(length / 2) bytes are derived
    // for hex output.
    let h_len = digest(&algo, b"").map_or(0, |d| d.len());
    let raw_len = match (key_len, binary) {
        (0, _) => h_len,
        (n, true) => n,
        (n, false) => n.div_ceil(2),
    };
    // safe_emalloc(loops, digest_size, 0) for the derived blocks: past the
    // hard limit php exits; don't allocate (or abort) here either.
    if h_len > 0
        && !rhp_values::mem::alloc_check(((raw_len.max(1) - 1) / h_len + 1).saturating_mul(h_len))
    {
        return Ok(Value::Null);
    }
    match pbkdf2(&algo, &pass, &salt, iterations, raw_len) {
        Some(raw) => {
            if binary {
                Ok(Value::str(raw))
            } else {
                let mut hex = to_hex(&raw);
                if key_len != 0 {
                    hex.truncate(key_len);
                }
                Ok(Value::str(hex))
            }
        }
        None => Err(invalid_algo("hash_pbkdf2")),
    }
}

fn fn_hash_equals(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let known = bytes_of(&args[0]);
    let user = bytes_of(&args[1]);
    // constant-time compare (length first, then byte diff accumulation)
    let eq = known.len() == user.len()
        && known
            .iter()
            .zip(user.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0;
    Ok(Value::Bool(eq))
}

fn fn_hash_algos(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(string_list(ALGOS))
}

fn fn_hash_hmac_algos(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(string_list(HMAC_ALGOS))
}

fn string_list(names: &[&str]) -> Value {
    let mut arr = rhp_values::PhpArray::new();
    for (i, name) in names.iter().enumerate() {
        arr.insert(rhp_values::ArrayKey::Int(i as i64), Value::str(*name));
    }
    Value::Array(arr)
}

/// Every algorithm rhp's `hash()` accepts, in PHP's `hash_algos()` order.
const ALGOS: &[&str] = &[
    "md2",
    "md4",
    "md5",
    "sha1",
    "sha224",
    "sha256",
    "sha384",
    "sha512/224",
    "sha512/256",
    "sha512",
    "sha3-224",
    "sha3-256",
    "sha3-384",
    "sha3-512",
    "ripemd128",
    "ripemd160",
    "ripemd256",
    "ripemd320",
    "whirlpool",
    "tiger128,3",
    "tiger160,3",
    "tiger192,3",
    "tiger128,4",
    "tiger160,4",
    "tiger192,4",
    "snefru",
    "snefru256",
    "gost",
    "gost-crypto",
    "adler32",
    "crc32",
    "crc32b",
    "crc32c",
    "fnv132",
    "fnv1a32",
    "fnv164",
    "fnv1a64",
    "joaat",
    "murmur3a",
    "murmur3c",
    "murmur3f",
    "xxh32",
    "xxh64",
    "xxh3",
    "xxh128",
    "haval128,3",
    "haval160,3",
    "haval192,3",
    "haval224,3",
    "haval256,3",
    "haval128,4",
    "haval160,4",
    "haval192,4",
    "haval224,4",
    "haval256,4",
    "haval128,5",
    "haval160,5",
    "haval192,5",
    "haval224,5",
    "haval256,5",
];

/// The subset valid for `hash_hmac()` / `hash_pbkdf2()` — crypto algorithms only.
const HMAC_ALGOS: &[&str] = &[
    "md2",
    "md4",
    "md5",
    "sha1",
    "sha224",
    "sha256",
    "sha384",
    "sha512/224",
    "sha512/256",
    "sha512",
    "sha3-224",
    "sha3-256",
    "sha3-384",
    "sha3-512",
    "ripemd128",
    "ripemd160",
    "ripemd256",
    "ripemd320",
    "whirlpool",
    "tiger128,3",
    "tiger160,3",
    "tiger192,3",
    "tiger128,4",
    "tiger160,4",
    "tiger192,4",
    "snefru",
    "snefru256",
    "gost",
    "gost-crypto",
    "haval128,3",
    "haval160,3",
    "haval192,3",
    "haval224,3",
    "haval256,3",
    "haval128,4",
    "haval160,4",
    "haval192,4",
    "haval224,4",
    "haval256,4",
    "haval128,5",
    "haval160,5",
    "haval192,5",
    "haval224,5",
    "haval256,5",
];

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    // md5/sha1/crc32/base64/bin2hex/hex2bin — the hand-rolled core primitives.
    primitives::register(m);
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
    put(m, "hash", fn_hash, 2, Some(4));
    put(m, "hash_file", fn_hash_file, 2, Some(4));
    put(m, "md5_file", fn_md5_file, 1, Some(2));
    put(m, "sha1_file", fn_sha1_file, 1, Some(2));
    put(m, "hash_hmac", fn_hash_hmac, 3, Some(4));
    put(m, "hash_pbkdf2", fn_hash_pbkdf2, 4, Some(7));
    put(m, "__hash_init_options", fn_hash_init_options, 2, Some(2));
    put(m, "__hash_digest", fn_hash_digest, 4, Some(4));
    put(m, "hash_equals", fn_hash_equals, 2, Some(2));
    put(m, "hash_algos", fn_hash_algos, 0, Some(0));
    put(m, "hash_hmac_algos", fn_hash_hmac_algos, 0, Some(0));
}

/// `HASH_HMAC`.
pub fn constants() -> Vec<(&'static str, Value)> {
    vec![("HASH_HMAC", Value::Int(1))]
}

/// The incremental-hashing API. A `HashContext` buffers the data fed to it
/// and digests it on `hash_final` with the one-shot primitives (the result
/// is identical to php's streaming state machines).
pub const PRELUDE_PHP: &str = r#"
final class HashContext {
    private $__algo = ''; private $__d = ''; private $__k = null; private $__done = false; private $__init = null;
    private function __construct() {}
    public function __debugInfo(): array { return ['algo' => $this->__algo]; }
    public static function __make(string $algo, $key, $init = null): HashContext {
        $c = new HashContext(); $c->__algo = $algo; $c->__k = $key; $c->__init = $init; return $c;
    }
    public static function __feed(HashContext $c, string $fn, string $data): void {
        if ($c->__done) { throw new TypeError("$fn(): Argument #1 (\$context) must be a valid, non-finalized HashContext"); }
        $c->__d .= $data;
    }
    public static function __final(HashContext $c, bool $binary): string {
        if ($c->__done) { throw new TypeError("hash_final(): Argument #1 (\$context) must be a valid, non-finalized HashContext"); }
        $c->__done = true;
        $r = $c->__k === null ? __hash_digest($c->__algo, $c->__d, $binary, $c->__init) : hash_hmac($c->__algo, $c->__d, $c->__k, $binary);
        $c->__d = '';
        return $r;
    }
    public static function __copy(HashContext $c): HashContext {
        if ($c->__done) { throw new TypeError("hash_copy(): Argument #1 (\$context) must be a valid, non-finalized HashContext"); }
        $n = HashContext::__make($c->__algo, $c->__k, $c->__init); $n->__d = $c->__d; return $n;
    }
}
function hash_init(string $algo, int $flags = 0, string $key = "", array $options = []): HashContext {
    $algo = strtolower($algo);
    if (!in_array($algo, hash_algos(), true)) {
        throw new ValueError('hash_init(): Argument #1 ($algo) must be a valid hashing algorithm');
    }
    if ($flags & HASH_HMAC) {
        if (!in_array($algo, hash_hmac_algos(), true)) {
            throw new ValueError('hash_init(): Argument #1 ($algo) must be a cryptographic hashing algorithm if HMAC is requested');
        }
        if ($key === '') {
            throw new ValueError('hash_init(): Argument #3 ($key) must not be empty when HMAC is requested');
        }
        return HashContext::__make($algo, $key);
    }
    return HashContext::__make($algo, null, func_num_args() > 3 ? __hash_init_options($algo, $options) : null);
}
function hash_update(HashContext $context, string $data): true {
    HashContext::__feed($context, 'hash_update', $data);
    return true;
}
function hash_update_stream(HashContext $context, $stream, int $length = -1): int {
    $data = $length < 0 ? stream_get_contents($stream) : fread($stream, $length);
    if ($data === false) { $data = ''; }
    HashContext::__feed($context, 'hash_update_stream', $data);
    return strlen($data);
}
function hash_update_file(HashContext $context, string $filename, $stream_context = null): bool {
    $data = @file_get_contents($filename);
    if ($data === false) {
        $e = error_get_last();
        if ($e !== null && str_starts_with($e['message'], 'file_get_contents(')) {
            __stdext_warning('hash_update_file(' . substr($e['message'], strlen('file_get_contents(')));
        }
        return false;
    }
    HashContext::__feed($context, 'hash_update_file', $data);
    return true;
}
function hash_final(HashContext $context, bool $binary = false): string {
    return HashContext::__final($context, $binary);
}
function hash_copy(HashContext $context): HashContext {
    return HashContext::__copy($context);
}
"#;
