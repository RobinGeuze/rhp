//! libcurl 8.23's URL API (urlapi.c) as far as php can observe it: the URL a
//! handle reports through `CURLINFO_EFFECTIVE_URL` before a transfer (the
//! `CURLOPT_URL` value parsed and put back together, or `false` with no `url`
//! key in `curl_getinfo()` when it does not parse), and the parse
//! `curl_exec` does first (with scheme guessing), whose failure is
//! `CURLE_URL_MALFORMAT` "URL rejected: …" with no transfer at all.
//!
//! Credentials are parsed off and not reported; scheme and IPv4 hosts are
//! normalised, IPv6 addresses shortened where libcurl does so, hostnames
//! percent-decoded, dot segments removed (unless `CURLOPT_PATH_AS_IS`) and an
//! empty path becomes `/`. URLs are bytes: non-ASCII bytes pass through.
//! Behaviour captured from the reference php's libcurl.

use std::net::Ipv6Addr;

/// Why libcurl's parser rejected a URL (`CURLUcode`, with the
/// `curl_url_strerror` text php shows after "URL rejected: ").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UrlError {
    Malformed,
    BadSlashes,
    BadScheme,
    NoHost,
    BadPort,
    BadHostname,
    BadIpv6,
    BadUser,
    BadPassword,
    BadFileUrl,
    BadFragment,
    BadQuery,
    BadPath,
}

impl UrlError {
    pub(super) fn text(self) -> &'static str {
        match self {
            UrlError::Malformed => "Malformed input to a URL function",
            UrlError::BadSlashes => "Unsupported number of slashes following scheme",
            UrlError::BadScheme => "Bad scheme",
            UrlError::NoHost => "No host present",
            UrlError::BadPort => "Port number was not a decimal number between 0 and 65535",
            UrlError::BadHostname => "Bad hostname",
            UrlError::BadIpv6 => "Bad IPv6 address",
            UrlError::BadUser => "Bad user",
            UrlError::BadPassword => "Bad password",
            UrlError::BadFileUrl => "Bad file:// URL",
            UrlError::BadFragment => "Bad fragment",
            UrlError::BadQuery => "Bad query",
            UrlError::BadPath => "Bad path",
        }
    }
}

/// The `curl_url_set` flags that change what php sees.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Flags {
    /// `CURLU_GUESS_SCHEME` (the transfer's parse): a URL without
    /// `scheme:/` gets one guessed from its hostname.
    pub guess_scheme: bool,
    /// `CURLU_PATH_AS_IS` (`CURLOPT_PATH_AS_IS`): keep dot segments.
    pub path_as_is: bool,
}

/// What `CURLINFO_EFFECTIVE_URL` reports before a transfer.
pub(super) fn pre_transfer_url(url: &[u8], path_as_is: bool) -> Option<Vec<u8>> {
    let flags = Flags {
        guess_scheme: false,
        path_as_is,
    };
    parse(url, flags).ok()
}

/// libcurl's `MAX_SCHEME_LEN`.
const MAX_SCHEME_LEN: usize = 40;

