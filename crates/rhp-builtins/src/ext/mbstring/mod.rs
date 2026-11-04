//! The mbstring extension: a port of php-src's ext/mbstring/mbstring.c
//! (PHP-3.01) over php's own libmbfl and php_unicode.c, which rhp-mbfl-sys
//! compiles from the pinned tag. Encodings, conversion, case mapping and
//! the width table are libmbfl's; this module is the glue mbstring.c puts
//! around them: argument handling, the per-request settings (MBSTRG), the
//! substring/position logic and encoding detection.
//!
//! Not ported yet: the functions rhp does not register (mb_ereg* and the
//! other mbstring.c functions without a counterpart here), the mbstring.*
//! ini handlers and php's internal-encoding hook (the internal encoding
//! starts as UTF-8 and doesn't follow default_charset), and php's
//! `IS_STR_VALID_UTF8` string flag (a shortcut
//! that gives the same result for UTF-8 itself; for the UTF-8-Mobile
//! variants php's answer can depend on it, and rhp takes the path php
//! takes for an unflagged string).

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use rhp_mbfl_sys::{self as mbfl, case_mode, encodings, illegal_mode, ConvertBuf, Encoding};
use rhp_values::{ArrayKey, PhpArray, PhpStr, Value};

use crate::ext::zpp::{self, deref};
use crate::{deprecated, warn, Builtin, Host, Thrown};

type Enc = &'static Encoding;

/// `MBFL_ENCTYPE_SBCS | MBFL_ENCTYPE_WCS2 | MBFL_ENCTYPE_WCS4`: the
/// fixed-width flags; their value is the character's byte width.
const FIXED_WIDTH: u32 = mbfl::ENCTYPE_SBCS | mbfl::ENCTYPE_WCS2 | mbfl::ENCTYPE_WCS4;
/// `MBFL_SUBSTR_UNTIL_END`.
const UNTIL_END: usize = usize::MAX;
/// `MBFL_ERROR_NOT_FOUND`.
const ERROR_NOT_FOUND: usize = usize::MAX;
/// `MBFL_ERROR_OFFSET`.
const ERROR_OFFSET: usize = usize::MAX - 15;

/// `mbfl_is_error`.
fn is_error(n: usize) -> bool {
    n >= ERROR_OFFSET
}

// ---------------------------------------------------------------------------
// per-request state (MBSTRG)
// ---------------------------------------------------------------------------

/// The module globals php's functions read and set.
struct State {
    /// `current_internal_encoding`.
    internal: Enc,
    /// `current_filter_illegal_mode`.
    illegal_mode: u32,
    /// `current_filter_illegal_substchar`.
    substchar: u32,
    /// `illegalchars`: unconvertible characters seen this request.
    illegalchars: usize,
    /// `last_used_encoding_name` / `last_used_encoding`.
    last_used: Option<(Vec<u8>, Enc)>,
    /// `current_detect_order_list`.
    detect_order: Vec<Enc>,
    /// `all_encodings_list`: the array `mb_list_encodings()` returns, kept
    /// so passing it back can be recognised.
    all_encodings: Option<PhpArray>,
}

impl State {
    fn new() -> State {
        State {
            internal: encodings::utf8(),
            illegal_mode: illegal_mode::CHAR,
            substchar: b'?' as u32,
            illegalchars: 0,
            last_used: None,
            detect_order: default_detect_order(),
            all_encodings: None,
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::new());
}

fn st<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

/// RINIT/RSHUTDOWN: a new request starts from the defaults.
pub(crate) fn reset_state() {
    st(|s| *s = State::new());
}

fn internal() -> Enc {
    st(|s| s.internal)
}

fn illegal_settings() -> (u32, u32) {
    st(|s| (s.illegal_mode, s.substchar))
}

/// `default_detect_order_list` for mbstring.language=neutral
/// (php_mb_default_identify_list_neut), which "auto" expands to.
fn default_detect_order() -> Vec<Enc> {
    vec![encodings::ascii(), encodings::utf8()]
}

/// `MBSTRG(strict_detection)`: the mbstring.strict_detection ini value.
fn strict_detection() -> bool {
    crate::ini_value(b"mbstring.strict_detection")
        .flatten()
        .is_some_and(|v| crate::runtime_fns::ini_parse_bool(&v))
}

/// php's `ZEND_HAS_OOM()`: an allocation crossed memory_limit, and the VM
/// raises the error once the builtin returns.
fn has_oom() -> bool {
    rhp_values::mem::state().attention()
}

// ---------------------------------------------------------------------------
// arguments
// ---------------------------------------------------------------------------

/// The name of parameter `i` (0-based) of `fname`, from its stub.
fn pname(fname: &str, i: usize) -> &'static str {
    crate::signatures::function(fname.as_bytes())
        .and_then(|f| f.params.get(i))
        .map_or("", |p| p.name)
}

fn value_error(fname: &str, i: usize, msg: &str) -> Thrown {
    zpp::value_error(fname, i + 1, pname(fname, i), msg)
}

/// `zend_argument_must_not_be_empty_error`.
fn empty_error(fname: &str, i: usize) -> Thrown {
    value_error(fname, i, "must not be empty")
}

/// A string argument: the caller's string, shared rather than copied, as
/// php's functions read their zend_string in place.
#[derive(Default)]
struct StrArg(PhpStr);

impl std::ops::Deref for StrArg {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// `Z_PARAM_STR`.
fn str_arg(host: &mut dyn Host, args: &[Value], i: usize, fname: &str) -> Result<StrArg, Thrown> {
    match args.get(i).map(deref) {
        Some(Value::Str(s)) => Ok(StrArg(s)),
        _ => zpp::str_arg(host, args, i, fname, pname(fname, i))
            .map(|b| StrArg(PhpStr::from_bytes(b))),
    }
}

/// `Z_PARAM_STR_OR_NULL`.
fn opt_str_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
) -> Result<Option<Vec<u8>>, Thrown> {
    zpp::opt_str_arg(host, args, i, fname, pname(fname, i))
}

/// `Z_PARAM_LONG`. The VM has already reported a null for a non-nullable
/// parameter.
fn long_arg(host: &mut dyn Host, args: &[Value], i: usize, fname: &str) -> Result<i64, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(0),
        Some(_) => zpp::long_arg(host, args, i, fname, pname(fname, i)),
    }
}

/// `Z_PARAM_LONG_OR_NULL`.
fn opt_long_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
) -> Result<Option<i64>, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => zpp::long_arg(host, args, i, fname, pname(fname, i)).map(Some),
    }
}

/// `Z_PARAM_BOOL`.
fn bool_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
    default: bool,
) -> Result<bool, Thrown> {
    zpp::bool_arg_or(host, args, i, fname, pname(fname, i), default)
}

/// A C string argument as php's `char *` APIs see it: up to the first NUL.
fn c_str(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]
}

/// `php_mb_get_encoding`: the named encoding (argument `i`, 0-based), or
/// the internal encoding when there is none. Base64, QPrint, HTML-ENTITIES
/// and UUENCODE raise their deprecation the first time in a row they are
/// named.
fn get_encoding(
    host: &mut dyn Host,
    name: Option<&[u8]>,
    i: usize,
    fname: &str,
) -> Result<Enc, Thrown> {
    let Some(name) = name else {
        return Ok(internal());
    };
    let cached = st(|s| {
        s.last_used
            .as_ref()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, e)| *e)
    });
    if let Some(enc) = cached {
        return Ok(enc);
    }
    let Some(enc) = Encoding::by_name(name) else {
        return Err(value_error(
            fname,
            i,
            &format!(
                "must be a valid encoding, \"{}\" given",
                String::from_utf8_lossy(c_str(name))
            ),
        ));
    };
    let mut notice = Ok(());
    if enc.no() <= mbfl::no::QPRINT {
        let msg = if enc == encodings::base64() {
            Some("Handling Base64 via mbstring is deprecated; use base64_encode/base64_decode instead")
        } else if enc == encodings::qprint() {
            Some("Handling QPrint via mbstring is deprecated; use quoted_printable_encode/quoted_printable_decode instead")
        } else if enc == encodings::html_ent() {
            Some("Handling HTML entities via mbstring is deprecated; use htmlspecialchars, htmlentities, or mb_encode_numericentity/mb_decode_numericentity instead")
        } else if enc == encodings::uuencode() {
            Some("Handling Uuencode via mbstring is deprecated; use convert_uuencode/convert_uudecode instead")
        } else {
            None
        };
        if let Some(msg) = msg {
            notice = deprecated(host, fname, msg);
        }
    }
    // Cached even when an error handler threw: php carries on and throws
    // once the builtin returns.
    st(|s| s.last_used = Some((name.to_vec(), enc)));
    notice.map(|()| enc)
}

/// An optional `?string $encoding` argument at `i`, resolved.
fn enc_arg(host: &mut dyn Host, args: &[Value], i: usize, fname: &str) -> Result<Enc, Thrown> {
    let name = opt_str_arg(host, args, i, fname)?;
    get_encoding(host, name.as_deref(), i, fname)
}

/// `php_mb_parse_encoding_list`: a comma-separated list (one pair of
/// enclosing double quotes allowed); "auto" (or any prefix of it, as php's
/// strncasecmp compares) adds the default detect order once.
fn parse_encoding_list(list: &[u8], fname: &str, i: usize) -> Result<Vec<Enc>, Thrown> {
    if list.is_empty() {
        return Ok(Vec::new());
    }
    let list = if list.len() > 2 && list[0] == b'"' && list[list.len() - 1] == b'"' {
        &list[1..list.len() - 1]
    } else {
        list
    };
    let defaults = default_detect_order();
    let size = 1 + list.iter().filter(|&&c| c == b',').count() + defaults.len();
    let mut out = Vec::new();
    let mut included_auto = false;
    let mut rest = list;
    loop {
        let comma = rest.iter().position(|&c| c == b',');
        let mut item = &rest[..comma.unwrap_or(rest.len())];
        while let [b' ' | b'\t', tail @ ..] = item {
            item = tail;
        }
        // php trims the end only while more than one byte is left.
        while item.len() > 1 && matches!(item[item.len() - 1], b' ' | b'\t') {
            item = &item[..item.len() - 1];
        }
        if strncasecmp_eq(item, b"auto") {
            if !included_auto {
                included_auto = true;
                out.extend_from_slice(&defaults);
            }
        } else {
            let Some(enc) = Encoding::by_name_ex(item) else {
                return Err(value_error(
                    fname,
                    i,
                    &format!(
                        "contains invalid encoding \"{}\"",
                        String::from_utf8_lossy(c_str(item))
                    ),
                ));
            };
            out.push(enc);
        }
        match comma {
            Some(c) if out.len() < size => rest = &rest[c + 1..],
            _ => break,
        }
    }
    Ok(out)
}

