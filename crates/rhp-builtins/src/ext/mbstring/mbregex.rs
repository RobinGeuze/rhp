//! mbstring's multibyte regex functions (mb_ereg*, mb_split,
//! mb_regex_encoding, mb_regex_set_options): a port of php-src's
//! ext/mbstring/php_mbregex.c (PHP-3.01) over the oniguruma php links
//! (libonig, BSD-2-Clause; docs/design/libraries.md). onig_shim.c reaches
//! what Rust cannot spell from oniguruma.h.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_int, c_uchar, c_uint, c_void};
use std::rc::Rc;

use rhp_values::{ArrayKey, PhpArray, PhpStr, Value};

use super::{c_str, check_encoding, empty_error, str_arg, value_error, Enc};
use crate::zend::zpp;
use crate::zend::zpp::deref;
use crate::{Host, Thrown};

// ---------------------------------------------------------------------------
// oniguruma
// ---------------------------------------------------------------------------

mod sys {
    use std::ffi::{c_int, c_uchar, c_uint, c_void};

    pub type ForeachName = unsafe extern "C" fn(
        name: *const c_uchar,
        name_end: *const c_uchar,
        ngroups: c_int,
        groups: *mut c_int,
        reg: *mut c_void,
        arg: *mut c_void,
    ) -> c_int;

    extern "C" {
        pub fn rhp_onig_init() -> c_int;
        pub fn rhp_onig_new(
            reg: *mut *mut c_void,
            pat: *const c_uchar,
            len: usize,
            options: c_uint,
            enc: c_int,
            syntax: c_int,
            err: *mut c_uchar,
        ) -> c_int;
        pub fn rhp_onig_error_str(code: c_int, err: *mut c_uchar);
        pub fn rhp_onig_search(
            reg: *mut c_void,
            s: *const c_uchar,
            len: usize,
            start: usize,
            region: *mut c_void,
            set_stack: c_int,
            stack: c_uint,
            set_retry: c_int,
            retry: c_uint,
        ) -> c_int;
        pub fn rhp_onig_match(
            reg: *mut c_void,
            s: *const c_uchar,
            len: usize,
            set_stack: c_int,
            stack: c_uint,
            set_retry: c_int,
            retry: c_uint,
        ) -> c_int;
        pub fn rhp_onig_region_regs(
            region: *const c_void,
            beg: *mut *const c_int,
            end: *mut *const c_int,
        ) -> c_int;
        pub fn onig_free(reg: *mut c_void);
        pub fn onig_region_new() -> *mut c_void;
        pub fn onig_region_free(region: *mut c_void, free_self: c_int);
        pub fn onig_number_of_names(reg: *mut c_void) -> c_int;
        pub fn onig_noname_group_capture_is_active(reg: *mut c_void) -> c_int;
        pub fn onig_name_to_backref_number(
            reg: *mut c_void,
            name: *const c_uchar,
            name_end: *const c_uchar,
            region: *const c_void,
        ) -> c_int;
        pub fn onig_foreach_name(reg: *mut c_void, f: ForeachName, arg: *mut c_void) -> c_int;
    }
}

/// `ONIG_MAX_ERROR_MESSAGE_LEN`.
const ERR_LEN: usize = 90;
const ONIG_NORMAL: c_int = 0;
const ONIG_MISMATCH: c_int = -1;

const OPTION_IGNORECASE: c_uint = 1;
const OPTION_EXTEND: c_uint = 2;
const OPTION_MULTILINE: c_uint = 4;
const OPTION_SINGLELINE: c_uint = 8;
const OPTION_FIND_LONGEST: c_uint = 16;
const OPTION_FIND_NOT_EMPTY: c_uint = 32;

/// The syntaxes php's option letters pick (onig_shim.c's `syntax()`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Syntax {
    Java = 0,
    GnuRegex,
    Grep,
    Emacs,
    Ruby,
    Perl,
    PosixBasic,
    PosixExtended,
}

/// php_mbregex.c's `enc_name_map`, in onig_shim.c's `encoding()` order.
const ENC_NAME_MAP: &[&[&[u8]]] = &[
    &[
        b"EUC-JP",
        b"EUCJP",
        b"X-EUC-JP",
        b"UJIS",
        b"EUCJP",
        b"EUCJP-WIN",
    ],
    &[b"UTF-8", b"UTF8"],
    &[b"UTF-16", b"UTF-16BE"],
    &[b"UTF-16LE"],
    &[b"UCS-4", b"UTF-32", b"UTF-32BE"],
    &[b"UCS-4LE", b"UTF-32LE"],
    &[
        b"SJIS",
        b"CP932",
        b"MS932",
        b"SHIFT_JIS",
        b"SJIS-WIN",
        b"WINDOWS-31J",
    ],
    &[b"BIG5", b"BIG-5", b"BIGFIVE", b"CN-BIG5", b"BIG-FIVE"],
    &[b"EUC-CN", b"EUCCN", b"EUC_CN", b"GB-2312", b"GB2312"],
    &[b"EUC-TW", b"EUCTW", b"EUC_TW"],
    &[b"EUC-KR", b"EUCKR", b"EUC_KR"],
    &[b"KOI8R", b"KOI8-R", b"KOI-8R"],
    &[b"ISO-8859-1", b"ISO8859-1"],
    &[b"ISO-8859-2", b"ISO8859-2"],
    &[b"ISO-8859-3", b"ISO8859-3"],
    &[b"ISO-8859-4", b"ISO8859-4"],
    &[b"ISO-8859-5", b"ISO8859-5"],
    &[b"ISO-8859-6", b"ISO8859-6"],
    &[b"ISO-8859-7", b"ISO8859-7"],
    &[b"ISO-8859-8", b"ISO8859-8"],
    &[b"ISO-8859-9", b"ISO8859-9"],
    &[b"ISO-8859-10", b"ISO8859-10"],
    &[b"ISO-8859-11", b"ISO8859-11"],
    &[b"ISO-8859-13", b"ISO8859-13"],
    &[b"ISO-8859-14", b"ISO8859-14"],
    &[b"ISO-8859-15", b"ISO8859-15"],
    &[b"ISO-8859-16", b"ISO8859-16"],
    &[b"ASCII", b"US-ASCII", b"US_ASCII", b"ISO646"],
];

