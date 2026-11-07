//! The iconv extension: a port of php-src's ext/iconv/iconv.c (PHP License
//! 3.01) over the system iconv(3) php links ([`sys`]).
//!
//! What a conversion produces, where it fails, and what //TRANSLIT and
//! //IGNORE do come from the platform's iconv, so they match the php on the
//! same machine and differ between macOS's libiconv and glibc (UTF-16 and
//! UCS-2 byte order, transliterations, which characters a charset has).
//! php's control flow is kept: the character-at-a-time UCS-4LE loops of
//! iconv_strlen, iconv_substr and iconv_strpos (with the library's
//! lookahead), the MIME encoder and state machine, and php's notices.
//!
//! The convert.iconv.* stream filter is not ported: rhp has no stream
//! filters.

pub(crate) mod iconv_filter;
mod sys;

use std::ffi::{CStr, CString};

use libc::{E2BIG, EBADF, EILSEQ, EINVAL};
use rhp_values::{ArrayKey, PhpArray, PhpStr, Value};

use crate::zend::zpp::{deref, long_arg, opt_long_arg, opt_str_arg, str_arg, value_error};
use crate::{warn, Host, OutputHandlerAlias, OutputHandlerCtx, Thrown};
use sys::{errno, set_errno, Cd, NulCopy, FAILED};

/// ICONV_CSNMAXLEN.
const CSNMAXLEN: usize = 64;
/// ICONV_UCS4_ENCODING: the generic superset the counting loops convert to.
const SUPERSET: &CStr = c"UCS-4LE";
const SUPERSET_NBYTES: usize = 4;
/// ICONV_ASCII_ENCODING.
const ASCII: &CStr = c"ASCII";

const MIME_DECODE_STRICT: i64 = 1;
const MIME_DECODE_CONTINUE_ON_ERROR: i64 = 2;

/// php_iconv_err_t.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum IconvErr {
    Success,
    Converter,
    WrongCharset,
    TooBig,
    IllegalSeq,
    IllegalChar,
    Unknown,
    Malformed,
    Alloc,
    OutOfBounds,
}

use IconvErr::*;

/// The error for an `iconv_open` that returned `(iconv_t)-1`.
fn open_error() -> IconvErr {
    if errno() == EINVAL {
        WrongCharset
    } else {
        Converter
    }
}

/// A charset name as php hands it to iconv_open: a C string, so it ends at
/// the first NUL.
fn cstr(b: &[u8]) -> CString {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    CString::new(&b[..end]).unwrap_or_default()
}

/// A `string` argument as php holds it, with a NUL after the bytes (see
/// [`Cd::conv`]). Derefs to the bytes.
enum StrArg {
    /// The caller's string, which php uses in place.
    Passed(NulCopy),
    /// A string converted for the call, which php allocates too.
    Converted(Vec<u8>),
}

impl std::ops::Deref for StrArg {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            StrArg::Passed(c) => c,
            StrArg::Converted(v) => &v[..v.len() - 1],
        }
    }
}

fn zstr_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<StrArg, Thrown> {
    if let Some(Value::Str(s)) = args.get(i).map(deref) {
        return Ok(StrArg::Passed(NulCopy::new(s.as_bytes())));
    }
    let mut v = str_arg(host, args, i, fname, pname)?;
    v.push(0);
    Ok(StrArg::Converted(v))
}

/// A `Z_PARAM_LONG` parameter (absent: 0). The VM has already raised the
/// deprecation for a null.
fn long_param(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    pname: &str,
) -> Result<i64, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(0),
        Some(_) => long_arg(host, args, i, fname, pname),
    }
}

/// ZEND_HAS_OOM(): an out-of-memory error is pending.
fn has_oom() -> bool {
    rhp_values::mem::state().attention()
}

/// Blocks above this are huge blocks in zend_mm (ZEND_MM_MAX_LARGE_SIZE):
/// growing one is checked against the limit for the growth only.
const MAX_LARGE: usize = rhp_values::mem::CHUNK_SIZE - 4096;

/// A fallible realloc of a php string from `old` to `new` bytes.
fn fallible_grow(old: usize, new: usize) -> bool {
    use rhp_values::mem::{reserve, reserve_str, zstr_size};
    if zstr_size(old) > MAX_LARGE {
        reserve(zstr_size(new).saturating_sub(zstr_size(old)))
    } else {
        reserve_str(new)
    }
}

// ---------------------------------------------------------------------------
// smart_str
// ---------------------------------------------------------------------------

/// php's smart_str. `s` stays `None` until something is appended: php tells
/// the two apart (iconv_substr, iconv_mime_decode_headers).
#[derive(Default)]
struct SmartStr {
    s: Option<Vec<u8>>,
    /// The capacity php allocated (`a`).
    a: usize,
}

/// SMART_STR_OVERHEAD (no ZEND_MM_OVERHEAD in release builds).
const SMART_STR_OVERHEAD: usize = 24 + 1;
const SMART_STR_START_LEN: usize = 256 - SMART_STR_OVERHEAD;

fn smart_str_new_len(len: usize) -> usize {
    (len.saturating_add(SMART_STR_OVERHEAD + 4095) & !4095) - SMART_STR_OVERHEAD
}

impl SmartStr {
    /// smart_str_falloc: room for `len` more bytes. `None` when a fallible
    /// allocation fails.
    fn falloc(&mut self, len: usize, fallible: bool) -> Option<usize> {
        let cur = self.s.as_ref().map(Vec::len);
        let total = cur.unwrap_or(0).saturating_add(len);
        if cur.is_none() || total >= self.a {
            let a = if cur.is_none() && total <= SMART_STR_START_LEN {
                SMART_STR_START_LEN
            } else {
                smart_str_new_len(total)
            };
            if fallible {
                let ok = match cur {
                    None => rhp_values::mem::reserve_str(a),
                    Some(_) => fallible_grow(self.a, a),
                };
                if !ok {
                    return None;
                }
            }
            self.a = a;
            let v = self.s.get_or_insert_with(Vec::new);
            v.reserve_exact(a.saturating_sub(v.len()));
        }
        Some(total)
    }

    fn appendl(&mut self, b: &[u8]) {
        self.falloc(b.len(), false);
        if let Some(v) = self.s.as_mut() {
            v.extend_from_slice(b);
        }
    }

    fn appendc(&mut self, c: u8) {
        self.appendl(&[c]);
    }

    /// smart_str_extract: "" when nothing was appended.
    fn extract(self) -> Vec<u8> {
        self.s.unwrap_or_default()
    }
}

