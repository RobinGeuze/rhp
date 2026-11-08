//! `Locale` and `locale_*`, ported from php-src
//! ext/intl/locale/locale_methods.cpp onto ICU's `uloc`.

use std::ffi::{c_char, CStr};

use rhp_values::{ArrayKey, PhpArray, PhpStr, Value};

use super::error;
use super::sys::{self, UChar, UErrorCode};
use super::{c_bytes, c_string, locale_get_default, utf16_to_utf8};
use crate::zend::zpp::{self, deref};
use crate::{Host, Thrown};

const LANG: &str = "language";
const SCRIPT: &str = "script";
const REGION: &str = "region";
const VARIANT: &str = "variant";
const EXTLANG: &str = "extlang";
const GRANDFATHERED: &str = "grandfathered";
const PRIVATE: &str = "private";
const CANONICALIZE: &str = "canonicalize";
const DISP_NAME: &str = "name";

const MAX_NO_VARIANT: usize = 15;
const MAX_NO_EXTLANG: usize = 3;
const MAX_NO_PRIVATE: usize = 15;

const INTL_MAX_LOCALE_LEN: usize = sys::ULOC_FULLNAME_CAPACITY - 1;

/// The IANA registry's grandfathered and redundant tags, and their
/// preferred values (php's LOC_GRANDFATHERED and
/// LOC_PREFERRED_GRANDFATHERED).
const GRANDFATHERED_TAGS: &[(&str, Option<&str>)] = &[
    ("art-lojban", Some("jbo")),
    ("cel-gaulish", None),
    ("en-GB-oed", Some("en-GB-oxendict")),
    ("i-ami", Some("ami")),
    ("i-bnn", Some("bnn")),
    ("i-default", None),
    ("i-enochian", None),
    ("i-hak", Some("hak")),
    ("i-klingon", Some("tlh")),
    ("i-lux", Some("lb")),
    ("i-mingo", None),
    ("i-navajo", Some("nv")),
    ("i-pwn", Some("pwn")),
    ("i-tao", Some("tao")),
    ("i-tay", Some("tay")),
    ("i-tsu", Some("tsu")),
    ("no-bok", Some("nb")),
    ("no-nyn", Some("nn")),
    ("sgn-BE-FR", Some("sfb")),
    ("sgn-BE-NL", Some("vgt")),
    ("sgn-BR", Some("bzs")),
    ("sgn-CH-DE", Some("sgg")),
    ("sgn-CO", Some("csn")),
    ("sgn-DE", Some("gsg")),
    ("sgn-DK", Some("dsl")),
    ("sgn-ES", Some("ssp")),
    ("sgn-FR", Some("fsl")),
    ("sgn-GB", Some("bfi")),
    ("sgn-GR", Some("gss")),
    ("sgn-IE", Some("isg")),
    ("sgn-IT", Some("ise")),
    ("sgn-JP", Some("jsl")),
    ("sgn-MX", Some("mfs")),
    ("sgn-NI", Some("ncs")),
    ("sgn-NL", Some("dse")),
    ("sgn-NO", Some("nsl")),
    ("sgn-PT", Some("psr")),
    ("sgn-SE", Some("swl")),
    ("sgn-US", Some("ase")),
    ("sgn-ZA", Some("sfs")),
    ("zh-cmn", Some("cmn")),
    ("zh-cmn-Hans", Some("cmn-Hans")),
    ("zh-cmn-Hant", Some("cmn-Hant")),
    ("zh-gan", Some("gan")),
    ("zh-guoyu", Some("cmn")),
    ("zh-hakka", Some("hak")),
    ("zh-min", None),
    ("zh-min-nan", Some("nan")),
    ("zh-wuu", Some("wuu")),
    ("zh-xiang", Some("hsn")),
];

/// `findOffset(LOC_GRANDFATHERED, key)` (strcasecmp).
fn grandfathered(loc: &[u8]) -> Option<usize> {
    GRANDFATHERED_TAGS
        .iter()
        .position(|(t, _)| t.as_bytes().eq_ignore_ascii_case(loc))
}

/// `getPreferredTag`.
fn preferred_tag(loc: &[u8]) -> Option<Vec<u8>> {
    let (tag, pref) = GRANDFATHERED_TAGS[grandfathered(loc)?];
    Some(pref.unwrap_or(tag).as_bytes().to_vec())
}

