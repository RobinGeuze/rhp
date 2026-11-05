//! ext/standard/http.c: `http_build_query`, `request_parse_body` and the
//! last response headers of the http:// wrapper.

use rhp_values::ops::deref;
use rhp_values::{conv, fmt, ArrayKey, PhpArray, Value};

use crate::main::ini::quantity_error;
use crate::main::php_variables::post_form_vars;
use crate::main::rfc1867::{
    actual_length_exceeds, content_type_dup, cstr, media_type, parse, BodyOptions, Diag, Limits,
};
use crate::zend::zpp::{int_param, str_or_null_param, str_param, zval_value_name};
use crate::{Host, Thrown};

const HEX: &[u8; 16] = b"0123456789ABCDEF";
const PHP_QUERY_RFC1738: i64 = 1;
const PHP_QUERY_RFC3986: i64 = 2;

/// `http_build_query`'s encodings.
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    vec![
        ("PHP_QUERY_RFC1738", Value::Int(PHP_QUERY_RFC1738)),
        ("PHP_QUERY_RFC3986", Value::Int(PHP_QUERY_RFC3986)),
    ]
}

fn url_encode_into(out: &mut Vec<u8>, s: &[u8], raw: bool) {
    for &b in s {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(b),
            b'~' if raw => out.push(b),
            b' ' if !raw => out.push(b'+'),
            _ => {
                out.push(b'%');
                out.push(HEX[(b >> 4) as usize]);
                out.push(HEX[(b & 0xf) as usize]);
            }
        }
    }
}

pub(crate) fn url_encode(s: &[u8], raw: bool) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len());
    url_encode_into(&mut v, s, raw);
    v
}

struct Enc<'a> {
    out: Vec<u8>,
    sep: &'a [u8],
    raw: bool,
}

/// Is this value an enum case object?
fn enum_class(host: &dyn Host, v: &Value) -> bool {
    match v {
        Value::Object(h) => host.is_enum_class(h.borrow().class),
        _ => false,
    }
}

/// One hash level of `php_url_encode_hash_ex`. `entries` is the (key, value)
/// list of an array or an object's visible properties.
fn encode_hash(
    host: &mut dyn Host,
    e: &mut Enc,
    entries: Vec<(ArrayKey, Value)>,
    num_prefix: Option<&[u8]>,
    key_prefix: Option<&[u8]>,
    depth: usize,
) -> Result<(), Thrown> {
    if depth > 256 {
        // php guards self-recursive arrays (GC_IS_RECURSIVE); stop descending.
        return Ok(());
    }
    for (key, v) in entries {
        let v = deref(&v);
        let nested = match &v {
            Value::Array(_) => true,
            Value::Object(_) => !enum_class(host, &v),
            _ => false,
        };
        if nested {
            let new_prefix = match &key {
                ArrayKey::Str(k) => {
                    let ek = url_encode(k.as_bytes(), e.raw);
                    match key_prefix {
                        Some(kp) => [kp, &ek, b"%5D%5B"].concat(),
                        None => [&ek[..], b"%5B"].concat(),
                    }
                }
                ArrayKey::Int(i) => {
                    let is = i.to_string().into_bytes();
                    match (key_prefix, num_prefix) {
                        (Some(kp), Some(np)) => [kp, np, &is, b"%5D%5B"].concat(),
                        (Some(kp), None) => [kp, &is, b"%5D%5B"].concat(),
                        (None, Some(np)) => [np, &is, b"%5B"].concat(),
                        (None, None) => [&is[..], b"%5B"].concat(),
                    }
                }
            };
            let sub = entries_of(host, &v);
            encode_hash(host, e, sub, None, Some(&new_prefix), depth + 1)?;
            continue;
        }
        if matches!(v, Value::Null | Value::Resource(_)) {
            continue;
        }
        // php_url_encode_scalar
        let mut part = Vec::new();
        if let Some(kp) = key_prefix {
            part.extend_from_slice(kp);
        }
        match &key {
            ArrayKey::Str(k) => url_encode_into(&mut part, k.as_bytes(), e.raw),
            ArrayKey::Int(i) => {
                if let Some(np) = num_prefix {
                    part.extend_from_slice(np);
                }
                part.extend_from_slice(i.to_string().as_bytes());
            }
        }
        if key_prefix.is_some() {
            part.extend_from_slice(b"%5D");
        }
        part.push(b'=');
        let mut val = v.clone();
        if enum_class(host, &val) {
            match host.object_prop(&val, b"value") {
                Some(bv) => val = bv,
                None => {
                    let cname = zval_value_name(host, &val);
                    return Err(Thrown::error(
                        "ValueError",
                        format!("Unbacked enum {cname} cannot be converted to a string"),
                    ));
                }
            }
        }
        match &val {
            Value::Str(s) => url_encode_into(&mut part, s.as_bytes(), e.raw),
            Value::Int(i) => part.extend_from_slice(i.to_string().as_bytes()),
            Value::Float(f) => url_encode_into(&mut part, &fmt::float_echo(*f), e.raw),
            Value::Bool(b) => part.push(if *b { b'1' } else { b'0' }),
            other => url_encode_into(&mut part, &conv::to_string_bytes(other), e.raw),
        }
        if !e.out.is_empty() {
            e.out.extend_from_slice(e.sep);
        }
        e.out.extend_from_slice(&part);
    }
    Ok(())
}

