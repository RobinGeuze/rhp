//! htmlspecialchars(), htmlentities(), their decoders and
//! get_html_translation_table(): a port of ext/standard/html.c over php's
//! own tables (`html_tables`, generated from php-src).

use rhp_values::{ArrayKey, PhpArray, Value};

use crate::ext::standard::html_tables as t;
use crate::ext::standard::html_tables::Enc;
use crate::zend::zpp::{bool_param, int_param, str_or_null_param, str_param};
use crate::{Host, Thrown};

const ENT_HTML_QUOTE_SINGLE: i64 = 1;
const ENT_HTML_QUOTE_DOUBLE: i64 = 2;
const ENT_HTML_IGNORE_ERRORS: i64 = 4;
const ENT_HTML_SUBSTITUTE_ERRORS: i64 = 8;
const ENT_HTML_DOC_TYPE_MASK: i64 = 16 | 32;
const ENT_HTML_DOC_HTML401: i64 = 0;
const ENT_HTML_DOC_XML1: i64 = 16;
const ENT_HTML_DOC_XHTML: i64 = 32;
const ENT_HTML_DOC_HTML5: i64 = 16 | 32;
const ENT_HTML_SUBSTITUTE_DISALLOWED_CHARS: i64 = 128;
/// ENT_QUOTES | ENT_SUBSTITUTE, every function's default.
const DEFAULT_FLAGS: i64 = 3 | 8;
/// The longest entity name, without `&` and `;`.
const LONGEST_ENTITY_LENGTH: usize = 31;

/// html.c's `ENT_*` flags and `get_html_translation_table`'s tables.
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    vec![
        (
            "ENT_QUOTES",
            Value::Int(ENT_HTML_QUOTE_SINGLE | ENT_HTML_QUOTE_DOUBLE),
        ),
        ("ENT_COMPAT", Value::Int(ENT_HTML_QUOTE_DOUBLE)),
        ("ENT_NOQUOTES", Value::Int(0)),
        ("ENT_HTML401", Value::Int(ENT_HTML_DOC_HTML401)),
        ("ENT_HTML5", Value::Int(ENT_HTML_DOC_HTML5)),
        ("ENT_XML1", Value::Int(ENT_HTML_DOC_XML1)),
        ("ENT_XHTML", Value::Int(ENT_HTML_DOC_XHTML)),
        ("ENT_IGNORE", Value::Int(ENT_HTML_IGNORE_ERRORS)),
        ("ENT_SUBSTITUTE", Value::Int(ENT_HTML_SUBSTITUTE_ERRORS)),
        (
            "ENT_DISALLOWED",
            Value::Int(ENT_HTML_SUBSTITUTE_DISALLOWED_CHARS),
        ),
        ("HTML_SPECIALCHARS", Value::Int(0)),
        ("HTML_ENTITIES", Value::Int(1)),
    ]
}

/// `enum entity_charset`, in php's order (the comparisons below rely on it).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Cs {
    Utf8,
    Iso8859_1,
    Cp1252,
    Iso8859_15,
    Cp1251,
    Iso8859_5,
    Cp866,
    MacRoman,
    Koi8r,
    Big5,
    Gb2312,
    Big5Hkscs,
    Sjis,
    EucJp,
}

impl Cs {
    /// CHARSET_UNICODE_COMPAT
    fn unicode_compat(self) -> bool {
        self <= Cs::Iso8859_1
    }
    /// CHARSET_SINGLE_BYTE
    fn single_byte(self) -> bool {
        self > Cs::Utf8 && self < Cs::Big5
    }
    /// CHARSET_PARTIAL_SUPPORT
    fn partial(self) -> bool {
        self >= Cs::Big5
    }
    /// enc_to_uni_index
    fn to_uni(self) -> Option<&'static [u16; 256]> {
        Some(match self {
            Cs::Cp1252 => &t::TO_UNI_CP1252,
            Cs::Iso8859_15 => &t::TO_UNI_ISO8859_15,
            Cs::Cp1251 => &t::TO_UNI_CP1251,
            Cs::Iso8859_5 => &t::TO_UNI_ISO8859_5,
            Cs::Cp866 => &t::TO_UNI_CP866,
            Cs::MacRoman => &t::TO_UNI_MACROMAN,
            Cs::Koi8r => &t::TO_UNI_KOI8R,
            _ => return None,
        })
    }
}

