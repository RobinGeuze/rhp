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
//! the VM, the oracle (echo), and `rhp-builtins` (var_dump) can share them.

/// Format a float the way `echo`/`(string)` does: `precision = 14`.
pub fn float_echo(f: f64) -> Vec<u8> {
    gcvt(f, Mode::Precision(14))
}

/// Format a float the way `var_dump`/`var_export` does: shortest round-trip.
pub fn float_serialize(f: f64) -> Vec<u8> {
    gcvt(f, Mode::Shortest)
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
        Mode::Shortest => {
            // "{:e}" gives the shortest round-trip mantissa in exp form.
            let s = format!("{:e}", abs);
            split_exp(&s)
        }
    };

    // Threshold for switching to exponential notation, matching PHP: use
    // exponential when exp10 < -4 or exp10 >= decimal_point_cutoff. For
    // precision mode the cutoff is the precision (14); for shortest PHP uses 15.
    let cutoff: i32 = match mode {
        Mode::Precision(p) => p as i32,
        Mode::Shortest => 15,
    };

    let trimmed = trim_trailing_zeros(&digits); // keep >= 1 digit
    let body = if exp10 < -4 || exp10 >= cutoff {
        format_exponential(trimmed, exp10)
    } else {
        format_fixed(trimmed, exp10)
    };
    [sign, &body].concat()
}

/// Parse a Rust `{:e}` / `{:.*e}` string "d.dddde±XX" into (digit_string, exp10).
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
mod tests {
    use super::*;

    fn echo(f: f64) -> String {
        String::from_utf8(float_echo(f)).unwrap()
    }
    fn ser(f: f64) -> String {
        String::from_utf8(float_serialize(f)).unwrap()
    }

    #[test]
    fn echo_forms_match_php_8_5() {
        // ground truth from tests/golden/echo_float_formats.out
        assert_eq!(echo(1.0), "1");
        assert_eq!(echo(0.1 + 0.2), "0.3");
        assert_eq!(echo(1.0 / 3.0), "0.33333333333333");
        assert_eq!(echo(1e20), "1.0E+20");
        assert_eq!(echo(1.5e-10), "1.5E-10");
        assert_eq!(echo(-0.0), "-0");
        assert_eq!(echo(0.00001), "1.0E-5");
    }

    #[test]
    fn echo_specials() {
        assert_eq!(echo(f64::NAN), "NAN");
        assert_eq!(echo(f64::INFINITY), "INF");
        assert_eq!(echo(f64::NEG_INFINITY), "-INF");
        assert_eq!(echo(0.0), "0");
        assert_eq!(echo(-42.5), "-42.5");
        assert_eq!(echo(100.0), "100");
    }

    #[test]
    fn serialize_forms_match_php_8_5() {
        // php -r 'var_dump(0.1+0.2);' => float(0.30000000000000004)
        assert_eq!(ser(0.1 + 0.2), "0.30000000000000004");
        // php -r 'var_dump(1/3);' => float(0.3333333333333333)
        assert_eq!(ser(1.0 / 3.0), "0.3333333333333333");
        assert_eq!(ser(1e20), "1.0E+20");
        assert_eq!(ser(1.0), "1");
        assert_eq!(ser(-0.0), "-0");
    }
}
