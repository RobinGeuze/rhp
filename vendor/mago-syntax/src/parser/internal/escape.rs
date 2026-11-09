//! rhp: php's own escape decoding (zend_scan_escape_string) in place of
//! mago_syntax_core's `parse_literal_string_in`, which misses `\X` and
//! decodes `\"` in heredocs and backtick strings.

use mago_allocator::prelude::*;

/// Decodes a literal string token (`'..'`, `".."`, with an optional `b`
/// prefix) the way php's scanner does. `None` when the token is not a
/// quoted string or holds an invalid `\u{..}`.
pub(crate) fn literal_string<'arena, A: Arena>(arena: &'arena A, s: &'arena [u8]) -> Option<&'arena [u8]> {
    let s = match s {
        [b'b' | b'B', rest @ ..] if matches!(rest.first(), Some(b'"' | b'\'')) => rest,
        _ => s,
    };
    if s.len() < 2 {
        return None;
    }
    let content = &s[1..s.len() - 1];
    match (s[0], s[s.len() - 1]) {
        (b'"', b'"') => escape_string(arena, content, b'"'),
        (b'\'', b'\'') => Some(single_quoted(arena, content)),
        _ => None,
    }
}

/// A single-quoted string: only `\'` and `\\` are escapes.
fn single_quoted<'arena, A: Arena>(arena: &'arena A, content: &'arena [u8]) -> &'arena [u8] {
    if !content.contains(&b'\\') {
        return content;
    }
    let mut out = Vec::with_capacity_in(content.len(), arena);
    let mut i = 0;
    while i < content.len() {
        let c = content[i];
        if c == b'\\' && matches!(content.get(i + 1), Some(b'\\' | b'\'')) {
            out.push(content[i + 1]);
            i += 2;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out.leak()
}

/// zend_scan_escape_string: the escapes of a double-quoted string part.
/// `quote` is `"` for a double-quoted string, `` ` `` for a shell-exec
/// string and 0 for a heredoc: `\"` / `` \` `` decode only for their own
/// quote.
pub(crate) fn escape_string<'arena, A: Arena>(
    arena: &'arena A,
    content: &'arena [u8],
    quote: u8,
) -> Option<&'arena [u8]> {
    if !content.contains(&b'\\') {
        return Some(content);
    }
    let hex = |c: u8| (c as char).to_digit(16);
    let mut out = Vec::with_capacity_in(content.len(), arena);
    let mut i = 0;
    while i < content.len() {
        let c = content[i];
        if c != b'\\' {
            out.push(c);
            i += 1;
            continue;
        }
        let Some(&n) = content.get(i + 1) else {
            out.push(b'\\');
            break;
        };
        i += 2;
        match n {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'f' => out.push(0x0c),
            b'v' => out.push(0x0b),
            b'e' => out.push(0x1b),
            b'"' | b'`' if n != quote => {
                out.push(b'\\');
                out.push(n);
            }
            b'"' | b'`' | b'\\' | b'$' => out.push(n),
            b'x' | b'X' => match content.get(i).and_then(|&d| hex(d)) {
                Some(mut v) => {
                    i += 1;
                    if let Some(d) = content.get(i).and_then(|&d| hex(d)) {
                        v = v * 16 + d;
                        i += 1;
                    }
                    out.push(v as u8);
                }
                None => {
                    out.push(b'\\');
                    out.push(n);
                }
            },
            b'u' => {
                if content.get(i) != Some(&b'{') {
                    out.push(b'\\');
                    out.push(b'u');
                    continue;
                }
                let digits = content[i + 1..].iter().take_while(|&&d| hex(d).is_some()).count();
                if digits == 0 || content.get(i + 1 + digits) != Some(&b'}') {
                    return None;
                }
                let mut cp: u32 = 0;
                for &d in &content[i + 1..i + 1 + digits] {
                    cp = cp.checked_mul(16)?.checked_add(hex(d)?)?;
                }
                if cp > 0x10_FFFF {
                    return None;
                }
                // Surrogates too, as php writes them (3 bytes).
                if cp < 0x80 {
                    out.push(cp as u8);
                } else if cp <= 0x7FF {
                    out.push((cp >> 6) as u8 + 0xC0);
                    out.push((cp & 0x3F) as u8 + 0x80);
                } else if cp <= 0xFFFF {
                    out.push((cp >> 12) as u8 + 0xE0);
                    out.push(((cp >> 6) & 0x3F) as u8 + 0x80);
                    out.push((cp & 0x3F) as u8 + 0x80);
                } else {
                    out.push((cp >> 18) as u8 + 0xF0);
                    out.push(((cp >> 12) & 0x3F) as u8 + 0x80);
                    out.push(((cp >> 6) & 0x3F) as u8 + 0x80);
                    out.push((cp & 0x3F) as u8 + 0x80);
                }
                i += 1 + digits + 1;
            }
            b'0'..=b'7' => {
                let mut v = u32::from(n - b'0');
                let mut k = 0;
                while k < 2 && matches!(content.get(i), Some(b'0'..=b'7')) {
                    v = v * 8 + u32::from(content[i] - b'0');
                    i += 1;
                    k += 1;
                }
                out.push(v as u8);
            }
            _ => {
                out.push(b'\\');
                out.push(n);
            }
        }
    }
    Some(out.leak())
}
