//! `grapheme_*` — extended-grapheme-cluster string functions, ported from
//! php-src `ext/intl/grapheme/grapheme_string.c` and `grapheme_util.c` onto
//! ICU's character break iterator (`ubrk`) and collation-based string
//! search (`usearch`).

use std::cell::Cell;
use std::ffi::CString;
use std::marker::PhantomData;
use std::ptr;

use rhp_values::{PhpArray, Value};

use super::sys::{self, UBreakIterator, UChar, UErrorCode, UBRK_DONE, USEARCH_DONE};
use super::{
    error_set, error_set_code, error_set_msg, has_oom, path_arg, utf16_to_utf8, utf8_to_utf16,
};
use crate::zend::zpp::{bool_arg_or, int_arg, int_arg_or, opt_int_arg, str_arg, value_error};
use crate::{Host, Thrown};

// ---------------------------------------------------------------------------
// ICU handles
// ---------------------------------------------------------------------------

/// The thread's character break iterator (php's `INTL_G(grapheme_iterator)`),
/// opened on first use; every call works on a clone.
struct Global(Cell<*mut UBreakIterator>);

impl Drop for Global {
    fn drop(&mut self) {
        let p = self.0.get();
        if !p.is_null() {
            // SAFETY: opened by ubrk_open below, closed once.
            unsafe { sys::ubrk_close(p) };
        }
    }
}

thread_local! {
    static GLOBAL: Global = const { Global(Cell::new(ptr::null_mut())) };
}

/// A break iterator over text that outlives it (`'t`).
struct Bi<'t> {
    p: *mut UBreakIterator,
    _text: PhantomData<&'t [UChar]>,
}

impl Drop for Bi<'_> {
    fn drop(&mut self) {
        // SAFETY: owned clone, closed once.
        unsafe { sys::ubrk_close(self.p) };
    }
}

impl<'t> Bi<'t> {
    /// `grapheme_get_break_iterator`: a clone of the thread's iterator.
    fn get(status: &mut UErrorCode) -> Option<Bi<'t>> {
        let global = GLOBAL.with(|g| {
            let mut p = g.0.get();
            if p.is_null() {
                // SAFETY: no text; the locale has no effect on this iterator.
                p = unsafe {
                    sys::ubrk_open(sys::UBRK_CHARACTER, ptr::null(), ptr::null(), 0, status)
                };
                g.0.set(p);
            }
            p
        });
        // SAFETY: ubrk_clone checks `status` first (and so a failed open).
        let p = unsafe { sys::ubrk_clone(global, status) };
        (!p.is_null()).then_some(Bi {
            p,
            _text: PhantomData,
        })
    }

    fn set_text(&mut self, text: &'t [UChar], status: &mut UErrorCode) {
        // SAFETY: `text` outlives the iterator.
        unsafe { sys::ubrk_setText(self.p, text.as_ptr(), text.len() as i32, status) };
    }

    fn set_utext(&mut self, text: &Utf8Text<'t>, status: &mut UErrorCode) {
        // SAFETY: the iterator keeps a shallow clone of the UText, whose
        // bytes outlive it.
        unsafe { sys::ubrk_setUText(self.p, text.ut, status) };
    }

    fn first(&mut self) -> i32 {
        // SAFETY: a live iterator.
        unsafe { sys::ubrk_first(self.p) }
    }
    fn last(&mut self) -> i32 {
        // SAFETY: a live iterator.
        unsafe { sys::ubrk_last(self.p) }
    }
    fn next(&mut self) -> i32 {
        // SAFETY: a live iterator.
        unsafe { sys::ubrk_next(self.p) }
    }
    fn previous(&mut self) -> i32 {
        // SAFETY: a live iterator.
        unsafe { sys::ubrk_previous(self.p) }
    }
    fn current(&self) -> i32 {
        // SAFETY: a live iterator.
        unsafe { sys::ubrk_current(self.p) }
    }
    fn following(&mut self, offset: i32) -> i32 {
        // SAFETY: a live iterator.
        unsafe { sys::ubrk_following(self.p, offset) }
    }
    fn is_boundary(&mut self, offset: i32) -> bool {
        // SAFETY: a live iterator.
        unsafe { sys::ubrk_isBoundary(self.p, offset) != 0 }
    }
}

