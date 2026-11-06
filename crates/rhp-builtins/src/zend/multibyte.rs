//! zend.multibyte (Zend/zend_multibyte.c and the scanner's
//! open_file_for_scanning): a script's encoding is detected (BOM, UTF-16/32
//! sniffing, zend.script_encoding) and the source converted so the scanner
//! sees an encoding it can lex. When the conversion goes through UTF-8, the
//! scanner's output filter converts each string value back to the internal
//! encoding: rhp applies it to the compiled program's string constants.
//! mbstring provides the encodings, as in php; without it nothing converts.

/// `CG(multibyte)`: the zend.multibyte ini.
pub fn enabled() -> bool {
    crate::ini_value(b"zend.multibyte")
        .flatten()
        .is_some_and(|v| crate::main::ini::ini_parse_bool(&v))
}

/// A script as the scanner reads it.
pub struct Filtered {
    /// The source to lex (converted, BOM removed), when it differs.
    pub source: Option<Vec<u8>>,
    /// The scanner's output filter, if any: (from, to) encoding names.
    pub output: Option<OutputFilter>,
}

/// The scanner's output filter: converts string values.
pub struct OutputFilter {
    #[cfg(feature = "mbstring-libmbfl")]
    from: &'static rhp_mbfl_sys::Encoding,
    #[cfg(feature = "mbstring-libmbfl")]
    to: &'static rhp_mbfl_sys::Encoding,
}

impl OutputFilter {
    /// A string value converted by the filter.
    pub fn apply(&self, s: &[u8]) -> Vec<u8> {
        #[cfg(feature = "mbstring-libmbfl")]
        return crate::ext::mbstring::convert_script(s, self.to, self.from);
        #[cfg(not(feature = "mbstring-libmbfl"))]
        return s.to_vec();
    }
}

/// open_file_for_scanning under zend.multibyte: how `src` is lexed. `None`
/// when zend.multibyte is off or the source is lexed as it is.
#[cfg(not(feature = "mbstring-libmbfl"))]
pub fn filter(_src: &[u8]) -> Option<Filtered> {
    None
}

#[cfg(feature = "mbstring-libmbfl")]
pub fn filter(src: &[u8]) -> Option<Filtered> {
    use crate::ext::mbstring;
    use rhp_mbfl_sys::{encodings, Encoding};
    type Enc = &'static Encoding;
    if !enabled() {
        return None;
    }
    let by_name = |n: &[u8]| Encoding::by_name(n).expect("libmbfl knows the UTF encodings");
    let mut script = src;
    // zend_multibyte_find_script_encoding
    let detect_unicode = crate::ini_value(b"zend.detect_unicode")
        .flatten()
        .is_none_or(|v| crate::main::ini::ini_parse_bool(&v));
    let mut found: Option<Enc> = None;
    if detect_unicode {
        found = detect_unicode_encoding(&mut script).map(by_name);
    }
    if found.is_none() {
        found = match mbstring::script_encoding_list() {
            Some(list) if list.len() > 1 => mbstring::detect_script_encoding(script, &list),
            Some(list) => Some(list[0]),
            None => None,
        };
    }
    // A leading declare(encoding=...) re-filters the script from there on
    // (zend_handle_encoding_declaration): all of it, as the declare is ASCII.
    if let Some(enc) = declared_encoding(script).and_then(|n| Encoding::by_name(&n)) {
        found = Some(enc);
    }
    let stripped = script.len() != src.len();
    let Some(script_enc) = found else {
        return stripped.then(|| Filtered {
            source: Some(script.to_vec()),
            output: None,
        });
    };
    // zend_multibyte_set_filter
    let internal = mbstring::ini_internal_encoding();
    let compatible = |e: Enc| e.flag() & GL_UNSAFE == 0;
    let utf8 = encodings::utf8();
    let (input, output): (Option<Enc>, Option<(Enc, Enc)>) = if script_enc == internal {
        if compatible(script_enc) {
            (None, None)
        } else {
            (Some(utf8), Some((utf8, script_enc)))
        }
    } else if compatible(internal) {
        (Some(internal), None)
    } else if compatible(script_enc) {
        (None, Some((script_enc, internal)))
    } else {
        (Some(utf8), Some((utf8, internal)))
    };
    let source = match input {
        Some(to) => Some(mbstring::convert_script(script, to, script_enc)),
        None => stripped.then(|| script.to_vec()),
    };
    Some(Filtered {
        source,
        output: output.map(|(from, to)| OutputFilter { from, to }),
    })
}

