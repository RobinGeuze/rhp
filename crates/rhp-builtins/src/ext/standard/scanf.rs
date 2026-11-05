//! ext/standard/scanf.c: `sscanf` (its `PHP_FUNCTION` is string.c's, a
//! wrapper around scanf.c's `php_sscanf_internal`).
//!
//! Behaviour verified byte-for-byte against the local php 8.5.8 binary (see
//! `scanf/tests.rs` — expected values are hardcoded from `php -r`).
//!
//! ## Scope
//! - `sscanf` supports the common conversions `%d %i %u %f %e %g %s %c %x %o`
//!   plus field widths, assignment suppression (`%*d`), and `%%`. Unsupported
//!   conversions (`%n`, `%[...]` scansets) are documented and treated as a no-op
//!   match failure. The assignment form (`sscanf($s, $fmt, &$a, ...)`) writes
//!   through the by-ref positions of its stub signature (`mixed &...$vars`).

use rhp_values::ops::deref;
use rhp_values::{conv, PhpArray, Value};

use crate::{Host, Thrown};

// ---------------------------------------------------------------------------
// sscanf
// ---------------------------------------------------------------------------

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// One parsed conversion result: a value, or `None` for a failed/skipped slot.
enum Field {
    Value(Value),
    Null,
}

/// Outcome of the scan: either normal, or EOF reached before any field matched
/// (php returns NULL for the whole call in that case).
struct ScanResult {
    fields: Vec<Field>,
    matched: usize,
    eof_before_match: bool,
}

fn fn_sscanf(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let _ = host;
    let input = conv::to_string_bytes(&deref(&args[0]));
    let format = conv::to_string_bytes(&deref(&args[1]));
    let scan = run_sscanf(&input, &format);

    if args.len() <= 2 {
        // Array-return form.
        if scan.eof_before_match {
            return Ok(Value::Null);
        }
        let mut out = PhpArray::new();
        for f in scan.fields {
            out.push(match f {
                Field::Value(v) => v,
                Field::Null => Value::Null,
            });
        }
        Ok(Value::Array(out))
    } else {
        // Assignment form: write matched fields through the by-ref args, return
        // the number of matched conversions (or NULL when none matched).
        if scan.matched == 0 {
            return Ok(Value::Null);
        }
        let mut arg_pos = 2;
        for f in &scan.fields {
            if arg_pos >= args.len() {
                break;
            }
            if let Field::Value(v) = f {
                if let Value::Ref(r) = &args[arg_pos] {
                    *r.borrow_mut() = v.clone();
                }
                arg_pos += 1;
            }
        }
        Ok(Value::Int(scan.matched as i64))
    }
}

fn run_sscanf(input: &[u8], format: &[u8]) -> ScanResult {
    let mut fields: Vec<Field> = Vec::new();
    let mut matched = 0usize;
    let mut eof_before_match = false;
    let mut ip = 0usize; // input position
    let mut fp = 0usize; // format position

    while fp < format.len() {
        let fc = format[fp];
        if is_ws(fc) {
            // Whitespace in the format matches zero or more input whitespace.
            fp += 1;
            while ip < input.len() && is_ws(input[ip]) {
                ip += 1;
            }
            continue;
        }
        if fc != b'%' {
            // Literal: must match the next input byte exactly.
            if ip < input.len() && input[ip] == fc {
                ip += 1;
                fp += 1;
            } else {
                // Literal mismatch ends the scan.
                break;
            }
            continue;
        }
        // Conversion specifier: '%' [*] [width] conv
        fp += 1;
        if fp >= format.len() {
            break;
        }
        if format[fp] == b'%' {
            // Literal percent.
            if ip < input.len() && input[ip] == b'%' {
                ip += 1;
                fp += 1;
            } else {
                break;
            }
            continue;
        }
        let suppress = format[fp] == b'*';
        if suppress {
            fp += 1;
        }
        // Width.
        let mut width = 0usize;
        let mut has_width = false;
        while fp < format.len() && format[fp].is_ascii_digit() {
            has_width = true;
            width = width * 10 + (format[fp] - b'0') as usize;
            fp += 1;
        }
        let width = if has_width { Some(width) } else { None };
        if fp >= format.len() {
            break;
        }
        let conv_ch = format[fp];
        fp += 1;

        // Most conversions skip leading whitespace first; %c does not.
        if conv_ch != b'c' {
            while ip < input.len() && is_ws(input[ip]) {
                ip += 1;
            }
        }
        if ip >= input.len() {
            // Ran out of input for a value conversion.
            if matched == 0 {
                eof_before_match = true;
                return ScanResult {
                    fields,
                    matched,
                    eof_before_match,
                };
            }
            if !suppress {
                fields.push(Field::Null);
            }
            continue;
        }

        let parsed = match conv_ch {
            b'd' | b'i' | b'u' => scan_int(input, &mut ip, width, 10),
            b'x' | b'X' => scan_int(input, &mut ip, width, 16),
            b'o' => scan_int(input, &mut ip, width, 8),
            b'f' | b'e' | b'E' | b'g' | b'G' => scan_float(input, &mut ip, width),
            b's' => scan_str(input, &mut ip, width),
            b'c' => scan_char(input, &mut ip, width),
            // Unsupported conversion (e.g. %n, %[...] scanset): treat as a
            // non-matching slot.
            _ => None,
        };
        match parsed {
            Some(v) => {
                matched += 1;
                if !suppress {
                    fields.push(Field::Value(v));
                }
            }
            None => {
                if !suppress {
                    fields.push(Field::Null);
                }
            }
        }
    }

    ScanResult {
        fields,
        matched,
        eof_before_match,
    }
}

