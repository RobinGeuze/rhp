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

fn fn_urldecode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(crate::query_fns::url_decode(&bytes_of(
        &args[0],
    ))))
}

fn fn_rawurldecode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(crate::query_fns::raw_url_decode(&bytes_of(
        &args[0],
    ))))
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
// strip_tags
// ---------------------------------------------------------------------------

fn fn_strip_tags(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let allow = match args.get(1).map(deref) {
        None | Some(Value::Null) => None,
        // Each element becomes `<tag>`; an empty array allows nothing.
        Some(Value::Array(a)) => {
            let mut set = Vec::new();
            for (_, v) in a.iter() {
                set.push(b'<');
                set.extend_from_slice(&crate::argparse::zval_get_string(host, v)?);
                set.push(b'>');
            }
            (!set.is_empty()).then_some(set)
        }
        Some(v) => Some(bytes_of(&v)),
    };
    Ok(Value::str(strip_tags(
        &bytes_of(&args[0]),
        allow.as_deref(),
    )))
}

/// php_tag_find: is the tag in `tag` (`<a href=..>`, `</a>`), normalized to
/// `<a>`, in the allowed `set`?
fn tag_find(tag: &[u8], set: &[u8]) -> bool {
    if tag.is_empty() {
        return false;
    }
    let at = |i: usize| tag.get(i).copied().unwrap_or(0);
    let mut norm = Vec::with_capacity(tag.len() + 1);
    let mut seen = false;
    let mut t = 0;
    while t < tag.len() {
        let c = at(t).to_ascii_lowercase();
        match c {
            b'<' => norm.push(c),
            b'>' => break,
            _ if !is_space(c) => {
                seen = true;
                if c != b'/' || (t > 0 && at(t - 1) != b'<' && at(t + 1) != b'>') {
                    norm.push(c);
                }
            }
            _ if seen => break,
            _ => {}
        }
        t += 1;
    }
    norm.push(b'>');
    // strstr: the set ends at its first NUL.
    let set = &set[..set.iter().position(|&b| b == 0).unwrap_or(set.len())];
    set.windows(norm.len()).any(|w| w == norm)
}