/// _php_iconv_appendl: convert `s` (`None`: the shift sequence back to the
/// initial state) onto `d`.
fn appendl(d: &mut SmartStr, s: Option<&[u8]>, cd: Option<&Cd>) -> IconvErr {
    let Some(cd) = cd else {
        // iconv on `(iconv_t)-1`.
        set_errno(EBADF);
        return Unknown;
    };
    let mut buf_growth = 128usize;
    match s {
        Some(mut inp) => {
            while !inp.is_empty() {
                let out_left = buf_growth;
                if d.falloc(out_left, true).is_none() {
                    return Alloc;
                }
                let Some(v) = d.s.as_mut() else {
                    return Alloc;
                };
                let len = v.len();
                v.resize(len + out_left, 0);
                let mut pos = len;
                if cd.conv(&mut inp, v, &mut pos) == FAILED {
                    let e = match errno() {
                        EINVAL => Some(IllegalChar),
                        EILSEQ => Some(IllegalSeq),
                        E2BIG => None,
                        _ => Some(Unknown),
                    };
                    if let Some(e) = e {
                        v.truncate(len);
                        return e;
                    }
                }
                v.truncate(pos);
                buf_growth <<= 1;
            }
        }
        None => loop {
            let out_left = buf_growth;
            if d.falloc(out_left, true).is_none() {
                return Alloc;
            }
            let Some(v) = d.s.as_mut() else {
                return Alloc;
            };
            let len = v.len();
            v.resize(len + out_left, 0);
            let mut pos = len;
            if cd.flush(v, &mut pos) == 0 {
                v.truncate(pos);
                break;
            }
            if errno() != E2BIG {
                v.truncate(len);
                return Unknown;
            }
            v.truncate(pos);
            buf_growth <<= 1;
        },
    }
    Success
}

/// _php_iconv_appendc. php passes the address of a local char.
fn appendc(d: &mut SmartStr, c: u8, cd: Option<&Cd>) -> IconvErr {
    let b = [c, 0];
    appendl(d, Some(&b[..1]), cd)
}

// ---------------------------------------------------------------------------
// php_iconv_string and the counting functions
// ---------------------------------------------------------------------------

/// Whether the linked iconv fails conversions under //IGNORE
/// (ICONV_BROKEN_IGNORE): php's configure check, run once against it.
fn broken_ignore() -> bool {
    static BROKEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *BROKEN.get_or_init(|| {
        let saved = errno();
        let works = Cd::open(c"UTF-8//IGNORE", c"UTF-8").is_some_and(|cd| {
            let input = b"\xC3\xC3\xC3\xB8\0";
            let mut inp = &input[..4];
            let mut out = vec![0u8; 4096];
            let mut pos = 0;
            cd.conv(&mut inp, &mut out, &mut pos) != FAILED
        });
        set_errno(saved);
        !works
    })
}

/// _php_check_ignore.
fn check_ignore(charset: &CStr) -> bool {
    if !broken_ignore() {
        return false;
    }
    let c = charset.to_bytes();
    (c.len() >= 9 && c.ends_with(b"//IGNORE"))
        || (c.len() >= 19 && c.ends_with(b"//IGNORE//TRANSLIT"))
}

/// php_iconv_string: the output, also on an illegal or incomplete sequence
/// (what was converted up to it).
fn iconv_string(
    input: &[u8],
    out_charset: &CStr,
    in_charset: &CStr,
) -> (IconvErr, Option<Vec<u8>>) {
    let ignore_ilseq = check_ignore(out_charset);
    let _locale = sys::Locale::enter();
    let Some(cd) = Cd::open(out_charset, in_charset) else {
        return (open_error(), None);
    };
    let in_len = input.len();
    let mut inp = input;
    let mut bsz = in_len.saturating_add(32);
    let mut out_size = 0usize;
    let mut result = 0usize;
    let mut buf = Vec::new();
    'fail: {
        if !rhp_values::mem::reserve_str(bsz) {
            result = FAILED;
            set_errno(0);
            break 'fail;
        }
        buf = vec![0u8; bsz];
        while !inp.is_empty() {
            result = cd.conv(&mut inp, &mut buf, &mut out_size);
            if result == FAILED {
                if ignore_ilseq && errno() == EILSEQ {
                    if inp.len() <= 1 {
                        result = 0;
                    } else {
                        set_errno(0);
                        inp = &inp[1..];
                        continue;
                    }
                }
                if errno() == E2BIG && !inp.is_empty() {
                    // The converted string is longer than the buffer.
                    let new = bsz.saturating_add(in_len);
                    if !fallible_grow(bsz, new) {
                        result = FAILED;
                        set_errno(0);
                        break 'fail;
                    }
                    bsz = new;
                    buf.resize(bsz, 0);
                    continue;
                }
            }
            break;
        }
        if result != FAILED {
            // The shift-out sequence.
            loop {
                result = cd.flush(&mut buf, &mut out_size);
                if result != FAILED {
                    break;
                }
                if errno() != E2BIG {
                    break;
                }
                if !fallible_grow(bsz, bsz + 16) {
                    result = FAILED;
                    set_errno(0);
                    break 'fail;
                }
                bsz += 16;
                buf.resize(bsz, 0);
            }
        }
    }
    drop(cd);
    let mut retval = Success;
    if result == FAILED {
        retval = match errno() {
            EINVAL => IllegalChar,
            EILSEQ => IllegalSeq,
            E2BIG => TooBig,
            _ => return (Unknown, None),
        };
    }
    buf.truncate(out_size);
    (retval, Some(buf))
}

/// _php_iconv_strlen.
fn iconv_strlen(s: &[u8], enc: &CStr) -> (IconvErr, usize) {
    let _locale = sys::Locale::enter();
    let Some(cd) = Cd::open(SUPERSET, enc) else {
        return (open_error(), usize::MAX);
    };
    let mut buf = [0u8; SUPERSET_NBYTES * 2];
    set_errno(0);
    let mut inp = s;
    let mut cnt = 0usize;
    let mut more = !s.is_empty();
    while more {
        let mut pos = 0;
        more = !inp.is_empty();
        if more {
            cd.conv(&mut inp, &mut buf, &mut pos);
        } else {
            cd.flush(&mut buf, &mut pos);
        }
        if pos == 0 {
            break;
        }
        cnt += pos / SUPERSET_NBYTES;
    }
    let r = match errno() {
        EINVAL => (IllegalChar, usize::MAX),
        EILSEQ => (IllegalSeq, usize::MAX),
        0 | E2BIG => (Success, cnt),
        _ => (Unknown, usize::MAX),
    };
    drop(cd);
    r
}

/// _php_iconv_substr.
fn iconv_substr(s: &[u8], mut offset: i64, mut len: i64, enc: &CStr) -> (IconvErr, SmartStr) {
    let mut pretval = SmartStr::default();
    let (err, total_len) = iconv_strlen(s, enc);
    if err != Success {
        return (err, pretval);
    }
    let total = total_len as i64;
    if offset < 0 {
        offset = offset.wrapping_add(total);
        if offset < 0 {
            offset = 0;
        }
    } else if offset as u64 > total_len as u64 {
        offset = total;
    }
    if len < 0 {
        len = len.wrapping_add(total - offset);
        if len < 0 {
            len = 0;
        }
    } else if len as u64 > total_len as u64 {
        len = total;
    }
    if offset.wrapping_add(len) as u64 > total_len as u64 {
        len = total - offset;
    }
    if len == 0 {
        pretval.appendl(b"");
        return (Success, pretval);
    }
    let _locale = sys::Locale::enter();
    let Some(cd1) = Cd::open(SUPERSET, enc) else {
        return (open_error(), pretval);
    };
    let mut cd2: Option<Cd> = None;
    let mut err = Success;
    set_errno(0);
    let mut inp = s;
    let mut cnt = 0i64;
    let mut more = !s.is_empty() && len > 0;
    // php converts into a 4-byte local buffer.
    let mut buf = [0u8; SUPERSET_NBYTES * 2];
    while more {
        let mut pos = 0;
        more = !inp.is_empty() && len > 0;
        let out = &mut buf[..SUPERSET_NBYTES];
        if more {
            cd1.conv(&mut inp, out, &mut pos);
        } else {
            cd1.flush(out, &mut pos);
        }
        if pos == 0 {
            break;
        }
        if cnt >= offset {
            if cd2.is_none() {
                cd2 = Cd::open(enc, SUPERSET);
                if cd2.is_none() {
                    err = open_error();
                    break;
                }
            }
            if appendl(&mut pretval, Some(&buf[..SUPERSET_NBYTES]), cd2.as_ref()) != Success {
                break;
            }
            len -= 1;
        }
        cnt += 1;
    }
    match errno() {
        EINVAL => err = IllegalChar,
        EILSEQ => err = IllegalSeq,
        _ => {}
    }
    if err == Success {
        if let Some(cd2) = cd2.as_ref() {
            appendl(&mut pretval, None, Some(cd2));
        }
    }
    drop(cd1);
    drop(cd2);
    (err, pretval)
}

