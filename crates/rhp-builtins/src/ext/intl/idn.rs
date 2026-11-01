//! `idn_to_ascii` / `idn_to_utf8` (php-src `ext/intl/idn/idn.cpp`), backed by
//! a UTS #46 processor that mirrors ICU's `uts46.cpp` (`UTS46::process`,
//! `processUnicode`, `processLabel`, `markBadACELabel`, the BiDi and CONTEXTJ
//! checks) so that the `errors` bitmask, `isTransitionalDifferent` and the
//! partially-processed `result` php exposes through `$idna_info` match.
//!
//! The UTS #46 mapping + NFC step is ICU4X's `Uts46Mapper` (same data ICU
//! uses: disallowed characters become U+FFFD, deviation characters pass
//! through); Punycode is the `idna` crate's RFC 3492 codec.

use icu_normalizer::properties::CanonicalCombiningClassMapBorrowed;
use icu_normalizer::uts46::Uts46MapperBorrowed;
use icu_properties::props::{BidiClass, GeneralCategory, JoiningType};
use icu_properties::CodePointMapData;
use rhp_values::{ArrayKey, PhpArray, Value};

use super::error_reset;
use crate::ext::zpp::{int_arg_or, set_ref, str_arg, value_error};
use crate::{Host, Thrown};

pub(super) const IDNA_DEFAULT: i64 = 48;
const USE_STD3_RULES: u32 = 2;
const CHECK_BIDI: u32 = 4;
const CHECK_CONTEXTJ: u32 = 8;
const NONTRANSITIONAL_TO_ASCII: u32 = 16;
const NONTRANSITIONAL_TO_UNICODE: u32 = 32;

const E_EMPTY_LABEL: u32 = 1;
const E_LABEL_TOO_LONG: u32 = 2;
const E_DOMAIN_NAME_TOO_LONG: u32 = 4;
const E_LEADING_HYPHEN: u32 = 8;
const E_TRAILING_HYPHEN: u32 = 0x10;
const E_HYPHEN_3_4: u32 = 0x20;
const E_LEADING_COMBINING_MARK: u32 = 0x40;
const E_DISALLOWED: u32 = 0x80;
const E_PUNYCODE: u32 = 0x100;
const E_LABEL_HAS_DOT: u32 = 0x200;
const E_INVALID_ACE_LABEL: u32 = 0x400;
const E_BIDI: u32 = 0x800;
const E_CONTEXTJ: u32 = 0x1000;

const SEVERE: u32 =
    E_LEADING_COMBINING_MARK | E_DISALLOWED | E_PUNYCODE | E_LABEL_HAS_DOT | E_INVALID_ACE_LABEL;

const FFFD: char = '\u{FFFD}';

#[derive(Default)]
struct Info {
    errors: u32,
    label_errors: u32,
    is_trans_diff: bool,
    is_bidi: bool,
    is_ok_bidi: bool,
}

/// ICU's `asciiData`: 1 for A-Z (lowercased), 0 for LDH and '.', -1 otherwise.
fn ascii_data(c: char) -> i8 {
    match c {
        'A'..='Z' => 1,
        'a'..='z' | '0'..='9' | '-' | '.' => 0,
        _ => -1,
    }
}

fn map_normalize(s: &[char]) -> Vec<char> {
    Uts46MapperBorrowed::new()
        .map_normalize(s.iter().copied())
        .collect()
}

struct Uts46 {
    options: u32,
}

