//! `grapheme_*` — extended-grapheme-cluster string functions
//! (php-src `ext/intl/grapheme/grapheme_string.c` + `grapheme_util.c`).
//!
//! Every function keeps php's ASCII fast paths (byte == grapheme, except that
//! a `"\r\n"` pair sends the string down the Unicode path), and otherwise works
//! on the default grapheme-cluster boundaries from `icu_segmenter`. Positions
//! php computes in UTF-16 units are computed here in UTF-8 bytes over the same
//! boundaries, which yields identical grapheme counts and substrings.
//!
//! ICU's `usearch` (collation-based string search, tertiary strength, or
//! secondary for the case-insensitive variants) is modelled as canonical
//! equivalence (NFD) with completely ignorable code points dropped, plus case
//! folding for the `i` variants, with matches required to start and end on
//! grapheme boundaries (and, for a needle that isn't itself ignorable, to
//! start on a grapheme that isn't).
//!
//! Boundaries come from ICU4X's state-machine grapheme data
//! (`GraphemeClusterSegmenter::new_neo`), which carries ICU 79's rules,
//! including the simplified GB9c (`Linker Extend* × Consonant`).

use icu_normalizer::DecomposingNormalizerBorrowed;
use icu_segmenter::GraphemeClusterSegmenter;
use rhp_values::{PhpArray, Value};

use super::{
    as_utf8, collation_ignorable, error_set, icu_fold_extra, path_arg, U_ILLEGAL_ARGUMENT_ERROR,
    U_INVALID_CHAR_FOUND,
};
use crate::ext::zpp::{bool_arg_or, int_arg, int_arg_or, opt_int_arg, str_arg, value_error};
use crate::{Host, Thrown};

/// Grapheme boundaries of valid UTF-8 text: always starts with 0 and ends with
/// `s.len()` (just `[0]` for the empty string).
fn breaks(s: &str) -> Vec<usize> {
    GraphemeClusterSegmenter::new_neo().segment_str(s).collect()
}

/// Boundaries of possibly ill-formed UTF-8 (ICU's `utext_openUTF8`, which
/// reads ill-formed sequences as U+FFFD).
fn breaks_bytes(b: &[u8]) -> Vec<usize> {
    GraphemeClusterSegmenter::new_neo()
        .segment_utf8(b)
        .collect()
}

/// `grapheme_ascii_check`: true when every byte is ASCII and there is no CRLF.
fn ascii_check(b: &[u8]) -> bool {
    for (i, &c) in b.iter().enumerate() {
        if c > 0x7f {
            return false;
        }
        if c == b'\r' && b.get(i + 1) == Some(&b'\n') {
            return false;
        }
    }
    true
}

/// `OUTSIDE_STRING(offset, max_len)`.
fn outside_string(offset: i64, len: usize) -> bool {
    offset <= i32::MIN as i64
        || offset > i32::MAX as i64
        || if offset < 0 {
            -offset > len as i64
        } else {
            offset > len as i64
        }
}

fn memnstr(h: &[u8], from: usize, n: &[u8]) -> Option<usize> {
    if n.is_empty() {
        return Some(from);
    }
    if from > h.len() || n.len() > h.len() - from {
        return None;
    }
    (from..=h.len() - n.len()).find(|&i| &h[i..i + n.len()] == n)
}

/// A UBreakIterator-like cursor over precomputed boundaries.
struct Bi<'a> {
    b: &'a [usize],
    idx: usize,
}

impl Bi<'_> {
    fn new(b: &[usize]) -> Bi<'_> {
        Bi { b, idx: 0 }
    }
    fn first(&mut self) -> usize {
        self.idx = 0;
        0
    }
    fn last(&mut self) -> usize {
        self.idx = self.b.len() - 1;
        self.b[self.idx]
    }
    fn next(&mut self) -> Option<usize> {
        if self.idx + 1 < self.b.len() {
            self.idx += 1;
            Some(self.b[self.idx])
        } else {
            None
        }
    }
    fn previous(&mut self) -> Option<usize> {
        if self.idx > 0 {
            self.idx -= 1;
            Some(self.b[self.idx])
        } else {
            None
        }
    }
}