/// php_strip_tags_ex: the state machine over text (0), an HTML tag (1),
/// PHP code (2), `<!...>` (3) and an HTML comment (4). With `allow`, a tag
/// in that set is kept.
fn strip_tags(s: &[u8], allow: Option<&[u8]>) -> Vec<u8> {
    let allow = allow.map(|a| a.to_ascii_lowercase());
    // The C reads one byte past the end, a NUL.
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let mut out = Vec::with_capacity(s.len());
    let mut tbuf: Vec<u8> = Vec::new();
    let (mut lc, mut br, mut depth, mut in_q) = (0u8, 0i32, 0i32, 0u8);
    let mut is_xml = false;
    let mut state = 0u8;
    let toggle = |in_q: u8, c: u8| if in_q != 0 { 0 } else { c };
    for p in 0..s.len() {
        let c = s[p];
        let prev = |n: usize| if p >= n { s[p - n] } else { 0 };
        // A byte of the tag being read in state 1.
        let mut reg_char = false;
        match state {
            0 => match c {
                0 => {}
                b'<' if in_q != 0 => {}
                b'<' if is_space(at(p + 1)) => out.push(c),
                b'<' => {
                    lc = b'<';
                    state = 1;
                    if allow.is_some() {
                        tbuf.push(b'<');
                    }
                }
                b'>' if depth > 0 => depth -= 1,
                b'>' if in_q != 0 => {}
                _ => out.push(c),
            },
            1 => match c {
                0 => {}
                b'<' if in_q != 0 => {}
                b'<' if is_space(at(p + 1)) => reg_char = true,
                b'<' => depth += 1,
                b'>' if depth > 0 => depth -= 1,
                b'>' if in_q != 0 => {}
                b'>' => {
                    lc = b'>';
                    if !(is_xml && prev(1) == b'-') {
                        in_q = 0;
                        state = 0;
                        is_xml = false;
                        if let Some(set) = &allow {
                            tbuf.push(b'>');
                            if tag_find(&tbuf, set) {
                                out.extend_from_slice(&tbuf);
                            }
                            tbuf.clear();
                        }
                    }
                }
                b'"' | b'\'' => {
                    if p != 0 && (in_q == 0 || c == in_q) {
                        in_q = toggle(in_q, c);
                    }
                    reg_char = true;
                }
                b'!' if prev(1) == b'<' => {
                    state = 3;
                    lc = c;
                }
                b'?' if prev(1) == b'<' => {
                    br = 0;
                    state = 2;
                }
                _ => reg_char = true,
            },
            2 => match c {
                b'(' if lc != b'"' && lc != b'\'' => {
                    lc = b'(';
                    br += 1;
                }
                b')' if lc != b'"' && lc != b'\'' => {
                    lc = b')';
                    br -= 1;
                }
                b'>' if depth > 0 => depth -= 1,
                b'>' if in_q != 0 => {}
                b'>' => {
                    if br == 0 && p >= 1 && lc != b'"' && prev(1) == b'?' {
                        in_q = 0;
                        state = 0;
                        tbuf.clear();
                    }
                }
                b'"' | b'\'' if p >= 1 && prev(1) != b'\\' => {
                    if lc == c {
                        lc = 0;
                    } else if lc != b'\\' {
                        lc = c;
                    }
                    if in_q == 0 || c == in_q {
                        in_q = toggle(in_q, c);
                    }
                }
                // `<?xml` is not PHP: back to a tag.
                b'l' | b'L'
                    if p > 4
                        && prev(1).eq_ignore_ascii_case(&b'm')
                        && prev(2).eq_ignore_ascii_case(&b'x')
                        && prev(3) == b'?'
                        && prev(4) == b'<' =>
                {
                    state = 1;
                    is_xml = true;
                }
                _ => {}
            },
            3 => match c {
                b'>' if depth > 0 => depth -= 1,
                b'>' if in_q != 0 => {}
                b'>' => {
                    in_q = 0;
                    state = 0;
                    tbuf.clear();
                }
                b'"' | b'\'' if p != 0 && prev(1) != b'\\' && (in_q == 0 || c == in_q) => {
                    in_q = toggle(in_q, c);
                }
                b'-' if p >= 2 && prev(1) == b'-' && prev(2) == b'!' => state = 4,
                // `<!DOCTYPE` is a tag.
                b'E' | b'e'
                    if p > 6 && (1..=6).all(|n| prev(n).eq_ignore_ascii_case(&b"epytcod"[n])) =>
                {
                    state = 1
                }
                _ => {}
            },
            _ => {
                if c == b'>' && in_q == 0 && p >= 2 && prev(1) == b'-' && prev(2) == b'-' {
                    in_q = 0;
                    state = 0;
                    tbuf.clear();
                }
            }
        }
        if reg_char && allow.is_some() {
            tbuf.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// parse_url
// ---------------------------------------------------------------------------

/// php `parse_url($url, $component = -1)`.
fn fn_parse_url(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let url = bytes_of(&args[0]);
    let component = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(-1);
    let Some(u) = crate::url::parse(&url) else {
        return Ok(Value::Bool(false));
    };
    let port = u.has_port.then_some(u.port as i64);
    if component > -1 {
        let part = match component {
            0 => u.scheme,
            1 => u.host,
            2 => return Ok(port.map_or(Value::Null, Value::Int)),
            3 => u.user,
            4 => u.pass,
            5 => u.path,
            6 => u.query,
            7 => u.fragment,
            _ => {
                return Err(Thrown::error(
                    "ValueError",
                    format!("parse_url(): Argument #2 ($component) must be a valid URL component identifier, {component} given"),
                ))
            }
        };
        return Ok(part.map_or(Value::Null, Value::str));
    }
    let mut arr = rhp_values::PhpArray::new();
    let mut put = |k: &[u8], v: Option<Value>| {
        if let Some(v) = v {
            arr.insert(rhp_values::ArrayKey::from_bytes(k), v);
        }
    };
    put(b"scheme", u.scheme.map(Value::str));
    put(b"host", u.host.map(Value::str));
    put(b"port", port.map(Value::Int));
    put(b"user", u.user.map(Value::str));
    put(b"pass", u.pass.map(Value::str));
    put(b"path", u.path.map(Value::str));
    put(b"query", u.query.map(Value::str));
    put(b"fragment", u.fragment.map(Value::str));
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
    put(m, "strip_tags", fn_strip_tags, 1, Some(2));
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
