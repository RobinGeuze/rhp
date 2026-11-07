//! ext/standard/head.c: `header`, `headers_sent`, `headers_list`,
//! `http_response_code` and `setcookie`, over the SAPI layer's header
//! operations ([`crate::main::sapi`]).

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Thrown, Value};

use crate::main::sapi::{header_op, started_at, HeaderOp};
use crate::zend::zpp::{bool_param, int_param, str_or_null_param, str_param};
use crate::Host;

// ---------------------------------------------------------------------------
// builtins
// ---------------------------------------------------------------------------

fn fn_header(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let h = str_param(host, args, 0, "header", "header")?;
    let replace = bool_param(host, args, 1, "header", "replace", true)?;
    let code = int_param(host, args, 2, "header", "response_code", 0)?;
    let op = if replace {
        HeaderOp::Replace(&h, code)
    } else {
        HeaderOp::Add(&h, code)
    };
    header_op(host, op)?;
    Ok(Value::Null)
}

fn fn_header_remove(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_or_null_param(host, args, 0, "header_remove", "name")?;
    match &name {
        None => header_op(host, HeaderOp::DeleteAll)?,
        Some(n) => header_op(host, HeaderOp::Delete(n))?,
    };
    Ok(Value::Null)
}

fn fn_headers_sent(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (sent, file, line) = match host.response() {
        Some(r) if r.sent => {
            let (f, l) = r.output_start.clone().unwrap_or_default();
            (true, f, l)
        }
        _ => (false, Vec::new(), 0),
    };
    // php assigns the line first.
    let r = crate::zend::zpp::try_assign_ref(host, args, 1, Value::Int(line));
    let f = crate::zend::zpp::try_assign_ref(host, args, 0, Value::str(file));
    r.and(f)?;
    Ok(Value::Bool(sent))
}

fn fn_headers_list(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    if let Some(r) = host.response() {
        for h in &r.headers {
            a.push(Value::str(h.clone()));
        }
    }
    Ok(Value::Array(a))
}

fn fn_http_response_code(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = int_param(host, args, 0, "http_response_code", "response_code", 0)?;
    let Some(r) = host.response() else {
        return Ok(Value::Bool(false));
    };
    if code != 0 {
        if r.sent && !r.no_headers {
            let msg = match started_at(r) {
                Some(at) => {
                    format!(
                        "Cannot set response code - headers already sent (output started at {at})"
                    )
                }
                None => "Cannot set response code - headers already sent".to_string(),
            };
            crate::warn(host, "http_response_code", &msg)?;
            return Ok(Value::Bool(false));
        }
        if r.status_line.is_some() {
            crate::warn(
                host,
                "http_response_code",
                "Calling http_response_code() after header('HTTP/...') has no effect",
            )?;
        }
        let Some(r) = host.response() else {
            return Ok(Value::Bool(false));
        };
        let old = r.code;
        r.code = code;
        return Ok(if old != 0 {
            Value::Int(old)
        } else {
            Value::Bool(true)
        });
    }
    Ok(if r.code == 0 {
        Value::Bool(false)
    } else {
        Value::Int(r.code)
    })
}

const ILLEGAL_COOKIE: &str = r#""=", ",", ";", " ", "\t", "\r", "\n", "\013", or "\014""#;
const ILLEGAL_COOKIE_VALUE: &str = r#"",", ";", " ", "\t", "\r", "\n", "\013", or "\014""#;

fn has_cookie_illegal(s: &[u8], with_eq: bool) -> bool {
    s.iter().any(|&b| {
        matches!(b, b',' | b';' | b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c)
            || (with_eq && b == b'=')
    })
}

/// The cookie attributes `setcookie` takes positionally or as options.
#[derive(Default)]
struct CookieOpts {
    expires: i64,
    path: Option<Vec<u8>>,
    domain: Option<Vec<u8>>,
    secure: bool,
    httponly: bool,
    samesite: Option<Vec<u8>>,
    partitioned: bool,
}