/// `grapheme_get_haystack_offset`: the position `offset` graphemes from the
/// start (or `-offset` from the end), or `None` when out of range.
fn haystack_offset(b: &[usize], offset: i64) -> Option<usize> {
    if offset == 0 {
        return Some(0);
    }
    let mut bi = Bi::new(b);
    let mut off = offset;
    let mut pos = Some(0);
    if off < 0 {
        bi.last();
    }
    while pos.is_some() && off != 0 {
        pos = if off < 0 { bi.previous() } else { bi.next() };
        if pos.is_some() {
            off += if off < 0 { 1 } else { -1 };
        }
    }
    if off != 0 {
        None
    } else {
        pos
    }
}

/// Number of graphemes in `s` (as standalone text).
fn count_graphemes(s: &str) -> usize {
    breaks(s).len() - 1
}

/// The comparison key of a piece of text for the collation-search model:
/// its NFD without collation-ignorable code points, case-folded (and
/// re-decomposed) when `fold`.
fn key(s: &str, fold: bool) -> Vec<u32> {
    let nfd = DecomposingNormalizerBorrowed::new_nfd();
    let d: Vec<u32> = nfd
        .normalize(s)
        .chars()
        .map(|c| c as u32)
        .filter(|&c| !collation_ignorable(c))
        .collect();
    if !fold {
        return d;
    }
    let folded: Vec<u32> = crate::ext::mbstring::case_fold_simple(&d)
        .into_iter()
        .map(|c| icu_fold_extra(c).unwrap_or(c))
        .collect();
    let st: String = folded.iter().filter_map(|&c| char::from_u32(c)).collect();
    nfd.normalize(&st)
        .chars()
        .map(|c| c as u32)
        .filter(|&c| !collation_ignorable(c))
        .collect()
}

/// All candidate matches `(start, end)` (byte offsets on grapheme
/// boundaries) of `needle` in `h`, by start.
fn matches(h: &str, b: &[usize], needle: &str, fold: bool) -> Vec<(usize, usize)> {
    let k = key(needle, fold);
    let gkeys: Vec<Vec<u32>> = b.windows(2).map(|w| key(&h[w[0]..w[1]], fold)).collect();
    let mut out = Vec::new();
    for i in 0..gkeys.len() {
        if !k.is_empty()
            && h[b[i]..]
                .chars()
                .next()
                .is_some_and(|c| collation_ignorable(c as u32))
        {
            // usearch starts a match at its first non-ignorable code point,
            // which must then be a grapheme boundary
            continue;
        }
        let mut acc: Vec<u32> = Vec::new();
        let mut j = i;
        while j < gkeys.len() && acc.len() < k.len() {
            acc.extend_from_slice(&gkeys[j]);
            j += 1;
        }
        if acc == k {
            out.push((b[i], b[j]));
        }
    }
    out
}