/// html_tables.h's charset_map.
const CHARSET_MAP: &[(&str, Cs)] = &[
    ("ISO-8859-1", Cs::Iso8859_1),
    ("ISO8859-1", Cs::Iso8859_1),
    ("ISO-8859-15", Cs::Iso8859_15),
    ("ISO8859-15", Cs::Iso8859_15),
    ("utf-8", Cs::Utf8),
    ("cp1252", Cs::Cp1252),
    ("Windows-1252", Cs::Cp1252),
    ("1252", Cs::Cp1252),
    ("BIG5", Cs::Big5),
    ("950", Cs::Big5),
    ("GB2312", Cs::Gb2312),
    ("936", Cs::Gb2312),
    ("BIG5-HKSCS", Cs::Big5Hkscs),
    ("Shift_JIS", Cs::Sjis),
    ("SJIS", Cs::Sjis),
    ("932", Cs::Sjis),
    ("SJIS-win", Cs::Sjis),
    ("CP932", Cs::Sjis),
    ("EUCJP", Cs::EucJp),
    ("EUC-JP", Cs::EucJp),
    ("eucJP-win", Cs::EucJp),
    ("KOI8-R", Cs::Koi8r),
    ("koi8-ru", Cs::Koi8r),
    ("koi8r", Cs::Koi8r),
    ("cp1251", Cs::Cp1251),
    ("Windows-1251", Cs::Cp1251),
    ("win-1251", Cs::Cp1251),
    ("iso8859-5", Cs::Iso8859_5),
    ("iso-8859-5", Cs::Iso8859_5),
    ("cp866", Cs::Cp866),
    ("866", Cs::Cp866),
    ("ibm866", Cs::Cp866),
    ("MacRoman", Cs::MacRoman),
];

/// A C string: up to the first NUL.
fn c_str(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]
}

/// `determine_charset`: the hint, else internal_encoding, else
/// default_charset, else UTF-8; an unknown name warns unless `quiet`.
fn determine_charset(
    host: &mut dyn Host,
    fname: &str,
    hint: Option<&[u8]>,
    quiet: bool,
) -> Result<Cs, Thrown> {
    let mut hint = hint.map(c_str).unwrap_or_default().to_vec();
    if hint.is_empty() {
        // get_default_charset
        for ini in [&b"internal_encoding"[..], b"default_charset"] {
            let v = host.ini_get(ini).unwrap_or_default();
            let v = c_str(&v);
            if !v.is_empty() {
                hint = v.to_vec();
                break;
            }
        }
    }
    if hint.is_empty() {
        return Ok(Cs::Utf8);
    }
    if let Some(&(_, cs)) = CHARSET_MAP
        .iter()
        .find(|(name, _)| name.as_bytes().eq_ignore_ascii_case(&hint))
    {
        return Ok(cs);
    }
    if !quiet {
        crate::warn(
            host,
            fname,
            &format!(
                "Charset \"{}\" is not supported, assuming UTF-8",
                String::from_utf8_lossy(&hint)
            ),
        )?;
    }
    Ok(Cs::Utf8)
}

