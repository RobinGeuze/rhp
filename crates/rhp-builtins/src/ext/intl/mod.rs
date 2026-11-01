//! The `intl` extension — the ICU-backed subset Composer and its bundled
//! `symfony/string` reach for: grapheme-cluster string functions
//! (`grapheme_*`), Unicode normalization (`normalizer_*` / `Normalizer`) and
//! IDNA (`idn_to_ascii` / `idn_to_utf8`), plus the global error state
//! (`intl_get_error_code` / `intl_get_error_message`).
//!
//! php links ICU; rhp uses the ICU4X crates (`icu_segmenter`,
//! `icu_normalizer`, `icu_properties`) which carry the same Unicode data, and a
//! UTS #46 processor modelled on ICU's `uts46.cpp` (see [`idn`]).
//!
//! `extension_loaded('intl')` stays false: the rest of the extension
//! (Collator, NumberFormatter, Locale, …) is not implemented, and callers that
//! gate on the extension would otherwise assume it.

use std::cell::RefCell;
use std::collections::HashMap;

use rhp_values::Value;

use crate::{Builtin, Host, Thrown};

mod grapheme;
mod idn;
mod normalizer;
mod uconst;

// ---------------------------------------------------------------------------
// global error state (intl_error.c)
// ---------------------------------------------------------------------------

/// ICU `UErrorCode`s the implemented functions can report.
pub(crate) const U_ZERO_ERROR: i64 = 0;
pub(crate) const U_ILLEGAL_ARGUMENT_ERROR: i64 = 1;
pub(crate) const U_INVALID_CHAR_FOUND: i64 = 10;

/// ICU's `u_errorName`.
fn u_error_name(code: i64) -> &'static str {
    match code {
        66560 => return "U_STRINGPREP_PROHIBITED_ERROR",
        66561 => return "U_STRINGPREP_UNASSIGNED_ERROR",
        66562 => return "U_STRINGPREP_CHECK_BIDI_ERROR",
        66816 => return "U_PLUGIN_TOO_HIGH",
        66817 => return "U_PLUGIN_DIDNT_SET_LEVEL",
        _ => {}
    }
    uconst::U_CONSTANTS
        .iter()
        .find(|(n, v)| {
            *v == code && !n.ends_with("_START") && !n.ends_with("_LIMIT") && !n.ends_with("_FIRST")
        })
        .map(|(n, _)| *n)
        .unwrap_or("[BOGUS UErrorCode]")
}

struct IntlError {
    code: i64,
    msg: Option<String>,
}

thread_local! {
    static ERR: RefCell<IntlError> = const { RefCell::new(IntlError { code: 0, msg: None }) };
}

/// `intl_error_reset(NULL)`.
pub(crate) fn error_reset() {
    ERR.with(|e| {
        let mut e = e.borrow_mut();
        e.code = U_ZERO_ERROR;
        e.msg = None;
    });
}

/// `intl_error_set(NULL, code, msg)`: the message is prefixed with the
/// calling function's name, as php does for the global error.
pub(crate) fn error_set(fname: &str, code: i64, msg: &str) {
    ERR.with(|e| {
        let mut e = e.borrow_mut();
        e.code = code;
        e.msg = Some(format!("{fname}(): {msg}"));
    });
}

fn fn_intl_get_error_code(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(ERR.with(|e| e.borrow().code)))
}

fn fn_intl_get_error_message(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let s = ERR.with(|e| {
        let e = e.borrow();
        match &e.msg {
            Some(m) => format!("{m}: {}", u_error_name(e.code)),
            None => u_error_name(e.code).to_string(),
        }
    });
    Ok(Value::str(s.into_bytes()))
}

fn fn_intl_is_failure(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = super::zpp::int_arg(host, args, 0, "intl_is_failure", "errorCode")?;
    Ok(Value::Bool(code > U_ZERO_ERROR))
}

fn fn_intl_error_name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = super::zpp::int_arg(host, args, 0, "intl_error_name", "errorCode")?;
    Ok(Value::str(u_error_name(code)))
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// `intl_convert_utf8_to_utf16` succeeds exactly when the input is well-formed
/// UTF-8.
pub(crate) fn as_utf8(b: &[u8]) -> Option<&str> {
    std::str::from_utf8(b).ok()
}