const UTF8_ID: usize = 1;

/// `_php_mb_regex_name2mbctype`.
fn name2mbctype(name: &[u8]) -> Option<usize> {
    if name.is_empty() {
        return None;
    }
    ENC_NAME_MAP
        .iter()
        .position(|names| names.iter().any(|n| n.eq_ignore_ascii_case(name)))
}

/// `_php_mb_regex_mbctype2name`.
fn mbctype2name(id: usize) -> &'static [u8] {
    ENC_NAME_MAP[id][0]
}

/// A compiled pattern (`php_mb_regex_t`).
struct Regex {
    ptr: *mut c_void,
    options: c_uint,
    enc: usize,
    syntax: Syntax,
}

impl Drop for Regex {
    fn drop(&mut self) {
        unsafe { sys::onig_free(self.ptr) }
    }
}

impl Regex {
    fn number_of_names(&self) -> c_int {
        unsafe { sys::onig_number_of_names(self.ptr) }
    }
    fn noname_group_capture_is_active(&self) -> bool {
        unsafe { sys::onig_noname_group_capture_is_active(self.ptr) != 0 }
    }
    fn name_to_backref_number(&self, name: &[u8], region: &Region) -> c_int {
        let r = name.as_ptr_range();
        unsafe { sys::onig_name_to_backref_number(self.ptr, r.start, r.end, region.0) }
    }

    /// `onig_foreach_name`: each group name, in oniguruma's order.
    fn names(&self) -> Vec<Vec<u8>> {
        unsafe extern "C" fn each(
            name: *const c_uchar,
            name_end: *const c_uchar,
            _n: c_int,
            _groups: *mut c_int,
            _reg: *mut c_void,
            arg: *mut c_void,
        ) -> c_int {
            let out = &mut *(arg as *mut Vec<Vec<u8>>);
            let len = name_end.offset_from(name) as usize;
            out.push(std::slice::from_raw_parts(name, len).to_vec());
            0
        }
        let mut out: Vec<Vec<u8>> = Vec::new();
        unsafe {
            sys::onig_foreach_name(self.ptr, each, &mut out as *mut _ as *mut c_void);
        }
        out
    }
}

/// An `OnigRegion`.
struct Region(*mut c_void);

impl Region {
    fn new() -> Region {
        Region(unsafe { sys::onig_region_new() })
    }

    /// The registers' (beg, end) pairs.
    fn regs(&self) -> Vec<(c_int, c_int)> {
        let mut beg = std::ptr::null();
        let mut end = std::ptr::null();
        let n = unsafe { sys::rhp_onig_region_regs(self.0, &mut beg, &mut end) };
        (0..n.max(0) as usize)
            .map(|i| unsafe { (*beg.add(i), *end.add(i)) })
            .collect()
    }

    /// `onig_region_free(region, 0)`: empties it for reuse.
    fn clear(&mut self) {
        unsafe { sys::onig_region_free(self.0, 0) }
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        unsafe { sys::onig_region_free(self.0, 1) }
    }
}

fn error_str(code: c_int) -> Vec<u8> {
    let mut buf = [0u8; ERR_LEN + 1];
    unsafe { sys::rhp_onig_error_str(code, buf.as_mut_ptr()) };
    c_str(&buf).to_vec()
}

/// php's MINIT: `onig_init()`, once per process.
fn init() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        sys::rhp_onig_init();
    });
}

// ---------------------------------------------------------------------------
// state (MBREX)
// ---------------------------------------------------------------------------

struct State {
    /// `default_mbctype`: the ini's (mbstring.internal_encoding); kept
    /// across requests.
    default_mbctype: usize,
    current_mbctype: usize,
    current_mbfl: Enc,
    /// `ht_rc`: compiled patterns by pattern.
    cache: HashMap<Vec<u8>, Rc<Regex>>,
    search_str: Option<PhpStr>,
    search_pos: usize,
    search_re: Option<Rc<Regex>>,
    search_regs: Option<Region>,
    default_options: c_uint,
    default_syntax: Syntax,
}