/// `get_next_char`: the character at `*cursor` and the cursor past it, or
/// `Err` with the cursor past the invalid sequence (UTR #36 strategy 2).
fn get_next_char(cs: Cs, s: &[u8], cursor: &mut usize) -> Result<u32, ()> {
    let pos = *cursor;
    let avail = s.len() - pos;
    let at = |i: usize| s.get(pos + i).copied().unwrap_or(0);
    let mut fail = |n: usize| {
        *cursor = pos + n;
        Err(())
    };
    if avail < 1 {
        return fail(1);
    }
    let c = s[pos];
    let utf8_lead = |c: u8| c < 0x80 || (0xC2..=0xF4).contains(&c);
    let utf8_trail = |c: u8| (0x80..=0xBF).contains(&c);
    let gb2312_lead = |c: u8| c != 0x8E && c != 0x8F && c != 0xA0 && c != 0xFF;
    let gb2312_trail = |c: u8| (0xA1..=0xFE).contains(&c);
    let sjis_lead = |c: u8| c != 0x80 && c != 0xA0 && c < 0xFD;
    let sjis_trail = |c: u8| c >= 0x40 && c != 0x7F && c < 0xFD;
    let (ch, len) = match cs {
        Cs::Utf8 => {
            if c < 0x80 {
                (c as u32, 1)
            } else if c < 0xC2 {
                return fail(1);
            } else if c < 0xE0 {
                if avail < 2 {
                    return fail(1);
                }
                if !utf8_trail(at(1)) {
                    return fail(if utf8_lead(at(1)) { 1 } else { 2 });
                }
                let ch = ((c as u32 & 0x1F) << 6) | (at(1) as u32 & 0x3F);
                if ch < 0x80 {
                    return fail(2);
                }
                (ch, 2)
            } else if c < 0xF0 {
                if avail < 3 || !utf8_trail(at(1)) || !utf8_trail(at(2)) {
                    return fail(if avail < 2 || utf8_lead(at(1)) {
                        1
                    } else if avail < 3 || utf8_lead(at(2)) {
                        2
                    } else {
                        3
                    });
                }
                let ch = ((c as u32 & 0x0F) << 12)
                    | ((at(1) as u32 & 0x3F) << 6)
                    | (at(2) as u32 & 0x3F);
                if ch < 0x800 || (0xD800..=0xDFFF).contains(&ch) {
                    return fail(3);
                }
                (ch, 3)
            } else if c < 0xF5 {
                if avail < 4 || !utf8_trail(at(1)) || !utf8_trail(at(2)) || !utf8_trail(at(3)) {
                    return fail(if avail < 2 || utf8_lead(at(1)) {
                        1
                    } else if avail < 3 || utf8_lead(at(2)) {
                        2
                    } else if avail < 4 || utf8_lead(at(3)) {
                        3
                    } else {
                        4
                    });
                }
                let ch = ((c as u32 & 0x07) << 18)
                    | ((at(1) as u32 & 0x3F) << 12)
                    | ((at(2) as u32 & 0x3F) << 6)
                    | (at(3) as u32 & 0x3F);
                if !(0x10000..=0x10FFFF).contains(&ch) {
                    return fail(4);
                }
                (ch, 4)
            } else {
                return fail(1);
            }
        }
        Cs::Big5 | Cs::Big5Hkscs => {
            if (0x81..=0xFE).contains(&c) {
                if avail < 2 {
                    return fail(1);
                }
                let next = at(1);
                if (0x40..=0x7E).contains(&next) || (0xA1..=0xFE).contains(&next) {
                    (((c as u32) << 8) | next as u32, 2)
                } else if cs == Cs::Big5 || (next != 0x80 && next != 0xFF) {
                    return fail(1);
                } else {
                    return fail(2);
                }
            } else {
                (c as u32, 1)
            }
        }
        Cs::Gb2312 => {
            if (0xA1..=0xFE).contains(&c) {
                if avail < 2 {
                    return fail(1);
                }
                let next = at(1);
                if gb2312_trail(next) {
                    (((c as u32) << 8) | next as u32, 2)
                } else if gb2312_lead(next) {
                    return fail(1);
                } else {
                    return fail(2);
                }
            } else if gb2312_lead(c) {
                (c as u32, 1)
            } else {
                return fail(1);
            }
        }
        Cs::Sjis => {
            if (0x81..=0x9F).contains(&c) || (0xE0..=0xFC).contains(&c) {
                if avail < 2 {
                    return fail(1);
                }
                let next = at(1);
                if sjis_trail(next) {
                    (((c as u32) << 8) | next as u32, 2)
                } else if sjis_lead(next) {
                    return fail(1);
                } else {
                    return fail(2);
                }
            } else if c < 0x80 || (0xA1..=0xDF).contains(&c) {
                (c as u32, 1)
            } else {
                return fail(1);
            }
        }
        Cs::EucJp => {
            let bad = |b: u8| if b != 0xA0 && b != 0xFF { 1 } else { 2 };
            if (0xA1..=0xFE).contains(&c) {
                if avail < 2 {
                    return fail(1);
                }
                let next = at(1);
                if (0xA1..=0xFE).contains(&next) {
                    (((c as u32) << 8) | next as u32, 2)
                } else {
                    return fail(bad(next));
                }
            } else if c == 0x8E {
                if avail < 2 {
                    return fail(1);
                }
                let next = at(1);
                if (0xA1..=0xDF).contains(&next) {
                    (((c as u32) << 8) | next as u32, 2)
                } else {
                    return fail(bad(next));
                }
            } else if c == 0x8F {
                let hi = |b: u8| (0xA1..=0xFE).contains(&b);
                if avail < 3 || !hi(at(1)) || !hi(at(2)) {
                    return fail(if avail < 2 || (at(1) != 0xA0 && at(1) != 0xFF) {
                        1
                    } else if avail < 3 || (at(2) != 0xA0 && at(2) != 0xFF) {
                        2
                    } else {
                        3
                    });
                }
                (((c as u32) << 16) | ((at(1) as u32) << 8) | at(2) as u32, 3)
            } else if c != 0xA0 && c != 0xFF {
                (c as u32, 1)
            } else {
                return fail(1);
            }
        }
        // single-byte charsets
        _ => (c as u32, 1),
    };
    *cursor = pos + len;
    Ok(ch)
}