/// `Z_PARAM_PATH`: a string that must not contain NUL bytes.
pub(crate) fn path_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<Vec<u8>, Thrown> {
    if args.len() <= i {
        return Ok(Vec::new());
    }
    let s = super::zpp::str_arg(host, args, i, fname, pname)?;
    if s.contains(&0) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{fname}(): Argument #{} (${pname}) must not contain any null bytes",
                i + 1
            ),
        ));
    }
    Ok(s)
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
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
    put(m, "intl_get_error_code", fn_intl_get_error_code, 0, Some(0));
    put(
        m,
        "intl_get_error_message",
        fn_intl_get_error_message,
        0,
        Some(0),
    );
    put(m, "intl_is_failure", fn_intl_is_failure, 1, Some(1));
    put(m, "intl_error_name", fn_intl_error_name, 1, Some(1));

    put(m, "grapheme_strlen", grapheme::fn_strlen, 1, Some(1));
    put(m, "grapheme_strpos", grapheme::fn_strpos, 2, Some(4));
    put(m, "grapheme_stripos", grapheme::fn_stripos, 2, Some(4));
    put(m, "grapheme_strrpos", grapheme::fn_strrpos, 2, Some(4));
    put(m, "grapheme_strripos", grapheme::fn_strripos, 2, Some(4));
    put(m, "grapheme_substr", grapheme::fn_substr, 2, Some(4));
    put(m, "grapheme_strstr", grapheme::fn_strstr, 2, Some(4));
    put(m, "grapheme_stristr", grapheme::fn_stristr, 2, Some(4));
    put(m, "grapheme_str_split", grapheme::fn_str_split, 1, Some(2));
    put(m, "grapheme_extract", grapheme::fn_extract, 2, Some(5));
    put(
        m,
        "grapheme_levenshtein",
        grapheme::fn_levenshtein,
        2,
        Some(6),
    );

    put(
        m,
        "normalizer_normalize",
        normalizer::fn_normalize,
        1,
        Some(2),
    );
    put(
        m,
        "normalizer_is_normalized",
        normalizer::fn_is_normalized,
        1,
        Some(2),
    );
    put(
        m,
        "normalizer_get_raw_decomposition",
        normalizer::fn_get_raw_decomposition,
        1,
        Some(2),
    );

    put(m, "idn_to_ascii", idn::fn_idn_to_ascii, 1, Some(4));
    put(m, "idn_to_utf8", idn::fn_idn_to_utf8, 1, Some(4));
}

/// The `Normalizer` class: static wrappers over the procedural functions.
pub const PRELUDE_PHP: &str = r#"
class Normalizer {
    const FORM_D = 4;
    const NFD = 4;
    const FORM_KD = 8;
    const NFKD = 8;
    const FORM_C = 16;
    const NFC = 16;
    const FORM_KC = 32;
    const NFKC = 32;
    const FORM_KC_CF = 48;
    const NFKC_CF = 48;
    public static function normalize(string $string, int $form = 16): string|false { return normalizer_normalize($string, $form); }
    public static function isNormalized(string $string, int $form = 16): bool { return normalizer_is_normalized($string, $form); }
    public static function getRawDecomposition(string $string, int $form = 16): ?string { return normalizer_get_raw_decomposition($string, $form); }
}
"#;

pub fn constants() -> Vec<(&'static str, Value)> {
    let mut v = vec![
        ("INTL_MAX_LOCALE_LEN", Value::Int(156)),
        ("INTL_ICU_VERSION", Value::str("78.3")),
        ("INTL_ICU_DATA_VERSION", Value::str("78.3")),
        ("GRAPHEME_EXTR_COUNT", Value::Int(0)),
        ("GRAPHEME_EXTR_MAXBYTES", Value::Int(1)),
        ("GRAPHEME_EXTR_MAXCHARS", Value::Int(2)),
        ("IDNA_DEFAULT", Value::Int(idn::IDNA_DEFAULT)),
        ("IDNA_ALLOW_UNASSIGNED", Value::Int(1)),
        ("IDNA_USE_STD3_RULES", Value::Int(2)),
        ("IDNA_CHECK_BIDI", Value::Int(4)),
        ("IDNA_CHECK_CONTEXTJ", Value::Int(8)),
        ("IDNA_NONTRANSITIONAL_TO_ASCII", Value::Int(16)),
        ("IDNA_NONTRANSITIONAL_TO_UNICODE", Value::Int(32)),
        ("INTL_IDNA_VARIANT_UTS46", Value::Int(1)),
        ("IDNA_ERROR_EMPTY_LABEL", Value::Int(1)),
        ("IDNA_ERROR_LABEL_TOO_LONG", Value::Int(2)),
        ("IDNA_ERROR_DOMAIN_NAME_TOO_LONG", Value::Int(4)),
        ("IDNA_ERROR_LEADING_HYPHEN", Value::Int(8)),
        ("IDNA_ERROR_TRAILING_HYPHEN", Value::Int(16)),
        ("IDNA_ERROR_HYPHEN_3_4", Value::Int(32)),
        ("IDNA_ERROR_LEADING_COMBINING_MARK", Value::Int(64)),
        ("IDNA_ERROR_DISALLOWED", Value::Int(128)),
        ("IDNA_ERROR_PUNYCODE", Value::Int(256)),
        ("IDNA_ERROR_LABEL_HAS_DOT", Value::Int(512)),
        ("IDNA_ERROR_INVALID_ACE_LABEL", Value::Int(1024)),
        ("IDNA_ERROR_BIDI", Value::Int(2048)),
        ("IDNA_ERROR_CONTEXTJ", Value::Int(4096)),
    ];
    v.extend(uconst::U_CONSTANTS.iter().map(|&(n, c)| (n, Value::Int(c))));
    v
}
