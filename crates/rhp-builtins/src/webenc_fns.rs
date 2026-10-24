//! Native URL + HTML encoding builtins (pure, non-callback, non-by-reference).
//!
//! Every function here is verified byte-for-byte against the local php 8.5.8
//! binary (see the `#[cfg(test)]` block — expected values are hardcoded from
//! `php -r '...'` output). Strings are byte strings; the HTML functions use the
//! php 8.x default charset UTF-8 for numeric/named entity output.
//!
//! Wire this module into `lib.rs` with:
//! ```ignore
//! mod webenc_fns;              // near the other `mod` lines
//! // ...and inside registry(), before returning `m`:
//! webenc_fns::register(&mut m);
//! ```
//!
//! NOTE (php 8.1 default-flags change): since php 8.1, `htmlspecialchars`,
//! `htmlentities`, `htmlspecialchars_decode` and `html_entity_decode` default
//! to `ENT_QUOTES | ENT_SUBSTITUTE | ENT_HTML401` — i.e. BOTH quote styles are
//! escaped by default (single quote `'` -> `&#039;`). Before 8.1 the default
//! was `ENT_COMPAT` (double quote only). This module hardcodes the 8.1+ default
//! since rhp targets php 8.5; the optional `$flags` argument is not honored.

use std::collections::HashMap;

use rhp_values::{conv, Value};

use crate::{Builtin, Host, Thrown};

// ---------------------------------------------------------------------------
// small shared helpers
// ---------------------------------------------------------------------------

/// Deref a (possibly `Ref`) value to an owned value.
fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

/// `(string)` bytes of a value (after deref).
fn bytes_of(v: &Value) -> Vec<u8> {
    conv::to_string_bytes(&deref(v))
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

/// Parse one ascii hex digit.
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// C `isspace`: space, \t, \n, \v, \f, \r.
fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// Encode a unicode scalar as UTF-8 bytes (appended to `out`).
fn push_utf8(out: &mut Vec<u8>, cp: u32) {
    if cp < 0x80 {
        out.push(cp as u8);
    } else if cp < 0x800 {
        out.push(0xc0 | (cp >> 6) as u8);
        out.push(0x80 | (cp & 0x3f) as u8);
    } else if cp < 0x10000 {
        out.push(0xe0 | (cp >> 12) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3f) as u8);
        out.push(0x80 | (cp & 0x3f) as u8);
    } else {
        out.push(0xf0 | (cp >> 18) as u8);
        out.push(0x80 | ((cp >> 12) & 0x3f) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3f) as u8);
        out.push(0x80 | (cp & 0x3f) as u8);
    }
}

// ---------------------------------------------------------------------------
// urlencode / rawurlencode / urldecode / rawurldecode
// ---------------------------------------------------------------------------

fn fn_urlencode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    let mut out = Vec::with_capacity(s.len());
    for &b in &s {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(b),
            b' ' => out.push(b'+'),
            _ => {
                out.push(b'%');
                out.push(HEX_UPPER[(b >> 4) as usize]);
                out.push(HEX_UPPER[(b & 0xf) as usize]);
            }
        }
    }
    Ok(Value::str(out))
}

fn fn_rawurlencode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    let mut out = Vec::with_capacity(s.len());
    for &b in &s {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b),
            _ => {
                out.push(b'%');
                out.push(HEX_UPPER[(b >> 4) as usize]);
                out.push(HEX_UPPER[(b & 0xf) as usize]);
            }
        }
    }
    Ok(Value::str(out))
}

/// Shared percent-decoder. `plus_to_space` distinguishes urldecode / rawurldecode.
fn url_decode(s: &[u8], plus_to_space: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        if b == b'%' && i + 2 < s.len() {
            if let (Some(hi), Some(lo)) = (hex_val(s[i + 1]), hex_val(s[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
            out.push(b'%');
            i += 1;
        } else if b == b'+' && plus_to_space {
            out.push(b' ');
            i += 1;
        } else {
            out.push(b);
            i += 1;
        }
    }
    out
}

fn fn_urldecode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(url_decode(&bytes_of(&args[0]), true)))
}

fn fn_rawurldecode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(url_decode(&bytes_of(&args[0]), false)))
}

// ---------------------------------------------------------------------------
// htmlspecialchars / htmlentities (ASCII: identical) and their decoders
// ---------------------------------------------------------------------------

/// Escape the 5 special chars (php 8.1+ default ENT_QUOTES | ENT_HTML401).
fn html_escape(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &b in s {
        match b {
            b'&' => out.extend_from_slice(b"&amp;"),
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'"' => out.extend_from_slice(b"&quot;"),
            b'\'' => out.extend_from_slice(b"&#039;"),
            o => out.push(o),
        }
    }
    out
}

fn fn_htmlspecialchars(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(html_escape(&bytes_of(&args[0]))))
}

/// `htmlspecialchars_decode`: reverses ONLY the htmlspecialchars entities.
/// Default ENT_QUOTES|ENT_HTML401 → `&amp; &lt; &gt; &quot; &#039; &#39;`
/// (note: `&apos;` is NOT decoded — it is an HTML5 entity).
fn fn_htmlspecialchars_decode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    // longest-match-first, single left-to-right pass (no re-scan of output).
    const PAIRS: &[(&[u8], u8)] = &[
        (b"&amp;", b'&'),
        (b"&lt;", b'<'),
        (b"&gt;", b'>'),
        (b"&quot;", b'"'),
        (b"&#039;", b'\''),
        (b"&#39;", b'\''),
    ];
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    'outer: while i < s.len() {
        if s[i] == b'&' {
            for (pat, rep) in PAIRS {
                if s[i..].starts_with(pat) {
                    out.push(*rep);
                    i += pat.len();
                    continue 'outer;
                }
            }
        }
        out.push(s[i]);
        i += 1;
    }
    Ok(Value::str(out))
}