impl State {
    fn new() -> State {
        State {
            default_mbctype: UTF8_ID,
            current_mbctype: UTF8_ID,
            current_mbfl: rhp_mbfl_sys::encodings::utf8(),
            cache: HashMap::new(),
            search_str: None,
            search_pos: 0,
            search_re: None,
            search_regs: None,
            default_options: OPTION_MULTILINE | OPTION_SINGLELINE,
            default_syntax: Syntax::Ruby,
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::new());
}

fn st<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

/// RINIT/RSHUTDOWN(mb_regex): the current encoding goes back to the
/// default; the search state and pattern cache go.
pub(super) fn reset() {
    let (search_regs, cache, search_re) = st(|s| {
        s.current_mbctype = s.default_mbctype;
        s.current_mbfl = rhp_mbfl_sys::Encoding::by_name(mbctype2name(s.default_mbctype))
            .unwrap_or(rhp_mbfl_sys::encodings::utf8());
        s.search_str = None;
        s.search_pos = 0;
        s.default_options = OPTION_MULTILINE | OPTION_SINGLELINE;
        s.default_syntax = Syntax::Ruby;
        (
            s.search_regs.take(),
            std::mem::take(&mut s.cache),
            s.search_re.take(),
        )
    });
    drop((search_regs, cache, search_re));
}

/// `php_mb_regex_set_mbctype`.
fn set_mbctype(name: &[u8]) -> bool {
    let Some(id) = name2mbctype(name) else {
        return false;
    };
    // An encoding oniguruma has but libmbfl lacks.
    let Some(enc) = rhp_mbfl_sys::Encoding::by_name(name) else {
        return false;
    };
    st(|s| {
        s.current_mbctype = id;
        s.current_mbfl = enc;
    });
    true
}

/// `php_mb_regex_set_default_mbctype`.
fn set_default_mbctype(name: &[u8]) -> bool {
    match name2mbctype(name) {
        Some(id) => {
            st(|s| s.default_mbctype = id);
            true
        }
        None => false,
    }
}

/// The mbregex part of `_php_mb_ini_mbstring_internal_encoding_set`.
pub(super) fn internal_encoding_set(name: &[u8]) {
    let name = c_str(name);
    if !set_default_mbctype(name) {
        set_default_mbctype(b"UTF-8");
    }
    set_mbctype(name);
}

/// `php_mb_regex_get_mbctype_encoding`.
fn mbctype_encoding() -> Enc {
    st(|s| s.current_mbfl)
}

/// `php_mb_mbchar_bytes` at `s[i]`, reading past the end as the NUL php's
/// strings end with.
fn mbchar_bytes(s: &[u8], i: usize, enc: Enc) -> usize {
    if let Some(t) = enc.mblen_table() {
        return t[s.get(i).copied().unwrap_or(0) as usize] as usize;
    }
    let flag = enc.flag();
    if flag & rhp_mbfl_sys::ENCTYPE_WCS2 != 0 {
        2
    } else if flag & rhp_mbfl_sys::ENCTYPE_WCS4 != 0 {
        4
    } else {
        1
    }
}

/// mbstring.regex_stack_limit / regex_retry_limit (OnUpdateLong).
fn limit(name: &[u8]) -> i64 {
    crate::main::ini::ini_value(name)
        .flatten()
        .map_or(0, |v| crate::main::ini::ini_parse_quantity(&v))
}

/// `!ZEND_LONG_UINT_OVFL(n)`.
fn fits_uint(n: i64) -> (c_int, c_uint) {
    if (0..=u32::MAX as i64).contains(&n) {
        (1, n as c_uint)
    } else {
        (0, 0)
    }
}

/// `_php_mb_onig_search`: the match's index (>= 0), ONIG_MISMATCH or an
/// error code.
fn search(re: &Regex, s: &[u8], start: usize, region: &mut Region) -> c_int {
    let (ss, stack) = fits_uint(limit(b"mbstring.regex_stack_limit"));
    let (sr, retry) = fits_uint(limit(b"mbstring.regex_retry_limit"));
    unsafe {
        sys::rhp_onig_search(
            re.ptr,
            s.as_ptr(),
            s.len(),
            start,
            region.0,
            ss,
            stack,
            sr,
            retry,
        )
    }
}

// ---------------------------------------------------------------------------
// patterns and options
// ---------------------------------------------------------------------------

/// `php_mbregex_compile_pattern`: the cached pattern, compiled again when
/// its options, encoding or syntax differ.
fn compile_pattern(
    host: &mut dyn Host,
    fname: &str,
    pattern: &[u8],
    options: c_uint,
    syntax: Syntax,
) -> Result<Option<Rc<Regex>>, Thrown> {
    let enc = st(|s| s.current_mbctype);
    if !check_encoding(pattern, mbctype_encoding()) {
        let msg = [
            b"Pattern is not valid under ".as_slice(),
            mbctype2name(enc),
            b" encoding",
        ]
        .concat();
        crate::warn(host, fname, &msg)?;
        return Ok(None);
    }
    let rc = st(|s| s.cache.get(pattern).cloned());
    if let Some(rc) = &rc {
        if rc.options == options && rc.enc == enc && rc.syntax == syntax {
            return Ok(Some(rc.clone()));
        }
    }
    init();
    let mut ptr = std::ptr::null_mut();
    let mut err = [0u8; ERR_LEN + 1];
    let code = unsafe {
        sys::rhp_onig_new(
            &mut ptr,
            pattern.as_ptr(),
            pattern.len(),
            options,
            enc as c_int,
            syntax as c_int,
            err.as_mut_ptr(),
        )
    };
    if code != ONIG_NORMAL {
        let msg = [b"mbregex compile err: ".as_slice(), c_str(&err)].concat();
        crate::warn(host, fname, &msg)?;
        return Ok(None);
    }
    let re = Rc::new(Regex {
        ptr,
        options,
        enc,
        syntax,
    });
    let old = st(|s| {
        let mut regs = None;
        let mut search_re = None;
        // The search pattern being replaced goes too (bug #72399).
        if let (Some(old), Some(cur)) = (&rc, &s.search_re) {
            if Rc::ptr_eq(old, cur) {
                search_re = s.search_re.take();
                regs = s.search_regs.take();
            }
        }
        (
            s.cache.insert(pattern.to_vec(), re.clone()),
            regs,
            search_re,
        )
    });
    drop(old);
    Ok(Some(re))
}

/// `_php_mb_regex_get_option_string`.
fn option_string(option: c_uint, syntax: Syntax) -> Vec<u8> {
    let mut out = Vec::new();
    if option & OPTION_IGNORECASE != 0 {
        out.push(b'i');
    }
    if option & OPTION_EXTEND != 0 {
        out.push(b'x');
    }
    if option & (OPTION_MULTILINE | OPTION_SINGLELINE) == OPTION_MULTILINE | OPTION_SINGLELINE {
        out.push(b'p');
    } else {
        if option & OPTION_MULTILINE != 0 {
            out.push(b'm');
        }
        if option & OPTION_SINGLELINE != 0 {
            out.push(b's');
        }
    }
    if option & OPTION_FIND_LONGEST != 0 {
        out.push(b'l');
    }
    if option & OPTION_FIND_NOT_EMPTY != 0 {
        out.push(b'n');
    }
    out.push(match syntax {
        Syntax::Java => b'j',
        Syntax::GnuRegex => b'u',
        Syntax::Grep => b'g',
        Syntax::Emacs => b'c',
        Syntax::Ruby => b'r',
        Syntax::Perl => b'z',
        Syntax::PosixBasic => b'b',
        Syntax::PosixExtended => b'd',
    });
    out
}

/// `_php_mb_regex_init_options`: the options the letters add, and the
/// syntax they pick (Ruby's by default).
fn init_options(arg: &[u8]) -> Result<(c_uint, Syntax), Thrown> {
    let mut opt = 0;
    let mut syntax = Syntax::Ruby;
    for &c in arg {
        match c {
            b'i' => opt |= OPTION_IGNORECASE,
            b'x' => opt |= OPTION_EXTEND,
            b'm' => opt |= OPTION_MULTILINE,
            b's' => opt |= OPTION_SINGLELINE,
            b'p' => opt |= OPTION_MULTILINE | OPTION_SINGLELINE,
            b'l' => opt |= OPTION_FIND_LONGEST,
            b'n' => opt |= OPTION_FIND_NOT_EMPTY,
            b'j' => syntax = Syntax::Java,
            b'u' => syntax = Syntax::GnuRegex,
            b'g' => syntax = Syntax::Grep,
            b'c' => syntax = Syntax::Emacs,
            b'r' => syntax = Syntax::Ruby,
            b'z' => syntax = Syntax::Perl,
            b'b' => syntax = Syntax::PosixBasic,
            b'd' => syntax = Syntax::PosixExtended,
            _ => {
                return Err(Thrown::error(
                    "ValueError",
                    format!(
                        "Option \"{}\" is not supported",
                        String::from_utf8_lossy(&[c])
                    ),
                ))
            }
        }
    }
    Ok((opt, syntax))
}

fn defaults() -> (c_uint, Syntax) {
    st(|s| (s.default_options, s.default_syntax))
}

/// The options argument (`s!`), else the defaults.
fn options_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    fname: &str,
) -> Result<Option<Vec<u8>>, Thrown> {
    super::opt_str_arg(host, args, i, fname)
}

