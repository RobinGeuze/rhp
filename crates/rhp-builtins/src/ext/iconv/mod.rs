//! The `iconv` extension's character-counting family (php-src
//! `ext/iconv/iconv.c`): `iconv_strlen`, `iconv_substr`, `iconv_strpos`,
//! `iconv_strrpos` and `iconv_mime_decode`.
//!
//! php drives the system iconv(3) converting to UCS-4LE one character at a
//! time; this module decodes through a small table of charsets (UTF-8,
//! ASCII, ISO-8859-1, Windows-1252, UTF-16/32 and UCS-2/4 in both byte
//! orders) and reproduces the C loops' observable quirks — notably that
//! `iconv_strpos` stops silently at the first illegal sequence (unless it
//! reached the character before it — libiconv's one-character lookahead)
//! while `iconv_strlen` / `iconv_substr` report it. `_php_iconv_show_error`'s
//! E_NOTICE / E_WARNING texts go to the host's diagnostic sink.
//!
//! `iconv()` converts through the same tables (`//TRANSLIT` with libiconv's
//! spellings of the common Latin letters).

use std::collections::HashMap;

use rhp_values::Value;

use crate::ext::zpp::{int_arg_or, opt_int_arg, opt_str_arg, str_arg, value_error};
use crate::{notice, warn, Builtin, Host, Thrown};

/// The charsets this module can convert.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cs {
    Utf8,
    Ascii,
    Latin1,
    Cp1252,
    Utf16Le,
    Utf16Be,
    Utf32Le,
    Utf32Be,
    Ucs2Le,
    Ucs2Be,
}

fn charset(name: &[u8]) -> Option<Cs> {
    let up = String::from_utf8_lossy(name).to_ascii_uppercase();
    let base = up.split("//").next().unwrap_or("");
    Some(match base {
        "UTF-8" | "UTF8" => Cs::Utf8,
        "ASCII" | "US-ASCII" | "ANSI_X3.4-1968" | "646" | "US" | "CP367" | "IBM367" => Cs::Ascii,
        "ISO-8859-1" | "ISO8859-1" | "ISO_8859-1" | "LATIN1" | "L1" | "CP819" | "IBM819" => {
            Cs::Latin1
        }
        "CP1252" | "WINDOWS-1252" => Cs::Cp1252,
        "UTF-16LE" => Cs::Utf16Le,
        "UTF-16BE" | "UTF-16" => Cs::Utf16Be,
        "UTF-32LE" | "UCS-4LE" => Cs::Utf32Le,
        "UTF-32BE" | "UTF-32" | "UCS-4" | "UCS-4BE" => Cs::Utf32Be,
        "UCS-2LE" => Cs::Ucs2Le,
        "UCS-2BE" | "UCS-2" => Cs::Ucs2Be,
        _ => return None,
    })
}

const CP1252_HI: [u16; 32] = [
    0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039,
    0x0152, 0, 0x017D, 0, 0, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC,
    0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
];

/// How decoding stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stop {
    /// Consumed everything.
    End,
    /// EILSEQ: an illegal sequence.
    Illegal,
    /// EINVAL: an incomplete sequence at the end of the input.
    Incomplete,
}