/// _php_iconv_strpos: the character offset of the first (last, with
/// `reverse`) match.
fn iconv_strpos(
    haystk: &[u8],
    ndl: &[u8],
    offset: usize,
    enc: &CStr,
    reverse: bool,
) -> (IconvErr, Option<usize>) {
    let (err, ndl_buf) = iconv_string(ndl, SUPERSET, enc);
    if err != Success {
        return (err, None);
    }
    // A NUL and zeros follow, as php's comparisons may read past the end.
    let mut ndl_buf = ndl_buf.unwrap_or_default();
    let ndl_len = ndl_buf.len();
    ndl_buf.extend_from_slice(&[0; SUPERSET_NBYTES * 2]);
    let _locale = sys::Locale::enter();
    let Some(cd) = Cd::open(SUPERSET, enc) else {
        return (open_error(), None);
    };
    let at = |i: usize| {
        ndl_buf
            .get(i..i + SUPERSET_NBYTES)
            .unwrap_or(&[0; SUPERSET_NBYTES])
    };
    let mut err = Success;
    let mut ret = None;
    let mut ndl_p = 0usize;
    let mut ndl_left = ndl_len;
    let mut match_ofs = usize::MAX;
    let mut buf = [0u8; SUPERSET_NBYTES];
    let mut inp = haystk;
    let mut cnt = 0usize;
    let mut more = !haystk.is_empty();
    while more {
        let mut pos = 0;
        more = !inp.is_empty();
        let r = if more {
            cd.conv(&mut inp, &mut buf, &mut pos)
        } else {
            cd.flush(&mut buf, &mut pos)
        };
        if pos == 0 {
            break;
        }
        if r == FAILED {
            match errno() {
                EINVAL => err = IllegalChar,
                EILSEQ => err = IllegalSeq,
                E2BIG => {}
                _ => err = Unknown,
            }
        }
        if cnt >= offset {
            if buf == at(ndl_p) {
                if match_ofs == usize::MAX {
                    match_ofs = cnt;
                }
                ndl_p += SUPERSET_NBYTES;
                ndl_left = ndl_left.wrapping_sub(SUPERSET_NBYTES);
                if ndl_left == 0 {
                    ret = Some(match_ofs);
                    if reverse {
                        // Searching backward: look for a later match.
                        ndl_p = 0;
                        ndl_left = ndl_len;
                        match_ofs = usize::MAX;
                    } else {
                        break;
                    }
                }
            } else {
                let mut i = 0usize;
                let mut j = SUPERSET_NBYTES;
                let lim = ndl_p;
                while j < lim {
                    if at(j) == at(i) {
                        i += SUPERSET_NBYTES;
                    } else {
                        j -= i;
                        i = 0;
                    }
                    j += SUPERSET_NBYTES;
                }
                if buf == at(i) {
                    match_ofs = match_ofs.wrapping_add((lim - i) / SUPERSET_NBYTES);
                    i += SUPERSET_NBYTES;
                    ndl_p = i;
                    ndl_left = ndl_len.wrapping_sub(i);
                } else {
                    match_ofs = usize::MAX;
                    ndl_p = 0;
                    ndl_left = ndl_len;
                }
            }
        }
        cnt += 1;
    }
    drop(cd);
    if err == Success && offset > cnt {
        return (OutOfBounds, None);
    }
    (err, ret)
}

// ---------------------------------------------------------------------------
// MIME
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scheme {
    Base64,
    QPrint,
}

/// _php_iconv_mime_encode's quoted-printable cost of each byte.
const QP_TABLE: [u8; 256] = {
    let mut t = [3u8; 256];
    let mut c = 0x21;
    while c < 0x7f {
        t[c] = 1;
        c += 1;
    }
    t[b'=' as usize] = 3;
    t[b'?' as usize] = 3;
    t[b'_' as usize] = 3;
    t
};

/// A zeroed buffer of `n` bytes, or `None` when the system has no memory
/// for it (php's emalloc then fails the request too).
fn zeroed(n: usize) -> Option<Vec<u8>> {
    if n == 0 {
        return Some(Vec::new());
    }
    let layout = std::alloc::Layout::array::<u8>(n).ok()?;
    // SAFETY: a non-zero layout; the zeroed block becomes the Vec's buffer
    // with the same layout and length.
    unsafe {
        let p = std::alloc::alloc_zeroed(layout);
        if p.is_null() {
            return None;
        }
        Some(Vec::from_raw_parts(p, n, n))
    }
}