/// `grapheme_strpos_utf16` (php 8.5.12). Returns `Ok(Some((ret, start)))`
/// where `ret` is the grapheme position and `start` the match's byte offset
/// (php returns its code point count, which `U8_FWD_N` turns back into the
/// same byte offset); `start` is `None` for an empty needle.
/// `Ok(None)` = not found / error.
fn strpos_utf16(
    fname: &str,
    h: &[u8],
    n: &[u8],
    offset: i64,
    fold: bool,
    last: bool,
) -> Result<Option<(usize, Option<usize>)>, Thrown> {
    if h.is_empty() && !n.is_empty() {
        // checked before any conversion; sets no error
        return Ok(None);
    }
    let Some(hs) = as_utf8(h) else {
        error_set(
            fname,
            U_INVALID_CHAR_FOUND,
            "Error converting input string to UTF-16",
        );
        return Ok(None);
    };
    let Some(ns) = as_utf8(n) else {
        error_set(
            fname,
            U_INVALID_CHAR_FOUND,
            "Error converting needle string to UTF-16",
        );
        return Ok(None);
    };
    let b = breaks(hs);
    let offset_err = || {
        value_error(
            fname,
            3,
            "offset",
            "must be contained in argument #1 ($haystack)",
        )
    };
    if ns.is_empty() {
        let pos = haystack_offset(&b, offset).ok_or_else(offset_err)?;
        let ret = if last && offset >= 0 {
            count_graphemes(hs)
        } else {
            count_graphemes(&hs[..pos])
        };
        return Ok(Some((ret, None)));
    }
    let mut offset_pos = 0;
    if offset != 0 {
        offset_pos = haystack_offset(&b, offset).ok_or_else(offset_err)?;
    }
    let all = matches(hs, &b, ns, fold);
    let found = if last {
        if offset >= 0 && key(ns, fold).is_empty() {
            // an all-ignorable needle "matches" at every code point, so
            // usearch_last lands on the last one, which may not start a
            // grapheme
            hs.char_indices()
                .last()
                .map(|(p, _)| p)
                .filter(|&p| p >= offset_pos && b.contains(&p))
        } else if offset >= 0 {
            all.last().map(|m| m.0).filter(|&p| p >= offset_pos)
        } else {
            // usearch_next() from 0, each search resuming at the grapheme
            // boundary following the previous match's start (so overlapping
            // matches count)
            let mut prev = None;
            let mut cur = 0usize;
            loop {
                match all.iter().find(|m| m.0 >= cur) {
                    Some(&(s, _)) if s <= offset_pos => {
                        prev = Some(s);
                        match b.iter().find(|&&x| x > s) {
                            Some(&next) => cur = next,
                            None => break,
                        }
                    }
                    _ => break,
                }
            }
            prev
        }
    } else {
        all.iter().map(|m| m.0).find(|&p| p >= offset_pos)
    };
    Ok(found.map(|p| (count_graphemes(&hs[..p]), Some(p))))
}

struct PosArgs {
    h: Vec<u8>,
    n: Vec<u8>,
    offset: i64,
}

fn pos_args(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<PosArgs, Thrown> {
    let h = str_arg(host, args, 0, fname, "haystack")?;
    let n = str_arg(host, args, 1, fname, "needle")?;
    let offset = int_arg_or(host, args, 2, fname, "offset", 0)?;
    path_arg(host, args, 3, fname, "locale")?;
    if outside_string(offset, h.len()) {
        return Err(value_error(
            fname,
            3,
            "offset",
            "must be contained in argument #1 ($haystack)",
        ));
    }
    Ok(PosArgs { h, n, offset })
}

fn ret_pos(r: Option<(usize, Option<usize>)>) -> Value {
    match r {
        Some((p, _)) => Value::Int(p as i64),
        None => Value::Bool(false),
    }
}

pub(super) fn fn_strlen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = str_arg(host, args, 0, "grapheme_strlen", "string")?;
    if ascii_check(&s) {
        return Ok(Value::Int(s.len() as i64));
    }
    match as_utf8(&s) {
        Some(t) => Ok(Value::Int(count_graphemes(t) as i64)),
        None => {
            error_set(
                "grapheme_strlen",
                U_INVALID_CHAR_FOUND,
                "Error converting input string to UTF-16",
            );
            Ok(Value::Null)
        }
    }
}

pub(super) fn fn_strpos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "grapheme_strpos";
    let a = pos_args(host, args, f)?;
    let noffset = if a.offset >= 0 {
        a.offset as usize
    } else {
        (a.h.len() as i64 + a.offset) as usize
    };
    if a.offset >= 0 && ascii_check(&a.h) {
        if let Some(p) = memnstr(&a.h, noffset, &a.n) {
            return Ok(Value::Int(p as i64));
        }
        // a non-ASCII needle can still match canonically (K vs U+212A)
        if ascii_check(&a.n) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(ret_pos(strpos_utf16(
        f, &a.h, &a.n, a.offset, false, false,
    )?))
}

