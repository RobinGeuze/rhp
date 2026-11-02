//! Standard-library builtins that tooling (phpunit, psalm, php-cs-fixer,
//! rector) needs: the seeded Mt19937 engine (`mt_srand`/`mt_rand`/`rand`,
//! `shuffle`, `array_rand`, `str_shuffle`), `array_change_key_case`,
//! `array_multisort`, `settype`, `fnmatch`, process introspection
//! (`getrusage`, `gc_status`, `sys_getloadavg`, `memory_reset_peak_usage`,
//! `gc_mem_caches`), opcache's disabled-state API, `getopt`, `readline` and
//! the `stream_set_read_buffer` / `stream_set_chunk_size` knobs.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;

use rhp_semantics::deref;
use rhp_values::{conv, ArrayKey, PhpArray, PhpStr, Value};

use crate::argparse::{int_or_null_param, int_param, str_param, value_name};
use crate::{Builtin, Host, Thrown};

// --- Mt19937 (ext/random/engine_mt19937.c) ----------------------------------

const MT_N: usize = 624;
const MT_M: usize = 397;
const MT_RAND_MT19937: i64 = 0;
const MT_RAND_PHP: i64 = 1;

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
    static CHUNK_SIZES: RefCell<HashMap<u32, i64>> = RefCell::new(HashMap::new());
}