/// _php_iconv_mime_encode.
#[allow(clippy::too_many_arguments)]
fn mime_encode(
    pretval: &mut SmartStr,
    fname: &[u8],
    fval: &[u8],
    max_line_len: usize,
    lfchars: &[u8],
    enc_scheme: Scheme,
    out_charset: &CStr,
    enc: &CStr,
) -> IconvErr {
    let out_charset_len = out_charset.to_bytes().len();
    let lfchars_len = lfchars.len();
    if fname.len().wrapping_add(2) >= max_line_len
        || out_charset_len.wrapping_add(12) >= max_line_len
    {
        // The field name is too long.
        return TooBig;
    }
    let _locale = sys::Locale::enter();
    let Some(cd_pl) = Cd::open(ASCII, enc) else {
        return open_error();
    };
    let Some(cd) = Cd::open(out_charset, enc) else {
        return open_error();
    };
    // safe_femalloc(1, max_line_len, 5, fallible)
    if !rhp_values::mem::reserve_safe(1, max_line_len, 5) {
        return Alloc;
    }
    let Some(mut buf) = max_line_len.checked_add(5).and_then(zeroed) else {
        return Alloc;
    };
    let mut char_cnt = max_line_len;
    appendl(pretval, Some(fname), Some(&cd_pl));
    char_cnt = char_cnt.wrapping_sub(fname.len());
    pretval.appendl(b": ");
    char_cnt = char_cnt.wrapping_sub(2);
    let mut inp = fval;
    loop {
        let encoded_word_min_len =
            7 + out_charset_len + if enc_scheme == Scheme::Base64 { 4 } else { 3 };
        if char_cnt < encoded_word_min_len + lfchars_len + 1 {
            // lfchars must be encoded in ASCII here.
            pretval.appendl(lfchars);
            pretval.appendc(b' ');
            char_cnt = max_line_len - 1;
        }
        pretval.appendl(b"=?");
        char_cnt = char_cnt.wrapping_sub(2);
        pretval.appendl(out_charset.to_bytes());
        char_cnt = char_cnt.wrapping_sub(out_charset_len);
        pretval.appendc(b'?');
        char_cnt = char_cnt.wrapping_sub(1);
        match enc_scheme {
            Scheme::Base64 => {
                let mut out_reserved = 4usize;
                pretval.appendc(b'B');
                pretval.appendc(b'?');
                char_cnt = char_cnt.wrapping_sub(2);
                let ini_in = inp;
                let prev_in_left = inp.len();
                let out_size = char_cnt.wrapping_sub(2) / 4 * 3;
                let mut pos;
                loop {
                    pos = 0;
                    if out_size <= out_reserved {
                        return TooBig;
                    }
                    let limit = (out_size - out_reserved).min(buf.len());
                    if cd.conv(&mut inp, &mut buf[..limit], &mut pos) == FAILED {
                        match errno() {
                            EINVAL => return IllegalChar,
                            EILSEQ => return IllegalSeq,
                            E2BIG => {
                                if prev_in_left == inp.len() {
                                    return TooBig;
                                }
                            }
                            _ => return Unknown,
                        }
                    }
                    let limit = out_size.min(buf.len());
                    if cd.flush(&mut buf[..limit], &mut pos) == FAILED {
                        if errno() != E2BIG {
                            return Unknown;
                        }
                    } else {
                        break;
                    }
                    if cd.reset() == FAILED {
                        return Unknown;
                    }
                    out_reserved += 4;
                    inp = ini_in;
                }
                let encoded = crate::ext::standard::base64::base64_encode(&buf[..pos]);
                if char_cnt < encoded.len() {
                    // Something went wrong.
                    return Unknown;
                }
                pretval.appendl(&encoded);
                char_cnt -= encoded.len();
                pretval.appendl(b"?=");
                char_cnt = char_cnt.wrapping_sub(2);
            }
            Scheme::QPrint => {
                pretval.appendc(b'Q');
                pretval.appendc(b'?');
                char_cnt = char_cnt.wrapping_sub(2);
                let ini_in = inp;
                let prev_in_left = inp.len();
                let mut out_size = char_cnt.wrapping_sub(2);
                let mut pos = 0;
                while out_size > 0 {
                    pos = 0;
                    let limit = out_size.min(buf.len());
                    if cd.conv(&mut inp, &mut buf[..limit], &mut pos) == FAILED {
                        match errno() {
                            EINVAL => return IllegalChar,
                            EILSEQ => return IllegalSeq,
                            E2BIG => {
                                if prev_in_left == inp.len() {
                                    return Unknown;
                                }
                            }
                            _ => return Unknown,
                        }
                    }
                    if cd.flush(&mut buf[..limit], &mut pos) == FAILED && errno() != E2BIG {
                        return Unknown;
                    }
                    let nbytes_required: usize = buf[..pos]
                        .iter()
                        .map(|&b| QP_TABLE[b as usize] as usize)
                        .sum();
                    if nbytes_required <= char_cnt.wrapping_sub(2) {
                        break;
                    }
                    out_size = out_size
                        .wrapping_sub((nbytes_required - char_cnt.wrapping_sub(2)).div_ceil(3));
                    inp = ini_in;
                }
                const QP_DIGITS: &[u8; 16] = b"0123456789ABCDEF";
                for &b in &buf[..pos] {
                    if QP_TABLE[b as usize] == 1 {
                        pretval.appendc(b);
                        char_cnt = char_cnt.wrapping_sub(1);
                    } else {
                        pretval.appendc(b'=');
                        pretval.appendc(QP_DIGITS[(b >> 4) as usize]);
                        pretval.appendc(QP_DIGITS[(b & 0x0f) as usize]);
                        char_cnt = char_cnt.wrapping_sub(3);
                    }
                }
                pretval.appendl(b"?=");
                char_cnt = char_cnt.wrapping_sub(2);
                if cd.reset() == FAILED {
                    return Unknown;
                }
            }
        }
        if inp.is_empty() || has_oom() {
            break;
        }
    }
    Success
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
                // The next character must be a hexadecimal digit.
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
                // A soft line break.
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

/// _php_iconv_mime_decode over `s`, which runs to the end of its
/// NUL-terminated buffer. `next_pos`: where decoding stopped (set on
/// success).
fn mime_decode(
    pretval: &mut SmartStr,
    s: &[u8],
    enc: &CStr,
    next_pos: Option<&mut usize>,
    mode: i64,
) -> IconvErr {
    let strict = mode & MIME_DECODE_STRICT != 0;
    let coe = mode & MIME_DECODE_CONTINUE_ON_ERROR != 0;
    let _locale = sys::Locale::enter();
    let Some(cd_pl) = Cd::open(enc, ASCII) else {
        return open_error();
    };
    let cd_pl = Some(&cd_pl);
    let mut cd: Option<Cd> = None;
    // The byte at `i`: php reads the string's NUL terminator past the end.
    let at = |i: isize| -> u8 {
        if i < 0 {
            0
        } else {
            s.get(i as usize).copied().unwrap_or(0)
        }
    };
    // `[from, to)` of the input; php's pointer arithmetic on a stale or
    // null word start is undefined, here it yields a short slice.
    let seg = |from: Option<usize>, to: isize| -> &[u8] {
        let to = (to.max(0) as usize).min(s.len());
        let from = from.unwrap_or(to).min(to);
        &s[from..to]
    };
    let mut err = Success;
    let mut p1: isize = 0;
    let mut str_left = s.len();
    let mut scan_stat = 0u32;
    let mut csname: Option<usize> = None;
    let mut encoded_text = 0usize;
    let mut encoded_text_len = 0usize;
    let mut encoded_word: Option<usize> = None;
    let mut spaces: Option<usize> = None;
    let mut enc_scheme = Scheme::Base64;
    let after_word = |strict: bool| if strict { 12 } else { 0 };

    'out_try: {
        while str_left > 0 {
            let c = at(p1);
            let p = p1 as usize;
            let mut eos = false;
            let mut choice = false;
            match scan_stat {
                // Expecting any character.
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
                        err = appendc(pretval, c, cd_pl);
                        if err != Success {
                            if coe {
                                err = Success;
                            } else {
                                break 'out_try;
                            }
                        }
                        encoded_word = None;
                        if strict {
                            scan_stat = 12;
                        }
                    }
                },
                // Expecting a delimiter.
                1 => {
                    if c != b'?' {
                        if c == b'\r' || c == b'\n' {
                            p1 -= 1;
                        }
                        err = appendl(pretval, Some(seg(encoded_word, p1 + 1)), cd_pl);
                        if err != Success {
                            break 'out_try;
                        }
                        encoded_word = None;
                        scan_stat = after_word(strict);
                    } else {
                        csname = Some(p + 1);
                        scan_stat = 2;
                    }
                }
                // Expecting a charset name.
                2 => {
                    match c {
                        // The normal delimiter: the encoding scheme follows.
                        b'?' => scan_stat = 3,
                        // The new style delimiter: a locale id follows.
                        b'*' => scan_stat = 10,
                        // Not an encoded word.
                        b'\r' | b'\n' => {
                            p1 -= 1;
                            appendc(pretval, b'=', cd_pl);
                            appendc(pretval, b'?', cd_pl);
                            err = appendl(pretval, Some(seg(csname, p1 + 1)), cd_pl);
                            if err != Success {
                                break 'out_try;
                            }
                            csname = None;
                            scan_stat = after_word(strict);
                            str_left -= 1;
                            p1 += 1;
                            continue;
                        }
                        _ => {}
                    }
                    if scan_stat != 2 {
                        let Some(cs) = csname else {
                            err = Malformed;
                            break 'out_try;
                        };
                        let csname_len = p - cs;
                        if csname_len > 79 {
                            if coe {
                                err = appendl(pretval, Some(seg(encoded_word, p1 + 1)), cd_pl);
                                if err != Success {
                                    break 'out_try;
                                }
                                encoded_word = None;
                                scan_stat = after_word(strict);
                            } else {
                                err = Malformed;
                                break 'out_try;
                            }
                        } else {
                            drop(cd.take());
                            cd = Cd::open(enc, &cstr(&s[cs..p]));
                            if cd.is_none() {
                                if coe {
                                    // A bad charset: pass the encoded word
                                    // through undecoded, up to its end.
                                    let mut qmarks = 2;
                                    while qmarks > 0 && str_left > 1 {
                                        p1 += 1;
                                        if at(p1) == b'?' {
                                            qmarks -= 1;
                                        }
                                        str_left -= 1;
                                    }
                                    // The terminating '=' too, if it is there.
                                    if at(p1 + 1) == b'=' {
                                        p1 += 1;
                                        if str_left > 1 {
                                            str_left -= 1;
                                        }
                                    }
                                    err = appendl(pretval, Some(seg(encoded_word, p1 + 1)), cd_pl);
                                    if err != Success {
                                        break 'out_try;
                                    }
                                    scan_stat = 12;
                                } else {
                                    err = open_error();
                                    break 'out_try;
                                }
                            }
                        }
                    }
                }
                // Expecting an encoding scheme specifier.
                3 => match c {
                    b'b' | b'B' => {
                        enc_scheme = Scheme::Base64;
                        scan_stat = 4;
                    }
                    b'q' | b'Q' => {
                        enc_scheme = Scheme::QPrint;
                        scan_stat = 4;
                    }
                    _ => {
                        if coe {
                            err = appendl(pretval, Some(seg(encoded_word, p1 + 1)), cd_pl);
                            if err != Success {
                                break 'out_try;
                            }
                            encoded_word = None;
                            scan_stat = after_word(strict);
                        } else {
                            err = Malformed;
                            break 'out_try;
                        }
                    }
                },
                // Expecting a delimiter.
                4 => {
                    if c != b'?' {
                        if coe {
                            // Pass the entire chunk through the converter.
                            err = appendl(pretval, Some(seg(encoded_word, p1 + 1)), cd_pl);
                            if err != Success {
                                break 'out_try;
                            }
                            encoded_word = None;
                            scan_stat = after_word(strict);
                        } else {
                            err = Malformed;
                            break 'out_try;
                        }
                    } else {
                        encoded_text = p + 1;
                        scan_stat = 5;
                    }
                }
                // Expecting an encoded portion.
                5 => {
                    if c == b'?' {
                        encoded_text_len = p - encoded_text;
                        scan_stat = 6;
                    }
                }
                // Expecting a "\n".
                7 => {
                    if c == b'\n' {
                        scan_stat = 8;
                    } else {
                        // A bare CR.
                        appendc(pretval, b'\r', cd_pl);
                        appendc(pretval, c, cd_pl);
                        scan_stat = 0;
                    }
                }
                // Is the following line part of a folded header?
                8 => {
                    if c != b' ' && c != b'\t' {
                        p1 -= 1;
                        str_left = 1;
                    } else {
                        if encoded_word.is_none() {
                            appendc(pretval, b' ', cd_pl);
                        }
                        spaces = None;
                        scan_stat = 11;
                    }
                }
                // Expecting the end-of-chunk "=".
                6 => {
                    if c != b'=' {
                        if coe {
                            // Pass the entire chunk through the converter.
                            err = appendl(pretval, Some(seg(encoded_word, p1 + 1)), cd_pl);
                            if err != Success {
                                break 'out_try;
                            }
                            encoded_word = None;
                            scan_stat = after_word(strict);
                        } else {
                            err = Malformed;
                            break 'out_try;
                        }
                    } else {
                        scan_stat = 9;
                        if str_left == 1 {
                            eos = true;
                            choice = true;
                        }
                    }
                }
                // The choice point.
                9 => choice = true,
                // Expecting a language specifier: dismissed.
                10 => {
                    if c == b'?' {
                        scan_stat = 3;
                    }
                }
                // Expecting a chunk of whitespace.
                11 => match c {
                    b'\r' => scan_stat = 7,
                    b'\n' => scan_stat = 8,
                    b'=' => {
                        if let (Some(sp), None) = (spaces, encoded_word) {
                            appendl(pretval, Some(&s[sp..p]), cd_pl);
                            spaces = None;
                        }
                        encoded_word = Some(p);
                        scan_stat = 1;
                    }
                    b' ' | b'\t' => {}
                    _ => {
                        if let Some(sp) = spaces {
                            appendl(pretval, Some(&s[sp..p]), cd_pl);
                            spaces = None;
                        }
                        appendc(pretval, c, cd_pl);
                        encoded_word = None;
                        scan_stat = after_word(strict);
                    }
                },
                // Expecting a non-encoded word.
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
                        appendc(pretval, c, cd_pl);
                    }
                },
                _ => {}
            }
            if choice {
                'choice: {
                    if !matches!(c, b'\r' | b'\n' | b' ' | b'\t') && !eos && strict {
                        // RFC 2047 wants whitespace after an encoded word;
                        // strict mode passes a glued one through as is.
                        err = appendl(pretval, Some(seg(encoded_word, p1 + 1)), cd_pl);
                        if err != Success {
                            break 'out_try;
                        }
                        scan_stat = 12;
                        break 'choice;
                    }
                    let text = &s[encoded_text..encoded_text + encoded_text_len];
                    let decoded = match enc_scheme {
                        Scheme::Base64 => crate::ext::standard::base64::base64_decode(text, false),
                        Scheme::QPrint => qprint_decode(text),
                    };
                    let Some(decoded) = decoded else {
                        if coe {
                            // Pass the entire chunk through the converter.
                            err = appendl(pretval, Some(seg(encoded_word, p1 + 1)), cd_pl);
                            if err != Success {
                                break 'out_try;
                            }
                            encoded_word = None;
                            scan_stat = after_word(strict);
                            break 'choice;
                        }
                        err = Unknown;
                        break 'out_try;
                    };
                    // php's decoded zend_string already counts.
                    let decoded = NulCopy::new(&decoded);
                    err = appendl(pretval, Some(&*decoded), cd.as_ref());
                    if err == Success {
                        err = appendl(pretval, None, cd.as_ref());
                    }
                    if err != Success {
                        if !coe {
                            break 'out_try;
                        }
                        // Pass the entire chunk through the converter.
                        err = appendl(pretval, Some(seg(encoded_word, p1)), cd_pl);
                        encoded_word = None;
                        if err != Success {
                            break 'choice;
                        }
                    }
                    if eos {
                        // The end of the string: done.
                        scan_stat = 0;
                        break 'choice;
                    }
                    match c {
                        b'\r' => scan_stat = 7,
                        b'\n' => scan_stat = 8,
                        b'=' => scan_stat = 1,
                        b' ' | b'\t' => {
                            spaces = Some(p);
                            scan_stat = 11;
                        }
                        _ => {
                            appendc(pretval, c, cd_pl);
                            scan_stat = 12;
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
                        appendc(pretval, b'=', cd_pl);
                    }
                    err = Success;
                } else {
                    err = Malformed;
                    break 'out_try;
                }
            }
        }
        if let Some(np) = next_pos {
            *np = p1.max(0) as usize;
        }
    }
    drop(cd);
    err
}