fn is_id_sep(c: u8) -> bool {
    c == b'_' || c == b'-'
}

/// A C string's byte at `i` (0 past the end).
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// `getStrrtokenPos`.
fn strrtoken_pos(s: &[u8], saved: i64) -> i64 {
    let mut result = -1;
    let mut i = saved - 1;
    while i >= 0 {
        let c = at(s, i as usize);
        if is_id_sep(c) || c == b'@' {
            result = if i >= 2 && is_id_sep(at(s, i as usize - 2)) {
                i - 2
            } else {
                i
            };
            break;
        }
        i -= 1;
    }
    if result < 1 {
        -1
    } else {
        result
    }
}

/// `getSingletonPos`.
fn singleton_pos(s: &[u8]) -> i64 {
    let len = s.len();
    for i in 0..len {
        if is_id_sep(s[i]) {
            if i == 1 {
                return 0;
            } else if i + 2 < len && is_id_sep(s[i + 2]) {
                return i as i64 + 1;
            }
        }
    }
    -1
}

/// php_strtok_r over `"-_"`: the tokens.
fn tokens(s: &[u8]) -> Vec<&[u8]> {
    s.split(|&c| is_id_sep(c))
        .filter(|t| !t.is_empty())
        .collect()
}

type UlocGet = unsafe extern "C" fn(*const c_char, *mut c_char, i32, *mut UErrorCode) -> i32;

/// `get_icu_value_internal`: the value and php's result (1 found, -1 no
/// value, 0 error).
fn icu_value(loc: &[u8], tag: &str, from_parse: bool) -> (Option<Vec<u8>>, i32) {
    if loc.len() > INTL_MAX_LOCALE_LEN {
        return (None, 0);
    }
    let mut mod_loc = loc.to_vec();
    if tag != CANONICALIZE {
        if grandfathered(loc).is_some() {
            if tag == LANG {
                return (Some(loc.to_vec()), 0);
            }
            return (None, 0);
        }
        if from_parse {
            if tag == LANG
                && loc.len() > 1
                && matches!(loc[0], b'x' | b'X' | b'i' | b'I')
                && is_id_sep(loc[1])
            {
                return (Some(loc.to_vec()), 0);
            }
            let pos = singleton_pos(loc);
            if pos == 0 {
                return (None, 0);
            } else if pos > 0 {
                mod_loc = loc[..pos as usize - 1].to_vec();
            }
        }
    }
    let f: UlocGet = match tag {
        SCRIPT => sys::uloc_getScript,
        LANG => sys::uloc_getLanguage,
        REGION => sys::uloc_getCountry,
        VARIANT => sys::uloc_getVariant,
        _ => sys::uloc_canonicalize,
    };
    let cloc = c_string(&mod_loc);
    let mut buflen: i32 = 512;
    let mut buf: Vec<u8>;
    loop {
        buf = vec![0; buflen as usize + 1];
        let cap = buflen;
        let mut status = sys::U_ZERO_ERROR;
        // SAFETY: `buf` holds `cap` bytes and one more.
        buflen = unsafe { f(cloc.as_ptr(), buf.as_mut_ptr().cast(), cap, &mut status) };
        if sys::failure(status) {
            if status == sys::U_BUFFER_OVERFLOW_ERROR {
                buflen += 1;
                continue;
            }
            return (None, 0);
        }
        if buflen <= cap {
            break;
        }
    }
    if buflen == 0 {
        return (None, -1);
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    buf.truncate(end);
    (Some(buf), 1)
}

/// `get_private_subtags`.
fn private_subtags(loc: &[u8]) -> Option<Vec<u8>> {
    let mut m = loc;
    if m.is_empty() {
        return None;
    }
    loop {
        let pos = singleton_pos(m);
        if pos < 0 {
            return None;
        }
        let pos = pos as usize;
        if matches!(m[pos], b'x' | b'X') {
            if pos + 2 == m.len() {
                return None;
            }
            return Some(m.get(pos + 2..).unwrap_or_default().to_vec());
        }
        if pos + 1 >= m.len() {
            return None;
        }
        m = &m[pos + 1..];
    }
}

/// `strToMatch`: lowercased, hyphens as underscores; `None` for "".
fn str_to_match(s: &[u8]) -> Option<Vec<u8>> {
    if s.is_empty() {
        return None;
    }
    Some(
        s.iter()
            .map(|&c| {
                if c == b'-' {
                    b'_'
                } else {
                    c.to_ascii_lowercase()
                }
            })
            .collect(),
    )
}

fn skey(s: &str) -> ArrayKey {
    ArrayKey::Str(PhpStr::from(s.as_bytes()))
}

fn path(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
    p: &str,
) -> Result<Vec<u8>, Thrown> {
    zpp::path_param(host, args, i, f, p)
}

fn opt_path(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
    p: &str,
) -> Result<Option<Vec<u8>>, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => path(host, args, i, f, p).map(Some),
    }
}

