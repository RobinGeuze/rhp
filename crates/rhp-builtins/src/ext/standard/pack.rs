//! ext/standard/pack.c: `pack` and `unpack`, ported function by function.
//! rhp runs on little-endian machines with a 4-byte C `int`.

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::ext::standard::formatted_print::zval_get_double;
use crate::zend::zpp::{zval_get_long, zval_get_string};
use crate::{Host, Thrown};

const INT_MAX: i64 = i32::MAX as i64;

fn value_error(msg: String) -> Thrown {
    Thrown::error("ValueError", msg)
}

fn warn(host: &mut dyn Host, msg: String) -> Result<(), Thrown> {
    host.docref_diagnostic(crate::E_WARNING, msg.as_bytes())
}

/// C `atoi` as php's pack() uses it: `strtol` truncated to `int`.
fn atoi(s: &[u8]) -> i64 {
    let mut n: i64 = 0;
    for &c in s.iter().take_while(|c| c.is_ascii_digit()) {
        n = n.saturating_mul(10).saturating_add((c - b'0') as i64);
    }
    n as i32 as i64
}

#[derive(Clone, Copy, PartialEq)]
enum Endian {
    Little,
    Big,
}

/// `php_pack`: the low `size` bytes of the integer in `endian` order.
fn pack_long(
    host: &mut dyn Host,
    out: &mut [u8],
    pos: usize,
    v: &Value,
    size: usize,
    endian: Endian,
) -> Result<(), Thrown> {
    let zl = zval_get_long(host, v)?;
    match endian {
        Endian::Little => out[pos..pos + size].copy_from_slice(&zl.to_le_bytes()[..size]),
        Endian::Big => out[pos..pos + size].copy_from_slice(&zl.to_be_bytes()[8 - size..]),
    }
    Ok(())
}