/// `utext_openUTF8` over bytes that outlive it.
struct Utf8Text<'t> {
    ut: *mut sys::UText,
    _text: PhantomData<&'t [u8]>,
}

impl<'t> Utf8Text<'t> {
    fn open(s: &'t [u8], status: &mut UErrorCode) -> Option<Utf8Text<'t>> {
        // SAFETY: ICU allocates the UText; `s` outlives it.
        let ut = unsafe {
            sys::utext_openUTF8(ptr::null_mut(), s.as_ptr().cast(), s.len() as i64, status)
        };
        (!sys::failure(*status) && !ut.is_null()).then_some(Utf8Text {
            ut,
            _text: PhantomData,
        })
    }
}

impl Drop for Utf8Text<'_> {
    fn drop(&mut self) {
        // SAFETY: opened above, closed once.
        unsafe { sys::utext_close(self.ut) };
    }
}

/// A `usearch` over text and a break iterator that outlive it.
struct Search<'t> {
    p: *mut sys::UStringSearch,
    _text: PhantomData<&'t [UChar]>,
}

impl Drop for Search<'_> {
    fn drop(&mut self) {
        // SAFETY: owned, closed once (before the break iterator it uses).
        unsafe { sys::usearch_close(self.p) };
    }
}

/// The `$locale` argument as a C string ("" is ICU's root locale).
fn c_locale(locale: &[u8]) -> CString {
    CString::new(locale).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// grapheme_util.c
// ---------------------------------------------------------------------------

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

/// `php_memnstr(h + from, n, len, h + h.len())`.
fn memnstr(h: &[u8], from: usize, n: &[u8]) -> Option<usize> {
    if n.is_empty() {
        return Some(from);
    }
    if from > h.len() || n.len() > h.len() - from {
        return None;
    }
    (from..=h.len() - n.len()).find(|&i| &h[i..i + n.len()] == n)
}

/// `grapheme_get_haystack_offset`: the boundary `offset` graphemes from the
/// start (or `-offset` from the end), or -1.
fn haystack_offset(bi: &mut Bi, offset: i32) -> i32 {
    if offset == 0 {
        return 0;
    }
    let back = offset < 0;
    if back {
        bi.last();
    }
    let mut offset = offset;
    let mut pos = 0;
    while pos != UBRK_DONE && offset != 0 {
        pos = if back { bi.previous() } else { bi.next() };
        if pos != UBRK_DONE {
            offset += if back { 1 } else { -1 };
        }
    }
    if offset != 0 {
        -1
    } else {
        pos
    }
}

/// `grapheme_count_graphemes`: points `bi` at `text` and counts.
fn count_graphemes<'t>(bi: &mut Bi<'t>, text: &'t [UChar]) -> i32 {
    let mut status = sys::U_ZERO_ERROR;
    bi.set_text(text, &mut status);
    let mut n = 0;
    while bi.next() != UBRK_DONE {
        n += 1;
    }
    n
}

/// `grapheme_split_string(text, NULL, 0)`: the number of graphemes, or -1.
fn split_string(text: &[UChar]) -> i32 {
    let mut status = sys::U_ZERO_ERROR;
    let Some(mut bi) = Bi::get(&mut status) else {
        return -1;
    };
    if sys::failure(status) {
        return -1;
    }
    bi.set_text(text, &mut status);
    let mut n = 0;
    while bi.next() != UBRK_DONE {
        n += 1;
    }
    n
}