// ---------------------------------------------------------------------------
// Diagnostics and encodings
// ---------------------------------------------------------------------------

/// _php_iconv_show_error's diagnostic for `err`: (level, message).
fn error_message(err: IconvErr, out_charset: &CStr, in_charset: &CStr) -> Option<(i64, Vec<u8>)> {
    let (level, msg): (i64, &[u8]) = match err {
        Success | OutOfBounds => return None,
        Converter => (crate::E_WARNING, b"Cannot open converter"),
        WrongCharset => {
            let msg = [
                b"Wrong encoding, conversion from \"",
                in_charset.to_bytes(),
                b"\" to \"",
                out_charset.to_bytes(),
                b"\" is not allowed",
            ]
            .concat();
            return Some((crate::E_WARNING, msg));
        }
        IllegalChar => (
            crate::E_NOTICE,
            b"Detected an incomplete multibyte character in input string",
        ),
        IllegalSeq => (
            crate::E_NOTICE,
            b"Detected an illegal character in input string",
        ),
        TooBig => (crate::E_WARNING, b"Buffer length exceeded"),
        Malformed => (crate::E_WARNING, b"Malformed string"),
        Unknown | Alloc => {
            let msg = format!("Unknown error ({})", errno()).into_bytes();
            return Some((crate::E_NOTICE, msg));
        }
    };
    Some((level, msg.to_vec()))
}