pub(crate) fn reset_state() {
    MT.with(|m| *m.borrow_mut() = None);
    CHUNK_SIZES.with(|c| c.borrow_mut().clear());
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

/// `shuffle(array &$array)`: Fisher-Yates over the values, keys dropped.
fn fn_shuffle(_host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let Value::Ref(r) = &args[0] else {
        return Ok(Value::Bool(true));
    };
    let arr = match &*r.borrow() {
        Value::Array(a) => a.clone(),
        _ => return Ok(Value::Bool(true)),
    };
    let mut vals: Vec<Value> = arr.iter().map(|(_, v)| v.clone()).collect();
    let n = vals.len();
    if n > 1 {
        let mut left = n - 1;
        while left > 0 {
            let j = mt_range(0, left as i64) as usize;
            vals.swap(left, j);
            left -= 1;
        }
    }
    let mut out = PhpArray::new();
    for v in vals {
        out.push(v);
    }
    *r.borrow_mut() = Value::Array(out);
    Ok(Value::Bool(true))
}

fn key_value(k: &ArrayKey) -> Value {
    match k {
        ArrayKey::Int(n) => Value::Int(*n),
        ArrayKey::Str(s) => Value::Str(s.clone()),
    }
}

/// `array_rand(array $array, int $num = 1)` (php_array_pick_keys).
fn fn_array_rand(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let arr = match deref(&args[0]) {
        Value::Array(a) => a,
        other => {
            return Err(Thrown::type_error(format!(
                "array_rand(): Argument #1 ($array) must be of type array, {} given",
                value_name(host, &other)
            )))
        }
    };
    let num = int_param(host, args, 1, "array_rand", "num", 1)?;
    let avail = arr.len() as i64;
    if avail == 0 {
        return Err(Thrown::error(
            "ValueError",
            "array_rand(): Argument #1 ($array) must not be empty",
        ));
    }
    if num == 1 {
        let i = mt_range(0, avail - 1) as usize;
        return Ok(arr.get_index(i).map(|(k, _)| key_value(k)).unwrap());
    }
    if num <= 0 || num > avail {
        return Err(Thrown::error(
            "ValueError",
            "array_rand(): Argument #2 ($num) must be between 1 and the number of elements in argument #1 ($array)",
        ));
    }
    let (mut want, negative) = if num > (avail >> 1) {
        (avail - num, true)
    } else {
        (num, false)
    };
    let mut bits = vec![false; avail as usize];
    while want > 0 {
        let r = mt_range(0, avail - 1) as usize;
        if !bits[r] {
            bits[r] = true;
            want -= 1;
        }
    }
    let mut out = PhpArray::new();
    for (i, (k, _)) in arr.iter().enumerate() {
        if bits[i] ^ negative {
            out.push(key_value(k));
        }
    }
    Ok(Value::Array(out))
}

fn fn_str_shuffle(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut s = str_param(host, args, 0, "str_shuffle", "string")?;
    let n = s.len();
    if n > 1 {
        let mut left = n - 1;
        while left > 0 {
            let j = mt_range(0, left as i64) as usize;
            s.swap(left, j);
            left -= 1;
        }
    }
    Ok(Value::str(s))
}

// --- arrays ------------------------------------------------------------------

fn fn_array_change_key_case(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let arr = match deref(&args[0]) {
        Value::Array(a) => a,
        other => {
            return Err(Thrown::type_error(format!(
                "array_change_key_case(): Argument #1 ($array) must be of type array, {} given",
                value_name(host, &other)
            )))
        }
    };
    let upper = int_param(host, args, 1, "array_change_key_case", "case", 0)? != 0;
    let mut out = PhpArray::new();
    for (k, v) in arr.iter() {
        let nk = match k {
            ArrayKey::Int(n) => ArrayKey::Int(*n),
            ArrayKey::Str(s) => {
                let b = if upper {
                    s.as_bytes().to_ascii_uppercase()
                } else {
                    s.as_bytes().to_ascii_lowercase()
                };
                ArrayKey::Str(PhpStr::from_bytes(b))
            }
        };
        out.insert(nk, v.clone());
    }
    Ok(Value::Array(out))
}

const SORT_ASC: i64 = 4;
const SORT_DESC: i64 = 3;
const SORT_FLAG_CASE: i64 = 8;

/// php names only the declared parameter (`$array`); variadic `$rest`
/// arguments are reported by position alone.
fn multisort_arg(i: usize) -> String {
    if i == 0 {
        "1 ($array)".to_string()
    } else {
        (i + 1).to_string()
    }
}

/// `array_multisort(array &$array, mixed &...$rest)`.
fn fn_array_multisort(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // (array slot, sort type, descending)
    let mut arrays: Vec<(usize, i64, bool)> = Vec::new();
    let (mut order_ok, mut type_ok) = (false, false);
    for (i, a) in args.iter().enumerate() {
        match deref(a) {
            Value::Array(_) => {
                arrays.push((i, 0, false));
                order_ok = true;
                type_ok = true;
            }
            Value::Int(f) => match f & !SORT_FLAG_CASE {
                SORT_ASC | SORT_DESC => {
                    if !order_ok {
                        return Err(Thrown::type_error(format!(
                            "array_multisort(): Argument #{} must be an array or a sort flag that has not already been specified",
                            multisort_arg(i)
                        )));
                    }
                    arrays.last_mut().unwrap().2 = f == SORT_DESC;
                    order_ok = false;
                }
                0 | 1 | 2 | 5 | 6 => {
                    if !type_ok {
                        return Err(Thrown::type_error(format!(
                            "array_multisort(): Argument #{} must be an array or a sort flag that has not already been specified",
                            multisort_arg(i)
                        )));
                    }
                    arrays.last_mut().unwrap().1 = f;
                    type_ok = false;
                }
                _ => {
                    return Err(Thrown::error(
                        "ValueError",
                        format!(
                            "array_multisort(): Argument #{} must be a valid sort flag",
                            multisort_arg(i)
                        ),
                    ))
                }
            },
            _ => {
                return Err(Thrown::type_error(format!(
                    "array_multisort(): Argument #{} must be an array or a sort flag",
                    multisort_arg(i)
                )))
            }
        }
    }
    let cols: Vec<Vec<(ArrayKey, Value)>> = arrays
        .iter()
        .map(|&(slot, _, _)| match deref(&args[slot]) {
            Value::Array(a) => a.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => Vec::new(),
        })
        .collect();
    let size = cols[0].len();
    if cols.iter().any(|c| c.len() != size) {
        return Err(Thrown::error("ValueError", "Array sizes are inconsistent"));
    }
    if size == 0 {
        return Ok(Value::Bool(true));
    }
    let mut order: Vec<usize> = (0..size).collect();
    let mut err = None;
    order.sort_by(|&x, &y| {
        for (c, &(_, ty, desc)) in arrays.iter().enumerate() {
            let (a, b) = (&cols[c][x].1, &cols[c][y].1);
            let o = if desc {
                crate::sort_fns::cmp_flagged_cx(host, b, a, ty, &mut err)
            } else {
                crate::sort_fns::cmp_flagged_cx(host, a, b, ty, &mut err)
            };
            if o != Ordering::Equal {
                return o;
            }
        }
        x.cmp(&y)
    });
    if let Some(t) = err.or_else(|| host.take_pending_throw()) {
        return Err(t);
    }
    for (c, &(slot, _, _)) in arrays.iter().enumerate() {
        let mut out = PhpArray::new();
        for &i in &order {
            let (k, v) = &cols[c][i];
            match k {
                ArrayKey::Int(_) => out.push(v.clone()),
                ArrayKey::Str(_) => out.insert(k.clone(), v.clone()),
            }
        }
        if let Value::Ref(r) = &args[slot] {
            *r.borrow_mut() = Value::Array(out);
        }
    }
    Ok(Value::Bool(true))
}

/// `settype(mixed &$var, string $type)`.
fn fn_settype(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ty = str_param(host, args, 1, "settype", "type")?.to_ascii_lowercase();
    let cur = deref(&args[0]);
    // convert_to_long / convert_to_double: an object warns and becomes 1.
    let object_warn = |host: &mut dyn Host, to: &str| -> Result<(), Thrown> {
        if matches!(cur, Value::Object(_) | Value::Closure(_)) {
            host.emit_diagnostic(
                crate::E_WARNING,
                &format!(
                    "Object of class {} could not be converted to {to}",
                    value_name(host, &cur)
                ),
            )?;
        }
        Ok(())
    };
    let new = match ty.as_slice() {
        b"integer" | b"int" => {
            object_warn(host, "int")?;
            Value::Int(conv::to_int(&cur))
        }
        b"float" | b"double" => {
            object_warn(host, "float")?;
            Value::Float(conv::to_float(&cur))
        }
        b"string" => match &cur {
            Value::Array(_) => {
                host.emit_diagnostic(crate::E_WARNING, "Array to string conversion")?;
                Value::str("Array")
            }
            // convert_to_string: only __toString converts an object; a
            // failure is an Error, not a parameter TypeError.
            Value::Object(_) | Value::Closure(_) => {
                let mut cb = PhpArray::new();
                cb.push(cur.clone());
                cb.push(Value::str("__toString"));
                let cb = Value::Array(cb);
                if matches!(cur, Value::Object(_)) && host.is_callable_value(&cb) {
                    Value::str(conv::to_string_bytes(&deref(
                        &host.call_callable(&cb, &[])?,
                    )))
                } else {
                    return Err(Thrown::error(
                        "Error",
                        format!(
                            "Object of class {} could not be converted to string",
                            value_name(host, &cur)
                        ),
                    ));
                }
            }
            _ => Value::str(conv::to_string_bytes(&cur)),
        },
        b"array" => match &cur {
            Value::Array(_) => cur.clone(),
            Value::Null => Value::Array(PhpArray::new()),
            Value::Object(h) => Value::Array(host.object_props_mangled(h)),
            other => {
                let mut a = PhpArray::new();
                a.push(other.clone());
                Value::Array(a)
            }
        },
        b"object" => match &cur {
            Value::Object(_) | Value::Closure(_) => cur.clone(),
            Value::Array(a) => {
                let props = a
                    .iter()
                    .map(|(k, v)| {
                        let name = match k {
                            ArrayKey::Int(n) => n.to_string().into_bytes(),
                            ArrayKey::Str(s) => s.as_bytes().to_vec(),
                        };
                        (name, v.clone())
                    })
                    .collect();
                host.make_object(props)
            }
            Value::Null => host.make_object(Vec::new()),
            other => host.make_object(vec![(b"scalar".to_vec(), other.clone())]),
        },
        b"bool" | b"boolean" => Value::Bool(conv::to_bool(&cur)),
        b"null" => Value::Null,
        b"resource" => {
            return Err(Thrown::error(
                "ValueError",
                "Cannot convert to resource type",
            ));
        }
        _ => {
            return Err(Thrown::error(
                "ValueError",
                "settype(): Argument #2 ($type) must be a valid type",
            ));
        }
    };
    if let Value::Ref(r) = &args[0] {
        *r.borrow_mut() = new;
    }
    Ok(Value::Bool(true))
}

// --- filesystem --------------------------------------------------------------

const MAXPATHLEN: usize = libc::PATH_MAX as usize;

fn fn_fnmatch(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let pattern = str_param(host, args, 0, "fnmatch", "pattern")?;
    let filename = str_param(host, args, 1, "fnmatch", "filename")?;
    let flags = int_param(host, args, 2, "fnmatch", "flags", 0)?;
    if pattern.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            "fnmatch(): Argument #1 ($pattern) must not contain any null bytes",
        ));
    }
    if filename.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            "fnmatch(): Argument #2 ($filename) must not contain any null bytes",
        ));
    }
    if filename.len() >= MAXPATHLEN {
        crate::warn(
            host,
            "fnmatch",
            &format!("Filename exceeds the maximum allowed length of {MAXPATHLEN} characters"),
        )?;
        return Ok(Value::Bool(false));
    }
    if pattern.len() >= MAXPATHLEN {
        crate::warn(
            host,
            "fnmatch",
            &format!("Pattern exceeds the maximum allowed length of {MAXPATHLEN} characters"),
        )?;
        return Ok(Value::Bool(false));
    }
    let p = std::ffi::CString::new(pattern).unwrap();
    let f = std::ffi::CString::new(filename).unwrap();
    let rc = unsafe { libc::fnmatch(p.as_ptr(), f.as_ptr(), flags as libc::c_int) };
    Ok(Value::Bool(rc == 0))
}