pub(super) fn fn_stripos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "grapheme_stripos";
    let a = pos_args(host, args, f)?;
    if ascii_check(&a.h) {
        let noffset = if a.offset >= 0 {
            a.offset as usize
        } else {
            (a.h.len() as i64 + a.offset) as usize
        };
        let hl = a.h.to_ascii_lowercase();
        let nl = a.n.to_ascii_lowercase();
        if let Some(p) = memnstr(&hl, noffset, &nl) {
            return Ok(Value::Int(p as i64));
        }
        if ascii_check(&a.n) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(ret_pos(strpos_utf16(f, &a.h, &a.n, a.offset, true, false)?))
}

/// `grapheme_strrpos_ascii` (borrowed from `strrpos`).
fn strrpos_ascii(h: &[u8], n: &[u8], offset: i64) -> Option<usize> {
    let hl = h.len() as i64;
    let nl = n.len() as i64;
    let (p, e) = if offset >= 0 {
        (offset, hl - nl)
    } else if nl > -offset {
        (0, hl - nl)
    } else {
        (0, hl + offset)
    };
    let mut e = e;
    while e >= p {
        if e >= 0 && (e + nl) <= hl && &h[e as usize..(e + nl) as usize] == n {
            return Some((e - p + if offset > 0 { offset } else { 0 }) as usize);
        }
        e -= 1;
    }
    None
}

pub(super) fn fn_strrpos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "grapheme_strrpos";
    let a = pos_args(host, args, f)?;
    if ascii_check(&a.h) {
        if let Some(p) = strrpos_ascii(&a.h, &a.n, a.offset) {
            return Ok(Value::Int(p as i64));
        }
        if ascii_check(&a.n) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(ret_pos(strpos_utf16(f, &a.h, &a.n, a.offset, false, true)?))
}

pub(super) fn fn_strripos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "grapheme_strripos";
    let a = pos_args(host, args, f)?;
    if ascii_check(&a.h) {
        let hl = a.h.to_ascii_lowercase();
        let nl = a.n.to_ascii_lowercase();
        if let Some(p) = strrpos_ascii(&hl, &nl, a.offset) {
            return Ok(Value::Int(p as i64));
        }
        if ascii_check(&a.n) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(ret_pos(strpos_utf16(f, &a.h, &a.n, a.offset, true, true)?))
}