/// Scan an integer in `base` (optional sign, base digits), honouring `width`.
fn scan_int(input: &[u8], ip: &mut usize, width: Option<usize>, base: u32) -> Option<Value> {
    let start = *ip;
    let limit = width
        .map(|w| (start + w).min(input.len()))
        .unwrap_or(input.len());
    let mut j = start;
    let mut neg = false;
    if j < limit && (input[j] == b'+' || input[j] == b'-') {
        neg = input[j] == b'-';
        j += 1;
    }
    let digits_start = j;
    let mut acc: i64 = 0;
    while j < limit {
        let d = match input[j] {
            b'0'..=b'9' => (input[j] - b'0') as u32,
            b'a'..=b'f' => (input[j] - b'a' + 10) as u32,
            b'A'..=b'F' => (input[j] - b'A' + 10) as u32,
            _ => break,
        };
        if d >= base {
            break;
        }
        acc = acc.saturating_mul(base as i64).saturating_add(d as i64);
        j += 1;
    }
    if j == digits_start {
        return None; // no digits
    }
    *ip = j;
    Some(Value::Int(if neg { -acc } else { acc }))
}

/// Scan a float (sign, digits, optional fraction and exponent), honouring `width`.
fn scan_float(input: &[u8], ip: &mut usize, width: Option<usize>) -> Option<Value> {
    let start = *ip;
    let limit = width
        .map(|w| (start + w).min(input.len()))
        .unwrap_or(input.len());
    let mut j = start;
    if j < limit && (input[j] == b'+' || input[j] == b'-') {
        j += 1;
    }
    let mut saw_digit = false;
    while j < limit && input[j].is_ascii_digit() {
        j += 1;
        saw_digit = true;
    }
    if j < limit && input[j] == b'.' {
        j += 1;
        while j < limit && input[j].is_ascii_digit() {
            j += 1;
            saw_digit = true;
        }
    }
    if !saw_digit {
        return None;
    }
    if j < limit && (input[j] == b'e' || input[j] == b'E') {
        let mut k = j + 1;
        if k < limit && (input[k] == b'+' || input[k] == b'-') {
            k += 1;
        }
        let exp_start = k;
        while k < limit && input[k].is_ascii_digit() {
            k += 1;
        }
        if k > exp_start {
            j = k; // only consume the exponent if it had digits
        }
    }
    let text = std::str::from_utf8(&input[start..j]).ok()?;
    let f = text.parse::<f64>().ok()?;
    *ip = j;
    Some(Value::Float(f))
}

/// Scan a run of non-whitespace bytes (`%s`), honouring `width`.
fn scan_str(input: &[u8], ip: &mut usize, width: Option<usize>) -> Option<Value> {
    let start = *ip;
    let limit = width
        .map(|w| (start + w).min(input.len()))
        .unwrap_or(input.len());
    let mut j = start;
    while j < limit && !is_ws(input[j]) {
        j += 1;
    }
    if j == start {
        return None;
    }
    *ip = j;
    Some(Value::str(input[start..j].to_vec()))
}

/// Scan exactly `width` (default 1) bytes (`%c`), including whitespace.
fn scan_char(input: &[u8], ip: &mut usize, width: Option<usize>) -> Option<Value> {
    let start = *ip;
    let want = width.unwrap_or(1);
    let end = (start + want).min(input.len());
    if end == start {
        return None;
    }
    *ip = end;
    Some(Value::str(input[start..end].to_vec()))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("sscanf", fn_sscanf)]);
}

#[cfg(test)]
mod tests;