// --- process introspection ---------------------------------------------------

fn fn_getrusage(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mode = int_param(host, args, 0, "getrusage", "mode", 0)?;
    let who = if mode == 1 {
        libc::RUSAGE_CHILDREN
    } else {
        libc::RUSAGE_SELF
    };
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(who, &mut u) } == -1 {
        return Ok(Value::Bool(false));
    }
    let mut a = PhpArray::new();
    let mut put = |k: &str, v: i64| a.insert(ArrayKey::from_bytes(k.as_bytes()), Value::Int(v));
    put("ru_oublock", u.ru_oublock as i64);
    put("ru_inblock", u.ru_inblock as i64);
    put("ru_msgsnd", u.ru_msgsnd as i64);
    put("ru_msgrcv", u.ru_msgrcv as i64);
    put("ru_maxrss", u.ru_maxrss as i64);
    put("ru_ixrss", u.ru_ixrss as i64);
    put("ru_idrss", u.ru_idrss as i64);
    put("ru_minflt", u.ru_minflt as i64);
    put("ru_majflt", u.ru_majflt as i64);
    put("ru_nsignals", u.ru_nsignals as i64);
    put("ru_nvcsw", u.ru_nvcsw as i64);
    put("ru_nivcsw", u.ru_nivcsw as i64);
    put("ru_nswap", u.ru_nswap as i64);
    put("ru_utime.tv_usec", u.ru_utime.tv_usec as i64);
    put("ru_utime.tv_sec", u.ru_utime.tv_sec as i64);
    put("ru_stime.tv_usec", u.ru_stime.tv_usec as i64);
    put("ru_stime.tv_sec", u.ru_stime.tv_sec as i64);
    Ok(Value::Array(a))
}