fn entries_of(host: &dyn Host, v: &Value) -> Vec<(ArrayKey, Value)> {
    match v {
        Value::Array(a) => a.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Value::Object(_) => host
            .object_scoped_props(v)
            .into_iter()
            .map(|(k, v)| (ArrayKey::Str(k), v))
            .collect(),
        _ => Vec::new(),
    }
}

fn fn_http_build_query(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let data = deref(&args[0]);
    match &data {
        Value::Array(_) => {}
        Value::Object(_) if enum_class(host, &data) => {
            return Err(Thrown::type_error(format!(
                "http_build_query(): Argument #1 ($data) must not be an enum, {} given",
                zval_value_name(host, &data)
            )));
        }
        Value::Object(_) => {}
        other => {
            return Err(Thrown::type_error(format!(
                "http_build_query(): Argument #1 ($data) must be of type array, {} given",
                zval_value_name(host, other)
            )));
        }
    }
    let prefix = match args.get(1) {
        Some(_) => str_param(host, args, 1, "http_build_query", "numeric_prefix")?,
        None => Vec::new(),
    };
    // A null separator falls back to `arg_separator.output` (never empty).
    let sep = str_or_null_param(host, args, 2, "http_build_query", "arg_separator")?
        .unwrap_or_else(|| {
            crate::main::ini::ini_value(b"arg_separator.output")
                .flatten()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| b"&".to_vec())
        });
    let enc = int_param(host, args, 3, "http_build_query", "encoding_type", 1)?;
    let mut e = Enc {
        out: Vec::new(),
        sep: &sep,
        raw: enc == PHP_QUERY_RFC3986,
    };
    let entries = entries_of(host, &data);
    let np = (!prefix.is_empty()).then_some(&prefix[..]);
    encode_hash(host, &mut e, entries, np, None, 0)?;
    Ok(Value::str(e.out))
}

/// php's `BG(last_http_headers)`: the response header lines (status line
/// first) of every hop of the last http(s) request; `None` before one.
#[derive(Default)]
struct LastHttpHeaders(Option<Vec<Vec<u8>>>);

fn last_http_headers(host: &mut dyn Host) -> Option<&mut Option<Vec<Vec<u8>>>> {
    host.module_state("last_http_headers", || Box::<LastHttpHeaders>::default())
        .and_then(|st| st.downcast_mut::<LastHttpHeaders>())
        .map(|st| &mut st.0)
}

/// Replace the request's [`LastHttpHeaders`].
pub(crate) fn set_last_http_headers(host: &mut dyn Host, headers: Option<Vec<Vec<u8>>>) {
    if let Some(slot) = last_http_headers(host) {
        *slot = headers;
    }
}

fn fn_http_get_last_response_headers(
    host: &mut dyn Host,
    _args: &[Value],
) -> Result<Value, Thrown> {
    let Some(headers) = last_http_headers(host).and_then(|h| h.clone()) else {
        return Ok(Value::Null);
    };
    let mut a = rhp_values::PhpArray::new();
    for h in headers {
        a.push(Value::str(h));
    }
    Ok(Value::Array(a))
}

fn fn_http_clear_last_response_headers(
    host: &mut dyn Host,
    _args: &[Value],
) -> Result<Value, Thrown> {
    set_last_http_headers(host, None);
    Ok(Value::Null)
}

/// `cache_request_parse_body_options`.
fn parse_options(host: &mut dyn Host, opts: &PhpArray) -> Result<BodyOptions, Thrown> {
    let mut o = BodyOptions::default();
    for (k, v) in opts.iter() {
        let ArrayKey::Str(k) = k else {
            return Err(Thrown::error(
                "ValueError",
                "Invalid integer key in $options argument",
            ));
        };
        let k = k.as_bytes();
        if k.is_empty() {
            return Err(Thrown::error(
                "ValueError",
                "Invalid empty string key in $options argument",
            ));
        }
        let lk = k.to_ascii_lowercase();
        let slot = match &lk[..] {
            b"max_file_uploads" => &mut o.max_file_uploads,
            b"max_input_vars" => &mut o.max_input_vars,
            b"max_multipart_body_parts" => &mut o.max_multipart_body_parts,
            b"post_max_size" => &mut o.post_max_size,
            b"upload_max_filesize" => &mut o.upload_max_filesize,
            _ => {
                return Err(Thrown::error(
                    "ValueError",
                    format!(
                        "Invalid key \"{}\" in $options argument",
                        String::from_utf8_lossy(cstr(k))
                    ),
                ))
            }
        };
        let v = deref(v);
        *slot = Some(match &v {
            Value::Int(i) => *i,
            Value::Str(s) => {
                if let Some(err) = quantity_error(s.as_bytes()) {
                    host.emit_diagnostic(crate::E_WARNING, err.as_bytes())?;
                }
                crate::ini_parse_quantity(s.as_bytes())
            }
            other => {
                return Err(Thrown::error(
                    "ValueError",
                    format!(
                        "Invalid {} value in $options argument",
                        crate::zend::zpp::zval_value_name(host, other)
                    ),
                ))
            }
        });
    }
    Ok(o)
}