/// `unimap_bsearch`.
fn unimap_bsearch(table: &[(u16, u8)], code: u32) -> Option<u32> {
    let code = u16::try_from(code).ok()?;
    table
        .binary_search_by_key(&code, |e| e.0)
        .ok()
        .map(|i| table[i].1 as u32)
}

/// `map_from_unicode`.
fn map_from_unicode(code: u32, cs: Cs) -> Option<u32> {
    let over_7f = |table: &[(u16, u8)]| {
        if code <= 0x7F {
            Some(code)
        } else {
            unimap_bsearch(table, code)
        }
    };
    match cs {
        Cs::Iso8859_1 => (code <= 0xFF).then_some(code),
        Cs::Iso8859_5 => {
            if code <= 0xA0 || code == 0xAD {
                Some(code)
            } else if code == 0x2116 {
                Some(0xF0)
            } else if code == 0xA7 {
                Some(0xFD)
            } else if (0x0401..=0x045F).contains(&code) {
                if matches!(code, 0x040D | 0x0450 | 0x045D) {
                    None
                } else {
                    Some(code - 0x360)
                }
            } else {
                None
            }
        }
        Cs::Iso8859_15 => {
            if code < 0xA4 || (code > 0xBE && code <= 0xFF) {
                Some(code)
            } else {
                unimap_bsearch(&t::FROM_UNI_ISO8859_15, code)
            }
        }
        Cs::Cp1252 => {
            if code <= 0x7F || (0xA0..=0xFF).contains(&code) {
                Some(code)
            } else {
                unimap_bsearch(&t::FROM_UNI_CP1252, code)
            }
        }
        Cs::MacRoman => {
            if code == 0x7F {
                None
            } else {
                over_7f(&t::FROM_UNI_MACROMAN)
            }
        }
        Cs::Cp1251 => over_7f(&t::FROM_UNI_CP1251),
        Cs::Koi8r => over_7f(&t::FROM_UNI_KOI8R),
        Cs::Cp866 => over_7f(&t::FROM_UNI_CP866),
        Cs::Sjis | Cs::EucJp => ((0x20..=0x7D).contains(&code) && code != 0x5C).then_some(code),
        Cs::Big5 | Cs::Big5Hkscs | Cs::Gb2312 => (0x20..=0x7D).contains(&code).then_some(code),
        Cs::Utf8 => None,
    }
}

/// `unicode_cp_is_allowed`.
fn unicode_cp_is_allowed(cp: u32, doctype: i64) -> bool {
    match doctype {
        ENT_HTML_DOC_HTML401 => {
            (0x20..=0x7E).contains(&cp)
                || matches!(cp, 0x0A | 0x09 | 0x0D)
                || (0xA0..=0xD7FF).contains(&cp)
                || (0xE000..=0x10FFFF).contains(&cp)
        }
        ENT_HTML_DOC_HTML5 => {
            (0x20..=0x7E).contains(&cp)
                || ((0x09..=0x0D).contains(&cp) && cp != 0x0B)
                || (0xA0..=0xD7FF).contains(&cp)
                || ((0xE000..=0x10FFFF).contains(&cp)
                    && (cp & 0xFFFF) < 0xFFFE
                    && !(0xFDD0..=0xFDEF).contains(&cp))
        }
        ENT_HTML_DOC_XHTML | ENT_HTML_DOC_XML1 => {
            (0x20..=0xD7FF).contains(&cp)
                || matches!(cp, 0x0A | 0x09 | 0x0D)
                || ((0xE000..=0x10FFFF).contains(&cp) && cp != 0xFFFE && cp != 0xFFFF)
        }
        _ => true,
    }
}

/// `numeric_entity_is_allowed`.
fn numeric_entity_is_allowed(cp: u32, doctype: i64) -> bool {
    match doctype {
        ENT_HTML_DOC_HTML401 => cp <= 0x10FFFF,
        ENT_HTML_DOC_HTML5 => {
            (0x20..=0x7E).contains(&cp)
                || ((0x09..=0x0C).contains(&cp) && cp != 0x0B)
                || ((0xA0..=0x10FFFF).contains(&cp)
                    && (cp & 0xFFFF) < 0xFFFE
                    && !(0xFDD0..=0xFDEF).contains(&cp))
        }
        _ => unicode_cp_is_allowed(cp, doctype),
    }
}