/// `strncasecmp(s, lit, s.len()) == 0`, as C compares: up to `s.len()`
/// bytes, stopping at a NUL in either string.
fn strncasecmp_eq(s: &[u8], lit: &[u8]) -> bool {
    for (k, &a) in s.iter().enumerate() {
        let b = lit.get(k).copied().unwrap_or(0);
        if !a.eq_ignore_ascii_case(&b) {
            return false;
        }
        if a == 0 {
            return true;
        }
    }
    true
}

/// `php_mb_parse_encoding_array`: names from an array; "auto" adds the
/// default detect order once.
fn parse_encoding_array(
    host: &mut dyn Host,
    a: &PhpArray,
    fname: &str,
    i: usize,
) -> Result<Vec<Enc>, Thrown> {
    let mut out = Vec::new();
    let mut included_auto = false;
    for (_, v) in a.iter() {
        let name = crate::argparse::zval_get_string(host, v)?;
        if name.eq_ignore_ascii_case(b"auto") {
            if !included_auto {
                included_auto = true;
                out.extend(default_detect_order());
            }
        } else {
            match Encoding::by_name(&name) {
                Some(enc) => out.push(enc),
                None => {
                    return Err(value_error(
                        fname,
                        i,
                        &format!(
                            "contains invalid encoding \"{}\"",
                            String::from_utf8_lossy(c_str(&name))
                        ),
                    ))
                }
            }
        }
    }
    Ok(out)
}

/// An `array|string|null` encoding-list argument: `None` when null or
/// absent; `Some((list, is_all_encodings))` otherwise.
fn enc_list_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
) -> Result<Option<(Vec<Enc>, bool)>, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => {
            let all = st(|s| {
                s.all_encodings
                    .as_ref()
                    .is_some_and(|l| l.data_ptr() == a.data_ptr())
            });
            Ok(Some((parse_encoding_array(host, &a, fname, i)?, all)))
        }
        Some(_) => {
            let s = str_arg(host, args, i, fname)?;
            Ok(Some((parse_encoding_list(&s, fname, i)?, false)))
        }
    }
}

/// `remove_non_encodings_from_elist`: Base64, QPrint and the other byte
/// encodings are never detected.
fn remove_non_encodings(list: &mut Vec<Enc>) {
    list.retain(|e| e.no() > mbfl::no::CHARSET_MIN);
}

/// `php_mb_is_no_encoding_utf8`: UTF-8 and its mobile variants.
fn is_utf8(enc: Enc) -> bool {
    (mbfl::no::UTF8..=mbfl::no::UTF8_SB).contains(&enc.no())
}

/// `php_mb_is_unsupported_no_encoding` (mb_ord, mb_chr).
fn is_unsupported(enc: Enc) -> bool {
    let n = enc.no();
    (mbfl::no::INVALID..=mbfl::no::QPRINT).contains(&n)
        || (mbfl::no::UTF7..=mbfl::no::UTF7IMAP).contains(&n)
        || (mbfl::no::JIS..=mbfl::no::ISO2022JPMS).contains(&n)
        || (mbfl::no::CP50220..=mbfl::no::CP50222).contains(&n)
}

// ---------------------------------------------------------------------------
// conversion helpers
// ---------------------------------------------------------------------------

/// The codepoints of `input`, 128 at a time, as `to_wchar` delivers them.
fn for_each_chunk(enc: Enc, input: &[u8], mut f: impl FnMut(&[u32], bool)) {
    let mut buf = [0u32; 128];
    let mut rest = input;
    let mut state = 0;
    while !rest.is_empty() {
        let n = enc.to_wchar(&mut rest, &mut buf, &mut state);
        f(&buf[..n], rest.is_empty());
    }
}

/// `php_mb_convert_encoding_ex`: with the current substitute settings,
/// counting unconvertible characters in `illegalchars`.
fn convert_ex(input: &[u8], to: Enc, from: Enc) -> Vec<u8> {
    let (mode, subst) = illegal_settings();
    let (out, errors) = mbfl::fast_convert(input, from, to, subst, mode);
    st(|s| s.illegalchars += errors as usize);
    out
}

/// To UTF-8 with 0xFF marking each bad byte sequence, as mbstring.c does
/// before searching.
fn to_utf8_marked(input: &[u8], enc: Enc) -> Vec<u8> {
    mbfl::fast_convert(input, enc, encodings::utf8(), 0, illegal_mode::BADUTF8).0
}

/// `mb_fast_strlen_utf8`: bytes that don't continue a UTF-8 sequence.
fn fast_strlen_utf8(s: &[u8]) -> usize {
    s.iter().filter(|&&c| (c as i8) >= -64).count()
}

/// `mb_get_strlen`.
fn get_strlen(s: &[u8], enc: Enc) -> usize {
    let char_len = enc.flag() & FIXED_WIDTH;
    if char_len != 0 {
        return s.len() / char_len as usize;
    }
    // php takes this path for strings it knows are valid UTF-8; the count
    // is the same as decoding them.
    if enc == encodings::utf8() && std::str::from_utf8(s).is_ok() {
        return fast_strlen_utf8(s);
    }
    let mut len = 0;
    for_each_chunk(enc, s, |w, _| len += w.len());
    len
}

/// `mb_get_substr_slow`.
fn get_substr_slow(input: &[u8], mut from: usize, mut len: usize, enc: Enc) -> Vec<u8> {
    let (mode, subst) = illegal_settings();
    // php sizes the buffer with `in_len - from`, which wraps when `from` is
    // past the end (possible only for SJIS-mac, and then it under-allocates);
    // the buffer grows as needed either way.
    let mut buf = ConvertBuf::new(len.min(input.len().saturating_sub(from)), subst, mode);
    let mut wchar = [0u32; 128];
    let mut rest = input;
    let mut state = 0;
    while !rest.is_empty() && len != 0 {
        let out_len = enc.to_wchar(&mut rest, &mut wchar, &mut state);
        if from >= out_len {
            from -= out_len;
        } else {
            let needed = (out_len - from).min(len);
            enc.from_wchar(
                &wchar[from..from + needed],
                &mut buf,
                rest.is_empty() || out_len >= len,
            );
            from = 0;
            len -= needed;
        }
    }
    buf.result(enc)
}

/// `mb_get_substr`: `len` characters from character `from`.
fn get_substr(input: &[u8], from: usize, len: usize, enc: Enc) -> Vec<u8> {
    if len == 0 || (from >= input.len() && enc != encodings::sjis_mac()) {
        // Only MacJapanese decodes to more than one codepoint per byte.
        return Vec::new();
    }
    let flag = enc.flag() & FIXED_WIDTH;
    if flag != 0 {
        let from = from.wrapping_mul(flag as usize);
        let len = len.wrapping_mul(flag as usize);
        if from >= input.len() {
            return Vec::new();
        }
        let rest = &input[from..];
        return rest[..len.min(rest.len())].to_vec();
    }
    get_substr_slow(input, from, len, enc)
}

/// `offset_to_pointer_utf8`: the byte position `offset` characters from
/// the start (negative: from the end), or None when out of range.
fn offset_to_pointer_utf8(s: &[u8], offset: isize) -> Option<usize> {
    if offset < 0 {
        let mut pos = s.len();
        let mut offset = offset;
        while offset < 0 {
            if pos == 0 {
                return None;
            }
            pos -= 1;
            let c = s[pos];
            if c < 0x80 || (c & 0xC0) != 0x80 {
                offset += 1;
            }
        }
        Some(pos)
    } else {
        let tbl = encodings::utf8()
            .mblen_table()
            .expect("UTF-8 has an mblen table");
        let mut pos = 0usize;
        for _ in 0..offset {
            if pos >= s.len() {
                return None;
            }
            pos += tbl[s[pos] as usize] as usize;
        }
        Some(pos.min(s.len()))
    }
}

/// `zend_memnstr`: the first occurrence of `needle` in `h[from..end]`.
fn memnstr(h: &[u8], from: usize, needle: &[u8], end: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from);
    }
    let hay = &h[from..end];
    if needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

/// `zend_memnrstr`: the last occurrence of `needle` that ends by `end`
/// and starts at `from` or later.
fn memnrstr(h: &[u8], from: usize, needle: &[u8], end: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(end);
    }
    if end < from || needle.len() > end - from {
        return None;
    }
    h[from..end]
        .windows(needle.len())
        .rposition(|w| w == needle)
        .map(|p| from + p)
}

/// `mb_find_strpos`: the character position of `needle`, or an
/// `MBFL_ERROR_*` value.
fn find_strpos(haystack: &[u8], needle: &[u8], enc: Enc, offset: isize, reverse: bool) -> usize {
    let (h, n);
    let (h, n): (&[u8], &[u8]) = if !is_utf8(enc) {
        h = to_utf8_marked(haystack, enc);
        n = to_utf8_marked(needle, enc);
        (&h, &n)
    } else {
        (haystack, needle)
    };
    let Some(off_ptr) = offset_to_pointer_utf8(h, offset) else {
        return ERROR_OFFSET;
    };
    if h.len() < n.len() {
        return ERROR_NOT_FOUND;
    }
    let found = if !reverse {
        memnstr(h, off_ptr, n, h.len())
    } else if offset >= 0 {
        memnrstr(h, off_ptr, n, h.len())
    } else {
        let needle_len = fast_strlen_utf8(n);
        let end = offset_to_pointer_utf8(&h[off_ptr..], needle_len as isize)
            .map_or(h.len(), |p| off_ptr + p);
        memnrstr(h, 0, n, end)
    };
    match found {
        Some(p) => fast_strlen_utf8(&h[..p]),
        None => ERROR_NOT_FOUND,
    }
}