fn request_start() -> std::time::Instant {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *START.get_or_init(std::time::Instant::now)
}

/// `gc_status()`: rhp's values are reference counted without a cycle
/// collector, so the collector never runs; the shape matches php's.
fn fn_gc_status(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    let mut put = |k: &str, v: Value| a.insert(ArrayKey::from_bytes(k.as_bytes()), v);
    put("running", Value::Bool(false));
    put("protected", Value::Bool(false));
    put("full", Value::Bool(false));
    put("runs", Value::Int(0));
    put("collected", Value::Int(0));
    put("threshold", Value::Int(10001));
    put("buffer_size", Value::Int(16384));
    put("roots", Value::Int(0));
    put(
        "application_time",
        Value::Float(request_start().elapsed().as_secs_f64()),
    );
    put("collector_time", Value::Float(0.0));
    put("destructor_time", Value::Float(0.0));
    put("free_time", Value::Float(0.0));
    Ok(Value::Array(a))
}

fn fn_memory_reset_peak_usage(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Null)
}

fn fn_gc_mem_caches(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(0))
}

fn fn_sys_getloadavg(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut l = [0f64; 3];
    if unsafe { libc::getloadavg(l.as_mut_ptr(), 3) } == -1 {
        return Ok(Value::Bool(false));
    }
    let mut a = PhpArray::new();
    for v in l {
        a.push(Value::Float(v));
    }
    Ok(Value::Array(a))
}

/// opcache is not enabled for the CLI (`opcache.enable_cli=0`): the status
/// / invalidate / reset family all report false.
fn fn_false(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(false))
}

// --- getopt (main/getopt.c + ext/standard/basic_functions.c) -----------------

struct OptSpec {
    ch: u8,
    name: Option<Vec<u8>>,
    need_param: u8,
}

/// `parse_opts`: the leading run of alphanumeric option letters, each
/// optionally followed by `:` (required value) or `::` (optional value).
fn parse_short_opts(s: &[u8]) -> Vec<OptSpec> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() && s[i].is_ascii_alphanumeric() {
        let ch = s[i];
        i += 1;
        let mut need = 0;
        if s.get(i) == Some(&b':') {
            need = 1;
            i += 1;
            if s.get(i) == Some(&b':') {
                need = 2;
                i += 1;
            }
        }
        out.push(OptSpec {
            ch,
            name: None,
            need_param: need,
        });
    }
    out
}

/// One argv string as C sees it (cut at the first NUL), indexable past the end.
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

enum Got {
    Eof,
    Invalid,
    Opt(usize, Option<Vec<u8>>),
}

struct GetoptState {
    optchr: usize,
    dash: bool,
}