fn or_default(loc: Vec<u8>) -> Vec<u8> {
    if loc.is_empty() {
        locale_get_default()
    } else {
        loc
    }
}

/// `get_icu_value_src_php`.
fn value_src(host: &mut dyn Host, f: &str, tag: &str, args: &[Value]) -> Result<Value, Thrown> {
    error::error_reset();
    let loc = or_default(path(host, args, 0, f, "locale")?);
    if super::locale_too_long(host, f, loc.len())? {
        return Ok(Value::Null);
    }
    let (v, result) = icu_value(&loc, tag, false);
    if result == -1 {
        return Ok(Value::str(""));
    }
    if let Some(v) = v {
        return Ok(Value::str(v));
    }
    if result == 0 {
        error::set(
            host,
            f,
            sys::U_ZERO_ERROR,
            &format!("unable to get locale {tag}"),
        )?;
    }
    Ok(Value::Null)
}

type UlocDisplay =
    unsafe extern "C" fn(*const c_char, *const c_char, *mut UChar, i32, *mut UErrorCode) -> i32;

/// `get_icu_disp_value_src_php`.
fn display_src(host: &mut dyn Host, f: &str, tag: &str, args: &[Value]) -> Result<Value, Thrown> {
    error::error_reset();
    let loc = path(host, args, 0, f, "locale")?;
    let disp = opt_path(host, args, 1, f, "displayLocale")?;
    if loc.len() > sys::ULOC_FULLNAME_CAPACITY {
        error::set(host, f, sys::U_ILLEGAL_ARGUMENT_ERROR, "name too long")?;
        return Ok(Value::Bool(false));
    }
    let loc = or_default(loc);
    let mut mod_loc = None;
    if tag != DISP_NAME && grandfathered(&loc).is_some() {
        if tag == LANG {
            mod_loc = preferred_tag(&loc);
        } else {
            return Ok(Value::Bool(false));
        }
    }
    let mod_loc = c_string(&mod_loc.unwrap_or(loc));
    let disp = c_string(&disp.unwrap_or_else(locale_get_default));
    let g: UlocDisplay = match tag {
        LANG => sys::uloc_getDisplayLanguage,
        SCRIPT => sys::uloc_getDisplayScript,
        REGION => sys::uloc_getDisplayCountry,
        VARIANT => sys::uloc_getDisplayVariant,
        _ => sys::uloc_getDisplayName,
    };
    let mut buflen: i32 = 512;
    let mut buf: Vec<UChar>;
    loop {
        buf = vec![0; buflen as usize];
        let cap = buflen;
        let mut status = sys::U_ZERO_ERROR;
        // SAFETY: `buf` holds `cap` units.
        buflen = unsafe {
            g(
                mod_loc.as_ptr(),
                disp.as_ptr(),
                buf.as_mut_ptr(),
                cap,
                &mut status,
            )
        };
        if sys::failure(status) {
            if status == sys::U_BUFFER_OVERFLOW_ERROR {
                if buflen > cap {
                    continue;
                }
                break;
            }
            error::set(host, f, status, &format!("unable to get locale {tag}"))?;
            return Ok(Value::Bool(false));
        }
        if buflen <= cap {
            break;
        }
    }
    match utf16_to_utf8(&buf[..(buflen.max(0) as usize).min(buf.len())]) {
        Ok(s) => Ok(Value::str(s)),
        Err(st) => {
            error::set(
                host,
                f,
                st,
                &format!("error converting display name for {tag} to UTF-8"),
            )?;
            Ok(Value::Bool(false))
        }
    }
}

