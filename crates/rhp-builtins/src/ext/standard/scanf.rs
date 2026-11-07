//! ext/standard/scanf.c: the scan engine behind `sscanf` (string.c) and
//! `fscanf` (file.c). A port of `ValidateFormat`, `BuildCharSet`,
//! `CharInSet` and `php_sscanf_internal`, kept close to the C so the odd
//! corners (`%n$` positions, scansets, the 63-byte number buffer, `%u`
//! turning negative values into strings) behave the same.
//!
//! The engine is Tcl 8.3.0 code (THIRD-PARTY-NOTICES, section 26).
//!
//! The C code walks NUL-terminated strings, so input and format end at
//! their first NUL byte here too.

use rhp_values::ops::deref;
use rhp_values::{conv, PhpArray, Value};

use crate::{Host, Thrown};

const SCAN_NOSKIP: u32 = 0x1;
const SCAN_SUPPRESS: u32 = 0x2;
const SCAN_UNSIGNED: u32 = 0x4;
const SCAN_WIDTH: u32 = 0x8;
const SCAN_SIGNOK: u32 = 0x10;
const SCAN_NODIGITS: u32 = 0x20;
const SCAN_NOZERO: u32 = 0x40;
const SCAN_XOK: u32 = 0x80;
const SCAN_PTOK: u32 = 0x100;
const SCAN_EXPOK: u32 = 0x200;

const SCAN_MAX_ARGS: i32 = 0xFF;
/// `SCAN_ERROR_EOF`: the assignment form's result when nothing converted.
const SCAN_ERROR_EOF: i64 = -1;

/// C `isspace` in the "C" locale.
fn isspace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// A NUL-terminated view: reads past the end (or at an embedded NUL) give 0.
struct CStr<'a>(&'a [u8]);

impl CStr<'_> {
    fn new(s: &[u8]) -> CStr<'_> {
        CStr(&s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())])
    }
    fn at(&self, i: usize) -> u8 {
        self.0.get(i).copied().unwrap_or(0)
    }
}

/// `ZEND_STRTOUL(s, &end, 10)` over the digits at `i`: the value (saturated
/// like strtoul) and the index after the digits.
fn strtoul10(s: &CStr, mut i: usize) -> (u64, usize) {
    let mut v: u64 = 0;
    let mut over = false;
    while s.at(i).is_ascii_digit() {
        match v
            .checked_mul(10)
            .and_then(|x| x.checked_add(u64::from(s.at(i) - b'0')))
        {
            Some(x) => v = x,
            None => over = true,
        }
        i += 1;
    }
    (if over { u64::MAX } else { v }, i)
}

/// C `strtoul`/`strtol` over a buffer the scanner built (sign, optional
/// `0x`, digits): the magnitude, sign and whether it overflowed u64.
fn c_strto(buf: &[u8], mut base: u32) -> (u64, bool, bool) {
    let mut i = 0;
    let mut neg = false;
    if let Some(&c @ (b'+' | b'-')) = buf.first() {
        neg = c == b'-';
        i = 1;
    }
    let has_x = buf.get(i) == Some(&b'0')
        && matches!(buf.get(i + 1), Some(b'x' | b'X'))
        && buf.get(i + 2).is_some_and(|c| c.is_ascii_hexdigit());
    if (base == 0 || base == 16) && has_x {
        base = 16;
        i += 2;
    } else if base == 0 {
        base = if buf.get(i) == Some(&b'0') { 8 } else { 10 };
    }
    let mut v: u64 = 0;
    let mut over = false;
    while let Some(d) = buf.get(i).and_then(|c| (*c as char).to_digit(base)) {
        match v
            .checked_mul(u64::from(base))
            .and_then(|x| x.checked_add(u64::from(d)))
        {
            Some(x) => v = x,
            None => over = true,
        }
        i += 1;
    }
    (v, neg, over)
}

fn c_strtol(buf: &[u8], base: u32) -> i64 {
    let (v, neg, over) = c_strto(buf, base);
    if neg {
        if over || v > i64::MAX as u64 + 1 {
            i64::MIN
        } else {
            (v as i64).wrapping_neg()
        }
    } else if over || v > i64::MAX as u64 {
        i64::MAX
    } else {
        v as i64
    }
}

fn c_strtoul(buf: &[u8], base: u32) -> u64 {
    let (v, neg, over) = c_strto(buf, base);
    if over {
        u64::MAX
    } else if neg {
        v.wrapping_neg()
    } else {
        v
    }
}