/// `process_numeric_entity`, from the byte after `#`: the code point and
/// the position of the closing `;`.
fn process_numeric_entity(s: &[u8], mut i: usize) -> Option<(u32, usize)> {
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let hex = matches!(at(i), b'x' | b'X');
    if hex {
        i += 1;
    }
    let digit = |c: u8| {
        if hex {
            c.is_ascii_hexdigit()
        } else {
            c.is_ascii_digit()
        }
    };
    if !digit(at(i)) {
        return None;
    }
    // ZEND_STRTOL: base 16 also takes a "0x" prefix
    if hex && at(i) == b'0' && matches!(at(i + 1), b'x' | b'X') && at(i + 2).is_ascii_hexdigit() {
        i += 2;
    }
    let mut v: u64 = 0;
    while digit(at(i)) {
        let d = (at(i) as char).to_digit(16).unwrap() as u64;
        v = v
            .saturating_mul(if hex { 16 } else { 10 })
            .saturating_add(d);
        i += 1;
    }
    if at(i) != b';' || v > 0x10FFFF {
        return None;
    }
    Some((v as u32, i))
}

/// `process_named_entity_html`: the length of an alphanumeric name at `i`
/// that a `;` ends.
fn process_named_entity(s: &[u8], i: usize) -> Option<usize> {
    let n = s[i.min(s.len())..]
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric())
        .count();
    (n > 0 && s.get(i + n) == Some(&b';')).then_some(n)
}

/// `resolve_named_entity_html`.
fn resolve_named_entity(name: &[u8], map: &[(&str, u32, u32)]) -> Option<(u32, u32)> {
    map.binary_search_by(|e| e.0.as_bytes().cmp(name))
        .ok()
        .map(|i| (map[i].1, map[i].2))
}

/// `unescape_inverse_map`.
fn inverse_map(all: bool, flags: i64) -> &'static [(&'static str, u32, u32)] {
    let doctype = flags & ENT_HTML_DOC_TYPE_MASK;
    match (all, doctype) {
        (true, ENT_HTML_DOC_HTML401 | ENT_HTML_DOC_XHTML) => &t::DEC_HTML4,
        (true, ENT_HTML_DOC_HTML5) => &t::DEC_HTML5,
        (false, ENT_HTML_DOC_HTML401) => &t::DEC_BASIC,
        _ => &t::DEC_BASIC_APOS,
    }
}

/// `determine_entity_table`.
enum Table {
    Multi(&'static [(u32, Enc)]),
    Basic(&'static [(u32, &'static str)]),
}

fn entity_table(all: bool, doctype: i64) -> Table {
    if all {
        Table::Multi(if doctype == ENT_HTML_DOC_HTML5 {
            &t::ENC_HTML5
        } else {
            &t::ENC_HTML4
        })
    } else {
        Table::Basic(if doctype == ENT_HTML_DOC_HTML401 {
            &t::ENC_BASIC
        } else {
            &t::ENC_BASIC_APOS
        })
    }
}

fn enc_row(table: &[(u32, Enc)], cp: u32) -> Option<&Enc> {
    table
        .binary_search_by_key(&cp, |e| e.0)
        .ok()
        .map(|i| &table[i].1)
}

/// `find_entity_for_char`: the entity for `k`, peeking at the next character
/// for the two-code-point entities (the cursor moves past it if used).
fn find_entity_for_char(
    k: u32,
    cs: Cs,
    table: &[(u32, Enc)],
    s: &[u8],
    cursor: &mut usize,
) -> Option<&'static str> {
    if (k & 0xFFF000) >> 12 > 0x1D {
        return None;
    }
    match enc_row(table, k)? {
        Enc::One(e) => Some(e),
        Enc::Multi(default, seconds) => {
            let before = *cursor;
            if *cursor < s.len() {
                if let Ok(next) = get_next_char(cs, s, cursor) {
                    if let Some(&(_, e)) = seconds.iter().find(|(c, _)| *c == next) {
                        return Some(e);
                    }
                }
            }
            *cursor = before;
            *default
        }
    }
}

