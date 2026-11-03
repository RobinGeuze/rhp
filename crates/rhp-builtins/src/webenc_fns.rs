//! Native URL and string-escaping builtins (urlencode, nl2br, addslashes,
//! strip_tags, parse_url, ...). The HTML entity functions are in `html`.
//!
//! The unit tests' expected values were captured from php.

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
// nl2br
// ---------------------------------------------------------------------------

fn fn_nl2br(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = bytes_of(&args[0]);
    let is_xhtml = args
        .get(1)
        .map(|v| conv::to_bool(&deref(v)))
        .unwrap_or(true);
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
    Tag,     // inside <...>
    Comment, // inside <!-- ... -->
    Decl,    // inside <! ... > (declaration, not a comment)
    Php,     // inside <? ... ?>
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
                if c == b'-' && s.get(i + 1) == Some(&b'-') && s.get(i + 2) == Some(&b'>') {
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
// parse_url
// ---------------------------------------------------------------------------

#[derive(Default)]
struct UrlParts {
    scheme: Option<Vec<u8>>,
    host: Option<Vec<u8>>,
    port: Option<i64>,
    user: Option<Vec<u8>>,
    pass: Option<Vec<u8>>,
    path: Option<Vec<u8>>,
    query: Option<Vec<u8>>,
    fragment: Option<Vec<u8>>,
}

fn is_scheme_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'+' || b == b'.' || b == b'-'
}

/// Faithful-enough port of php's `php_url_parse_ex2`. Returns `None` on the
/// inputs php rejects (empty, a bare port like `:80`, or a port out of range).
fn do_parse_url(s: &[u8]) -> Option<UrlParts> {
    let ue = s.len();
    let mut out = UrlParts::default();
    let mut i = 0usize; // current cursor

    // --- scheme ---
    // The limit before any '?'/'#' (a '?'/'#' before ':' means no scheme).
    let qhash = s.iter().position(|&b| b == b'?' || b == b'#').unwrap_or(ue);
    if let Some(colon) = s.iter().position(|&b| b == b':') {
        if colon != 0 && colon < qhash {
            let scheme_ok =
                s[0].is_ascii_alphabetic() && s[..colon].iter().all(|&b| is_scheme_char(b));
            if scheme_ok {
                // Distinguish a real scheme from a leading "host:port".
                if colon + 1 < ue && s[colon + 1] == b'/' {
                    out.scheme = Some(s[..colon].to_vec());
                    // scheme://  or scheme:/
                    if colon + 2 < ue && s[colon + 2] == b'/' {
                        i = colon + 3;
                        // file:///path keeps the leading slash as path.
                        if out
                            .scheme
                            .as_deref()
                            .map(|sc| sc.eq_ignore_ascii_case(b"file"))
                            .unwrap_or(false)
                            && i < ue
                            && s[i] == b'/'
                        {
                            // file://<empty-host>/path -> just path from here.
                            return finish_path(out, s, i);
                        }
                        return parse_authority(out, s, i);
                    } else {
                        // scheme:/path
                        i = colon + 1;
                        return finish_path(out, s, i);
                    }
                } else {
                    // No '//' after ':'. Could be "scheme:path" (mailto:) or
                    // "host:port". If what follows the colon is all digits up to
                    // end or '/', treat the leading token as host:port.
                    let mut p = colon + 1;
                    while p < ue && s[p].is_ascii_digit() {
                        p += 1;
                    }
                    let only_digits_then_end = p > colon + 1 && (p == ue || s[p] == b'/');
                    if only_digits_then_end && (p - colon) < 7 {
                        // host:port form — fall through to authority parse from 0.
                        return parse_authority(out, s, 0);
                    }
                    out.scheme = Some(s[..colon].to_vec());
                    i = colon + 1;
                    return finish_path(out, s, i);
                }
            }
        } else if colon == 0 {
            // leading ':' with digits is a bare port -> php fails.
            let mut p = 1;
            while p < ue && s[p].is_ascii_digit() {
                p += 1;
            }
            if p > 1 && (p == ue || s[p] == b'/') {
                return None;
            }
        }
    }

    // No scheme. "//host..." is a relative-scheme URL (authority); else path.
    if ue >= 2 && s[0] == b'/' && s[1] == b'/' {
        return parse_authority(out, s, 2);
    }
    let _ = i;
    finish_path(out, s, 0)
}

/// Parse `[user[:pass]@]host[:port]` starting at `i`, up to the first `/?#`.
fn parse_authority(mut out: UrlParts, s: &[u8], i: usize) -> Option<UrlParts> {
    let ue = s.len();
    let end = s[i..]
        .iter()
        .position(|&b| b == b'/' || b == b'?' || b == b'#')
        .map(|p| i + p)
        .unwrap_or(ue);
    let auth = &s[i..end];

    let mut host_start = i;
    // userinfo
    if let Some(at_rel) = auth.iter().position(|&b| b == b'@') {
        let at = i + at_rel;
        let cred = &s[i..at];
        if let Some(c) = cred.iter().position(|&b| b == b':') {
            out.user = Some(cred[..c].to_vec());
            out.pass = Some(cred[c + 1..].to_vec());
        } else {
            out.user = Some(cred.to_vec());
        }
        host_start = at + 1;
    }

    // host (+ optional port). Support [ipv6].
    let host_region = &s[host_start..end];
    if host_region.first() == Some(&b'[') {
        // IPv6 literal: host is up to and including ']'.
        {
            let close_rel = host_region.iter().position(|&b| b == b']')?;
            out.host = Some(host_region[..=close_rel].to_vec());
            let after = &host_region[close_rel + 1..];
            if after.first() == Some(&b':') {
                let digits = &after[1..];
                if digits.iter().all(|b| b.is_ascii_digit()) && !digits.is_empty() {
                    out.port = parse_port(digits)?;
                }
            }
        }
    } else if let Some(c) = host_region.iter().rposition(|&b| b == b':') {
        let (h, pport) = (&host_region[..c], &host_region[c + 1..]);
        if pport.iter().all(|b| b.is_ascii_digit()) && !pport.is_empty() {
            out.port = parse_port(pport)?;
            if !h.is_empty() {
                out.host = Some(h.to_vec());
            }
        } else {
            // not a port; whole thing is the host
            if !host_region.is_empty() {
                out.host = Some(host_region.to_vec());
            }
        }
    } else if !host_region.is_empty() {
        out.host = Some(host_region.to_vec());
    }

    finish_path(out, s, end)
}

fn parse_port(digits: &[u8]) -> Option<Option<i64>> {
    let n: i64 = std::str::from_utf8(digits).ok()?.parse().ok()?;
    if n > 65535 {
        return None; // php rejects out-of-range ports
    }
    Some(Some(n))
}

/// Everything from `i` to end is path[?query][#fragment].
fn finish_path(mut out: UrlParts, s: &[u8], i: usize) -> Option<UrlParts> {
    let rest = &s[i..];
    // fragment (first '#')
    let (before_frag, frag) = match rest.iter().position(|&b| b == b'#') {
        Some(h) => (&rest[..h], Some(&rest[h + 1..])),
        None => (rest, None),
    };
    // query (first '?')
    let (path, query) = match before_frag.iter().position(|&b| b == b'?') {
        Some(q) => (&before_frag[..q], Some(&before_frag[q + 1..])),
        None => (before_frag, None),
    };
    if !path.is_empty() {
        out.path = Some(path.to_vec());
    }
    if let Some(q) = query {
        out.query = Some(q.to_vec());
    }
    if let Some(f) = frag {
        out.fragment = Some(f.to_vec());
    }
    Some(out)
}

/// php `parse_url($url, $component = -1)`.
fn fn_parse_url(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let url = bytes_of(&args[0]);
    let component = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(-1);

    let parts = match do_parse_url(&url) {
        Some(p) => p,
        None => return Ok(Value::Bool(false)),
    };

    // A specific component was requested.
    if component >= 0 {
        return Ok(match component {
            0 => parts.scheme.map(Value::str).unwrap_or(Value::Null),
            1 => parts.host.map(Value::str).unwrap_or(Value::Null),
            2 => parts.port.map(Value::Int).unwrap_or(Value::Null),
            3 => parts.user.map(Value::str).unwrap_or(Value::Null),
            4 => parts.pass.map(Value::str).unwrap_or(Value::Null),
            5 => parts.path.map(Value::str).unwrap_or(Value::Null),
            6 => parts.query.map(Value::str).unwrap_or(Value::Null),
            7 => parts.fragment.map(Value::str).unwrap_or(Value::Null),
            _ => Value::Null,
        });
    }

    let mut arr = rhp_values::PhpArray::new();
    use rhp_values::ArrayKey as K;
    if let Some(v) = parts.scheme {
        arr.insert(K::from_bytes(b"scheme"), Value::str(v));
    }
    if let Some(v) = parts.host {
        arr.insert(K::from_bytes(b"host"), Value::str(v));
    }
    if let Some(v) = parts.port {
        arr.insert(K::from_bytes(b"port"), Value::Int(v));
    }
    if let Some(v) = parts.user {
        arr.insert(K::from_bytes(b"user"), Value::str(v));
    }
    if let Some(v) = parts.pass {
        arr.insert(K::from_bytes(b"pass"), Value::str(v));
    }
    if let Some(v) = parts.path {
        arr.insert(K::from_bytes(b"path"), Value::str(v));
    }
    if let Some(v) = parts.query {
        arr.insert(K::from_bytes(b"query"), Value::str(v));
    }
    if let Some(v) = parts.fragment {
        arr.insert(K::from_bytes(b"fragment"), Value::str(v));
    }
    Ok(Value::Array(arr))
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
    put(m, "nl2br", fn_nl2br, 1, Some(2));
    put(m, "addslashes", fn_addslashes, 1, Some(1));
    put(m, "stripslashes", fn_stripslashes, 1, Some(1));
    put(m, "quotemeta", fn_quotemeta, 1, Some(1));
    // strip_tags: basic (no allowed-tags list) — single-argument form only.
    put(m, "strip_tags", fn_strip_tags, 1, Some(1));
    put(m, "parse_url", fn_parse_url, 1, Some(2));
}

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
        assert_eq!(
            calls(fn_urlencode, &[s("a b+c&d=e/f?g~h.i_j-k")]),
            "a+b%2Bc%26d%3De%2Ff%3Fg%7Eh.i_j-k"
        );
        assert_eq!(call(fn_urlencode, &[sb(b"\xff\x00A~")]), b"%FF%00A%7E");
    }

    #[test]
    fn rawurlencode_cases() {
        assert_eq!(
            calls(fn_rawurlencode, &[s("a b+c&d=e/f?g~h.i_j-k")]),
            "a%20b%2Bc%26d%3De%2Ff%3Fg~h.i_j-k"
        );
        assert_eq!(call(fn_rawurlencode, &[sb(b"\xff\x00A~")]), b"%FF%00A~");
    }

    #[test]
    fn urldecode_cases() {
        assert_eq!(
            calls(fn_urldecode, &[s("a+b%20c%26d%ZZ%4")]),
            "a b c&d%ZZ%4"
        );
        assert_eq!(calls(fn_urldecode, &[s("abc%")]), "abc%");
        assert_eq!(calls(fn_rawurldecode, &[s("a+b%20c%26d")]), "a+b c&d");
        assert_eq!(calls(fn_rawurldecode, &[s("ab%2")]), "ab%2");
    }

    #[test]
    fn nl2br_cases() {
        assert_eq!(
            call(fn_nl2br, &[sb(b"a\nb\r\nc\rd\n\re")]),
            b"a<br />\nb<br />\r\nc<br />\rd<br />\n\re"
        );
        assert_eq!(
            call(fn_nl2br, &[sb(b"a\nb"), Value::Bool(false)]),
            b"a<br>\nb"
        );
    }

    #[test]
    fn addslashes_cases() {
        assert_eq!(
            call(fn_addslashes, &[sb(b"a'b\"c\\d\x00e")]),
            b"a\\'b\\\"c\\\\d\\0e"
        );
    }

    #[test]
    fn stripslashes_cases() {
        assert_eq!(
            call(fn_stripslashes, &[sb(b"a\\'b\\\"c\\\\d\\0e\\n")]),
            b"a'b\"c\\d\x00en"
        );
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
        for i in 0..cases.len() {
            assert_eq!(
                hex(&call(fn_strip_tags, &[s(cases[i])])),
                php_strip[i],
                "strip_tags case {i}"
            );
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