impl Uts46 {
    /// `UTS46::process` for a whole domain name.
    fn process(&self, src: &[char], to_ascii: bool, info: &mut Info) -> Vec<char> {
        *info = Info {
            is_ok_bidi: true,
            ..Info::default()
        };
        if src.is_empty() {
            info.errors |= E_EMPTY_LABEL;
            return Vec::new();
        }
        let std3 = self.options & USE_STD3_RULES != 0;
        let mut dest: Vec<char> = Vec::with_capacity(src.len());
        let mut label_start = 0usize;
        let mut i = 0usize;
        // ASCII fast path
        loop {
            if i == src.len() {
                if to_ascii {
                    if i - label_start > 63 {
                        info.label_errors |= E_LABEL_TOO_LONG;
                    }
                    if i >= 254 && (i > 254 || label_start < i) {
                        info.errors |= E_DOMAIN_NAME_TOO_LONG;
                    }
                }
                info.errors |= info.label_errors;
                return dest;
            }
            let c = src[i];
            if c as u32 > 0x7f {
                break;
            }
            let d = ascii_data(c);
            if d > 0 {
                dest.push(c.to_ascii_lowercase());
            } else if d < 0 && std3 {
                break;
            } else {
                dest.push(c);
                if c == '-' {
                    if i == label_start + 3 && src[i - 1] == '-' {
                        i += 1;
                        break;
                    }
                    if i == label_start {
                        info.label_errors |= E_LEADING_HYPHEN;
                    }
                    if i + 1 == src.len() || src[i + 1] == '.' {
                        info.label_errors |= E_TRAILING_HYPHEN;
                    }
                } else if c == '.' {
                    if i == label_start {
                        info.label_errors |= E_EMPTY_LABEL;
                    }
                    if to_ascii && i - label_start > 63 {
                        info.label_errors |= E_LABEL_TOO_LONG;
                    }
                    info.errors |= info.label_errors;
                    info.label_errors = 0;
                    label_start = i + 1;
                }
            }
            i += 1;
        }
        let _ = i;
        // Slow path: map + normalize everything from the current label on.
        dest.truncate(label_start);
        let dest = self.process_unicode(dest, &src[label_start..], label_start, to_ascii, info);
        if info.is_bidi
            && info.errors & SEVERE == 0
            && (!info.is_ok_bidi || (label_start > 0 && !is_ascii_ok_bidi(&dest[..label_start])))
        {
            info.errors |= E_BIDI;
        }
        dest
    }

    fn process_unicode(
        &self,
        mut dest: Vec<char>,
        rest: &[char],
        mut label_start: usize,
        to_ascii: bool,
        info: &mut Info,
    ) -> Vec<char> {
        let mut mapped = map_normalize(rest);
        let do_map_dev = if to_ascii {
            self.options & NONTRANSITIONAL_TO_ASCII == 0
        } else {
            self.options & NONTRANSITIONAL_TO_UNICODE == 0
        };
        let is_dev = |c: char| matches!(c, '\u{DF}' | '\u{3C2}' | '\u{200C}' | '\u{200D}');
        if mapped.iter().any(|&c| is_dev(c)) {
            info.is_trans_diff = true;
            if do_map_dev {
                let mut m = Vec::with_capacity(mapped.len() + 4);
                for c in mapped {
                    match c {
                        '\u{DF}' => m.extend(['s', 's']),
                        '\u{3C2}' => m.push('\u{3C3}'),
                        '\u{200C}' | '\u{200D}' => {}
                        o => m.push(o),
                    }
                }
                mapped = map_normalize(&m);
            }
        }
        dest.extend_from_slice(&mapped);
        let mut label_limit = label_start;
        while label_limit < dest.len() {
            if dest[label_limit] == '.' {
                let label_len = label_limit - label_start;
                let new_len = self.process_label(&mut dest, label_start, label_len, to_ascii, info);
                info.errors |= info.label_errors;
                info.label_errors = 0;
                label_start += new_len + 1;
                label_limit = label_start;
                continue;
            }
            label_limit += 1;
        }
        // An empty label at the end is fine; elsewhere (or an empty name) not.
        if label_start == 0 || label_start < label_limit {
            self.process_label(
                &mut dest,
                label_start,
                label_limit - label_start,
                to_ascii,
                info,
            );
            info.errors |= info.label_errors;
        }
        if to_ascii {
            let n = dest.len();
            let trailing_dot = dest.last() == Some(&'.');
            if n >= 254 && (n > 254 || !trailing_dot) {
                info.errors |= E_DOMAIN_NAME_TOO_LONG;
            }
        }
        dest
    }

