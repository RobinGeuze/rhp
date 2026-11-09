//! The charset half of the WHATWG MIME type parser, ported from php-src
//! ext/libxml/mime_sniff.c (PHP License 3.01): the charset of an HTTP
//! `Content-Type` header, strictly parsed (a malformed header has none).

use rhp_values::{ops::deref, Value};

fn is_http_tab_or_space(c: u8) -> bool {
    c == 0x09 || c == 0x20
}

fn is_http_whitespace(c: u8) -> bool {
    c == 0x0A || c == 0x0D || is_http_tab_or_space(c)
}

fn is_http_quoted_string_token(c: u8) -> bool {
    c == 0x09 || (c >= 0x20 && c != 0x7F)
}

fn is_http_token(c: u8) -> bool {
    c == 0x21
        || (0x23..=0x27).contains(&c)
        || matches!(
            c,
            0x2A | 0x2B | 0x2D | 0x2E | 0x5E | 0x5F | 0x60 | 0x7C | 0x7E
        )
        || c.is_ascii_alphanumeric()
}

/// collect_a_sequence_of_code_points: the length of the run at `s[pos..]`.
fn collect(s: &[u8], pos: usize, cond: impl Fn(u8) -> bool) -> usize {
    s[pos..].iter().take_while(|&&c| cond(c)).count()
}

fn not_solely_tokens(s: &[u8]) -> bool {
    s.is_empty() || !s.iter().all(|&c| is_http_token(c))
}

/// collect_an_http_quoted_string_with_extract_value: the value and the
/// position after it (`s[pos]` is the opening quote).
fn collect_quoted(s: &[u8], mut pos: usize) -> (Vec<u8>, usize) {
    let mut value = Vec::new();
    pos += 1;
    loop {
        let n = collect(s, pos, |c| c != b'"' && c != b'\\');
        value.extend_from_slice(&s[pos..pos + n]);
        pos += n;
        if pos >= s.len() {
            break;
        }
        let quote_or_backslash = s[pos];
        pos += 1;
        if quote_or_backslash == b'\\' {
            if pos >= s.len() {
                value.push(b'\\');
                break;
            }
            value.push(s[pos]);
            pos += 1;
        } else {
            break;
        }
    }
    (value, pos)
}

/// php_libxml_sniff_charset_from_string
pub fn charset_from_string(s: &[u8]) -> Option<Vec<u8>> {
    let mut start = 0;
    let mut end = s.len();
    while start < end && is_http_whitespace(s[start]) {
        start += 1;
    }
    while start < end && is_http_whitespace(s[end - 1]) {
        end -= 1;
    }
    let s = &s[start..end];
    let mut pos = 0;
    let type_len = collect(s, pos, |c| c != b'/');
    if not_solely_tokens(&s[pos..pos + type_len]) {
        return None;
    }
    pos += type_len;
    if pos >= s.len() {
        return None;
    }
    pos += 1;
    let subtype_len = collect(s, pos, |c| c != b';');
    if not_solely_tokens(&s[pos..pos + subtype_len]) {
        return None;
    }
    pos += subtype_len;
    while pos < s.len() {
        pos += 1;
        pos += collect(s, pos, is_http_whitespace);
        let name_len = collect(s, pos, |c| c != b';' && c != b'=');
        let name = &s[pos..pos + name_len];
        pos += name_len;
        if pos < s.len() {
            if s[pos] == b';' {
                continue;
            }
            pos += 1;
        } else {
            break;
        }
        let value = if s.get(pos) == Some(&b'"') {
            let (v, p) = collect_quoted(s, pos);
            pos = p + collect(s, p, |c| c != b';');
            v
        } else {
            let n = collect(s, pos, |c| c != b';');
            let mut v = s[pos..pos + n].to_vec();
            pos += n;
            while v.last().is_some_and(|&c| is_http_whitespace(c)) {
                v.pop();
            }
            if v.is_empty() {
                continue;
            }
            v
        };
        if name.eq_ignore_ascii_case(b"charset")
            && value.iter().all(|&c| is_http_quoted_string_token(c))
        {
            return Some(value);
        }
    }
    None
}

/// php_libxml_sniff_charset_from_stream: the charset of the last
/// `Content-Type` among a stream's response headers (its wrapper data).
pub fn charset_from_stream(host: &dyn crate::Host, res: &Value) -> Option<Vec<u8>> {
    let Some(Value::Array(headers)) = host
        .stream_meta(res)
        .and_then(|m| m.wrapper_data)
        .map(|v| deref(&v))
    else {
        return None;
    };
    // Scan backwards: after a redirect the array holds every response's
    // headers.
    for v in headers.values().collect::<Vec<_>>().into_iter().rev() {
        let Value::Str(h) = deref(v) else {
            continue;
        };
        let h = h.as_bytes();
        // No colon (or a space before it): the status line.
        let colon = h.iter().position(|&c| c == b':');
        let space = h.iter().position(|&c| c == b' ');
        let colon = colon?;
        if space.is_some_and(|sp| sp < colon) {
            return None;
        }
        if h.len() >= 13 && h[..13].eq_ignore_ascii_case(b"content-type:") {
            return charset_from_string(&h[13..]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::charset_from_string as cs;

    #[test]
    fn charset() {
        let s = |v: &str| cs(v.as_bytes()).map(|c| String::from_utf8(c).unwrap());
        assert_eq!(
            s("text/html; charset=ISO-8859-1"),
            Some("ISO-8859-1".into())
        );
        assert_eq!(s(" text/html;Charset=\"a\\\"b\" "), Some("a\"b".into()));
        // The subtype ends at ';', trailing whitespace included.
        assert_eq!(s("text/html ;charset=x"), None);
        assert_eq!(s("text/html; foo=bar; charset = x"), None);
        assert_eq!(s("text/html; foo; charset=x"), Some("x".into()));
        assert_eq!(s("text/html; charset="), None);
        assert_eq!(s("text/; charset=x"), None);
        assert_eq!(s("/html; charset=x"), None);
        assert_eq!(s("foo"), None);
        assert_eq!(s("text/html;charset=\"x"), Some("x".into()));
    }
}
