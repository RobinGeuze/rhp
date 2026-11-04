//! PHP-compatible float formatting primitives.
//!
//! PHP formats floats two different ways depending on context:
//!
//! - **echo / print / concat / `(string)` / interpolation** use `precision`
//!   (default **14** significant digits).
//! - **var_dump / var_export / json_encode / serialize** use
//!   `serialize_precision` (default **-1** = shortest round-trip).
//!
//! Both share PHP's `zend_gcvt` reshaping: decide fixed vs. exponential by the
//! decimal exponent, always keep at least one fractional digit in exponential
//! form, uppercase `E`, signed exponent with no leading zeros, and `-0` for
//! negative zero. The only difference is how the significant digits are chosen
//! (round-to-N vs. shortest).
//!
//! These are pure numeric→bytes helpers (no PHP value logic), kept here so both
//! the VM (echo) and `rhp-builtins` (var_dump) can share them.

use std::cell::Cell;

thread_local! {
    /// The `precision` ini setting (echo / string conversion).
    static PRECISION: Cell<i64> = const { Cell::new(14) };
    /// The `serialize_precision` ini setting (var_dump / var_export / json).
    static SERIALIZE_PRECISION: Cell<i64> = const { Cell::new(-1) };
}

/// Set the `precision` ini value (validated by the caller: `>= -1`).
pub fn set_precision(p: i64) {
    PRECISION.with(|c| c.set(p));
}

/// Set the `serialize_precision` ini value (validated by the caller).
pub fn set_serialize_precision(p: i64) {
    SERIALIZE_PRECISION.with(|c| c.set(p));
}

/// Restore both precision settings to php's defaults (14 / -1).
pub fn reset_precision() {
    set_precision(14);
    set_serialize_precision(-1);
}

/// `zend_gcvt(value, precision ? precision : 1, ...)`: `-1` = shortest.
fn mode_for(p: i64) -> Mode {
    match p {
        p if p < 0 => Mode::Shortest,
        0 => Mode::Precision(1),
        // zend_dtoa yields at most ~767 significant digits; cap the request.
        p => Mode::Precision(p.min(500) as usize),
    }
}

/// Format a float the way `echo`/`(string)` does (`precision`, default 14).
pub fn float_echo(f: f64) -> Vec<u8> {
    gcvt(f, mode_for(PRECISION.with(|c| c.get())))
}

/// Format a float the way `var_dump`/`var_export` does (`serialize_precision`,
/// default -1 = shortest round-trip).
pub fn float_serialize(f: f64) -> Vec<u8> {
    gcvt(f, mode_for(SERIALIZE_PRECISION.with(|c| c.get())))
}

enum Mode {
    /// Round to exactly N significant digits (C `%.*G` style).
    Precision(usize),
    /// Shortest string that round-trips (serialize_precision = -1).
    Shortest,
}

/// PHP's `zend_gcvt`-equivalent: produce the significant digits + decimal
/// exponent per `mode`, then reshape into PHP's canonical textual form.
fn gcvt(f: f64, mode: Mode) -> Vec<u8> {
    if f.is_nan() {
        return b"NAN".to_vec();
    }
    if f.is_infinite() {
        return if f < 0.0 {
            b"-INF".to_vec()
        } else {
            b"INF".to_vec()
        };
    }
    let negative = f.is_sign_negative();
    let sign: &[u8] = if negative { b"-" } else { b"" };
    if f == 0.0 {
        // Preserves PHP's `-0` for negative zero.
        return [sign, b"0"].concat();
    }

    // `digits` = significant decimal digits (no point, no sign); `exp10` = the
    // power of ten of the leading digit (so value = 0.d1d2.. is wrong; value =
    // d1.d2d3.. * 10^exp10).
    let abs = f.abs();
    let (digits, exp10) = match mode {
        Mode::Precision(p) => {
            // "{:.*e}" gives p-1 fractional digits => p significant digits,
            // already rounded and re-normalized (e.g. 9.99e0 -> 1.00e1).
            let s = format!("{:.*e}", p - 1, abs);
            split_exp(&s)
        }
        Mode::Shortest => shortest_digits(abs),
    };

    // Threshold for switching to exponential notation, matching zend_gcvt:
    // exponential when exp10 < -4 or exp10 >= ndigit, where ndigit is the
    // precision, or 17 in shortest mode (mode 0).
    let cutoff: i32 = match mode {
        Mode::Precision(p) => p as i32,
        Mode::Shortest => 17,
    };

    let trimmed = trim_trailing_zeros(&digits); // keep >= 1 digit
    let body = if exp10 < -4 || exp10 >= cutoff {
        format_exponential(trimmed, exp10)
    } else {
        format_fixed(trimmed, exp10)
    };
    [sign, &body].concat()
}

