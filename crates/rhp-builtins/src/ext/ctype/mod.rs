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

use std::collections::HashMap;
use std::sync::OnceLock;

use rhp_values::Value;

use crate::{Builtin, Host, Thrown};

/// Local deref: unwrap a `Ref` to an owned clone of its target.
fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

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

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    macro_rules! reg {
        ($name:literal, $func:path, $min:literal, $max:expr) => {
            m.insert(
                $name,
                Builtin {
                    name: $name,
                    func: $func,
                    min_args: $min,
                    max_args: $max,
                },
            );
        };
    }
    reg!("ctype_digit", fn_ctype_digit, 1, Some(1));
    reg!("ctype_alpha", fn_ctype_alpha, 1, Some(1));
    reg!("ctype_alnum", fn_ctype_alnum, 1, Some(1));
    reg!("ctype_space", fn_ctype_space, 1, Some(1));
    reg!("ctype_upper", fn_ctype_upper, 1, Some(1));
    reg!("ctype_lower", fn_ctype_lower, 1, Some(1));
    reg!("ctype_punct", fn_ctype_punct, 1, Some(1));
    reg!("ctype_xdigit", fn_ctype_xdigit, 1, Some(1));
    reg!("ctype_cntrl", fn_ctype_cntrl, 1, Some(1));
    reg!("ctype_graph", fn_ctype_graph, 1, Some(1));
    reg!("ctype_print", fn_ctype_print, 1, Some(1));
}

// ---------------------------------------------------------------------------
// tests — expected values captured from php 8.5.8
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn s(b: &str) -> Value {
        Value::str(b)
    }

    #[test]
    fn ctype_strings() {
        assert!(ctype_check(&s("1234"), is_digit));
        assert!(!ctype_check(&s("12.3"), is_digit));
        assert!(!ctype_check(&s(""), is_digit)); // empty -> false
        assert!(ctype_check(&s("Hello"), is_alpha));
        assert!(!ctype_check(&s("Hello1"), is_alpha));
        assert!(ctype_check(&s("abc123"), is_alnum));
        assert!(!ctype_check(&s("abc 123"), is_alnum));
        assert!(ctype_check(&s(" \t\n"), is_space));
        assert!(!ctype_check(&s(" x"), is_space));
        assert!(ctype_check(&s("ABC"), is_upper));
        assert!(!ctype_check(&s("ABc"), is_upper));
        assert!(ctype_check(&s("abc"), is_lower));
        assert!(ctype_check(&s("!?#"), is_punct));
        assert!(!ctype_check(&s("!a"), is_punct));
        assert!(!ctype_check(&s(" "), is_punct));
        assert!(ctype_check(&s("1aF"), is_xdigit));
        assert!(!ctype_check(&s("1aG"), is_xdigit));
    }

    #[test]
    fn ctype_int_quirk() {
        assert!(ctype_check(&Value::Int(48), is_digit)); // '0'
        assert!(!ctype_check(&Value::Int(1), is_digit)); // SOH
        assert!(ctype_check(&Value::Int(256), is_digit)); // -> "256"
        assert!(!ctype_check(&Value::Int(-1), is_digit)); // byte 255
        assert!(!ctype_check(&Value::Int(-128), is_digit)); // byte 128
        assert!(!ctype_check(&Value::Int(-129), is_digit)); // -> "-129"
        assert!(ctype_check(&Value::Int(99999999999), is_digit)); // -> all digits
        assert!(!ctype_check(&Value::Int(-99999), is_digit)); // -> "-99999"
        assert!(ctype_check(&Value::Int(100), is_alnum)); // 'd'
        assert!(!ctype_check(&Value::Int(0), is_alpha)); // NUL
    }

    #[test]
    fn ctype_non_string_non_int() {
        assert!(!ctype_check(&Value::Bool(true), is_digit));
        assert!(!ctype_check(&Value::Float(1.5), is_digit));
        assert!(!ctype_check(&Value::Float(48.0), is_digit));
        assert!(!ctype_check(&Value::Null, is_digit));
    }

    #[test]
    fn ctype_classes_locale_independent_part() {
        // High bytes classify per the host locale (as in php), so only assert
        // what holds in every locale: the ASCII classes, and that no high byte
        // is a digit / hex digit. The live `ctype` difftest sweep checks the
        // high bytes against the php on the same machine.
        for b in 0u8..128 {
            let c = b as char;
            assert_eq!(is_digit(b), c.is_ascii_digit(), "digit {b}");
            assert_eq!(is_xdigit(b), c.is_ascii_hexdigit(), "xdigit {b}");
            assert_eq!(is_upper(b), c.is_ascii_uppercase(), "upper {b}");
            assert_eq!(is_lower(b), c.is_ascii_lowercase(), "lower {b}");
            assert_eq!(is_alpha(b), c.is_ascii_alphabetic(), "alpha {b}");
            assert_eq!(is_alnum(b), c.is_ascii_alphanumeric(), "alnum {b}");
            assert_eq!(is_punct(b), c.is_ascii_punctuation(), "punct {b}");
            assert_eq!(is_cntrl(b), c.is_ascii_control(), "cntrl {b}");
            assert_eq!(is_graph(b), c.is_ascii_graphic(), "graph {b}");
            assert_eq!(is_print(b), c.is_ascii_graphic() || b == b' ', "print {b}");
            assert_eq!(
                is_space(b),
                matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'),
                "space {b}"
            );
        }
        for b in 128u8..=255 {
            assert!(!is_digit(b) && !is_xdigit(b), "high byte {b} is a digit");
        }
    }
}
