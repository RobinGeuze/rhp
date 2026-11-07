//! The native halves of the engine classes and of Random\Randomizer
//! (engine_*.c's methods, randomizer.c, gammasection.c). The classes are
//! in prelude.php; their methods call `__random_engine` / `__randomizer`.

use std::cell::RefCell;
use std::rc::Rc;

use rhp_values::ops::deref;
use rhp_values::{conv, ObjectHandle, PhpArray, Value};

use super::engine::*;
use crate::zend::object_handlers::{self, ClassHandlers, CloneObj, ObjectHandlers};
use crate::{Host, Thrown};

type State = RefCell<Algo>;

/// What a Randomizer draws from: a native engine's state (shared with the
/// engine object, as php copies the state pointer) or a user engine.
#[derive(Clone)]
enum Bound {
    Native(Rc<State>),
    User(Value),
}

struct RandomizerState(RefCell<Option<Bound>>);

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(deref).unwrap_or(Value::Null)
}

fn int(args: &[Value], i: usize) -> i64 {
    conv::to_int(&arg(args, i))
}

fn value_error(m: &str, n: usize, pname: &str, msg: &str) -> Thrown {
    Thrown::error(
        "ValueError",
        format!("{m}(): Argument #{n} (${pname}) {msg}"),
    )
}

fn class_of(host: &dyn Host, v: &Value) -> String {
    String::from_utf8_lossy(&host.object_class_name(v).unwrap_or_default()).into_owned()
}

/// The engine object's state; zeroed for one whose constructor never ran.
fn state_of(host: &dyn Host, this: &Value) -> Option<Rc<State>> {
    if let Some(s) = object_handlers::payload::<State>(this) {
        return Some(s);
    }
    let algo = match class_of(host, this).as_str() {
        "Random\\Engine\\Mt19937" => Algo::Mt(Box::new(Mt::zeroed())),
        "Random\\Engine\\PcgOneseq128XslRr64" => Algo::Pcg(0),
        "Random\\Engine\\Xoshiro256StarStar" => Algo::Xoshiro([0; 4]),
        "Random\\Engine\\Secure" => Algo::Secure,
        _ => return None,
    };
    let s = Rc::new(RefCell::new(algo));
    object_handlers::set_payload(&object_handlers::object_of(this)?, s.clone());
    Some(s)
}

/// The seed argument of the Pcg/Xoshiro constructors: little-endian words
/// of a string of exactly `N * 8` bytes.
fn seed_words<const N: usize>(s: &[u8]) -> Option<[u64; N]> {
    if s.len() != N * 8 {
        return None;
    }
    let mut t = [0u64; N];
    for (i, w) in t.iter_mut().enumerate() {
        *w = u64::from_le_bytes(s[i * 8..i * 8 + 8].try_into().ok()?);
    }
    Some(t)
}