fn get_keywords(host: &mut dyn Host, f: &str, args: &[Value]) -> Result<Value, Thrown> {
    error::error_reset();
    let loc = path(host, args, 0, f, "locale")?;
    if super::locale_too_long(host, f, loc.len())? {
        return Ok(Value::Null);
    }
    let loc = c_string(&or_default(loc));
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: a NUL-terminated locale name.
    let e = unsafe { sys::uloc_openKeywords(loc.as_ptr(), &mut status) };
    if e.is_null() {
        return Ok(Value::Null);
    }
    let mut out = PhpArray::new();
    loop {
        let mut klen = 0i32;
        // SAFETY: a live enumeration.
        let k = unsafe { sys::uenum_next(e, &mut klen, &mut status) };
        if k.is_null() {
            break;
        }
        let mut buf = vec![0u8; 101];
        // SAFETY: `buf` holds 100 bytes and one more.
        let mut n = unsafe {
            sys::uloc_getKeywordValue(loc.as_ptr(), k, buf.as_mut_ptr().cast(), 100, &mut status)
        };
        if status == sys::U_BUFFER_OVERFLOW_ERROR {
            status = sys::U_ZERO_ERROR;
            buf = vec![0u8; n as usize + 1];
            let len = n;
            // SAFETY: `buf` holds `len + 1` bytes.
            unsafe {
                sys::uloc_getKeywordValue(
                    loc.as_ptr(),
                    k,
                    buf.as_mut_ptr().cast(),
                    len + 1,
                    &mut status,
                )
            };
            n = len;
        }
        if sys::failure(status) {
            // SAFETY: closing the enumeration we opened.
            unsafe { sys::uenum_close(e) };
            error::set(
                host,
                f,
                sys::U_ILLEGAL_ARGUMENT_ERROR,
                "Error encountered while getting the keyword value for the keyword",
            )?;
            return Ok(Value::Bool(false));
        }
        buf.truncate(n.max(0) as usize);
        // SAFETY: ICU's NUL-terminated keyword.
        let key = unsafe { CStr::from_ptr(k) }.to_bytes();
        out.insert(ArrayKey::from_bytes(key), Value::str(buf));
    }
    // SAFETY: closing the enumeration we opened.
    unsafe { sys::uenum_close(e) };
    Ok(Value::Array(out))
}

/// `append_key_value`: Ok(true) appended, Ok(false) not found, Err(()) not
/// a string.
fn append_key_value(out: &mut Vec<u8>, arr: &PhpArray, key: &str) -> Result<bool, ()> {
    match arr.get(&skey(key)).map(deref) {
        Some(Value::Str(s)) => {
            if key != LANG && key != GRANDFATHERED {
                out.push(b'_');
            }
            out.extend_from_slice(s.as_bytes());
            Ok(true)
        }
        Some(_) => Err(()),
        None => Ok(false),
    }
}

fn add_prefix(out: &mut Vec<u8>, key: &str) {
    if key.starts_with(PRIVATE) {
        out.extend_from_slice(b"_x");
    }
}

/// `append_multiple_key_values`.
fn append_multiple(out: &mut Vec<u8>, arr: &PhpArray, key: &str) -> Result<(), ()> {
    match arr.get(&skey(key)).map(deref) {
        Some(Value::Str(s)) => {
            add_prefix(out, key);
            out.push(b'_');
            out.extend_from_slice(s.as_bytes());
            Ok(())
        }
        Some(Value::Array(a)) => {
            let mut first = true;
            for v in a.values() {
                let Value::Str(s) = deref(v) else {
                    return Err(());
                };
                if first {
                    add_prefix(out, key);
                    first = false;
                }
                out.push(b'_');
                out.extend_from_slice(s.as_bytes());
            }
            Ok(())
        }
        Some(_) => Err(()),
        None => {
            let max = match key {
                VARIANT => MAX_NO_VARIANT,
                EXTLANG => MAX_NO_EXTLANG,
                PRIVATE => MAX_NO_PRIVATE,
                _ => 0,
            };
            let mut first = true;
            for i in 0..max {
                let k = format!("{key}{i}");
                match arr.get(&skey(&k)).map(deref) {
                    Some(Value::Str(s)) => {
                        if first {
                            add_prefix(out, &k);
                            first = false;
                        }
                        out.push(b'_');
                        out.extend_from_slice(s.as_bytes());
                    }
                    Some(_) => return Err(()),
                    None => {}
                }
            }
            Ok(())
        }
    }
}