// ---------------------------------------------------------------------------
// results
// ---------------------------------------------------------------------------

/// The registers as an array: matched groups as strings, the others false
/// (`beg < end` for mb_ereg, `beg <= end` for the search functions), then
/// the named groups (`mb_regex_groups_iter`).
fn regs_array(re: &Regex, region: &Region, s: &[u8], allow_empty: bool, out: &mut PhpArray) {
    for (i, (beg, end)) in region.regs().into_iter().enumerate() {
        let ok = beg >= 0
            && (if allow_empty { beg <= end } else { beg < end })
            && end as usize <= s.len();
        let v = if ok {
            Value::Str(PhpStr::from_slice(&s[beg as usize..end as usize]))
        } else {
            Value::Bool(false)
        };
        out.insert(ArrayKey::Int(i as i64), v);
    }
    add_named_groups(re, region, s, out);
}

/// `mb_regex_groups_iter` over every name: the last group of that name
/// that matched (as preg_match's PCRE2_DUPNAMES).
fn add_named_groups(re: &Regex, region: &Region, s: &[u8], out: &mut PhpArray) {
    if re.number_of_names() <= 0 {
        return;
    }
    let regs = region.regs();
    for name in re.names() {
        let gn = re.name_to_backref_number(&name, region);
        let (beg, end) = usize::try_from(gn)
            .ok()
            .and_then(|g| regs.get(g).copied())
            .unwrap_or((-1, -1));
        let v = if beg >= 0 && beg < end && end as usize <= s.len() {
            Value::Str(PhpStr::from_slice(&s[beg as usize..end as usize]))
        } else {
            Value::Bool(false)
        };
        out.insert(ArrayKey::from_bytes(&name), v);
    }
}