/// `php_mb_stripos`: simple case folding to UTF-8, then [`find_strpos`].
fn stripos(reverse: bool, haystack: &[u8], needle: &[u8], offset: isize, enc: Enc) -> usize {
    let fold = |s: &[u8]| {
        mbfl::convert_case(
            case_mode::FOLD_SIMPLE,
            s,
            enc,
            encodings::utf8(),
            illegal_mode::BADUTF8,
            0,
        )
    };
    find_strpos(
        &fold(haystack),
        &fold(needle),
        encodings::utf8(),
        offset,
        reverse,
    )
}

/// `handle_strpos_error`.
fn strpos_error(fname: &str, n: usize) -> Result<Value, Thrown> {
    if n == ERROR_OFFSET {
        return Err(value_error(
            fname,
            2,
            "must be contained in argument #1 ($haystack)",
        ));
    }
    Ok(Value::Bool(false))
}

/// `character_width`.
fn character_width(c: u32) -> usize {
    if c < mbfl::FIRST_DOUBLEWIDTH_CODEPOINT {
        return 1;
    }
    let t = mbfl::eaw_table();
    let (mut lo, mut hi) = (0, t.len());
    while lo < hi {
        let probe = (lo + hi) / 2;
        if c < t[probe][0] as u32 {
            hi = probe;
        } else if c > t[probe][1] as u32 {
            lo = probe + 1;
        } else {
            return 2;
        }
    }
    1
}

/// `mb_get_strwidth`.
fn get_strwidth(s: &[u8], enc: Enc) -> usize {
    let mut width = 0;
    for_each_chunk(enc, s, |w, _| {
        width += w.iter().map(|&c| character_width(c)).sum::<usize>()
    });
    width
}

/// `mbstring_convert_case`, with the current substitute settings.
fn convert_case(mode: i32, s: &[u8], enc: Enc) -> Vec<u8> {
    let (illegal, subst) = illegal_settings();
    mbfl::convert_case(mode, s, enc, enc, illegal, subst)
}

/// `php_mb_check_encoding`.
fn check_encoding(s: &[u8], enc: Enc) -> bool {
    if let Some(ok) = enc.check(s) {
        return ok;
    }
    // A wrong encoding usually shows in the first few characters.
    let mut buf = [0u32; 128];
    let mut rest = s;
    let mut state = 0;
    let n = enc.to_wchar(&mut rest, &mut buf[..8], &mut state);
    if buf[..n].contains(&mbfl::BAD_INPUT) {
        return false;
    }
    while !rest.is_empty() {
        let n = enc.to_wchar(&mut rest, &mut buf, &mut state);
        if buf[..n].contains(&mbfl::BAD_INPUT) {
            return false;
        }
    }
    true
}

/// `mb_check_str_encoding`.
fn check_str_encoding(s: &[u8], enc: Enc) -> bool {
    if enc == encodings::utf8() {
        std::str::from_utf8(s).is_ok()
    } else {
        check_encoding(s, enc)
    }
}

// ---------------------------------------------------------------------------
// encoding detection
// ---------------------------------------------------------------------------

/// `estimate_demerits`.
fn estimate_demerits(w: u32) -> u64 {
    if w > 0xFFFF {
        40
    } else if (0x21..=0x2F).contains(&w) {
        6
    } else if (mbfl::rare_cp_bitvec()[(w >> 5) as usize] >> (w & 0x1F)) & 1 != 0 {
        30
    } else {
        1
    }
}

/// `struct candidate`.
#[derive(Clone)]
struct Candidate<'a> {
    enc: Enc,
    input: &'a [u8],
    demerits: u64,
    state: u32,
    multiplier: f32,
}

/// `mb_guess_encoding_for_strings`.
fn guess_encoding(
    strings: &[&[u8]],
    list: &[Enc],
    strict: bool,
    order_significant: bool,
) -> Option<Enc> {
    if list.is_empty() {
        return None;
    }
    if list.len() == 1 {
        if strict && strings.iter().rev().any(|s| !check_encoding(s, list[0])) {
            return None;
        }
        return Some(list[0]);
    }
    if strings.len() == 1 && strings[0].is_empty() {
        return Some(list[0]);
    }
    // init_candidate_array
    let mut cands: Vec<Candidate> = Vec::with_capacity(list.len());
    'next: for (i, &enc) in list.iter().enumerate() {
        let mut demerits = 0;
        if enc.has_check() {
            for s in strings {
                if enc.check(s) == Some(false) {
                    if strict {
                        continue 'next;
                    }
                    demerits += 500;
                }
            }
        }
        let multiplier = if order_significant {
            (1.0 + (0.3 * i as f64) / list.len() as f64) as f32
        } else {
            1.0
        };
        cands.push(Candidate {
            enc,
            input: &[],
            demerits,
            state: 0,
            multiplier,
        });
    }
    for s in strings.iter().rev() {
        // start_string: skip the byte order mark of UTF-8/16
        for c in cands.iter_mut() {
            c.input = s;
            if c.enc == encodings::utf8() {
                if s.starts_with(b"\xEF\xBB\xBF") {
                    c.input = &s[3..];
                }
            } else if c.enc == encodings::utf16be() {
                if s.starts_with(b"\xFE\xFF") {
                    c.input = &s[2..];
                }
            } else if c.enc == encodings::utf16le() && s.starts_with(b"\xFF\xFE") {
                c.input = &s[2..];
            }
        }
        count_demerits(&mut cands, strict);
        if cands.is_empty() {
            return None;
        }
    }
    let mut best = 0;
    for i in 1..cands.len() {
        if cands[i].demerits < cands[best].demerits {
            best = i;
        }
    }
    Some(cands[best].enc)
}

/// `count_demerits`.
fn count_demerits(cands: &mut Vec<Candidate>, strict: bool) {
    let mut buf = [0u32; 128];
    let mut finished = cands.iter().filter(|c| c.input.is_empty()).count();
    while (strict || cands.len() > 1) && finished < cands.len() {
        // In reverse, so that removing a candidate moves none still to come.
        let mut i = cands.len();
        while i > 0 {
            i -= 1;
            let c = &mut cands[i];
            if c.input.is_empty() {
                continue;
            }
            let n = c.enc.to_wchar(&mut c.input, &mut buf, &mut c.state);
            let mut eliminated = false;
            for &w in buf[..n].iter().rev() {
                if w == mbfl::BAD_INPUT {
                    if strict {
                        eliminated = true;
                        break;
                    }
                    c.demerits += 1000;
                } else {
                    c.demerits += estimate_demerits(w);
                }
            }
            if eliminated {
                cands.remove(i);
                continue;
            }
            if c.input.is_empty() {
                finished += 1;
            }
        }
    }
    for c in cands.iter_mut() {
        let d = c.demerits as f64 * c.multiplier as f64;
        c.demerits = if d < u64::MAX as f64 {
            d as u64
        } else {
            u64::MAX
        };
    }
}

/// `php_mb_convert_encoding`: from the one encoding given, or the one
/// detected among several (None, after php's warning, when none fits).
fn convert_detect(
    host: &mut dyn Host,
    fname: &str,
    input: &[u8],
    to: Enc,
    from: &[Enc],
) -> Result<Option<Vec<u8>>, Thrown> {
    let from = if from.len() == 1 {
        from[0]
    } else {
        match guess_encoding(&[input], from, strict_detection(), true) {
            Some(e) => e,
            None => {
                warn(host, fname, "Unable to detect character encoding")?;
                return Ok(None);
            }
        }
    };
    Ok(Some(convert_ex(input, to, from)))
}

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

/// `mb_internal_encoding(?string $encoding = null)`.
fn fn_mb_internal_encoding(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_internal_encoding";
    match opt_str_arg(host, args, 0, f)? {
        None => Ok(Value::str(internal().name())),
        Some(name) => match Encoding::by_name(&name) {
            Some(enc) => {
                st(|s| s.internal = enc);
                Ok(Value::Bool(true))
            }
            None => Err(value_error(
                f,
                0,
                &format!(
                    "must be a valid encoding, \"{}\" given",
                    String::from_utf8_lossy(c_str(&name))
                ),
            )),
        },
    }
}

/// `php_mb_check_code_point`.
fn valid_code_point(cp: i64) -> bool {
    (0..0x110000).contains(&cp) && !(0xD800..=0xDFFF).contains(&cp)
}

/// `mb_substitute_character(string|int|null $substitute_character = null)`.
fn fn_mb_substitute_character(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_substitute_character";
    let arg = match args.first().map(deref) {
        None | Some(Value::Null) => {
            let (mode, subst) = illegal_settings();
            return Ok(match mode {
                illegal_mode::NONE => Value::str("none"),
                illegal_mode::LONG => Value::str("long"),
                illegal_mode::ENTITY => Value::str("entity"),
                _ => Value::Int(subst as i64),
            });
        }
        Some(v) => v,
    };
    let mode_error = || {
        value_error(
            f,
            0,
            "must be \"none\", \"long\", \"entity\" or a valid codepoint",
        )
    };
    if let Value::Str(s) = &arg {
        let s = s.as_bytes();
        let mode = if s.eq_ignore_ascii_case(b"none") {
            illegal_mode::NONE
        } else if s.eq_ignore_ascii_case(b"long") {
            illegal_mode::LONG
        } else if s.eq_ignore_ascii_case(b"entity") {
            illegal_mode::ENTITY
        } else {
            return Err(mode_error());
        };
        st(|s| s.illegal_mode = mode);
        return Ok(Value::Bool(true));
    }
    // Z_PARAM_STR_OR_LONG for other scalars: in coercive mode int first
    // (zend_parse_arg_long_weak), then string, so a float that isn't an int
    // is taken as a string, and its string form ("INF", "1.0E+30", …) is
    // never a mode name.
    const TWO_63: f64 = 9_223_372_036_854_775_808.0;
    let cp = match &arg {
        Value::Int(n) => *n,
        _ if host.caller_strict_types() => {
            return Err(Thrown::type_error(format!(
                "{f}(): Argument #1 (${}) must be of type string|int|null, {} given",
                pname(f, 0),
                crate::argparse::value_name(host, &arg)
            )));
        }
        Value::Float(x) if x.is_nan() || !(-TWO_63..TWO_63).contains(x) => {
            if x.is_nan() {
                host.emit_diagnostic(
                    crate::E_WARNING,
                    "unexpected NAN value was coerced to string",
                )?;
            }
            return Err(mode_error());
        }
        _ => long_arg(host, args, 0, f)?,
    };
    if !valid_code_point(cp) {
        return Err(value_error(f, 0, "is not a valid codepoint"));
    }
    st(|s| {
        s.illegal_mode = illegal_mode::CHAR;
        s.substchar = cp as u32;
    });
    Ok(Value::Bool(true))
}

