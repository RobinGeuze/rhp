//! sapi/cgi/cgi_main.c: `apache_request_headers` (`getallheaders`),
//! `apache_response_headers` and `apache_child_terminate`.

use rhp_values::{ArrayKey, PhpArray, Thrown, Value};

use crate::Host;

fn fn_apache_request_headers(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    for (k, v) in host.request_headers() {
        a.insert(ArrayKey::from_bytes(&k), Value::str(v));
    }
    Ok(Value::Array(a))
}

/// php-cgi's `apache_child_terminate`: only meaningful under FastCGI.
fn fn_apache_child_terminate(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Null)
}

fn fn_apache_response_headers(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    if let Some(r) = host.response() {
        for h in &r.headers {
            let Some(colon) = h.iter().position(|&b| b == b':') else {
                continue;
            };
            let mut len = colon;
            while len > 0 && matches!(h[len - 1], b' ' | b'\t') {
                len -= 1;
            }
            if len == 0 {
                continue;
            }
            let mut p = colon + 1;
            while p < h.len() && matches!(h[p], b' ' | b'\t') {
                p += 1;
            }
            a.insert(ArrayKey::from_bytes(&h[..len]), Value::str(h[p..].to_vec()));
        }
    }
    Ok(Value::Array(a))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("getallheaders", fn_apache_request_headers),
            ("apache_request_headers", fn_apache_request_headers),
            ("apache_response_headers", fn_apache_response_headers),
            ("apache_child_terminate", fn_apache_child_terminate),
        ],
    );
}