/// `mb_regex_substitute`: the replacement with its `\0`-`\9` and `\k<name>`
/// references filled in.
fn substitute(
    out: &mut Vec<u8>,
    subject: &[u8],
    replace: &[u8],
    re: &Regex,
    region: &Region,
    enc: Enc,
) {
    let eos = replace.len();
    let append = |out: &mut Vec<u8>, from: usize, to: usize| {
        out.extend_from_slice(&replace[from.min(eos)..to.min(eos)]);
    };
    let regs = region.regs();
    let mut p = 0;
    while p < eos {
        let clen = mbchar_bytes(replace, p, enc);
        if clen != 1 || replace[p] != b'\\' {
            append(out, p, p + clen);
            p += clen;
            continue;
        }
        let sp = p;
        p += 1;
        let clen = mbchar_bytes(replace, p, enc);
        if clen != 1 || p == eos {
            append(out, sp, p);
            continue;
        }
        let no: c_int;
        match replace[p] {
            b'0' => {
                no = 0;
                p += 1;
            }
            c @ b'1'..=b'9' => {
                if !re.noname_group_capture_is_active() {
                    // Oniguruma refuses numbered references in a pattern
                    // with named groups: php leaves them as they are.
                    p += 1;
                    append(out, sp, p);
                    continue;
                }
                no = (c - b'0') as c_int;
                p += 1;
            }
            b'k' => {
                p += 1;
                let clen = mbchar_bytes(replace, p, enc);
                if clen != 1 || p == eos || (replace[p] != b'<' && replace[p] != b'\'') {
                    p = (p + clen).min(eos);
                    append(out, sp, p);
                    continue;
                }
                let delim = if replace[p] == b'<' { b'>' } else { b'\'' };
                let name = p + 1;
                let mut name_end = name;
                let mut maybe_num = true;
                while name_end < eos {
                    let clen = mbchar_bytes(replace, name_end, enc);
                    if clen != 1 {
                        name_end += clen;
                        maybe_num = false;
                        continue;
                    }
                    if replace[name_end] == delim {
                        break;
                    }
                    if maybe_num && !replace[name_end].is_ascii_digit() {
                        maybe_num = false;
                    }
                    name_end += 1;
                }
                if name_end < name + 1 || name_end >= eos {
                    // Empty, or no closing delimiter.
                    p = (name_end + 1).min(eos);
                    append(out, sp, p);
                    continue;
                }
                p = name_end + 1;
                if maybe_num {
                    if !re.noname_group_capture_is_active() {
                        append(out, sp, p);
                        continue;
                    }
                    let digits = &replace[name..name_end];
                    if digits.len() == 1 {
                        no = (digits[0] - b'0') as c_int;
                    } else if digits[0] == b'0' {
                        // 01 is not a number.
                        no = -1;
                    } else {
                        // strtoul, cast to int.
                        let mut n: u64 = 0;
                        for &d in digits {
                            n = n
                                .checked_mul(10)
                                .and_then(|n| n.checked_add((d - b'0') as u64))
                                .unwrap_or(u64::MAX);
                            if n == u64::MAX {
                                break;
                            }
                        }
                        no = n as c_int;
                    }
                } else {
                    no = re.name_to_backref_number(&replace[name..name_end], region);
                }
            }
            _ => {
                // A backslash is no escape: `\\1` is `\` then `\1`.
                append(out, sp, p);
                continue;
            }
        }
        if no < 0 || no as usize >= regs.len() {
            append(out, sp, p);
            continue;
        }
        let (beg, end) = regs[no as usize];
        if beg >= 0 && beg < end && end as usize <= subject.len() {
            out.extend_from_slice(&subject[beg as usize..end as usize]);
        }
    }
}

// ---------------------------------------------------------------------------
// functions
// ---------------------------------------------------------------------------

/// `mb_regex_encoding(?string $encoding = null): string|bool`.
fn fn_mb_regex_encoding(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_regex_encoding";
    match super::opt_str_arg(host, args, 0, f)? {
        None => Ok(Value::str(mbctype2name(st(|s| s.current_mbctype)))),
        Some(name) => {
            if !set_mbctype(c_str(&name)) {
                return Err(value_error(
                    f,
                    0,
                    &format!(
                        "must be a valid encoding, \"{}\" given",
                        String::from_utf8_lossy(c_str(&name))
                    ),
                ));
            }
            Ok(Value::Bool(true))
        }
    }
}

