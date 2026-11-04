//! php's URL parser (`php_url_parse_ex2` in ext/standard/url.c): what
//! `parse_url()` and the `parse_url` URI parser of the stream wrappers use.

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

#[cfg(test)]
mod tests;