/// Shortest round-trip digits of a finite, non-zero `abs` and the power of ten
/// of the leading digit, as zend_dtoa mode 0 produces them. When two shortest
/// candidates are equally close, both pick the even one; std's `{:e}` rounds
/// up instead (php `-887777373534812.2`, std `...812.3`).
fn shortest_digits(abs: f64) -> (Vec<u8>, i32) {
    let mut buf = zmij::Buffer::new();
    // zmij prints "12.34", "0.001234", "12340000000.0", "1.234e+33" or "5e-7".
    let s = buf.format_finite(abs).as_bytes();
    let (mantissa, exp) = match s.iter().position(|&b| b == b'e') {
        Some(e) => {
            let exp = std::str::from_utf8(&s[e + 1..])
                .ok()
                .and_then(|x| x.parse::<i32>().ok())
                .expect("zmij exponent");
            (&s[..e], exp)
        }
        None => (s, 0),
    };
    let point = mantissa
        .iter()
        .position(|&b| b == b'.')
        .unwrap_or(mantissa.len());
    let mut digits: Vec<u8> = mantissa.iter().copied().filter(|&b| b != b'.').collect();
    let lead = digits
        .iter()
        .position(|&b| b != b'0')
        .expect("non-zero value");
    digits.drain(..lead);
    (digits, point as i32 - 1 - lead as i32 + exp)
}

/// Parse a Rust `{:.*e}` string "d.dddde±XX" into (digit_string, exp10).
/// The returned digit string has the decimal point removed.
fn split_exp(s: &str) -> (Vec<u8>, i32) {
    let bytes = s.as_bytes();
    let e_pos = bytes
        .iter()
        .position(|&b| b == b'e')
        .expect("exp form has 'e'");
    let mantissa = &s[..e_pos];
    let exp: i32 = s[e_pos + 1..].parse().expect("valid exponent");
    let mut digits = Vec::with_capacity(mantissa.len());
    for &b in mantissa.as_bytes() {
        if b != b'.' {
            digits.push(b);
        }
    }
    (digits, exp)
}

fn trim_trailing_zeros(digits: &[u8]) -> &[u8] {
    let mut end = digits.len();
    while end > 1 && digits[end - 1] == b'0' {
        end -= 1;
    }
    &digits[..end]
}

/// Exponential form: `d.ddddE±XX`, always at least one fractional digit.
fn format_exponential(digits: &[u8], exp10: i32) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(digits[0]);
    out.push(b'.');
    if digits.len() > 1 {
        out.extend_from_slice(&digits[1..]);
    } else {
        out.push(b'0');
    }
    out.push(b'E');
    out.push(if exp10 < 0 { b'-' } else { b'+' });
    out.extend_from_slice(exp10.unsigned_abs().to_string().as_bytes());
    out
}

/// Fixed form derived from significant digits and the exponent.
fn format_fixed(digits: &[u8], exp10: i32) -> Vec<u8> {
    let mut out = Vec::new();
    if exp10 >= 0 {
        let int_len = (exp10 as usize) + 1;
        if digits.len() <= int_len {
            // All digits are integer part; pad with zeros.
            out.extend_from_slice(digits);
            out.extend(std::iter::repeat_n(b'0', int_len - digits.len()));
        } else {
            out.extend_from_slice(&digits[..int_len]);
            out.push(b'.');
            out.extend_from_slice(&digits[int_len..]);
        }
    } else {
        // 0.00..digits
        out.extend_from_slice(b"0.");
        out.extend(std::iter::repeat_n(b'0', (-exp10 - 1) as usize));
        out.extend_from_slice(digits);
    }
    out
}

#[cfg(test)]
mod tests;