/// `_php_mb_regex_ereg_exec`: mb_ereg and mb_eregi.
fn ereg_exec(host: &mut dyn Host, args: &[Value], f: &str, icase: bool) -> Result<Value, Thrown> {
    let pattern = str_arg(host, args, 0, f)?;
    let string = str_arg(host, args, 1, f)?;
    if pattern.is_empty() {
        return Err(empty_error(f, 0));
    }
    let has_array = args.len() > 2;
    if has_array {
        zpp::try_assign_ref(host, args, 2, Value::Array(PhpArray::new()))?;
    }
    if !check_encoding(&string, mbctype_encoding()) {
        return Ok(Value::Bool(false));
    }
    let mut options = st(|s| s.default_options);
    if icase {
        options |= OPTION_IGNORECASE;
    }
    let syntax = st(|s| s.default_syntax);
    let Some(re) = compile_pattern(host, f, &pattern, options, syntax)? else {
        return Ok(Value::Bool(false));
    };
    let mut region = Region::new();
    if search(&re, &string, 0, &mut region) < 0 {
        return Ok(Value::Bool(false));
    }
    if has_array {
        let mut arr = PhpArray::new();
        regs_array(&re, &region, &string, false, &mut arr);
        zpp::set_ref(args, 2, Value::Array(arr));
    }
    Ok(Value::Bool(true))
}

fn fn_mb_ereg(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ereg_exec(host, args, "mb_ereg", false)
}

fn fn_mb_eregi(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ereg_exec(host, args, "mb_eregi", true)
}

/// `_php_mb_regex_ereg_replace_exec`: mb_ereg_replace, mb_eregi_replace
/// and mb_ereg_replace_callback.
fn ereg_replace_exec(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    mut options: c_uint,
    callable: bool,
) -> Result<Value, Thrown> {
    let enc = mbctype_encoding();
    let pattern = str_arg(host, args, 0, f)?;
    let mut replace = None;
    let mut callback = Value::Null;
    if callable {
        callback = args.get(1).map(deref).unwrap_or(Value::Null);
        zpp::require_callback(host, f, 2, &callback)?;
    } else {
        replace = Some(str_arg(host, args, 1, f)?);
    }
    let string = str_arg(host, args, 2, f)?;
    let option_str = options_arg(host, args, 3, f)?;
    if !check_encoding(&string, enc) {
        return Ok(Value::Null);
    }
    let syntax = match option_str {
        Some(o) => {
            let (opt, syntax) = init_options(&o)?;
            options |= opt;
            syntax
        }
        None => {
            let (opt, syntax) = defaults();
            options |= opt;
            syntax
        }
    };
    let Some(re) = compile_pattern(host, f, &pattern, options, syntax)? else {
        return Ok(Value::Bool(false));
    };
    let s: &[u8] = &string;
    let lim = s.len();
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut region = Region::new();
    let mut err = 0;
    while err >= 0 {
        err = search(&re, s, pos, &mut region);
        if err <= -2 {
            let msg = [
                b"mbregex search failure in php_mbereg_replace_exec(): ".as_slice(),
                &error_str(err),
            ]
            .concat();
            crate::warn(host, f, &msg)?;
            break;
        }
        if err >= 0 {
            let regs = region.regs();
            let (beg0, end0) = regs[0];
            out.extend_from_slice(&s[pos.min(lim)..(beg0 as usize).max(pos.min(lim))]);
            if let Some(rep) = &replace {
                substitute(&mut out, s, rep, &re, &region, enc);
            } else {
                let mut subpats = PhpArray::new();
                for (i, &(beg, end)) in regs.iter().enumerate() {
                    let v = if beg >= 0 && end >= beg {
                        PhpStr::from_slice(&s[beg as usize..end as usize])
                    } else {
                        PhpStr::from_slice(b"")
                    };
                    subpats.insert(ArrayKey::Int(i as i64), Value::Str(v));
                }
                add_named_groups(&re, &region, s, &mut subpats);
                let ret = host.call_callable(&callback, &[Value::Array(subpats)])?;
                let r = zpp::zval_get_string(host, &ret)?;
                out.extend_from_slice(&r);
            }
            let n = end0 as usize;
            if pos < n {
                pos = n;
            } else {
                if pos < lim {
                    out.push(s[pos]);
                }
                pos += 1;
            }
        } else if pos < lim {
            // No match: the rest of the string.
            out.extend_from_slice(&s[pos..]);
        }
        region.clear();
    }
    if err <= -2 {
        return Ok(Value::Bool(false));
    }
    Ok(Value::str(out))
}

fn fn_mb_ereg_replace(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ereg_replace_exec(host, args, "mb_ereg_replace", 0, false)
}

fn fn_mb_eregi_replace(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ereg_replace_exec(host, args, "mb_eregi_replace", OPTION_IGNORECASE, false)
}

fn fn_mb_ereg_replace_callback(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ereg_replace_exec(host, args, "mb_ereg_replace_callback", 0, true)
}