/// A scanset (`%[...]`).
struct CharSet {
    exclude: bool,
    chars: Vec<u8>,
    ranges: Vec<(u8, u8)>,
}

impl CharSet {
    /// `BuildCharSet`: parse the set starting at `format` (just after the
    /// `[`); returns the set and the format index after the closing `]`.
    fn build(f: &CStr, mut format: usize) -> (CharSet, usize) {
        let mut cset = CharSet {
            exclude: false,
            chars: Vec::new(),
            ranges: Vec::new(),
        };
        if f.at(format) == b'^' {
            cset.exclude = true;
            format += 1;
        }
        let mut ch = format;
        format += 1;
        let mut start = f.at(ch);
        if f.at(ch) == b']' || f.at(ch) == b'-' {
            cset.chars.push(f.at(ch));
            ch = format;
            format += 1;
        }
        while f.at(ch) != b']' {
            if f.at(format) == b'-' {
                start = f.at(ch);
            } else if f.at(ch) == b'-' {
                if f.at(format) == b']' {
                    cset.chars.push(start);
                    cset.chars.push(f.at(ch));
                } else {
                    ch = format;
                    format += 1;
                    // C compares plain (signed) chars.
                    let (a, b) = (start as i8, f.at(ch) as i8);
                    if a < b {
                        cset.ranges.push((start, f.at(ch)));
                    } else {
                        cset.ranges.push((f.at(ch), start));
                    }
                }
            } else {
                cset.chars.push(f.at(ch));
            }
            ch = format;
            format += 1;
        }
        (cset, format)
    }

    fn contains(&self, c: u8) -> bool {
        let m = self.chars.contains(&c)
            || self
                .ranges
                .iter()
                .any(|&(s, e)| (s as i8) <= (c as i8) && (c as i8) <= (e as i8));
        m != self.exclude
    }
}

fn value_error(msg: impl Into<String>) -> Thrown {
    Thrown::error("ValueError", msg)
}

/// `ValidateFormat`: check the format and count the result slots
/// (`totalSubs`).
fn validate_format(f: &CStr, num_vars: usize) -> Result<usize, Thrown> {
    let num_vars_i = num_vars as i64;
    let mut nassign: Vec<u32> = vec![0; num_vars.max(16)];
    let (mut xpg_size, mut obj_index): (i64, i64) = (0, 0);
    let (mut got_xpg, mut got_sequential) = (false, false);
    let mut format = 0usize;
    let bad_index = |got_xpg: bool| {
        if got_xpg {
            value_error("\"%n$\" argument index out of range")
        } else {
            value_error("Different numbers of variable names and field specifiers")
        }
    };
    while f.at(format) != 0 {
        let mut ch = format;
        format += 1;
        let mut flags = 0;
        if f.at(ch) != b'%' {
            continue;
        }
        ch = format;
        format += 1;
        if f.at(ch) == b'%' {
            continue;
        }
        let mut xpg_done = false;
        if f.at(ch) == b'*' {
            flags |= SCAN_SUPPRESS;
            ch = format;
            format += 1;
            xpg_done = true;
        } else if f.at(ch).is_ascii_digit() {
            let (value, end) = strtoul10(f, ch);
            if f.at(end) == b'$' {
                let value = value as i32 as i64;
                format = end + 1;
                ch = format;
                format += 1;
                got_xpg = true;
                if got_sequential {
                    return Err(value_error(
                        "cannot mix \"%\" and \"%n$\" conversion specifiers",
                    ));
                }
                if value < 1 || (num_vars != 0 && value > num_vars_i) {
                    return Err(bad_index(got_xpg));
                } else if num_vars == 0 {
                    if value > i64::from(SCAN_MAX_ARGS) {
                        return Err(bad_index(got_xpg));
                    }
                    xpg_size = xpg_size.max(value);
                }
                obj_index = value - 1;
                xpg_done = true;
            }
        }
        if !xpg_done {
            got_sequential = true;
            if got_xpg {
                return Err(value_error(
                    "cannot mix \"%\" and \"%n$\" conversion specifiers",
                ));
            }
        }
        // width
        if f.at(ch).is_ascii_digit() {
            let (_, end) = strtoul10(f, ch);
            format = end;
            flags |= SCAN_WIDTH;
            ch = format;
            format += 1;
        }
        // size modifier (ignored)
        if matches!(f.at(ch), b'l' | b'L' | b'h') {
            ch = format;
            format += 1;
        }
        if flags & SCAN_SUPPRESS == 0 && num_vars != 0 && obj_index >= num_vars_i {
            return Err(bad_index(got_xpg));
        }
        match f.at(ch) {
            b'n' | b'd' | b'D' | b'i' | b'o' | b'x' | b'X' | b'u' | b'f' | b'e' | b'E' | b'g'
            | b's' | b'c' => {}
            b'[' => {
                let bad_set = || value_error("Unmatched [ in format string");
                if f.at(format) == 0 {
                    return Err(bad_set());
                }
                ch = format;
                format += 1;
                if f.at(ch) == b'^' {
                    if f.at(format) == 0 {
                        return Err(bad_set());
                    }
                    ch = format;
                    format += 1;
                }
                if f.at(ch) == b']' {
                    if f.at(format) == 0 {
                        return Err(bad_set());
                    }
                    ch = format;
                    format += 1;
                }
                while f.at(ch) != b']' {
                    if f.at(format) == 0 {
                        return Err(bad_set());
                    }
                    ch = format;
                    format += 1;
                }
            }
            c => {
                // The message is a C string: a NUL ends it.
                let mut msg = b"Bad scan conversion character \"".to_vec();
                if c != 0 {
                    msg.push(c);
                    msg.push(b'"');
                }
                return Err(value_error(String::from_utf8_lossy(&msg).into_owned()));
            }
        }
        if flags & SCAN_SUPPRESS == 0 {
            let idx = obj_index as usize;
            if idx >= nassign.len() {
                let grow = if xpg_size != 0 {
                    xpg_size as usize
                } else {
                    nassign.len() + 16
                };
                nassign.resize(grow.max(idx + 1), 0);
            }
            nassign[idx] += 1;
            obj_index += 1;
        }
    }
    let total = if num_vars == 0 {
        if xpg_size != 0 {
            xpg_size as usize
        } else {
            obj_index as usize
        }
    } else {
        num_vars
    };
    for i in 0..total {
        let n = nassign.get(i).copied().unwrap_or(0);
        if n > 1 {
            return Err(value_error(
                "Variable is assigned by multiple \"%n$\" conversion specifiers",
            ));
        } else if xpg_size == 0 && n == 0 {
            return Err(value_error(
                "Variable is not assigned by any conversion specifiers",
            ));
        }
    }
    Ok(total)
}