/// Decode as many characters as possible.
fn decode(cs: Cs, b: &[u8]) -> (Vec<u32>, Stop) {
    let mut out = Vec::with_capacity(b.len());
    let n = b.len();
    let mut i = 0;
    macro_rules! unit16 {
        ($le:expr, $j:expr) => {
            if $le {
                u16::from_le_bytes([b[$j], b[$j + 1]]) as u32
            } else {
                u16::from_be_bytes([b[$j], b[$j + 1]]) as u32
            }
        };
    }
    match cs {
        Cs::Utf8 => {
            let s = b;
            while i < n {
                let c = s[i];
                if c < 0x80 {
                    out.push(c as u32);
                    i += 1;
                    continue;
                }
                let (len, lo, hi) = match c {
                    0xc2..=0xdf => (2, 0x80, 0xbf),
                    0xe0 => (3, 0xa0, 0xbf),
                    0xed => (3, 0x80, 0x9f),
                    0xe1..=0xef => (3, 0x80, 0xbf),
                    0xf0 => (4, 0x90, 0xbf),
                    0xf4 => (4, 0x80, 0x8f),
                    0xf1..=0xf3 => (4, 0x80, 0xbf),
                    _ => return (out, Stop::Illegal),
                };
                for k in 1..len {
                    if i + k >= n {
                        return (out, Stop::Incomplete);
                    }
                    let ok = if k == 1 {
                        (lo..=hi).contains(&s[i + k])
                    } else {
                        (0x80..=0xbf).contains(&s[i + k])
                    };
                    if !ok {
                        return (out, Stop::Illegal);
                    }
                }
                let cp = match len {
                    2 => ((c as u32 & 0x1f) << 6) | (s[i + 1] as u32 & 0x3f),
                    3 => {
                        ((c as u32 & 0x0f) << 12)
                            | ((s[i + 1] as u32 & 0x3f) << 6)
                            | (s[i + 2] as u32 & 0x3f)
                    }
                    _ => {
                        ((c as u32 & 0x07) << 18)
                            | ((s[i + 1] as u32 & 0x3f) << 12)
                            | ((s[i + 2] as u32 & 0x3f) << 6)
                            | (s[i + 3] as u32 & 0x3f)
                    }
                };
                out.push(cp);
                i += len;
            }
        }
        Cs::Ascii => {
            for &c in b {
                if c >= 0x80 {
                    return (out, Stop::Illegal);
                }
                out.push(c as u32);
            }
        }
        Cs::Latin1 => out.extend(b.iter().map(|&c| c as u32)),
        Cs::Cp1252 => {
            for &c in b {
                if (0x80..0xa0).contains(&c) {
                    let m = CP1252_HI[(c - 0x80) as usize];
                    if m == 0 {
                        return (out, Stop::Illegal);
                    }
                    out.push(m as u32);
                } else {
                    out.push(c as u32);
                }
            }
        }
        Cs::Utf16Le | Cs::Utf16Be => {
            let le = cs == Cs::Utf16Le;
            while i < n {
                if i + 1 >= n {
                    return (out, Stop::Incomplete);
                }
                let u = unit16!(le, i);
                if (0xd800..0xdc00).contains(&u) {
                    if i + 3 >= n {
                        return (out, Stop::Incomplete);
                    }
                    let u2 = unit16!(le, i + 2);
                    if !(0xdc00..0xe000).contains(&u2) {
                        return (out, Stop::Illegal);
                    }
                    out.push(0x10000 + ((u - 0xd800) << 10) + (u2 - 0xdc00));
                    i += 4;
                } else if (0xdc00..0xe000).contains(&u) {
                    return (out, Stop::Illegal);
                } else {
                    out.push(u);
                    i += 2;
                }
            }
        }
        Cs::Ucs2Le | Cs::Ucs2Be => {
            let le = cs == Cs::Ucs2Le;
            while i < n {
                if i + 1 >= n {
                    return (out, Stop::Incomplete);
                }
                let u = unit16!(le, i);
                if (0xd800..0xe000).contains(&u) {
                    return (out, Stop::Illegal);
                }
                out.push(u);
                i += 2;
            }
        }
        Cs::Utf32Le | Cs::Utf32Be => {
            while i < n {
                if i + 3 >= n {
                    return (out, Stop::Incomplete);
                }
                let w = [b[i], b[i + 1], b[i + 2], b[i + 3]];
                let u = if cs == Cs::Utf32Le {
                    u32::from_le_bytes(w)
                } else {
                    u32::from_be_bytes(w)
                };
                if u > 0x10ffff || (0xd800..0xe000).contains(&u) {
                    return (out, Stop::Illegal);
                }
                out.push(u);
                i += 4;
            }
        }
    }
    (out, Stop::End)
}

/// Encode code points; `None` when one is not representable (EILSEQ).
fn encode(cs: Cs, cps: &[u32]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(cps.len());
    for &cp in cps {
        match cs {
            Cs::Utf8 => {
                let c = char::from_u32(cp)?;
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            Cs::Ascii => {
                if cp >= 0x80 {
                    return None;
                }
                out.push(cp as u8);
            }
            Cs::Latin1 => {
                if cp >= 0x100 {
                    return None;
                }
                out.push(cp as u8);
            }
            Cs::Cp1252 => {
                if cp < 0x80 || (0xa0..0x100).contains(&cp) {
                    out.push(cp as u8);
                } else {
                    let k = CP1252_HI.iter().position(|&m| m != 0 && m as u32 == cp)?;
                    out.push(0x80 + k as u8);
                }
            }
            Cs::Utf16Le | Cs::Utf16Be | Cs::Ucs2Le | Cs::Ucs2Be => {
                let c = char::from_u32(cp)?;
                let ucs2 = matches!(cs, Cs::Ucs2Le | Cs::Ucs2Be);
                if ucs2 && cp > 0xffff {
                    return None;
                }
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    if matches!(cs, Cs::Utf16Le | Cs::Ucs2Le) {
                        out.extend_from_slice(&u.to_le_bytes());
                    } else {
                        out.extend_from_slice(&u.to_be_bytes());
                    }
                }
            }
            Cs::Utf32Le => out.extend_from_slice(&cp.to_le_bytes()),
            Cs::Utf32Be => out.extend_from_slice(&cp.to_be_bytes()),
        }
    }
    Some(out)
}

/// php's `php_iconv_err_t` outcomes this module reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Err {
    WrongCharset,
    IllegalChar,
    IllegalSeq,
    Malformed,
    Unknown,
    OutOfBounds,
}

impl Err {
    fn from_stop(stop: Stop) -> Option<Err> {
        match stop {
            Stop::End => None,
            Stop::Incomplete => Some(Err::IllegalChar),
            Stop::Illegal => Some(Err::IllegalSeq),
        }
    }
}

/// The `errno` php's "Unknown error (%d)" prints: the decoders' failure leaves
/// EILSEQ behind.
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
const EILSEQ: i32 = 92;
#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "freebsd")))]
const EILSEQ: i32 = 84;

/// `_php_iconv_show_error`.
fn show_error(
    host: &mut dyn Host,
    fname: &str,
    err: Err,
    out_charset: &[u8],
    in_charset: &[u8],
) -> Result<(), Thrown> {
    match err {
        Err::WrongCharset => warn(
            host,
            fname,
            &format!(
                "Wrong encoding, conversion from \"{}\" to \"{}\" is not allowed",
                String::from_utf8_lossy(in_charset),
                String::from_utf8_lossy(out_charset)
            ),
        ),
        Err::IllegalChar => notice(
            host,
            fname,
            "Detected an incomplete multibyte character in input string",
        ),
        Err::IllegalSeq => notice(host, fname, "Detected an illegal character in input string"),
        Err::Malformed => warn(host, fname, "Malformed string"),
        Err::Unknown => notice(host, fname, &format!("Unknown error ({EILSEQ})")),
        Err::OutOfBounds => Err(value_error(
            fname,
            3,
            "offset",
            "must be contained in argument #1 ($haystack)",
        )),
    }
}