/// Parse and reassemble `url` as libcurl does.
pub(super) fn parse(url: &[u8], flags: Flags) -> Result<Vec<u8>, UrlError> {
    if url.is_empty() || url.len() > 8_000_000 {
        return Err(UrlError::Malformed);
    }
    let scheme_len = scheme_len(url, flags.guess_scheme);
    let mut out = Vec::with_capacity(url.len() + 8);
    let path_etc: &[u8];
    if scheme_len > 0 && url[..scheme_len].eq_ignore_ascii_case(b"file") {
        // file: URLs: no host but localhost / 127.0.0.1 (POSIX libcurl).
        if url.len() <= 6 {
            return Err(UrlError::BadFileUrl);
        }
        let mut path = &url[5..];
        if let Some(rest) = path.strip_prefix(b"//") {
            path = rest;
            if !path.starts_with(b"/") {
                let host_ok = path.len() >= 10
                    && (path[..10].eq_ignore_ascii_case(b"localhost/")
                        || &path[..10] == b"127.0.0.1/");
                if !host_ok {
                    return Err(UrlError::BadFileUrl);
                }
                path = &path[9..];
            }
        }
        out.extend_from_slice(b"file://");
        path_etc = path;
    } else {
        let (scheme, host_part): (Vec<u8>, &[u8]) = if scheme_len > 0 {
            let after = &url[scheme_len + 1..];
            let slashes = after.iter().take(4).take_while(|&&c| c == b'/').count();
            if !(1..=3).contains(&slashes) {
                return Err(UrlError::BadSlashes);
            }
            (url[..scheme_len].to_ascii_lowercase(), &after[slashes..])
        } else if flags.guess_scheme {
            (Vec::new(), url)
        } else {
            return Err(UrlError::BadScheme);
        };
        let host_len = host_part
            .iter()
            .position(|c| matches!(c, b'/' | b'?' | b'#'))
            .unwrap_or(host_part.len());
        if host_len == 0 {
            return Err(UrlError::NoHost);
        }
        let (host, port) = authority(&host_part[..host_len], scheme_len > 0)?;
        let scheme = if scheme.is_empty() {
            guessed_scheme(&host).to_vec()
        } else {
            scheme
        };
        out.extend_from_slice(&scheme);
        out.extend_from_slice(b"://");
        out.extend_from_slice(&host);
        if let Some(p) = port {
            out.extend_from_slice(format!(":{p}").as_bytes());
        }
        path_etc = &host_part[host_len..];
    }
    // Fragment, then query, then path, each checked for junk.
    let frag_at = path_etc.iter().position(|&c| c == b'#');
    let (before_frag, fragment) = match frag_at {
        Some(i) => (&path_etc[..i], Some(&path_etc[i + 1..])),
        None => (path_etc, None),
    };
    if fragment.is_some_and(junk) {
        return Err(UrlError::BadFragment);
    }
    let (path, query) = match before_frag.iter().position(|&c| c == b'?') {
        Some(i) => (&before_frag[..i], Some(&before_frag[i + 1..])),
        None => (before_frag, None),
    };
    if query.is_some_and(junk) {
        return Err(UrlError::BadQuery);
    }
    if junk(path) {
        return Err(UrlError::BadPath);
    }
    if path.len() <= 1 {
        out.push(b'/');
    } else if flags.path_as_is {
        out.extend_from_slice(path);
    } else {
        out.extend_from_slice(&remove_dot_segments(path));
    }
    if let Some(q) = query {
        out.push(b'?');
        out.extend_from_slice(q);
    }
    if let Some(f) = fragment {
        out.push(b'#');
        out.extend_from_slice(f);
    }
    Ok(out)
}

/// Control bytes, space or DEL: rejected in every URL part.
fn junk(s: &[u8]) -> bool {
    s.iter().any(|&c| c <= b' ' || c == 0x7f)
}

/// `Curl_is_absolute_url`: the scheme's length, or 0. When guessing, the
/// colon must be followed by a slash (so `host:port` is not a scheme).
fn scheme_len(url: &[u8], guess: bool) -> usize {
    if !url.first().is_some_and(u8::is_ascii_alphabetic) {
        return 0;
    }
    let n = url
        .iter()
        .take(MAX_SCHEME_LEN)
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
        .count();
    if n < MAX_SCHEME_LEN && url.get(n) == Some(&b':') && (!guess || url.get(n + 1) == Some(&b'/'))
    {
        n
    } else {
        0
    }
}

/// libcurl's legacy scheme guess from the hostname.
fn guessed_scheme(host: &[u8]) -> &'static [u8] {
    for (prefix, scheme) in [
        (&b"ftp."[..], &b"ftp"[..]),
        (b"dict.", b"dict"),
        (b"ldap.", b"ldap"),
        (b"imap.", b"imap"),
        (b"smtp.", b"smtp"),
        (b"pop3.", b"pop3"),
    ] {
        if host.len() >= prefix.len() && host[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return scheme;
        }
    }
    b"http"
}

/// `parse_authority`: credentials, port and host (normalised).
fn authority(auth: &[u8], has_scheme: bool) -> Result<(Vec<u8>, Option<u32>), UrlError> {
    // The login ends at the first '@'; it is validated, not reported.
    let hostport = match auth.iter().position(|&c| c == b'@') {
        Some(at) => {
            let login = &auth[..at];
            let (user, password) = match login.iter().position(|&c| c == b':') {
                Some(i) => (&login[..i], Some(&login[i + 1..])),
                None => (login, None),
            };
            if junk(user) {
                return Err(UrlError::BadUser);
            }
            if password.is_some_and(junk) {
                return Err(UrlError::BadPassword);
            }
            &auth[at + 1..]
        }
        None => auth,
    };
    let (host, port) = split_port(hostport, has_scheme)?;
    if host.is_empty() {
        return Err(UrlError::NoHost);
    }
    if host[0] == b'[' {
        return Ok((ipv6(host)?, port));
    }
    if let Some(v4) = ipv4(host) {
        return Ok((v4.into_bytes(), port));
    }
    Ok((hostname(host)?, port))
}