/// `mb_list_encodings()`: one shared array per request.
fn fn_mb_list_encodings(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let list = st(|s| {
        s.all_encodings
            .get_or_insert_with(|| {
                let mut a = PhpArray::new();
                for e in Encoding::supported() {
                    a.push(Value::str(e.name()));
                }
                a
            })
            .clone()
    });
    Ok(Value::Array(list))
}

/// `mb_encoding_aliases(string $encoding)`.
fn fn_mb_encoding_aliases(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_encoding_aliases";
    let name = str_arg(host, args, 0, f)?;
    let enc = get_encoding(host, Some(&name), 0, f)?;
    let mut a = PhpArray::new();
    for alias in enc.aliases() {
        a.push(Value::str(alias));
    }
    Ok(Value::Array(a))
}

// ---------------------------------------------------------------------------
// length, substrings, positions
// ---------------------------------------------------------------------------

/// `mb_str_split(string $string, int $length = 1, ?string $encoding = null)`.
fn fn_mb_str_split(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_str_split";
    let s = str_arg(host, args, 0, f)?;
    let mut split_len = if args.len() > 1 {
        long_arg(host, args, 1, f)?
    } else {
        1
    };
    let enc_name = opt_str_arg(host, args, 2, f)?;
    if split_len <= 0 {
        return Err(value_error(f, 1, "must be greater than 0"));
    } else if split_len > (u32::MAX / 4) as i64 {
        return Err(value_error(f, 1, "is too large"));
    }
    if split_len as usize > s.len() {
        split_len = s.len() as i64;
    }
    let enc = get_encoding(host, enc_name.as_deref(), 2, f)?;
    if s.is_empty() {
        return Ok(Value::Array(PhpArray::new()));
    }
    let split_len = split_len as usize;
    let mut out = PhpArray::new();
    let char_len = (enc.flag() & FIXED_WIDTH) as usize;
    if char_len != 0 {
        let chunk_len = char_len * split_len;
        let chunks = (s.len() / chunk_len).div_ceil(split_len);
        if !rhp_values::mem::reserve_packed_array(chunks) {
            return Ok(Value::Null);
        }
        for chunk in s.chunks(chunk_len) {
            out.push(Value::str(chunk));
            if has_oom() {
                return Ok(Value::Null);
            }
        }
    } else if let Some(tbl) = enc.mblen_table() {
        if !rhp_values::mem::reserve_packed_array(s.len().div_ceil(split_len)) {
            return Ok(Value::Null);
        }
        let mut p = 0;
        while p < s.len() {
            let start = p;
            for _ in 0..split_len {
                if p >= s.len() {
                    break;
                }
                p += tbl[s[p] as usize] as usize;
            }
            p = p.min(s.len());
            out.push(Value::str(&s[start..p]));
            if has_oom() {
                return Ok(Value::Null);
            }
        }
    } else {
        if !rhp_values::mem::reserve_packed_array(s.len().div_ceil(split_len)) {
            return Ok(Value::Null);
        }
        let (mode, subst) = illegal_settings();
        let mut wchar = [0u32; 128];
        let mut rest = &s[..];
        let mut state = 0;
        let mut char_count = 0usize;
        let mut buf: Option<ConvertBuf> = None;
        while !rest.is_empty() {
            if has_oom() {
                return Ok(Value::Null);
            }
            let out_len = enc.to_wchar(&mut rest, &mut wchar, &mut state);
            let mut i = 0;
            if char_count != 0 {
                let b = buf.as_mut().expect("a chunk is in progress");
                if out_len >= split_len - char_count {
                    enc.from_wchar(&wchar[..split_len - char_count], b, true);
                    i += split_len - char_count;
                    char_count = 0;
                    out.push(Value::str(buf.take().unwrap().result(enc)));
                } else {
                    enc.from_wchar(&wchar[..out_len], b, rest.is_empty());
                    char_count += out_len;
                    continue;
                }
            }
            while i < out_len {
                if has_oom() {
                    return Ok(Value::Null);
                }
                let mut b = ConvertBuf::new(split_len, subst, mode);
                if out_len - i >= split_len {
                    enc.from_wchar(&wchar[i..i + split_len], &mut b, true);
                    i += split_len;
                    out.push(Value::str(b.result(enc)));
                } else {
                    enc.from_wchar(&wchar[i..out_len], &mut b, rest.is_empty());
                    char_count = out_len - i;
                    buf = Some(b);
                    break;
                }
            }
        }
        if char_count != 0 {
            if let Some(b) = buf.take() {
                out.push(Value::str(b.result(enc)));
            }
        }
    }
    Ok(Value::Array(out))
}

/// `mb_strlen(string $string, ?string $encoding = null)`.
fn fn_mb_strlen(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_strlen";
    let s = str_arg(host, args, 0, f)?;
    let enc = enc_arg(host, args, 1, f)?;
    Ok(Value::Int(get_strlen(&s, enc) as i64))
}

/// The shared argument handling of mb_strpos, mb_strrpos, mb_stripos and
/// mb_strripos.
fn strpos_common(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    fold: bool,
    reverse: bool,
) -> Result<Value, Thrown> {
    let haystack = str_arg(host, args, 0, f)?;
    let needle = str_arg(host, args, 1, f)?;
    let offset = if args.len() > 2 {
        long_arg(host, args, 2, f)?
    } else {
        0
    };
    let enc = enc_arg(host, args, 3, f)?;
    let offset = offset as isize;
    let n = if fold {
        stripos(reverse, &haystack, &needle, offset, enc)
    } else {
        find_strpos(&haystack, &needle, enc, offset, reverse)
    };
    if !is_error(n) {
        Ok(Value::Int(n as i64))
    } else {
        strpos_error(f, n)
    }
}

fn fn_mb_strpos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strpos_common(host, args, "mb_strpos", false, false)
}

fn fn_mb_strrpos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strpos_common(host, args, "mb_strrpos", false, true)
}

fn fn_mb_stripos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strpos_common(host, args, "mb_stripos", true, false)
}

fn fn_mb_strripos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strpos_common(host, args, "mb_strripos", true, true)
}

/// `php_mb_strstr_variants`.
fn strstr_variant(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    fold: bool,
    reverse: bool,
) -> Result<Value, Thrown> {
    let haystack = str_arg(host, args, 0, f)?;
    let needle = str_arg(host, args, 1, f)?;
    let part = bool_arg(host, args, 2, f, false)?;
    let enc = enc_arg(host, args, 3, f)?;
    let n = if fold {
        stripos(reverse, &haystack, &needle, 0, enc)
    } else {
        find_strpos(&haystack, &needle, enc, 0, reverse)
    };
    if is_error(n) {
        return Ok(Value::Bool(false));
    }
    Ok(Value::str(if part {
        get_substr(&haystack, 0, n, enc)
    } else {
        get_substr(&haystack, n, UNTIL_END, enc)
    }))
}

fn fn_mb_strstr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strstr_variant(host, args, "mb_strstr", false, false)
}

fn fn_mb_strrchr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strstr_variant(host, args, "mb_strrchr", false, true)
}

fn fn_mb_stristr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strstr_variant(host, args, "mb_stristr", true, false)
}

fn fn_mb_strrichr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    strstr_variant(host, args, "mb_strrichr", true, true)
}

/// `mb_substr_count(string $haystack, string $needle, ?string $encoding = null)`.
fn fn_mb_substr_count(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_substr_count";
    let haystack = str_arg(host, args, 0, f)?;
    let needle = str_arg(host, args, 1, f)?;
    let enc_name = opt_str_arg(host, args, 2, f)?;
    if needle.is_empty() {
        return Err(empty_error(f, 1));
    }
    let enc = get_encoding(host, enc_name.as_deref(), 2, f)?;
    let (h, n): (Cow<[u8]>, Cow<[u8]>) = if is_utf8(enc) {
        // php skips the conversion for strings it knows are valid UTF-8,
        // which converting leaves unchanged.
        fn utf8(s: &[u8], enc: Enc) -> Cow<'_, [u8]> {
            if enc == encodings::utf8() && std::str::from_utf8(s).is_ok() {
                Cow::Borrowed(s)
            } else {
                Cow::Owned(to_utf8_marked(s, enc))
            }
        }
        (utf8(&haystack, enc), utf8(&needle, enc))
    } else {
        let n = to_utf8_marked(&needle, enc);
        // Bytes that decode to nothing (escape sequences only).
        if n.is_empty() {
            return Err(empty_error(f, 1));
        }
        (Cow::Owned(to_utf8_marked(&haystack, enc)), Cow::Owned(n))
    };
    let (h, n) = (&h[..], &n[..]);
    let mut count = 0i64;
    if h.len() >= n.len() {
        let mut p = 0;
        while let Some(at) = memnstr(h, p, n, h.len()) {
            p = at + n.len();
            count += 1;
        }
    }
    Ok(Value::Int(count))
}