/// php's generic superset charset (`ICONV_UCS4_ENCODING`).
const SUPERSET: &[u8] = b"UCS-4LE";

/// The `$encoding` argument (`s!`, defaulting to the internal encoding UTF-8).
/// php warns and returns false for names of 64+ bytes.
fn enc_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
) -> Result<Option<Vec<u8>>, Thrown> {
    match opt_str_arg(host, args, i, fname, "encoding")? {
        None => Ok(Some(b"UTF-8".to_vec())),
        Some(e) if e.len() >= 64 => {
            warn(
                host,
                fname,
                "Encoding parameter exceeds the maximum allowed length of 64 characters",
            )?;
            Ok(None)
        }
        Some(e) => Ok(Some(e)),
    }
}

/// `_php_iconv_strlen`.
fn iconv_strlen(s: &[u8], enc: &[u8]) -> Result<usize, Err> {
    let cs = charset(enc).ok_or(Err::WrongCharset)?;
    let (cps, stop) = decode(cs, s);
    match Err::from_stop(stop) {
        None => Ok(cps.len()),
        Some(e) => Err(e),
    }
}

fn fn_iconv_strlen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = str_arg(host, args, 0, "iconv_strlen", "string")?;
    let Some(enc) = enc_arg(host, args, 1, "iconv_strlen")? else {
        return Ok(Value::Bool(false));
    };
    Ok(match iconv_strlen(&s, &enc) {
        Ok(n) => Value::Int(n as i64),
        Err(e) => {
            show_error(host, "iconv_strlen", e, SUPERSET, &enc)?;
            Value::Bool(false)
        }
    })
}

fn fn_iconv_substr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "iconv_substr";
    let s = str_arg(host, args, 0, f, "string")?;
    let mut offset = crate::ext::zpp::int_arg(host, args, 1, f, "offset")?;
    let len = opt_int_arg(host, args, 2, f, "length")?;
    let Some(enc) = enc_arg(host, args, 3, f)? else {
        return Ok(Value::Bool(false));
    };
    let mut len = len.unwrap_or(s.len() as i64);
    let total = match iconv_strlen(&s, &enc) {
        Ok(t) => t as i64,
        Err(e) => {
            show_error(host, f, e, SUPERSET, &enc)?;
            return Ok(Value::Bool(false));
        }
    };
    if offset < 0 {
        offset += total;
        if offset < 0 {
            offset = 0;
        }
    } else if offset > total {
        offset = total;
    }
    if len < 0 {
        len += total - offset;
        if len < 0 {
            len = 0;
        }
    } else if len > total {
        len = total;
    }
    if offset + len > total {
        len = total - offset;
    }
    if len == 0 {
        return Ok(Value::str(""));
    }
    let cs = charset(&enc).unwrap_or(Cs::Utf8);
    let (cps, _) = decode(cs, &s);
    let piece = &cps[offset as usize..(offset + len) as usize];
    Ok(match encode(cs, piece) {
        Some(b) => Value::str(b),
        None => Value::Bool(false),
    })
}

/// `_php_iconv_strpos`: the C loop's partial-match bookkeeping. php feeds
/// iconv(3) one output character at a time, so decoding stops silently at the
/// first undecodable input — except that libiconv looks one character ahead:
/// converting the character just before the bad input already fails, which
/// sets `err` (and the result is then `false`). A forward search that
/// completes its match earlier never gets there.
fn iconv_strpos(
    h: &[u8],
    n: &[u8],
    offset: usize,
    enc: &[u8],
    reverse: bool,
) -> Result<Option<usize>, Err> {
    let cs = charset(enc).ok_or(Err::WrongCharset)?;
    let (ndl, nstop) = decode(cs, n);
    if let Some(e) = Err::from_stop(nstop) {
        return Err(e);
    }
    let (hay, hstop) = decode(cs, h);
    let hay_err = Err::from_stop(hstop);
    let mut err = None;
    let mut ret = None;
    let mut p = 0usize;
    let mut match_ofs: Option<usize> = None;
    let mut cnt = 0usize;
    for &c in &hay {
        if cnt + 1 == hay.len() && hay_err.is_some() {
            err = hay_err;
        }
        if cnt >= offset {
            if c == ndl[p] {
                if match_ofs.is_none() {
                    match_ofs = Some(cnt);
                }
                p += 1;
                if p == ndl.len() {
                    ret = match_ofs;
                    if reverse {
                        p = 0;
                        match_ofs = None;
                    } else {
                        break;
                    }
                }
            } else {
                let mut i = 0usize;
                let mut j = 1usize;
                let lim = p;
                while j < lim {
                    if ndl[j] == ndl[i] {
                        i += 1;
                    } else {
                        j -= i;
                        i = 0;
                    }
                    j += 1;
                }
                if c == ndl[i] {
                    match_ofs = match_ofs.map(|m| m + (lim - i));
                    i += 1;
                    p = i;
                } else {
                    match_ofs = None;
                    p = 0;
                }
            }
        }
        cnt += 1;
    }
    if let Some(e) = err {
        return Err(e);
    }
    if offset > cnt {
        return Err(Err::OutOfBounds);
    }
    Ok(ret)
}