/// `php_escape_html_entities_ex`.
#[allow(clippy::too_many_arguments)]
fn escape(
    host: &mut dyn Host,
    fname: &str,
    old: &[u8],
    all: bool,
    flags: i64,
    hint: Option<&[u8]>,
    double_encode: bool,
    quiet: bool,
) -> Result<Vec<u8>, Thrown> {
    let cs = determine_charset(host, fname, hint, quiet)?;
    let doctype = flags & ENT_HTML_DOC_TYPE_MASK;
    let mut all = all;
    if all {
        if cs.partial() {
            crate::notice(
                host,
                fname,
                "Only basic entities substitution is supported for multi-byte encodings other than UTF-8; functionality is equivalent to htmlspecialchars",
            )?;
        }
        // LIMIT_ALL
        all = !cs.partial() && doctype != ENT_HTML_DOC_XML1;
    }
    let table = entity_table(all, doctype);
    let to_uni = if all && !cs.unicode_compat() {
        cs.to_uni()
    } else {
        None
    };
    let inv_map = inverse_map(true, flags);
    let replacement: &[u8] = if cs == Cs::Utf8 {
        b"\xEF\xBF\xBD"
    } else {
        b"&#xFFFD;"
    };
    let quote_off = |c: u32| {
        (c == b'\'' as u32 && flags & ENT_HTML_QUOTE_SINGLE == 0)
            || (c == b'"' as u32 && flags & ENT_HTML_QUOTE_DOUBLE == 0)
    };

    let mut out = Vec::with_capacity(old.len() + old.len() / 4);
    let mut cursor = 0;
    while cursor < old.len() {
        let before = cursor;
        let Ok(mut ch) = get_next_char(cs, old, &mut cursor) else {
            if flags & ENT_HTML_IGNORE_ERRORS != 0 {
                continue;
            } else if flags & ENT_HTML_SUBSTITUTE_ERRORS != 0 {
                out.extend_from_slice(replacement);
                continue;
            }
            return Ok(Vec::new());
        };
        let seq = &old[before..cursor];
        if ch != b'&' as u32 {
            if quote_off(ch) {
                out.extend_from_slice(seq);
                continue;
            }
            let mut rep = None;
            match table {
                Table::Multi(t) => {
                    if let Some(map) = to_uni {
                        ch = map[ch as usize] as u32;
                        if ch == 0xFFFF {
                            out.extend_from_slice(seq);
                            continue;
                        }
                    }
                    rep = find_entity_for_char(ch, cs, t, old, &mut cursor);
                }
                Table::Basic(t) => {
                    if ch < 64 {
                        rep = t.iter().find(|e| e.0 == ch).map(|e| e.1);
                    }
                }
            }
            if let Some(e) = rep {
                out.push(b'&');
                out.extend_from_slice(e.as_bytes());
                out.push(b';');
                continue;
            }
            let mut seq = seq;
            if flags & ENT_HTML_SUBSTITUTE_DISALLOWED_CHARS != 0 {
                // with a map to Unicode, `ch` is already a code point
                let disallowed = if cs.unicode_compat() || to_uni.is_some() {
                    !unicode_cp_is_allowed(ch, doctype)
                } else {
                    ch <= 0x7D && !unicode_cp_is_allowed(ch, doctype)
                };
                if disallowed {
                    seq = replacement;
                }
            }
            out.extend_from_slice(seq);
        } else if double_encode {
            out.extend_from_slice(b"&amp;");
        } else {
            // keep an existing valid entity
            let ent_len = if old.get(cursor) == Some(&b'#') {
                match process_numeric_entity(old, cursor + 1) {
                    Some((cp, end))
                        if flags & ENT_HTML_SUBSTITUTE_DISALLOWED_CHARS == 0
                            || numeric_entity_is_allowed(cp, doctype) =>
                    {
                        Some(end - cursor)
                    }
                    _ => None,
                }
            } else {
                process_named_entity(old, cursor).filter(|&n| {
                    let name = &old[cursor..cursor + n];
                    resolve_named_entity(name, inv_map).is_some()
                        || (doctype == ENT_HTML_DOC_XHTML && name == b"apos")
                })
            };
            match ent_len {
                Some(n) => {
                    out.push(b'&');
                    out.extend_from_slice(&old[cursor..cursor + n]);
                    out.push(b';');
                    cursor += n + 1;
                }
                None => out.extend_from_slice(b"&amp;"),
            }
        }
    }
    Ok(out)
}

/// `write_octet_sequence`: UTF-8, or one byte.
fn write_octets(out: &mut Vec<u8>, cs: Cs, code: u32) {
    if cs == Cs::Utf8 {
        let ch = char::from_u32(code).unwrap_or('\u{FFFD}');
        // surrogates are written raw, as php_utf32_utf8 does
        if (0xD800..=0xDFFF).contains(&code) {
            out.push(0xE0 | (code >> 12) as u8);
            out.push(0x80 | ((code >> 6) & 0x3F) as u8);
            out.push(0x80 | (code & 0x3F) as u8);
        } else {
            let mut b = [0; 4];
            out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
        }
    } else {
        out.push(code as u8);
    }
}