/// `new RequestParseBodyException($msg)`.
fn parse_body_exception(host: &mut dyn Host, msg: String) -> Thrown {
    match host.reflect_new_instance(
        b"RequestParseBodyException",
        vec![Value::str(msg.clone().into_bytes())],
        true,
    ) {
        Ok(Value::Object(h)) => Thrown::Object(h),
        Ok(_) => Thrown::error("RequestParseBodyException", msg),
        Err(t) => t,
    }
}

fn fn_request_parse_body(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let opts = match args.first().map(deref) {
        None | Some(Value::Null) => BodyOptions::default(),
        Some(Value::Array(a)) => parse_options(host, &a)?,
        Some(other) => {
            return Err(Thrown::type_error(format!(
                "request_parse_body(): Argument #1 ($options) must be of type ?array, {} given",
                crate::zend::zpp::zval_value_name(host, &other)
            )))
        }
    };
    let info = host.request_info().and_then(|r| {
        r.content_type.clone().map(|ct| {
            (
                ct,
                r.method.clone(),
                r.content_length.unwrap_or(r.body.len() as i64),
                r.body.clone(),
            )
        })
    });
    let Some((ct, method, content_length, body)) = info else {
        return Err(parse_body_exception(
            host,
            "Request does not provide a content type".into(),
        ));
    };
    let is_post = method.as_deref() == Some(b"POST");
    let kind = media_type(&ct);
    let lim = Limits::new(host, &opts);
    let mut post = PhpArray::new();
    let mut files = PhpArray::new();
    // sapi_read_standard_form_data.
    let read_standard = |host: &mut dyn Host| -> Result<bool, Thrown> {
        if lim.post_max_size > 0 && content_length > lim.post_max_size {
            crate::warn(
                host,
                "request_parse_body",
                &format!(
                    "POST Content-Length of {content_length} bytes exceeds the limit of {} bytes",
                    lim.post_max_size
                ),
            )?;
            return Ok(false);
        }
        let over = host
            .body_state()
            .is_some_and(|s| s.read_standard(&body, lim.post_max_size));
        if over {
            crate::warn(
                host,
                "request_parse_body",
                &actual_length_exceeds(lim.post_max_size),
            )?;
        }
        Ok(true)
    };
    match &kind[..] {
        b"application/x-www-form-urlencoded" => {
            if read_standard(host)? {
                let data = host
                    .body_state()
                    .and_then(|s| s.buffered())
                    .unwrap_or_default();
                let (a, over) = post_form_vars(&data, lim.max_input_vars);
                post = a;
                if over {
                    crate::warn(
                        host,
                        "request_parse_body",
                        &format!(
                            "Input variables exceeded {}. To increase the limit change max_input_vars in php.ini.",
                            lim.max_input_vars as u64
                        ),
                    )?;
                }
            }
        }
        b"multipart/form-data" => {
            let (rest, read_before) = match host.body_state() {
                Some(s) => (s.remaining(&body).to_vec(), s.read_bytes()),
                None => (Vec::new(), 0),
            };
            let mut out = parse(
                &rest,
                read_before,
                &content_type_dup(&ct),
                content_length,
                &lim,
                &mut post,
                &mut files,
            );
            if let Some(s) = host.body_state() {
                s.apply(&mut out);
            }
            let mut thrown: Option<Thrown> = None;
            for d in out.diags {
                match d {
                    Diag::Emit(m) => {
                        if thrown.is_none() {
                            thrown = Some(parse_body_exception(host, m));
                        }
                    }
                    Diag::Notice(m) => crate::notice(host, "request_parse_body", &m)?,
                    Diag::Sapi(m) => host.emit_diagnostic(crate::E_WARNING, m.as_bytes())?,
                }
            }
            if let Some(t) = thrown {
                return Err(t);
            }
        }
        _ => {
            if is_post {
                read_standard(host)?;
            }
            return Err(parse_body_exception(
                host,
                format!(
                    "Content-Type \"{}\" is not supported",
                    String::from_utf8_lossy(cstr(&ct))
                ),
            ));
        }
    }
    let mut pair = PhpArray::new();
    pair.push(Value::Array(post));
    pair.push(Value::Array(files));
    Ok(Value::Array(pair))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("http_build_query", fn_http_build_query),
            (
                "http_get_last_response_headers",
                fn_http_get_last_response_headers,
            ),
            (
                "http_clear_last_response_headers",
                fn_http_clear_last_response_headers,
            ),
            ("request_parse_body", fn_request_parse_body),
        ],
    );
}