fn fn_iconv_strpos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "iconv_strpos";
    let h = str_arg(host, args, 0, f, "haystack")?;
    let n = str_arg(host, args, 1, f, "needle")?;
    let mut offset = int_arg_or(host, args, 2, f, "offset", 0)?;
    let Some(enc) = enc_arg(host, args, 3, f)? else {
        return Ok(Value::Bool(false));
    };
    if offset < 0 {
        let hl = match iconv_strlen(&h, &enc) {
            Ok(hl) => hl,
            Err(e) => {
                show_error(host, f, e, SUPERSET, &enc)?;
                return Ok(Value::Bool(false));
            }
        };
        offset += hl as i64;
        if offset < 0 {
            show_error(host, f, Err::OutOfBounds, SUPERSET, &enc)?;
        }
    }
    if n.is_empty() {
        return Ok(Value::Bool(false));
    }
    match iconv_strpos(&h, &n, offset as usize, &enc, false) {
        Ok(Some(p)) => Ok(Value::Int(p as i64)),
        Ok(None) => Ok(Value::Bool(false)),
        Err(e) => {
            show_error(host, f, e, SUPERSET, &enc)?;
            Ok(Value::Bool(false))
        }
    }
}

fn fn_iconv_strrpos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "iconv_strrpos";
    let h = str_arg(host, args, 0, f, "haystack")?;
    let n = str_arg(host, args, 1, f, "needle")?;
    if n.is_empty() {
        return Ok(Value::Bool(false));
    }
    let Some(enc) = enc_arg(host, args, 2, f)? else {
        return Ok(Value::Bool(false));
    };
    match iconv_strpos(&h, &n, 0, &enc, true) {
        Ok(Some(p)) => Ok(Value::Int(p as i64)),
        Ok(None) => Ok(Value::Bool(false)),
        Err(e) => {
            show_error(host, f, e, SUPERSET, &enc)?;
            Ok(Value::Bool(false))
        }
    }
}

// ---------------------------------------------------------------------------
// iconv_mime_decode
// ---------------------------------------------------------------------------

const MODE_STRICT: i64 = 1;
const MODE_CONTINUE_ON_ERROR: i64 = 2;

/// An iconv descriptor: append `bytes` (in `from`) to `out` (in `to`).
fn append(out: &mut Vec<u8>, bytes: &[u8], from: Cs, to: Cs) -> Result<(), Err> {
    let (cps, stop) = decode(from, bytes);
    if let Some(e) = Err::from_stop(stop) {
        return Err(e);
    }
    match encode(to, &cps) {
        Some(b) => {
            out.extend_from_slice(&b);
            Ok(())
        }
        None => Err(Err::IllegalSeq),
    }
}

/// php_base64_decode (non-strict): skips unknown characters and padding;
/// never fails.
fn base64_decode(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut cur = 0u8;
    for &c in s {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        match i % 4 {
            0 => cur = v << 2,
            1 => {
                out.push(cur | (v >> 4));
                cur = (v & 0x0f) << 4;
            }
            2 => {
                out.push(cur | (v >> 2));
                cur = (v & 0x03) << 6;
            }
            _ => out.push(cur | v),
        }
        i += 1;
    }
    Some(out)
}

/// php_quot_print_decode(str, len, replace_us_by_ws = 1).
fn qprint_decode(s: &[u8]) -> Option<Vec<u8>> {
    // hexval_tbl: 0-15 hex digit, 32 for SP/HT, 16 for CR/LF, 64 otherwise.
    let hv = |c: u8| -> u32 {
        match c {
            b'0'..=b'9' => (c - b'0') as u32,
            b'A'..=b'F' => (c - b'A' + 10) as u32,
            b'a'..=b'f' => (c - b'a' + 10) as u32,
            b' ' | b'\t' => 32,
            b'\r' | b'\n' => 16,
            _ => 64,
        }
    };
    let mut out = Vec::with_capacity(s.len());
    let mut i = s.len();
    let mut p = 0usize;
    while i > 0 && s[p] != 0 {
        if s[p] == b'=' {
            i -= 1;
            p += 1;
            if i == 0 || s[p] == 0 {
                break;
            }
            let mut h = hv(s[p]);
            if h < 16 {
                i -= 1;
                if i == 0 {
                    return None;
                }
                p += 1;
                let l = hv(s[p]);
                if l >= 16 {
                    return None;
                }
                out.push(((h << 4) | l) as u8);
                i -= 1;
                p += 1;
            } else if h < 64 {
                while h == 32 {
                    i -= 1;
                    if i == 0 {
                        return None;
                    }
                    p += 1;
                    h = hv(s[p]);
                    if h == 64 {
                        return None;
                    }
                }
                if s[p] == b'\r' && i >= 2 && s[p + 1] == b'\n' {
                    i -= 1;
                    p += 1;
                }
                i -= 1;
                p += 1;
            } else {
                return None;
            }
        } else {
            out.push(if s[p] == b'_' { b' ' } else { s[p] });
            i -= 1;
            p += 1;
        }
    }
    Some(out)
}