/// _php_iconv_show_error from builtin `fname`.
fn show_error(
    host: &mut dyn Host,
    fname: &str,
    err: IconvErr,
    out_charset: &CStr,
    in_charset: &CStr,
) -> Result<(), Thrown> {
    if err == OutOfBounds {
        return Err(value_error(
            fname,
            3,
            "offset",
            "must be contained in argument #1 ($haystack)",
        ));
    }
    match error_message(err, out_charset, in_charset) {
        Some((level, msg)) => {
            host.emit_diagnostic(level, &[fname.as_bytes(), b"(): ", &msg].concat())
        }
        None => Ok(()),
    }
}

/// A directive's value as php's C reads it: up to the first NUL.
fn ini_cstr(name: &[u8]) -> Vec<u8> {
    let v = crate::main::ini::ini_value(name)
        .flatten()
        .unwrap_or_default();
    let end = v.iter().position(|&c| c == 0).unwrap_or(v.len());
    v[..end].to_vec()
}

/// php_get_{internal,input,output}_encoding: the core directive, else
/// default_charset, else UTF-8.
fn php_encoding(core: &[u8]) -> Vec<u8> {
    for name in [core, b"default_charset"] {
        let v = ini_cstr(name);
        if !v.is_empty() {
            return v;
        }
    }
    b"UTF-8".to_vec()
}

/// get_{internal,input,output}_encoding: `iconv.<kind>_encoding`, else
/// php's.
fn encoding(kind: &str) -> Vec<u8> {
    let v = ini_cstr(format!("iconv.{kind}_encoding").as_bytes());
    if !v.is_empty() {
        return v;
    }
    php_encoding(format!("{kind}_encoding").as_bytes())
}

fn too_long(host: &mut dyn Host, fname: &str) -> Result<(), Thrown> {
    warn(
        host,
        fname,
        &format!("Encoding parameter exceeds the maximum allowed length of {CSNMAXLEN} characters"),
    )
}

/// A `?string $encoding` argument: null is the internal encoding. php warns
/// and returns false for one of 64 bytes or more (`Ok(None)`).
fn encoding_arg(
    host: &mut dyn Host,
    raw: Option<Vec<u8>>,
    fname: &str,
) -> Result<Option<CString>, Thrown> {
    match raw {
        None => Ok(Some(cstr(&encoding("internal")))),
        Some(e) if e.len() >= CSNMAXLEN => {
            too_long(host, fname)?;
            Ok(None)
        }
        Some(e) => Ok(Some(cstr(&e))),
    }
}

// ---------------------------------------------------------------------------
// Builtins
// ---------------------------------------------------------------------------

fn fn_iconv_strlen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "iconv_strlen";
    let s = zstr_arg(host, args, 0, f, "string")?;
    let raw = opt_str_arg(host, args, 1, f, "encoding")?;
    let Some(charset) = encoding_arg(host, raw, f)? else {
        return Ok(Value::Bool(false));
    };
    let (err, n) = iconv_strlen(&s, &charset);
    show_error(host, f, err, SUPERSET, &charset)?;
    Ok(if err == Success {
        Value::Int(n as i64)
    } else {
        Value::Bool(false)
    })
}

fn fn_iconv_substr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "iconv_substr";
    let s = zstr_arg(host, args, 0, f, "string")?;
    let offset = long_param(host, args, 1, f, "offset")?;
    let length = opt_long_arg(host, args, 2, f, "length")?;
    let raw = opt_str_arg(host, args, 3, f, "encoding")?;
    let Some(charset) = encoding_arg(host, raw, f)? else {
        return Ok(Value::Bool(false));
    };
    let s = &*s;
    let length = length.unwrap_or(s.len() as i64);
    let (err, retval) = iconv_substr(s, offset, length, &charset);
    show_error(host, f, err, SUPERSET, &charset)?;
    Ok(match retval.s {
        Some(v) if err == Success => Value::str(v),
        _ => Value::Bool(false),
    })
}

fn fn_iconv_strpos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "iconv_strpos";
    let haystk = zstr_arg(host, args, 0, f, "haystack")?;
    let ndl = zstr_arg(host, args, 1, f, "needle")?;
    let mut offset = long_param(host, args, 2, f, "offset")?;
    let raw = opt_str_arg(host, args, 3, f, "encoding")?;
    let Some(charset) = encoding_arg(host, raw, f)? else {
        return Ok(Value::Bool(false));
    };
    let (haystk, ndl) = (&*haystk, &*ndl);
    if offset < 0 {
        // A negative offset counts from the end.
        let (err, haystk_len) = iconv_strlen(haystk, &charset);
        if err != Success {
            show_error(host, f, err, SUPERSET, &charset)?;
            return Ok(Value::Bool(false));
        }
        offset = offset.wrapping_add(haystk_len as i64);
        if offset < 0 {
            return Err(value_error(
                f,
                3,
                "offset",
                "must be contained in argument #1 ($haystack)",
            ));
        }
    }
    if ndl.is_empty() {
        return Ok(Value::Bool(false));
    }
    let (err, pos) = iconv_strpos(haystk, ndl, offset as usize, &charset, false);
    show_error(host, f, err, SUPERSET, &charset)?;
    Ok(match pos {
        Some(p) if err == Success => Value::Int(p as i64),
        _ => Value::Bool(false),
    })
}