/// `__random_engine($op, $this, ...$args)`.
fn fn_random_engine(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = conv::to_string_bytes(&arg(args, 0));
    let this = arg(args, 1);
    let Some(state) = state_of(host, &this) else {
        return Ok(Value::Null);
    };
    match &op[..] {
        b"Mt19937::__construct" => {
            let mode = int(args, 3);
            match mode {
                MT_RAND_MT19937 => {}
                MT_RAND_PHP => host.emit_diagnostic(
                    crate::E_DEPRECATED,
                    b"The MT_RAND_PHP variant of Mt19937 is deprecated",
                )?,
                _ => {
                    return Err(value_error(
                        "Random\\Engine\\Mt19937::__construct",
                        2,
                        "mode",
                        "must be either MT_RAND_MT19937 or MT_RAND_PHP",
                    ))
                }
            }
            let seed = match arg(args, 2) {
                Value::Null => {
                    let mut b = [0u8; 8];
                    csprng(&mut b).map_err(|_| seed_failed())?;
                    i64::from_le_bytes(b)
                }
                v => conv::to_int(&v),
            };
            *state.borrow_mut() = Algo::Mt(Box::new(Mt::seeded(seed as u32, mode)));
        }
        b"PcgOneseq128XslRr64::__construct" => {
            let s = match arg(args, 2) {
                Value::Null => {
                    let mut b = [0u8; 16];
                    csprng(&mut b).map_err(|_| seed_failed())?;
                    u128::from_le_bytes(b)
                }
                Value::Str(s) => match seed_words::<2>(s.as_bytes()) {
                    Some([hi, lo]) => ((hi as u128) << 64) | lo as u128,
                    None => {
                        return Err(value_error(
                            "Random\\Engine\\PcgOneseq128XslRr64::__construct",
                            1,
                            "seed",
                            "must be a 16 byte (128 bit) string",
                        ))
                    }
                },
                v => conv::to_int(&v) as u64 as u128,
            };
            *state.borrow_mut() = Algo::Pcg(pcg_seed(s));
        }
        b"Xoshiro256StarStar::__construct" => {
            let s = match arg(args, 2) {
                Value::Null => loop {
                    let mut b = [0u8; 32];
                    csprng(&mut b).map_err(|_| seed_failed())?;
                    if let Some(t) = seed_words::<4>(&b).filter(|t| *t != [0; 4]) {
                        break t;
                    }
                },
                Value::Str(s) => {
                    let m = "Random\\Engine\\Xoshiro256StarStar::__construct";
                    match seed_words::<4>(s.as_bytes()) {
                        Some([0, 0, 0, 0]) => {
                            return Err(value_error(
                                m,
                                1,
                                "seed",
                                "must not consist entirely of NUL bytes",
                            ))
                        }
                        Some(t) => t,
                        None => {
                            return Err(value_error(
                                m,
                                1,
                                "seed",
                                "must be a 32 byte (256 bit) string",
                            ))
                        }
                    }
                }
                v => xoshiro_seed64(conv::to_int(&v) as u64),
            };
            *state.borrow_mut() = Algo::Xoshiro(s);
        }
        b"generate" => {
            let g = state.borrow_mut().generate()?;
            return Ok(Value::str(g.bytes()));
        }
        b"jump" => match &mut *state.borrow_mut() {
            Algo::Pcg(s) => {
                let advance = int(args, 2);
                if advance < 0 {
                    return Err(value_error(
                        "Random\\Engine\\PcgOneseq128XslRr64::jump",
                        1,
                        "advance",
                        "must be greater than or equal to 0",
                    ));
                }
                pcg_advance(s, advance as u64);
            }
            Algo::Xoshiro(s) => xoshiro_jump(s),
            _ => {}
        },
        b"jumpLong" => {
            if let Algo::Xoshiro(s) = &mut *state.borrow_mut() {
                xoshiro_jump_long(s);
            }
        }
        b"state" => {
            return Ok(state
                .borrow()
                .state()
                .map(Value::Array)
                .unwrap_or(Value::Null))
        }
        b"unserialize" => {
            let invalid = || {
                Thrown::error(
                    "Exception",
                    format!(
                        "Invalid serialization data for {} object",
                        class_of(host, &this)
                    ),
                )
            };
            let Value::Array(d) = arg(args, 2) else {
                return Err(invalid());
            };
            if d.len() != 2 {
                return Err(invalid());
            }
            // members: the engines declare none and allow no dynamic ones
            match d.get(&rhp_values::ArrayKey::Int(0)) {
                Some(Value::Array(m)) if m.is_empty() => {}
                _ => return Err(invalid()),
            }
            let Some(Value::Array(s)) = d.get(&rhp_values::ArrayKey::Int(1)) else {
                return Err(invalid());
            };
            if !state.borrow_mut().load_state(s) {
                return Err(invalid());
            }
        }
        _ => {}
    }
    Ok(Value::Null)
}

fn seed_failed() -> Thrown {
    Thrown::error(
        "Random\\RandomException",
        "Failed to generate a random seed",
    )
}

/// One draw from the bound engine.
fn generate(host: &mut &mut dyn Host, b: &Bound) -> Result<Gen, Thrown> {
    match b {
        Bound::Native(s) => s.borrow_mut().generate(),
        Bound::User(o) => user_generate(*host, o),
    }
}

