//! Query-string builtins: `http_build_query` (ext/standard/http.c) and
//! `parse_str` (main/php_variables.c `php_default_treat_data` +
//! `php_register_variable_ex`), with php's exact key mangling.

use rhp_values::ops::deref;
use rhp_values::{conv, fmt, ArrayKey, PhpArray, Value};

use crate::argparse::{int_param, str_or_null_param, str_param, zval_value_name};
use crate::{Host, Thrown};

const HEX: &[u8; 16] = b"0123456789ABCDEF";
const PHP_QUERY_RFC3986: i64 = 2;

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
            crate::runtime_fns::ini_value(b"arg_separator.output")
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

// ---------------------------------------------------------------------------
// parse_str
// ---------------------------------------------------------------------------

fn hexv(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// `php_url_decode`: `+` → space, `%XX` → byte, malformed escapes kept.
pub fn url_decode(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        if b == b'+' {
            out.push(b' ');
        } else if b == b'%' && i + 2 < s.len() {
            if let (Some(h), Some(l)) = (hexv(s[i + 1]), hexv(s[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
            out.push(b);
        } else {
            out.push(b);
        }
        i += 1;
    }
    out
}

/// `ZEND_HANDLE_NUMERIC_STR` + string key: canonical decimal ints become int keys.
fn sym_key(k: &[u8]) -> ArrayKey {
    ArrayKey::from_bytes(k)
}

fn is_forbidden(mangled: &[u8], pre: &[u8]) -> bool {
    (mangled.starts_with(b"__Host-") && !pre.starts_with(b"__Host-"))
        || (mangled.starts_with(b"__Secure-") && !pre.starts_with(b"__Secure-"))
}

/// `max_input_nesting_level` (default 64).
fn max_nesting() -> i64 {
    crate::runtime_fns::ini_value(b"max_input_nesting_level")
        .flatten()
        .map_or(64, |v| crate::runtime_fns::c_atol(&v))
}

/// Walk to `path` inside `root`, creating/replacing arrays as php does, and
/// return the target array. Each path element is `Some(key)` or `None` (`[]`
/// append). Returns `None` when an append fails.
fn descend<'a>(arr: &'a mut PhpArray, key: Option<&[u8]>) -> Option<&'a mut PhpArray> {
    let k = match key {
        None => {
            arr.push(Value::Array(PhpArray::new()));
            // the just-pushed element is the last one
            let last = arr.iter().last().map(|(k, _)| k.clone())?;
            last
        }
        Some(k) => {
            let k = sym_key(k);
            match arr.get(&k) {
                Some(Value::Array(_)) => {}
                _ => {
                    arr.insert(k.clone(), Value::Array(PhpArray::new()));
                }
            }
            k
        }
    };
    match arr.get_mut(&k) {
        Some(Value::Array(a)) => Some(a),
        _ => None,
    }
}

/// `php_register_variable_ex(var_name, val, array)`.
fn register_variable(root: &mut PhpArray, var_name: &[u8], val: Vec<u8>) {
    register_variable_ex(root, var_name, val, false)
}

/// [`register_variable`]; `cookie`: the target is `$_COOKIE`, where a
/// duplicate top-level name keeps the first value (rfc2965 ordering).
pub(crate) fn register_variable_ex(
    root: &mut PhpArray,
    var_name: &[u8],
    val: Vec<u8>,
    cookie: bool,
) {
    register_variable_value(root, var_name, Value::str(val), cookie)
}

/// [`register_variable_ex`] with any value (the upload parser registers
/// `$_FILES[..][error]` / `[size]` as ints).
pub(crate) fn register_variable_value(
    root: &mut PhpArray,
    var_name: &[u8],
    value: Value,
    cookie: bool,
) {
    // C string: the name stops at its first NUL.
    let var_name = match var_name.iter().position(|&b| b == 0) {
        Some(p) => &var_name[..p],
        None => var_name,
    };
    let mut start = 0;
    while start < var_name.len() && var_name[start] == b' ' {
        start += 1;
    }
    let var_name = &var_name[start..];
    let mut var = var_name.to_vec();
    // Mangle ' ' and '.' up to the first '['.
    let mut bracket = None;
    for (i, c) in var.iter_mut().enumerate() {
        if *c == b' ' || *c == b'.' {
            *c = b'_';
        } else if *c == b'[' {
            bracket = Some(i);
            break;
        }
    }
    let var_len = bracket.unwrap_or(var.len());
    if var_len == 0 {
        return;
    }
    let Some(mut ip) = bracket else {
        // plain var
        let index = &var[..var_len];
        if is_forbidden(index, var_name) {
            return;
        }
        if cookie && root.get(&sym_key(index)).is_some() {
            return;
        }
        root.insert(sym_key(index), value);
        return;
    };

    // Array form: collect path segments.
    let mut index: Option<Vec<u8>> = Some(var[..var_len].to_vec());
    let top = sym_key(&var[..var_len]);
    let max_nest = max_nesting();
    let mut cur: &mut PhpArray = root;
    let mut nest = 0i64;
    loop {
        nest += 1;
        if nest > max_nest {
            break;
        }
        // var[ip] == '['
        let mut p = ip + 1;
        let index_s_start = p;
        if p < var.len() && matches!(var[p], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
            p += 1;
        }
        let new_index: Option<Vec<u8>>;
        let close;
        if p < var.len() && var[p] == b']' {
            new_index = None;
            close = p;
        } else {
            match var[p.min(var.len())..].iter().position(|&b| b == b']') {
                Some(off) => {
                    close = p + off;
                    new_index = Some(var[index_s_start..close].to_vec());
                }
                None => {
                    // Not an index: php un-terminates the '[' — at the top
                    // level that splices the mangled rest onto the name; deeper
                    // down the previous ']' NUL still ends the name.
                    if index.is_none() {
                        cur.push(value);
                        return;
                    }
                    let mut name = index.clone().unwrap_or_default();
                    if nest == 1 {
                        name.push(b'_');
                        for &c in &var[index_s_start..] {
                            name.push(if matches!(c, b' ' | b'.' | b'[') {
                                b'_'
                            } else {
                                c
                            });
                        }
                    }
                    if is_forbidden(&name, var_name) {
                        return;
                    }
                    cur.insert(sym_key(&name), value);
                    return;
                }
            }
        }
        // Step into `index` (create/replace as array).
        if let Some(ix) = &index {
            if is_forbidden(ix, var_name) {
                return;
            }
        }
        cur = match descend(cur, index.as_deref()) {
            Some(a) => a,
            None => return,
        };
        index = new_index;
        let after = close + 1;
        if after < var.len() && var[after] == b'[' {
            ip = after;
            continue;
        }
        // plain_var with the current index
        match index {
            None => {
                cur.push(value);
            }
            Some(ix) => {
                if is_forbidden(&ix, var_name) {
                    return;
                }
                cur.insert(sym_key(&ix), value);
            }
        }
        return;
    }
    // Too deeply nested: php drops the whole top-level variable.
    root.remove(&top);
}

fn fn_parse_str(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = str_param(host, args, 0, "parse_str", "string")?;
    // C string: processing stops at the first NUL byte.
    let s = match s.iter().position(|&b| b == 0) {
        Some(p) => s[..p].to_vec(),
        None => s,
    };
    let mut out = PhpArray::new();
    let mut count = 0usize;
    for var in s.split(|&b| b == b'&').filter(|t| !t.is_empty()) {
        count += 1;
        if count > 1000 {
            // php: Warning "Input variables exceeded 1000 ..."
            break;
        }
        let (name, val) = match var.iter().position(|&b| b == b'=') {
            Some(p) => (&var[..p], url_decode(&var[p + 1..])),
            None => (var, Vec::new()),
        };
        let name = url_decode(name);
        register_variable(&mut out, &name, val);
    }
    if let Some(Value::Ref(r)) = args.get(1) {
        *r.borrow_mut() = Value::Array(out);
    }
    Ok(Value::Null)
}

/// `php_raw_url_decode`: `%XX` → byte (no `+` → space).
pub(crate) fn raw_url_decode(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%' && i + 2 < s.len() {
            if let (Some(h), Some(l)) = (hexv(s[i + 1]), hexv(s[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

/// Which request input `php_default_treat_data` parses.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TreatData {
    /// The query string (`$_GET`) or an urlencoded body (`$_POST`):
    /// split on any `arg_separator.input` byte, names and values
    /// `php_url_decode`d.
    Form,
    /// The `Cookie` header (`$_COOKIE`): split on `;`, leading blanks
    /// dropped from names, values raw-url-decoded, first duplicate wins.
    Cookie,
}

/// `php_default_treat_data`: parse request input into a superglobal array.
/// Returns the array and whether `max_input_vars` cut the input short.
pub fn treat_data(
    kind: TreatData,
    data: &[u8],
    separators: &[u8],
    max_vars: i64,
) -> (PhpArray, bool) {
    let mut out = PhpArray::new();
    let data = match data.iter().position(|&b| b == 0) {
        Some(p) => &data[..p],
        None => data,
    };
    let seps: &[u8] = match kind {
        TreatData::Form => separators,
        TreatData::Cookie => b";",
    };
    let mut count = 0i64;
    for var in data.split(|b| seps.contains(b)).filter(|t| !t.is_empty()) {
        let mut var = var;
        if kind == TreatData::Cookie {
            let lead = var
                .iter()
                .take_while(|&&b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
                .count();
            var = &var[lead..];
            if var.is_empty() || var[0] == b'=' {
                continue;
            }
        }
        let eq = var.iter().position(|&b| b == b'=');
        count += 1;
        if count > max_vars {
            return (out, true);
        }
        let (name, val) = match eq {
            Some(p) => {
                let raw = &var[p + 1..];
                let v = match kind {
                    TreatData::Cookie => raw_url_decode(raw),
                    TreatData::Form => url_decode(raw),
                };
                (&var[..p], v)
            }
            None => (var, Vec::new()),
        };
        let name = match kind {
            TreatData::Cookie => name.to_vec(),
            TreatData::Form => url_decode(name),
        };
        register_variable_ex(&mut out, &name, val, kind == TreatData::Cookie);
    }
    (out, false)
}

/// `php_std_post_handler` (`add_post_vars`): an urlencoded request body
/// into `$_POST`. Unlike [`treat_data`] it splits on `&` only (never
/// `arg_separator.input`), counts empty pairs, and checks
/// `max_input_vars` after registering (so one variable more is kept).
/// Returns the array and whether the limit was exceeded.
pub fn post_form_vars(body: &[u8], max_vars: i64) -> (PhpArray, bool) {
    let mut out = PhpArray::new();
    let mut ptr = 0;
    let mut count = 0i64;
    while ptr < body.len() {
        let vsep = body[ptr..]
            .iter()
            .position(|&b| b == b'&')
            .map_or(body.len(), |p| ptr + p);
        let pair = &body[ptr..vsep];
        let (name, val) = match pair.iter().position(|&b| b == b'=') {
            Some(k) => (url_decode(&pair[..k]), url_decode(&pair[k + 1..])),
            None => (url_decode(pair), Vec::new()),
        };
        register_variable_ex(&mut out, &name, val, false);
        ptr = if vsep == body.len() { vsep } else { vsep + 1 };
        count += 1;
        if count > max_vars {
            return (out, true);
        }
    }
    (out, false)
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("http_build_query", fn_http_build_query),
            ("parse_str", fn_parse_str),
        ],
    );
}