/// `php_getopt` for one option (show_err = 0, arg_start = 1).
fn php_getopt(argv: &[Vec<u8>], opts: &[OptSpec], optind: &mut usize, st: &mut GetoptState) -> Got {
    let argc = argv.len();
    if *optind >= argc {
        return Got::Eof;
    }
    let cur = |optind: usize| argv[optind].as_slice();
    if !st.dash {
        if at(cur(*optind), 0) != b'-' {
            return Got::Eof;
        }
        if at(cur(*optind), 1) == 0 {
            return Got::Eof;
        }
    }
    let mut idx: Option<usize> = None;
    let mut arg_start: usize;
    let a = cur(*optind);
    if at(a, 0) == b'-' && at(a, 1) == b'-' {
        if at(a, 2) == 0 {
            *optind += 1;
            return Got::Eof;
        }
        arg_start = 2;
        let mut arg_end = a.len() - 1;
        let search_len = arg_end - arg_start;
        if let Some(p) = a[arg_start..arg_start + search_len]
            .iter()
            .position(|&b| b == b'=')
        {
            arg_end = p;
            arg_start += 1;
        } else {
            arg_end -= 1;
        }
        let rest = &a[2..];
        // strncmp(argv + 2, name, arg_end) == 0 && arg_end == strlen(name)
        let found = opts.iter().position(|o| {
            o.name
                .as_deref()
                .is_some_and(|n| n.len() == arg_end && rest.starts_with(n))
        });
        match found {
            Some(i) => idx = Some(i),
            None => {
                *optind += 1;
                return Got::Invalid;
            }
        }
        st.optchr = 0;
        st.dash = false;
        arg_start += opts[idx.unwrap()].name.as_ref().unwrap().len();
    } else {
        if !st.dash {
            st.dash = true;
            st.optchr = 1;
        }
        if at(a, st.optchr) == b':' {
            st.dash = false;
            *optind += 1;
            return Got::Invalid;
        }
        arg_start = 1 + st.optchr;
    }
    let idx = match idx {
        Some(i) => i,
        None => {
            let c = at(a, st.optchr);
            match opts.iter().position(|o| o.name.is_none() && o.ch == c) {
                Some(i) => i,
                None => {
                    if at(a, st.optchr + 1) == 0 {
                        st.dash = false;
                        *optind += 1;
                    } else {
                        st.optchr += 1;
                    }
                    return Got::Invalid;
                }
            }
        }
    };
    let o = &opts[idx];
    if o.need_param > 0 {
        st.dash = false;
        let mut optarg = None;
        if at(a, arg_start) == 0 {
            *optind += 1;
            if *optind == argc {
                if o.need_param == 1 {
                    return Got::Invalid;
                }
            } else if o.need_param == 1 {
                optarg = Some(argv[*optind].clone());
                *optind += 1;
            }
        } else if at(a, arg_start) == b'=' {
            optarg = Some(a[arg_start + 1..].to_vec());
            *optind += 1;
        } else {
            optarg = Some(a[arg_start..].to_vec());
            *optind += 1;
        }
        Got::Opt(idx, optarg)
    } else {
        if arg_start >= 2 && !(at(a, 0) == b'-' && at(a, 1) == b'-') {
            if at(a, st.optchr + 1) == 0 {
                st.dash = false;
                *optind += 1;
            } else {
                st.optchr += 1;
            }
        } else {
            *optind += 1;
        }
        Got::Opt(idx, None)
    }
}

fn cstr(b: &[u8]) -> Vec<u8> {
    match b.iter().position(|&c| c == 0) {
        Some(p) => b[..p].to_vec(),
        None => b.to_vec(),
    }
}

/// `getopt(string $short_options, array $long_options = [], &$rest_index = null)`.
fn fn_getopt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let short = cstr(&str_param(host, args, 0, "getopt", "short_options")?);
    let longs = match args.get(1).map(deref) {
        None => PhpArray::new(),
        Some(Value::Array(a)) => a,
        Some(other) => {
            return Err(Thrown::type_error(format!(
                "getopt(): Argument #2 ($long_options) must be of type array, {} given",
                value_name(host, &other)
            )))
        }
    };
    let set_rest = |v: i64| {
        if let Some(Value::Ref(r)) = args.get(2) {
            *r.borrow_mut() = Value::Int(v);
        }
    };
    set_rest(1);
    let Some(argv) = host.cli_argv() else {
        return Ok(Value::Bool(false));
    };
    let argv: Vec<Vec<u8>> = argv.iter().map(|a| cstr(a)).collect();
    let mut opts = parse_short_opts(&short);
    for (_, v) in longs.iter() {
        let mut name = cstr(&conv::to_string_bytes(&deref(v)));
        let mut need = 0;
        if name.last() == Some(&b':') {
            need = 1;
            name.pop();
            if name.last() == Some(&b':') {
                need = 2;
                name.pop();
            }
        }
        opts.push(OptSpec {
            ch: 0,
            name: Some(name),
            need_param: need,
        });
    }
    let mut out = PhpArray::new();
    let mut optind = 1usize;
    let mut st = GetoptState {
        optchr: 0,
        dash: false,
    };
    loop {
        match php_getopt(&argv, &opts, &mut optind, &mut st) {
            Got::Eof => break,
            Got::Invalid => continue,
            Got::Opt(i, optarg) => {
                let name = match &opts[i].name {
                    Some(n) => n.clone(),
                    None => vec![opts[i].ch],
                };
                let val = optarg.map(Value::str).unwrap_or(Value::Bool(false));
                // A numeric option name ("1") becomes an integer key.
                let key = ArrayKey::from_bytes(&name);
                match out.get_mut(&key) {
                    Some(existing) => {
                        if let Value::Array(a) = existing {
                            a.push(val);
                        } else {
                            let mut a = PhpArray::new();
                            a.push(existing.clone());
                            a.push(val);
                            *existing = Value::Array(a);
                        }
                    }
                    None => out.insert(key, val),
                }
            }
        }
    }
    set_rest(optind as i64);
    Ok(Value::Array(out))
}

