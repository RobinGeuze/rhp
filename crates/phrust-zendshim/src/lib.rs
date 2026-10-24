//! Rust side of the Zend macro-shim bridge.
//!
//! This crate links a small amount of C (`csrc/shim.c`, plus hand-written and
//! vendored "extension" sources) compiled against our own `include/zend_shim.h`.
//! The C never touches php; it manipulates the shim's own `zval`/`zend_string`
//! structs and, for arrays, calls back into the Rust callbacks here so phrust's
//! [`PhpArray`] provides the real key/order semantics.
//!
//! Flow of one call ([`call`]):
//!   1. reset the per-call arena + array registry;
//!   2. marshal `&[Value]` arguments into shim `zval`s (strings via the C
//!      allocator so `ZSTR_VAL`/`efree` stay consistent);
//!   3. invoke the C entry point (`zif_<name>`) with a synthetic `execute_data`
//!      and a zeroed `return_value`;
//!   4. if the C set a pending exception, convert it to [`Thrown`];
//!   5. otherwise marshal `return_value` back to an owned [`Value`]
//!      (arrays pulled out of the registry, nested arrays included);
//!   6. free the arena.
//!
//! Everything is thread-local, so calls on different threads do not interfere.

use std::cell::RefCell;
use std::ffi::c_void;
use std::os::raw::c_char;

use phrust_values::{ArrayKey, PhpArray, PhpStr, Thrown, Value};