fn fn_iconv_strrpos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "iconv_strrpos";
    let haystk = zstr_arg(host, args, 0, f, "haystack")?;
    let ndl = zstr_arg(host, args, 1, f, "needle")?;
    let raw = opt_str_arg(host, args, 2, f, "encoding")?;
    let (haystk, ndl) = (&*haystk, &*ndl);
    if ndl.is_empty() {
        return Ok(Value::Bool(false));
    }
    let Some(charset) = encoding_arg(host, raw, f)? else {
        return Ok(Value::Bool(false));
    };
    let (err, pos) = iconv_strpos(haystk, ndl, 0, &charset, true);
    show_error(host, f, err, SUPERSET, &charset)?;
    Ok(match pos {
        Some(p) if err == Success => Value::Int(p as i64),
        _ => Value::Bool(false),
    })
}

/// zend_hash_find_deref on a string key.
fn pref_get(pref: &PhpArray, key: &str) -> Option<Value> {
    pref.get(&ArrayKey::Str(PhpStr::from_bytes(key.as_bytes())))
        .map(deref)
}

/// An `input-charset` / `output-charset` preference: `Err(())` when php
/// warns and returns false.
fn pref_charset(
    host: &mut dyn Host,
    pref: &PhpArray,
    key: &str,
    target: &mut CString,
) -> Result<Result<(), ()>, Thrown> {
    if let Some(Value::Str(s)) = pref_get(pref, key) {
        if s.len() >= CSNMAXLEN {
            too_long(host, "iconv_mime_encode")?;
            return Ok(Err(()));
        }
        if !s.is_empty() {
            *target = cstr(s.as_bytes());
        }
    }
    Ok(Ok(()))
}

fn fn_iconv_mime_encode(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "iconv_mime_encode";
    let field_name = zstr_arg(host, args, 0, f, "field_name")?;
    let field_value = zstr_arg(host, args, 1, f, "field_value")?;
    let pref = match args.get(2) {
        None => None,
        Some(_) => Some(crate::zend::zpp::array_param(host, args, 2, f)?),
    };
    let mut in_charset = cstr(&encoding("internal"));
    let mut out_charset = in_charset.clone();
    let mut line_len: i64 = 76;
    let mut lfchars = b"\r\n".to_vec();
    let mut scheme = Scheme::Base64;
    if let Some(pref) = pref {
        if let Some(Value::Str(s)) = pref_get(&pref, "scheme") {
            match s.as_bytes().first() {
                Some(b'B' | b'b') => scheme = Scheme::Base64,
                Some(b'Q' | b'q') => scheme = Scheme::QPrint,
                _ => {}
            }
        }
        if pref_charset(host, &pref, "input-charset", &mut in_charset)?.is_err()
            || pref_charset(host, &pref, "output-charset", &mut out_charset)?.is_err()
        {
            return Ok(Value::Bool(false));
        }
        if let Some(v) = pref_get(&pref, "line-length") {
            line_len = crate::zend::zpp::zval_get_long(host, &v)?;
        }
        if let Some(v) = pref_get(&pref, "line-break-chars") {
            lfchars = match v {
                Value::Str(s) => s.as_bytes().to_vec(),
                v => crate::zend::zpp::zval_get_string(host, &v)?,
            };
        }
    }
    let lfchars = cstr(&lfchars);
    let mut retval = SmartStr::default();
    let err = mime_encode(
        &mut retval,
        &field_name,
        &field_value,
        line_len as usize,
        lfchars.as_bytes(),
        scheme,
        &out_charset,
        &in_charset,
    );
    show_error(host, f, err, &out_charset, &in_charset)?;
    Ok(if err == Success {
        Value::str(retval.extract())
    } else {
        Value::Bool(false)
    })
}

fn fn_iconv_mime_decode(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "iconv_mime_decode";
    let encoded = zstr_arg(host, args, 0, f, "string")?;
    let mode = long_param(host, args, 1, f, "mode")?;
    let raw = opt_str_arg(host, args, 2, f, "encoding")?;
    let Some(charset) = encoding_arg(host, raw, f)? else {
        return Ok(Value::Bool(false));
    };
    let mut retval = SmartStr::default();
    let err = mime_decode(&mut retval, &encoded, &charset, None, mode as i32 as i64);
    show_error(host, f, err, &charset, c"???")?;
    Ok(if err == Success {
        Value::str(retval.extract())
    } else {
        Value::Bool(false)
    })
}

fn fn_iconv_mime_decode_headers(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "iconv_mime_decode_headers";
    let encoded = zstr_arg(host, args, 0, f, "headers")?;
    let mode = long_param(host, args, 1, f, "mode")?;
    let raw = opt_str_arg(host, args, 2, f, "encoding")?;
    let Some(charset) = encoding_arg(host, raw, f)? else {
        return Ok(Value::Bool(false));
    };
    let end = encoded.len();
    let mut out = PhpArray::new();
    let mut err = Success;
    let mut pos = 0usize;
    while pos < end && !has_oom() {
        let mut decoded = SmartStr::default();
        let mut next = 0usize;
        err = mime_decode(
            &mut decoded,
            &encoded[pos..end],
            &charset,
            Some(&mut next),
            mode as i32 as i64,
        );
        if err != Success {
            break;
        }
        let Some(h) = decoded.s else {
            break;
        };
        if let Some(colon) = h.iter().position(|&c| c == b':') {
            let name = &h[..colon];
            let mut v = colon + 1;
            while v < h.len() && (h[v] == b' ' || h[v] == b'\t') {
                v += 1;
            }
            let value = Value::str(&h[v..]);
            // zend_hash_str_find, then add_assoc_stringl_ex (a symtable
            // update: numeric names become integer keys).
            match out.get_mut(&ArrayKey::Str(PhpStr::from_bytes(name))) {
                Some(elem) => {
                    if !matches!(elem, Value::Array(_)) {
                        let mut list = PhpArray::new();
                        list.push(elem.clone());
                        *elem = Value::Array(list);
                    }
                    if let Value::Array(list) = elem {
                        list.push(value);
                    }
                }
                None => out.insert(ArrayKey::from_bytes(name), value),
            }
        }
        pos += next;
    }
    if err != Success {
        show_error(host, f, err, &charset, c"???")?;
        return Ok(Value::Bool(false));
    }
    Ok(Value::Array(out))
}

fn fn_iconv(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "iconv";
    let in_charset = str_arg(host, args, 0, f, "from_encoding")?;
    let out_charset = str_arg(host, args, 1, f, "to_encoding")?;
    let input = zstr_arg(host, args, 2, f, "string")?;
    if in_charset.len() >= CSNMAXLEN || out_charset.len() >= CSNMAXLEN {
        too_long(host, f)?;
        return Ok(Value::Bool(false));
    }
    let (in_charset, out_charset) = (cstr(&in_charset), cstr(&out_charset));
    let (err, out) = iconv_string(&input, &out_charset, &in_charset);
    show_error(host, f, err, &out_charset, &in_charset)?;
    Ok(match out {
        Some(o) if err == Success => Value::str(o),
        _ => Value::Bool(false),
    })
}