/// Reverse of [`NAMED_ENTITIES`]: UTF-8 byte sequence → canonical HTML 4.01
/// entity name. First-wins over the decode table, so the 5 base entities
/// (`quot`/`amp`/`lt`/`gt`) take their single-byte code points.
fn entity_rev_map() -> &'static std::collections::HashMap<&'static [u8], &'static [u8]> {
    use std::sync::OnceLock;
    static MAP: OnceLock<std::collections::HashMap<&'static [u8], &'static [u8]>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = std::collections::HashMap::new();
        for (name, bytes) in NAMED_ENTITIES {
            m.entry(*bytes).or_insert(*name);
        }
        m
    })
}

/// Encode one run of valid UTF-8, replacing each code point that has an HTML 4.01
/// named entity with `&name;`. The single quote has no HTML 4.01 named entity, so
/// under the php 8.1+ default (ENT_QUOTES) it becomes `&#039;`.
fn entity_encode_str(s: &str, out: &mut Vec<u8>) {
    let map = entity_rev_map();
    let mut buf = [0u8; 4];
    for ch in s.chars() {
        if ch == '\'' {
            out.extend_from_slice(b"&#039;");
            continue;
        }
        let bytes = ch.encode_utf8(&mut buf).as_bytes();
        if let Some(name) = map.get(bytes) {
            out.push(b'&');
            out.extend_from_slice(name);
            out.push(b';');
        } else {
            out.extend_from_slice(bytes);
        }
    }
}

/// `htmlentities`: like `htmlspecialchars` but every code point with an HTML 4.01
/// named entity is converted (`é` → `&eacute;`, `€` → `&euro;`, …), not just the
/// 5 base characters. Input is treated as UTF-8 (the php 8.x default charset);
/// malformed byte sequences are substituted with U+FFFD, matching the php 8.1+
/// default `ENT_SUBSTITUTE`. The full HTML 4.01 named-entity set is covered via
/// the shared [`NAMED_ENTITIES`] table (not just the Latin-1 range).
fn fn_htmlentities(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let input = bytes_of(&args[0]);
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        match std::str::from_utf8(&input[i..]) {
            Ok(valid) => {
                entity_encode_str(valid, &mut out);
                break;
            }
            Err(e) => {
                let good = e.valid_up_to();
                if good > 0 {
                    // SAFETY: bytes 0..good validated by from_utf8 above.
                    entity_encode_str(std::str::from_utf8(&input[i..i + good]).unwrap(), &mut out);
                    i += good;
                }
                out.extend_from_slice(&[0xEF, 0xBF, 0xBD]); // U+FFFD
                match e.error_len() {
                    Some(n) => i += n,
                    None => break, // truncated sequence at end of input
                }
            }
        }
    }
    Ok(Value::str(out))
}

/// Is a unicode scalar permitted as a numeric entity target (UTF-8, HTML401)?
/// Matches php's `unicode_cp_is_allowed`: tab/LF/CR, 0x20..0x7E, and
/// 0xA0..=0x10FFFF excluding the UTF-16 surrogate range.
fn cp_allowed(cp: u32) -> bool {
    matches!(cp, 0x09 | 0x0a | 0x0d)
        || (0x20..=0x7e).contains(&cp)
        || (0xa0..=0x10ffff).contains(&cp) && !(0xd800..=0xdfff).contains(&cp)
}

fn fn_html_entity_decode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] != b'&' {
            out.push(s[i]);
            i += 1;
            continue;
        }
        // Try numeric: &#DDD; or &#xHH; / &#XHH; (semicolon required).
        if i + 1 < s.len() && s[i + 1] == b'#' {
            if let Some((cp, consumed)) = parse_numeric_entity(&s[i..]) {
                if cp_allowed(cp) {
                    push_utf8(&mut out, cp);
                    i += consumed;
                    continue;
                }
            }
            out.push(b'&');
            i += 1;
            continue;
        }
        // Try a named entity (semicolon required). Longest match wins.
        if let Some((bytes, consumed)) = match_named_entity(&s[i..]) {
            out.extend_from_slice(bytes);
            i += consumed;
            continue;
        }
        out.push(b'&');
        i += 1;
    }
    Ok(Value::str(out))
}

/// Parse `&#...;` at the start of `s`; returns (codepoint, bytes_consumed).
fn parse_numeric_entity(s: &[u8]) -> Option<(u32, usize)> {
    // s[0] == '&', s[1] == '#'
    let mut j = 2;
    let hex = j < s.len() && (s[j] == b'x' || s[j] == b'X');
    if hex {
        j += 1;
    }
    let digits_start = j;
    let mut cp: u64 = 0;
    while j < s.len() {
        let d = if hex { hex_val(s[j]) } else {
            if s[j].is_ascii_digit() { Some(s[j] - b'0') } else { None }
        };
        match d {
            Some(v) => {
                cp = cp.saturating_mul(if hex { 16 } else { 10 }).saturating_add(v as u64);
                j += 1;
            }
            None => break,
        }
    }
    if j == digits_start || j >= s.len() || s[j] != b';' {
        return None; // no digits, or missing terminating ';'
    }
    Some((cp.min(0x7fff_ffff) as u32, j + 1))
}