/// `pack(string $format, mixed ...$values): string`.
fn fn_pack(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let format = conv::to_string_bytes(&args[0]);
    let mut argv: Vec<Value> = args[1..].iter().map(deref).collect();
    let num_args = argv.len() as i64;
    let mut codes: Vec<(u8, i64)> = Vec::new();
    let mut currentarg: i64 = 0;
    let mut i = 0;
    while i < format.len() {
        let code = format[i];
        i += 1;
        let mut arg: i64 = 1;
        if i < format.len() {
            let c = format[i];
            if c == b'*' {
                arg = -1;
                i += 1;
            } else if c.is_ascii_digit() {
                arg = atoi(&format[i..]);
                while i < format.len() && format[i].is_ascii_digit() {
                    i += 1;
                }
            }
        }
        match code {
            b'x' | b'X' | b'@' => {
                if arg < 0 {
                    warn(host, format!("Type {}: '*' ignored", code as char))?;
                    arg = 1;
                }
            }
            b'a' | b'A' | b'Z' | b'h' | b'H' => {
                if currentarg >= num_args {
                    return Err(value_error(format!(
                        "Type {}: not enough arguments",
                        code as char
                    )));
                }
                if arg < 0 {
                    let s = zval_get_string(host, &argv[currentarg as usize])?;
                    arg = s.len() as i64;
                    if code == b'Z' {
                        // Z is always NUL-terminated.
                        arg += 1;
                    }
                    argv[currentarg as usize] = Value::str(s);
                }
                currentarg += 1;
            }
            b'q' | b'Q' | b'J' | b'P' | b'c' | b'C' | b's' | b'S' | b'i' | b'I' | b'l' | b'L'
            | b'n' | b'N' | b'v' | b'V' | b'f' | b'g' | b'G' | b'd' | b'e' | b'E' => {
                if arg < 0 {
                    arg = num_args - currentarg;
                }
                if currentarg > INT_MAX - arg || currentarg + arg > num_args {
                    return Err(value_error(format!(
                        "Type {}: too few arguments",
                        code as char
                    )));
                }
                currentarg += arg;
            }
            _ => {
                return Err(value_error(format!(
                    "Type {}: unknown format code",
                    code as char
                )))
            }
        }
        codes.push((code, arg));
    }
    if currentarg < num_args {
        warn(host, format!("{} arguments unused", num_args - currentarg))?;
    }

    // The output length's upper bound.
    let mut outputpos: i64 = 0;
    let mut outputsize: i64 = 0;
    for &(code, arg) in &codes {
        let unit: i64 = match code {
            b'h' | b'H' => {
                let a = arg / 2 + arg % 2;
                inc_outputpos(&mut outputpos, a, 1, code)?;
                0
            }
            b'a' | b'A' | b'Z' | b'c' | b'C' | b'x' => 1,
            b's' | b'S' | b'n' | b'v' => 2,
            b'i' | b'I' | b'l' | b'L' | b'N' | b'V' | b'f' | b'g' | b'G' => 4,
            b'q' | b'Q' | b'J' | b'P' | b'd' | b'e' | b'E' => 8,
            b'X' => {
                outputpos -= arg;
                if outputpos < 0 {
                    warn(host, format!("Type {}: outside of string", code as char))?;
                    outputpos = 0;
                }
                0
            }
            _ => {
                // '@'
                outputpos = arg;
                0
            }
        };
        if unit > 0 {
            inc_outputpos(&mut outputpos, arg, unit, code)?;
        }
        outputsize = outputsize.max(outputpos);
    }

    let mut out = vec![0u8; outputsize as usize + 1];
    let mut pos: i64 = 0;
    let mut cur = 0usize;
    for &(code, arg) in &codes {
        let mut arg = arg;
        match code {
            b'a' | b'A' | b'Z' => {
                let arg_cp = if code != b'Z' { arg } else { (arg - 1).max(0) } as usize;
                let s = zval_get_string(host, &argv[cur])?;
                cur += 1;
                let p = pos as usize;
                let fill = if code == b'A' { b' ' } else { 0 };
                out[p..p + arg as usize].fill(fill);
                let n = s.len().min(arg_cp);
                out[p..p + n].copy_from_slice(&s[..n]);
                pos += arg;
            }
            b'h' | b'H' => {
                let mut nibbleshift = if code == b'h' { 0 } else { 4 };
                let mut first = true;
                let s = zval_get_string(host, &argv[cur])?;
                cur += 1;
                pos -= 1;
                if arg as usize > s.len() {
                    warn(
                        host,
                        format!("Type {}: not enough characters in string", code as char),
                    )?;
                    arg = s.len() as i64;
                }
                for &c in &s[..arg as usize] {
                    let n = match c {
                        b'0'..=b'9' => c - b'0',
                        b'A'..=b'F' => c - b'A' + 10,
                        b'a'..=b'f' => c - b'a' + 10,
                        _ => {
                            let mut m =
                                format!("Type {}: illegal hex digit ", code as char).into_bytes();
                            m.push(c);
                            host.docref_diagnostic(crate::E_WARNING, &m)?;
                            0
                        }
                    };
                    if first {
                        pos += 1;
                        out[pos as usize] = 0;
                        first = false;
                    } else {
                        first = true;
                    }
                    out[pos as usize] |= n << nibbleshift;
                    nibbleshift = (nibbleshift + 4) & 7;
                }
                pos += 1;
            }
            b'c' | b'C' | b's' | b'S' | b'n' | b'v' | b'i' | b'I' | b'l' | b'L' | b'N' | b'V'
            | b'q' | b'Q' | b'J' | b'P' => {
                let (size, endian) = match code {
                    b'c' | b'C' => (1, Endian::Little),
                    b'n' => (2, Endian::Big),
                    b's' | b'S' | b'v' => (2, Endian::Little),
                    b'N' => (4, Endian::Big),
                    b'i' | b'I' | b'l' | b'L' | b'V' => (4, Endian::Little),
                    b'J' => (8, Endian::Big),
                    _ => (8, Endian::Little),
                };
                for _ in 0..arg {
                    let v = argv[cur].clone();
                    cur += 1;
                    pack_long(host, &mut out, pos as usize, &v, size, endian)?;
                    pos += size as i64;
                }
            }
            b'f' | b'g' | b'G' => {
                for _ in 0..arg {
                    let v = argv[cur].clone();
                    cur += 1;
                    let f = zval_get_double(host, &v)? as f32;
                    let b = if code == b'G' {
                        f.to_be_bytes()
                    } else {
                        f.to_le_bytes()
                    };
                    out[pos as usize..pos as usize + 4].copy_from_slice(&b);
                    pos += 4;
                }
            }
            b'd' | b'e' | b'E' => {
                for _ in 0..arg {
                    let v = argv[cur].clone();
                    cur += 1;
                    let d = zval_get_double(host, &v)?;
                    let b = if code == b'E' {
                        d.to_be_bytes()
                    } else {
                        d.to_le_bytes()
                    };
                    out[pos as usize..pos as usize + 8].copy_from_slice(&b);
                    pos += 8;
                }
            }
            b'x' => {
                out[pos as usize..(pos + arg) as usize].fill(0);
                pos += arg;
            }
            b'X' => {
                pos = (pos - arg).max(0);
            }
            _ => {
                // '@'
                if arg > pos {
                    out[pos as usize..arg as usize].fill(0);
                }
                pos = arg;
            }
        }
    }
    out.truncate(pos as usize);
    Ok(Value::str(out))
}

