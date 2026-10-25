//! Minimal synchronous HTTP(S) client for the `http://` / `https://` stream
//! paths (`file_get_contents($url, false, $context)` — Composer's non-curl
//! transport). Backed by `ureq` (rustls TLS with bundled Mozilla roots).

use std::io::Read;
use std::time::Duration;

use rhp_values::{conv, ArrayKey, Value};

use crate::Host;

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

/// `file_get_contents()` over http(s): read the request shape from the stream
/// context's `http` options, perform the request, publish the response headers
/// via the host (for `http_get_last_response_headers`), and return the body
/// (or `false` on a transport error).
pub fn fetch(host: &mut dyn Host, url: &[u8], ctx: &Value) -> Value {
    let url_s = String::from_utf8_lossy(url).into_owned();

    // Pull the `http` sub-array from the context options.
    let opts = host.stream_context_options(ctx);
    let http_opts = match &opts {
        Value::Array(a) => match a.get(&ArrayKey::from_bytes(b"http")) {
            Some(Value::Array(h)) => Some(h.clone()),
            _ => None,
        },
        _ => None,
    };

    let mut method = "GET".to_string();
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut body: Option<Vec<u8>> = None;
    let mut follow = true;
    let mut timeout = None;

    if let Some(h) = &http_opts {
        if let Some(m) = h.get(&ArrayKey::from_bytes(b"method")) {
            let m = conv::to_string_bytes(&deref(m));
            if !m.is_empty() {
                method = String::from_utf8_lossy(&m).into_owned();
            }
        }
        if let Some(hdr) = h.get(&ArrayKey::from_bytes(b"header")) {
            // May be a string ("A: b\r\nC: d") or an array of lines.
            let lines: Vec<Vec<u8>> = match deref(hdr) {
                Value::Array(a) => a.iter().map(|(_, v)| conv::to_string_bytes(v)).collect(),
                other => conv::to_string_bytes(&other)
                    .split(|&b| b == b'\n')
                    .map(|l| l.strip_suffix(b"\r").unwrap_or(l).to_vec())
                    .collect(),
            };
            for line in lines {
                if line.is_empty() {
                    continue;
                }
                if let Some(c) = line.iter().position(|&b| b == b':') {
                    let name = String::from_utf8_lossy(&line[..c]).trim().to_string();
                    let value = String::from_utf8_lossy(&line[c + 1..]).trim().to_string();
                    if !name.is_empty() {
                        headers.push((name, value));
                    }
                }
            }
        }
        if let Some(c) = h.get(&ArrayKey::from_bytes(b"content")) {
            let b = conv::to_string_bytes(&deref(c));
            if !b.is_empty() {
                body = Some(b);
            }
        }
        if let Some(f) = h.get(&ArrayKey::from_bytes(b"follow_location")) {
            follow = conv::to_int(&deref(f)) != 0;
        }
        if let Some(t) = h.get(&ArrayKey::from_bytes(b"timeout")) {
            let secs = conv::to_float(&deref(t));
            if secs > 0.0 {
                timeout = Some(secs.ceil() as u64);
            }
        }
    }

    match request(&method, &url_s, &headers, body.as_deref(), follow, timeout) {
        Ok(resp) => {
            host.set_http_response_headers(resp.headers);
            Value::str(resp.body)
        }
        Err(_) => {
            host.set_http_response_headers(Vec::new());
            Value::Bool(false)
        }
    }
}

/// A completed HTTP response: php-style header lines (status line first) + body.
pub struct HttpResponse {
    pub headers: Vec<Vec<u8>>,
    pub body: Vec<u8>,
}

/// Perform one HTTP request. `headers` are raw "Name: value" lines. Returns the
/// response (including for 4xx/5xx — Composer reads error bodies); `Err` only on
/// a transport failure (DNS, connect, TLS).
pub fn request(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
    follow: bool,
    timeout_secs: Option<u64>,
) -> Result<HttpResponse, String> {
    let mut builder = ureq::AgentBuilder::new().redirects(if follow { 10 } else { 0 });
    if let Some(t) = timeout_secs {
        builder = builder.timeout(Duration::from_secs(t));
    }
    let agent = builder.build();

    let mut req = agent.request(method, url);
    for (name, value) in headers {
        req = req.set(name, value);
    }
    let result = match body {
        Some(b) => req.send_bytes(b),
        None => req.call(),
    };

    let resp = match result {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r, // HTTP error status: still a response
        Err(ureq::Error::Transport(t)) => return Err(t.to_string()),
    };

    let mut lines: Vec<Vec<u8>> = Vec::new();
    lines.push(format!("HTTP/1.1 {} {}", resp.status(), resp.status_text()).into_bytes());
    for name in resp.headers_names() {
        if let Some(v) = resp.header(&name) {
            lines.push(format!("{}: {}", name, v).into_bytes());
        }
    }

    let mut body = Vec::new();
    // Cap at a generous size to avoid unbounded memory on a runaway response.
    resp.into_reader()
        .take(256 * 1024 * 1024)
        .read_to_end(&mut body)
        .map_err(|e| e.to_string())?;

    Ok(HttpResponse { headers: lines, body })
}
