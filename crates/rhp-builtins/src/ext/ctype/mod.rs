//! The `ctype` extension: character-class tests (`ctype_*`).
//!
//! Reimplemented in Rust and verified byte-for-byte against php 8.5.8
//! (`tools/rhp-difftest` sweep `ctype`).
//!
//! ## Quirks (matched to php)
//! - A `string` yields false when empty, otherwise true iff *every* byte is in
//!   the class.
//! - Any other type raises php 8.1's "Argument of type T will be interpreted
//!   as string in the future" deprecation first.
//! - An `int` in `-128..=255` is interpreted as a single ASCII byte
//!   (`ctype_digit(48)` tests `'0'` → true, `ctype_digit(1)` tests SOH → false).
//! - An `int` outside that range is answered as its decimal string would be
//!   (`ctype_digit(256)` → true, `ctype_digit(-99999)` → false because of the
//!   `-`).
//! - Any other type (bool/float/null/array/object) yields false.
//! - High bytes (128..=255) follow the host libc's `C.UTF-8` locale, exactly as
//!   php does: php's ctype calls libc's `is*()`, and at startup php pins
//!   `LC_CTYPE` to `C.UTF-8` (falling back to `C`) regardless of the environment
//!   (`zend_reset_lc_ctype_locale`). libc's single-byte tables for `C.UTF-8`
//!   differ by platform — macOS classifies bytes as U+0080..U+00FF (so
//!   `ctype_alpha("\xE9")` / `é` → true), glibc is ASCII-only (→ false) — so we
//!   read the same tables from the same libc and match the php on that host.

use std::sync::OnceLock;

use rhp_values::ops::deref;
use rhp_values::Value;

use crate::zend::zpp::zval_type_name;
use crate::{Host, Thrown};

// ---------------------------------------------------------------------------
// ctype character classes (byte predicates), read from libc under C.UTF-8 like
// php (see the module docs). The 256-byte table is built once, inside a
// thread-local locale, so the process-global locale is never touched.
// ---------------------------------------------------------------------------

const ALNUM: u16 = 1 << 0;
const ALPHA: u16 = 1 << 1;
const CNTRL: u16 = 1 << 2;
const DIGIT: u16 = 1 << 3;
const GRAPH: u16 = 1 << 4;
const LOWER: u16 = 1 << 5;
const PRINT: u16 = 1 << 6;
const PUNCT: u16 = 1 << 7;
const SPACE: u16 = 1 << 8;
const UPPER: u16 = 1 << 9;
const XDIGIT: u16 = 1 << 10;

fn class_table() -> &'static [u16; 256] {
    static TABLE: OnceLock<[u16; 256]> = OnceLock::new();
    TABLE.get_or_init(build_class_table)
}

fn build_class_table() -> [u16; 256] {
    let mut table = [0u16; 256];
    // SAFETY: newlocale/uselocale/freelocale are used per their POSIX
    // contracts — the locale is installed only on this thread, the previous
    // thread locale is restored, and the locale is freed after use. If C.UTF-8
    // isn't available, newlocale returns null and we classify under the "C"
    // locale — php's own fallback.
    unsafe {
        let loc = libc::newlocale(
            libc::LC_CTYPE_MASK,
            c"C.UTF-8".as_ptr(),
            std::ptr::null_mut(),
        );
        let prev = if loc.is_null() {
            std::ptr::null_mut()
        } else {
            libc::uselocale(loc)
        };
        let classes: [(unsafe extern "C" fn(libc::c_int) -> libc::c_int, u16); 11] = [
            (libc::isalnum, ALNUM),
            (libc::isalpha, ALPHA),
            (libc::iscntrl, CNTRL),
            (libc::isdigit, DIGIT),
            (libc::isgraph, GRAPH),
            (libc::islower, LOWER),
            (libc::isprint, PRINT),
            (libc::ispunct, PUNCT),
            (libc::isspace, SPACE),
            (libc::isupper, UPPER),
            (libc::isxdigit, XDIGIT),
        ];
        for (b, slot) in table.iter_mut().enumerate() {
            for (test, bit) in classes {
                if test(b as libc::c_int) != 0 {
                    *slot |= bit;
                }
            }
        }
        if !loc.is_null() {
            libc::uselocale(prev);
            libc::freelocale(loc);
        }
    }
    table
}

fn has(b: u8, class: u16) -> bool {
    class_table()[b as usize] & class != 0
}