/// `mb_split(string $pattern, string $string, int $limit = -1): array|false`.
fn fn_mb_split(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_split";
    let pattern = str_arg(host, args, 0, f)?;
    let string = str_arg(host, args, 1, f)?;
    // zend_ulong, so -1 is no limit.
    let mut count = if args.len() > 2 {
        super::long_arg(host, args, 2, f)? as u64
    } else {
        u64::MAX
    };
    count = count.saturating_sub(1);
    if !check_encoding(&string, mbctype_encoding()) {
        return Ok(Value::Bool(false));
    }
    let (options, syntax) = defaults();
    let Some(re) = compile_pattern(host, f, &pattern, options, syntax)? else {
        return Ok(Value::Bool(false));
    };
    let s: &[u8] = &string;
    let mut out = PhpArray::new();
    let mut pos = 0usize;
    let mut chunk_pos = 0usize;
    let mut err = 0;
    let mut region = Region::new();
    while count != 0 && pos < s.len() {
        err = search(&re, s, pos, &mut region);
        if err < 0 {
            break;
        }
        let (beg, end) = region.regs()[0];
        let (beg, end) = (beg as usize, end as usize);
        if pos < end {
            if beg < s.len() && beg >= chunk_pos {
                out.push(Value::Str(PhpStr::from_slice(&s[chunk_pos..beg])));
                count = count.wrapping_sub(1);
            } else {
                err = -2;
                break;
            }
            pos = end;
            chunk_pos = end;
        } else {
            pos += 1;
        }
        region.clear();
    }
    drop(region);
    if err <= -2 {
        let msg = [
            b"mbregex search failure in mbsplit(): ".as_slice(),
            &error_str(err),
        ]
        .concat();
        crate::warn(host, f, &msg)?;
        return Ok(Value::Bool(false));
    }
    if s.len() > chunk_pos {
        out.push(Value::Str(PhpStr::from_slice(&s[chunk_pos..])));
    } else {
        out.push(Value::str(""));
    }
    Ok(Value::Array(out))
}

/// `mb_ereg_match(string $pattern, string $string, ?string $options = null): bool`.
fn fn_mb_ereg_match(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_ereg_match";
    let pattern = str_arg(host, args, 0, f)?;
    let string = str_arg(host, args, 1, f)?;
    let (option, syntax) = match options_arg(host, args, 2, f)? {
        Some(o) => init_options(&o)?,
        None => defaults(),
    };
    if !check_encoding(&string, mbctype_encoding()) {
        return Ok(Value::Bool(false));
    }
    let Some(re) = compile_pattern(host, f, &pattern, option, syntax)? else {
        return Ok(Value::Bool(false));
    };
    let inside = |n: i64| -> (c_int, c_uint) {
        if n > 0 && n < u32::MAX as i64 {
            (1, n as c_uint)
        } else {
            (0, 0)
        }
    };
    let (ss, stack) = inside(limit(b"mbstring.regex_stack_limit"));
    let (sr, retry) = inside(limit(b"mbstring.regex_retry_limit"));
    let err =
        unsafe { sys::rhp_onig_match(re.ptr, string.as_ptr(), string.len(), ss, stack, sr, retry) };
    Ok(Value::Bool(err >= 0))
}

/// `_php_mb_regex_ereg_search_exec`: mode 0 mb_ereg_search, 1
/// mb_ereg_search_pos, 2 mb_ereg_search_regs.
fn ereg_search_exec(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    mode: u8,
) -> Result<Value, Thrown> {
    let pattern = super::opt_str_arg(host, args, 0, f)?;
    let (option, syntax) = match options_arg(host, args, 1, f)? {
        Some(o) => init_options(&o)?,
        None => defaults(),
    };
    let old = st(|s| s.search_regs.take());
    drop(old);
    if let Some(p) = pattern {
        match compile_pattern(host, f, &p, option, syntax)? {
            Some(re) => {
                let old = st(|s| s.search_re.replace(re));
                drop(old);
            }
            None => {
                let old = st(|s| s.search_re.take());
                drop(old);
                return Ok(Value::Bool(false));
            }
        }
    }
    let (pos, string, re) = st(|s| (s.search_pos, s.search_str.clone(), s.search_re.clone()));
    let Some(re) = re else {
        return Err(Thrown::error("Error", "No pattern was provided"));
    };
    let Some(string) = string else {
        return Err(Thrown::error("Error", "No string was provided"));
    };
    let s = string.as_bytes();
    let len = s.len();
    let mut region = Region::new();
    let err = search(&re, s, pos, &mut region);
    let mut ret = Value::Bool(false);
    if err == ONIG_MISMATCH {
        st(|st| st.search_pos = len);
    } else if err <= -2 {
        let msg = [
            b"mbregex search failure in mbregex_search(): ".as_slice(),
            &error_str(err),
        ]
        .concat();
        crate::warn(host, f, &msg)?;
    } else {
        let regs = region.regs();
        let (beg, end) = regs[0];
        ret = match mode {
            1 => {
                let mut a = PhpArray::new();
                a.push(Value::Int(beg as i64));
                a.push(Value::Int((end - beg) as i64));
                Value::Array(a)
            }
            2 => {
                let mut a = PhpArray::new();
                regs_array(&re, &region, s, true, &mut a);
                Value::Array(a)
            }
            _ => Value::Bool(true),
        };
        let end = end as i64;
        st(|st| {
            st.search_pos = if pos as i64 <= end {
                end as usize
            } else {
                pos + 1
            }
        });
    }
    if err >= 0 {
        st(|st| st.search_regs = Some(region));
    }
    Ok(ret)
}