/// Where converted values go: the by-reference variables, or the result
/// array (pre-filled with NULLs).
enum Sink<'a> {
    Vars(&'a [Value]),
    Array(PhpArray),
}

impl Sink<'_> {
    /// Store `v` at slot `idx`; false when the variables ran out (the C
    /// `objIndex >= argCount` check, which skips the store).
    fn put(&mut self, idx: &mut usize, v: Value) -> bool {
        match self {
            Sink::Vars(vars) => {
                let Some(slot) = vars.get(*idx) else {
                    return false;
                };
                if let Value::Ref(r) = slot {
                    *r.borrow_mut() = v;
                }
            }
            Sink::Array(a) => {
                a.insert(rhp_values::ArrayKey::Int(*idx as i64), v);
            }
        }
        *idx += 1;
        true
    }
}

/// `php_sscanf_internal`: scan `string` with `format`, assigning to `vars`
/// (by-reference variables) or returning the array of values.
pub(crate) fn php_sscanf_internal(
    string: &[u8],
    format: &[u8],
    vars: &[Value],
) -> Result<Value, Thrown> {
    let s = CStr::new(string);
    let f = CStr::new(format);
    let num_vars = vars.len();
    let total_vars = validate_format(&f, num_vars)?;
    let mut sink = if num_vars != 0 {
        Sink::Vars(vars)
    } else {
        let mut a = PhpArray::new();
        for _ in 0..total_vars {
            a.push(Value::Null);
        }
        Sink::Array(a)
    };
    let mut obj_index = 0usize;
    let mut sp = 0usize; // string position
    let mut fp = 0usize; // format position
    let mut nconversions = 0usize;
    let mut underflow = false;
    let mut base: u32 = 0;
    let mut buf: Vec<u8> = Vec::with_capacity(64);

    'scan: while f.at(fp) != 0 {
        let mut ch = fp;
        fp += 1;
        let mut flags = 0u32;

        if isspace(f.at(ch)) {
            while isspace(s.at(sp)) {
                sp += 1;
            }
            continue;
        }

        let literal = if f.at(ch) != b'%' {
            true
        } else {
            ch = fp;
            fp += 1;
            f.at(ch) == b'%'
        };
        if literal {
            if s.at(sp) == 0 {
                underflow = true;
                break 'scan;
            }
            let sch = s.at(sp);
            sp += 1;
            if f.at(ch) != sch {
                break 'scan;
            }
            continue;
        }

        if f.at(ch) == b'*' {
            flags |= SCAN_SUPPRESS;
            ch = fp;
            fp += 1;
        } else if f.at(ch).is_ascii_digit() {
            let (value, end) = strtoul10(&f, ch);
            if f.at(end) == b'$' {
                fp = end + 1;
                ch = fp;
                fp += 1;
                obj_index = (value as i32 as i64 - 1) as usize;
            }
        }

        let mut width: usize = if f.at(ch).is_ascii_digit() {
            let (w, end) = strtoul10(&f, ch);
            fp = end;
            ch = fp;
            fp += 1;
            w as usize
        } else {
            0
        };

        if matches!(f.at(ch), b'l' | b'L' | b'h') {
            ch = fp;
            fp += 1;
        }

        let mut unsigned = false;
        let op = match f.at(ch) {
            b'n' => {
                if flags & SCAN_SUPPRESS == 0 {
                    sink.put(&mut obj_index, Value::Int(sp as i64));
                }
                nconversions += 1;
                continue;
            }
            b'd' | b'D' => {
                base = 10;
                b'i'
            }
            b'i' => {
                base = 0;
                b'i'
            }
            b'o' => {
                base = 8;
                b'i'
            }
            b'x' | b'X' => {
                base = 16;
                b'i'
            }
            b'u' => {
                base = 10;
                unsigned = true;
                flags |= SCAN_UNSIGNED;
                b'i'
            }
            b'f' | b'e' | b'E' | b'g' => b'f',
            b's' => b's',
            b'c' => {
                flags |= SCAN_NOSKIP;
                if width == 0 {
                    width = 1;
                }
                b's'
            }
            b'[' => {
                flags |= SCAN_NOSKIP;
                b'['
            }
            _ => 0,
        };

        if s.at(sp) == 0 {
            underflow = true;
            break 'scan;
        }
        if flags & SCAN_NOSKIP == 0 {
            while s.at(sp) != 0 && isspace(s.at(sp)) {
                sp += 1;
            }
            if s.at(sp) == 0 {
                underflow = true;
                break 'scan;
            }
        }

        match op {
            b's' => {
                let mut w = if width == 0 { usize::MAX } else { width };
                let mut end = sp;
                while s.at(end) != 0 {
                    if isspace(s.at(end)) {
                        break;
                    }
                    end += 1;
                    w -= 1;
                    if w == 0 {
                        break;
                    }
                }
                if flags & SCAN_SUPPRESS == 0
                    && !sink.put(&mut obj_index, Value::str(&s.0[sp..end]))
                {
                    nconversions += 1;
                    continue;
                }
                sp = end;
            }
            b'[' => {
                let mut w = if width == 0 { usize::MAX } else { width };
                let mut end = sp;
                let (cset, next) = CharSet::build(&f, fp);
                fp = next;
                while s.at(end) != 0 {
                    if !cset.contains(s.at(end)) {
                        break;
                    }
                    end += 1;
                    w -= 1;
                    if w == 0 {
                        break;
                    }
                }
                if sp == end {
                    break 'scan;
                }
                if flags & SCAN_SUPPRESS == 0
                    && !sink.put(&mut obj_index, Value::str(&s.0[sp..end]))
                {
                    nconversions += 1;
                    continue;
                }
                sp = end;
            }
            b'i' => {
                buf.clear();
                let mut w = if width == 0 || width > 63 { 63 } else { width };
                flags |= SCAN_SIGNOK | SCAN_NODIGITS | SCAN_NOZERO;
                while w > 0 {
                    let c = s.at(sp);
                    let add = match c {
                        b'0' => {
                            if base == 16 {
                                flags |= SCAN_XOK;
                            }
                            if base == 0 {
                                base = 8;
                                flags |= SCAN_XOK;
                            }
                            if flags & SCAN_NOZERO != 0 {
                                flags &= !(SCAN_SIGNOK | SCAN_NODIGITS | SCAN_NOZERO);
                            } else {
                                flags &= !(SCAN_SIGNOK | SCAN_XOK | SCAN_NODIGITS);
                            }
                            true
                        }
                        b'1'..=b'7' => {
                            if base == 0 {
                                base = 10;
                            }
                            flags &= !(SCAN_SIGNOK | SCAN_XOK | SCAN_NODIGITS);
                            true
                        }
                        b'8' | b'9' => {
                            if base == 0 {
                                base = 10;
                            }
                            if base <= 8 {
                                false
                            } else {
                                flags &= !(SCAN_SIGNOK | SCAN_XOK | SCAN_NODIGITS);
                                true
                            }
                        }
                        b'A'..=b'F' | b'a'..=b'f' => {
                            if base <= 10 {
                                false
                            } else {
                                flags &= !(SCAN_SIGNOK | SCAN_XOK | SCAN_NODIGITS);
                                true
                            }
                        }
                        b'+' | b'-' => {
                            if flags & SCAN_SIGNOK != 0 {
                                flags &= !SCAN_SIGNOK;
                                true
                            } else {
                                false
                            }
                        }
                        b'x' | b'X' if flags & SCAN_XOK != 0 && buf.len() == 1 => {
                            base = 16;
                            flags &= !SCAN_XOK;
                            true
                        }
                        _ => false,
                    };
                    if !add {
                        break;
                    }
                    buf.push(c);
                    sp += 1;
                    if s.at(sp) == 0 {
                        break;
                    }
                    w -= 1;
                }
                if flags & SCAN_NODIGITS != 0 {
                    if s.at(sp) == 0 {
                        underflow = true;
                    }
                    break 'scan;
                } else if matches!(buf.last(), Some(b'x' | b'X')) {
                    buf.pop();
                    sp -= 1;
                }
                if flags & SCAN_SUPPRESS == 0 {
                    let v = if unsigned {
                        let u = c_strtoul(&buf, base);
                        if (u as i64) < 0 {
                            Value::str(u.to_string().into_bytes())
                        } else {
                            Value::Int(u as i64)
                        }
                    } else {
                        Value::Int(c_strtol(&buf, base))
                    };
                    if !sink.put(&mut obj_index, v) {
                        nconversions += 1;
                        continue;
                    }
                }
            }
            b'f' => {
                buf.clear();
                let mut w = if width == 0 || width > 63 { 63 } else { width };
                flags |= SCAN_SIGNOK | SCAN_NODIGITS | SCAN_PTOK | SCAN_EXPOK;
                while w > 0 {
                    let c = s.at(sp);
                    let add = match c {
                        b'0'..=b'9' => {
                            flags &= !(SCAN_SIGNOK | SCAN_NODIGITS);
                            true
                        }
                        b'+' | b'-' => {
                            if flags & SCAN_SIGNOK != 0 {
                                flags &= !SCAN_SIGNOK;
                                true
                            } else {
                                false
                            }
                        }
                        b'.' => {
                            if flags & SCAN_PTOK != 0 {
                                flags &= !(SCAN_SIGNOK | SCAN_PTOK);
                                true
                            } else {
                                false
                            }
                        }
                        b'e' | b'E' if flags & (SCAN_NODIGITS | SCAN_EXPOK) == SCAN_EXPOK => {
                            flags =
                                (flags & !(SCAN_EXPOK | SCAN_PTOK)) | SCAN_SIGNOK | SCAN_NODIGITS;
                            true
                        }
                        _ => false,
                    };
                    if !add {
                        break;
                    }
                    buf.push(c);
                    sp += 1;
                    if s.at(sp) == 0 {
                        break;
                    }
                    w -= 1;
                }
                if flags & SCAN_NODIGITS != 0 {
                    if flags & SCAN_EXPOK != 0 {
                        if s.at(sp) == 0 {
                            underflow = true;
                        }
                        break 'scan;
                    }
                    // a bad exponent: 'e' and maybe a sign
                    let last = buf.pop();
                    sp -= 1;
                    if !matches!(last, Some(b'e' | b'E')) {
                        buf.pop();
                        sp -= 1;
                    }
                }
                if flags & SCAN_SUPPRESS == 0 {
                    let d = std::str::from_utf8(&buf)
                        .ok()
                        .and_then(|t| t.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    if !sink.put(&mut obj_index, Value::Float(d)) {
                        nconversions += 1;
                        continue;
                    }
                }
            }
            _ => {}
        }
        nconversions += 1;
    }

    if underflow && nconversions == 0 {
        return Ok(if num_vars != 0 {
            Value::Int(SCAN_ERROR_EOF)
        } else {
            Value::Null
        });
    }
    Ok(match sink {
        Sink::Vars(_) => Value::Int(nconversions as i64),
        Sink::Array(a) => Value::Array(a),
    })
}

fn fn_sscanf(_host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let input = conv::to_string_bytes(&deref(&args[0]));
    let format = conv::to_string_bytes(&deref(&args[1]));
    php_sscanf_internal(&input, &format, &args[2.min(args.len())..])
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("sscanf", fn_sscanf)]);
}

#[cfg(test)]
mod tests;
