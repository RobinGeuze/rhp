//! ext/standard/url.c: php's URL parser (`php_url_parse_ex2`), which
//! `parse_url()` and the stream wrappers use, the `urlencode` family and
//! `get_headers`.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::{Host, Thrown};

/// The parts of a URL, as `php_url` holds them.
#[derive(Default, Clone, Debug)]
pub struct Url {
    pub scheme: Option<Vec<u8>>,
    pub user: Option<Vec<u8>>,
    pub pass: Option<Vec<u8>>,
    pub host: Option<Vec<u8>>,
    /// `0` when there is none (php's `unsigned short port`).
    pub port: u16,
    /// Whether a port was given (it may be 0).
    pub has_port: bool,
    pub path: Option<Vec<u8>>,
    pub query: Option<Vec<u8>>,
    pub fragment: Option<Vec<u8>>,
}

/// `php_replace_controlchars`.
fn clean(s: &[u8]) -> Vec<u8> {
    s.iter()
        .map(|&c| if c.is_ascii_control() { b'_' } else { c })
        .collect()
}

/// `binary_strcspn`: the first of `chars` in `s[from..to]`, else `to`.
fn strcspn(s: &[u8], from: usize, to: usize, chars: &[u8]) -> usize {
    let mut e = to;
    for &c in chars {
        if let Some(p) = s[from..e].iter().position(|&b| b == c) {
            e = from + p;
        }
    }
    e
}