fn compose(host: &mut dyn Host, f: &str, args: &[Value]) -> Result<Value, Thrown> {
    error::error_reset();
    let arr = zpp::array_param(host, args, 0, f)?;
    if arr.is_empty() {
        return Ok(Value::Bool(false));
    }
    // handleAppendResult
    fn not_string(host: &mut dyn Host, f: &str) -> Result<Value, Thrown> {
        error::error_reset();
        error::set(
            host,
            f,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "parameter array element is not a string",
        )?;
        Ok(Value::Bool(false))
    }
    let mut out = Vec::new();
    match append_key_value(&mut out, &arr, GRANDFATHERED) {
        Ok(true) => return Ok(Value::str(out)),
        Err(()) => return not_string(host, f),
        Ok(false) => error::error_reset(),
    }
    match append_key_value(&mut out, &arr, LANG) {
        Ok(false) => {
            return Err(zpp::value_error(
                f,
                1,
                "subtags",
                "must contain a \"language\" key",
            ));
        }
        Err(()) => return not_string(host, f),
        Ok(true) => error::error_reset(),
    }
    let steps: [(&str, bool); 5] = [
        (EXTLANG, true),
        (SCRIPT, false),
        (REGION, false),
        (VARIANT, true),
        (PRIVATE, true),
    ];
    for (key, multiple) in steps {
        let r = if multiple {
            append_multiple(&mut out, &arr, key)
        } else {
            append_key_value(&mut out, &arr, key).map(|_| ())
        };
        if r.is_err() {
            return not_string(host, f);
        }
        error::error_reset();
    }
    Ok(Value::str(out))
}

/// `add_array_entry`.
fn add_array_entry(loc: &[u8], out: &mut PhpArray, key: &str) {
    let (value, result) = if key == PRIVATE {
        (private_subtags(loc), 1)
    } else {
        icu_value(loc, key, true)
    };
    if key == PRIVATE || key == VARIANT {
        if let (true, Some(v)) = (result > 0, value) {
            let toks = tokens(&v);
            let mut it = toks.iter();
            let mut cnt = 0;
            if let Some(first) = it.next() {
                out.insert(
                    ArrayKey::from_bytes(format!("{key}{cnt}").as_bytes()),
                    Value::str(*first),
                );
                cnt += 1;
                for t in it {
                    if t.len() <= 1 {
                        break;
                    }
                    out.insert(
                        ArrayKey::from_bytes(format!("{key}{cnt}").as_bytes()),
                        Value::str(*t),
                    );
                    cnt += 1;
                }
            }
        }
    } else if result == 1 {
        if let Some(v) = value {
            out.insert(skey(key), Value::str(v));
        }
    }
}

fn parse(host: &mut dyn Host, f: &str, args: &[Value]) -> Result<Value, Thrown> {
    error::error_reset();
    let loc = path(host, args, 0, f, "locale")?;
    if super::locale_too_long(host, f, loc.len())? {
        return Ok(Value::Null);
    }
    let loc = or_default(loc);
    let mut out = PhpArray::new();
    if grandfathered(&loc).is_some() {
        out.insert(skey(GRANDFATHERED), Value::str(loc));
    } else {
        for key in [LANG, SCRIPT, REGION, VARIANT, PRIVATE] {
            add_array_entry(&loc, &mut out, key);
        }
    }
    Ok(Value::Array(out))
}

fn all_variants(host: &mut dyn Host, f: &str, args: &[Value]) -> Result<Value, Thrown> {
    error::error_reset();
    let loc = or_default(path(host, args, 0, f, "locale")?);
    if super::locale_too_long(host, f, loc.len())? {
        return Ok(Value::Null);
    }
    let mut out = PhpArray::new();
    if grandfathered(&loc).is_none() {
        if let (Some(v), 1..) = icu_value(&loc, VARIANT, false) {
            let toks = tokens(&v);
            if let Some((first, rest)) = toks.split_first() {
                out.push(Value::str(*first));
                for t in rest {
                    if t.len() <= 1 {
                        break;
                    }
                    out.push(Value::str(*t));
                }
            }
        }
    }
    Ok(Value::Array(out))
}