/// The encoding a `declare(encoding=...)` at the start of `script` names.
#[cfg(feature = "mbstring-libmbfl")]
fn declared_encoding(script: &[u8]) -> Option<Vec<u8>> {
    use crate::ext::tokenizer::ids::*;
    let toks = crate::ext::tokenizer::lexer::tokenize(script);
    let mut it = toks
        .iter()
        .filter(|t| !matches!(t.id, T_OPEN_TAG | T_WHITESPACE | T_COMMENT | T_DOC_COMMENT));
    if it.next()?.id != T_DECLARE || it.next()?.id != 0x28 {
        return None;
    }
    loop {
        let name = it.next()?;
        if name.id != T_STRING || it.next()?.id != 0x3D {
            return None;
        }
        let value = it.next()?;
        if name.text(script).eq_ignore_ascii_case(b"encoding") {
            let v = value.text(script);
            return (value.id == T_CONSTANT_ENCAPSED_STRING && v.len() >= 2)
                .then(|| v[1..v.len() - 1].to_vec());
        }
        if it.next()?.id != 0x2C {
            return None;
        }
    }
}

/// `MBFL_ENCTYPE_GL_UNSAFE`: the encoding can't be lexed as it is.
#[cfg(feature = "mbstring-libmbfl")]
const GL_UNSAFE: u32 = 0x4000;

/// zend_multibyte_detect_unicode: a BOM (removed from `script`), or for a
/// script with NUL bytes before any `__HALT_COMPILER();`, UTF-16/32
/// sniffing. The encoding's name.
#[cfg(feature = "mbstring-libmbfl")]
fn detect_unicode_encoding(script: &mut &[u8]) -> Option<&'static [u8]> {
    let s = *script;
    if s.len() < 4 {
        return None;
    }
    const BOMS: [(&[u8], &[u8]); 5] = [
        (b"\x00\x00\xfe\xff", b"UTF-32BE"),
        (b"\xff\xfe\x00\x00", b"UTF-32LE"),
        (b"\xfe\xff", b"UTF-16BE"),
        (b"\xff\xfe", b"UTF-16LE"),
        (b"\xef\xbb\xbf", b"UTF-8"),
    ];
    for (bom, name) in BOMS {
        if s.starts_with(bom) {
            *script = &s[bom.len()..];
            return Some(name);
        }
    }
    let nul = s.iter().position(|&c| c == 0)?;
    if halt_compiler_before(&s[..nul]) {
        return None;
    }
    Some(detect_utf_encoding(s))
}

/// Whether `head` (the script up to its first NUL) holds
/// `__HALT_COMPILER();` (the scan php makes, whitespace allowed).
#[cfg(feature = "mbstring-libmbfl")]
fn halt_compiler_before(head: &[u8]) -> bool {
    let ws = |c: &u8| matches!(c, b' ' | b'\t' | b'\r' | b'\n');
    let lc = head.to_ascii_lowercase();
    let mut from = 0;
    while let Some(p) = lc[from..].windows(15).position(|w| w == b"__halt_compiler") {
        let mut i = from + p + 15;
        let skip = |i: &mut usize| {
            while lc.get(*i).is_some_and(ws) {
                *i += 1;
            }
        };
        skip(&mut i);
        if lc.get(i) == Some(&b'(') {
            i += 1;
            skip(&mut i);
            if lc.get(i) == Some(&b')') {
                i += 1;
                skip(&mut i);
                if lc.get(i) == Some(&b';') {
                    return true;
                }
            }
        }
        from += p + 1;
    }
    false
}

/// zend_multibyte_detect_utf_encoding: UTF-16 or UTF-32 (three NULs in a
/// row), big or little endian (where the first non-NUL byte sits).
#[cfg(feature = "mbstring-libmbfl")]
fn detect_utf_encoding(s: &[u8]) -> &'static [u8] {
    let mut wchar = 2;
    let mut offset = 0;
    while offset < s.len() && s.len() - offset > 2 {
        let Some(p) = s[offset..s.len() - 2].iter().position(|&c| c == 0) else {
            break;
        };
        let p = offset + p;
        if s[p + 1] == 0 && s[p + 2] == 0 {
            wchar = 4;
            break;
        }
        offset = p + 4;
    }
    let mut le = false;
    let mut offset = 0;
    while s.len() - offset >= wchar {
        let (first, last) = (s[offset], s[offset + wchar - 1]);
        if first == 0 && last != 0 {
            break;
        } else if first != 0 && last == 0 {
            le = true;
            break;
        }
        offset += wchar;
    }
    match (wchar, le) {
        (2, true) => b"UTF-16LE",
        (2, false) => b"UTF-16BE",
        (_, true) => b"UTF-32LE",
        _ => b"UTF-32BE",
    }
}