    /// `processLabel`: validate (and for toASCII, Punycode-encode) the label at
    /// `dest[start..start+len]`, replacing it in place. Returns the new length.
    fn process_label(
        &self,
        dest: &mut Vec<char>,
        start: usize,
        len: usize,
        to_ascii: bool,
        info: &mut Info,
    ) -> usize {
        if len == 0 {
            info.label_errors |= E_EMPTY_LABEL;
            return 0;
        }
        let orig: Vec<char> = dest[start..start + len].to_vec();
        let was_punycode = len >= 4 && orig[..4] == ['x', 'n', '-', '-'];
        let mut label: Vec<char> = if was_punycode {
            let enc: String = orig[4..].iter().collect();
            let decoded = if enc.is_ascii() {
                idna::punycode::decode(&enc)
            } else {
                None
            };
            let Some(decoded) = decoded else {
                info.label_errors |= E_PUNYCODE;
                return self.mark_bad_ace_label(dest, start, len, to_ascii, info);
            };
            let valid = !decoded.is_empty()
                && !decoded.iter().all(|c| c.is_ascii())
                && Uts46MapperBorrowed::new()
                    .normalize_validate(decoded.iter().copied())
                    .eq(decoded.iter().copied());
            if !valid {
                info.label_errors |= E_INVALID_ACE_LABEL;
                return self.mark_bad_ace_label(dest, start, len, to_ascii, info);
            }
            decoded
        } else {
            orig.clone()
        };
        let n = label.len();
        if n >= 4 && label[2] == '-' && label[3] == '-' {
            info.label_errors |= E_HYPHEN_3_4;
        }
        if label[0] == '-' {
            info.label_errors |= E_LEADING_HYPHEN;
        }
        if label[n - 1] == '-' {
            info.label_errors |= E_TRAILING_HYPHEN;
        }
        let std3 = self.options & USE_STD3_RULES != 0;
        let mut ored: u32 = 0;
        for c in label.iter_mut() {
            if (*c as u32) <= 0x7f {
                if *c == '.' {
                    info.label_errors |= E_LABEL_HAS_DOT;
                    *c = FFFD;
                } else if std3 && ascii_data(*c) < 0 {
                    info.label_errors |= E_DISALLOWED;
                    *c = FFFD;
                }
            } else {
                ored |= *c as u32;
                if *c == FFFD {
                    info.label_errors |= E_DISALLOWED;
                }
            }
        }
        let gc = CodePointMapData::<GeneralCategory>::new().get(label[0]);
        if matches!(
            gc,
            GeneralCategory::NonspacingMark
                | GeneralCategory::SpacingMark
                | GeneralCategory::EnclosingMark
        ) {
            info.label_errors |= E_LEADING_COMBINING_MARK;
            label[0] = FFFD;
        }
        if info.label_errors & SEVERE == 0 {
            if self.options & CHECK_BIDI != 0 && (!info.is_bidi || info.is_ok_bidi) {
                check_label_bidi(&label, info);
            }
            if self.options & CHECK_CONTEXTJ != 0
                && (ored & 0x200c) == 0x200c
                && !is_label_ok_contextj(&label)
            {
                info.label_errors |= E_CONTEXTJ;
            }
            if to_ascii {
                if was_punycode {
                    if len > 63 {
                        info.label_errors |= E_LABEL_TOO_LONG;
                    }
                    return len;
                } else if ored >= 0x80 {
                    let enc = idna::punycode::encode(&label).unwrap_or_default();
                    let mut p: Vec<char> = "xn--".chars().collect();
                    p.extend(enc.chars());
                    if p.len() > 63 {
                        info.label_errors |= E_LABEL_TOO_LONG;
                    }
                    return replace_label(dest, start, len, &p);
                } else if n > 63 {
                    info.label_errors |= E_LABEL_TOO_LONG;
                }
            }
        } else if was_punycode {
            info.label_errors |= E_INVALID_ACE_LABEL;
            return self.mark_bad_ace_label(dest, start, len, to_ascii, info);
        }
        replace_label(dest, start, len, &label)
    }