/// Match a named entity `&name;` at the start of `s` (longest name wins).
fn match_named_entity(s: &[u8]) -> Option<(&'static [u8], usize)> {
    let mut best: Option<(&'static [u8], usize)> = None;
    for (name, bytes) in NAMED_ENTITIES {
        let full = name.len() + 2; // '&' + name + ';'
        if s.len() >= full && s[0] == b'&' && &s[1..1 + name.len()] == *name && s[full - 1] == b';' {
            match best {
                Some((_, blen)) if blen >= full => {}
                _ => best = Some((*bytes, full)),
            }
        }
    }
    best
}

// ---------------------------------------------------------------------------
// nl2br
// ---------------------------------------------------------------------------

fn fn_nl2br(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    let is_xhtml = args.get(1).map(|v| conv::to_bool(&deref(v))).unwrap_or(true);
    let br: &[u8] = if is_xhtml { b"<br />" } else { b"<br>" };
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'\r' | b'\n' => {
                out.extend_from_slice(br);
                // consume a two-char newline pair (\r\n or \n\r) as one unit.
                if i + 1 < s.len() && (s[i + 1] == b'\r' || s[i + 1] == b'\n') && s[i + 1] != s[i] {
                    out.push(s[i]);
                    out.push(s[i + 1]);
                    i += 2;
                } else {
                    out.push(s[i]);
                    i += 1;
                }
            }
            o => {
                out.push(o);
                i += 1;
            }
        }
    }
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// addslashes / stripslashes / quotemeta
// ---------------------------------------------------------------------------

fn fn_addslashes(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    let mut out = Vec::with_capacity(s.len());
    for &b in &s {
        match b {
            b'\'' | b'"' | b'\\' => {
                out.push(b'\\');
                out.push(b);
            }
            0 => {
                out.push(b'\\');
                out.push(b'0');
            }
            o => out.push(o),
        }
    }
    Ok(Value::str(out))
}

fn fn_stripslashes(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\\' {
            if i + 1 >= s.len() {
                // trailing backslash is dropped
                break;
            }
            let n = s[i + 1];
            if n == b'0' {
                out.push(0);
            } else {
                out.push(n);
            }
            i += 2;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    Ok(Value::str(out))
}

fn fn_quotemeta(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    let mut out = Vec::with_capacity(s.len());
    for &b in &s {
        if matches!(
            b,
            b'.' | b'\\' | b'+' | b'*' | b'?' | b'[' | b'^' | b']' | b'$' | b'(' | b')'
        ) {
            out.push(b'\\');
        }
        out.push(b);
    }
    Ok(Value::str(out))
}

// ---------------------------------------------------------------------------
// strip_tags (basic — no allowed-tags list)
// ---------------------------------------------------------------------------

fn fn_strip_tags(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(strip_tags(&bytes_of(&args[0]))))
}

#[derive(PartialEq)]
enum St {
    Text,
    Tag,       // inside <...>
    Comment,   // inside <!-- ... -->
    Decl,      // inside <! ... > (declaration, not a comment)
    Php,       // inside <? ... ?>
}