/// `php_setcookie`.
fn php_setcookie(
    host: &mut dyn Host,
    func: &str,
    name: &[u8],
    value: Option<&[u8]>,
    o: &CookieOpts,
    url_encode: bool,
) -> Result<bool, Thrown> {
    // C strings: everything after a NUL is invisible to strpbrk.
    let cstr = |s: &[u8]| -> Vec<u8> { s.iter().take_while(|&&b| b != 0).copied().collect() };
    if name.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            format!("{func}(): Argument #1 ($name) must not be empty"),
        ));
    }
    if has_cookie_illegal(&cstr(name), true) {
        return Err(Thrown::error(
            "ValueError",
            format!("{func}(): Argument #1 ($name) cannot contain {ILLEGAL_COOKIE}"),
        ));
    }
    if !url_encode {
        if let Some(v) = value {
            if has_cookie_illegal(&cstr(v), false) {
                return Err(Thrown::error(
                    "ValueError",
                    format!("{func}(): Argument #2 ($value) cannot contain {ILLEGAL_COOKIE_VALUE}"),
                ));
            }
        }
    }
    if o.path
        .as_deref()
        .is_some_and(|p| has_cookie_illegal(&cstr(p), false))
    {
        return Err(Thrown::error(
            "ValueError",
            format!("{func}(): \"path\" option cannot contain {ILLEGAL_COOKIE_VALUE}"),
        ));
    }
    if o.domain
        .as_deref()
        .is_some_and(|p| has_cookie_illegal(&cstr(p), false))
    {
        return Err(Thrown::error(
            "ValueError",
            format!("{func}(): \"domain\" option cannot contain {ILLEGAL_COOKIE_VALUE}"),
        ));
    }
    if o.expires >= 253_402_300_800 {
        return Err(Thrown::error(
            "ValueError",
            format!("{func}(): \"expires\" option cannot have a year greater than 9999"),
        ));
    }
    if o.partitioned && !o.secure {
        return Err(Thrown::error(
            "ValueError",
            format!("{func}(): \"partitioned\" option cannot be used without \"secure\" option"),
        ));
    }
    let mut buf = b"Set-Cookie: ".to_vec();
    match value.filter(|v| !v.is_empty()) {
        None => {
            buf.extend_from_slice(name);
            buf.extend_from_slice(b"=deleted; expires=Thu, 01 Jan 1970 00:00:01 GMT; Max-Age=0");
        }
        Some(v) => {
            buf.extend_from_slice(name);
            buf.push(b'=');
            if url_encode {
                buf.extend_from_slice(&crate::ext::standard::http::url_encode(v, true));
            } else {
                buf.extend_from_slice(v);
            }
            if o.expires > 0 {
                buf.extend_from_slice(b"; expires=");
                buf.extend_from_slice(&crate::ext::date::php_format_date(
                    b"D, d M Y H:i:s \\G\\M\\T",
                    o.expires,
                    false,
                ));
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                buf.extend_from_slice(b"; Max-Age=");
                buf.extend_from_slice((o.expires - now).max(0).to_string().as_bytes());
            }
        }
    }
    let attr = |buf: &mut Vec<u8>, k: &[u8], v: &Option<Vec<u8>>| {
        if let Some(v) = v.as_deref().filter(|v| !v.is_empty()) {
            buf.extend_from_slice(k);
            buf.extend_from_slice(v);
        }
    };
    attr(&mut buf, b"; path=", &o.path);
    attr(&mut buf, b"; domain=", &o.domain);
    if o.secure {
        buf.extend_from_slice(b"; secure");
    }
    if o.httponly {
        buf.extend_from_slice(b"; HttpOnly");
    }
    attr(&mut buf, b"; SameSite=", &o.samesite);
    if o.partitioned {
        buf.extend_from_slice(b"; Partitioned");
    }
    header_op(host, HeaderOp::Add(&buf, 0))
}

fn setcookie_common(
    host: &mut dyn Host,
    args: &[Value],
    func: &str,
    raw: bool,
) -> Result<Value, Thrown> {
    let name = str_param(host, args, 0, func, "name")?;
    let value = match args.get(1) {
        None => None,
        Some(_) => Some(str_param(host, args, 1, func, "value")?),
    };
    let mut o = CookieOpts::default();
    match args.get(2).map(deref) {
        Some(Value::Array(opts)) => {
            if args.len() > 3 {
                return Err(Thrown::error(
                    "ArgumentCountError",
                    format!("{func}(): Expects exactly 3 arguments when argument #3 ($expires_or_options) is an array"),
                ));
            }
            for (k, v) in opts.iter() {
                let ArrayKey::Str(k) = k else {
                    return Err(Thrown::error(
                        "ValueError",
                        format!("{func}(): option array cannot have numeric keys"),
                    ));
                };
                let v = deref(v);
                let kb = k.as_bytes();
                if kb.eq_ignore_ascii_case(b"expires") {
                    o.expires = conv::to_int(&v);
                } else if kb.eq_ignore_ascii_case(b"path") {
                    o.path = Some(conv::to_string_bytes(&v));
                } else if kb.eq_ignore_ascii_case(b"domain") {
                    o.domain = Some(conv::to_string_bytes(&v));
                } else if kb.eq_ignore_ascii_case(b"secure") {
                    o.secure = conv::to_bool(&v);
                } else if kb.eq_ignore_ascii_case(b"httponly") {
                    o.httponly = conv::to_bool(&v);
                } else if kb.eq_ignore_ascii_case(b"samesite") {
                    o.samesite = Some(conv::to_string_bytes(&v));
                } else if kb.eq_ignore_ascii_case(b"partitioned") {
                    o.partitioned = conv::to_bool(&v);
                } else {
                    return Err(Thrown::error(
                        "ValueError",
                        format!(
                            "{func}(): option \"{}\" is invalid",
                            String::from_utf8_lossy(kb)
                        ),
                    ));
                }
            }
        }
        Some(Value::Object(_) | Value::Resource(_) | Value::Closure(_)) => {
            return Err(Thrown::type_error(format!(
                "{func}(): Argument #3 ($expires_or_options) must be of type array|int, {} given",
                crate::zend::zpp::zval_value_name(host, &deref(&args[2]))
            )));
        }
        Some(_) => {
            o.expires = int_param(host, args, 2, func, "expires_or_options", 0)?;
        }
        None => {}
    }
    if args.len() > 3 {
        o.path = Some(str_param(host, args, 3, func, "path")?);
    }
    if args.len() > 4 {
        o.domain = Some(str_param(host, args, 4, func, "domain")?);
    }
    o.secure = o.secure || bool_param(host, args, 5, func, "secure", false)?;
    o.httponly = o.httponly || bool_param(host, args, 6, func, "httponly", false)?;
    let ok = php_setcookie(host, func, &name, value.as_deref(), &o, !raw)?;
    Ok(Value::Bool(ok))
}

fn fn_setcookie(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    setcookie_common(host, args, "setcookie", false)
}

fn fn_setrawcookie(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    setcookie_common(host, args, "setrawcookie", true)
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("header", fn_header),
            ("header_remove", fn_header_remove),
            ("headers_sent", fn_headers_sent),
            ("headers_list", fn_headers_list),
            ("http_response_code", fn_http_response_code),
            ("setcookie", fn_setcookie),
            ("setrawcookie", fn_setrawcookie),
        ],
    );
}