/// `grapheme_strpos_utf16`: `Some((grapheme position, code point position))`
/// (the second is -1 for an empty needle), `None` for -1.
fn strpos_utf16(
    fname: &str,
    haystack: &[u8],
    needle: &[u8],
    offset: i32,
    ignore_case: bool,
    last: bool,
    locale: &[u8],
) -> Result<Option<(i32, i32)>, Thrown> {
    if haystack.is_empty() && !needle.is_empty() {
        return Ok(None);
    }
    // STRPOS_CHECK_STATUS
    let fail = |status: UErrorCode, msg: &str| -> Result<Option<(i32, i32)>, Thrown> {
        error_set_code(status);
        error_set_msg(fname, msg);
        Ok(None)
    };
    let offset_error = || {
        value_error(
            fname,
            3,
            "offset",
            "must be contained in argument #1 ($haystack)",
        )
    };
    let uhaystack = match utf8_to_utf16(haystack) {
        Ok(u) => u,
        Err(st) => return fail(st, "Error converting input string to UTF-16"),
    };
    let uneedle = match utf8_to_utf16(needle) {
        Ok(u) => u,
        Err(st) => return fail(st, "Error converting needle string to UTF-16"),
    };
    let mut status = sys::U_ZERO_ERROR;
    let bi = Bi::get(&mut status);
    if sys::failure(status) {
        return fail(status, "Failed to get iterator");
    }
    let Some(mut bi) = bi else {
        return fail(sys::U_ILLEGAL_ARGUMENT_ERROR, "Failed to get iterator");
    };
    bi.set_text(&uhaystack, &mut status);
    if sys::failure(status) {
        return fail(status, "Failed to set up iterator");
    }

    if uneedle.is_empty() {
        let offset_pos = haystack_offset(&mut bi, offset);
        if offset_pos == -1 {
            return Err(offset_error());
        }
        let n = if last && offset >= 0 {
            count_graphemes(&mut bi, &uhaystack)
        } else {
            count_graphemes(&mut bi, &uhaystack[..offset_pos as usize])
        };
        return Ok(Some((n, -1)));
    }

    status = sys::U_ZERO_ERROR;
    let loc = c_locale(locale);
    // SAFETY: needle, haystack and the break iterator outlive the search
    // (it is dropped first).
    let p = unsafe {
        sys::usearch_open(
            uneedle.as_ptr(),
            uneedle.len() as i32,
            uhaystack.as_ptr(),
            uhaystack.len() as i32,
            loc.as_ptr(),
            bi.p,
            &mut status,
        )
    };
    if sys::failure(status) || p.is_null() {
        return fail(status, "Error creating search object");
    }
    let src = Search {
        p,
        _text: PhantomData::<&[UChar]>,
    };

    if ignore_case {
        status = sys::U_ZERO_ERROR;
        // SAFETY: the collator belongs to the live search.
        unsafe {
            let coll = sys::usearch_getCollator(src.p);
            sys::ucol_setAttribute(coll, sys::UCOL_STRENGTH, sys::UCOL_SECONDARY, &mut status);
        }
        if sys::failure(status) {
            return fail(status, "Error setting collation strength");
        }
        // SAFETY: a live search.
        unsafe { sys::usearch_reset(src.p) };
    }

    let mut offset_pos = 0;
    if offset != 0 {
        offset_pos = haystack_offset(&mut bi, offset);
        if offset_pos == -1 {
            return Err(offset_error());
        }
        status = sys::U_ZERO_ERROR;
        // SAFETY: a live search.
        unsafe { sys::usearch_setOffset(src.p, if last { 0 } else { offset_pos }, &mut status) };
        if sys::failure(status) {
            return fail(status, "Invalid search offset");
        }
    }

    let char_pos = if last {
        if offset >= 0 {
            // SAFETY: a live search.
            let p = unsafe { sys::usearch_last(src.p, &mut status) };
            if p < offset_pos {
                USEARCH_DONE
            } else {
                p
            }
        } else {
            // php: searching backwards is broken, so search forwards.
            let mut prev = USEARCH_DONE;
            loop {
                // SAFETY: a live search.
                let p = unsafe { sys::usearch_next(src.p, &mut status) };
                if p == USEARCH_DONE || p > offset_pos {
                    break prev;
                }
                prev = p;
                let next = bi.following(p);
                if next == UBRK_DONE {
                    break p;
                }
                // SAFETY: a live search.
                unsafe { sys::usearch_setOffset(src.p, next, &mut status) };
                if sys::failure(status) {
                    return fail(status, "Invalid search offset");
                }
            }
        }
    } else {
        // SAFETY: a live search.
        unsafe { sys::usearch_next(src.p, &mut status) }
    };
    if sys::failure(status) {
        return fail(status, "Error looking up string");
    }
    if char_pos != USEARCH_DONE && bi.is_boundary(char_pos) {
        let n = count_graphemes(&mut bi, &uhaystack[..char_pos as usize]);
        // SAFETY: `char_pos` lies within the haystack.
        let cps = unsafe { sys::u_countChar32(uhaystack.as_ptr(), char_pos) };
        return Ok(Some((n, cps)));
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// grapheme_string.c
// ---------------------------------------------------------------------------

struct PosArgs {
    h: Vec<u8>,
    n: Vec<u8>,
    offset: i32,
    locale: Vec<u8>,
}

fn pos_args(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<PosArgs, Thrown> {
    let h = str_arg(host, args, 0, fname, "haystack")?;
    let n = str_arg(host, args, 1, fname, "needle")?;
    let offset = int_arg_or(host, args, 2, fname, "offset", 0)?;
    let locale = path_arg(host, args, 3, fname, "locale")?;
    if outside_string(offset, h.len()) {
        return Err(value_error(
            fname,
            3,
            "offset",
            "must be contained in argument #1 ($haystack)",
        ));
    }
    Ok(PosArgs {
        h,
        n,
        offset: offset as i32,
        locale,
    })
}

fn ret_pos(r: Option<(i32, i32)>) -> Value {
    match r {
        Some((p, _)) if p >= 0 => Value::Int(p as i64),
        _ => Value::Bool(false),
    }
}

pub(super) fn fn_strlen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "grapheme_strlen";
    let s = str_arg(host, args, 0, f, "string")?;
    if ascii_check(&s) {
        return Ok(Value::Int(s.len() as i64));
    }
    let u = match utf8_to_utf16(&s) {
        Ok(u) => u,
        Err(st) => {
            error_set(f, st, "Error converting input string to UTF-16");
            return Ok(Value::Null);
        }
    };
    let n = split_string(&u);
    Ok(if n >= 0 {
        Value::Int(n as i64)
    } else {
        Value::Bool(false)
    })
}

pub(super) fn fn_strpos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "grapheme_strpos";
    let a = pos_args(host, args, f)?;
    if a.offset >= 0 && ascii_check(&a.h) {
        if let Some(p) = memnstr(&a.h, a.offset as usize, &a.n) {
            return Ok(Value::Int(p as i64));
        }
        if ascii_check(&a.n) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(ret_pos(strpos_utf16(
        f, &a.h, &a.n, a.offset, false, false, &a.locale,
    )?))
}

pub(super) fn fn_stripos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "grapheme_stripos";
    let a = pos_args(host, args, f)?;
    if ascii_check(&a.h) {
        let noffset = if a.offset >= 0 {
            a.offset
        } else {
            a.h.len() as i32 + a.offset
        };
        let hl = a.h.to_ascii_lowercase();
        let nl = a.n.to_ascii_lowercase();
        if let Some(p) = memnstr(&hl, noffset as usize, &nl) {
            return Ok(Value::Int(p as i64));
        }
        if ascii_check(&a.n) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(ret_pos(strpos_utf16(
        f, &a.h, &a.n, a.offset, true, false, &a.locale,
    )?))
}

/// `grapheme_strrpos_ascii` (borrowed from `strrpos`).
fn strrpos_ascii(h: &[u8], n: &[u8], offset: i32) -> Option<usize> {
    let hl = h.len() as i64;
    let nl = n.len() as i64;
    let offset = offset as i64;
    let (p, mut e) = if offset >= 0 {
        (offset, hl - nl)
    } else if nl > -offset {
        (0, hl - nl)
    } else {
        (0, hl + offset)
    };
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
    Ok(ret_pos(strpos_utf16(
        f, &a.h, &a.n, a.offset, false, true, &a.locale,
    )?))
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
    Ok(ret_pos(strpos_utf16(
        f, &a.h, &a.n, a.offset, true, true, &a.locale,
    )?))
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
    let mut start = lstart as i32;
    let mut length = length.unwrap_or(s.len() as i64);
    if length < i32::MIN as i64 || length > i32::MAX as i64 {
        return Err(value_error(f, 3, "length", "is too large"));
    }
    if ascii_check(&s) {
        // grapheme_substr_ascii
        if s.len() > i32::MAX as usize {
            error_set(f, sys::U_ILLEGAL_ARGUMENT_ERROR, "invalid parameters");
            return Ok(Value::Bool(false));
        }
        let slen = s.len() as i32;
        let mut fr = start;
        if fr < 0 {
            fr = (slen + fr).max(0);
        } else if fr > slen {
            fr = slen;
        }
        let mut l = length as i32;
        if l < 0 {
            l = ((slen - fr) + l).max(0);
        } else if l > slen - fr {
            l = slen - fr;
        }
        return Ok(Value::str(&s[fr as usize..(fr + l) as usize]));
    }
    let u = match utf8_to_utf16(&s) {
        Ok(u) => u,
        Err(st) => {
            error_set(f, st, "Error converting input string to UTF-16");
            return Ok(Value::Bool(false));
        }
    };
    let mut status = sys::U_ZERO_ERROR;
    let bi = Bi::get(&mut status);
    let Some(mut bi) = bi.filter(|_| !sys::failure(status)) else {
        return Ok(Value::Bool(false));
    };
    bi.set_text(&u, &mut status);

    let backwards = start < 0;
    if backwards {
        bi.last();
    }
    let mut sub_start = 0;
    while start != 0 {
        sub_start = if backwards { bi.previous() } else { bi.next() };
        if sub_start == UBRK_DONE {
            break;
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
    let out = |from: i32, to: usize| match utf16_to_utf8(&u[from as usize..to]) {
        Ok(b) => Value::str(b),
        Err(st) => {
            error_set(f, st, "Error converting output string to UTF-8");
            Value::Bool(false)
        }
    };
    if length >= s.len() as i32 as i64 {
        return Ok(out(sub_start, u.len()));
    }
    if length == 0 {
        return Ok(Value::str(""));
    }
    let backwards = length < 0;
    if backwards {
        bi.last();
    }
    let mut sub_end = 0;
    while length != 0 {
        sub_end = if backwards { bi.previous() } else { bi.next() };
        if sub_end == UBRK_DONE {
            break;
        }
        length += if backwards { 1 } else { -1 };
    }
    drop(bi);
    let sub_end = if sub_end == UBRK_DONE {
        if length < 0 {
            return Ok(Value::str(""));
        }
        u.len() as i32
    } else {
        sub_end
    };
    if sub_start > sub_end {
        return Ok(Value::str(""));
    }
    Ok(out(sub_start, sub_end as usize))
}

fn strstr_common(host: &mut dyn Host, args: &[Value], ignore_case: bool) -> Result<Value, Thrown> {
    let f = if ignore_case {
        "grapheme_stristr"
    } else {
        "grapheme_strstr"
    };
    let h = str_arg(host, args, 0, f, "haystack")?;
    let n = str_arg(host, args, 1, f, "needle")?;
    let part = bool_arg_or(host, args, 2, f, "beforeNeedle", false)?;
    let locale = path_arg(host, args, 3, f, "locale")?;
    if !ignore_case && ascii_check(&h) && ascii_check(&n) {
        return Ok(match memnstr(&h, 0, &n) {
            None => Value::Bool(false),
            Some(o) if part => Value::str(&h[..o]),
            Some(o) => Value::str(&h[o..]),
        });
    }
    let Some((_, uchar_pos)) = strpos_utf16(f, &h, &n, 0, ignore_case, false, &locale)? else {
        return Ok(Value::Bool(false));
    };
    // U8_FWD_N over the (valid UTF-8) haystack.
    let mut pos = 0;
    for _ in 0..uchar_pos.max(0) {
        pos = u8_fwd_1(&h, pos);
    }
    Ok(if part {
        Value::str(&h[..pos])
    } else {
        Value::str(&h[pos..])
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
    let mut status = sys::U_ZERO_ERROR;
    let Some(ut) = Utf8Text::open(&s, &mut status) else {
        error_set(f, status, "Error opening UTF-8 text");
        return Ok(Value::Bool(false));
    };
    status = sys::U_ZERO_ERROR;
    let bi = Bi::get(&mut status);
    let Some(mut bi) = bi.filter(|_| !sys::failure(status)) else {
        return Ok(Value::Bool(false));
    };
    bi.set_utext(&ut, &mut status);
    let split_len = split_len as i32;
    let (mut pstr, mut end) = (0usize, 0usize);
    let (mut pos, mut current, mut i, mut end_len) = (0i32, 0i32, 0i32, 0i32);
    while pos != UBRK_DONE {
        end_len = pos - current;
        pos = bi.next();
        if i == split_len - 1 {
            if pos != UBRK_DONE {
                let n = (pos - current) as usize;
                out.push(Value::str(&s[pstr..pstr + n]));
                if has_oom() {
                    return Ok(Value::Bool(false));
                }
                end = pstr + n;
                i = 0;
                pstr += n;
                current = pos;
            }
        } else {
            i += 1;
        }
    }
    if i != 0 && end_len != 0 {
        out.push(Value::str(&s[end..end + end_len as usize]));
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
    let set_next = |v: i64| crate::zend::zpp::set_ref(args, 4, Value::Int(v));
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
        error_set(
            f,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "start not contained in string",
        );
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
    // Off a continuation byte to the start of the next character.
    let single_or_lead = |c: u8| c < 0x80 || (0xc2..=0xf4).contains(&c);
    while !single_or_lead(s[start]) {
        start += 1;
        if start >= s.len() {
            error_set(
                f,
                sys::U_ILLEGAL_ARGUMENT_ERROR,
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
        return Ok(Value::str(&p[..nsize]));
    }
    let mut status = sys::U_ZERO_ERROR;
    let Some(ut) = Utf8Text::open(p, &mut status) else {
        error_set(f, status, "Error opening UTF-8 text");
        return Ok(Value::Bool(false));
    };
    status = sys::U_ZERO_ERROR;
    let Some(mut bi) = Bi::get(&mut status) else {
        return Ok(Value::Bool(false));
    };
    bi.set_utext(&ut, &mut status);
    let size = size as i32;
    let mut ret = 0i32;
    match extract_type {
        0 => {
            // grapheme_extract_count_iter
            let mut left = size;
            while left != 0 {
                let next = bi.next();
                if next == UBRK_DONE {
                    break;
                }
                ret = next;
                left -= 1;
            }
        }
        1 => loop {
            // grapheme_extract_bytecount_iter
            let pos = bi.next();
            if pos == UBRK_DONE || pos > size {
                break;
            }
            ret = pos;
        },
        _ => {
            // grapheme_extract_charcount_iter
            let mut csize = size;
            let mut count = 0;
            loop {
                let pos = bi.next();
                if pos == UBRK_DONE {
                    break;
                }
                let mut bp = ret as usize;
                while bp < pos as usize {
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
                ret = bp as i32;
            }
        }
    }
    drop(bi);
    if has_next {
        set_next((start + ret as usize) as i64);
    }
    Ok(Value::str(&p[..ret as usize]))
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

/// ICU's `U8_FWD_1` (a header macro): advance over one possibly ill-formed
/// UTF-8 sequence.
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

/// The owner of `ucol_open`'s result.
struct Collator(*mut sys::UCollator);

impl Drop for Collator {
    fn drop(&mut self) {
        // SAFETY: ucol_close accepts null; closed once.
        unsafe { sys::ucol_close(self.0) };
    }
}

pub(super) fn fn_levenshtein(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "grapheme_levenshtein";
    let mut s1 = str_arg(host, args, 0, f, "string1")?;
    let mut s2 = str_arg(host, args, 1, f, "string2")?;
    let ins = int_arg_or(host, args, 2, f, "insertion_cost", 1)?;
    let rep = int_arg_or(host, args, 3, f, "replacement_cost", 1)?;
    let del = int_arg_or(host, args, 4, f, "deletion_cost", 1)?;
    let locale = path_arg(host, args, 5, f, "locale")?;
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
    // Equal costs make the distance symmetric: keep the rows short.
    if s1.len() < s2.len() && ins == rep && rep == del {
        std::mem::swap(&mut s1, &mut s2);
    }
    let u1 = utf8_to_utf16(&s1);
    if has_oom() {
        return Ok(Value::Bool(false));
    }
    let u1 = match u1 {
        Ok(u) => u,
        Err(st) => {
            error_set(f, st, "Error converting input string to UTF-16");
            return Ok(Value::Bool(false));
        }
    };
    let u2 = utf8_to_utf16(&s2);
    if has_oom() {
        return Ok(Value::Bool(false));
    }
    let u2 = match u2 {
        Ok(u) => u,
        Err(st) => {
            error_set(f, st, "Error converting input string to UTF-16");
            return Ok(Value::Bool(false));
        }
    };
    let (len1, len2) = (split_string(&u1), split_string(&u2));
    if len1 < 0 || len2 < 0 {
        return Ok(Value::Bool(false));
    }
    if len1 == 0 {
        return Ok(Value::Int(len2 as i64 * ins));
    }
    if len2 == 0 {
        return Ok(Value::Int(len1 as i64 * del));
    }
    let mut status = sys::U_ZERO_ERROR;
    let bi1 = Bi::get(&mut status);
    let Some(mut bi1) = bi1.filter(|_| !sys::failure(status)) else {
        error_set(
            f,
            status,
            "Error on grapheme_get_break_iterator for argument #1 ($string1)",
        );
        return Ok(Value::Bool(false));
    };
    let bi2 = Bi::get(&mut status);
    let Some(mut bi2) = bi2.filter(|_| !sys::failure(status)) else {
        error_set(
            f,
            status,
            "Error on grapheme_get_break_iterator for argument #2 ($string2)",
        );
        return Ok(Value::Bool(false));
    };
    bi1.set_text(&u1, &mut status);
    if sys::failure(status) {
        error_set(
            f,
            status,
            "Error on ubrk_setText for argument #1 ($string1)",
        );
        return Ok(Value::Bool(false));
    }
    bi2.set_text(&u2, &mut status);
    if sys::failure(status) {
        error_set(
            f,
            status,
            "Error on ubrk_setText for argument #2 ($string2)",
        );
        return Ok(Value::Bool(false));
    }
    let loc = c_locale(&locale);
    // SAFETY: a C string locale.
    let coll = Collator(unsafe { sys::ucol_open(loc.as_ptr(), &mut status) });
    if sys::failure(status) {
        error_set(f, status, "Error on ucol_open");
        return Ok(Value::Bool(false));
    }

    let len2 = len2 as usize;
    // safe_femalloc(len2 + 1, sizeof(zend_long), 0, fallible), twice
    if !rhp_values::mem::reserve_safe(len2 + 1, 8, 0) {
        return Ok(Value::Bool(false));
    }
    let mut p1: Vec<i64> = (0..=len2 as i64).map(|i| i * ins).collect();
    if !rhp_values::mem::reserve_safe(len2 + 1, 8, 0) {
        return Ok(Value::Bool(false));
    }
    let mut p2: Vec<i64> = vec![0; len2 + 1];
    loop {
        let current1 = bi1.current();
        let pos1 = bi1.next();
        if pos1 == UBRK_DONE {
            break;
        }
        p2[0] = p1[0] + del;
        let mut i2 = 0;
        loop {
            let current2 = bi2.current();
            let pos2 = bi2.next();
            if pos2 == UBRK_DONE {
                break;
            }
            let g1 = &u1[current1 as usize..pos1 as usize];
            let g2 = &u2[current2 as usize..pos2 as usize];
            // SAFETY: both slices lie within their strings.
            let equal = unsafe {
                sys::ucol_strcoll(
                    coll.0,
                    g1.as_ptr(),
                    g1.len() as i32,
                    g2.as_ptr(),
                    g2.len() as i32,
                )
            } == sys::UCOL_EQUAL;
            let mut c0 = p1[i2] + if equal { 0 } else { rep };
            let c1 = p1[i2 + 1] + del;
            if c1 < c0 {
                c0 = c1;
            }
            let c2 = p2[i2] + ins;
            if c2 < c0 {
                c0 = c2;
            }
            p2[i2 + 1] = c0;
            i2 += 1;
        }
        bi2.first();
        std::mem::swap(&mut p1, &mut p2);
    }
    Ok(Value::Int(p1[len2]))
}
