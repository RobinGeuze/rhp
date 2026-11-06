//! The engines (ext/random/engine_*.c) and the range functions every
//! engine shares (random.c's php_random_range32/64).

use rhp_values::{ArrayKey, PhpArray, PhpStr, Value};

use crate::{Host, Thrown};

// --- Mt19937 (ext/random/engine_mt19937.c) ----------------------------------
// Based on MTRand by Richard J. Wagner and on code by Makoto Matsumoto and
// Takuji Nishimura (BSD-3-Clause); see THIRD-PARTY-NOTICES, section 24.

pub(crate) const MT_N: usize = 624;
const MT_M: usize = 397;
pub(crate) const MT_RAND_MT19937: i64 = 0;
pub(crate) const MT_RAND_PHP: i64 = 1;

#[derive(Clone)]
pub(crate) struct Mt {
    pub(crate) state: [u32; MT_N],
    pub(crate) count: usize,
    pub(crate) mode: i64,
}

#[inline]
fn twist(m: u32, u: u32, v: u32, php: bool) -> u32 {
    let mix = (u & 0x8000_0000) | (v & 0x7fff_ffff);
    let lo = if php { u & 1 } else { v & 1 };
    m ^ (mix >> 1) ^ ((lo as i32).wrapping_neg() as u32 & 0x9908_b0df)
}

impl Mt {
    /// The zeroed state of an engine whose constructor never ran.
    pub(crate) fn zeroed() -> Mt {
        Mt {
            state: [0; MT_N],
            count: 0,
            mode: MT_RAND_MT19937,
        }
    }

    pub(crate) fn seeded(seed: u32, mode: i64) -> Mt {
        let mut mt = Mt {
            mode,
            ..Mt::zeroed()
        };
        mt.seed(seed);
        mt
    }

    /// `php_random_mt19937_seed32`.
    pub(crate) fn seed(&mut self, seed: u32) {
        self.state[0] = seed;
        for i in 1..MT_N {
            let prev = self.state[i - 1];
            self.state[i] = 1_812_433_253u32
                .wrapping_mul(prev ^ (prev >> 30))
                .wrapping_add(i as u32);
        }
        self.reload();
    }

    fn reload(&mut self) {
        let php = self.mode == MT_RAND_PHP;
        let s = &mut self.state;
        for i in 0..MT_N - MT_M {
            s[i] = twist(s[i + MT_M], s[i], s[i + 1], php);
        }
        for i in MT_N - MT_M..MT_N - 1 {
            s[i] = twist(s[i + MT_M - MT_N], s[i], s[i + 1], php);
        }
        s[MT_N - 1] = twist(s[MT_M - 1], s[MT_N - 1], s[0], php);
        self.count = 0;
    }

    pub(crate) fn next32(&mut self) -> u32 {
        if self.count >= MT_N {
            self.reload();
        }
        let mut s1 = self.state[self.count];
        self.count += 1;
        s1 ^= s1 >> 11;
        s1 ^= (s1 << 7) & 0x9d2c_5680;
        s1 ^= (s1 << 15) & 0xefc6_0000;
        s1 ^ (s1 >> 18)
    }

    /// `php_random_range` over this engine (never fails).
    pub(crate) fn range(&mut self, min: i64, max: i64) -> i64 {
        let mut g = |_: &mut ()| -> Result<Gen, Thrown> { Ok(Gen::of32(self.next32())) };
        range(&mut (), &mut g, min, max).unwrap_or(min)
    }
}

// --- PcgOneseq128XslRr64 (engine_pcgoneseq128xslrr64.c) ----------------------

const PCG_MULT: u128 = (2549297995355413924u128 << 64) | 4865540595714422341;
const PCG_PLUS: u128 = (6364136223846793005u128 << 64) | 1442695040888963407;

fn pcg_step(s: &mut u128) {
    *s = s.wrapping_mul(PCG_MULT).wrapping_add(PCG_PLUS);
}

/// `php_random_pcgoneseq128xslrr64_seed128`.
pub(crate) fn pcg_seed(seed: u128) -> u128 {
    let mut s = 0u128;
    pcg_step(&mut s);
    s = s.wrapping_add(seed);
    pcg_step(&mut s);
    s
}

fn pcg_generate(s: &mut u128) -> u64 {
    pcg_step(s);
    let hi = (*s >> 64) as u64;
    let v = hi ^ (*s as u64);
    v.rotate_right((hi >> 58) as u32)
}

