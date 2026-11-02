//! What libcurl 8.23's `CURLINFO_EFFECTIVE_URL` reports before any transfer:
//! the `CURLOPT_URL` value parsed by its URL API (urlapi.c) and put back
//! together, or an error (php: `false`, and no `url` key in the
//! `curl_getinfo()` array) when the URL does not parse. Credentials are left
//! out; scheme and IPv4/IPv6 hosts are normalised, dot segments removed and an
//! empty path becomes `/`. Behaviour captured from the reference php's libcurl.

use std::net::Ipv6Addr;

/// The normalised URL, or `None` where libcurl's parser fails.
pub(super) fn pre_transfer_url(url: &str) -> Option<String> {
    let b = url.as_bytes();
    // junkscan + no CURLU_ALLOW_SPACE: no control bytes, no spaces.
    if b.iter().any(|&c| c <= b' ' || c == 0x7f) {
        return None;
    }
    let colon = url.find(':')?;
    let scheme = &url[..colon];
    let sb = scheme.as_bytes();
    if sb.is_empty()
        || sb.len() > 40
        || !sb[0].is_ascii_alphabetic()
        || !sb[1..]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
    {
        return None;
    }
    let scheme = scheme.to_ascii_lowercase();
    let rest = &url[colon + 1..];
    if scheme == "file" {
        return file_url(rest);
    }
    let slashes = rest.bytes().take_while(|&c| c == b'/').count();
    if !(1..=3).contains(&slashes) {
        return None;
    }
    let rest = &rest[slashes..];
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    // Credentials are parsed off and not reported.
    let hostport = match authority.find('@') {
        Some(i) => &authority[i + 1..],
        None => authority,
    };
    let (host, port) = split_port(hostport)?;
    let host = normalise_host(host)?;
    let mut out = format!("{scheme}://{host}");
    if let Some(p) = port {
        out.push(':');
        out.push_str(&p.to_string());
    }
    out.push_str(&path_query_fragment(tail));
    Some(out)
}

/// `file:` URLs: no host but `localhost`/`127.0.0.1` (POSIX libcurl).
fn file_url(rest: &str) -> Option<String> {
    let path = match rest.strip_prefix("//") {
        Some(r) if r.starts_with('/') => r,
        Some(r) => {
            let slash = r.find('/')?;
            let host = &r[..slash];
            if !host.eq_ignore_ascii_case("localhost") && host != "127.0.0.1" {
                return None;
            }
            &r[slash..]
        }
        None => rest,
    };
    if !path.starts_with('/') {
        return None;
    }
    Some(format!("file://{}", path_query_fragment(path)))
}

/// `host[:port]` → host and the port (`None` when absent or empty).
fn split_port(hp: &str) -> Option<(&str, Option<u32>)> {
    let after_host = if hp.starts_with('[') {
        hp.find(']')? + 1
    } else {
        hp.find(':').unwrap_or(hp.len())
    };
    let (host, rest) = hp.split_at(after_host);
    if rest.is_empty() {
        return Some((host, None));
    }
    let digits = rest.strip_prefix(':')?;
    if digits.is_empty() {
        return Some((host, None));
    }
    if !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let p: u32 = digits.trim_start_matches('0').parse().unwrap_or(0);
    if digits.len() > 5 || p > 65535 {
        return None;
    }
    Some((host, Some(p)))
}

fn normalise_host(host: &str) -> Option<String> {
    if host.is_empty() {
        return None;
    }
    if let Some(inner) = host.strip_prefix('[') {
        let inner = inner.strip_suffix(']')?;
        let (addr, zone) = match inner.find('%') {
            Some(i) => (&inner[..i], Some(&inner[i..])),
            None => (inner, None),
        };
        let a: Ipv6Addr = addr.parse().ok()?;
        return Some(format!("[{a}{}]", zone.unwrap_or("")));
    }
    let host = percent_decode(host)?;
    if let Some(v4) = ipv4(&host) {
        return Some(v4);
    }
    const BAD: &[u8] = b" \r\n\t/:#?!@{}[]\\$'\"^`*<>=;,+&()%";
    if host.bytes().any(|c| BAD.contains(&c)) {
        return None;
    }
    Some(host)
}

fn percent_decode(s: &str) -> Option<String> {
    if !s.contains('%') {
        return Some(s.to_string());
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// curl's `ipv4_normalize`: one to four dot-separated numbers (decimal,
/// `0x` hex or `0` octal), the last filling the remaining bytes.
fn ipv4(host: &str) -> Option<String> {
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

/// The path (dot segments removed, `/` when empty), query and fragment.
fn path_query_fragment(tail: &str) -> String {
    let end = tail.find(['?', '#']).unwrap_or(tail.len());
    let (path, qf) = tail.split_at(end);
    let mut path = remove_dot_segments(path);
    if path.is_empty() {
        path.push('/');
    }
    path + qf
}

/// RFC 3986 section 5.2.4.
fn remove_dot_segments(path: &str) -> String {
    let mut input = path;
    let mut out: Vec<&str> = Vec::new();
    while !input.is_empty() {
        if let Some(r) = input.strip_prefix("../") {
            input = r;
        } else if let Some(r) = input.strip_prefix("./") {
            input = r;
        } else if input.starts_with("/./") {
            input = &input[2..];
        } else if input == "/." {
            input = "/";
        } else if input.starts_with("/../") {
            input = &input[3..];
            out.pop();
        } else if input == "/.." {
            input = "/";
            out.pop();
        } else if input == "." || input == ".." {
            input = "";
        } else {
            let start = usize::from(input.starts_with('/'));
            let seg_end = input[start..].find('/').map_or(input.len(), |i| i + start);
            out.push(&input[..seg_end]);
            input = &input[seg_end..];
        }
    }
    out.concat()
}

#[cfg(test)]
mod tests {
    use super::pre_transfer_url as n;

    /// Cases captured from php 8.5.12 (libcurl 8.23.0):
    /// `curl_setopt($h, CURLOPT_URL, $u); curl_getinfo($h, CURLINFO_EFFECTIVE_URL)`.
    #[test]
    fn matches_libcurl() {
        let ok = [
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
        ];
        for (u, want) in ok {
            assert_eq!(n(u).as_deref(), Some(want), "{u}");
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
        ] {
            assert_eq!(n(u), None, "{u:?}");
        }
    }
}