/// `php_unescape_html_entities` / `traverse_for_entities`.
fn unescape(
    host: &mut dyn Host,
    fname: &str,
    s: &[u8],
    all: bool,
    flags: i64,
    hint: Option<&[u8]>,
) -> Result<Vec<u8>, Thrown> {
    if !s.contains(&b'&') {
        return Ok(s.to_vec());
    }
    let cs = if all {
        determine_charset(host, fname, hint, false)?
    } else {
        Cs::Iso8859_1
    };
    let doctype = flags & ENT_HTML_DOC_TYPE_MASK;
    let map = inverse_map(all, flags);
    let mut out = Vec::with_capacity(s.len());
    let mut cur = 0;
    while cur < s.len() {
        let Some(amp) = s[cur..].iter().position(|&c| c == b'&').map(|p| p + cur) else {
            out.extend_from_slice(&s[cur..]);
            break;
        };
        out.extend_from_slice(&s[cur..amp]);
        cur = amp;
        if s.len() - cur < 4 {
            out.extend_from_slice(&s[cur..]);
            break;
        }
        // (code, code2, position of ';'), or None: not an entity
        let parsed = if s[cur + 1] == b'#' {
            process_numeric_entity(s, cur + 2)
                .filter(|&(code, _)| {
                    if !all && (code > 63 || !t::ENC_BASIC_APOS.iter().any(|e| e.0 == code)) {
                        return false;
                    }
                    unicode_cp_is_allowed(code, doctype)
                        && !(doctype == ENT_HTML_DOC_HTML5 && code == 0x0D)
                })
                .map(|(code, end)| (code, 0, end))
        } else {
            let name_start = cur + 1;
            let max = (LONGEST_ENTITY_LENGTH + 1).min(s.len() - name_start);
            s[name_start..name_start + max]
                .iter()
                .position(|&c| c == b';')
                .filter(|&n| n > 0)
                .and_then(|n| {
                    let name = &s[name_start..name_start + n];
                    match resolve_named_entity(name, map) {
                        Some((a, b)) => Some((a, b, name_start + n)),
                        None if doctype == ENT_HTML_DOC_XHTML && name == b"apos" => {
                            Some((b'\'' as u32, 0, name_start + n))
                        }
                        None => None,
                    }
                })
        };
        let Some((mut code, code2, end)) = parsed else {
            // invalid_incomplete_entity: just the '&'
            out.push(b'&');
            cur += 1;
            continue;
        };
        let quote_off = (code == b'\'' as u32 && flags & ENT_HTML_QUOTE_SINGLE == 0)
            || (code == b'"' as u32 && flags & ENT_HTML_QUOTE_DOUBLE == 0);
        let mapped = if cs == Cs::Utf8 {
            Some(code)
        } else {
            map_from_unicode(code, cs).filter(|_| code2 == 0)
        };
        match mapped {
            Some(c) if !quote_off => {
                code = c;
                write_octets(&mut out, cs, code);
                if code2 != 0 {
                    write_octets(&mut out, cs, code2);
                }
                cur = end + 1;
            }
            // invalid_complete_entity: the text up to the ';' stays
            _ => {
                out.extend_from_slice(&s[cur..end]);
                cur = end;
            }
        }
    }
    Ok(out)
}

fn fn_htmlspecialchars(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    html_entities(host, args, false, "htmlspecialchars")
}

fn fn_htmlentities(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    html_entities(host, args, true, "htmlentities")
}

/// `php_html_entities`.
fn html_entities(host: &mut dyn Host, args: &[Value], all: bool, f: &str) -> Result<Value, Thrown> {
    let s = str_param(host, args, 0, f, "string")?;
    let flags = int_param(host, args, 1, f, "flags", DEFAULT_FLAGS)?;
    let hint = str_or_null_param(host, args, 2, f, "encoding")?;
    let double_encode = bool_param(host, args, 3, f, "double_encode", true)?;
    if s.is_empty() {
        return Ok(Value::str(Vec::new()));
    }
    let flags = flags as i32 as i64;
    Ok(Value::str(escape(
        host,
        f,
        &s,
        all,
        flags,
        hint.as_deref(),
        double_encode,
        false,
    )?))
}

/// `php_escape_html_entities_ex(s, ENT_QUOTES|ENT_SUBSTITUTE, no double
/// encoding, quiet)` in the default charset (the URL rewriter's hidden
/// inputs).
pub(crate) fn escape_quotes_substitute(host: &mut dyn Host, s: &[u8]) -> Vec<u8> {
    let flags = ENT_HTML_QUOTE_SINGLE | ENT_HTML_QUOTE_DOUBLE | ENT_HTML_SUBSTITUTE_ERRORS;
    escape(host, "", s, false, flags, None, false, true).unwrap_or_default()
}