// --- readline / stream knobs --------------------------------------------------

/// `readline($prompt)`: one line from stdin without its newline, or false at
/// EOF. With a non-terminal stdin (libedit's non-interactive mode) the
/// prompt is not echoed.
fn fn_readline(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let prompt = match args.first().map(deref) {
        None | Some(Value::Null) => Vec::new(),
        Some(_) => str_param(host, args, 0, "readline", "prompt")?,
    };
    let tty = unsafe { libc::isatty(0) } == 1;
    if tty && !prompt.is_empty() {
        use std::io::Write;
        let mut so = std::io::stdout();
        let _ = so.write_all(&prompt);
        let _ = so.flush();
    }
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    loop {
        let n = unsafe { libc::read(0, b.as_mut_ptr() as *mut libc::c_void, 1) };
        if n <= 0 {
            if line.is_empty() {
                return Ok(Value::Bool(false));
            }
            break;
        }
        if b[0] == b'\n' {
            break;
        }
        line.push(b[0]);
    }
    Ok(Value::str(line))
}

fn fn_readline_add_history(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let _ = str_param(host, args, 0, "readline_add_history", "prompt")?;
    Ok(Value::Bool(true))
}

fn fn_true(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}

fn stream_id(host: &dyn Host, args: &[Value], func: &str) -> Result<u32, Thrown> {
    match deref(&args[0]) {
        Value::Resource(r) => Ok(r.id),
        other => Err(Thrown::type_error(format!(
            "{func}(): Argument #1 ($stream) must be of type resource, {} given",
            value_name(host, &other)
        ))),
    }
}

fn fn_stream_set_read_buffer(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    stream_id(host, args, "stream_set_read_buffer")?;
    let _ = int_param(host, args, 1, "stream_set_read_buffer", "size", 0)?;
    Ok(Value::Int(0))
}

fn fn_stream_set_chunk_size(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let id = stream_id(host, args, "stream_set_chunk_size")?;
    let size = int_param(host, args, 1, "stream_set_chunk_size", "size", 0)?;
    if size <= 0 {
        return Err(Thrown::error(
            "ValueError",
            "stream_set_chunk_size(): Argument #2 ($size) must be greater than 0",
        ));
    }
    let old = CHUNK_SIZES.with(|c| c.borrow_mut().insert(id, size).unwrap_or(8192));
    Ok(Value::Int(old))
}

// --- class / include introspection ---------------------------------------------

fn fn_get_included_files(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    for f in host.included_files() {
        a.push(Value::str(f));
    }
    Ok(Value::Array(a))
}

fn fn_trait_exists(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, "trait_exists", "trait")?;
    let autoload = crate::argparse::bool_param(host, args, 1, "trait_exists", "autoload", true)?;
    let cid = if autoload {
        host.resolve_class_autoload(&name)?
    } else {
        host.resolve_class(&name)
    };
    Ok(Value::Bool(
        cid.and_then(|c| host.class_kind(c)) == Some(rhp_values::ClassKind::Trait),
    ))
}

/// `get_class_methods(object|string $object_or_class)`.
fn fn_get_class_methods(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let v = deref(&args[0]);
    let cid = match &v {
        Value::Object(h) => Some(h.borrow().class),
        Value::Closure(_) => host.resolve_class(b"Closure"),
        Value::Str(s) => host.resolve_class_autoload(s.as_bytes())?,
        _ => None,
    };
    let Some(cid) = cid else {
        return Err(Thrown::type_error(format!(
            "get_class_methods(): Argument #1 ($object_or_class) must be an object or a valid class name, {} given",
            value_name(host, &v)
        )));
    };
    let mut a = PhpArray::new();
    for n in host.class_method_names(cid) {
        a.push(Value::str(n));
    }
    Ok(Value::Array(a))
}

fn declared(host: &dyn Host, kind: rhp_values::ClassKind) -> Value {
    let mut a = PhpArray::new();
    for n in host.declared_classes(kind) {
        a.push(Value::str(n));
    }
    Value::Array(a)
}

fn fn_get_declared_classes(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(declared(host, rhp_values::ClassKind::Class))
}