/// `ZEND_STRTOL(buf, &end, 10)` on at most 5 bytes: leading white space and
/// a sign, then digits; `None` when there is no digit.
fn port_num(buf: &[u8]) -> Option<i64> {
    let mut i = buf
        .iter()
        .take_while(|&&b| matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'))
        .count();
    let neg = buf.get(i) == Some(&b'-');
    if matches!(buf.get(i), Some(b'-' | b'+')) {
        i += 1;
    }
    let n = buf[i..].iter().take_while(|b| b.is_ascii_digit()).count();
    if n == 0 {
        return None;
    }
    let v = buf[i..i + n]
        .iter()
        .fold(0i64, |a, &d| a * 10 + (d - b'0') as i64);
    Some(if neg { -v } else { v })
}

/// `php_url_parse_ex2`: `None` where php returns NULL.
pub fn parse(str: &[u8]) -> Option<Url> {
    let mut ret = Url::default();
    let ue = str.len();
    let mut s = 0usize;
    let rel = |s: usize| s + 1 < ue && str[s] == b'/' && str[s + 1] == b'/';

    enum Next {
        Port(usize),
        Host,
        Path,
    }

    let colon = str.iter().position(|&b| b == b':');
    let next = match colon {
        Some(e) if e != s => 'scheme: {
            for &c in &str[s..e] {
                if !c.is_ascii_alphanumeric() && c != b'+' && c != b'.' && c != b'-' {
                    if e + 1 < ue && e < strcspn(str, s, ue, b"?#") {
                        break 'scheme Next::Port(e);
                    } else if rel(s) {
                        s += 2;
                        break 'scheme Next::Host;
                    } else {
                        break 'scheme Next::Path;
                    }
                }
            }
            if e + 1 == ue {
                ret.scheme = Some(clean(&str[s..e]));
                return Some(ret);
            }
            if str[e + 1] != b'/' {
                // a.com:80 is a host and port, not a scheme
                let mut p = e + 1;
                while p < ue && str[p].is_ascii_digit() {
                    p += 1;
                }
                if (p == ue || str[p] == b'/') && p - e < 7 {
                    break 'scheme Next::Port(e);
                }
                ret.scheme = Some(clean(&str[s..e]));
                s = e + 1;
                Next::Path
            } else {
                ret.scheme = Some(clean(&str[s..e]));
                if e + 2 < ue && str[e + 2] == b'/' {
                    s = e + 3;
                    let is_file = ret
                        .scheme
                        .as_deref()
                        .is_some_and(|sc| sc.eq_ignore_ascii_case(b"file"));
                    if is_file && e + 3 < ue && str[e + 3] == b'/' {
                        // file:///c:/dir: windows drive letters
                        if e + 5 < ue && str[e + 5] == b':' {
                            s = e + 4;
                        }
                        Next::Path
                    } else {
                        Next::Host
                    }
                } else {
                    s = e + 1;
                    Next::Path
                }
            }
        }
        Some(e) => Next::Port(e),
        None if rel(s) => {
            s += 2;
            Next::Host
        }
        None => Next::Path,
    };

    let next = match next {
        Next::Port(e) => {
            let p = e + 1;
            let mut pp = p;
            while pp < ue && pp - p < 6 && str[pp].is_ascii_digit() {
                pp += 1;
            }
            if pp - p > 0 && pp - p < 6 && (pp == ue || str[pp] == b'/') {
                match port_num(&str[p..pp]) {
                    Some(port) if (0..=65535).contains(&port) => {
                        ret.has_port = true;
                        ret.port = port as u16;
                        if rel(s) {
                            s += 2;
                        }
                        Next::Host
                    }
                    _ => return None,
                }
            } else if p == pp && pp == ue {
                return None;
            } else if rel(s) {
                s += 2;
                Next::Host
            } else {
                Next::Path
            }
        }
        n => n,
    };

    if let Next::Host = next {
        let e = strcspn(str, s, ue, b"/?#");
        // user and password
        if let Some(at) = str[s..e].iter().rposition(|&b| b == b'@').map(|p| s + p) {
            match str[s..at].iter().position(|&b| b == b':').map(|p| s + p) {
                Some(pp) => {
                    ret.user = Some(clean(&str[s..pp]));
                    ret.pass = Some(clean(&str[pp + 1..at]));
                }
                None => ret.user = Some(clean(&str[s..at])),
            }
            s = at + 1;
        }
        // port
        let ipv6 = s < ue && str[s] == b'[' && e > 0 && str[e - 1] == b']';
        let colon = if ipv6 {
            None
        } else {
            str[s..e].iter().rposition(|&b| b == b':').map(|p| s + p)
        };
        let p = match colon {
            Some(c) => {
                if ret.port == 0 {
                    let p = c + 1;
                    if e - p > 5 {
                        return None;
                    } else if e > p {
                        match port_num(&str[p..e]) {
                            Some(port) if (0..=65535).contains(&port) => {
                                ret.has_port = true;
                                ret.port = port as u16;
                            }
                            _ => return None,
                        }
                    }
                }
                c
            }
            None => e,
        };
        if p <= s {
            return None;
        }
        ret.host = Some(clean(&str[s..p]));
        if e == ue {
            return Some(ret);
        }
        s = e;
    }

    // the path, query and fragment
    let mut e = ue;
    if let Some(h) = str[s..e].iter().position(|&b| b == b'#').map(|p| s + p) {
        ret.fragment = Some(clean(&str[h + 1..e]));
        e = h;
    }
    if let Some(q) = str[s..e].iter().position(|&b| b == b'?').map(|p| s + p) {
        ret.query = Some(clean(&str[q + 1..e]));
        e = q;
    }
    if s < e || s == ue {
        ret.path = Some(clean(&str[s..e]));
    }
    Some(ret)
}

// Native URL and string-escaping builtins (urlencode, nl2br, addslashes,
// strip_tags, parse_url, ...). The HTML entity functions are in `html`.
//
// The unit tests' expected values were captured from php.

// ---------------------------------------------------------------------------
// small shared helpers
// ---------------------------------------------------------------------------

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

// ---------------------------------------------------------------------------
// urlencode / rawurlencode / urldecode / rawurldecode
// ---------------------------------------------------------------------------

fn fn_urlencode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
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
    let s = conv::to_string_bytes(&args[0]);
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
    Ok(Value::str(crate::main::php_variables::url_decode(
        &conv::to_string_bytes(&args[0]),
    )))
}

fn fn_rawurldecode(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(crate::main::php_variables::raw_url_decode(
        &conv::to_string_bytes(&args[0]),
    )))
}

// ---------------------------------------------------------------------------
// parse_url
// ---------------------------------------------------------------------------