/// The `iconv.*` directive `iconv_set_encoding` / `iconv_get_encoding` name
/// with `type`.
fn encoding_kind(ty: &[u8]) -> Option<&'static str> {
    ["input", "output", "internal"]
        .into_iter()
        .find(|k| ty.eq_ignore_ascii_case(format!("{k}_encoding").as_bytes()))
}

fn fn_iconv_set_encoding(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "iconv_set_encoding";
    let ty = str_arg(host, args, 0, f, "type")?;
    let charset = str_arg(host, args, 1, f, "encoding")?;
    if charset.len() >= CSNMAXLEN {
        too_long(host, f)?;
        return Ok(Value::Bool(false));
    }
    let Some(kind) = encoding_kind(&ty) else {
        return Ok(Value::Bool(false));
    };
    let name = format!("iconv.{kind}_encoding");
    let ok = crate::main::ini::alter_ini_entry(host, name.as_bytes(), charset, f)?;
    Ok(Value::Bool(ok.is_some()))
}

fn fn_iconv_get_encoding(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "iconv_get_encoding";
    let ty = match args.first() {
        None => None,
        Some(_) => Some(str_arg(host, args, 0, f, "type")?),
    };
    match ty {
        Some(t) if !t.eq_ignore_ascii_case(b"all") => Ok(match encoding_kind(&t) {
            Some(kind) => Value::str(encoding(kind)),
            None => Value::Bool(false),
        }),
        _ => {
            let mut a = PhpArray::new();
            for kind in ["input", "output", "internal"] {
                a.insert(
                    ArrayKey::from_bytes(format!("{kind}_encoding").as_bytes()),
                    Value::str(encoding(kind)),
                );
            }
            Ok(Value::Array(a))
        }
    }
}

// ---------------------------------------------------------------------------
// ob_iconv_handler and the ini directives
// ---------------------------------------------------------------------------

const OUTPUT_START: i64 = 0x01;
const OUTPUT_CLEAN: i64 = 0x02;
const OUTPUT_FINAL: i64 = 0x08;

/// php_iconv_output_handler: on start, add a Content-Type header naming the
/// output encoding; convert the buffer from the internal encoding.
fn output_handler(host: &mut dyn Host, cx: &mut OutputHandlerCtx) -> bool {
    if cx.op & OUTPUT_START != 0 {
        if cx.output_sent {
            return false;
        }
        let default_mimetype = crate::main::ini::ini_value(b"default_mimetype").flatten();
        let mimetype = host.response().and_then(|r| match &r.mimetype {
            Some(m) if m.len() >= 5 && m[..5].eq_ignore_ascii_case(b"text/") => {
                // Up to a ';', when there is one.
                let end = match m.iter().position(|&c| c == b';') {
                    Some(0) | None => m.len(),
                    Some(i) => i,
                };
                Some(m[..end].to_vec())
            }
            _ if r.send_default_content_type => {
                Some(default_mimetype.unwrap_or_else(|| b"text/html".to_vec()))
            }
            _ => None,
        });
        if let Some(mimetype) = mimetype {
            if cx.op & OUTPUT_CLEAN == 0 || cx.op & OUTPUT_FINAL == 0 {
                let out = encoding("output");
                let charset = match out.windows(2).position(|w| w == b"//") {
                    Some(i) => &out[..i],
                    None => &out[..],
                };
                // `%.*s` stops at a NUL.
                let mimetype = cstr(&mimetype);
                let mut line = b"Content-Type:".to_vec();
                line.extend_from_slice(mimetype.as_bytes());
                line.extend_from_slice(b"; charset=");
                line.extend_from_slice(charset);
                if crate::main::sapi::header_op(
                    host,
                    crate::main::sapi::HeaderOp::Replace(&line, 0),
                )
                .unwrap_or(false)
                {
                    if let Some(r) = host.response() {
                        r.send_default_content_type = false;
                    }
                    cx.immutable = true;
                }
            }
        }
    }
    if !cx.input.is_empty() {
        let (out_c, in_c) = (cstr(&encoding("output")), cstr(&encoding("internal")));
        let input = NulCopy::new(cx.input);
        let (err, out) = iconv_string(&input, &out_c, &in_c);
        cx.diags.extend(error_message(err, &out_c, &in_c));
        cx.out = out.unwrap_or_default();
    }
    true
}

/// The output handlers this extension registers by name.
pub(crate) const OUTPUT_HANDLERS: &[OutputHandlerAlias] = &[OutputHandlerAlias {
    name: "ob_iconv_handler",
    func: output_handler,
    conflicts: &["ob_iconv_handler", "mb_output_handler"],
    init: None,
}];

fn is_iconv_directive(name: &[u8]) -> bool {
    matches!(
        name,
        b"iconv.input_encoding" | b"iconv.output_encoding" | b"iconv.internal_encoding"
    )
}

/// OnUpdate{Input,Output,Internal}Encoding's refusal: a value of 64 bytes or
/// more.
pub(crate) fn ini_refuses(name: &[u8], v: &[u8]) -> bool {
    is_iconv_directive(name) && v.len() >= CSNMAXLEN
}

/// The same handlers at runtime (from builtin `func`): every change is
/// deprecated.
pub(crate) fn ini_runtime_update(
    host: &mut dyn Host,
    name: &[u8],
    func: &str,
) -> Result<(), Thrown> {
    if !is_iconv_directive(name) {
        return Ok(());
    }
    crate::deprecated(
        host,
        func,
        &format!("Use of {} is deprecated", String::from_utf8_lossy(name)),
    )
}

/// PHP_MINFO_FUNCTION(miconv)'s rows.
pub(crate) fn info_rows() -> [(&'static str, String); 3] {
    [
        ("iconv support", "enabled".to_string()),
        ("iconv implementation", sys::implementation().to_string()),
        ("iconv library version", sys::version()),
    ]
}

/// A new request: php's errno starts out clear.
pub(crate) fn reset_state() {
    set_errno(0);
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("iconv_strlen", fn_iconv_strlen),
            ("iconv_substr", fn_iconv_substr),
            ("iconv_strpos", fn_iconv_strpos),
            ("iconv_strrpos", fn_iconv_strrpos),
            ("iconv_mime_encode", fn_iconv_mime_encode),
            ("iconv_mime_decode", fn_iconv_mime_decode),
            ("iconv_mime_decode_headers", fn_iconv_mime_decode_headers),
            ("iconv", fn_iconv),
            ("iconv_set_encoding", fn_iconv_set_encoding),
            ("iconv_get_encoding", fn_iconv_get_encoding),
        ],
    );
}

pub fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("ICONV_IMPL", Value::str(sys::implementation())),
        ("ICONV_VERSION", Value::str(sys::version())),
        ("ICONV_MIME_DECODE_STRICT", Value::Int(MIME_DECODE_STRICT)),
        (
            "ICONV_MIME_DECODE_CONTINUE_ON_ERROR",
            Value::Int(MIME_DECODE_CONTINUE_ON_ERROR),
        ),
    ]
}

#[cfg(test)]
mod tests;