fn fn_get_declared_interfaces(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(declared(host, rhp_values::ClassKind::Interface))
}

fn fn_get_declared_traits(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(declared(host, rhp_values::ClassKind::Trait))
}

/// `get_defined_functions(bool $exclude_disabled = true)`: `internal` lists
/// the native builtins (sorted; engine `__` helpers hidden) and the
/// functions the engine defines in PHP source; `user` the script's own.
fn fn_get_defined_functions(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let _ = crate::argparse::bool_param(
        host,
        args,
        0,
        "get_defined_functions",
        "exclude_disabled",
        true,
    )?;
    let (user, engine) = host.defined_functions();
    let mut names: Vec<&str> = crate::registry()
        .keys()
        .copied()
        .filter(|n| !n.starts_with("__"))
        .collect();
    names.sort_unstable();
    let mut internal = PhpArray::new();
    for n in names {
        internal.push(Value::str(n));
    }
    for n in engine {
        internal.push(Value::str(n));
    }
    let mut u = PhpArray::new();
    for n in user {
        u.push(Value::str(n));
    }
    let mut out = PhpArray::new();
    out.insert(ArrayKey::from_bytes(b"internal"), Value::Array(internal));
    out.insert(ArrayKey::from_bytes(b"user"), Value::Array(u));
    Ok(Value::Array(out))
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    let mut put = |name: &'static str, func: crate::NativeFn, min: usize, max: Option<usize>| {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    };
    put("mt_srand", fn_mt_srand, 0, Some(2));
    put("srand", fn_mt_srand, 0, Some(2));
    put("mt_rand", fn_mt_rand, 0, Some(2));
    put("rand", fn_rand, 0, Some(2));
    put("shuffle", fn_shuffle, 1, Some(1));
    put("array_rand", fn_array_rand, 1, Some(2));
    put("str_shuffle", fn_str_shuffle, 1, Some(1));
    put(
        "array_change_key_case",
        fn_array_change_key_case,
        1,
        Some(2),
    );
    put("array_multisort", fn_array_multisort, 1, None);
    put("settype", fn_settype, 2, Some(2));
    put("fnmatch", fn_fnmatch, 2, Some(3));
    put("getrusage", fn_getrusage, 0, Some(1));
    put("gc_status", fn_gc_status, 0, Some(0));
    put(
        "memory_reset_peak_usage",
        fn_memory_reset_peak_usage,
        0,
        Some(0),
    );
    put("gc_mem_caches", fn_gc_mem_caches, 0, Some(0));
    put("sys_getloadavg", fn_sys_getloadavg, 0, Some(0));
    put("opcache_get_status", fn_false, 0, Some(1));
    put("opcache_invalidate", fn_false, 1, Some(2));
    put("opcache_is_script_cached", fn_false, 1, Some(1));
    put("opcache_reset", fn_false, 0, Some(0));
    put("getopt", fn_getopt, 1, Some(3));
    put("readline", fn_readline, 0, Some(1));
    put("readline_add_history", fn_readline_add_history, 1, Some(1));
    put("readline_clear_history", fn_true, 0, Some(0));
    put(
        "stream_set_read_buffer",
        fn_stream_set_read_buffer,
        2,
        Some(2),
    );
    put(
        "stream_set_chunk_size",
        fn_stream_set_chunk_size,
        2,
        Some(2),
    );
    put("get_included_files", fn_get_included_files, 0, Some(0));
    put("get_required_files", fn_get_included_files, 0, Some(0));
    put("trait_exists", fn_trait_exists, 1, Some(2));
    put("get_class_methods", fn_get_class_methods, 1, Some(1));
    put("get_declared_classes", fn_get_declared_classes, 0, Some(0));
    put(
        "get_declared_interfaces",
        fn_get_declared_interfaces,
        0,
        Some(0),
    );
    put("get_declared_traits", fn_get_declared_traits, 0, Some(0));
    put(
        "get_defined_functions",
        fn_get_defined_functions,
        0,
        Some(1),
    );
    put("__stdext_deprecated", fn_stdext_deprecated, 1, Some(1));
    put("__stdext_warning", fn_stdext_warning, 1, Some(1));
}