/// php `parse_url($url, $component = -1)`.
fn fn_parse_url(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let url = conv::to_string_bytes(&args[0]);
    let component = args.get(1).map(|v| conv::to_int(&deref(v))).unwrap_or(-1);
    let Some(u) = parse(&url) else {
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

/// `get_headers($url, $associative = false, $context = null)`: the
/// response headers of an http(s) URL (the wrapper's only-headers mode).
fn fn_get_headers(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let fname = "get_headers";
    let url = crate::zend::zpp::path_param(host, args, 0, fname, "url")?;
    let assoc = crate::zend::zpp::bool_param(host, args, 1, fname, "associative", false)?;
    let ctx = match args.get(2).map(deref) {
        None | Some(Value::Null) => host.default_stream_context(),
        Some(v @ Value::Resource(_)) => v,
        Some(v) => {
            return Err(Thrown::type_error(format!(
                "{fname}(): Argument #3 ($context) must be of type resource or null, {} given",
                crate::zend::zpp::zval_value_name(host, &v)
            )))
        }
    };
    if crate::main::streams::is_data_url(&url) {
        // a URL wrapper whose stream has no header array
        match host.stream_open(&url, b"r") {
            Some(res) => {
                host.stream_close(&res);
            }
            None => crate::open_failed(host, fname, &url)?,
        }
        return Ok(Value::Bool(false));
    }
    if !crate::ext::standard::http_fopen_wrapper::is_http(&url) {
        // STREAM_USE_URL refuses a wrapper that is not for URLs (the
        // plain-files one when none matches the scheme)
        let scheme = crate::main::streams::split_scheme(&url)
            .0
            .to_ascii_lowercase();
        let known = host.stream_wrapper_schemes();
        if !matches!(&scheme[..], b"file" | b"glob" | b"php")
            && !known.iter().any(|k| k.eq_ignore_ascii_case(&scheme))
        {
            let name = String::from_utf8_lossy(&scheme).into_owned();
            crate::warn(
                host,
                fname,
                &format!(
                    "Unable to find the wrapper \"{name}\" - did you forget to enable it when you configured PHP?"
                ),
            )?;
        }
        crate::warn(host, fname, "This function may only be used against URLs")?;
        return Ok(Value::Bool(false));
    }
    let Some(res) =
        crate::ext::standard::http_fopen_wrapper::open(host, fname, &url, b"r", &ctx, true)?
    else {
        return Ok(Value::Bool(false));
    };
    let data = host.stream_meta(&res).and_then(|m| m.wrapper_data);
    host.stream_close(&res);
    let Some(Value::Array(headers)) = data else {
        return Ok(Value::Bool(false));
    };
    let mut out = rhp_values::PhpArray::new();
    for (_, h) in headers.iter() {
        let Value::Str(h) = h else { continue };
        let h = h.as_bytes();
        let colon = h.iter().position(|&c| c == b':').filter(|_| assoc);
        let Some(p) = colon else {
            out.push(Value::str(h.to_vec()));
            continue;
        };
        let mut v = p + 1;
        while v < h.len() && h[v].is_ascii_whitespace() {
            v += 1;
        }
        let key = rhp_values::ArrayKey::from_bytes(&h[..p]);
        let value = Value::str(h[v..].to_vec());
        match out.get(&key).cloned() {
            None => {
                out.insert(key, value);
            }
            Some(Value::Array(mut prev)) => {
                prev.push(value);
                out.insert(key, Value::Array(prev));
            }
            Some(prev) => {
                let mut a = rhp_values::PhpArray::new();
                a.push(prev);
                a.push(value);
                out.insert(key, Value::Array(a));
            }
        }
    }
    Ok(Value::Array(out))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("urlencode", fn_urlencode),
            ("urldecode", fn_urldecode),
            ("rawurlencode", fn_rawurlencode),
            ("rawurldecode", fn_rawurldecode),
            ("parse_url", fn_parse_url),
            ("get_headers", fn_get_headers),
        ],
    );
}

#[cfg(test)]
mod tests;