fn strip_tags(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut state = St::Text;
    let mut depth = 0usize; // nested '<' within a Tag
    let mut quote = 0u8; // 0 = not in quote, else the active quote byte
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        match state {
            St::Text => {
                if c == b'<' {
                    let nxt = s.get(i + 1).copied().unwrap_or(0);
                    if is_space(nxt) {
                        out.push(c); // '<' followed by whitespace is literal
                    } else if nxt == b'!' {
                        if s.get(i + 2) == Some(&b'-') && s.get(i + 3) == Some(&b'-') {
                            state = St::Comment;
                            i += 4;
                            continue;
                        }
                        state = St::Decl;
                        i += 2;
                        continue;
                    } else if nxt == b'?' {
                        state = St::Php;
                        i += 2;
                        continue;
                    } else {
                        state = St::Tag;
                        depth = 0;
                        quote = 0;
                    }
                } else {
                    out.push(c);
                }
            }
            St::Tag => {
                if quote != 0 {
                    if c == quote {
                        quote = 0;
                    }
                } else if c == b'"' || c == b'\'' {
                    quote = c;
                } else if c == b'<' {
                    depth += 1;
                } else if c == b'>' {
                    if depth > 0 {
                        depth -= 1;
                    } else {
                        state = St::Text;
                    }
                }
            }
            St::Comment => {
                if c == b'-'
                    && s.get(i + 1) == Some(&b'-')
                    && s.get(i + 2) == Some(&b'>')
                {
                    state = St::Text;
                    i += 3;
                    continue;
                }
            }
            St::Decl => {
                if c == b'>' {
                    state = St::Text;
                }
            }
            St::Php => {
                if c == b'?' && s.get(i + 1) == Some(&b'>') {
                    state = St::Text;
                    i += 2;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register all URL/HTML encoding builtins into the shared table.
pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
    put(m, "urlencode", fn_urlencode, 1, Some(1));
    put(m, "urldecode", fn_urldecode, 1, Some(1));
    put(m, "rawurlencode", fn_rawurlencode, 1, Some(1));
    put(m, "rawurldecode", fn_rawurldecode, 1, Some(1));
    // htmlspecialchars/htmlentities accept optional ($flags, $encoding,
    // $double_encode); only the default-flags single-arg form is honored.
    put(m, "htmlspecialchars", fn_htmlspecialchars, 1, Some(4));
    put(m, "htmlspecialchars_decode", fn_htmlspecialchars_decode, 1, Some(2));
    put(m, "htmlentities", fn_htmlentities, 1, Some(4));
    put(m, "html_entity_decode", fn_html_entity_decode, 1, Some(3));
    put(m, "nl2br", fn_nl2br, 1, Some(2));
    put(m, "addslashes", fn_addslashes, 1, Some(1));
    put(m, "stripslashes", fn_stripslashes, 1, Some(1));
    put(m, "quotemeta", fn_quotemeta, 1, Some(1));
    // strip_tags: basic (no allowed-tags list) — single-argument form only.
    put(m, "strip_tags", fn_strip_tags, 1, Some(1));
}

// ---------------------------------------------------------------------------
// HTML 4.01 named-entity table (ENT_QUOTES | ENT_HTML401, UTF-8 output).
// Generated from php: get_html_translation_table(HTML_ENTITIES, ...).
// Numeric `&#039;` is handled by the numeric path, not listed here.
// ---------------------------------------------------------------------------

#[rustfmt::skip]
const NAMED_ENTITIES: &[(&[u8], &[u8])] = &[
    (b"quot", &[0x22]),
    (b"amp", &[0x26]),
    (b"lt", &[0x3c]),
    (b"gt", &[0x3e]),
    (b"nbsp", &[0xc2, 0xa0]),
    (b"iexcl", &[0xc2, 0xa1]),
    (b"cent", &[0xc2, 0xa2]),
    (b"pound", &[0xc2, 0xa3]),
    (b"curren", &[0xc2, 0xa4]),
    (b"yen", &[0xc2, 0xa5]),
    (b"brvbar", &[0xc2, 0xa6]),
    (b"sect", &[0xc2, 0xa7]),
    (b"uml", &[0xc2, 0xa8]),
    (b"copy", &[0xc2, 0xa9]),
    (b"ordf", &[0xc2, 0xaa]),
    (b"laquo", &[0xc2, 0xab]),
    (b"not", &[0xc2, 0xac]),
    (b"shy", &[0xc2, 0xad]),
    (b"reg", &[0xc2, 0xae]),
    (b"macr", &[0xc2, 0xaf]),
    (b"deg", &[0xc2, 0xb0]),
    (b"plusmn", &[0xc2, 0xb1]),
    (b"sup2", &[0xc2, 0xb2]),
    (b"sup3", &[0xc2, 0xb3]),
    (b"acute", &[0xc2, 0xb4]),
    (b"micro", &[0xc2, 0xb5]),
    (b"para", &[0xc2, 0xb6]),
    (b"middot", &[0xc2, 0xb7]),
    (b"cedil", &[0xc2, 0xb8]),
    (b"sup1", &[0xc2, 0xb9]),
    (b"ordm", &[0xc2, 0xba]),
    (b"raquo", &[0xc2, 0xbb]),
    (b"frac14", &[0xc2, 0xbc]),
    (b"frac12", &[0xc2, 0xbd]),
    (b"frac34", &[0xc2, 0xbe]),
    (b"iquest", &[0xc2, 0xbf]),
    (b"Agrave", &[0xc3, 0x80]),
    (b"Aacute", &[0xc3, 0x81]),
    (b"Acirc", &[0xc3, 0x82]),
    (b"Atilde", &[0xc3, 0x83]),
    (b"Auml", &[0xc3, 0x84]),
    (b"Aring", &[0xc3, 0x85]),
    (b"AElig", &[0xc3, 0x86]),
    (b"Ccedil", &[0xc3, 0x87]),
    (b"Egrave", &[0xc3, 0x88]),
    (b"Eacute", &[0xc3, 0x89]),
    (b"Ecirc", &[0xc3, 0x8a]),
    (b"Euml", &[0xc3, 0x8b]),
    (b"Igrave", &[0xc3, 0x8c]),
    (b"Iacute", &[0xc3, 0x8d]),
    (b"Icirc", &[0xc3, 0x8e]),
    (b"Iuml", &[0xc3, 0x8f]),
    (b"ETH", &[0xc3, 0x90]),
    (b"Ntilde", &[0xc3, 0x91]),
    (b"Ograve", &[0xc3, 0x92]),
    (b"Oacute", &[0xc3, 0x93]),
    (b"Ocirc", &[0xc3, 0x94]),
    (b"Otilde", &[0xc3, 0x95]),
    (b"Ouml", &[0xc3, 0x96]),
    (b"times", &[0xc3, 0x97]),
    (b"Oslash", &[0xc3, 0x98]),
    (b"Ugrave", &[0xc3, 0x99]),
    (b"Uacute", &[0xc3, 0x9a]),
    (b"Ucirc", &[0xc3, 0x9b]),
    (b"Uuml", &[0xc3, 0x9c]),
    (b"Yacute", &[0xc3, 0x9d]),
    (b"THORN", &[0xc3, 0x9e]),
    (b"szlig", &[0xc3, 0x9f]),
    (b"agrave", &[0xc3, 0xa0]),
    (b"aacute", &[0xc3, 0xa1]),
    (b"acirc", &[0xc3, 0xa2]),
    (b"atilde", &[0xc3, 0xa3]),
    (b"auml", &[0xc3, 0xa4]),
    (b"aring", &[0xc3, 0xa5]),
    (b"aelig", &[0xc3, 0xa6]),
    (b"ccedil", &[0xc3, 0xa7]),
    (b"egrave", &[0xc3, 0xa8]),
    (b"eacute", &[0xc3, 0xa9]),
    (b"ecirc", &[0xc3, 0xaa]),
    (b"euml", &[0xc3, 0xab]),
    (b"igrave", &[0xc3, 0xac]),
    (b"iacute", &[0xc3, 0xad]),
    (b"icirc", &[0xc3, 0xae]),
    (b"iuml", &[0xc3, 0xaf]),
    (b"eth", &[0xc3, 0xb0]),
    (b"ntilde", &[0xc3, 0xb1]),
    (b"ograve", &[0xc3, 0xb2]),
    (b"oacute", &[0xc3, 0xb3]),
    (b"ocirc", &[0xc3, 0xb4]),
    (b"otilde", &[0xc3, 0xb5]),
    (b"ouml", &[0xc3, 0xb6]),
    (b"divide", &[0xc3, 0xb7]),
    (b"oslash", &[0xc3, 0xb8]),
    (b"ugrave", &[0xc3, 0xb9]),
    (b"uacute", &[0xc3, 0xba]),
    (b"ucirc", &[0xc3, 0xbb]),
    (b"uuml", &[0xc3, 0xbc]),
    (b"yacute", &[0xc3, 0xbd]),
    (b"thorn", &[0xc3, 0xbe]),
    (b"yuml", &[0xc3, 0xbf]),
    (b"OElig", &[0xc5, 0x92]),
    (b"oelig", &[0xc5, 0x93]),
    (b"Scaron", &[0xc5, 0xa0]),
    (b"scaron", &[0xc5, 0xa1]),
    (b"Yuml", &[0xc5, 0xb8]),
    (b"fnof", &[0xc6, 0x92]),
    (b"circ", &[0xcb, 0x86]),
    (b"tilde", &[0xcb, 0x9c]),
    (b"Alpha", &[0xce, 0x91]),
    (b"Beta", &[0xce, 0x92]),
    (b"Gamma", &[0xce, 0x93]),
    (b"Delta", &[0xce, 0x94]),
    (b"Epsilon", &[0xce, 0x95]),
    (b"Zeta", &[0xce, 0x96]),
    (b"Eta", &[0xce, 0x97]),
    (b"Theta", &[0xce, 0x98]),
    (b"Iota", &[0xce, 0x99]),
    (b"Kappa", &[0xce, 0x9a]),
    (b"Lambda", &[0xce, 0x9b]),
    (b"Mu", &[0xce, 0x9c]),
    (b"Nu", &[0xce, 0x9d]),
    (b"Xi", &[0xce, 0x9e]),
    (b"Omicron", &[0xce, 0x9f]),
    (b"Pi", &[0xce, 0xa0]),
    (b"Rho", &[0xce, 0xa1]),
    (b"Sigma", &[0xce, 0xa3]),
    (b"Tau", &[0xce, 0xa4]),
    (b"Upsilon", &[0xce, 0xa5]),
    (b"Phi", &[0xce, 0xa6]),
    (b"Chi", &[0xce, 0xa7]),
    (b"Psi", &[0xce, 0xa8]),
    (b"Omega", &[0xce, 0xa9]),
    (b"alpha", &[0xce, 0xb1]),
    (b"beta", &[0xce, 0xb2]),
    (b"gamma", &[0xce, 0xb3]),
    (b"delta", &[0xce, 0xb4]),
    (b"epsilon", &[0xce, 0xb5]),
    (b"zeta", &[0xce, 0xb6]),
    (b"eta", &[0xce, 0xb7]),
    (b"theta", &[0xce, 0xb8]),
    (b"iota", &[0xce, 0xb9]),
    (b"kappa", &[0xce, 0xba]),
    (b"lambda", &[0xce, 0xbb]),
    (b"mu", &[0xce, 0xbc]),
    (b"nu", &[0xce, 0xbd]),
    (b"xi", &[0xce, 0xbe]),
    (b"omicron", &[0xce, 0xbf]),
    (b"pi", &[0xcf, 0x80]),
    (b"rho", &[0xcf, 0x81]),
    (b"sigmaf", &[0xcf, 0x82]),
    (b"sigma", &[0xcf, 0x83]),
    (b"tau", &[0xcf, 0x84]),
    (b"upsilon", &[0xcf, 0x85]),
    (b"phi", &[0xcf, 0x86]),
    (b"chi", &[0xcf, 0x87]),
    (b"psi", &[0xcf, 0x88]),
    (b"omega", &[0xcf, 0x89]),
    (b"thetasym", &[0xcf, 0x91]),
    (b"upsih", &[0xcf, 0x92]),
    (b"piv", &[0xcf, 0x96]),
    (b"ensp", &[0xe2, 0x80, 0x82]),
    (b"emsp", &[0xe2, 0x80, 0x83]),
    (b"thinsp", &[0xe2, 0x80, 0x89]),
    (b"zwnj", &[0xe2, 0x80, 0x8c]),
    (b"zwj", &[0xe2, 0x80, 0x8d]),
    (b"lrm", &[0xe2, 0x80, 0x8e]),
    (b"rlm", &[0xe2, 0x80, 0x8f]),
    (b"ndash", &[0xe2, 0x80, 0x93]),
    (b"mdash", &[0xe2, 0x80, 0x94]),
    (b"lsquo", &[0xe2, 0x80, 0x98]),
    (b"rsquo", &[0xe2, 0x80, 0x99]),
    (b"sbquo", &[0xe2, 0x80, 0x9a]),
    (b"ldquo", &[0xe2, 0x80, 0x9c]),
    (b"rdquo", &[0xe2, 0x80, 0x9d]),
    (b"bdquo", &[0xe2, 0x80, 0x9e]),
    (b"dagger", &[0xe2, 0x80, 0xa0]),
    (b"Dagger", &[0xe2, 0x80, 0xa1]),
    (b"bull", &[0xe2, 0x80, 0xa2]),
    (b"hellip", &[0xe2, 0x80, 0xa6]),
    (b"permil", &[0xe2, 0x80, 0xb0]),
    (b"prime", &[0xe2, 0x80, 0xb2]),
    (b"Prime", &[0xe2, 0x80, 0xb3]),
    (b"lsaquo", &[0xe2, 0x80, 0xb9]),
    (b"rsaquo", &[0xe2, 0x80, 0xba]),
    (b"oline", &[0xe2, 0x80, 0xbe]),
    (b"frasl", &[0xe2, 0x81, 0x84]),
    (b"euro", &[0xe2, 0x82, 0xac]),
    (b"image", &[0xe2, 0x84, 0x91]),
    (b"weierp", &[0xe2, 0x84, 0x98]),
    (b"real", &[0xe2, 0x84, 0x9c]),
    (b"trade", &[0xe2, 0x84, 0xa2]),
    (b"alefsym", &[0xe2, 0x84, 0xb5]),
    (b"larr", &[0xe2, 0x86, 0x90]),
    (b"uarr", &[0xe2, 0x86, 0x91]),
    (b"rarr", &[0xe2, 0x86, 0x92]),
    (b"darr", &[0xe2, 0x86, 0x93]),
    (b"harr", &[0xe2, 0x86, 0x94]),
    (b"crarr", &[0xe2, 0x86, 0xb5]),
    (b"lArr", &[0xe2, 0x87, 0x90]),
    (b"uArr", &[0xe2, 0x87, 0x91]),
    (b"rArr", &[0xe2, 0x87, 0x92]),
    (b"dArr", &[0xe2, 0x87, 0x93]),
    (b"hArr", &[0xe2, 0x87, 0x94]),
    (b"forall", &[0xe2, 0x88, 0x80]),
    (b"part", &[0xe2, 0x88, 0x82]),
    (b"exist", &[0xe2, 0x88, 0x83]),
    (b"empty", &[0xe2, 0x88, 0x85]),
    (b"nabla", &[0xe2, 0x88, 0x87]),
    (b"isin", &[0xe2, 0x88, 0x88]),
    (b"notin", &[0xe2, 0x88, 0x89]),
    (b"ni", &[0xe2, 0x88, 0x8b]),
    (b"prod", &[0xe2, 0x88, 0x8f]),
    (b"sum", &[0xe2, 0x88, 0x91]),
    (b"minus", &[0xe2, 0x88, 0x92]),
    (b"lowast", &[0xe2, 0x88, 0x97]),
    (b"radic", &[0xe2, 0x88, 0x9a]),
    (b"prop", &[0xe2, 0x88, 0x9d]),
    (b"infin", &[0xe2, 0x88, 0x9e]),
    (b"ang", &[0xe2, 0x88, 0xa0]),
    (b"and", &[0xe2, 0x88, 0xa7]),
    (b"or", &[0xe2, 0x88, 0xa8]),
    (b"cap", &[0xe2, 0x88, 0xa9]),
    (b"cup", &[0xe2, 0x88, 0xaa]),
    (b"int", &[0xe2, 0x88, 0xab]),
    (b"there4", &[0xe2, 0x88, 0xb4]),
    (b"sim", &[0xe2, 0x88, 0xbc]),
    (b"cong", &[0xe2, 0x89, 0x85]),
    (b"asymp", &[0xe2, 0x89, 0x88]),
    (b"ne", &[0xe2, 0x89, 0xa0]),
    (b"equiv", &[0xe2, 0x89, 0xa1]),
    (b"le", &[0xe2, 0x89, 0xa4]),
    (b"ge", &[0xe2, 0x89, 0xa5]),
    (b"sub", &[0xe2, 0x8a, 0x82]),
    (b"sup", &[0xe2, 0x8a, 0x83]),
    (b"nsub", &[0xe2, 0x8a, 0x84]),
    (b"sube", &[0xe2, 0x8a, 0x86]),
    (b"supe", &[0xe2, 0x8a, 0x87]),
    (b"oplus", &[0xe2, 0x8a, 0x95]),
    (b"otimes", &[0xe2, 0x8a, 0x97]),
    (b"perp", &[0xe2, 0x8a, 0xa5]),
    (b"sdot", &[0xe2, 0x8b, 0x85]),
    (b"lceil", &[0xe2, 0x8c, 0x88]),
    (b"rceil", &[0xe2, 0x8c, 0x89]),
    (b"lfloor", &[0xe2, 0x8c, 0x8a]),
    (b"rfloor", &[0xe2, 0x8c, 0x8b]),
    (b"lang", &[0xe2, 0x8c, 0xa9]),
    (b"rang", &[0xe2, 0x8c, 0xaa]),
    (b"loz", &[0xe2, 0x97, 0x8a]),
    (b"spades", &[0xe2, 0x99, 0xa0]),
    (b"clubs", &[0xe2, 0x99, 0xa3]),
    (b"hearts", &[0xe2, 0x99, 0xa5]),
    (b"diams", &[0xe2, 0x99, 0xa6]),
];

// ---------------------------------------------------------------------------
// tests — expected values captured from php 8.5.8
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct NullHost;
    impl Host for NullHost {
        fn write_out(&mut self, _: &[u8]) {}
    }

    fn s(b: &str) -> Value {
        Value::str(b)
    }
    fn sb(b: &[u8]) -> Value {
        Value::str(b.to_vec())
    }
    fn call(f: crate::NativeFn, args: &[Value]) -> Vec<u8> {
        match f(&mut NullHost, args).expect("no throw") {
            Value::Str(b) => b.as_bytes().to_vec(),
            other => panic!("expected string, got {}", other.type_name()),
        }
    }
    fn calls(f: crate::NativeFn, args: &[Value]) -> String {
        String::from_utf8(call(f, args)).unwrap()
    }

    #[test]
    fn urlencode_cases() {
        assert_eq!(calls(fn_urlencode, &[s("a b+c&d=e/f?g~h.i_j-k")]), "a+b%2Bc%26d%3De%2Ff%3Fg%7Eh.i_j-k");
        assert_eq!(call(fn_urlencode, &[sb(b"\xff\x00A~")]), b"%FF%00A%7E");
    }

    #[test]
    fn rawurlencode_cases() {
        assert_eq!(calls(fn_rawurlencode, &[s("a b+c&d=e/f?g~h.i_j-k")]), "a%20b%2Bc%26d%3De%2Ff%3Fg~h.i_j-k");
        assert_eq!(call(fn_rawurlencode, &[sb(b"\xff\x00A~")]), b"%FF%00A~");
    }

    #[test]
    fn urldecode_cases() {
        assert_eq!(calls(fn_urldecode, &[s("a+b%20c%26d%ZZ%4")]), "a b c&d%ZZ%4");
        assert_eq!(calls(fn_urldecode, &[s("abc%")]), "abc%");
        assert_eq!(calls(fn_rawurldecode, &[s("a+b%20c%26d")]), "a+b c&d");
        assert_eq!(calls(fn_rawurldecode, &[s("ab%2")]), "ab%2");
    }

    #[test]
    fn htmlspecialchars_cases() {
        assert_eq!(
            calls(fn_htmlspecialchars, &[s("Tom & Jerry <> \"q\" 'a'")]),
            "Tom &amp; Jerry &lt;&gt; &quot;q&quot; &#039;a&#039;"
        );
        assert_eq!(calls(fn_htmlspecialchars, &[s("'")]), "&#039;");
        assert_eq!(
            calls(fn_htmlentities, &[s("<a> & \"q\" 'a'")]),
            "&lt;a&gt; &amp; &quot;q&quot; &#039;a&#039;"
        );
    }

    #[test]
    fn htmlspecialchars_decode_cases() {
        assert_eq!(
            calls(
                fn_htmlspecialchars_decode,
                &[s("&lt;a&gt; &amp; &quot;q&quot; &#039;a&#039; &#39;b&#39; &apos;c&apos;")]
            ),
            "<a> & \"q\" 'a' 'b' &apos;c&apos;"
        );
        // once &amp; -> & the following "lt;" is NOT re-scanned
        assert_eq!(calls(fn_htmlspecialchars_decode, &[s("&amp;lt;")]), "&lt;");
    }

    #[test]
    fn html_entity_decode_cases() {
        assert_eq!(
            call(fn_html_entity_decode, &[s("&lt;a&gt;&amp;&quot;&#039;&#39;&apos;&copy;&nbsp;&#65;&#x41;")]),
            // < a > & " ' '  &apos; © nbsp A A   (copy=c2a9, nbsp=c2a0)
            b"<a>&\"''&apos;\xc2\xa9\xc2\xa0AA"
        );
        // numeric decimal + hex (incl. astral plane)
        assert_eq!(call(fn_html_entity_decode, &[s("&#65;&#128512;")]), b"A\xf0\x9f\x98\x80");
        assert_eq!(call(fn_html_entity_decode, &[s("&#x41;&#X1F600;")]), b"A\xf0\x9f\x98\x80");
        // semicolon required
        assert_eq!(calls(fn_html_entity_decode, &[s("&copy x &amp y")]), "&copy x &amp y");
        assert_eq!(calls(fn_html_entity_decode, &[s("&#65 &#65;")]), "&#65 A");
        // unknown entity stays; &amp;lt; not re-scanned
        assert_eq!(calls(fn_html_entity_decode, &[s("&foobar; &amp;")]), "&foobar; &");
        assert_eq!(calls(fn_html_entity_decode, &[s("&amp;lt;")]), "&lt;");
        // forbidden codepoints are left literal
        assert_eq!(calls(fn_html_entity_decode, &[s("&#0;&#127;&#128;")]), "&#0;&#127;&#128;");
        assert_eq!(call(fn_html_entity_decode, &[s("&#9;")]), b"\t");
        assert_eq!(calls(fn_html_entity_decode, &[s("&#1114112;&#xD800;")]), "&#1114112;&#xD800;");
        assert_eq!(call(fn_html_entity_decode, &[s("&#65535;")]), b"\xef\xbf\xbf");
    }

    #[test]
    fn nl2br_cases() {
        assert_eq!(
            call(fn_nl2br, &[sb(b"a\nb\r\nc\rd\n\re")]),
            b"a<br />\nb<br />\r\nc<br />\rd<br />\n\re"
        );
        assert_eq!(call(fn_nl2br, &[sb(b"a\nb"), Value::Bool(false)]), b"a<br>\nb");
    }

    #[test]
    fn addslashes_cases() {
        assert_eq!(call(fn_addslashes, &[sb(b"a'b\"c\\d\x00e")]), b"a\\'b\\\"c\\\\d\\0e");
    }

    #[test]
    fn stripslashes_cases() {
        assert_eq!(call(fn_stripslashes, &[sb(b"a\\'b\\\"c\\\\d\\0e\\n")]), b"a'b\"c\\d\x00en");
        assert_eq!(call(fn_stripslashes, &[sb(b"\\0")]), b"\x00");
        assert_eq!(call(fn_stripslashes, &[sb(b"ab\\")]), b"ab");
    }

    #[test]
    fn quotemeta_cases() {
        assert_eq!(
            calls(fn_quotemeta, &[s("1+1=2. (a|b) [x] *? \\^$ end")]),
            "1\\+1=2\\. \\(a|b\\) \\[x\\] \\*\\? \\\\\\^\\$ end"
        );
    }

    #[test]
    fn differential_sweep() {
        // extra inputs cross-checked against php 8.5.8 (hex of output)
        fn hex(b: &[u8]) -> String {
            b.iter().map(|x| format!("{:02x}", x)).collect()
        }
        let cases = [
            "<b>a</b> & 'x' <!--c--> <?p ?> <d e=\">\">f",
            "A&amp;B&#38;C&#x26;D",
            "café &eacute; &#233;",
            "<!--a--b-->z",
            "<a b=1>c",
        ];
        let php_strip = [
            "6120262027782720202066",
            "4126616d703b42262333383b4326237832363b44",
            "636166c3a920266561637574653b2026233233333b",
            "7a",
            "63",
        ];
        let php_hed = [
            "3c623e613c2f623e202620277827203c212d2d632d2d3e203c3f70203f3e203c6420653d223e223e66",
            "41264226432644",
            "636166c3a920c3a920c3a9",
            "3c212d2d612d2d622d2d3e7a",
            "3c6120623d313e63",
        ];
        for i in 0..cases.len() {
            assert_eq!(hex(&call(fn_strip_tags, &[s(cases[i])])), php_strip[i], "strip_tags case {i}");
            assert_eq!(hex(&call(fn_html_entity_decode, &[s(cases[i])])), php_hed[i], "html_entity_decode case {i}");
        }
    }

    #[test]
    fn strip_tags_cases() {
        let f = fn_strip_tags;
        assert_eq!(calls(f, &[s("<p>Hello <b>World</b></p>")]), "Hello World");
        assert_eq!(calls(f, &[s("<a href=\"x\">link</a> text")]), "link text");
        assert_eq!(calls(f, &[s("a<!-- comment -->b")]), "ab");
        assert_eq!(calls(f, &[s("a < b and c > d")]), "a < b and c > d");
        assert_eq!(calls(f, &[s("text <b>bold")]), "text bold");
        assert_eq!(calls(f, &[s("<a title=\"<b>\">x</a>")]), "x");
        assert_eq!(calls(f, &[s("a<?php echo 1; ?>b")]), "ab");
        assert_eq!(calls(f, &[s("<script>alert(1)</script>hi")]), "alert(1)hi");
        assert_eq!(calls(f, &[s("a &lt;b&gt; c")]), "a &lt;b&gt; c");
        assert_eq!(calls(f, &[s("abc<")]), "abc");
        assert_eq!(calls(f, &[s("<div")]), "");
        // nested '<' depth, comment/decl/php end conditions
        assert_eq!(calls(f, &[s("<a<b>c>d")]), "d");
        assert_eq!(calls(f, &[s("a<!--x-->b<!--y--->c")]), "abc");
        assert_eq!(calls(f, &[s("a<!-- > -->b")]), "ab");
        assert_eq!(calls(f, &[s("a<!DOCTYPE html>b")]), "ab");
        assert_eq!(calls(f, &[s("a<? x > y ?>b")]), "ab");
        assert_eq!(calls(f, &[s("a<![CDATA[x<y>]]>b")]), "a]]>b");
        assert_eq!(calls(f, &[s("<input value=\">\">x")]), "x");
        assert_eq!(calls(f, &[s("<a t='>'>z")]), "z");
        assert_eq!(calls(f, &[s("a<?php x")]), "a");
        assert_eq!(calls(f, &[s("a<!-- x")]), "a");
        assert_eq!(call(f, &[sb(b"a<\x0cb>c")]), b"a<\x0cb>c");
    }
}