pub(crate) fn is_alnum(b: u8) -> bool {
    has(b, ALNUM)
}

pub(crate) fn is_alpha(b: u8) -> bool {
    has(b, ALPHA)
}

fn is_cntrl(b: u8) -> bool {
    has(b, CNTRL)
}

fn is_digit(b: u8) -> bool {
    has(b, DIGIT)
}

fn is_graph(b: u8) -> bool {
    has(b, GRAPH)
}

fn is_lower(b: u8) -> bool {
    has(b, LOWER)
}

fn is_print(b: u8) -> bool {
    has(b, PRINT)
}

fn is_punct(b: u8) -> bool {
    has(b, PUNCT)
}

fn is_space(b: u8) -> bool {
    has(b, SPACE)
}

fn is_upper(b: u8) -> bool {
    has(b, UPPER)
}

fn is_xdigit(b: u8) -> bool {
    has(b, XDIGIT)
}

/// `ctype_fallback`'s answer for a non-string argument: an int in
/// -128..=255 is a byte (a negative one plus 256); a larger int passes when
/// the class allows digits, a smaller one when it also allows `-`; any
/// other type fails.
fn fallback(v: &Value, class: fn(u8) -> bool, allow_digits: bool, allow_minus: bool) -> bool {
    let Value::Int(n) = *v else {
        return false;
    };
    match n {
        0..=255 => class(n as u8),
        -128..=-1 => class((n + 256) as u8),
        256.. => allow_digits,
        _ => allow_minus,
    }
}

/// php's `ctype_impl`: a string passes when it is non-empty and every byte
/// is in the class; any other argument takes `ctype_fallback`, which first
/// raises its deprecation.
fn ctype_impl(
    host: &mut dyn Host,
    fname: &str,
    arg: &Value,
    class: fn(u8) -> bool,
    allow_digits: bool,
    allow_minus: bool,
) -> Result<Value, Thrown> {
    let v = deref(arg);
    if let Value::Str(s) = &v {
        let b = s.as_bytes();
        return Ok(Value::Bool(!b.is_empty() && b.iter().all(|&c| class(c))));
    }
    let msg = format!(
        "Argument of type {} will be interpreted as string in the future",
        zval_type_name(host, &v)
    );
    crate::deprecated(host, fname, &msg)?;
    Ok(Value::Bool(fallback(&v, class, allow_digits, allow_minus)))
}

macro_rules! ctype_fn {
    ($name:ident, $php:literal, $class:path, $digits:literal, $minus:literal) => {
        fn $name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            ctype_impl(host, $php, &args[0], $class, $digits, $minus)
        }
    };
}
// php's ctype_impl(iswhat, allow_digits, allow_minus) for each function.
ctype_fn!(fn_ctype_digit, "ctype_digit", is_digit, true, false);
ctype_fn!(fn_ctype_alpha, "ctype_alpha", is_alpha, false, false);
ctype_fn!(fn_ctype_alnum, "ctype_alnum", is_alnum, true, false);
ctype_fn!(fn_ctype_space, "ctype_space", is_space, false, false);
ctype_fn!(fn_ctype_upper, "ctype_upper", is_upper, false, false);
ctype_fn!(fn_ctype_lower, "ctype_lower", is_lower, false, false);
ctype_fn!(fn_ctype_punct, "ctype_punct", is_punct, false, false);
ctype_fn!(fn_ctype_xdigit, "ctype_xdigit", is_xdigit, true, false);
ctype_fn!(fn_ctype_cntrl, "ctype_cntrl", is_cntrl, false, false);
ctype_fn!(fn_ctype_graph, "ctype_graph", is_graph, true, true);
ctype_fn!(fn_ctype_print, "ctype_print", is_print, true, true);

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("ctype_digit", fn_ctype_digit),
            ("ctype_alpha", fn_ctype_alpha),
            ("ctype_alnum", fn_ctype_alnum),
            ("ctype_space", fn_ctype_space),
            ("ctype_upper", fn_ctype_upper),
            ("ctype_lower", fn_ctype_lower),
            ("ctype_punct", fn_ctype_punct),
            ("ctype_xdigit", fn_ctype_xdigit),
            ("ctype_cntrl", fn_ctype_cntrl),
            ("ctype_graph", fn_ctype_graph),
            ("ctype_print", fn_ctype_print),
        ],
    );
}

#[cfg(test)]
mod tests;