fn filter_matches(host: &mut dyn Host, f: &str, args: &[Value]) -> Result<Value, Thrown> {
    error::error_reset();
    let lang_tag = path(host, args, 0, f, "languageTag")?;
    let loc_range = or_default(path(host, args, 1, f, "locale")?);
    let canonical = zpp::bool_param(host, args, 2, f, "canonicalize", false)?;
    if loc_range == b"*" {
        return Ok(Value::Bool(true));
    }
    if super::locale_too_long(host, f, loc_range.len())?
        || super::locale_too_long(host, f, lang_tag.len())?
    {
        return Ok(Value::Null);
    }
    let (tag, range) = if canonical {
        let (r, res) = icu_value(&loc_range, CANONICALIZE, false);
        let Some(r) = r.filter(|_| res > 0) else {
            error::set(
                host,
                f,
                sys::U_ZERO_ERROR,
                "unable to canonicalize loc_range",
            )?;
            return Ok(Value::Bool(false));
        };
        let (t, res) = icu_value(&lang_tag, CANONICALIZE, false);
        let Some(t) = t.filter(|_| res > 0) else {
            error::set(
                host,
                f,
                sys::U_ZERO_ERROR,
                "unable to canonicalize lang_tag",
            )?;
            return Ok(Value::Bool(false));
        };
        (t, r)
    } else {
        (lang_tag, loc_range)
    };
    let (Some(t), Some(r)) = (str_to_match(&tag), str_to_match(&range)) else {
        return Ok(Value::Bool(false));
    };
    // strstr() found the range at the start of the tag
    if let Some(i) = find(&t, &r) {
        if i == 0 {
            let c = at(&t, r.len());
            if is_id_sep(c) || c == 0 || (canonical && c == b'@') {
                return Ok(Value::Bool(true));
            }
        }
    }
    Ok(Value::Bool(false))
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// `lookup_loc_range`: `Ok(None)` with the global error set or no match.
fn lookup_loc_range(
    host: &mut dyn Host,
    f: &str,
    loc_range: &[u8],
    arr: &PhpArray,
    canonicalize: bool,
) -> Result<Option<Vec<u8>>, Thrown> {
    let mut cur: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for v in arr.values() {
        let Value::Str(s) = deref(v) else {
            return Err(Thrown::type_error(format!(
                "{f}(): Argument #2 ($locale) must only contain string values"
            )));
        };
        let s = s.as_bytes().to_vec();
        if s.contains(&0) {
            return Err(zpp::value_error(
                f,
                2,
                "locale",
                "must not contain any null bytes",
            ));
        }
        let Some(m) = str_to_match(&s) else {
            error::set(
                host,
                f,
                sys::U_ILLEGAL_ARGUMENT_ERROR,
                "unable to canonicalize lang_tag",
            )?;
            return Ok(None);
        };
        cur.push((m, s));
    }
    if canonicalize {
        for e in cur.iter_mut() {
            let (t, res) = icu_value(&e.0, CANONICALIZE, false);
            let m = t
                .filter(|t| res == 1 && !t.is_empty())
                .and_then(|t| str_to_match(&t));
            let Some(m) = m else {
                error::set(
                    host,
                    f,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "unable to canonicalize lang_tag",
                )?;
                return Ok(None);
            };
            e.0 = m;
        }
    }
    let range = if canonicalize {
        let (r, res) = icu_value(loc_range, CANONICALIZE, false);
        match r.filter(|r| res == 1 && !r.is_empty()) {
            Some(r) => r,
            None => {
                error::set(
                    host,
                    f,
                    sys::U_ILLEGAL_ARGUMENT_ERROR,
                    "unable to canonicalize loc_range",
                )?;
                return Ok(None);
            }
        }
    } else {
        loc_range.to_vec()
    };
    let Some(range) = str_to_match(&range) else {
        error::set(
            host,
            f,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "unable to canonicalize lang_tag",
        )?;
        return Ok(None);
    };
    let mut saved = range.len() as i64;
    while saved > 0 {
        for (m, orig) in &cur {
            if m.len() as i64 == saved && range[..saved as usize] == m[..] {
                return Ok(Some(if canonicalize {
                    m.clone()
                } else {
                    orig.clone()
                }));
            }
        }
        saved = strrtoken_pos(&range, saved);
    }
    Ok(None)
}

fn lookup(host: &mut dyn Host, f: &str, args: &[Value]) -> Result<Value, Thrown> {
    error::error_reset();
    let arr = zpp::array_param(host, args, 0, f)?;
    let mut loc_range = path(host, args, 1, f, "locale")?;
    let canonical = zpp::bool_param(host, args, 2, f, "canonicalize", false)?;
    let fallback = opt_path(host, args, 3, f, "defaultLocale")?;
    if loc_range.is_empty() {
        loc_range = fallback.clone().unwrap_or_else(locale_get_default);
    }
    if super::locale_too_long(host, f, loc_range.len())? {
        return Ok(Value::Null);
    }
    if arr.is_empty() {
        return Ok(Value::str(""));
    }
    let r = lookup_loc_range(host, f, &loc_range, &arr, canonical)?;
    if sys::failure(error::error_code()) {
        return Ok(Value::Null);
    }
    match r.filter(|r| !r.is_empty()) {
        Some(r) => Ok(Value::str(r)),
        None => Ok(Value::str(fallback.unwrap_or_default())),
    }
}

// c_char is i8 or u8 depending on the target.
#[allow(clippy::unnecessary_cast)]
fn accept_from_http(host: &mut dyn Host, f: &str, args: &[Value]) -> Result<Value, Thrown> {
    let header = zpp::str_arg(host, args, 0, f, "header")?;
    if header.len() > sys::ULOC_FULLNAME_CAPACITY
        && header
            .split(|&c| c == b',')
            .any(|p| p.len() > sys::ULOC_FULLNAME_CAPACITY)
    {
        error::set(
            host,
            f,
            sys::U_ILLEGAL_ARGUMENT_ERROR,
            "locale string too long",
        )?;
        return Ok(Value::Bool(false));
    }
    let mut status = sys::U_ZERO_ERROR;
    // SAFETY: ICU's list of available locales.
    let available = unsafe { sys::ures_openAvailableLocales(std::ptr::null(), &mut status) };
    if error::check_global(host, status, f, "failed to retrieve locale list")? {
        return Ok(Value::Bool(false));
    }
    let http = c_string(&header);
    let mut result = [0 as c_char; INTL_MAX_LOCALE_LEN + 1];
    let mut out = 0i32;
    // SAFETY: a buffer of INTL_MAX_LOCALE_LEN + 1 bytes, of which ICU is
    // told INTL_MAX_LOCALE_LEN; then closing the enumeration.
    let len = unsafe {
        let n = sys::uloc_acceptLanguageFromHTTP(
            result.as_mut_ptr(),
            INTL_MAX_LOCALE_LEN as i32,
            &mut out,
            http.as_ptr(),
            available,
            &mut status,
        );
        sys::uenum_close(available);
        n
    };
    if error::check_global(host, status, f, "failed to find acceptable locale")? {
        return Ok(Value::Bool(false));
    }
    if len < 0 || out == sys::ULOC_ACCEPT_FAILED {
        return Ok(Value::Bool(false));
    }
    let bytes: Vec<u8> = result[..(len as usize).min(INTL_MAX_LOCALE_LEN)]
        .iter()
        .map(|&c| c as u8)
        .collect();
    Ok(Value::str(bytes))
}

/// `locale_add_likely_subtags` / `locale_minimize_subtags`.
// c_char is i8 or u8 depending on the target.
#[allow(clippy::unnecessary_cast)]
fn subtags(host: &mut dyn Host, f: &str, args: &[Value], add: bool) -> Result<Value, Thrown> {
    let loc = c_string(&or_default(path(host, args, 0, f, "locale")?));
    let mut buf = [0 as c_char; sys::ULOC_FULLNAME_CAPACITY];
    let mut status = sys::U_ZERO_ERROR;
    let g: UlocGet = if add {
        sys::uloc_addLikelySubtags
    } else {
        sys::uloc_minimizeSubtags
    };
    // SAFETY: a buffer of ULOC_FULLNAME_CAPACITY bytes.
    let len = unsafe {
        g(
            loc.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as i32,
            &mut status,
        )
    };
    if error::check_global(host, status, f, "invalid locale")? {
        return Ok(Value::Bool(false));
    }
    if len < 0 {
        return Ok(Value::Bool(false));
    }
    let bytes: Vec<u8> = buf[..(len as usize).min(buf.len())]
        .iter()
        .map(|&c| c as u8)
        .collect();
    Ok(Value::str(bytes))
}

fn set_default(host: &mut dyn Host, f: &str, args: &[Value]) -> Result<Value, Thrown> {
    let mut loc = path(host, args, 0, f, "locale")?;
    if loc.is_empty() {
        // SAFETY: ICU's static default name.
        loc = c_bytes(unsafe { sys::uloc_getDefault() });
    }
    crate::main::ini::alter_ini_entry(host, b"intl.default_locale", loc, f)?;
    Ok(Value::Bool(true))
}

fn dispatch(host: &mut dyn Host, name: &str, f: &str, args: &[Value]) -> Result<Value, Thrown> {
    match name {
        "getDefault" => Ok(Value::str(locale_get_default())),
        "setDefault" => set_default(host, f, args),
        "getPrimaryLanguage" => value_src(host, f, LANG, args),
        "getScript" => value_src(host, f, SCRIPT, args),
        "getRegion" => value_src(host, f, REGION, args),
        "canonicalize" => value_src(host, f, CANONICALIZE, args),
        "getKeywords" => get_keywords(host, f, args),
        "getDisplayScript" => display_src(host, f, SCRIPT, args),
        "getDisplayRegion" => display_src(host, f, REGION, args),
        "getDisplayName" => display_src(host, f, DISP_NAME, args),
        "getDisplayLanguage" => display_src(host, f, LANG, args),
        "getDisplayVariant" => display_src(host, f, VARIANT, args),
        "composeLocale" => compose(host, f, args),
        "parseLocale" => parse(host, f, args),
        "getAllVariants" => all_variants(host, f, args),
        "filterMatches" => filter_matches(host, f, args),
        "lookup" => lookup(host, f, args),
        "acceptFromHttp" => accept_from_http(host, f, args),
        "isRightToLeft" => {
            let loc = c_string(&or_default(path(host, args, 0, f, "locale")?));
            // SAFETY: a NUL-terminated locale name.
            Ok(Value::Bool(
                unsafe { sys::uloc_isRightToLeft(loc.as_ptr()) } != 0,
            ))
        }
        "addLikelySubtags" => subtags(host, f, args, true),
        "minimizeSubtags" => subtags(host, f, args, false),
        _ => Ok(Value::Null),
    }
}

const FUNCS: &[(&str, &str)] = &[
    ("locale_get_default", "getDefault"),
    ("locale_set_default", "setDefault"),
    ("locale_get_primary_language", "getPrimaryLanguage"),
    ("locale_get_script", "getScript"),
    ("locale_get_region", "getRegion"),
    ("locale_get_keywords", "getKeywords"),
    ("locale_get_display_script", "getDisplayScript"),
    ("locale_get_display_region", "getDisplayRegion"),
    ("locale_get_display_name", "getDisplayName"),
    ("locale_get_display_language", "getDisplayLanguage"),
    ("locale_get_display_variant", "getDisplayVariant"),
    ("locale_compose", "composeLocale"),
    ("locale_parse", "parseLocale"),
    ("locale_get_all_variants", "getAllVariants"),
    ("locale_filter_matches", "filterMatches"),
    ("locale_canonicalize", "canonicalize"),
    ("locale_lookup", "lookup"),
    ("locale_accept_from_http", "acceptFromHttp"),
    ("locale_is_right_to_left", "isRightToLeft"),
    ("locale_add_likely_subtags", "addLikelySubtags"),
    ("locale_minimize_subtags", "minimizeSubtags"),
];

macro_rules! procedural {
    ($($f:ident => $i:expr),*) => {
        $(fn $f(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            let (fname, name) = FUNCS[$i];
            dispatch(host, name, fname, args)
        })*
        pub(super) fn register(m: &mut crate::Registry) {
            let list: Vec<(&'static str, crate::NativeFn)> = vec![$((FUNCS[$i].0, $f as crate::NativeFn)),*];
            crate::register(m, &list);
        }
    };
}

procedural!(
    f0 => 0, f1 => 1, f2 => 2, f3 => 3, f4 => 4, f5 => 5, f6 => 6, f7 => 7, f8 => 8,
    f9 => 9, f10 => 10, f11 => 11, f12 => 12, f13 => 13, f14 => 14, f15 => 15, f16 => 16,
    f17 => 17, f18 => 18, f19 => 19, f20 => 20
);

/// `__locale($method, ...$args)`: the `Locale` static methods.
pub(super) fn fn_method(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let method = rhp_values::conv::to_string_bytes(&deref(&args[0]));
    let method = String::from_utf8_lossy(&method).into_owned();
    let fname = format!("Locale::{method}");
    dispatch(host, &method, &fname, &args[1..])
}