/// The engine's `range` handler.
fn engine_range(host: &mut &mut dyn Host, b: &Bound, min: i64, max: i64) -> Result<i64, Thrown> {
    if let Bound::Native(s) = b {
        if matches!(*s.borrow(), Algo::Secure) {
            return secure_range(min, max);
        }
    }
    range(host, &mut |h| generate(h, b), min, max)
}

const R: &str = "Random\\Randomizer";

/// `__randomizer($op, $this, ...$args)`.
fn fn_randomizer(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let op = conv::to_string_bytes(&arg(args, 0));
    let this = arg(args, 1);
    let Some(h) = object_handlers::object_of(&this) else {
        return Ok(Value::Null);
    };
    if op == b"bind" {
        let engine = arg(args, 2);
        let bound = match state_of(host, &engine) {
            Some(s) => Bound::Native(s),
            None => Bound::User(engine),
        };
        object_handlers::set_payload(&h, Rc::new(RandomizerState(RefCell::new(Some(bound)))));
        return Ok(Value::Null);
    }
    let Some(rs) = object_handlers::payload_of::<RandomizerState>(&h) else {
        return Err(Thrown::error(
            "Error",
            "Typed property Random\\Randomizer::$engine must not be accessed before initialization",
        ));
    };
    // a clone: a user engine may rebind this Randomizer
    let Some(bound) = rs.0.borrow().clone() else {
        return Ok(Value::Null);
    };
    let b = &bound;
    let mut host = host;
    let host = &mut host;
    match &op[..] {
        b"nextInt" => {
            let g = generate(host, b)?;
            Ok(Value::Int((g.result >> 1) as i64))
        }
        b"nextFloat" => {
            let (mut result, mut total) = (0u64, 0usize);
            while total < 8 {
                let r = generate(host, b)?;
                result |= r.result << (total * 8);
                total += r.size;
            }
            Ok(Value::Float(
                (1.0 / (1u64 << 53) as f64) * (result >> 11) as f64,
            ))
        }
        b"getFloat" => get_float(host, b, args),
        b"getInt" => {
            let (min, max) = (int(args, 2), int(args, 3));
            if max < min {
                return Err(value_error(
                    &format!("{R}::getInt"),
                    2,
                    "max",
                    "must be greater than or equal to argument #1 ($min)",
                ));
            }
            if let Bound::Native(s) = b {
                if let Algo::Mt(mt) = &mut *s.borrow_mut() {
                    if mt.mode != MT_RAND_MT19937 {
                        let r = (mt.next32() >> 1) as f64;
                        let offset =
                            ((max as f64 - min as f64 + 1.0) * (r / (2147483647.0 + 1.0))) as u64;
                        return Ok(Value::Int(offset.wrapping_add(min as u64) as i64));
                    }
                }
            }
            Ok(Value::Int(engine_range(host, b, min, max)?))
        }
        b"getBytes" => {
            let length = int(args, 2);
            if length < 1 {
                return Err(value_error(
                    &format!("{R}::getBytes"),
                    1,
                    "length",
                    "must be greater than 0",
                ));
            }
            let length = length as usize;
            let mut out = Vec::with_capacity(length);
            while out.len() < length {
                let mut g = generate(host, b)?;
                for _ in 0..g.size {
                    out.push(g.result as u8);
                    g.result >>= 8;
                    if out.len() >= length {
                        break;
                    }
                }
            }
            Ok(Value::str(out))
        }
        b"getBytesFromString" => {
            let source = conv::to_string_bytes(&arg(args, 2));
            let length = int(args, 3);
            let m = format!("{R}::getBytesFromString");
            if source.is_empty() {
                return Err(value_error(&m, 1, "string", "must not be empty"));
            }
            if length < 1 {
                return Err(value_error(&m, 2, "length", "must be greater than 0"));
            }
            let length = length as usize;
            let max_offset = source.len() as u64 - 1;
            let mut out = Vec::with_capacity(length);
            if max_offset > 0xff {
                while out.len() < length {
                    let off = engine_range(host, b, 0, max_offset as i64)?;
                    out.push(source[off as usize]);
                }
            } else {
                let mut mask = max_offset;
                mask |= mask >> 1;
                mask |= mask >> 2;
                mask |= mask >> 4;
                mask = mask.wrapping_mul(0x0101_0101_0101_0101);
                let mut failures = 0;
                'outer: while out.len() < length {
                    let g = generate(host, b)?;
                    let mut offsets = g.result & mask;
                    for _ in 0..g.size {
                        let off = offsets & 0xff;
                        offsets >>= 8;
                        if off > max_offset {
                            failures += 1;
                            if failures > RANGE_ATTEMPTS {
                                return Err(broken_engine());
                            }
                            continue;
                        }
                        failures = 0;
                        out.push(source[off as usize]);
                        if out.len() >= length {
                            break 'outer;
                        }
                    }
                }
            }
            Ok(Value::str(out))
        }
        b"shuffleArray" => {
            let Value::Array(a) = arg(args, 2) else {
                return Ok(Value::Null);
            };
            let mut vals: Vec<Value> = a.values().cloned().collect();
            super::shuffle(&mut vals, &mut |lo, hi| engine_range(host, b, lo, hi))?;
            let mut out = PhpArray::with_capacity(vals.len());
            for v in vals {
                out.push(v);
            }
            Ok(Value::Array(out))
        }
        b"shuffleBytes" => {
            let mut s = conv::to_string_bytes(&arg(args, 2));
            super::shuffle(&mut s, &mut |lo, hi| engine_range(host, b, lo, hi))?;
            Ok(Value::str(s))
        }
        b"pickArrayKeys" => {
            let Value::Array(a) = arg(args, 2) else {
                return Ok(Value::Null);
            };
            let num = int(args, 3);
            let m = format!("{R}::pickArrayKeys");
            match super::pick_keys(&a, num, &mut |lo, hi| engine_range(host, b, lo, hi))? {
                Ok(Value::Array(keys)) => Ok(Value::Array(keys)),
                Ok(key) => {
                    let mut out = PhpArray::new();
                    out.push(key);
                    Ok(Value::Array(out))
                }
                Err(super::PickError::Empty) => {
                    Err(value_error(&m, 1, "array", "must not be empty"))
                }
                Err(super::PickError::Num) => Err(value_error(
                    &m,
                    2,
                    "num",
                    "must be between 1 and the number of elements in argument #1 ($array)",
                )),
            }
        }
        _ => Ok(Value::Null),
    }
}