pub(super) fn fn_substr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "grapheme_substr";
    let s = str_arg(host, args, 0, f, "string")?;
    let lstart = int_arg(host, args, 1, f, "offset")?;
    let length = opt_int_arg(host, args, 2, f, "length")?;
    path_arg(host, args, 3, f, "locale")?;
    if lstart < i32::MIN as i64 || lstart > i32::MAX as i64 {
        return Err(value_error(f, 2, "offset", "is too large"));
    }
    let mut start = lstart;
    let mut length = length.unwrap_or(s.len() as i64);
    if length < i32::MIN as i64 || length > i32::MAX as i64 {
        return Err(value_error(f, 3, "length", "is too large"));
    }
    let slen = s.len() as i64;
    if ascii_check(&s) {
        // grapheme_substr_ascii
        let mut fr = start;
        if fr < 0 {
            fr = (slen + fr).max(0);
        } else if fr > slen {
            fr = slen;
        }
        let mut l = length;
        if l < 0 {
            l = ((slen - fr) + l).max(0);
        } else if l > slen - fr {
            l = slen - fr;
        }
        return Ok(Value::str(s[fr as usize..(fr + l) as usize].to_vec()));
    }
    let Some(t) = as_utf8(&s) else {
        error_set(
            f,
            U_INVALID_CHAR_FOUND,
            "Error converting input string to UTF-16",
        );
        return Ok(Value::Bool(false));
    };
    let b = breaks(t);
    let mut bi = Bi::new(&b);
    let backwards = start < 0;
    if backwards {
        bi.last();
    }
    let mut sub_start = 0usize;
    while start != 0 {
        let p = if backwards { bi.previous() } else { bi.next() };
        match p {
            None => break,
            Some(p) => sub_start = p,
        }
        start += if backwards { 1 } else { -1 };
    }
    if start != 0 {
        if start > 0 {
            return Ok(Value::str(""));
        }
        sub_start = 0;
        bi.first();
    }
    if length >= slen {
        return Ok(Value::str(t.as_bytes()[sub_start..].to_vec()));
    }
    if length == 0 {
        return Ok(Value::str(""));
    }
    let backwards = length < 0;
    if backwards {
        bi.last();
    }
    let mut sub_end: Option<usize> = Some(0);
    while length != 0 {
        sub_end = if backwards { bi.previous() } else { bi.next() };
        if sub_end.is_none() {
            break;
        }
        length += if backwards { 1 } else { -1 };
    }
    let sub_end = match sub_end {
        None if length < 0 => return Ok(Value::str("")),
        None => t.len(),
        Some(e) => e,
    };
    if sub_start > sub_end {
        return Ok(Value::str(""));
    }
    Ok(Value::str(t.as_bytes()[sub_start..sub_end].to_vec()))
}

fn strstr_common(host: &mut dyn Host, args: &[Value], fold: bool) -> Result<Value, Thrown> {
    let f = if fold {
        "grapheme_stristr"
    } else {
        "grapheme_strstr"
    };
    let h = str_arg(host, args, 0, f, "haystack")?;
    let n = str_arg(host, args, 1, f, "needle")?;
    let part = bool_arg_or(host, args, 2, f, "beforeNeedle", false)?;
    path_arg(host, args, 3, f, "locale")?;
    if !fold && ascii_check(&h) && ascii_check(&n) {
        // ASCII on both sides: a byte search is exact
        return Ok(match memnstr(&h, 0, &n) {
            None => Value::Bool(false),
            Some(o) if part => Value::str(h[..o].to_vec()),
            Some(o) => Value::str(h[o..].to_vec()),
        });
    }
    let Some((_, start)) = strpos_utf16(f, &h, &n, 0, fold, false)? else {
        return Ok(Value::Bool(false));
    };
    let pos = start.unwrap_or(0);
    Ok(if part {
        Value::str(h[..pos].to_vec())
    } else {
        Value::str(h[pos..].to_vec())
    })
}

pub(super) fn fn_strstr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strstr_common(host, args, false)
}

pub(super) fn fn_stristr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strstr_common(host, args, true)
}

pub(super) fn fn_str_split(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "grapheme_str_split";
    let s = str_arg(host, args, 0, f, "string")?;
    let split_len = int_arg_or(host, args, 1, f, "length", 1)?;
    let max = (u32::MAX / 4) as i64;
    if split_len <= 0 || split_len > max {
        return Err(value_error(
            f,
            2,
            "length",
            &format!("must be greater than 0 and less than or equal to {max}"),
        ));
    }
    let mut out = PhpArray::new();
    if s.is_empty() {
        return Ok(Value::Array(out));
    }
    let b = breaks_bytes(&s);
    let mut bi = Bi::new(&b);
    let mut pos: Option<usize> = Some(0);
    let mut pstr = 0usize; // byte index into s
    let mut end = 0usize;
    let mut current = 0usize;
    let mut i = 0i64;
    let mut end_len = 0usize;
    while let Some(p) = pos {
        end_len = p - current;
        pos = bi.next();
        if i == split_len - 1 {
            if let Some(np) = pos {
                out.push(Value::str(s[pstr..pstr + (np - current)].to_vec()));
                end = pstr + np - current;
                i = 0;
                pstr += np - current;
                current = np;
            }
        } else {
            i += 1;
        }
    }
    if i != 0 && end_len != 0 {
        out.push(Value::str(s[end..end + end_len].to_vec()));
    }
    Ok(Value::Array(out))
}