/// `Curl_parse_port`: `host[:port]` → host and the port number (`None`
/// when absent, or empty after a scheme).
fn split_port(hp: &[u8], has_scheme: bool) -> Result<(&[u8], Option<u32>), UrlError> {
    let colon = if hp.first() == Some(&b'[') {
        match hp.iter().position(|&c| c == b']') {
            Some(close) => match hp.get(close + 1) {
                None => None,
                Some(b':') => Some(close + 1),
                Some(_) => return Err(UrlError::BadPort),
            },
            // No closing bracket: the IPv6 parse rejects it.
            None => None,
        }
    } else {
        hp.iter().position(|&c| c == b':')
    };
    let Some(colon) = colon else {
        return Ok((hp, None));
    };
    let digits = &hp[colon + 1..];
    if digits.is_empty() {
        return if has_scheme {
            Ok((&hp[..colon], None))
        } else {
            Err(UrlError::BadPort)
        };
    }
    let mut n: u32 = 0;
    for &c in digits {
        if !c.is_ascii_digit() {
            return Err(UrlError::BadPort);
        }
        n = n * 10 + u32::from(c - b'0');
        if n > 0xffff {
            return Err(UrlError::BadPort);
        }
    }
    Ok((&hp[..colon], Some(n)))
}

/// `%XX` decoding that keeps a `%` not followed by two hex digits.
fn percent_decode(s: &[u8]) -> Vec<u8> {
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%' && i + 2 < s.len() {
            if let (Some(h), Some(l)) = (hex(s[i + 1]), hex(s[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

/// A registered name: percent-decoded, then `hostname_check`.
fn hostname(host: &[u8]) -> Result<Vec<u8>, UrlError> {
    let host = percent_decode(host);
    const BAD: &[u8] = b"!\"#$%&'()*+,/:;<=>?@[\\]^`{|}";
    if host
        .iter()
        .any(|&c| c <= b' ' || c == 0x7f || BAD.contains(&c))
    {
        return Err(UrlError::BadHostname);
    }
    // One trailing dot is allowed (a rooted name), not two.
    let stem = host.strip_suffix(b".").unwrap_or(&host);
    if stem.is_empty() || stem.ends_with(b".") {
        return Err(UrlError::BadHostname);
    }
    Ok(host)
}

/// `ipv6_parse`: `[address%zone]`, percent-decoded first. The address is
/// rewritten in libcurl's `inet_ntop` form when that is not longer; the
/// zone id (1 to 15 of `A-Za-z0-9-._~` or non-ASCII bytes, after an
/// optional encoded `25`) is reported as `%25zone`.
fn ipv6(host: &[u8]) -> Result<Vec<u8>, UrlError> {
    let inner = host
        .strip_prefix(b"[")
        .and_then(|h| h.strip_suffix(b"]"))
        .ok_or(UrlError::BadIpv6)?;
    let inner = percent_decode(inner);
    let addr_len = inner
        .iter()
        .take_while(|c| c.is_ascii_hexdigit() || matches!(c, b':' | b'.'))
        .count();
    let zone = if addr_len < inner.len() {
        if inner[addr_len] != b'%' {
            return Err(UrlError::BadIpv6);
        }
        let mut z = &inner[addr_len + 1..];
        if z.len() > 2 && z.starts_with(b"25") {
            z = &z[2..];
        }
        let ok = |c: &u8| {
            c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~') || *c >= 0x80
        };
        if z.is_empty() || z.len() > 15 || !z.iter().all(ok) {
            return Err(UrlError::BadIpv6);
        }
        Some(z)
    } else {
        None
    };
    let addr = &inner[..addr_len];
    if addr.len() < 2 {
        return Err(UrlError::BadIpv6);
    }
    let parsed: Ipv6Addr = std::str::from_utf8(addr)
        .ok()
        .and_then(|a| a.parse().ok())
        .ok_or(UrlError::BadIpv6)?;
    let norm = ntop(&parsed);
    let mut out = b"[".to_vec();
    if norm.len() <= addr.len() {
        out.extend_from_slice(norm.as_bytes());
    } else {
        out.extend_from_slice(addr);
    }
    if let Some(z) = zone {
        out.extend_from_slice(b"%25");
        out.extend_from_slice(z);
    }
    out.push(b']');
    Ok(out)
}

/// libcurl's `inet_ntop` for IPv6 (the classic ISC algorithm): the longest
/// run of two or more zero groups becomes `::`, and an IPv4-compatible or
/// -mapped address ends in dotted quad.
fn ntop(a: &Ipv6Addr) -> String {
    let w = a.segments();
    let (mut best, mut cur): ((usize, usize), Option<(usize, usize)>) = ((0, 0), None);
    for (i, &g) in w.iter().enumerate() {
        if g == 0 {
            let c = cur.get_or_insert((i, 0));
            c.1 += 1;
            if c.1 > best.1 {
                best = *c;
            }
        } else {
            cur = None;
        }
    }
    if best.1 < 2 {
        best = (usize::MAX, 0);
    }
    let mut s = String::new();
    let mut i = 0;
    while i < 8 {
        if i == best.0 {
            s.push(':');
            i += best.1;
            if i == 8 {
                s.push(':');
            }
            continue;
        }
        if i > 0 {
            s.push(':');
        }
        if i == 6 && best.0 == 0 && (best.1 == 6 || (best.1 == 5 && w[5] == 0xffff)) {
            let o = a.octets();
            s.push_str(&format!("{}.{}.{}.{}", o[12], o[13], o[14], o[15]));
            break;
        }
        s.push_str(&format!("{:x}", w[i]));
        i += 1;
    }
    s
}

/// curl's `ipv4_normalize`: one to four dot-separated numbers (decimal,
/// `0x` hex or `0` octal), the last filling the remaining bytes.
fn ipv4(host: &[u8]) -> Option<String> {
    let host = std::str::from_utf8(host).ok()?;
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() > 4 {
        return None;
    }
    let mut nums = Vec::with_capacity(parts.len());
    for p in &parts {
        let (digits, radix) = if let Some(h) = p.strip_prefix("0x").or(p.strip_prefix("0X")) {
            (h, 16)
        } else if p.len() > 1 && p.starts_with('0') {
            (&p[1..], 8)
        } else {
            (*p, 10)
        };
        if digits.is_empty() && radix != 8 {
            return None;
        }
        if !digits.bytes().all(|c| (c as char).is_digit(radix)) {
            return None;
        }
        let n = if digits.is_empty() {
            0
        } else {
            u64::from_str_radix(digits, radix).ok()?
        };
        nums.push(n);
    }
    let (last, init) = nums.split_last()?;
    if init.iter().any(|&n| n > 255) {
        return None;
    }
    let room = 8 * (4 - init.len() as u32);
    if room < 64 && *last >= 1u64 << room {
        return None;
    }
    let mut v: u32 = 0;
    for (i, &n) in init.iter().enumerate() {
        v |= (n as u32) << (24 - 8 * i);
    }
    v |= *last as u32;
    let o = v.to_be_bytes();
    Some(format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3]))
}

/// A path segment that is `.` (`dots == 1`) or `..` (`dots == 2`), where
/// libcurl also counts `%2e` / `%2E` as a dot.
fn is_dots(seg: &[u8], dots: usize) -> bool {
    let mut n = 0;
    let mut i = 0;
    while i < seg.len() {
        if seg[i] == b'.' {
            i += 1;
        } else if seg[i] == b'%'
            && seg.get(i + 1) == Some(&b'2')
            && matches!(seg.get(i + 2), Some(b'e' | b'E'))
        {
            i += 3;
        } else {
            return false;
        }
        n += 1;
    }
    n == dots
}

/// RFC 3986 section 5.2.4 on an absolute path, segment by segment.
fn remove_dot_segments(path: &[u8]) -> Vec<u8> {
    let Some(rest) = path.strip_prefix(b"/") else {
        return path.to_vec();
    };
    let segs: Vec<&[u8]> = rest.split(|&c| c == b'/').collect();
    let mut out: Vec<&[u8]> = Vec::new();
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        if is_dots(seg, 1) {
        } else if is_dots(seg, 2) {
            out.pop();
        } else {
            out.push(seg);
            continue;
        }
        if last {
            out.push(b"");
        }
    }
    let mut r = Vec::with_capacity(path.len());
    for seg in out {
        r.push(b'/');
        r.extend_from_slice(seg);
    }
    if r.is_empty() {
        r.push(b'/');
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre(u: &[u8]) -> Option<String> {
        pre_transfer_url(u, false).map(|v| String::from_utf8(v).unwrap())
    }

    /// Cases captured from php 8.5.12 (libcurl 8.23.0):
    /// `curl_setopt($h, CURLOPT_URL, $u); curl_getinfo($h, CURLINFO_EFFECTIVE_URL)`.
    #[test]
    fn matches_libcurl() {
        let ok: &[(&str, &str)] = &[
            ("http://x", "http://x/"),
            ("HTTP://X.COM", "http://X.COM/"),
            ("http://x?q=1", "http://x/?q=1"),
            ("http://x#f", "http://x/#f"),
            ("http://user:pw@x:80/p", "http://x:80/p"),
            ("http://[::1]/", "http://[::1]/"),
            ("http://x/../a", "http://x/a"),
            ("file:///tmp/x", "file:///tmp/x"),
            ("bogus://x", "bogus://x/"),
            ("http://x/é", "http://x/é"),
            ("http://x:/", "http://x/"),
            ("http://x:080/", "http://x:80/"),
            ("http://0x7f.1/", "http://127.0.0.1/"),
            ("http://127.1/", "http://127.0.0.1/"),
            ("http:/x", "http://x/"),
            ("http://x/a/./b/../c?d=../e", "http://x/a/c?d=../e"),
            ("http://x/%7e", "http://x/%7e"),
            ("http://@x/", "http://x/"),
            ("ftp://u:p@h/f;type=a", "ftp://h/f;type=a"),
            ("http://x?", "http://x/?"),
            ("http://x#", "http://x/#"),
            ("https://x:443", "https://x:443/"),
            ("http://[::FFFF:1.2.3.4]/", "http://[::ffff:1.2.3.4]/"),
            ("http://a_b/", "http://a_b/"),
            ("http://x/%zz", "http://x/%zz"),
            ("http://256.1.1.1/", "http://256.1.1.1/"),
            ("http://1.2.3.4.5/", "http://1.2.3.4.5/"),
            ("http://4294967295/", "http://255.255.255.255/"),
            ("http://4294967296/", "http://4294967296/"),
            ("http://0777.1/", "http://0777.1/"),
            ("http://1.0x/", "http://1.0x/"),
            ("file://localhost/x", "file:///x"),
            ("file:/x", "file:///x"),
            ("http:///x/", "http://x/"),
            ("http://x:65535/", "http://x:65535/"),
            ("http://[fe80::1%25eth0]/", "http://[fe80::1%25eth0]/"),
            ("http://x/a/..", "http://x/"),
            ("http://x/..", "http://x/"),
            ("http://x/a/b/.", "http://x/a/b/"),
            ("http://a%41/", "http://aA/"),
            ("http://a.b./", "http://a.b./"),
            ("javascript://x", "javascript://x/"),
            ("http://example.invalid/str", "http://example.invalid/str"),
            // ports: leading zeros, any length
            ("http://x:000080/", "http://x:80/"),
            ("http://x:0000000000000000000000080/", "http://x:80/"),
            ("http://x:00000/", "http://x:0/"),
            // IPv6 zones: percent-decoded, "25" skipped, re-encoded
            ("http://[fe80::1%eth0]/", "http://[fe80::1%25eth0]/"),
            ("http://[fe80::1%25e%74h0]/", "http://[fe80::1%25eth0]/"),
            ("http://[fe80::1%2]/", "http://[fe80::1%252]/"),
            ("http://[::1%2x]/", "http://[::1%252x]/"),
            ("http://[::1%2525x]/", "http://[::1%25x]/"),
            ("http://[::1%2541]/", "http://[::1%2541]/"),
            ("http://[::1%25%41]/", "http://[::1%25A]/"),
            ("http://[::1%41]/", "http://[::1a]/"),
            ("http://[::1%25x-y.z_~]/", "http://[::1%25x-y.z_~]/"),
            (
                "http://[::1%25abcdefghijklmno]/",
                "http://[::1%25abcdefghijklmno]/",
            ),
            ("http://[%3a%3a1]/", "http://[::1]/"),
            // IPv6: inet_ntop form only when not longer than the input
            ("http://[::1.2.3.4]/", "http://[::1.2.3.4]/"),
            ("http://[::0.0.0.2]/", "http://[::2]/"),
            ("http://[::0.1.0.0]/", "http://[::0.1.0.0]/"),
            ("http://[::0.0.1.0]/", "http://[::100]/"),
            ("http://[::ffff:0:1]/", "http://[::ffff:0:1]/"),
            ("http://[::FFFF:0:1]/", "http://[::FFFF:0:1]/"),
            ("http://[::1:1]/", "http://[::1:1]/"),
            ("http://[::ABCD]/", "http://[::abcd]/"),
            ("http://[1:0:0:1:0:0:0:1]/", "http://[1:0:0:1::1]/"),
            ("http://[::2:3:4:5:6:7:8]/", "http://[0:2:3:4:5:6:7:8]/"),
            ("http://[64:ff9b::1.2.3.4]/", "http://[64:ff9b::102:304]/"),
            ("http://[0:0:0:0:0:0:0:1]/", "http://[::1]/"),
            ("http://[1:0:0::]/", "http://[1::]/"),
            ("http://[::1:0:0:0]/", "http://[::1:0:0:0]/"),
            ("http://[::ffff:0.0.0.0]/", "http://[::ffff:0.0.0.0]/"),
            // hostnames
            ("http://..x/", "http://..x/"),
            ("http://x..y/", "http://x..y/"),
            ("http://.x/", "http://.x/"),
            ("http://x~y/", "http://x~y/"),
            ("http://-x/", "http://-x/"),
            ("http://a%2eb/", "http://a.b/"),
            ("http://x%2e/", "http://x./"),
            // %2e counts as a dot in dot segments
            ("http://x/%2e%2e/a", "http://x/a"),
            ("http://x/a/%2E/b", "http://x/a/b"),
            ("http://x/a/%2e%2E/b", "http://x/b"),
            ("http://x/a/.%2e/b", "http://x/b"),
            ("http://x/a/%2e", "http://x/a/"),
            ("http://x/a/%2e.", "http://x/"),
            ("http://x/a%2fb/../c", "http://x/c"),
            ("http://x/%2e%2e%2f/a", "http://x/%2e%2e%2f/a"),
            ("http://x/a/.%2E%2E/b", "http://x/a/.%2E%2E/b"),
            ("http://x/a/..%2f/b", "http://x/a/..%2f/b"),
        ];
        for (u, want) in ok {
            assert_eq!(pre(u.as_bytes()).as_deref(), Some(*want), "{u}");
        }
        for u in [
            "Array",
            "12.5",
            "",
            "x.com",
            "example",
            "http://x/a b",
            "http://",
            "http://x:99999/",
            "http:x",
            "http://x\ty/",
            "http://a b/",
            "//x/",
            "mailto:a@b",
            "data:text/plain,hi",
            "http://x/?a=1#f g",
            "http://a@b@c/",
            "file://h/x",
            "http:////x/",
            "http://x:+1/",
            "http://[zz]/",
            "http://x:065536/",
            "http://[fe80::1%25]/",
            "http://[fe80::1%]/",
            "http://[fe80::1%25eth0%]/",
            "http://[::1%25x:y]/",
            "http://[::1%25abcdefghijklmnop]/",
            "http://[::1%25%2541]/",
            "http://[::1%ab%cd]/",
            "http://[::1]x/",
            "http://[::1/",
            "http://[v1.x]/",
            "http://[::ffff:1.2.3.04]/",
            "http://x../",
            "http://x.../",
            "http://x.y../",
            "http://./",
            "http://../",
            "http://x%2e./",
            "http://x|y/",
            "http://x\"y/",
            "http://x`y/",
            "http://x%7cy/",
            "http://x%00/",
            "http://x%2fy/",
            "http://x%20y/",
            "http://x/\x7f",
        ] {
            assert_eq!(pre(u.as_bytes()), None, "{u:?}");
        }
        // Non-ASCII bytes pass through untouched.
        assert_eq!(
            pre_transfer_url(b"http://x/a\x80b?\x81#\x82", false).as_deref(),
            Some(&b"http://x/a\x80b?\x81#\x82"[..])
        );
        assert_eq!(
            pre_transfer_url(b"http://x\x80/", false).as_deref(),
            Some(&b"http://x\x80/"[..])
        );
        assert_eq!(
            pre_transfer_url(b"http://[::1%25ab%cd]/", false).as_deref(),
            Some(&b"http://[::1%25ab\xcd]/"[..])
        );
    }

    #[test]
    fn path_as_is() {
        for (u, want) in [
            ("http://x/a/../b", "http://x/a/../b"),
            ("http://x", "http://x/"),
            ("http://x/%2e%2e/a", "http://x/%2e%2e/a"),
            ("http://x/./a/", "http://x/./a/"),
            ("http://x?q", "http://x/?q"),
            ("file:///a/../b", "file:///a/../b"),
            ("http://x/a/..", "http://x/a/.."),
        ] {
            let got = pre_transfer_url(u.as_bytes(), true).map(|v| String::from_utf8(v).unwrap());
            assert_eq!(got.as_deref(), Some(want), "{u}");
        }
    }

    /// The transfer's parse (scheme guessing): php's `curl_exec` effective
    /// URL, or its "URL rejected: …" error.
    #[test]
    fn transfer_parse() {
        let guess = Flags {
            guess_scheme: true,
            path_as_is: false,
        };
        let p = |u: &str| parse(u.as_bytes(), guess).map(|v| String::from_utf8(v).unwrap());
        for (u, want) in [
            ("Array", "http://Array/"),
            ("12.5", "http://12.0.0.5/"),
            ("x.com", "http://x.com/"),
            ("nope", "http://nope/"),
            ("mailto:a@b", "http://b/"),
            ("x:80/p", "http://x:80/p"),
            ("x/../a", "http://x/a"),
            ("x?q", "http://x/?q"),
            ("u:p@x", "http://x/"),
            ("[::1]:80", "http://[::1]:80/"),
            ("X/A/./b", "http://X/A/b"),
            ("http:/x:/", "http://x/"),
            ("HTTP:/X", "http://X/"),
            ("ftp:/x/a/../b", "ftp://x/b"),
            ("x://y", "x://y/"),
            ("x%41", "http://xA/"),
            ("FTP.x/y", "ftp://FTP.x/y"),
            ("dict.X", "dict://dict.X/"),
            ("pop3.x", "pop3://pop3.x/"),
            ("file:///a/../b", "file:///b"),
            ("HTTP://Nope:80", "http://Nope:80/"),
        ] {
            assert_eq!(p(u), Ok(want.to_string()), "{u}");
        }
        use UrlError::*;
        for (u, want) in [
            ("", Malformed),
            ("http://a/b c", BadPath),
            ("http://", NoHost),
            ("//x/", NoHost),
            ("http:/", NoHost),
            ("x:/", NoHost),
            ("http://x:99999/", BadPort),
            ("http:x", BadPort),
            ("data:text/plain,hi", BadPort),
            ("file:", BadPort),
            ("h\x01ttp://x/", BadPort),
            ("1http://x/", BadPort),
            ("http://[::1]x/", BadPort),
            ("http://x:1a/a b", BadPort),
            ("http://x\ty/", BadHostname),
            ("x y", BadHostname),
            ("http://a@b@c/", BadHostname),
            ("http://a b/c d", BadHostname),
            ("http://x/?a=1#f g", BadFragment),
            ("http://x/?a b#c d", BadFragment),
            ("http://x/a b?c d", BadQuery),
            ("http://x/\x01", BadPath),
            ("http://x/\x7f", BadPath),
            ("file:///a b", BadPath),
            ("file://h/x", BadFileUrl),
            ("file://x", BadFileUrl),
            ("file:/", BadFileUrl),
            ("http:////x/", BadSlashes),
            ("http://[zz]/", BadIpv6),
            ("http://[::1/a b", BadIpv6),
            ("http://u\x01@x/", BadUser),
            ("http://u @a b/", BadUser),
            ("http://u:p w@x/", BadPassword),
        ] {
            assert_eq!(p(u), Err(want), "{u:?}");
        }
        assert_eq!(
            BadPort.text(),
            "Port number was not a decimal number between 0 and 65535"
        );
    }
}