// --- getFloat (randomizer.c, gammasection.c) -----------------------------------

fn get_float(host: &mut &mut dyn Host, b: &Bound, args: &[Value]) -> Result<Value, Thrown> {
    let min = conv::to_float(&arg(args, 2));
    let max = conv::to_float(&arg(args, 3));
    let boundary = conv::to_string_bytes(&arg(args, 4));
    let m = format!("{R}::getFloat");
    if !min.is_finite() {
        return Err(value_error(&m, 1, "min", "must be finite"));
    }
    if !max.is_finite() {
        return Err(value_error(&m, 2, "max", "must be finite"));
    }
    let greater = || value_error(&m, 2, "max", "must be greater than argument #1 ($min)");
    let mut r64 = |umax: u64| range64(host, &mut |h| generate(h, b), umax);
    let g = gamma_max(min, max);
    let hi = ceilint(min, max, g);
    let near_min = min.abs() <= max.abs();
    let f = match &boundary[..] {
        b"ClosedClosed" => {
            if max < min {
                return Err(value_error(
                    &m,
                    2,
                    "max",
                    "must be greater than or equal to argument #1 ($min)",
                ));
            }
            let k = r64(hi)?;
            if k == hi {
                if near_min {
                    min
                } else {
                    max
                }
            } else if near_min {
                from_max(max, k, g)
            } else {
                from_min(min, k, g)
            }
        }
        b"OpenClosed" => {
            if max <= min {
                return Err(greater());
            }
            if hi < 1 {
                f64::NAN
            } else {
                let k = r64(hi - 1)?;
                if near_min {
                    from_max(max, k, g)
                } else if k == hi - 1 {
                    max
                } else {
                    from_min(min, k + 1, g)
                }
            }
        }
        b"OpenOpen" => {
            if max <= min {
                return Err(greater());
            }
            let f = if hi < 2 {
                f64::NAN
            } else {
                let k = 1 + r64(hi - 2)?;
                if near_min {
                    from_max(max, k, g)
                } else {
                    from_min(min, k, g)
                }
            };
            if f.is_nan() {
                return Err(Thrown::error(
                    "ValueError",
                    "The given interval is empty, there are no floats between argument #1 ($min) and argument #2 ($max)",
                ));
            }
            f
        }
        _ => {
            // ClosedOpen
            if max <= min {
                return Err(greater());
            }
            if hi < 1 {
                f64::NAN
            } else {
                let k = 1 + r64(hi - 1)?;
                if near_min {
                    if k == hi {
                        min
                    } else {
                        from_max(max, k, g)
                    }
                } else {
                    from_min(min, k - 1, g)
                }
            }
        }
    };
    Ok(Value::Float(f))
}