fn mime_decode(s: &[u8], enc: &[u8], mode: i64) -> Result<Vec<u8>, Err> {
    let to = charset(enc).ok_or(Err::WrongCharset)?;
    let pl = Cs::Ascii;
    let strict = mode & MODE_STRICT != 0;
    let coe = mode & MODE_CONTINUE_ON_ERROR != 0;
    let mut out = Vec::new();
    let mut scan_stat = 0u32;
    let mut csname: Option<usize> = None;
    let mut cd: Option<Cs> = None;
    let mut encoded_text = 0usize;
    let mut encoded_text_len = 0usize;
    let mut encoded_word: Option<usize> = None;
    let mut spaces: Option<usize> = None;
    let mut b64 = true;
    let n = s.len();
    let mut p1: isize = 0;
    let mut str_left = n;
    macro_rules! ap {
        ($from:expr, $to:expr) => {
            append(&mut out, &s[$from..$to], pl, to)
        };
    }
    macro_rules! apc {
        ($c:expr) => {
            append(&mut out, &[$c], pl, to)
        };
    }
    while str_left > 0 {
        let p = p1 as usize;
        let c = s[p];
        let mut eos = false;
        let mut fall9 = false;
        match scan_stat {
            0 => match c {
                b'\r' => scan_stat = 7,
                b'\n' => scan_stat = 8,
                b'=' => {
                    encoded_word = Some(p);
                    scan_stat = 1;
                }
                b' ' | b'\t' => {
                    spaces = Some(p);
                    scan_stat = 11;
                }
                _ => {
                    if let Err(e) = apc!(c) {
                        if !coe {
                            return Err(e);
                        }
                    }
                    encoded_word = None;
                    if strict {
                        scan_stat = 12;
                    }
                }
            },
            1 => {
                if c != b'?' {
                    let mut q = p1;
                    if c == b'\r' || c == b'\n' {
                        q -= 1;
                    }
                    ap!(encoded_word.unwrap_or(0), (q + 1) as usize)?;
                    p1 = q;
                    encoded_word = None;
                    scan_stat = if strict { 12 } else { 0 };
                } else {
                    csname = Some(p + 1);
                    scan_stat = 2;
                }
            }
            2 => {
                match c {
                    b'?' => scan_stat = 3,
                    b'*' => scan_stat = 10,
                    b'\r' | b'\n' => {
                        let q = p1 - 1;
                        let _ = apc!(b'=');
                        let _ = apc!(b'?');
                        ap!(csname.unwrap_or(0), (q + 1) as usize)?;
                        p1 = q;
                        csname = None;
                        scan_stat = if strict { 12 } else { 0 };
                        str_left -= 1;
                        p1 += 1;
                        continue;
                    }
                    _ => {}
                }
                if scan_stat != 2 {
                    let Some(cs0) = csname else {
                        return Err(Err::Malformed);
                    };
                    let name = &s[cs0..p];
                    if name.len() > 79 {
                        if coe {
                            ap!(encoded_word.unwrap_or(0), p + 1)?;
                            encoded_word = None;
                            scan_stat = if strict { 12 } else { 0 };
                        } else {
                            return Err(Err::Malformed);
                        }
                    } else {
                        cd = charset(name);
                        if cd.is_none() {
                            if coe {
                                let mut qmarks = 2;
                                let mut q = p;
                                while qmarks > 0 && str_left > 1 {
                                    q += 1;
                                    if s[q] == b'?' {
                                        qmarks -= 1;
                                    }
                                    str_left -= 1;
                                }
                                if s.get(q + 1) == Some(&b'=') {
                                    q += 1;
                                    if str_left > 1 {
                                        str_left -= 1;
                                    }
                                }
                                ap!(encoded_word.unwrap_or(0), q + 1)?;
                                p1 = q as isize;
                                scan_stat = 12;
                            } else {
                                return Err(Err::WrongCharset);
                            }
                        }
                    }
                }
            }
            3 => match c {
                b'b' | b'B' => {
                    b64 = true;
                    scan_stat = 4;
                }
                b'q' | b'Q' => {
                    b64 = false;
                    scan_stat = 4;
                }
                _ => {
                    if coe {
                        ap!(encoded_word.unwrap_or(0), p + 1)?;
                        encoded_word = None;
                        scan_stat = if strict { 12 } else { 0 };
                    } else {
                        return Err(Err::Malformed);
                    }
                }
            },
            4 => {
                if c != b'?' {
                    if coe {
                        ap!(encoded_word.unwrap_or(0), p + 1)?;
                        encoded_word = None;
                        scan_stat = if strict { 12 } else { 0 };
                    } else {
                        return Err(Err::Malformed);
                    }
                } else {
                    encoded_text = p + 1;
                    scan_stat = 5;
                }
            }
            5 => {
                if c == b'?' {
                    encoded_text_len = p - encoded_text;
                    scan_stat = 6;
                }
            }
            7 => {
                if c == b'\n' {
                    scan_stat = 8;
                } else {
                    let _ = apc!(b'\r');
                    let _ = apc!(c);
                    scan_stat = 0;
                }
            }
            8 => {
                if c != b' ' && c != b'\t' {
                    p1 -= 1;
                    str_left = 1;
                } else {
                    if encoded_word.is_none() {
                        let _ = apc!(b' ');
                    }
                    spaces = None;
                    scan_stat = 11;
                }
            }
            6 => {
                if c != b'=' {
                    if coe {
                        ap!(encoded_word.unwrap_or(0), p + 1)?;
                        encoded_word = None;
                        scan_stat = if strict { 12 } else { 0 };
                    } else {
                        return Err(Err::Malformed);
                    }
                } else {
                    scan_stat = 9;
                    if str_left == 1 {
                        eos = true;
                        fall9 = true;
                    }
                }
            }
            9 => fall9 = true,
            10 => {
                if c == b'?' {
                    scan_stat = 3;
                }
            }
            11 => match c {
                b'\r' => scan_stat = 7,
                b'\n' => scan_stat = 8,
                b'=' => {
                    if let (Some(sp), None) = (spaces, encoded_word) {
                        let _ = ap!(sp, p);
                        spaces = None;
                    }
                    encoded_word = Some(p);
                    scan_stat = 1;
                }
                b' ' | b'\t' => {}
                _ => {
                    if let Some(sp) = spaces {
                        let _ = ap!(sp, p);
                        spaces = None;
                    }
                    let _ = apc!(c);
                    encoded_word = None;
                    scan_stat = if strict { 12 } else { 0 };
                }
            },
            12 => match c {
                b'\r' => scan_stat = 7,
                b'\n' => scan_stat = 8,
                b' ' | b'\t' => {
                    spaces = Some(p);
                    scan_stat = 11;
                }
                b'=' if !strict => {
                    encoded_word = Some(p);
                    scan_stat = 1;
                }
                _ => {
                    let _ = apc!(c);
                }
            },
            _ => {}
        }
        if fall9 {
            // case 9: choice point
            let mut handled = false;
            if !matches!(c, b'\r' | b'\n' | b' ' | b'\t') && !eos && strict {
                ap!(encoded_word.unwrap_or(0), p + 1)?;
                scan_stat = 12;
                handled = true;
            }
            if !handled {
                let text = &s[encoded_text..encoded_text + encoded_text_len];
                let decoded = if b64 {
                    base64_decode(text)
                } else {
                    qprint_decode(text)
                };
                match decoded {
                    None => {
                        if coe {
                            ap!(encoded_word.unwrap_or(0), p + 1)?;
                            encoded_word = None;
                            scan_stat = if strict { 12 } else { 0 };
                        } else {
                            return Err(Err::Unknown);
                        }
                    }
                    Some(d) => {
                        let mut ok = true;
                        if let Err(e) = append(&mut out, &d, cd.unwrap_or(Cs::Utf8), to) {
                            if coe {
                                let _ = ap!(encoded_word.unwrap_or(0), p);
                                encoded_word = None;
                                ok = false;
                            } else {
                                return Err(e);
                            }
                        }
                        let _ = ok;
                        if eos {
                            scan_stat = 0;
                        } else {
                            match c {
                                b'\r' => scan_stat = 7,
                                b'\n' => scan_stat = 8,
                                b'=' => scan_stat = 1,
                                b' ' | b'\t' => {
                                    spaces = Some(p);
                                    scan_stat = 11;
                                }
                                _ => {
                                    let _ = apc!(c);
                                    scan_stat = 12;
                                }
                            }
                        }
                    }
                }
            }
        }
        str_left -= 1;
        p1 += 1;
    }
    match scan_stat {
        0 | 8 | 11 | 12 => {}
        _ => {
            if coe {
                if scan_stat == 1 {
                    let _ = apc!(b'=');
                }
            } else {
                return Err(Err::Malformed);
            }
        }
    }
    Ok(out)
}