/// `mb_substr(string $string, int $start, ?int $length = null, ?string $encoding = null)`.
fn fn_mb_substr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_substr";
    let s = str_arg(host, args, 0, f)?;
    let from = long_arg(host, args, 1, f)?;
    let len = opt_long_arg(host, args, 2, f)?;
    let enc_name = opt_str_arg(host, args, 3, f)?;
    let range = format!("must be between {} and {}", i64::MIN + 1, i64::MAX);
    if from == i64::MIN {
        return Err(value_error(f, 1, &range));
    }
    if len == Some(i64::MIN) {
        return Err(value_error(f, 2, &range));
    }
    let enc = get_encoding(host, enc_name.as_deref(), 3, f)?;
    let mblen = if from < 0 || len.is_some_and(|l| l < 0) {
        get_strlen(&s, enc)
    } else {
        0
    };
    let real_from = if from >= 0 {
        from as usize
    } else if ((-from) as usize) < mblen {
        (mblen as i64 + from) as usize
    } else {
        0
    };
    let real_len = match len {
        None => UNTIL_END,
        Some(l) if l >= 0 => l as usize,
        Some(l) if real_from < mblen && ((-l) as usize) < mblen - real_from => {
            ((mblen - real_from) as i64 + l) as usize
        }
        Some(_) => 0,
    };
    Ok(Value::str(get_substr(&s, real_from, real_len, enc)))
}

/// `mb_strcut(string $string, int $start, ?int $length = null, ?string $encoding = null)`.
fn fn_mb_strcut(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_strcut";
    let s = str_arg(host, args, 0, f)?;
    let mut from = long_arg(host, args, 1, f)?;
    let len = opt_long_arg(host, args, 2, f)?;
    let enc = enc_arg(host, args, 3, f)?;
    let n = s.len() as i64;
    let mut len = len.unwrap_or(n);
    // php's size_t/zend_long arithmetic, which wraps.
    if from < 0 {
        from = n.wrapping_add(from).max(0);
    }
    if len < 0 {
        len = n.wrapping_sub(from).wrapping_add(len).max(0);
    }
    if from > n || len == 0 {
        return Ok(Value::str(""));
    }
    let (from, mut len) = (from as usize, len as usize);
    if enc.has_cut() {
        return Ok(Value::str(enc.cut(&s, from, len).unwrap_or_default()));
    }
    let char_len = (enc.flag() & FIXED_WIDTH) as usize;
    if char_len != 0 {
        // Round `from` down to a multiple of the (power-of-two) width.
        let from = from & !(char_len - 1);
        if len > s.len() - from {
            len = s.len() - from;
        }
        return Ok(Value::str(&s[from..from + (len & !(char_len - 1))]));
    }
    if let Some(tbl) = enc.mblen_table() {
        let mut p = 0usize;
        let mut m = 0usize;
        while p < from {
            m = tbl[s[p] as usize] as usize;
            p += m;
        }
        if p > from {
            p -= m;
        }
        let start = p;
        let end = if len >= s.len() - start {
            s.len()
        } else {
            let q = p + len;
            while p < q {
                m = tbl[s[p] as usize] as usize;
                p += m;
            }
            if p > q {
                p -= m;
            }
            p
        };
        return Ok(Value::str(&s[start..end]));
    }
    match mbfl::strcut(&s, enc, from, len) {
        Some(r) => Ok(Value::str(r)),
        None => Ok(Value::Null),
    }
}

/// `mb_strwidth(string $string, ?string $encoding = null)`.
fn fn_mb_strwidth(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_strwidth";
    let s = str_arg(host, args, 0, f)?;
    let enc = enc_arg(host, args, 1, f)?;
    Ok(Value::Int(get_strwidth(&s, enc) as i64))
}

/// `mb_trim_string`: `input` from character `from`, cut to `width`
/// columns with `marker` appended when it doesn't fit.
fn trim_string(
    input_arg: &StrArg,
    marker_arg: &StrArg,
    enc: Enc,
    mut from: usize,
    mut width: usize,
) -> Value {
    let (input, marker): (&[u8], &[u8]) = (input_arg, marker_arg);
    let (mode, subst) = illegal_settings();
    let mut wchar = [0u32; 128];
    let mut rest = input;
    let mut state = 0;
    let mut remaining_width = width;
    let mut to_skip = from;
    let mut first_call = true;
    let mut input_err = false;
    let mut out_len = 0;
    // Where the cut starts: Some(true) to decode again from the start.
    let mut cut: Option<(bool, ConvertBuf)> = None;
    'scan: while !rest.is_empty() {
        out_len = enc.to_wchar(&mut rest, &mut wchar, &mut state);
        if out_len <= to_skip {
            to_skip -= out_len;
        } else {
            for &w in &wchar[to_skip..out_len] {
                let cw = character_width(w);
                input_err |= w == mbfl::BAD_INPUT;
                if remaining_width < cw {
                    let marker_width = get_strwidth(marker, enc);
                    if width <= marker_width {
                        return Value::Str(marker_arg.0.clone());
                    }
                    width -= marker_width;
                    cut = Some((!first_call, ConvertBuf::new(width, subst, mode)));
                    break 'scan;
                }
                remaining_width -= cw;
            }
            to_skip = 0;
        }
        first_call = false;
    }
    let Some((restart, mut buf)) = cut else {
        // It fits; bad byte sequences still become error markers.
        if !input_err {
            return if from == 0 {
                Value::Str(input_arg.0.clone())
            } else {
                Value::str(get_substr(input, from, UNTIL_END, enc))
            };
        }
        return Value::str(get_substr_slow(input, from, UNTIL_END, enc));
    };
    if restart {
        rest = input;
        state = 0;
        out_len = enc.to_wchar(&mut rest, &mut wchar, &mut state);
    }
    'emit: loop {
        if out_len <= from {
            from -= out_len;
        } else {
            for i in from..out_len {
                let cw = character_width(wchar[i]);
                if width < cw {
                    enc.from_wchar(&wchar[from..i], &mut buf, true);
                    break 'emit;
                }
                width -= cw;
            }
            enc.from_wchar(&wchar[from..out_len], &mut buf, false);
            from = 0;
        }
        if rest.is_empty() {
            // Not reached: the scan found the string too wide.
            break;
        }
        out_len = enc.to_wchar(&mut rest, &mut wchar, &mut state);
    }
    if !marker.is_empty() && !buf.append(marker) {
        return Value::str(buf.result(enc));
    }
    // Not marked as valid UTF-8: the marker may not be.
    Value::str(buf.result_raw())
}

/// `mb_strimwidth(string $string, int $start, int $width, string $trim_marker = "", ?string $encoding = null)`.
fn fn_mb_strimwidth(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_strimwidth";
    let s = str_arg(host, args, 0, f)?;
    let mut from = long_arg(host, args, 1, f)?;
    let mut width = long_arg(host, args, 2, f)?;
    let marker = if args.len() > 3 {
        str_arg(host, args, 3, f)?
    } else {
        StrArg::default()
    };
    let enc = enc_arg(host, args, 4, f)?;
    if from != 0 {
        let str_len = get_strlen(&s, enc) as i64;
        if from < 0 {
            from += str_len;
        }
        if from < 0 || from > str_len {
            return Err(value_error(f, 1, "is out of range"));
        }
    }
    if width < 0 {
        deprecated(
            host,
            f,
            "passing a negative integer to argument #3 ($width) is deprecated",
        )?;
        width += get_strwidth(&s, enc) as i64;
        if from > 0 {
            let trimmed = get_substr(&s, 0, from as usize, enc);
            width -= get_strwidth(&trimmed, enc) as i64;
        }
        if width < 0 {
            return Err(value_error(f, 2, "is out of range"));
        }
    }
    Ok(trim_string(&s, &marker, enc, from as usize, width as usize))
}

// ---------------------------------------------------------------------------
// conversion
// ---------------------------------------------------------------------------

/// `mb_convert_encoding(array|string $string, string $to_encoding, array|string|null $from_encoding = null)`.
fn fn_mb_convert_encoding(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_convert_encoding";
    let input = deref(&args[0]);
    let input = match input {
        Value::Array(a) => Err(a),
        _ => Ok(str_arg(host, args, 0, f)?),
    };
    let to_name = str_arg(host, args, 1, f)?;
    let to = get_encoding(host, Some(&to_name), 1, f)?;
    let mut from = match enc_list_arg(host, args, 2, f)? {
        Some((list, _)) => list,
        None => vec![internal()],
    };
    if from.len() > 1 {
        remove_non_encodings(&mut from);
    }
    if from.is_empty() {
        return Err(value_error(f, 2, "must specify at least one encoding"));
    }
    match input {
        Ok(s) => Ok(match convert_detect(host, f, &s, to, &from)? {
            Some(r) => Value::str(r),
            None => Value::Bool(false),
        }),
        Err(a) => Ok(convert_recursive(host, &a, to, &from, &mut Vec::new())?
            .map_or(Value::Array(PhpArray::new()), Value::Array)),
    }
}

/// `php_mb_convert_encoding_recursive`: keys and string values converted,
/// other scalars copied, arrays recursed, objects dropped with a warning.
fn convert_recursive(
    host: &mut dyn Host,
    a: &PhpArray,
    to: Enc,
    from: &[Enc],
    path: &mut Vec<usize>,
) -> Result<Option<PhpArray>, Thrown> {
    let f = "mb_convert_encoding";
    let id = a.data_ptr();
    if path.contains(&id) {
        warn(host, f, "Cannot convert recursively referenced values")?;
        return Ok(None);
    }
    path.push(id);
    let mut out = PhpArray::new();
    for (k, v) in a.iter() {
        let key = match k {
            ArrayKey::Str(s) => match convert_detect(host, f, s.as_bytes(), to, from)? {
                Some(c) => ArrayKey::Str(PhpStr::from_bytes(c)),
                None => continue,
            },
            k => k.clone(),
        };
        let nv = match deref(v) {
            Value::Str(s) => match convert_detect(host, f, s.as_bytes(), to, from)? {
                Some(c) => Value::str(c),
                None => continue,
            },
            v @ (Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_)) => v,
            Value::Array(inner) => {
                Value::Array(convert_recursive(host, &inner, to, from, path)?.unwrap_or_default())
            }
            _ => {
                warn(host, f, "Object is not supported")?;
                continue;
            }
        };
        if out.get(&key).is_none() {
            out.insert(key, nv);
        }
    }
    path.pop();
    Ok(Some(out))
}

/// `mb_convert_case(string $string, int $mode, ?string $encoding = null)`.
fn fn_mb_convert_case(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_convert_case";
    let s = str_arg(host, args, 0, f)?;
    let mode = long_arg(host, args, 1, f)?;
    let enc = enc_arg(host, args, 2, f)?;
    if !(0..case_mode::MAX as i64).contains(&mode) {
        return Err(value_error(f, 1, "must be one of the MB_CASE_* constants"));
    }
    Ok(Value::str(convert_case(mode as i32, &s, enc)))
}