// ---------------------------------------------------------------------------
// Shim ABI mirror (must match include/zend_shim.h byte-for-byte)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
union ZendValue {
    lval: i64,
    dval: f64,
    str_: *mut c_void, // zend_string*, opaque here (read via C helpers)
    arr: u32,          // 1-based registry handle
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Zval {
    value: ZendValue,
    u1_type: u8,
}

#[repr(C)]
struct ZendExecuteData {
    argc: u32,
    argv: *mut Zval,
}

// Type tags — mirror of the IS_* numbers in the header.
const IS_NULL: u8 = 1;
const IS_FALSE: u8 = 2;
const IS_TRUE: u8 = 3;
const IS_LONG: u8 = 4;
const IS_DOUBLE: u8 = 5;
const IS_STRING: u8 = 6;
const IS_ARRAY: u8 = 7;

extern "C" {
    // glue (shim.c)
    fn zshim_string_init(s: *const c_char, len: usize) -> *mut c_void;
    fn zshim_str_ptr(s: *const c_void) -> *const c_char;
    fn zshim_str_len(s: *const c_void) -> usize;
    fn zshim_arena_reset();
    fn zshim_has_exception() -> i32;

    // extension entry points (compiled from C against the shim)
    fn zif_demo_sum_and_label(ed: *mut ZendExecuteData, rv: *mut Zval);
}

#[cfg(zendshim_bin2hex)]
extern "C" {
    fn zif_bin2hex(ed: *mut ZendExecuteData, rv: *mut Zval);
}

// ---------------------------------------------------------------------------
// Per-call thread-local state: array registry + pending exception
// ---------------------------------------------------------------------------

thread_local! {
    static ARRAYS: RefCell<Vec<Option<PhpArray>>> = const { RefCell::new(Vec::new()) };
    static EXCEPTION: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

fn key_from(k: *const c_char, klen: usize) -> ArrayKey {
    let bytes = unsafe { std::slice::from_raw_parts(k as *const u8, klen) };
    ArrayKey::from_bytes(bytes)
}

fn with_array<R>(h: u32, f: impl FnOnce(&mut PhpArray) -> R) -> R {
    ARRAYS.with(|a| {
        let mut a = a.borrow_mut();
        let slot = &mut a[(h - 1) as usize];
        f(slot.as_mut().expect("array handle live"))
    })
}

// ---------------------------------------------------------------------------
// Rust callbacks invoked from shim.c
// ---------------------------------------------------------------------------

#[no_mangle]
extern "C" fn rs_array_new() -> u32 {
    ARRAYS.with(|a| {
        let mut a = a.borrow_mut();
        a.push(Some(PhpArray::new()));
        a.len() as u32 // 1-based
    })
}

#[no_mangle]
extern "C" fn rs_array_set_long(h: u32, k: *const c_char, klen: usize, v: i64) {
    let key = key_from(k, klen);
    with_array(h, |arr| arr.insert(key, Value::Int(v)));
}

#[no_mangle]
extern "C" fn rs_array_set_double(h: u32, k: *const c_char, klen: usize, v: f64) {
    let key = key_from(k, klen);
    with_array(h, |arr| arr.insert(key, Value::Float(v)));
}

#[no_mangle]
extern "C" fn rs_array_set_bool(h: u32, k: *const c_char, klen: usize, v: i32) {
    let key = key_from(k, klen);
    with_array(h, |arr| arr.insert(key, Value::Bool(v != 0)));
}

#[no_mangle]
extern "C" fn rs_array_set_null(h: u32, k: *const c_char, klen: usize) {
    let key = key_from(k, klen);
    with_array(h, |arr| arr.insert(key, Value::Null));
}

#[no_mangle]
extern "C" fn rs_array_set_str(h: u32, k: *const c_char, klen: usize, v: *const c_char, vlen: usize) {
    let key = key_from(k, klen);
    let bytes = unsafe { std::slice::from_raw_parts(v as *const u8, vlen) };
    let val = Value::Str(PhpStr::from_bytes(bytes.to_vec()));
    with_array(h, |arr| arr.insert(key, val));
}

#[no_mangle]
extern "C" fn rs_array_set_arr(h: u32, k: *const c_char, klen: usize, child: u32) {
    let key = key_from(k, klen);
    let child_arr = ARRAYS.with(|a| a.borrow_mut()[(child - 1) as usize].take());
    if let Some(c) = child_arr {
        with_array(h, |arr| arr.insert(key, Value::Array(c)));
    }
}

#[no_mangle]
extern "C" fn rs_array_set_index_long(h: u32, idx: u64, v: i64) {
    with_array(h, |arr| arr.insert(ArrayKey::Int(idx as i64), Value::Int(v)));
}

#[no_mangle]
extern "C" fn rs_array_push_long(h: u32, v: i64) {
    with_array(h, |arr| arr.push(Value::Int(v)));
}

#[no_mangle]
extern "C" fn rs_array_push_double(h: u32, v: f64) {
    with_array(h, |arr| arr.push(Value::Float(v)));
}

#[no_mangle]
extern "C" fn rs_array_push_str(h: u32, v: *const c_char, vlen: usize) {
    let bytes = unsafe { std::slice::from_raw_parts(v as *const u8, vlen) };
    let val = Value::Str(PhpStr::from_bytes(bytes.to_vec()));
    with_array(h, |arr| arr.push(val));
}

#[no_mangle]
extern "C" fn rs_array_push_arr(h: u32, child: u32) {
    let child_arr = ARRAYS.with(|a| a.borrow_mut()[(child - 1) as usize].take());
    if let Some(c) = child_arr {
        with_array(h, |arr| arr.push(Value::Array(c)));
    }
}

#[no_mangle]
extern "C" fn rs_set_exception(class_name: *const c_char, message: *const c_char) {
    let class = unsafe { cstr(class_name) };
    let msg = unsafe { cstr(message) };
    EXCEPTION.with(|e| *e.borrow_mut() = Some((class, msg)));
}

unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *p.add(len) != 0 {
        len += 1;
    }
    let bytes = std::slice::from_raw_parts(p as *const u8, len);
    String::from_utf8_lossy(bytes).into_owned()
}

// ---------------------------------------------------------------------------
// Marshalling
// ---------------------------------------------------------------------------

/// Convert one owned `Value` into a shim `zval`. String bytes are copied into a
/// C-allocated `zend_string` (arena-tracked) so the ext sees normal `ZSTR_*`.
fn value_to_zval(v: &Value) -> Zval {
    let v = deref_clone(v);
    match v {
        Value::Null => Zval { value: ZendValue { lval: 0 }, u1_type: IS_NULL },
        Value::Bool(b) => Zval {
            value: ZendValue { lval: 0 },
            u1_type: if b { IS_TRUE } else { IS_FALSE },
        },
        Value::Int(n) => Zval { value: ZendValue { lval: n }, u1_type: IS_LONG },
        Value::Float(f) => Zval { value: ZendValue { dval: f }, u1_type: IS_DOUBLE },
        Value::Str(s) => {
            let bytes = s.as_bytes();
            let ptr = unsafe { zshim_string_init(bytes.as_ptr() as *const c_char, bytes.len()) };
            Zval { value: ZendValue { str_: ptr }, u1_type: IS_STRING }
        }
        // Inbound arrays/objects are not yet bridged (the ext would need
        // zend_hash to read them). Pass NULL; documented limitation.
        _ => Zval { value: ZendValue { lval: 0 }, u1_type: IS_NULL },
    }
}

fn deref_clone(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        other => other.clone(),
    }
}