fn fn_mb_ereg_search(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ereg_search_exec(host, args, "mb_ereg_search", 0)
}

fn fn_mb_ereg_search_pos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ereg_search_exec(host, args, "mb_ereg_search_pos", 1)
}

fn fn_mb_ereg_search_regs(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ereg_search_exec(host, args, "mb_ereg_search_regs", 2)
}

/// `mb_ereg_search_init(string $string, ?string $pattern = null, ?string $options = null): bool`.
fn fn_mb_ereg_search_init(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_ereg_search_init";
    let string = str_arg(host, args, 0, f)?;
    let pattern = super::opt_str_arg(host, args, 1, f)?;
    let options = options_arg(host, args, 2, f)?;
    if pattern.as_ref().is_some_and(|p| p.is_empty()) {
        return Err(empty_error(f, 1));
    }
    let (option, syntax) = match options {
        Some(o) => init_options(&o)?,
        None => defaults(),
    };
    if let Some(p) = pattern {
        match compile_pattern(host, f, &p, option, syntax)? {
            Some(re) => {
                let old = st(|s| s.search_re.replace(re));
                drop(old);
            }
            None => {
                let old = st(|s| s.search_re.take());
                drop(old);
                return Ok(Value::Bool(false));
            }
        }
    }
    let ok = check_encoding(&string, mbctype_encoding());
    let len = string.len();
    let old = st(|s| {
        s.search_str = Some(string.0.clone());
        s.search_pos = if ok { 0 } else { len };
        s.search_regs.take()
    });
    drop(old);
    Ok(Value::Bool(ok))
}

/// `mb_ereg_search_getregs(): array|false`.
fn fn_mb_ereg_search_getregs(_host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    STATE.with(|s| {
        let s = s.borrow();
        match (&s.search_regs, &s.search_str, &s.search_re) {
            (Some(region), Some(string), Some(re)) => {
                let mut a = PhpArray::new();
                regs_array(re, region, string.as_bytes(), true, &mut a);
                Ok(Value::Array(a))
            }
            _ => Ok(Value::Bool(false)),
        }
    })
}

/// `mb_ereg_search_getpos(): int`.
fn fn_mb_ereg_search_getpos(_host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Int(st(|s| s.search_pos) as i64))
}

/// `mb_ereg_search_setpos(int $offset): bool`.
fn fn_mb_ereg_search_setpos(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_ereg_search_setpos";
    let mut position = super::long_arg(host, args, 0, f)?;
    let len = st(|s| s.search_str.as_ref().map(|s| s.len() as i64));
    if position < 0 {
        if let Some(len) = len {
            position += len;
        }
    }
    if position < 0 || len.is_some_and(|len| position > len) {
        return Err(value_error(f, 0, "is out of range"));
    }
    st(|s| s.search_pos = position as usize);
    Ok(Value::Bool(true))
}

/// `mb_regex_set_options(?string $options = null): string`: the options
/// before the call.
fn fn_mb_regex_set_options(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "mb_regex_set_options";
    let (opt, syntax) = match super::opt_str_arg(host, args, 0, f)? {
        Some(o) => {
            let (opt, syntax) = init_options(&o)?;
            st(|s| {
                let prev = (s.default_options, s.default_syntax);
                s.default_options = opt;
                s.default_syntax = syntax;
                prev
            })
        }
        None => defaults(),
    };
    Ok(Value::str(option_string(opt, syntax)))
}

/// The oniguruma version php reports (`MB_ONIGURUMA_VERSION`, phpinfo):
/// the linked oniguruma.h's.
pub(crate) const ONIGURUMA_VERSION: &str = concat!(
    env!("RHP_ONIGURUMA_VERSION_MAJOR"),
    ".",
    env!("RHP_ONIGURUMA_VERSION_MINOR"),
    ".",
    env!("RHP_ONIGURUMA_VERSION_TEENY"),
);

pub(super) const FUNCTIONS: &[(&str, crate::registry::NativeFn)] = &[
    ("mb_regex_encoding", fn_mb_regex_encoding),
    ("mb_ereg", fn_mb_ereg),
    ("mb_eregi", fn_mb_eregi),
    ("mb_ereg_replace", fn_mb_ereg_replace),
    ("mb_eregi_replace", fn_mb_eregi_replace),
    ("mb_ereg_replace_callback", fn_mb_ereg_replace_callback),
    ("mb_split", fn_mb_split),
    ("mb_ereg_match", fn_mb_ereg_match),
    ("mb_ereg_search", fn_mb_ereg_search),
    ("mb_ereg_search_pos", fn_mb_ereg_search_pos),
    ("mb_ereg_search_regs", fn_mb_ereg_search_regs),
    ("mb_ereg_search_init", fn_mb_ereg_search_init),
    ("mb_ereg_search_getregs", fn_mb_ereg_search_getregs),
    ("mb_ereg_search_getpos", fn_mb_ereg_search_getpos),
    ("mb_ereg_search_setpos", fn_mb_ereg_search_setpos),
    ("mb_regex_set_options", fn_mb_regex_set_options),
];