/// `mb_strtoupper(string $string, ?string $encoding = null)`.
fn fn_mb_strtoupper(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_strtoupper";
    let s = str_arg(host, args, 0, f)?;
    let enc = enc_arg(host, args, 1, f)?;
    Ok(Value::str(convert_case(case_mode::UPPER, &s, enc)))
}

/// `mb_strtolower(string $string, ?string $encoding = null)`.
fn fn_mb_strtolower(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_strtolower";
    let s = str_arg(host, args, 0, f)?;
    let enc = enc_arg(host, args, 1, f)?;
    Ok(Value::str(convert_case(case_mode::LOWER, &s, enc)))
}

/// `mb_detect_encoding(string $string, array|string|null $encodings = null, bool $strict = false)`.
fn fn_mb_detect_encoding(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_detect_encoding";
    let s = str_arg(host, args, 0, f)?;
    let (mut list, all) = match enc_list_arg(host, args, 1, f)? {
        Some(l) => l,
        None => (st(|s| s.detect_order.clone()), false),
    };
    let strict = if args.len() >= 3 {
        bool_arg(host, args, 2, f, false)?
    } else {
        strict_detection()
    };
    if list.is_empty() {
        return Err(value_error(f, 1, "must specify at least one encoding"));
    }
    remove_non_encodings(&mut list);
    if list.is_empty() {
        return Ok(Value::Bool(false));
    }
    Ok(match guess_encoding(&[&s], &list, strict, !all) {
        Some(e) => Value::str(e.name()),
        None => Value::Bool(false),
    })
}

/// `mb_check_encoding(array|string|null $value = null, ?string $encoding = null)`.
fn fn_mb_check_encoding(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_check_encoding";
    let value = match args.first().map(deref) {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => Some(Err(a)),
        Some(_) => Some(Ok(str_arg(host, args, 0, f)?)),
    };
    let enc = enc_arg(host, args, 1, f)?;
    match value {
        Some(Err(a)) => Ok(Value::Bool(check_encoding_recursive(
            host,
            &a,
            enc,
            &mut Vec::new(),
        )?)),
        Some(Ok(s)) => Ok(Value::Bool(check_str_encoding(&s, enc))),
        None => {
            deprecated(
                host,
                f,
                "Calling mb_check_encoding() without argument is deprecated",
            )?;
            Ok(Value::Bool(st(|s| s.illegalchars) == 0))
        }
    }
}

/// `php_mb_check_encoding_recursive`: string keys and values must be
/// valid, arrays recurse, other scalars pass, anything else fails. An
/// invalid key ends the walk; an invalid value doesn't (php's `break`
/// leaves only the switch).
fn check_encoding_recursive(
    host: &mut dyn Host,
    a: &PhpArray,
    enc: Enc,
    path: &mut Vec<usize>,
) -> Result<bool, Thrown> {
    let id = a.data_ptr();
    if path.contains(&id) {
        warn(
            host,
            "mb_check_encoding",
            "Cannot not handle circular references",
        )?;
        return Ok(false);
    }
    path.push(id);
    let mut valid = true;
    for (k, v) in a.iter() {
        if let ArrayKey::Str(s) = k {
            if !check_str_encoding(s.as_bytes(), enc) {
                valid = false;
                break;
            }
        }
        match deref(v) {
            Value::Str(s) => {
                if !check_str_encoding(s.as_bytes(), enc) {
                    valid = false;
                }
            }
            Value::Array(inner) => {
                if !check_encoding_recursive(host, &inner, enc, path)? {
                    valid = false;
                }
            }
            Value::Int(_) | Value::Float(_) | Value::Null | Value::Bool(_) => {}
            _ => valid = false,
        }
    }
    path.pop();
    Ok(valid)
}

/// `mb_ord(string $string, ?string $encoding = null)`.
fn fn_mb_ord(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_ord";
    let s = str_arg(host, args, 0, f)?;
    let enc_name = opt_str_arg(host, args, 1, f)?;
    if s.is_empty() {
        return Err(empty_error(f, 0));
    }
    let enc = get_encoding(host, enc_name.as_deref(), 1, f)?;
    if is_unsupported(enc) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "mb_ord() does not support the \"{}\" encoding",
                String::from_utf8_lossy(enc.name())
            ),
        ));
    }
    // SJIS-Mac, UTF-7 and UTF7-IMAP need this much room.
    let mut wchar = [0u32; mbfl::MIN_WCHAR_BUFSIZE];
    let mut rest = &s[..];
    let mut state = 0;
    let n = enc.to_wchar(&mut rest, &mut wchar, &mut state);
    if n == 0 || wchar[0] == mbfl::BAD_INPUT {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Int(wchar[0] as i64))
}

/// `mb_chr(int $codepoint, ?string $encoding = null)`.
fn fn_mb_chr(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_chr";
    let cp = long_arg(host, args, 0, f)?;
    let enc = enc_arg(host, args, 1, f)?;
    if is_unsupported(enc) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "mb_chr() does not support the \"{}\" encoding",
                String::from_utf8_lossy(enc.name())
            ),
        ));
    }
    if !(0..=0x10FFFF).contains(&cp) {
        return Ok(Value::Bool(false));
    }
    if is_utf8(enc) {
        if (0xD800..0xE000).contains(&cp) {
            return Ok(Value::Bool(false));
        }
        let c = char::from_u32(cp as u32).expect("a scalar value");
        return Ok(Value::str(c.to_string()));
    }
    let buf = (cp as u32).to_be_bytes();
    let orig = st(|s| std::mem::take(&mut s.illegalchars));
    let r = convert_ex(&buf, enc, encodings::ucs4be());
    let failed = st(|s| std::mem::replace(&mut s.illegalchars, orig)) != 0;
    Ok(if failed {
        Value::Bool(false)
    } else {
        Value::str(r)
    })
}

/// `mb_str_pad(string $string, int $length, string $pad_string = " ", int $pad_type = STR_PAD_RIGHT, ?string $encoding = null)`.
fn fn_mb_str_pad(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_str_pad";
    let input = str_arg(host, args, 0, f)?;
    let pad_to = long_arg(host, args, 1, f)?;
    let pad = if args.len() > 2 {
        str_arg(host, args, 2, f)?
    } else {
        StrArg(PhpStr::from(" "))
    };
    let pad_type = if args.len() > 3 {
        long_arg(host, args, 3, f)?
    } else {
        1
    };
    let enc = enc_arg(host, args, 4, f)?;
    let input_len = get_strlen(&input, enc);
    if pad_to < 0 || pad_to as usize <= input_len {
        return Ok(Value::Str(input.0));
    }
    if pad.is_empty() {
        return Err(empty_error(f, 2));
    }
    if !(0..=2).contains(&pad_type) {
        return Err(value_error(
            f,
            3,
            "must be STR_PAD_LEFT, STR_PAD_RIGHT, or STR_PAD_BOTH",
        ));
    }
    let pad_len = get_strlen(&pad, enc);
    if pad_len == 0 {
        // Possible with an invalidly encoded pad string.
        return Err(empty_error(f, 2));
    }
    let num = pad_to as usize - input_len;
    let (left, right) = match pad_type {
        0 => (num, 0),
        1 => (0, num),
        _ => (num / 2, num - num / 2),
    };
    let overflow = || Thrown::error("Error", "String size overflow");
    let (full_left, full_right) = (left / pad_len, right / pad_len);
    let (left_bytes, right_bytes) = match (
        full_left.checked_mul(pad.len()),
        full_right.checked_mul(pad.len()),
    ) {
        (Some(l), Some(r)) => (l, r),
        _ => return Err(overflow()),
    };
    let rem_left = get_substr(&pad, 0, left % pad_len, enc);
    let rem_right = get_substr(&pad, 0, right % pad_len, enc);
    // ZSTR_MAX_LEN
    const MAX_LEN: usize = usize::MAX - 32;
    if left_bytes > MAX_LEN - rem_left.len() || right_bytes > MAX_LEN - rem_right.len() {
        return Err(overflow());
    }
    let left_bytes = left_bytes + rem_left.len();
    let right_bytes = right_bytes + rem_right.len();
    if left_bytes > MAX_LEN - right_bytes || input.len() > MAX_LEN - left_bytes - right_bytes {
        return Err(overflow());
    }
    // zend_string_safe_falloc(1, len, pads)
    let pads = (24 + left_bytes + right_bytes + 1 + 7) & !7;
    if !rhp_values::mem::reserve_safe(1, input.len(), pads) {
        return Ok(Value::Null);
    }
    let mut out = Vec::with_capacity(input.len() + left_bytes + right_bytes);
    for _ in 0..full_left {
        out.extend_from_slice(&pad);
    }
    out.extend_from_slice(&rem_left);
    out.extend_from_slice(&input);
    for _ in 0..full_right {
        out.extend_from_slice(&pad);
    }
    out.extend_from_slice(&rem_right);
    Ok(Value::str(out))
}

/// `mb_scrub(string $string, ?string $encoding = null)`.
fn fn_mb_scrub(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_scrub";
    let s = str_arg(host, args, 0, f)?;
    let enc = enc_arg(host, args, 1, f)?;
    if enc == encodings::utf8() && std::str::from_utf8(&s).is_ok() {
        return Ok(Value::Str(s.0));
    }
    Ok(Value::str(convert_ex(&s, enc, enc)))
}

// ---------------------------------------------------------------------------
// mb_convert_variables
// ---------------------------------------------------------------------------

/// A walk over the variables stopped: at a cycle (php's `GC_IS_RECURSIVE` /
/// `Z_IS_RECURSIVE` guards) or past memory_limit. php also stops at its
/// stack limit; these walks keep their stack on the heap and need none.
struct Stopped;

/// The identity of a reference cell or object, which the walks track
/// along the current path. Like php's protect/unprotect flags this only
/// rejects true cycles: a value reached twice along different paths is
/// walked (and converted) twice.
fn walk_id(v: &Value) -> Option<*const ()> {
    match v {
        Value::Ref(r) => Some(Rc::as_ptr(r) as *const ()),
        Value::Object(h) => Some(Rc::as_ptr(h) as *const ()),
        _ => None,
    }
}