/// main.c's `escape_html`, for `html_errors`: htmlspecialchars with
/// ENT_COMPAT in the default charset, quietly; a message that is not valid
/// in that charset is escaped again with its invalid sequences substituted.
pub(crate) fn escape_html(host: &mut dyn Host, s: &[u8]) -> Vec<u8> {
    let flags = ENT_HTML_QUOTE_DOUBLE;
    let run = |host: &mut dyn Host, flags| {
        escape(host, "", s, false, flags, None, true, true).unwrap_or_default()
    };
    let out = run(host, flags);
    if out.is_empty() {
        run(host, flags | ENT_HTML_SUBSTITUTE_ERRORS)
    } else {
        out
    }
}

fn fn_htmlspecialchars_decode(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "htmlspecialchars_decode";
    let s = str_param(host, args, 0, f, "string")?;
    let flags = int_param(host, args, 1, f, "flags", DEFAULT_FLAGS)? as i32 as i64;
    Ok(Value::str(unescape(host, f, &s, false, flags, None)?))
}

fn fn_html_entity_decode(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "html_entity_decode";
    let s = str_param(host, args, 0, f, "string")?;
    let flags = int_param(host, args, 1, f, "flags", DEFAULT_FLAGS)? as i32 as i64;
    let hint = str_or_null_param(host, args, 2, f, "encoding")?;
    Ok(Value::str(unescape(
        host,
        f,
        &s,
        true,
        flags,
        hint.as_deref(),
    )?))
}

/// `write_s3row_data`: the row's entries, keyed by their characters.
fn write_row(arr: &mut PhpArray, row: &Enc, orig: u32, cs: Cs) {
    let mut key = Vec::new();
    write_octets(&mut key, cs, orig);
    let mut add = |key: &[u8], e: &str| {
        let v = format!("&{e};");
        arr.insert(ArrayKey::from_bytes(key), Value::str(v.into_bytes()));
    };
    match row {
        Enc::One(e) => add(&key, e),
        Enc::Multi(default, seconds) => {
            if let Some(d) = default {
                add(&key, d);
            }
            for &(second, e) in *seconds {
                let spe = if cs.unicode_compat() {
                    second
                } else {
                    match map_from_unicode(second, cs) {
                        Some(c) => c,
                        None => continue,
                    }
                };
                let mut k = key.clone();
                write_octets(&mut k, cs, spe);
                add(&k, e);
            }
        }
    }
}

fn fn_get_html_translation_table(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "get_html_translation_table";
    let all = int_param(host, args, 0, f, "table", 0)?;
    let flags = int_param(host, args, 1, f, "flags", DEFAULT_FLAGS)?;
    let hint = match args.get(2) {
        Some(_) => Some(str_param(host, args, 2, f, "encoding")?),
        None => None,
    };
    let cs = determine_charset(host, f, hint.as_deref(), false)?;
    let doctype = flags & ENT_HTML_DOC_TYPE_MASK;
    let all = all != 0 && !cs.partial() && doctype != ENT_HTML_DOC_XML1;
    let quote_off = |c: u32| {
        (c == b'\'' as u32 && flags & ENT_HTML_QUOTE_SINGLE == 0)
            || (c == b'"' as u32 && flags & ENT_HTML_QUOTE_DOUBLE == 0)
    };
    let mut arr = PhpArray::new();
    match entity_table(all, doctype) {
        Table::Multi(table) if cs.unicode_compat() => {
            // ISO-8859-1 has only the first 256 code points
            let limit = if cs.single_byte() { 0x100 } else { 0x1E000 };
            for (cp, row) in table.iter() {
                if *cp >= limit || quote_off(*cp) {
                    continue;
                }
                write_row(&mut arr, row, *cp, cs);
            }
        }
        Table::Multi(table) => {
            let map = cs.to_uni().expect("a single-byte charset");
            for i in 0..=0xFFu32 {
                if quote_off(i) {
                    continue;
                }
                if let Some(row) = enc_row(table, map[i as usize] as u32) {
                    write_row(&mut arr, row, i, cs);
                }
            }
        }
        Table::Basic(table) => {
            for &(cp, e) in table {
                if quote_off(cp) {
                    continue;
                }
                write_row(&mut arr, &Enc::One(e), cp, Cs::Iso8859_1);
            }
        }
    }
    Ok(Value::Array(arr))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("htmlspecialchars", fn_htmlspecialchars),
            ("htmlspecialchars_decode", fn_htmlspecialchars_decode),
            ("htmlentities", fn_htmlentities),
            ("html_entity_decode", fn_html_entity_decode),
            ("get_html_translation_table", fn_get_html_translation_table),
        ],
    );
}

#[cfg(test)]
mod tests;