    fn mark_bad_ace_label(
        &self,
        dest: &mut Vec<char>,
        start: usize,
        mut len: usize,
        to_ascii: bool,
        info: &mut Info,
    ) -> usize {
        let std3 = self.options & USE_STD3_RULES != 0;
        let mut is_ascii = true;
        let mut only_ldh = true;
        for c in dest[start + 4..start + len].iter_mut() {
            if (*c as u32) <= 0x7f {
                if *c == '.' {
                    info.label_errors |= E_LABEL_HAS_DOT;
                    *c = FFFD;
                    is_ascii = false;
                    only_ldh = false;
                } else if ascii_data(*c) < 0 {
                    only_ldh = false;
                    if std3 {
                        *c = FFFD;
                        is_ascii = false;
                    }
                }
            } else {
                is_ascii = false;
                only_ldh = false;
            }
        }
        if only_ldh {
            dest.insert(start + len, FFFD);
            len += 1;
        } else if to_ascii && is_ascii && len > 63 {
            info.label_errors |= E_LABEL_TOO_LONG;
        }
        len
    }
}

fn replace_label(dest: &mut Vec<char>, start: usize, len: usize, new: &[char]) -> usize {
    dest.splice(start..start + len, new.iter().copied());
    new.len()
}

// BidiClass masks (ICU's U_MASK(UCharDirection)).
fn bidi_bit(c: char) -> u32 {
    let bc = CodePointMapData::<BidiClass>::new().get(c);
    let idx = match bc {
        BidiClass::LeftToRight => 0,
        BidiClass::RightToLeft => 1,
        BidiClass::EuropeanNumber => 2,
        BidiClass::EuropeanSeparator => 3,
        BidiClass::EuropeanTerminator => 4,
        BidiClass::ArabicNumber => 5,
        BidiClass::CommonSeparator => 6,
        BidiClass::ParagraphSeparator => 7,
        BidiClass::SegmentSeparator => 8,
        BidiClass::WhiteSpace => 9,
        BidiClass::OtherNeutral => 10,
        BidiClass::LeftToRightEmbedding => 11,
        BidiClass::LeftToRightOverride => 12,
        BidiClass::ArabicLetter => 13,
        BidiClass::RightToLeftEmbedding => 14,
        BidiClass::RightToLeftOverride => 15,
        BidiClass::PopDirectionalFormat => 16,
        BidiClass::NonspacingMark => 17,
        BidiClass::BoundaryNeutral => 18,
        BidiClass::FirstStrongIsolate => 19,
        BidiClass::LeftToRightIsolate => 20,
        BidiClass::RightToLeftIsolate => 21,
        BidiClass::PopDirectionalIsolate => 22,
        _ => 10,
    };
    1 << idx
}

const M_L: u32 = 1 << 0;
const M_R: u32 = 1 << 1;
const M_EN: u32 = 1 << 2;
const M_ES: u32 = 1 << 3;
const M_ET: u32 = 1 << 4;
const M_AN: u32 = 1 << 5;
const M_CS: u32 = 1 << 6;
const M_ON: u32 = 1 << 10;
const M_AL: u32 = 1 << 13;
const M_NSM: u32 = 1 << 17;
const M_BN: u32 = 1 << 18;

fn check_label_bidi(label: &[char], info: &mut Info) {
    let first = bidi_bit(label[0]);
    if first & !(M_L | M_R | M_AL) != 0 {
        info.is_ok_bidi = false;
    }
    let mut end = label.len();
    let mut last = first;
    let mut i = 1;
    loop {
        if i >= end {
            break;
        }
        end -= 1;
        let b = bidi_bit(label[end]);
        if b != M_NSM {
            last = b;
            break;
        }
    }
    let bad_end = if first & M_L != 0 {
        last & !(M_L | M_EN) != 0
    } else {
        last & !(M_R | M_AL | M_EN | M_AN) != 0
    };
    if bad_end {
        info.is_ok_bidi = false;
    }
    let mut mask = first | last;
    while i < end {
        mask |= bidi_bit(label[i]);
        i += 1;
    }
    if first & M_L != 0 {
        if mask & !(M_L | M_EN | M_ES | M_CS | M_ET | M_ON | M_BN | M_NSM) != 0 {
            info.is_ok_bidi = false;
        }
    } else {
        if mask & !(M_R | M_AL | M_AN | M_EN | M_ES | M_CS | M_ET | M_ON | M_BN | M_NSM) != 0 {
            info.is_ok_bidi = false;
        }
        if mask & (M_EN | M_AN) == (M_EN | M_AN) {
            info.is_ok_bidi = false;
        }
    }
    if mask & (M_R | M_AL | M_AN) != 0 {
        info.is_bidi = true;
    }
}

