//! ext/random: the Mt19937 engine behind `mt_rand`/`rand` (with its
//! seeding), `random_int`, `random_bytes` and `lcg_value`.

use std::cell::RefCell;

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::zend::zpp::{int_or_null_param, int_param};
use crate::{Host, Thrown};

// --- Mt19937 (ext/random/engine_mt19937.c) ----------------------------------

const MT_N: usize = 624;
const MT_M: usize = 397;
pub(crate) const MT_RAND_MT19937: i64 = 0;
pub(crate) const MT_RAND_PHP: i64 = 1;

struct Mt {
    state: [u32; MT_N],
    count: usize,
    mode: i64,
}

#[inline]
fn twist(m: u32, u: u32, v: u32, php: bool) -> u32 {
    let mix = (u & 0x8000_0000) | (v & 0x7fff_ffff);
    let lo = if php { u & 1 } else { v & 1 };
    m ^ (mix >> 1) ^ ((lo as i32).wrapping_neg() as u32 & 0x9908_b0df)
}

impl Mt {
    fn seeded(seed: u32, mode: i64) -> Mt {
        let mut mt = Mt {
            state: [0; MT_N],
            count: 0,
            mode,
        };
        mt.seed(seed);
        mt
    }

    fn seed(&mut self, seed: u32) {
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

    fn next32(&mut self) -> u32 {
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

    /// `php_random_range32`.
    fn range32(&mut self, umax: u32) -> u32 {
        let mut result = self.next32();
        if umax == u32::MAX {
            return result;
        }
        let umax = umax + 1;
        if umax & (umax - 1) == 0 {
            return result & (umax - 1);
        }
        let limit = u32::MAX - (u32::MAX % umax) - 1;
        while result > limit {
            result = self.next32();
        }
        result % umax
    }

    /// `php_random_range64` (two 32-bit draws per 64-bit result).
    fn range64(&mut self, umax: u64) -> u64 {
        let draw = |mt: &mut Mt| (mt.next32() as u64) | ((mt.next32() as u64) << 32);
        let mut result = draw(self);
        if umax == u64::MAX {
            return result;
        }
        let umax = umax + 1;
        if umax & (umax - 1) == 0 {
            return result & (umax - 1);
        }
        let limit = u64::MAX - (u64::MAX % umax) - 1;
        while result > limit {
            result = draw(self);
        }
        result % umax
    }

    /// `php_random_range`: uniform in `[min, max]`.
    fn range(&mut self, min: i64, max: i64) -> i64 {
        let umax = (max as u64).wrapping_sub(min as u64);
        if umax > u32::MAX as u64 {
            return (self.range64(umax).wrapping_add(min as u64)) as i64;
        }
        (self.range32(umax as u32) as u64).wrapping_add(min as u64) as i64
    }
}

thread_local! {
    static MT: RefCell<Option<Mt>> = const { RefCell::new(None) };
}

pub(crate) fn reset_state() {
    MT.with(|m| *m.borrow_mut() = None);
}

fn random_seed() -> u32 {
    let mut b = [0u8; 4];
    let _ = getrandom::getrandom(&mut b);
    u32::from_le_bytes(b)
}

/// Run `f` on the default (lazily, randomly seeded) Mt19937 state.
fn with_mt<R>(f: impl FnOnce(&mut Mt) -> R) -> R {
    MT.with(|m| {
        let mut m = m.borrow_mut();
        let mt = m.get_or_insert_with(|| Mt::seeded(random_seed(), MT_RAND_MT19937));
        f(mt)
    })
}

/// `php_mt_rand_range`: the engine behind shuffle/array_rand/str_shuffle.
pub(crate) fn mt_range(min: i64, max: i64) -> i64 {
    with_mt(|mt| mt.range(min, max))
}

/// `php_mt_rand_common`: rand()/mt_rand() with a range (honours MT_RAND_PHP).
fn mt_rand_common(min: i64, max: i64) -> i64 {
    with_mt(|mt| {
        if mt.mode == MT_RAND_MT19937 {
            return mt.range(min, max);
        }
        let r = (mt.next32() >> 1) as f64;
        let offset = ((max as f64 - min as f64 + 1.0) * (r / (2147483647.0 + 1.0))) as u64;
        offset.wrapping_add(min as u64) as i64
    })
}

fn fn_mt_srand(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let seed = int_or_null_param(host, args, 0, "mt_srand", "seed")?;
    let mode = int_param(host, args, 1, "mt_srand", "mode", MT_RAND_MT19937)?;
    let mode = if mode == MT_RAND_PHP {
        host.emit_diagnostic(
            crate::E_DEPRECATED,
            "The MT_RAND_PHP variant of Mt19937 is deprecated",
        )?;
        MT_RAND_PHP
    } else {
        MT_RAND_MT19937
    };
    let seed = seed.map(|s| s as u32).unwrap_or_else(random_seed);
    MT.with(|m| *m.borrow_mut() = Some(Mt::seeded(seed, mode)));
    Ok(Value::Null)
}

fn fn_mt_rand(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if args.is_empty() {
        return Ok(Value::Int((with_mt(|mt| mt.next32()) >> 1) as i64));
    }
    if args.len() == 1 {
        return Err(Thrown::error(
            "ArgumentCountError",
            "mt_rand() expects exactly 2 arguments, 1 given",
        ));
    }
    let min = int_param(host, args, 0, "mt_rand", "min", 0)?;
    let max = int_param(host, args, 1, "mt_rand", "max", 0)?;
    if max < min {
        return Err(Thrown::error(
            "ValueError",
            "mt_rand(): Argument #2 ($max) must be greater than or equal to argument #1 ($min)",
        ));
    }
    Ok(Value::Int(mt_rand_common(min, max)))
}

fn fn_rand(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    if args.is_empty() {
        return Ok(Value::Int((with_mt(|mt| mt.next32()) >> 1) as i64));
    }
    if args.len() == 1 {
        return Err(Thrown::error(
            "ArgumentCountError",
            "rand() expects exactly 2 arguments, 1 given",
        ));
    }
    let min = int_param(host, args, 0, "rand", "min", 0)?;
    let max = int_param(host, args, 1, "rand", "max", 0)?;
    Ok(Value::Int(if max < min {
        mt_rand_common(max, min)
    } else {
        mt_rand_common(min, max)
    }))
}

// --- randomness --------------------------------------------------------------

fn rand_u64() -> u64 {
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    u64::from_le_bytes(b)
}

/// A uniform integer in `[min, max]` (inclusive) via rejection sampling.
pub(crate) fn rand_range(min: i64, max: i64) -> i64 {
    if min >= max {
        return min;
    }
    let span = (max as i128 - min as i128 + 1) as u128;
    // Rejection-sample to avoid modulo bias.
    let limit = u128::MAX - (u128::MAX % span);
    loop {
        let r = ((rand_u64() as u128) << 64) | rand_u64() as u128;
        if r < limit {
            return (min as i128 + (r % span) as i128) as i64;
        }
    }
}

fn fn_random_bytes(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let len = conv::to_int(&deref(&args[0]));
    if len <= 0 {
        return Err(Thrown::error(
            "ValueError",
            "random_bytes(): Argument #1 ($length) must be greater than 0",
        ));
    }
    let mut b = vec![0u8; len as usize];
    if getrandom::getrandom(&mut b).is_err() {
        return Err(Thrown::error(
            "Random\\RandomException",
            "Could not gather sufficient random data",
        ));
    }
    Ok(Value::str(b))
}

fn fn_random_int(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let min = conv::to_int(&deref(&args[0]));
    let max = conv::to_int(&deref(&args[1]));
    if min > max {
        return Err(Thrown::error(
            "ValueError",
            "random_int(): Argument #1 ($min) must be less than or equal to argument #2 ($max)",
        ));
    }
    Ok(Value::Int(rand_range(min, max)))
}

fn fn_getrandmax(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(2147483647))
}

fn fn_lcg_value(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    crate::deprecated_fn(
        host,
        "lcg_value",
        "8.4",
        "use \\Random\\Randomizer::getFloat() instead",
    )?;
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    // 53 random bits → [0, 1).
    let n = u64::from_le_bytes(b) >> 11;
    Ok(Value::Float(n as f64 / (1u64 << 53) as f64))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("mt_srand", fn_mt_srand),
            ("srand", fn_mt_srand),
            ("mt_rand", fn_mt_rand),
            ("rand", fn_rand),
            ("random_bytes", fn_random_bytes),
            ("random_int", fn_random_int),
            ("mt_getrandmax", fn_getrandmax),
            ("getrandmax", fn_getrandmax),
            ("lcg_value", fn_lcg_value),
        ],
    );
}

#[cfg(test)]
mod tests;
