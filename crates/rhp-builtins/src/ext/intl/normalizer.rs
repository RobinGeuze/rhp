//! `normalizer_*` — Unicode normalization
//! (php-src `ext/intl/normalizer/normalizer_normalize.cpp`).
//!
//! NFC / NFD / NFKC / NFKD come straight from `icu_normalizer`. ICU's
//! `NFKC_Casefold` (`Normalizer::NFKC_CF`) has no ICU4X counterpart and is
//! computed from its definition: NFKC of the full case folding (applied to the
//! compatibility decomposition until stable) with Default_Ignorable code points
//! removed.

use icu_normalizer::properties::{CanonicalDecompositionBorrowed, Decomposed};
use icu_normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
use icu_properties::props::DefaultIgnorableCodePoint;
use icu_properties::CodePointSetData;
use rhp_values::Value;

use super::{as_utf8, error_reset, error_set, U_ILLEGAL_ARGUMENT_ERROR, U_INVALID_CHAR_FOUND};
use crate::ext::zpp::{int_arg_or, str_arg, value_error};
use crate::{Host, Thrown};

const FORM_D: i64 = 4;
const FORM_KD: i64 = 8;
const FORM_C: i64 = 16;
const FORM_KC: i64 = 32;
const FORM_KC_CF: i64 = 48;

fn nfkc_casefold(s: &str) -> String {
    let nfkd = DecomposingNormalizerBorrowed::new_nfkd();
    let ignorable = CodePointSetData::new::<DefaultIgnorableCodePoint>();
    let mut cur: String = nfkd.normalize(s).into_owned();
    // fold + NFKD until a fixed point (bounded; converges in 1-2 rounds).
    for _ in 0..4 {
        let cps: Vec<u32> = cur
            .chars()
            .filter(|&c| !ignorable.contains(c))
            .map(|c| c as u32)
            .collect();
        let folded = crate::ext::mbstring::case_fold_full(&cps);
        let st: String = folded.iter().filter_map(|&c| char::from_u32(c)).collect();
        let next = nfkd.normalize(&st).into_owned();
        if next == cur {
            break;
        }
        cur = next;
    }
    ComposingNormalizerBorrowed::new_nfkc()
        .normalize(&cur)
        .into_owned()
}

fn normalize(form: i64, s: &str) -> String {
    match form {
        FORM_D => DecomposingNormalizerBorrowed::new_nfd()
            .normalize(s)
            .into_owned(),
        FORM_KD => DecomposingNormalizerBorrowed::new_nfkd()
            .normalize(s)
            .into_owned(),
        FORM_KC => ComposingNormalizerBorrowed::new_nfkc()
            .normalize(s)
            .into_owned(),
        FORM_KC_CF => nfkc_casefold(s),
        _ => ComposingNormalizerBorrowed::new_nfc()
            .normalize(s)
            .into_owned(),
    }
}

fn is_normalized(form: i64, s: &str) -> bool {
    match form {
        FORM_D => DecomposingNormalizerBorrowed::new_nfd().is_normalized(s),
        FORM_KD => DecomposingNormalizerBorrowed::new_nfkd().is_normalized(s),
        FORM_KC => ComposingNormalizerBorrowed::new_nfkc().is_normalized(s),
        FORM_KC_CF => nfkc_casefold(s) == s,
        _ => ComposingNormalizerBorrowed::new_nfc().is_normalized(s),
    }
}

fn valid_form(form: i64) -> bool {
    matches!(form, FORM_D | FORM_KD | FORM_C | FORM_KC | FORM_KC_CF)
}

pub(super) fn fn_normalize(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "normalizer_normalize";
    error_reset();
    let s = str_arg(host, args, 0, f, "string")?;
    let form = int_arg_or(host, args, 1, f, "form", FORM_C)?;
    if !valid_form(form) {
        return Err(value_error(
            f,
            2,
            "form",
            "must be a a valid normalization form",
        ));
    }
    let Some(t) = as_utf8(&s) else {
        error_set(
            f,
            U_INVALID_CHAR_FOUND,
            "Error converting input string to UTF-16",
        );
        return Ok(Value::Bool(false));
    };
    Ok(Value::str(normalize(form, t).into_bytes()))
}

pub(super) fn fn_is_normalized(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "normalizer_is_normalized";
    error_reset();
    let s = str_arg(host, args, 0, f, "string")?;
    let form = int_arg_or(host, args, 1, f, "form", FORM_C)?;
    if !valid_form(form) {
        return Err(value_error(
            f,
            2,
            "form",
            "must be a a valid normalization form",
        ));
    }
    let Some(t) = as_utf8(&s) else {
        error_set(
            f,
            U_INVALID_CHAR_FOUND,
            "Error converting string to UTF-16.",
        );
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(is_normalized(form, t)))
}

/// The one-level (raw) decomposition mapping of `c`, if it has one.
fn raw_decomposition(form: i64, c: char) -> Option<String> {
    let canon = match CanonicalDecompositionBorrowed::new().decompose(c) {
        Decomposed::Default => None,
        Decomposed::Singleton(a) => Some(a.to_string()),
        Decomposed::Expansion(a, b) => Some([a, b].iter().collect()),
    };
    if matches!(form, FORM_KD | FORM_KC | FORM_KC_CF) && canon.is_none() {
        // compatibility mapping: approximated by the full NFKD when it differs.
        let s = c.to_string();
        let k = DecomposingNormalizerBorrowed::new_nfkd().normalize(&s);
        if k != s {
            return Some(k.into_owned());
        }
    }
    canon
}

pub(super) fn fn_get_raw_decomposition(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let f = "normalizer_get_raw_decomposition";
    error_reset();
    let s = str_arg(host, args, 0, f, "string")?;
    let form = int_arg_or(host, args, 1, f, "form", FORM_C)?;
    let one = as_utf8(&s).and_then(|t| {
        let mut it = t.chars();
        match (it.next(), it.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        }
    });
    let Some(c) = one else {
        error_set(
            f,
            U_ILLEGAL_ARGUMENT_ERROR,
            "Input string must be exactly one UTF-8 encoded code point long.",
        );
        return Ok(Value::Null);
    };
    Ok(match raw_decomposition(form, c) {
        Some(d) => Value::str(d.into_bytes()),
        None => Value::Null,
    })
}