pub(super) fn fn_extract(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "grapheme_extract";
    let s = str_arg(host, args, 0, f, "haystack")?;
    let size = int_arg(host, args, 1, f, "size")?;
    let extract_type = int_arg_or(host, args, 2, f, "type", 0)?;
    let mut lstart = int_arg_or(host, args, 3, f, "offset", 0)?;
    let has_next = matches!(args.get(4), Some(Value::Ref(_)));
    let set_next = |v: i64| crate::ext::zpp::set_ref(args, 4, Value::Int(v));
    if lstart < 0 {
        lstart += s.len() as i64;
    }
    if has_next {
        set_next(lstart);
    }
    if !(0..=2).contains(&extract_type) {
        return Err(value_error(
            f,
            3,
            "type",
            "must be one of GRAPHEME_EXTR_COUNT, GRAPHEME_EXTR_MAXBYTES, or GRAPHEME_EXTR_MAXCHARS",
        ));
    }
    if lstart > i32::MAX as i64 || lstart < 0 || lstart as usize >= s.len() {
        error_set(f, U_ILLEGAL_ARGUMENT_ERROR, "start not contained in string");
        return Ok(Value::Bool(false));
    }
    if size < 0 {
        return Err(value_error(
            f,
            2,
            "size",
            "must be greater than or equal to 0",
        ));
    }
    if size > i32::MAX as i64 {
        return Err(value_error(f, 2, "size", "is too large"));
    }
    if size == 0 {
        return Ok(Value::str(""));
    }
    let mut start = lstart as usize;
    // move forward off a continuation byte
    let is_single_or_lead = |c: u8| c < 0x80 || (0xc2..=0xf4).contains(&c);
    while !is_single_or_lead(s[start]) {
        start += 1;
        if start >= s.len() {
            error_set(
                f,
                U_ILLEGAL_ARGUMENT_ERROR,
                "grapheme_extract: invalid input string",
            );
            return Ok(Value::Bool(false));
        }
    }
    let p = &s[start..];
    let size = size as usize;
    let check_len = (size + 1).min(p.len());
    if p[..check_len].iter().all(|&c| c <= 0x7f) && ascii_check_quirk(p, check_len) {
        let nsize = size.min(p.len());
        if has_next {
            set_next((start + nsize) as i64);
        }
        return Ok(Value::str(p[..nsize].to_vec()));
    }
    let b = breaks_bytes(p);
    let mut ret = 0usize;
    match extract_type {
        0 => {
            // COUNT
            let mut left = size;
            for &np in &b[1..] {
                if left == 0 {
                    break;
                }
                ret = np;
                left -= 1;
            }
        }
        1 => {
            // MAXBYTES
            for &np in &b[1..] {
                if np > size {
                    break;
                }
                ret = np;
            }
        }
        _ => {
            // MAXCHARS
            let mut count = 0usize;
            let mut csize = size;
            for &np in &b[1..] {
                let mut bp = ret;
                while bp < np {
                    count += 1;
                    let prev = bp;
                    bp = u8_fwd_1(p, bp);
                    if prev == bp {
                        csize = 0;
                        break;
                    }
                }
                if count > csize {
                    break;
                }
                ret = bp;
            }
        }
    }
    if has_next {
        set_next((start + ret) as i64);
    }
    Ok(Value::str(p[..ret].to_vec()))
}

/// The CRLF part of `grapheme_ascii_check` over the first `n` bytes (it peeks
/// one byte past the range, like the C loop).
fn ascii_check_quirk(p: &[u8], n: usize) -> bool {
    for i in 0..n {
        if p[i] == b'\r' && p.get(i + 1) == Some(&b'\n') {
            return false;
        }
    }
    true
}