/// `mb_recursive_find_strings`: every string inside `v`, in php's order.
fn find_strings(v: &Value, out: &mut Vec<PhpStr>) -> Result<(), Stopped> {
    // The containers on the current path, each with its next child.
    let mut stack: Vec<(Value, usize)> = Vec::new();
    let mut path: HashSet<*const ()> = HashSet::new();
    let mut next = Some(v.clone());
    loop {
        if let Some(v) = next.take() {
            match v {
                Value::Str(s) => out.push(s),
                Value::Array(_) => stack.push((v, 0)),
                Value::Ref(_) | Value::Object(_) => {
                    if !path.insert(walk_id(&v).expect("a reference or object")) {
                        return Err(Stopped);
                    }
                    stack.push((v, 0));
                }
                _ => {}
            }
        }
        let Some((top, pos)) = stack.last_mut() else {
            return Ok(());
        };
        let i = *pos;
        *pos += 1;
        next = match top {
            Value::Array(a) => a.get_index(i).map(|(_, c)| c.clone()),
            Value::Object(h) => h.borrow().properties.get_index(i).map(|(_, c)| c.clone()),
            Value::Ref(r) => (i == 0).then(|| r.borrow().clone()),
            _ => None,
        };
        if next.is_none() {
            if let Some((done, _)) = stack.pop() {
                if let Some(id) = walk_id(&done) {
                    path.remove(&id);
                }
            }
        }
    }
}

/// A container `convert_variable` is inside of. Its children are taken
/// out one at a time and put back converted: array elements, object
/// properties, or the value of a reference cell (taken out of the cell).
enum Frame {
    Array(PhpArray, usize),
    Object(rhp_values::ObjectHandle, usize),
    Cell(rhp_values::PhpRef, Value, bool),
}

impl Frame {
    /// The next child, taken out of its slot.
    fn next_child(&mut self) -> Option<Value> {
        let take = |v: &mut Value| std::mem::replace(v, Value::Null);
        match self {
            Frame::Array(a, pos) => {
                if *pos >= a.len() {
                    return None;
                }
                let i = *pos;
                *pos += 1;
                a.make_mut().entries.get_index_mut(i).map(|(_, v)| take(v))
            }
            Frame::Object(h, pos) => {
                let i = *pos;
                *pos += 1;
                h.borrow_mut()
                    .properties
                    .get_index_mut(i)
                    .map(|(_, v)| take(v))
            }
            Frame::Cell(_, inner, visited) => {
                (!std::mem::replace(visited, true)).then(|| take(inner))
            }
        }
    }

    /// Puts the child last taken out back, converted.
    fn put_back(&mut self, v: Value) {
        match self {
            Frame::Array(a, pos) => {
                if let Some((_, slot)) = a.make_mut().entries.get_index_mut(*pos - 1) {
                    *slot = v;
                }
            }
            Frame::Object(h, pos) => {
                if let Some((_, slot)) = h.borrow_mut().properties.get_index_mut(*pos - 1) {
                    *slot = v;
                }
            }
            Frame::Cell(_, inner, _) => *inner = v,
        }
    }

    /// The container once its children are done: a cell gets its value
    /// back, and the value for the parent's slot is returned.
    fn finish(self, path: &mut HashSet<*const ()>) -> Value {
        let v = match self {
            Frame::Array(a, _) => return Value::Array(a),
            Frame::Object(h, _) => Value::Object(h),
            Frame::Cell(r, inner, _) => {
                *r.borrow_mut() = inner;
                Value::Ref(r)
            }
        };
        if let Some(id) = walk_id(&v) {
            path.remove(&id);
        }
        v
    }
}

/// `mb_recursive_convert_variable` on one argument. A string reached
/// through a reference *inside* an array or object is replaced by a plain
/// string (php overwrites the slot's zval, dropping the reference); arrays
/// and objects behind references are converted in place. A partial
/// conversion stays when the walk stops, as in php.
fn convert_variable(var: &Value, from: Enc, to: Enc) -> Result<(), Stopped> {
    let convert = |s: &PhpStr| Value::str(convert_ex(s.as_bytes(), to, from));
    // php ZVAL_DEREFs each argument: a by-reference variable is converted
    // in place, a string included.
    if let Value::Ref(r) = var {
        let s = match &*r.borrow() {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        };
        if let Some(s) = s {
            *r.borrow_mut() = convert(&s);
            return if has_oom() { Err(Stopped) } else { Ok(()) };
        }
    }
    let mut stack: Vec<Frame> = Vec::new();
    let mut path: HashSet<*const ()> = HashSet::new();
    let mut next = Some(var.clone());
    loop {
        // Convert the child, or enter it.
        if let Some(v) = next.take() {
            let (back, stop) = match v {
                Value::Str(s) => (convert(&s), has_oom()),
                Value::Ref(r) => {
                    let s = match &*r.borrow() {
                        Value::Str(s) => Some(s.clone()),
                        _ => None,
                    };
                    if let Some(s) = s {
                        (convert(&s), has_oom())
                    } else if !path.insert(Rc::as_ptr(&r) as *const ()) {
                        (Value::Ref(r), true)
                    } else {
                        let inner = std::mem::replace(&mut *r.borrow_mut(), Value::Null);
                        stack.push(Frame::Cell(r, inner, false));
                        continue;
                    }
                }
                Value::Array(a) => {
                    stack.push(Frame::Array(a, 0));
                    continue;
                }
                Value::Object(h) => {
                    if !path.insert(Rc::as_ptr(&h) as *const ()) {
                        (Value::Object(h), true)
                    } else {
                        stack.push(Frame::Object(h, 0));
                        continue;
                    }
                }
                v => (v, false),
            };
            if let Some(parent) = stack.last_mut() {
                parent.put_back(back);
            }
            if stop {
                while let Some(frame) = stack.pop() {
                    let v = frame.finish(&mut path);
                    if let Some(parent) = stack.last_mut() {
                        parent.put_back(v);
                    }
                }
                return Err(Stopped);
            }
        }
        // The next child of the innermost container, finishing the
        // containers that have none left.
        loop {
            let Some(top) = stack.last_mut() else {
                return Ok(());
            };
            if let Some(c) = top.next_child() {
                next = Some(c);
                break;
            }
            let v = stack.pop().expect("a frame").finish(&mut path);
            if let Some(parent) = stack.last_mut() {
                parent.put_back(v);
            }
        }
    }
}

/// `mb_convert_variables(string $to_encoding, array|string $from_encoding, mixed &$var, mixed &...$vars)`.
fn fn_mb_convert_variables(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_convert_variables";
    let to_name = str_arg(host, args, 0, f)?;
    let to = get_encoding(host, Some(&to_name), 0, f)?;
    let (list, all) = match deref(&args[1]) {
        Value::Array(a) => {
            let all = st(|s| {
                s.all_encodings
                    .as_ref()
                    .is_some_and(|l| l.data_ptr() == a.data_ptr())
            });
            (parse_encoding_array(host, &a, f, 1)?, all)
        }
        _ => {
            let s = str_arg(host, args, 1, f)?;
            (parse_encoding_list(&s, f, 1)?, false)
        }
    };
    if list.is_empty() {
        return Err(value_error(f, 1, "must specify at least one encoding"));
    }
    let stopped = |host: &mut dyn Host| -> Result<Value, Thrown> {
        warn(host, f, "Cannot handle recursive references")?;
        Ok(Value::Bool(false))
    };
    let from = if list.len() == 1 {
        list[0]
    } else {
        let mut strs = Vec::new();
        for v in &args[2..] {
            if find_strings(v, &mut strs).is_err() {
                return stopped(host);
            }
        }
        let strs: Vec<&[u8]> = strs.iter().map(PhpStr::as_bytes).collect();
        match guess_encoding(&strs, &list, strict_detection(), !all) {
            Some(e) => e,
            None => {
                warn(host, f, "Unable to detect encoding")?;
                return Ok(Value::Bool(false));
            }
        }
    };
    for v in &args[2..] {
        if convert_variable(v, from, to).is_err() {
            return stopped(host);
        }
    }
    Ok(Value::str(from.name()))
}

// ---------------------------------------------------------------------------
// HTML numeric entities
// ---------------------------------------------------------------------------

/// `make_conversion_map`: the map as php's uint32 quadruples (low, high,
/// offset, mask).
fn conversion_map(
    host: &mut dyn Host,
    map: &PhpArray,
    fname: &str,
) -> Result<Vec<[u32; 4]>, Thrown> {
    if !map.len().is_multiple_of(4) {
        return Err(value_error(fname, 1, "must have a multiple of 4 elements"));
    }
    let mut out = Vec::with_capacity(map.len() / 4);
    let mut quad = [0u32; 4];
    for (k, (_, v)) in map.iter().enumerate() {
        match zpp::try_get_long(host, &deref(v))? {
            Some(n) => {
                quad[k % 4] = n as u32;
                if k % 4 == 3 {
                    out.push(quad);
                }
            }
            None => {
                return Err(value_error(
                    fname,
                    1,
                    "must only be composed of values of type int",
                ))
            }
        }
    }
    Ok(out)
}

/// The array argument at `i` (the VM has checked its type).
fn array_arg(args: &[Value], i: usize) -> PhpArray {
    match args.get(i).map(deref) {
        Some(Value::Array(a)) => a,
        _ => PhpArray::new(),
    }
}

/// `html_numeric_entity_convert`.
fn entity_convert(w: u32, map: &[[u32; 4]]) -> Option<u32> {
    map.iter()
        .find(|m| w >= m[0] && w <= m[1])
        .map(|m| w.wrapping_add(m[2]) & m[3])
}

