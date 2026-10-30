//! String-parsing builtins: `str_getcsv` and `sscanf`.
//!
//! Behaviour verified byte-for-byte against the local php 8.5.8 binary (see the
//! `#[cfg(test)]` block — expected values are hardcoded from `php -r`).
//!
//! ## Scope
//! - `sscanf` supports the common conversions `%d %i %u %f %e %g %s %c %x %o`
//!   plus field widths, assignment suppression (`%*d`), and `%%`. Unsupported
//!   conversions (`%n`, `%[...]` scansets) are documented and treated as a no-op
//!   match failure. The assignment form (`sscanf($s, $fmt, &$a, ...)`) writes
//!   through the by-ref argument positions registered in `builtin_ref_params`.

use rhp_values::{conv, PhpArray, Value};

use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn first_byte(v: Option<&Value>, default: u8) -> u8 {
    match v {
        None => default,
        Some(v) => conv::to_string_bytes(&deref(v))
            .first()
            .copied()
            .unwrap_or(default),
    }
}

// ---------------------------------------------------------------------------
// str_getcsv
// ---------------------------------------------------------------------------

/// `str_getcsv($string, $separator = ",", $enclosure = '"', $escape = "\\")`.
/// Parses a single CSV record into an array of fields. An empty input yields
/// `[null]`, matching php.
fn fn_str_getcsv(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&deref(&args[0]));
    let sep = first_byte(args.get(1), b',');
    let enc = first_byte(args.get(2), b'"');
    // The escape may be disabled by passing "" in php; default is backslash.
    let esc: Option<u8> = match args.get(3) {
        Some(v) => conv::to_string_bytes(&deref(v)).first().copied(),
        None => Some(b'\\'),
    };

    let mut out = PhpArray::new();
    if s.is_empty() {
        out.push(Value::Null);
        return Ok(Value::Array(out));
    }

    let bytes = &s;
    let mut i = 0usize;
    loop {
        let mut field = Vec::new();
        if bytes.get(i) == Some(&enc) {
            // Quoted field.
            i += 1;
            loop {
                match bytes.get(i) {
                    None => break,
                    Some(&c) if Some(c) == esc && i + 1 < bytes.len() => {
                        // php keeps BOTH the escape char and the escaped char.
                        field.push(c);
                        field.push(bytes[i + 1]);
                        i += 2;
                    }
                    Some(&c) if c == enc => {
                        if bytes.get(i + 1) == Some(&enc) {
                            // Doubled enclosure -> one literal enclosure.
                            field.push(enc);
                            i += 2;
                        } else {
                            i += 1; // closing enclosure
                            break;
                        }
                    }
                    Some(&c) => {
                        field.push(c);
                        i += 1;
                    }
                }
            }
            // Trailing content after the closing quote, up to the separator, is
            // appended to the field (php behaviour).
            while let Some(&c) = bytes.get(i) {
                if c == sep {
                    break;
                }
                field.push(c);
                i += 1;
            }
        } else {
            // Unquoted field: read until the separator.
            while let Some(&c) = bytes.get(i) {
                if c == sep {
                    break;
                }
                field.push(c);
                i += 1;
            }
        }
        out.push(Value::str(field));
        match bytes.get(i) {
            None => break,
            Some(&c) if c == sep => {
                i += 1;
                if i == bytes.len() {
                    // Trailing separator -> one final empty field.
                    out.push(Value::str(Vec::new()));
                    break;
                }
            }
            _ => break,
        }
    }
    Ok(Value::Array(out))
}

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

pub fn register(m: &mut std::collections::HashMap<&'static str, Builtin>) {
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
    reg!("str_getcsv", fn_str_getcsv, 1, Some(4));
    reg!("sscanf", fn_sscanf, 2, None);
}

