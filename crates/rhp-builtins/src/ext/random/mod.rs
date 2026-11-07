//! ext/random: `mt_rand`/`rand` (with their seeding), `random_int`,
//! `random_bytes`, `lcg_value`, and the Random\Randomizer and engine
//! classes (randomizer.rs, engine.rs).

use std::cell::RefCell;

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::zend::zpp::{int_or_null_param, int_param};
use crate::{Host, Thrown};

pub(crate) mod engine;
mod randomizer;

use engine::Mt;
pub(crate) use engine::{MT_RAND_MT19937, MT_RAND_PHP};
pub(crate) use randomizer::CLASSES;

/// The Random\* classes.
pub const PRELUDE_PHP: &str = include_str!("prelude.php");

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
            b"The MT_RAND_PHP variant of Mt19937 is deprecated",
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

fn fn_lcg_value(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    // 53 random bits → [0, 1).
    let n = u64::from_le_bytes(b) >> 11;
    Ok(Value::Float(n as f64 / (1u64 << 53) as f64))
}

/// The `MT_RAND_*` constants.
pub fn mt_rand_constants() -> Vec<(&'static str, Value)> {
    vec![
        ("MT_RAND_MT19937", Value::Int(MT_RAND_MT19937)),
        ("MT_RAND_PHP", Value::Int(MT_RAND_PHP)),
    ]
}

/// php_array_data_shuffle / php_binary_string_shuffle: Fisher-Yates with
/// `range(0, left)` draws from the end.
pub(crate) fn shuffle<T>(
    v: &mut [T],
    range: &mut dyn FnMut(i64, i64) -> Result<i64, Thrown>,
) -> Result<(), Thrown> {
    let mut left = v.len().saturating_sub(1);
    while left > 0 {
        let j = range(0, left as i64)? as usize;
        v.swap(left, j);
        left -= 1;
    }
    Ok(())
}

/// What [`pick_keys`] rejects.
pub(crate) enum PickError {
    Empty,
    Num,
}

/// php_array_pick_keys: one key, or an array of `num` keys in order.
pub(crate) fn pick_keys(
    arr: &rhp_values::PhpArray,
    num: i64,
    range: &mut dyn FnMut(i64, i64) -> Result<i64, Thrown>,
) -> Result<Result<Value, PickError>, Thrown> {
    let avail = arr.len() as i64;
    if avail == 0 {
        return Ok(Err(PickError::Empty));
    }
    if num == 1 {
        let i = range(0, avail - 1)? as usize;
        return Ok(Ok(arr.get_index(i).map(|(k, _)| k.to_value()).unwrap()));
    }
    if num <= 0 || num > avail {
        return Ok(Err(PickError::Num));
    }
    let (mut want, negative) = if num > (avail >> 1) {
        (avail - num, true)
    } else {
        (num, false)
    };
    let mut bits = vec![false; avail as usize];
    let mut failures = 0;
    while want > 0 {
        let r = range(0, avail - 1)? as usize;
        if bits[r] {
            failures += 1;
            if failures > engine::RANGE_ATTEMPTS {
                return Err(engine::broken_engine());
            }
            continue;
        }
        bits[r] = true;
        want -= 1;
        failures = 0;
    }
    let mut out = rhp_values::PhpArray::with_capacity(num as usize);
    for (i, (k, _)) in arr.iter().enumerate() {
        if bits[i] ^ negative {
            out.push(k.to_value());
        }
    }
    Ok(Ok(Value::Array(out)))
}

pub fn register(m: &mut crate::Registry) {
    randomizer::register(m);
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
