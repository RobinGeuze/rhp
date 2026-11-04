//! The `ctype` extension: character-class tests (`ctype_*`).
//!
//! Reimplemented in Rust and verified byte-for-byte against php 8.5.8
//! (`tools/rhp-difftest` sweep `ctype`).
//!
//! ## Quirks (matched to php)
//! - An `int` in `-128..=255` is interpreted as a single ASCII byte
//!   (`ctype_digit(48)` tests `'0'` → true, `ctype_digit(1)` tests SOH → false).
//! - An `int` outside that range is rendered as its decimal string and tested
//!   whole (`ctype_digit(256)` tests `"256"` → true, `ctype_digit(-99999)`
//!   tests `"-99999"` → false because of the `-`).
//! - A `string` yields false when empty, otherwise true iff *every* byte is in
//!   the class.
//! - Any other type (bool/float/null/array/object) yields false. (php emits a
//!   deprecation notice to stderr for these and for the int-as-string path; we
//!   don't, which is fine — notices go to stderr.)
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

fn is_alnum(b: u8) -> bool {
    has(b, ALNUM)
}

fn is_alpha(b: u8) -> bool {
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

/// Run a ctype class test with php's int/string argument quirks.
fn ctype_check(v: &Value, class: fn(u8) -> bool) -> bool {
    match deref(v) {
        Value::Int(n) => {
            if (-128..=255).contains(&n) {
                class(n as u8)
            } else {
                let s = n.to_string();
                !s.is_empty() && s.bytes().all(class)
            }
        }
        Value::Str(s) => {
            let b = s.as_bytes();
            !b.is_empty() && b.iter().all(|&c| class(c))
        }
        _ => false,
    }
}

macro_rules! ctype_fn {
    ($name:ident, $class:path) => {
        fn $name(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            Ok(Value::Bool(ctype_check(&args[0], $class)))
        }
    };
}
ctype_fn!(fn_ctype_digit, is_digit);
ctype_fn!(fn_ctype_alpha, is_alpha);
ctype_fn!(fn_ctype_alnum, is_alnum);
ctype_fn!(fn_ctype_space, is_space);
ctype_fn!(fn_ctype_upper, is_upper);
ctype_fn!(fn_ctype_lower, is_lower);
ctype_fn!(fn_ctype_punct, is_punct);
ctype_fn!(fn_ctype_xdigit, is_xdigit);
ctype_fn!(fn_ctype_cntrl, is_cntrl);
ctype_fn!(fn_ctype_graph, is_graph);
ctype_fn!(fn_ctype_print, is_print);

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