/// C's nextafter for the two directions gammasection.c uses.
fn next_after(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return x + y;
    }
    if x == y {
        return y;
    }
    if x == 0.0 {
        let tiny = f64::from_bits(1);
        return if y > 0.0 { tiny } else { -tiny };
    }
    let bits = x.to_bits();
    let up = (y > x) == (x > 0.0);
    f64::from_bits(if up { bits + 1 } else { bits - 1 })
}

fn gamma_low(x: f64) -> f64 {
    x - next_after(x, -f64::MAX)
}

fn gamma_high(x: f64) -> f64 {
    next_after(x, f64::MAX) - x
}

fn gamma_max(x: f64, y: f64) -> f64 {
    if x.abs() > y.abs() {
        gamma_high(x)
    } else {
        gamma_low(y)
    }
}

fn splitint64(v: u64) -> (f64, f64) {
    ((v >> 2) as f64, (v & 3) as f64)
}

fn ceilint(a: f64, b: f64, g: f64) -> u64 {
    let s = b / g - a / g;
    let e = if a.abs() <= b.abs() {
        -a / g - (s - b / g)
    } else {
        b / g - (s + a / g)
    };
    let si = s.ceil();
    if s != si {
        si as u64
    } else {
        (si as u64).wrapping_add((e > 0.0) as u64)
    }
}

fn from_max(max: f64, k: u64, g: f64) -> f64 {
    let (k_hi, k_lo) = splitint64(k);
    4.0 * (max * 0.25 - k_hi * g) - k_lo * g
}

fn from_min(min: f64, k: u64, g: f64) -> f64 {
    let (k_hi, k_lo) = splitint64(k);
    4.0 * (min * 0.25 + k_hi * g) + k_lo * g
}

// --- object handlers ------------------------------------------------------------

/// php_random_engine_common_clone_object: the copy gets its own state.
fn engine_clone(_: &mut dyn Host, old: &ObjectHandle, new: &ObjectHandle) -> Result<(), Thrown> {
    if let Some(s) = object_handlers::payload_of::<State>(old) {
        let copy = s.borrow().clone();
        object_handlers::set_payload(new, Rc::new(RefCell::new(copy)));
    }
    Ok(())
}

static ENGINE_HANDLERS: ObjectHandlers = ObjectHandlers {
    clone_obj: CloneObj::Custom(engine_clone),
    ..ObjectHandlers::STD
};

pub(crate) static CLASSES: &[ClassHandlers] = &[
    ClassHandlers {
        class: "Random\\Engine\\Mt19937",
        handlers: &ENGINE_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "Random\\Engine\\PcgOneseq128XslRr64",
        handlers: &ENGINE_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "Random\\Engine\\Xoshiro256StarStar",
        handlers: &ENGINE_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "Random\\Engine\\Secure",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
    ClassHandlers {
        class: "Random\\Randomizer",
        handlers: &ObjectHandlers::UNCLONEABLE,
        create_object: true,
    },
];

pub(crate) fn register(m: &mut crate::Registry) {
    crate::register_helpers(
        m,
        &[
            ("__random_engine", fn_random_engine, 2, None),
            ("__randomizer", fn_randomizer, 2, None),
        ],
    );
}