/// The procedural `date_*` / `timezone_*` API over the DateTime classes
/// (ext/date's function aliases), plus `utf8_encode` / `utf8_decode`.
pub const PRELUDE_PHP: &str = r#"
function date_create(string $datetime = "now", ?DateTimeZone $timezone = null): DateTime|false {
    try { return new DateTime($datetime, $timezone); } catch (Exception $e) { return false; }
}
function date_create_immutable(string $datetime = "now", ?DateTimeZone $timezone = null): DateTimeImmutable|false {
    try { return new DateTimeImmutable($datetime, $timezone); } catch (Exception $e) { return false; }
}
function date_create_from_format(string $format, string $datetime, ?DateTimeZone $timezone = null): DateTime|false {
    return DateTime::createFromFormat($format, $datetime, $timezone);
}
function date_create_immutable_from_format(string $format, string $datetime, ?DateTimeZone $timezone = null): DateTimeImmutable|false {
    return DateTimeImmutable::createFromFormat($format, $datetime, $timezone);
}
function date_format(DateTimeInterface $object, string $format): string { return $object->format($format); }
function date_modify(DateTime $object, string $modifier): DateTime|false {
    try { return $object->modify($modifier); } catch (DateMalformedStringException $e) {
        __stdext_warning('date_modify(): ' . substr($e->getMessage(), strlen('DateTime::modify(): ')));
        return false;
    }
}
function date_add(DateTime $object, DateInterval $interval): DateTime { return $object->add($interval); }
function date_sub(DateTime $object, DateInterval $interval): DateTime { return $object->sub($interval); }
function date_diff(DateTimeInterface $baseObject, DateTimeInterface $targetObject, bool $absolute = false): DateInterval {
    return $baseObject->diff($targetObject, $absolute);
}
function date_timestamp_get(DateTimeInterface $object): int { return $object->getTimestamp(); }
function date_timestamp_set(DateTime $object, int $timestamp): DateTime { return $object->setTimestamp($timestamp); }
function date_timezone_get(DateTimeInterface $object): DateTimeZone|false { return $object->getTimezone(); }
function date_timezone_set(DateTime $object, DateTimeZone $timezone): DateTime { return $object->setTimezone($timezone); }
function date_offset_get(DateTimeInterface $object): int { return $object->getOffset(); }
function date_date_set(DateTime $object, int $year, int $month, int $day): DateTime { return $object->setDate($year, $month, $day); }
function date_time_set(DateTime $object, int $hour, int $minute, int $second = 0, int $microsecond = 0): DateTime {
    return $object->setTime($hour, $minute, $second, $microsecond);
}
function date_interval_format(DateInterval $object, string $format): string { return $object->format($format); }
function date_interval_create_from_date_string(string $datetime): DateInterval|false {
    return DateInterval::createFromDateString($datetime);
}
function timezone_open(string $timezone): DateTimeZone|false {
    try { return new DateTimeZone($timezone); } catch (Exception $e) {
        __stdext_warning("timezone_open(): Unknown or bad timezone ($timezone)");
        return false;
    }
}
function timezone_name_get(DateTimeZone $object): string { return $object->getName(); }
function timezone_offset_get(DateTimeZone $object, DateTimeInterface $datetime): int { return $object->getOffset($datetime); }
function utf8_encode(string $string): string {
    __stdext_deprecated('Function utf8_encode() is deprecated since 8.2, visit the php.net documentation for various alternatives');
    return mb_convert_encoding($string, 'UTF-8', 'ISO-8859-1');
}
function utf8_decode(string $string): string {
    __stdext_deprecated('Function utf8_decode() is deprecated since 8.2, visit the php.net documentation for various alternatives');
    return mb_convert_encoding($string, 'ISO-8859-1', 'UTF-8');
}
"#;

/// `__stdext_deprecated($message)`: E_DEPRECATED from a prelude function.
fn fn_stdext_deprecated(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let msg = conv::to_string_bytes(&deref(&args[0]));
    host.emit_diagnostic(crate::E_DEPRECATED, &String::from_utf8_lossy(&msg))?;
    Ok(Value::Null)
}

/// `__stdext_warning($message)`: E_WARNING from a prelude function.
fn fn_stdext_warning(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let msg = conv::to_string_bytes(&deref(&args[0]));
    host.emit_diagnostic(crate::E_WARNING, &String::from_utf8_lossy(&msg))?;
    Ok(Value::Null)
}

/// The `FNM_*` / `MT_RAND_*` constants.
pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("FNM_NOESCAPE", Value::Int(libc::FNM_NOESCAPE as i64)),
        ("FNM_PATHNAME", Value::Int(libc::FNM_PATHNAME as i64)),
        ("FNM_PERIOD", Value::Int(libc::FNM_PERIOD as i64)),
        ("FNM_CASEFOLD", Value::Int(libc::FNM_CASEFOLD as i64)),
        ("CASE_LOWER", Value::Int(0)),
        ("CASE_UPPER", Value::Int(1)),
        ("MT_RAND_MT19937", Value::Int(MT_RAND_MT19937)),
        ("MT_RAND_PHP", Value::Int(MT_RAND_PHP)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mt_matches_php_seed_1() {
        // php: mt_srand(1); mt_rand() === 895547922
        let mut mt = Mt::seeded(1, MT_RAND_MT19937);
        assert_eq!(mt.next32() >> 1, 895547922);
    }
}