/// `html_numeric_entity_encode`.
fn entity_encode(input: &[u8], enc: Enc, map: &[[u32; 4]], hex: bool) -> Vec<u8> {
    let (mode, subst) = illegal_settings();
    let mut buf = ConvertBuf::new(input.len(), subst, mode);
    let mut wchar = [0u32; 32];
    let mut converted: Vec<u32> = Vec::with_capacity(32 * 13);
    let mut rest = input;
    let mut state = 0;
    while !rest.is_empty() {
        let n = enc.to_wchar(&mut rest, &mut wchar, &mut state);
        converted.clear();
        for &w in &wchar[..n] {
            match entity_convert(w, map) {
                Some(mut w) => {
                    converted.extend([b'&' as u32, b'#' as u32]);
                    if hex {
                        converted.push(b'x' as u32);
                    }
                    if w == 0 {
                        converted.push(b'0' as u32);
                    } else {
                        let mut digits = Vec::new();
                        if hex {
                            while w > 0 {
                                digits.push(b"0123456789ABCDEF"[(w & 0xF) as usize] as u32);
                                w >>= 4;
                            }
                        } else {
                            while w > 0 {
                                digits.push(b'0' as u32 + w % 10);
                                w /= 10;
                            }
                        }
                        converted.extend(digits.iter().rev());
                    }
                    converted.push(b';' as u32);
                }
                None => converted.push(w),
            }
        }
        enc.from_wchar(&converted, &mut buf, rest.is_empty());
    }
    buf.result(enc)
}

/// `mb_encode_numericentity(string $string, array $map, ?string $encoding = null, bool $hex = false)`.
fn fn_mb_encode_numericentity(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_encode_numericentity";
    let s = str_arg(host, args, 0, f)?;
    let map = array_arg(args, 1);
    let enc_name = opt_str_arg(host, args, 2, f)?;
    let hex = bool_arg(host, args, 3, f, false)?;
    let enc = get_encoding(host, enc_name.as_deref(), 2, f)?;
    let map = conversion_map(host, &map, f)?;
    Ok(Value::str(entity_encode(&s, enc, &map, hex)))
}

/// `html_numeric_entity_deconvert`.
fn entity_deconvert(number: u32, map: &[[u32; 4]]) -> Option<u32> {
    map.iter().find_map(|m| {
        let cp = number.wrapping_sub(m[2]);
        (cp >= m[0] && cp <= m[1]).then_some(cp)
    })
}

/// `html_numeric_entity_decode`.
fn entity_decode(input: &[u8], enc: Enc, map: &[[u32; 4]]) -> Vec<u8> {
    const DEC_MINLEN: usize = 3; // "&#" and a digit
    const HEX_MINLEN: usize = 4; // "&#x" and a digit
    const DEC_MAXLEN: usize = 12; // "&#" and 10 digits
    const HEX_MAXLEN: usize = 11; // "&#x" and 8 digits
    let amp = b'&' as u32;
    let (mode, subst) = illegal_settings();
    let mut buf = ConvertBuf::new(input.len(), subst, mode);
    // As in php: up to 127 codepoints and the '&' sentinel after them.
    let mut wchar = [0u32; 128];
    let mut converted: Vec<u32> = Vec::with_capacity(128);
    let mut offset = 0usize;
    let mut rest = input;
    let mut state = 0;
    let is_hex = |w: u32| matches!(w, 0x30..=0x39 | 0x41..=0x46 | 0x61..=0x66);
    let is_dec = |w: u32| (0x30..=0x39).contains(&w);
    while !rest.is_empty() {
        let n = enc.to_wchar(&mut rest, &mut wchar[offset..127], &mut state);
        let out_len = n + offset;
        wchar[out_len] = amp;
        let more = !rest.is_empty();
        converted.clear();
        let mut p = 0usize;
        if offset == 0 {
            while wchar[p] != amp {
                p += 1;
            }
            if p == out_len {
                enc.from_wchar(&wchar[..out_len], &mut buf, !more);
                continue;
            }
            converted.extend_from_slice(&wchar[..p]);
        }
        offset = 0;
        // At an '&' (wchar[p]).
        'amp: loop {
            let mut p2 = p + 1;
            if wchar[p2] == b'#' as u32 {
                p2 += 1;
                if wchar[p2] == b'x' as u32 {
                    p2 += 1;
                    while is_hex(wchar[p2]) {
                        p2 += 1;
                    }
                    let len = p2 - p;
                    if p2 == out_len && more && len <= HEX_MAXLEN {
                        // Truncated by the buffer: again with the next one.
                        wchar.copy_within(p..p2, 0);
                        offset = len;
                        break 'amp;
                    } else if !(HEX_MINLEN..=HEX_MAXLEN).contains(&len) {
                        converted.extend_from_slice(&wchar[p..p2]);
                    } else {
                        let mut value = 0u32;
                        for &w in &wchar[p + 3..p2] {
                            let d = if w <= b'9' as u32 {
                                w - b'0' as u32
                            } else if w >= b'a' as u32 {
                                10 + w - b'a' as u32
                            } else {
                                10 + w - b'A' as u32
                            };
                            value = value.wrapping_mul(16).wrapping_add(d);
                        }
                        match entity_deconvert(value, map) {
                            Some(cp) => {
                                converted.push(cp);
                                if wchar[p2] == b';' as u32 {
                                    p2 += 1;
                                }
                            }
                            None => converted.extend_from_slice(&wchar[p..p2]),
                        }
                    }
                } else {
                    while is_dec(wchar[p2]) {
                        p2 += 1;
                    }
                    let len = p2 - p;
                    if p2 == out_len && more && len <= DEC_MAXLEN {
                        wchar.copy_within(p..p2, 0);
                        offset = len;
                        break 'amp;
                    } else if !(DEC_MINLEN..=DEC_MAXLEN).contains(&len) {
                        converted.extend_from_slice(&wchar[p..p2]);
                    } else {
                        let mut value = 0u32;
                        let mut too_big = false;
                        for &w in &wchar[p + 2..p2] {
                            // Past 0x19999999, times 10 overflows.
                            if value > 0x1999_9999 {
                                too_big = true;
                                break;
                            }
                            value = value.wrapping_mul(10).wrapping_add(w - b'0' as u32);
                        }
                        if too_big {
                            converted.extend_from_slice(&wchar[p..p2]);
                        } else {
                            match entity_deconvert(value, map) {
                                Some(cp) => {
                                    converted.push(cp);
                                    if wchar[p2] == b';' as u32 {
                                        p2 += 1;
                                    }
                                }
                                None => converted.extend_from_slice(&wchar[p..p2]),
                            }
                        }
                    }
                }
            } else if p2 == out_len && more {
                // An '&' at the end of the buffer.
                wchar[0] = amp;
                offset = 1;
                break 'amp;
            } else {
                converted.push(amp);
            }
            // The next stretch without an '&'.
            p = p2;
            while wchar[p2] != amp {
                p2 += 1;
            }
            if p2 > p {
                converted.extend_from_slice(&wchar[p..p2]);
                p = p2;
            }
            if p >= out_len {
                break 'amp;
            }
        }
        enc.from_wchar(&converted, &mut buf, !more);
    }
    buf.result(enc)
}

/// `mb_decode_numericentity(string $string, array $map, ?string $encoding = null)`.
fn fn_mb_decode_numericentity(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_decode_numericentity";
    let s = str_arg(host, args, 0, f)?;
    let map = array_arg(args, 1);
    let enc = enc_arg(host, args, 2, f)?;
    let map = conversion_map(host, &map, f)?;
    Ok(Value::str(entity_decode(&s, enc, &map)))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register the mbstring builtins.
pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    let fns: &[(&'static str, crate::NativeFn, usize, Option<usize>)] = &[
        ("mb_internal_encoding", fn_mb_internal_encoding, 0, Some(1)),
        (
            "mb_substitute_character",
            fn_mb_substitute_character,
            0,
            Some(1),
        ),
        ("mb_str_split", fn_mb_str_split, 1, Some(3)),
        ("mb_strlen", fn_mb_strlen, 1, Some(2)),
        ("mb_strpos", fn_mb_strpos, 2, Some(4)),
        ("mb_strrpos", fn_mb_strrpos, 2, Some(4)),
        ("mb_stripos", fn_mb_stripos, 2, Some(4)),
        ("mb_strripos", fn_mb_strripos, 2, Some(4)),
        ("mb_strstr", fn_mb_strstr, 2, Some(4)),
        ("mb_strrchr", fn_mb_strrchr, 2, Some(4)),
        ("mb_stristr", fn_mb_stristr, 2, Some(4)),
        ("mb_strrichr", fn_mb_strrichr, 2, Some(4)),
        ("mb_substr_count", fn_mb_substr_count, 2, Some(3)),
        ("mb_substr", fn_mb_substr, 2, Some(4)),
        ("mb_strcut", fn_mb_strcut, 2, Some(4)),
        ("mb_strwidth", fn_mb_strwidth, 1, Some(2)),
        ("mb_strimwidth", fn_mb_strimwidth, 3, Some(5)),
        ("mb_convert_encoding", fn_mb_convert_encoding, 2, Some(3)),
        ("mb_convert_case", fn_mb_convert_case, 2, Some(3)),
        ("mb_strtoupper", fn_mb_strtoupper, 1, Some(2)),
        ("mb_strtolower", fn_mb_strtolower, 1, Some(2)),
        ("mb_detect_encoding", fn_mb_detect_encoding, 1, Some(3)),
        ("mb_list_encodings", fn_mb_list_encodings, 0, Some(0)),
        ("mb_encoding_aliases", fn_mb_encoding_aliases, 1, Some(1)),
        ("mb_convert_variables", fn_mb_convert_variables, 3, None),
        (
            "mb_encode_numericentity",
            fn_mb_encode_numericentity,
            2,
            Some(4),
        ),
        (
            "mb_decode_numericentity",
            fn_mb_decode_numericentity,
            2,
            Some(3),
        ),
        ("mb_check_encoding", fn_mb_check_encoding, 0, Some(2)),
        ("mb_scrub", fn_mb_scrub, 1, Some(2)),
        ("mb_ord", fn_mb_ord, 1, Some(2)),
        ("mb_chr", fn_mb_chr, 1, Some(2)),
        ("mb_str_pad", fn_mb_str_pad, 2, Some(5)),
    ];
    for &(name, func, min_args, max_args) in fns {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args,
                max_args,
            },
        );
    }
}

#[cfg(test)]
mod tests;