/// `INC_OUTPUTPOS`: grow the bound by `a * b` within INT_MAX.
fn inc_outputpos(outputpos: &mut i64, a: i64, b: i64, code: u8) -> Result<(), Thrown> {
    if a < 0 || (INT_MAX - *outputpos) / b < a {
        return Err(value_error(format!(
            "Type {}: integer overflow in format string",
            code as char
        )));
    }
    *outputpos += a * b;
    Ok(())
}

/// C `strtol(s, NULL, 10)` on leading digits, with its ERANGE overflow.
fn strtol10(s: &[u8]) -> Option<i64> {
    let mut n: i64 = 0;
    for &c in s.iter().take_while(|c| c.is_ascii_digit()) {
        n = n.checked_mul(10)?.checked_add((c - b'0') as i64)?;
    }
    Some(n)
}

/// `unpack(string $format, string $string, int $offset = 0): array|false`.
fn fn_unpack(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let format = conv::to_string_bytes(&args[0]);
    let data = conv::to_string_bytes(&args[1]);
    let offset = args.get(2).map(|v| conv::to_int(&deref(v))).unwrap_or(0);
    if offset < 0 || offset > data.len() as i64 {
        return Err(value_error(
            "unpack(): Argument #3 ($offset) must be contained in argument #2 ($data)".into(),
        ));
    }
    let input = &data[offset as usize..];
    let inputlen = input.len() as i64;
    let mut inputpos: i64 = 0;
    let mut result = PhpArray::new();
    let mut f = 0usize;
    while f < format.len() {
        let ty = format[f];
        f += 1;
        let mut repetitions: i64 = 1;
        if f < format.len() {
            let c = format[f];
            if c.is_ascii_digit() {
                match strtol10(&format[f..]) {
                    Some(n) if n <= INT_MAX => repetitions = n,
                    _ => {
                        warn(host, format!("Type {}: integer overflow", ty as char))?;
                        return Ok(Value::Bool(false));
                    }
                }
                while f < format.len() && format[f].is_ascii_digit() {
                    f += 1;
                }
            } else if c == b'*' {
                repetitions = -1;
                f += 1;
            }
        }
        let name_start = f;
        let argb = repetitions;
        while f < format.len() && format[f] != b'/' {
            f += 1;
        }
        let name = &format[name_start..name_start + (f - name_start).min(200)];
        let mut size: i64 = match ty {
            b'X' => {
                if repetitions < 0 {
                    warn(host, format!("Type {}: '*' ignored", ty as char))?;
                    repetitions = 1;
                }
                -1
            }
            b'@' => 0,
            b'a' | b'A' | b'Z' => {
                let s = repetitions;
                repetitions = 1;
                s
            }
            b'h' | b'H' => {
                let s = if repetitions > 0 {
                    (repetitions + 1) / 2
                } else {
                    repetitions
                };
                repetitions = 1;
                s
            }
            b'c' | b'C' | b'x' => 1,
            b's' | b'S' | b'n' | b'v' => 2,
            b'i' | b'I' | b'l' | b'L' | b'N' | b'V' | b'f' | b'g' | b'G' => 4,
            b'q' | b'Q' | b'J' | b'P' | b'd' | b'e' | b'E' => 8,
            _ => {
                return Err(value_error(format!("Invalid format type {}", ty as char)));
            }
        };
        let mut i: i64 = 0;
        while i != repetitions {
            if size != 0 && size != -1 && INT_MAX - size + 1 < inputpos {
                warn(host, format!("Type {}: integer overflow", ty as char))?;
                return Ok(Value::Bool(false));
            }
            if inputpos + size <= inputlen {
                let key = if name.is_empty() {
                    ArrayKey::Int(i + 1)
                } else if repetitions == 1 {
                    ArrayKey::from_bytes(name)
                } else {
                    let mut n = name.to_vec();
                    n.extend_from_slice((i + 1).to_string().as_bytes());
                    ArrayKey::from_bytes(&n)
                };
                let p = inputpos as usize;
                let rest = (inputlen - inputpos) as usize;
                let le = |n: usize| {
                    let mut b = [0u8; 8];
                    b[..n].copy_from_slice(&input[p..p + n]);
                    u64::from_le_bytes(b)
                };
                let be = |n: usize| {
                    let mut b = [0u8; 8];
                    b[8 - n..].copy_from_slice(&input[p..p + n]);
                    u64::from_be_bytes(b)
                };
                let mut output = true;
                let val = match ty {
                    b'a' | b'A' | b'Z' => {
                        let mut len = rest;
                        if size >= 0 && len as i64 > size {
                            len = size as usize;
                        }
                        size = len as i64;
                        let s = &input[p..p + len];
                        Value::str(match ty {
                            b'a' => s.to_vec(),
                            b'A' => {
                                let end = s
                                    .iter()
                                    .rposition(|&c| !matches!(c, 0 | b' ' | b'\t' | b'\r' | b'\n'))
                                    .map_or(0, |e| e + 1);
                                s[..end].to_vec()
                            }
                            _ => {
                                let end = s.iter().position(|&c| c == 0).unwrap_or(len);
                                s[..end].to_vec()
                            }
                        })
                    }
                    b'h' | b'H' => {
                        if size > INT_MAX / 2 {
                            return Err(value_error(format!(
                                "unpack(): Argument #1 ($format) repeater must be less than or equal to {}",
                                INT_MAX / 2
                            )));
                        }
                        let mut len = rest as i64 * 2;
                        if size >= 0 && len > size * 2 {
                            len = size * 2;
                        }
                        if len > 0 && argb > 0 {
                            len -= argb % 2;
                        }
                        let mut nibbleshift = if ty == b'h' { 0 } else { 4 };
                        let mut first = true;
                        let mut ipos = 0usize;
                        let mut buf = Vec::with_capacity(len.max(0) as usize);
                        for _ in 0..len {
                            let cc =
                                (input.get(p + ipos).copied().unwrap_or(0) >> nibbleshift) & 0xf;
                            buf.push(if cc < 10 { cc + b'0' } else { cc - 10 + b'a' });
                            nibbleshift = (nibbleshift + 4) & 7;
                            if first {
                                first = false;
                            } else {
                                ipos += 1;
                                first = true;
                            }
                        }
                        Value::str(buf)
                    }
                    b'c' => Value::Int(input[p] as i8 as i64),
                    b'C' => Value::Int(input[p] as i64),
                    b's' => Value::Int(le(2) as u16 as i16 as i64),
                    b'S' | b'v' => Value::Int(le(2) as i64),
                    b'n' => Value::Int(be(2) as i64),
                    b'i' | b'l' => Value::Int(le(4) as u32 as i32 as i64),
                    b'I' | b'L' | b'V' => Value::Int(le(4) as i64),
                    b'N' => Value::Int(be(4) as i64),
                    b'q' | b'Q' | b'P' => Value::Int(le(8) as i64),
                    b'J' => Value::Int(be(8) as i64),
                    b'f' | b'g' => Value::Float(f32::from_bits(le(4) as u32) as f64),
                    b'G' => Value::Float(f32::from_bits(be(4) as u32) as f64),
                    b'd' | b'e' => Value::Float(f64::from_bits(le(8))),
                    b'E' => Value::Float(f64::from_bits(be(8))),
                    b'x' => {
                        output = false;
                        Value::Null
                    }
                    b'X' => {
                        if inputpos < size {
                            inputpos = -size;
                            i = repetitions - 1;
                            if repetitions >= 0 {
                                warn(host, format!("Type {}: outside of string", ty as char))?;
                            }
                        }
                        output = false;
                        Value::Null
                    }
                    _ => {
                        // '@'
                        if repetitions <= inputlen {
                            inputpos = repetitions;
                        } else {
                            warn(host, format!("Type {}: outside of string", ty as char))?;
                        }
                        i = repetitions - 1;
                        output = false;
                        Value::Null
                    }
                };
                if output {
                    result.insert(key, val);
                }
                inputpos += size;
                if inputpos < 0 {
                    if size != -1 {
                        warn(host, format!("Type {}: outside of string", ty as char))?;
                    }
                    inputpos = 0;
                }
            } else if repetitions < 0 {
                break;
            } else {
                let left = inputlen - inputpos;
                warn(
                    host,
                    format!(
                        "Type {}: not enough input values, need {size} values but only {left} {} provided",
                        ty as char,
                        if left == 1 { "was" } else { "were" }
                    ),
                )?;
                return Ok(Value::Bool(false));
            }
            i += 1;
        }
        if f < format.len() {
            f += 1; // the '/'
        }
    }
    Ok(Value::Array(result))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("pack", fn_pack), ("unpack", fn_unpack)]);
}