fn fn_iconv_mime_decode(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "iconv_mime_decode";
    let s = str_arg(host, args, 0, f, "string")?;
    let mode = int_arg_or(host, args, 1, f, "mode", 0)?;
    let Some(enc) = enc_arg(host, args, 2, f)? else {
        return Ok(Value::Bool(false));
    };
    Ok(match mime_decode(&s, &enc, mode) {
        Ok(b) => Value::str(b),
        Err(e) => {
            show_error(host, f, e, &enc, b"???")?;
            Value::Bool(false)
        }
    })
}

/// libiconv's `//TRANSLIT` spelling of a code point the target lacks (the
/// common Latin letters and symbols); `None` = not transliterable.
fn translit(cp: u32) -> Option<&'static str> {
    Some(match cp {
        0xe0 => "`a",
        0xe1 => "'a",
        0xe2 => "^a",
        0xe3 => "~a",
        0xe4 => "\"a",
        0xe5 => "a",
        0xe8 => "`e",
        0xe9 => "'e",
        0xea => "^e",
        0xeb => "\"e",
        0xec => "`i",
        0xed => "'i",
        0xee => "^i",
        0xef => "\"i",
        0xf2 => "`o",
        0xf3 => "'o",
        0xf4 => "^o",
        0xf5 => "~o",
        0xf6 => "\"o",
        0xf8 => "o",
        0xf9 => "`u",
        0xfa => "'u",
        0xfb => "^u",
        0xfc => "\"u",
        0xfd => "'y",
        0xff => "\"y",
        0xf1 => "~n",
        0xe7 => "c",
        0xc0 => "`A",
        0xc1 => "'A",
        0xc2 => "^A",
        0xc3 => "~A",
        0xc4 => "\"A",
        0xc5 => "A",
        0xc8 => "`E",
        0xc9 => "'E",
        0xca => "^E",
        0xcb => "\"E",
        0xcc => "`I",
        0xcd => "'I",
        0xce => "^I",
        0xcf => "\"I",
        0xd2 => "`O",
        0xd3 => "'O",
        0xd4 => "^O",
        0xd5 => "~O",
        0xd6 => "\"O",
        0xd8 => "O",
        0xd9 => "`U",
        0xda => "'U",
        0xdb => "^U",
        0xdc => "\"U",
        0xdd => "'Y",
        0xd1 => "~N",
        0xc7 => "C",
        0xdf => "ss",
        0xe6 => "ae",
        0xc6 => "AE",
        0x153 => "oe",
        0x152 => "OE",
        0x20ac => "EUR",
        0xa3 => "lb",
        0xa9 => "(C)",
        0xae => "(R)",
        0x2122 => "TM",
        0x2018 | 0x2019 => "'",
        0x201c | 0x201d => "\"",
        0x2013 | 0x2014 => "-",
        0x2026 => "...",
        0xa0 => " ",
        0xb2 => "^2",
        0xb3 => "^3",
        0xb9 => "^1",
        _ => return None,
    })
}