/// Convert a shim `return_value` zval back into an owned `Value`.
fn zval_to_value(z: &Zval) -> Value {
    unsafe {
        match z.u1_type {
            IS_NULL => Value::Null,
            IS_TRUE => Value::Bool(true),
            IS_FALSE => Value::Bool(false),
            IS_LONG => Value::Int(z.value.lval),
            IS_DOUBLE => Value::Float(z.value.dval),
            IS_STRING => {
                let p = z.value.str_;
                let ptr = zshim_str_ptr(p);
                let len = zshim_str_len(p);
                let bytes = std::slice::from_raw_parts(ptr as *const u8, len);
                Value::Str(PhpStr::from_bytes(bytes.to_vec()))
            }
            IS_ARRAY => {
                let h = z.value.arr;
                let arr = ARRAYS.with(|a| a.borrow_mut()[(h - 1) as usize].take());
                Value::Array(arr.unwrap_or_default())
            }
            _ => Value::Null,
        }
    }
}

// ---------------------------------------------------------------------------
// The bridge
// ---------------------------------------------------------------------------

type ZifFn = unsafe extern "C" fn(*mut ZendExecuteData, *mut Zval);

fn call(entry: ZifFn, args: &[Value]) -> Result<Value, Thrown> {
    // Fresh per-call state.
    ARRAYS.with(|a| a.borrow_mut().clear());
    EXCEPTION.with(|e| *e.borrow_mut() = None);
    unsafe { zshim_arena_reset() };

    let mut argv: Vec<Zval> = args.iter().map(value_to_zval).collect();
    let mut ed = ZendExecuteData {
        argc: argv.len() as u32,
        argv: argv.as_mut_ptr(),
    };
    let mut rv = Zval { value: ZendValue { lval: 0 }, u1_type: IS_NULL };

    unsafe { entry(&mut ed, &mut rv) };

    let result = if unsafe { zshim_has_exception() } != 0 {
        let exc = EXCEPTION.with(|e| e.borrow_mut().take());
        Err(exc_to_thrown(exc))
    } else {
        Ok(zval_to_value(&rv))
    };

    // Everything the ext allocated is copied out by now; drop the arena.
    unsafe { zshim_arena_reset() };
    ARRAYS.with(|a| a.borrow_mut().clear());
    result
}

fn exc_to_thrown(exc: Option<(String, String)>) -> Thrown {
    let (class, msg) = exc.unwrap_or_else(|| ("Exception".into(), "exception".into()));
    let class_static: &'static str = match class.as_str() {
        "TypeError" => "TypeError",
        "ValueError" => "ValueError",
        "ArgumentCountError" => "ArgumentCountError",
        "Error" => "Error",
        _ => "Exception",
    };
    Thrown::error(class_static, msg)
}

// ---------------------------------------------------------------------------
// Public entry points (wired as phrust builtins by phrust-builtins)
// ---------------------------------------------------------------------------