// ---------------------------------------------------------------------------
// tests — expected values captured from php 8.5.8
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopHost;
    impl Host for NoopHost {
        fn write_out(&mut self, _: &[u8]) {}
    }

    fn s(b: &str) -> Value {
        Value::str(b)
    }

    fn csv(args: &[Value]) -> Vec<Option<String>> {
        let r = fn_str_getcsv(&mut NoopHost, args).unwrap();
        match r {
            Value::Array(a) => a
                .iter()
                .map(|(_, v)| match v {
                    Value::Null => None,
                    Value::Str(s) => Some(String::from_utf8(s.as_bytes().to_vec()).unwrap()),
                    _ => unreachable!(),
                })
                .collect(),
            _ => unreachable!(),
        }
    }

    fn some(items: &[&str]) -> Vec<Option<String>> {
        items.iter().map(|x| Some(x.to_string())).collect()
    }

    #[test]
    fn csv_basic() {
        assert_eq!(csv(&[s("a,b,c")]), some(&["a", "b", "c"]));
        assert_eq!(csv(&[s("a,\"b,c\",d")]), some(&["a", "b,c", "d"]));
        assert_eq!(
            csv(&[s("\"he said \"\"hi\"\"\",x")]),
            some(&["he said \"hi\"", "x"])
        );
        assert_eq!(csv(&[s("a;b;c"), s(";")]), some(&["a", "b", "c"]));
        assert_eq!(csv(&[s("a,b,,")]), some(&["a", "b", "", ""]));
        assert_eq!(csv(&[s("")]), vec![None]);
        assert_eq!(csv(&[s("  a , b ")]), some(&["  a ", " b "]));
        assert_eq!(csv(&[s("a,\"b\nc\",d")]), some(&["a", "b\nc", "d"]));
    }

    #[test]
    fn csv_escape() {
        // escape char keeps both chars; close-quote handling.
        assert_eq!(
            csv(&[s("x,\"y\\\"z\""), s(","), s("\""), s("\\")]),
            some(&["x", "y\\\"z"])
        );
        assert_eq!(
            csv(&[s("a\"b,c"), s(","), s("\""), s("\\")]),
            some(&["a\"b", "c"])
        );
        assert_eq!(
            csv(&[s("\"a\"b,c"), s(","), s("\""), s("\\")]),
            some(&["ab", "c"])
        );
        assert_eq!(
            csv(&[s("a\\,b,c"), s(","), s("\""), s("\\")]),
            some(&["a\\", "b", "c"])
        );
    }

    /// Canonical string form of a value, for structural comparison (Value has no
    /// `PartialEq`). Floats use a short rounded repr adequate for these tests.
    fn vrepr(v: &Value) -> String {
        match v {
            Value::Null => "null".to_string(),
            Value::Int(n) => format!("i:{n}"),
            Value::Float(f) => format!("f:{:.6}", f),
            Value::Str(s) => format!("s:{}", String::from_utf8_lossy(s.as_bytes())),
            Value::Bool(b) => format!("b:{b}"),
            _ => "?".to_string(),
        }
    }

    fn scan_arr(input: &str, fmt: &str) -> Option<Vec<String>> {
        match fn_sscanf(&mut NoopHost, &[s(input), s(fmt)]).unwrap() {
            Value::Array(a) => Some(a.iter().map(|(_, v)| vrepr(v)).collect()),
            Value::Null => None,
            _ => unreachable!(),
        }
    }

    fn v(items: &[&str]) -> Option<Vec<String>> {
        Some(items.iter().map(|x| x.to_string()).collect())
    }

    #[test]
    fn sscanf_basic() {
        assert_eq!(scan_arr("age:25", "age:%d"), v(&["i:25"]));
        assert_eq!(scan_arr("3.14 x", "%f %s"), v(&["f:3.140000", "s:x"]));
        assert_eq!(scan_arr("hello world", "%s %s"), v(&["s:hello", "s:world"]));
        assert_eq!(scan_arr("12ab", "%d%s"), v(&["i:12", "s:ab"]));
        assert_eq!(scan_arr("  42", "%d"), v(&["i:42"]));
        assert_eq!(scan_arr("255", "%3d"), v(&["i:255"]));
        assert_eq!(scan_arr("abcdef", "%3s"), v(&["s:abc"]));
        assert_eq!(scan_arr("hello", "%2c"), v(&["s:he"]));
        assert_eq!(scan_arr("ff", "%x"), v(&["i:255"]));
        assert_eq!(scan_arr("-17", "%d"), v(&["i:-17"]));
    }

    #[test]
    fn sscanf_failures() {
        // whole-NULL when EOF before any match.
        assert_eq!(scan_arr("", "%d %d"), None);
        // partial -> trailing NULLs.
        assert_eq!(scan_arr("5", "%d %d %d"), v(&["i:5", "null", "null"]));
        // mismatch on non-empty input -> array with NULL.
        assert_eq!(scan_arr("hello", "%d"), v(&["null"]));
        // no conversions -> empty array.
        assert_eq!(scan_arr("%", "%%"), v(&[]));
    }

    #[test]
    fn sscanf_assign_form() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let a = Rc::new(RefCell::new(Value::Null));
        let b = Rc::new(RefCell::new(Value::Null));
        let n = fn_sscanf(
            &mut NoopHost,
            &[
                s("42 foo"),
                s("%d %s"),
                Value::Ref(a.clone()),
                Value::Ref(b.clone()),
            ],
        )
        .unwrap();
        assert!(matches!(n, Value::Int(2)));
        assert!(matches!(&*a.borrow(), Value::Int(42)));
        assert!(matches!(&*b.borrow(), Value::Str(s) if s.as_bytes() == b"foo"));

        // count 1 when only the first matches.
        let a2 = Rc::new(RefCell::new(Value::Null));
        let b2 = Rc::new(RefCell::new(Value::Null));
        let n2 = fn_sscanf(
            &mut NoopHost,
            &[s("5"), s("%d %d"), Value::Ref(a2.clone()), Value::Ref(b2)],
        )
        .unwrap();
        assert!(matches!(n2, Value::Int(1)));

        // 0 matched -> NULL.
        let a3 = Rc::new(RefCell::new(Value::Null));
        let n3 = fn_sscanf(&mut NoopHost, &[s("nomatch"), s("%d"), Value::Ref(a3)]).unwrap();
        assert!(matches!(n3, Value::Null));
    }
}