/// `iconv($from_encoding, $to_encoding, $string)` (php_iconv_string): the
/// `//IGNORE` / `//TRANSLIT` suffixes of the target are honoured; failures
/// raise `_php_iconv_show_error`'s diagnostics and return false.
fn fn_iconv(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let from_name = str_arg(host, args, 0, "iconv", "from_encoding")?;
    let to_name = str_arg(host, args, 1, "iconv", "to_encoding")?;
    let data = str_arg(host, args, 2, "iconv", "string")?;
    let upper = String::from_utf8_lossy(&to_name).to_ascii_uppercase();
    let ignore = upper.contains("//IGNORE");
    let trans = upper.contains("//TRANSLIT");
    let (Some(from), Some(to)) = (charset(&from_name), charset(&to_name)) else {
        show_error(host, "iconv", Err::WrongCharset, &to_name, &from_name)?;
        return Ok(Value::Bool(false));
    };
    // Decode, skipping illegal input under //IGNORE.
    let mut cps = Vec::with_capacity(data.len());
    let mut rest: &[u8] = &data;
    loop {
        let (got, stop) = decode(from, rest);
        match stop {
            Stop::End => {
                cps.extend(got);
                break;
            }
            Stop::Illegal if ignore => {
                let used = encode(from, &got).map_or(0, |b| b.len());
                cps.extend(got);
                rest = rest.get(used + 1..).unwrap_or(&[]);
            }
            other => {
                show_error(
                    host,
                    "iconv",
                    Err::from_stop(other).unwrap_or(Err::Unknown),
                    &to_name,
                    &from_name,
                )?;
                return Ok(Value::Bool(false));
            }
        }
    }
    let mut out = Vec::with_capacity(cps.len());
    for &cp in &cps {
        if let Some(b) = encode(to, &[cp]) {
            out.extend_from_slice(&b);
            continue;
        }
        if trans {
            match translit(cp)
                .and_then(|t| encode(to, &t.chars().map(|c| c as u32).collect::<Vec<_>>()))
            {
                Some(b) => out.extend_from_slice(&b),
                None if ignore => {}
                None => out.push(b'?'),
            }
            continue;
        }
        if ignore {
            continue;
        }
        show_error(host, "iconv", Err::IllegalSeq, &to_name, &from_name)?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::str(out))
}

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
    put(m, "iconv", fn_iconv, 3, Some(3));
    put(m, "iconv_strlen", fn_iconv_strlen, 1, Some(2));
    put(m, "iconv_substr", fn_iconv_substr, 2, Some(4));
    put(m, "iconv_strpos", fn_iconv_strpos, 2, Some(4));
    put(m, "iconv_strrpos", fn_iconv_strrpos, 2, Some(3));
    put(m, "iconv_mime_decode", fn_iconv_mime_decode, 1, Some(3));
}

pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("ICONV_MIME_DECODE_STRICT", Value::Int(MODE_STRICT)),
        (
            "ICONV_MIME_DECODE_CONTINUE_ON_ERROR",
            Value::Int(MODE_CONTINUE_ON_ERROR),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag_test::run;

    fn s(b: &[u8]) -> Value {
        Value::str(b.to_vec())
    }

    /// `(result-as-debug, diagnostics)`.
    fn go(f: crate::NativeFn, args: &[Value]) -> (String, Vec<String>) {
        let (r, d) = run(f, args);
        let r = match r {
            Ok(Value::Bool(b)) => format!("bool({b})"),
            Ok(Value::Int(i)) => format!("int({i})"),
            Ok(Value::Str(b)) => format!("string({:?})", String::from_utf8_lossy(b.as_bytes())),
            Ok(_) => "other".into(),
            Err(Thrown::Error { class, message }) => format!("{class}: {message}"),
            Err(_) => "thrown".into(),
        };
        (r, d)
    }

    const ILLEGAL: &str = "Notice: iconv_strlen(): Detected an illegal character in input string";

    #[test]
    fn strlen_diagnostics() {
        assert_eq!(
            go(fn_iconv_strlen, &[s(b"h\xe9llo")]),
            ("bool(false)".into(), vec![ILLEGAL.to_string()])
        );
        assert_eq!(
            go(fn_iconv_strlen, &[s(b"h\xc3")]),
            (
                "bool(false)".into(),
                vec![
                    "Notice: iconv_strlen(): Detected an incomplete multibyte character in input string"
                        .to_string()
                ]
            )
        );
        assert_eq!(
            go(fn_iconv_strlen, &[s(b"abc"), s(b"bogus")]),
            (
                "bool(false)".into(),
                vec![
                    "Warning: iconv_strlen(): Wrong encoding, conversion from \"bogus\" to \"UCS-4LE\" is not allowed"
                        .to_string()
                ]
            )
        );
        assert_eq!(
            go(fn_iconv_strlen, &[s(b"abc"), s(&[b'x'; 64])]),
            (
                "bool(false)".into(),
                vec![
                    "Warning: iconv_strlen(): Encoding parameter exceeds the maximum allowed length of 64 characters"
                        .to_string()
                ]
            )
        );
        assert_eq!(go(fn_iconv_strlen, &[s(b"abc")]), ("int(3)".into(), vec![]));
    }

    #[test]
    fn substr_diagnostics() {
        assert_eq!(
            go(fn_iconv_substr, &[s(b"h\xe9llo"), Value::Int(1)]).1,
            vec!["Notice: iconv_substr(): Detected an illegal character in input string"]
        );
        assert_eq!(
            go(fn_iconv_substr, &[s(b"abc"), Value::Int(1), Value::Int(1), s(b"bogus")]).1,
            vec!["Warning: iconv_substr(): Wrong encoding, conversion from \"bogus\" to \"UCS-4LE\" is not allowed"]
        );
    }

    #[test]
    fn strpos_lookahead() {
        let n = |h: &[u8], nd: &[u8]| go(fn_iconv_strpos, &[s(h), s(nd)]);
        let ill = "Notice: iconv_strpos(): Detected an illegal character in input string";
        let inc =
            "Notice: iconv_strpos(): Detected an incomplete multibyte character in input string";
        assert_eq!(n(b"ab\xc3", b"b"), ("bool(false)".into(), vec![inc.into()]));
        assert_eq!(n(b"ab\xc3", b"a"), ("int(0)".into(), vec![]));
        assert_eq!(n(b"abc\xff", b"b"), ("int(1)".into(), vec![]));
        assert_eq!(
            n(b"abc\xff", b"c"),
            ("bool(false)".into(), vec![ill.into()])
        );
        assert_eq!(
            n(b"abc\xff", b"z"),
            ("bool(false)".into(), vec![ill.into()])
        );
        assert_eq!(n(b"ab\xff", b"b"), ("bool(false)".into(), vec![ill.into()]));
        assert_eq!(
            go(
                fn_iconv_strpos,
                &[s(b"ab\xffcd"), s(b"b"), Value::Int(0), s(b"ASCII")]
            ),
            ("bool(false)".into(), vec![ill.into()])
        );
        // err suppresses the out-of-bounds ValueError
        assert_eq!(
            go(fn_iconv_strpos, &[s(b"abc\xff"), s(b"a"), Value::Int(10)]),
            ("bool(false)".into(), vec![ill.into()])
        );
        // bad input at the very start: nothing decoded, no error
        assert_eq!(
            go(fn_iconv_strpos, &[s(b"\xff"), s(b"a"), Value::Int(3)]).0,
            "ValueError: iconv_strpos(): Argument #3 ($offset) must be contained in argument #1 ($haystack)"
        );
        let r = |h: &[u8], nd: &[u8]| go(fn_iconv_strrpos, &[s(h), s(nd)]);
        let rill = "Notice: iconv_strrpos(): Detected an illegal character in input string";
        assert_eq!(
            r(b"a\xffbc", b"a"),
            ("bool(false)".into(), vec![rill.into()])
        );
        assert_eq!(
            r(b"abcdef\xff", b"a"),
            ("bool(false)".into(), vec![rill.into()])
        );
        assert_eq!(r(b"\xffab", b"a"), ("bool(false)".into(), vec![]));
        assert_eq!(
            r(b"abc", b"\xff"),
            ("bool(false)".into(), vec![rill.into()])
        );
        assert_eq!(
            go(fn_iconv_strpos, &[s(b"abc"), s(b"b"), Value::Int(0), s(b"bogus")]).1,
            vec!["Warning: iconv_strpos(): Wrong encoding, conversion from \"bogus\" to \"UCS-4LE\" is not allowed"]
        );
    }

    #[test]
    fn mime_decode_diagnostics() {
        let m = |x: &[u8]| go(fn_iconv_mime_decode, &[s(x)]);
        assert_eq!(
            m(b"=?UTF-8?X?abc?="),
            (
                "bool(false)".into(),
                vec!["Warning: iconv_mime_decode(): Malformed string".into()]
            )
        );
        assert_eq!(
            m(b"=?FOO?Q?abc?= x").1,
            vec!["Warning: iconv_mime_decode(): Wrong encoding, conversion from \"???\" to \"UTF-8\" is not allowed"]
        );
        assert_eq!(
            go(fn_iconv_mime_decode, &[s(b""), Value::Int(0), s(b"bogus")]).1,
            vec!["Warning: iconv_mime_decode(): Wrong encoding, conversion from \"???\" to \"bogus\" is not allowed"]
        );
        assert_eq!(
            m(b"=?UTF-8?Q?\xff?=").1,
            vec!["Notice: iconv_mime_decode(): Detected an illegal character in input string"]
        );
        assert_eq!(
            m(b"=?UTF-8?Q?=ZZ?=").1,
            vec![format!(
                "Notice: iconv_mime_decode(): Unknown error ({EILSEQ})"
            )]
        );
    }
}