/// ICU's `U8_FWD_1`: advance over one (possibly ill-formed) UTF-8 sequence.
pub(super) fn u8_fwd_1(s: &[u8], mut i: usize) -> usize {
    let n = s.len();
    if i >= n {
        return i;
    }
    let b = s[i];
    i += 1;
    if !(0xc2..=0xf4).contains(&b) {
        return i;
    }
    let (count, lo2, hi2) = match b {
        0xc2..=0xdf => (1, 0x80, 0xbf),
        0xe0 => (2, 0xa0, 0xbf),
        0xed => (2, 0x80, 0x9f),
        0xe1..=0xef => (2, 0x80, 0xbf),
        0xf0 => (3, 0x90, 0xbf),
        0xf4 => (3, 0x80, 0x8f),
        _ => (3, 0x80, 0xbf),
    };
    if i < n && (lo2..=hi2).contains(&s[i]) {
        i += 1;
        let mut k = 1;
        while k < count && i < n && (0x80..=0xbf).contains(&s[i]) {
            i += 1;
            k += 1;
        }
    }
    i
}

pub(super) fn fn_levenshtein(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "grapheme_levenshtein";
    let mut s1 = str_arg(host, args, 0, f, "string1")?;
    let mut s2 = str_arg(host, args, 1, f, "string2")?;
    let ins = int_arg_or(host, args, 2, f, "insertion_cost", 1)?;
    let rep = int_arg_or(host, args, 3, f, "replacement_cost", 1)?;
    let del = int_arg_or(host, args, 4, f, "deletion_cost", 1)?;
    path_arg(host, args, 5, f, "locale")?;
    let max = (u32::MAX / 4) as i64;
    for (n, name, c) in [
        (3, "insertion_cost", ins),
        (4, "replacement_cost", rep),
        (5, "deletion_cost", del),
    ] {
        if c <= 0 || c > max {
            return Err(value_error(
                f,
                n,
                name,
                &format!("must be greater than 0 and less than or equal to {max}"),
            ));
        }
    }
    if s1.len() < s2.len() && ins == rep && rep == del {
        std::mem::swap(&mut s1, &mut s2);
    }
    let (Some(t1), Some(t2)) = (as_utf8(&s1), as_utf8(&s2)) else {
        error_set(
            f,
            U_INVALID_CHAR_FOUND,
            "Error converting input string to UTF-16",
        );
        return Ok(Value::Bool(false));
    };
    let b1 = breaks(t1);
    let b2 = breaks(t2);
    let g1: Vec<Vec<u32>> = b1.windows(2).map(|w| key(&t1[w[0]..w[1]], false)).collect();
    let g2: Vec<Vec<u32>> = b2.windows(2).map(|w| key(&t2[w[0]..w[1]], false)).collect();
    if g1.is_empty() {
        return Ok(Value::Int(g2.len() as i64 * ins));
    }
    if g2.is_empty() {
        return Ok(Value::Int(g1.len() as i64 * del));
    }
    let mut p1: Vec<i64> = (0..=g2.len() as i64).map(|i| i * ins).collect();
    let mut p2 = vec![0i64; g2.len() + 1];
    for a in &g1 {
        p2[0] = p1[0] + del;
        for (i2, bb) in g2.iter().enumerate() {
            let mut c0 = p1[i2] + if a == bb { 0 } else { rep };
            let c1 = p1[i2 + 1] + del;
            if c1 < c0 {
                c0 = c1;
            }
            let c2 = p2[i2] + ins;
            if c2 < c0 {
                c0 = c2;
            }
            p2[i2 + 1] = c0;
        }
        std::mem::swap(&mut p1, &mut p2);
    }
    Ok(Value::Int(p1[g2.len()]))
}