/// `php_random_pcgoneseq128xslrr64_advance`.
pub(crate) fn pcg_advance(s: &mut u128, mut advance: u64) {
    let (mut cur_mult, mut cur_plus) = (PCG_MULT, PCG_PLUS);
    let (mut acc_mult, mut acc_plus) = (1u128, 0u128);
    while advance > 0 {
        if advance & 1 != 0 {
            acc_mult = acc_mult.wrapping_mul(cur_mult);
            acc_plus = acc_plus.wrapping_mul(cur_mult).wrapping_add(cur_plus);
        }
        cur_plus = cur_mult.wrapping_add(1).wrapping_mul(cur_plus);
        cur_mult = cur_mult.wrapping_mul(cur_mult);
        advance /= 2;
    }
    *s = acc_mult.wrapping_mul(*s).wrapping_add(acc_plus);
}

// --- Xoshiro256StarStar (engine_xoshiro256starstar.c) ------------------------

fn splitmix64(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut r = *seed;
    r = (r ^ (r >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    r = (r ^ (r >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    r ^ (r >> 31)
}

/// `php_random_xoshiro256starstar_seed64`.
pub(crate) fn xoshiro_seed64(mut seed: u64) -> [u64; 4] {
    [
        splitmix64(&mut seed),
        splitmix64(&mut seed),
        splitmix64(&mut seed),
        splitmix64(&mut seed),
    ]
}

fn xoshiro_generate(s: &mut [u64; 4]) -> u64 {
    let r = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
    let t = s[1] << 17;
    s[2] ^= s[0];
    s[3] ^= s[1];
    s[1] ^= s[2];
    s[0] ^= s[3];
    s[2] ^= t;
    s[3] = s[3].rotate_left(45);
    r
}

fn xoshiro_jump_by(s: &mut [u64; 4], jmp: &[u64; 4]) {
    let mut acc = [0u64; 4];
    for word in jmp {
        for j in 0..64 {
            if word & (1u64 << j) != 0 {
                for (a, x) in acc.iter_mut().zip(s.iter()) {
                    *a ^= x;
                }
            }
            xoshiro_generate(s);
        }
    }
    *s = acc;
}

pub(crate) fn xoshiro_jump(s: &mut [u64; 4]) {
    xoshiro_jump_by(
        s,
        &[
            0x180e_c6d3_3cfd_0aba,
            0xd5a6_1266_f0c9_392c,
            0xa958_2618_e03f_c9aa,
            0x39ab_dc45_29b1_661c,
        ],
    );
}

pub(crate) fn xoshiro_jump_long(s: &mut [u64; 4]) {
    xoshiro_jump_by(
        s,
        &[
            0x76e1_5d3e_fefd_cbbf,
            0xc500_4e44_1c52_2fb3,
            0x7771_0069_854e_e241,
            0x3910_9bb0_2acb_e635,
        ],
    );
}

// --- the engines as one type ---------------------------------------------------

/// php_random_result: `size` bytes of `result` (little end first).
#[derive(Clone, Copy)]
pub(crate) struct Gen {
    pub(crate) size: usize,
    pub(crate) result: u64,
}

impl Gen {
    fn of32(r: u32) -> Gen {
        Gen {
            size: 4,
            result: r as u64,
        }
    }
    fn of64(r: u64) -> Gen {
        Gen { size: 8, result: r }
    }
    /// The bytes `generate()` returns.
    pub(crate) fn bytes(self) -> Vec<u8> {
        (0..self.size)
            .map(|i| (self.result >> (i * 8)) as u8)
            .collect()
    }
}

/// A native engine's state.
#[derive(Clone)]
pub(crate) enum Algo {
    Mt(Box<Mt>),
    Pcg(u128),
    Xoshiro([u64; 4]),
    Secure,
}

impl Algo {
    pub(crate) fn generate(&mut self) -> Result<Gen, Thrown> {
        Ok(match self {
            Algo::Mt(mt) => Gen::of32(mt.next32()),
            Algo::Pcg(s) => Gen::of64(pcg_generate(s)),
            Algo::Xoshiro(s) => Gen::of64(xoshiro_generate(s)),
            Algo::Secure => {
                let mut b = [0u8; 8];
                csprng(&mut b)?;
                Gen::of64(u64::from_le_bytes(b))
            }
        })
    }

    /// The algorithm's `serialize`: its state as little-endian hex strings.
    pub(crate) fn state(&self) -> Option<PhpArray> {
        let mut a = PhpArray::new();
        let mut hex = |b: &[u8]| a.push(Value::Str(PhpStr::from_bytes(bin2hex(b))));
        match self {
            Algo::Mt(mt) => {
                for w in mt.state.iter() {
                    hex(&w.to_le_bytes());
                }
                a.push(Value::Int(mt.count as i64));
                a.push(Value::Int(mt.mode));
            }
            Algo::Pcg(s) => {
                hex(&((*s >> 64) as u64).to_le_bytes());
                hex(&(*s as u64).to_le_bytes());
            }
            Algo::Xoshiro(s) => {
                for w in s {
                    hex(&w.to_le_bytes());
                }
            }
            Algo::Secure => return None,
        }
        Some(a)
    }

    /// The algorithm's `unserialize`; false on bad data.
    pub(crate) fn load_state(&mut self, data: &PhpArray) -> bool {
        fn word<const N: usize>(data: &PhpArray, i: i64) -> Option<[u8; N]> {
            match data.get(&ArrayKey::Int(i))? {
                Value::Str(s) if s.len() == 2 * N => hex2bin(s.as_bytes()),
                _ => None,
            }
        }
        match self {
            Algo::Mt(mt) => {
                if data.len() != MT_N + 2 {
                    return false;
                }
                let mut state = [0u32; MT_N];
                for (i, w) in state.iter_mut().enumerate() {
                    match word::<4>(data, i as i64) {
                        Some(b) => *w = u32::from_le_bytes(b),
                        None => return false,
                    }
                }
                let Some(Value::Int(count)) = data.get(&ArrayKey::Int(MT_N as i64)) else {
                    return false;
                };
                // php stores it in a uint32_t first
                let count = *count as u32 as usize;
                if count > MT_N {
                    return false;
                }
                let Some(Value::Int(mode)) = data.get(&ArrayKey::Int(MT_N as i64 + 1)) else {
                    return false;
                };
                if *mode != MT_RAND_MT19937 && *mode != MT_RAND_PHP {
                    return false;
                }
                mt.state = state;
                mt.count = count;
                mt.mode = *mode;
                true
            }
            Algo::Pcg(s) => {
                if data.len() != 2 {
                    return false;
                }
                let (Some(hi), Some(lo)) = (word::<8>(data, 0), word::<8>(data, 1)) else {
                    return false;
                };
                *s = ((u64::from_le_bytes(hi) as u128) << 64) | u64::from_le_bytes(lo) as u128;
                true
            }
            Algo::Xoshiro(s) => {
                if data.len() != 4 {
                    return false;
                }
                let mut t = [0u64; 4];
                for (i, w) in t.iter_mut().enumerate() {
                    match word::<8>(data, i as i64) {
                        Some(b) => *w = u64::from_le_bytes(b),
                        None => return false,
                    }
                }
                if t == [0; 4] {
                    return false;
                }
                *s = t;
                true
            }
            Algo::Secure => false,
        }
    }
}

/// `php_random_bytes_throw`.
pub(crate) fn csprng(buf: &mut [u8]) -> Result<(), Thrown> {
    getrandom::getrandom(buf).map_err(|_| {
        Thrown::error(
            "Random\\RandomException",
            "Failed to retrieve randomness from the operating system (Error)",
        )
    })
}

/// `php_random_bin2hex_le`.
fn bin2hex(b: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = Vec::with_capacity(b.len() * 2);
    for &c in b {
        out.push(HEX[(c >> 4) as usize]);
        out.push(HEX[(c & 15) as usize]);
    }
    out
}

/// `php_random_hex2bin_le`.
fn hex2bin<const N: usize>(s: &[u8]) -> Option<[u8; N]> {
    let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = (digit(s[2 * i])? << 4) | digit(s[2 * i + 1])?;
    }
    Some(out)
}

pub(crate) const RANGE_ATTEMPTS: u32 = 50;

pub(crate) fn broken_engine() -> Thrown {
    Thrown::error(
        "Random\\BrokenRandomEngineError",
        format!("Failed to generate an acceptable random number in {RANGE_ATTEMPTS} attempts"),
    )
}

/// Draws until at least `bytes` bytes are collected.
fn collect<C>(
    cx: &mut C,
    generate: &mut impl FnMut(&mut C) -> Result<Gen, Thrown>,
    bytes: usize,
) -> Result<u64, Thrown> {
    let (mut result, mut total) = (0u64, 0usize);
    loop {
        let r = generate(cx)?;
        let part = if bytes == 4 {
            (r.result as u32 as u64) << (total * 8)
        } else {
            r.result << (total * 8)
        };
        result |= part;
        total += r.size;
        if total >= bytes {
            return Ok(if bytes == 4 {
                result as u32 as u64
            } else {
                result
            });
        }
    }
}

/// `php_random_range32`.
pub(crate) fn range32<C>(
    cx: &mut C,
    generate: &mut impl FnMut(&mut C) -> Result<Gen, Thrown>,
    umax: u32,
) -> Result<u32, Thrown> {
    let mut result = collect(cx, generate, 4)? as u32;
    if umax == u32::MAX {
        return Ok(result);
    }
    let umax = umax + 1;
    if umax & (umax - 1) == 0 {
        return Ok(result & (umax - 1));
    }
    let limit = u32::MAX - (u32::MAX % umax) - 1;
    let mut count = 0;
    while result > limit {
        count += 1;
        if count > RANGE_ATTEMPTS {
            return Err(broken_engine());
        }
        result = collect(cx, generate, 4)? as u32;
    }
    Ok(result % umax)
}

/// `php_random_range64`.
pub(crate) fn range64<C>(
    cx: &mut C,
    generate: &mut impl FnMut(&mut C) -> Result<Gen, Thrown>,
    umax: u64,
) -> Result<u64, Thrown> {
    let mut result = collect(cx, generate, 8)?;
    if umax == u64::MAX {
        return Ok(result);
    }
    let umax = umax + 1;
    if umax & (umax - 1) == 0 {
        return Ok(result & (umax - 1));
    }
    let limit = u64::MAX - (u64::MAX % umax) - 1;
    let mut count = 0;
    while result > limit {
        count += 1;
        if count > RANGE_ATTEMPTS {
            return Err(broken_engine());
        }
        result = collect(cx, generate, 8)?;
    }
    Ok(result % umax)
}

/// `php_random_range`: uniform in `[min, max]`.
pub(crate) fn range<C>(
    cx: &mut C,
    generate: &mut impl FnMut(&mut C) -> Result<Gen, Thrown>,
    min: i64,
    max: i64,
) -> Result<i64, Thrown> {
    let umax = (max as u64).wrapping_sub(min as u64);
    if umax > u32::MAX as u64 {
        return Ok(range64(cx, generate, umax)?.wrapping_add(min as u64) as i64);
    }
    Ok((range32(cx, generate, umax as u32)? as u64).wrapping_add(min as u64) as i64)
}

/// `php_random_int` (the Secure engine's range): rejection sampling over
/// CSPRNG bytes.
pub(crate) fn secure_range(min: i64, max: i64) -> Result<i64, Thrown> {
    let umax = (max as u64).wrapping_sub(min as u64);
    if umax == 0 {
        return Ok(min);
    }
    let draw = || -> Result<u64, Thrown> {
        let mut b = [0u8; 8];
        csprng(&mut b)?;
        Ok(u64::from_le_bytes(b))
    };
    let mut result = draw()?;
    if umax == u64::MAX {
        return Ok(result as i64);
    }
    let umax = umax + 1;
    if umax & (umax - 1) != 0 {
        let limit = u64::MAX - (u64::MAX % umax) - 1;
        while result > limit {
            result = draw()?;
        }
    }
    Ok((result % umax).wrapping_add(min as u64) as i64)
}

/// Calls the user engine's `generate()` (engine_user.c).
pub(crate) fn user_generate(host: &mut dyn Host, engine: &Value) -> Result<Gen, Thrown> {
    let mut cb = PhpArray::new();
    cb.push(engine.clone());
    cb.push(Value::str("generate"));
    let r = host.call_callable(&Value::Array(cb), &[])?;
    let s = match rhp_values::ops::deref(&r) {
        Value::Str(s) => s,
        other => PhpStr::from_bytes(rhp_values::conv::to_string_bytes(&other)),
    };
    let b = s.as_bytes();
    if b.is_empty() {
        return Err(Thrown::error(
            "Random\\BrokenRandomEngineError",
            "A random engine must return a non-empty string",
        ));
    }
    let size = b.len().min(8);
    let mut result = 0u64;
    for (i, &c) in b[..size].iter().enumerate() {
        result += (c as u64) << (8 * i);
    }
    Ok(Gen { size, result })
}