/// Stage 1 demo: `demo_sum_and_label(int $a, int $b, string $label): array`.
pub fn demo_sum_and_label(args: &[Value]) -> Result<Value, Thrown> {
    call(zif_demo_sum_and_label, args)
}

/// Stage 2: `bin2hex(string $data): string`, compiled from php-src's actual
/// `php_bin2hex` source against the shim.
#[cfg(zendshim_bin2hex)]
pub fn bin2hex(args: &[Value]) -> Result<Value, Thrown> {
    call(zif_bin2hex, args)
}

/// Fallback when the vendored `php_bin2hex.c` is not present at build time.
#[cfg(not(zendshim_bin2hex))]
pub fn bin2hex(_args: &[Value]) -> Result<Value, Thrown> {
    Err(Thrown::unsupported(
        "__zendshim_bin2hex",
        "vendored php_bin2hex.c was not compiled in",
    ))
}

/// Whether the Stage 2 `bin2hex` entry was compiled in.
pub const fn has_bin2hex() -> bool {
    cfg!(zendshim_bin2hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(b: &str) -> Value {
        Value::Str(PhpStr::from_bytes(b.as_bytes().to_vec()))
    }
    fn as_int(v: Option<&Value>) -> Option<i64> {
        match v {
            Some(Value::Int(n)) => Some(*n),
            _ => None,
        }
    }
    fn as_str(v: &Value) -> Vec<u8> {
        match v {
            Value::Str(s) => s.as_bytes().to_vec(),
            _ => panic!("not a string"),
        }
    }

    #[test]
    fn demo_builds_array_through_shim() {
        let r = demo_sum_and_label(&[Value::Int(2), Value::Int(3), s("hello")]).unwrap();
        let Value::Array(a) = r else { panic!("expected array") };
        assert_eq!(a.len(), 3);
        assert_eq!(as_int(a.get(&ArrayKey::from_bytes(b"sum"))), Some(5));
        assert_eq!(as_str(a.get(&ArrayKey::from_bytes(b"label")).unwrap()), b"hello");
        // add_next_index_long on a fresh array → key 0
        assert_eq!(as_int(a.get(&ArrayKey::Int(0))), Some(2));
        // insertion order preserved
        let keys: Vec<_> = a.iter().map(|(k, _)| format!("{k:?}")).collect();
        assert_eq!(keys, vec!["\"sum\"".to_string(), "\"label\"".to_string(), "0".to_string()]);
    }

    #[test]
    fn demo_negative_and_empty_label() {
        let r = demo_sum_and_label(&[Value::Int(-10), Value::Int(7), s("")]).unwrap();
        let Value::Array(a) = r else { panic!("expected array") };
        assert_eq!(as_int(a.get(&ArrayKey::from_bytes(b"sum"))), Some(-3));
        assert_eq!(as_str(a.get(&ArrayKey::from_bytes(b"label")).unwrap()), b"");
        assert_eq!(as_int(a.get(&ArrayKey::Int(0))), Some(-10));
    }

    #[test]
    #[cfg(zendshim_bin2hex)]
    fn bin2hex_real_source() {
        assert_eq!(as_str(&bin2hex(&[s("abc")]).unwrap()), b"616263");
        assert_eq!(as_str(&bin2hex(&[s("")]).unwrap()), b"");
        // binary-safe: NUL and high bytes
        let input = Value::Str(PhpStr::from_bytes(vec![0x00, 0x01, 0xff, 0xfe]));
        assert_eq!(as_str(&bin2hex(&[input]).unwrap()), b"0001fffe");
    }

    #[test]
    fn arena_is_reused_across_calls() {
        // Many calls must not leak handles or corrupt state.
        for i in 0..100 {
            let r = demo_sum_and_label(&[Value::Int(i), Value::Int(1), s("x")]).unwrap();
            let Value::Array(a) = r else { panic!() };
            assert_eq!(as_int(a.get(&ArrayKey::from_bytes(b"sum"))), Some(i + 1));
        }
    }
}