/// `isASCIIOkBiDi` over the already-processed ASCII labels.
fn is_ascii_ok_bidi(s: &[char]) -> bool {
    let mut label_start = 0;
    for (i, &c) in s.iter().enumerate() {
        if c == '.' {
            if i > label_start {
                let p = s[i - 1];
                if !(p.is_ascii_lowercase() || p.is_ascii_digit()) {
                    return false;
                }
            }
            label_start = i + 1;
        } else if i == label_start {
            if !c.is_ascii_lowercase() {
                return false;
            }
        } else if (c as u32) <= 0x20 && ((c as u32) >= 0x1c || (9..=0xd).contains(&(c as u32))) {
            return false;
        }
    }
    true
}

fn is_label_ok_contextj(label: &[char]) -> bool {
    let ccc = CanonicalCombiningClassMapBorrowed::new();
    let jt = CodePointMapData::<JoiningType>::new();
    for i in 0..label.len() {
        if label[i] == '\u{200C}' {
            if i == 0 {
                return false;
            }
            let mut j = i - 1;
            if ccc.get_u8(label[j]) == 9 {
                continue;
            }
            loop {
                let t = jt.get(label[j]);
                if t == JoiningType::Transparent {
                    if j == 0 {
                        return false;
                    }
                    j -= 1;
                } else if t == JoiningType::LeftJoining || t == JoiningType::DualJoining {
                    break;
                } else {
                    return false;
                }
            }
            let mut j = i + 1;
            loop {
                if j == label.len() {
                    return false;
                }
                let t = jt.get(label[j]);
                j += 1;
                if t == JoiningType::Transparent {
                    continue;
                } else if t == JoiningType::RightJoining || t == JoiningType::DualJoining {
                    break;
                } else {
                    return false;
                }
            }
        } else if label[i] == '\u{200D}' {
            if i == 0 {
                return false;
            }
            if ccc.get_u8(label[i - 1]) != 9 {
                return false;
            }
        }
    }
    true
}

fn handoff(host: &mut dyn Host, args: &[Value], to_ascii: bool) -> Result<Value, Thrown> {
    let f = if to_ascii {
        "idn_to_ascii"
    } else {
        "idn_to_utf8"
    };
    error_reset();
    let domain = str_arg(host, args, 0, f, "domain")?;
    let option = int_arg_or(host, args, 1, f, "flags", IDNA_DEFAULT)?;
    let variant = int_arg_or(host, args, 2, f, "variant", 1)?;
    if domain.is_empty() {
        return Err(value_error(f, 1, "domain", "must not be empty"));
    }
    if variant != 1 {
        return Err(value_error(
            f,
            2,
            "flags",
            "must be INTL_IDNA_VARIANT_UTS46",
        ));
    }
    let want_info = args.len() > 3;
    if want_info {
        set_ref(args, 3, Value::Array(PhpArray::new()));
    }
    let src: Vec<char> = String::from_utf8_lossy(&domain).chars().collect();
    let uts = Uts46 {
        options: option as u32,
    };
    let mut info = Info::default();
    let out: String = uts.process(&src, to_ascii, &mut info).into_iter().collect();
    let capac = if to_ascii { 255 } else { 252 * 4 };
    if out.len() >= capac {
        return Ok(Value::Bool(false));
    }
    if want_info {
        let mut a = PhpArray::new();
        a.insert(
            ArrayKey::from_bytes(b"result"),
            Value::str(out.clone().into_bytes()),
        );
        a.insert(
            ArrayKey::from_bytes(b"isTransitionalDifferent"),
            Value::Bool(info.is_trans_diff),
        );
        a.insert(
            ArrayKey::from_bytes(b"errors"),
            Value::Int(info.errors as i64),
        );
        set_ref(args, 3, Value::Array(a));
    }
    if info.errors == 0 {
        Ok(Value::str(out.into_bytes()))
    } else {
        Ok(Value::Bool(false))
    }
}

pub(super) fn fn_idn_to_ascii(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    handoff(host, args, true)
}

pub(super) fn fn_idn_to_utf8(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    handoff(host, args, false)
}
