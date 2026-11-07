//! ext/filter: filter.c's driver (`php_filter_call`, `php_zval_filter`,
//! the array handler, the INPUT_* storage) over the validating filters in
//! [`logical_filters`] and the sanitizing ones in [`sanitizing_filters`].
//!
//! The request's raw GET/POST/COOKIE/SERVER/ENV arrays (php_sapi_filter's
//! copies) are handed over by the engine at request start
//! ([`set_request_input`]); filter_input* read those, not the superglobals.

mod logical_filters;
mod sanitizing_filters;

use std::cell::RefCell;

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::{Host, Thrown};

use logical_filters::*;
use sanitizing_filters::*;

// filter_private.h
const FILTER_FLAG_NONE: i64 = 0x0000;
const FILTER_REQUIRE_ARRAY: i64 = 0x1000000;
const FILTER_REQUIRE_SCALAR: i64 = 0x2000000;
const FILTER_FORCE_ARRAY: i64 = 0x4000000;
const FILTER_NULL_ON_FAILURE: i64 = 0x8000000;
const FILTER_THROW_ON_FAILURE: i64 = 0x10000000;
const FILTER_FLAG_ALLOW_OCTAL: i64 = 0x0001;
const FILTER_FLAG_ALLOW_HEX: i64 = 0x0002;
const FILTER_FLAG_STRIP_LOW: i64 = 0x0004;
const FILTER_FLAG_STRIP_HIGH: i64 = 0x0008;
const FILTER_FLAG_ENCODE_LOW: i64 = 0x0010;
const FILTER_FLAG_ENCODE_HIGH: i64 = 0x0020;
const FILTER_FLAG_ENCODE_AMP: i64 = 0x0040;
const FILTER_FLAG_NO_ENCODE_QUOTES: i64 = 0x0080;
const FILTER_FLAG_EMPTY_STRING_NULL: i64 = 0x0100;
const FILTER_FLAG_STRIP_BACKTICK: i64 = 0x0200;
const FILTER_FLAG_ALLOW_FRACTION: i64 = 0x1000;
const FILTER_FLAG_ALLOW_THOUSAND: i64 = 0x2000;
const FILTER_FLAG_ALLOW_SCIENTIFIC: i64 = 0x4000;
const FILTER_FLAG_PATH_REQUIRED: i64 = 0x040000;
const FILTER_FLAG_QUERY_REQUIRED: i64 = 0x080000;
const FILTER_FLAG_IPV4: i64 = 0x00100000;
const FILTER_FLAG_IPV6: i64 = 0x00200000;
const FILTER_FLAG_NO_RES_RANGE: i64 = 0x00400000;
const FILTER_FLAG_NO_PRIV_RANGE: i64 = 0x00800000;
const FILTER_FLAG_GLOBAL_RANGE: i64 = 0x20000000;
const FILTER_FLAG_HOSTNAME: i64 = 0x100000;
const FILTER_FLAG_EMAIL_UNICODE: i64 = 0x100000;

const FILTER_VALIDATE_INT: i64 = 0x0101;
const FILTER_VALIDATE_BOOL: i64 = 0x0102;
const FILTER_VALIDATE_FLOAT: i64 = 0x0103;
const FILTER_VALIDATE_REGEXP: i64 = 0x0110;
const FILTER_VALIDATE_URL: i64 = 0x0111;
const FILTER_VALIDATE_EMAIL: i64 = 0x0112;
const FILTER_VALIDATE_IP: i64 = 0x0113;
const FILTER_VALIDATE_MAC: i64 = 0x0114;
const FILTER_VALIDATE_DOMAIN: i64 = 0x0115;
const FILTER_VALIDATE_LAST: i64 = 0x0115;
const FILTER_VALIDATE_ALL: i64 = 0x0100;
const FILTER_DEFAULT: i64 = 0x0204;
const FILTER_UNSAFE_RAW: i64 = 0x0204;
const FILTER_SANITIZE_STRING: i64 = 0x0201;
const FILTER_SANITIZE_ENCODED: i64 = 0x0202;
const FILTER_SANITIZE_SPECIAL_CHARS: i64 = 0x0203;
const FILTER_SANITIZE_EMAIL: i64 = 0x0205;
const FILTER_SANITIZE_URL: i64 = 0x0206;
const FILTER_SANITIZE_NUMBER_INT: i64 = 0x0207;
const FILTER_SANITIZE_NUMBER_FLOAT: i64 = 0x0208;
const FILTER_SANITIZE_FULL_SPECIAL_CHARS: i64 = 0x020a;
const FILTER_SANITIZE_ADD_SLASHES: i64 = 0x020b;
const FILTER_SANITIZE_LAST: i64 = 0x020b;
const FILTER_SANITIZE_ALL: i64 = 0x0200;
const FILTER_CALLBACK: i64 = 0x0400;

// main/php_variables.h
const PARSE_POST: i64 = 0;
const PARSE_GET: i64 = 1;
const PARSE_COOKIE: i64 = 2;
const PARSE_ENV: i64 = 4;
const PARSE_SERVER: i64 = 5;

/// `PHP_FILTER_ID_EXISTS`.
fn filter_id_exists(id: i64) -> bool {
    (FILTER_SANITIZE_ALL..=FILTER_SANITIZE_LAST).contains(&id)
        || (FILTER_VALIDATE_ALL..=FILTER_VALIDATE_LAST).contains(&id)
        || id == FILTER_CALLBACK
}

/// A filter: the value (a string) in, the result left in it. `Ok(false)`
/// is php's FAILURE: validation failed under FILTER_THROW_ON_FAILURE.
type FilterFn = fn(&mut dyn Host, &str, &mut Value, i64, Option<&Value>) -> Result<bool, Thrown>;

/// filter.c's `filter_list`.
const FILTER_LIST: &[(&str, i64, FilterFn)] = &[
    ("int", FILTER_VALIDATE_INT, php_filter_int),
    ("boolean", FILTER_VALIDATE_BOOL, php_filter_boolean),
    ("float", FILTER_VALIDATE_FLOAT, php_filter_float),
    (
        "validate_regexp",
        FILTER_VALIDATE_REGEXP,
        php_filter_validate_regexp,
    ),
    (
        "validate_domain",
        FILTER_VALIDATE_DOMAIN,
        php_filter_validate_domain,
    ),
    ("validate_url", FILTER_VALIDATE_URL, php_filter_validate_url),
    (
        "validate_email",
        FILTER_VALIDATE_EMAIL,
        php_filter_validate_email,
    ),
    ("validate_ip", FILTER_VALIDATE_IP, php_filter_validate_ip),
    ("validate_mac", FILTER_VALIDATE_MAC, php_filter_validate_mac),
    ("string", FILTER_SANITIZE_STRING, php_filter_string),
    ("stripped", FILTER_SANITIZE_STRING, php_filter_string),
    ("encoded", FILTER_SANITIZE_ENCODED, php_filter_encoded),
    (
        "special_chars",
        FILTER_SANITIZE_SPECIAL_CHARS,
        php_filter_special_chars,
    ),
    (
        "full_special_chars",
        FILTER_SANITIZE_FULL_SPECIAL_CHARS,
        php_filter_full_special_chars,
    ),
    ("unsafe_raw", FILTER_UNSAFE_RAW, php_filter_unsafe_raw),
    ("email", FILTER_SANITIZE_EMAIL, php_filter_email),
    ("url", FILTER_SANITIZE_URL, php_filter_url),
    (
        "number_int",
        FILTER_SANITIZE_NUMBER_INT,
        php_filter_number_int,
    ),
    (
        "number_float",
        FILTER_SANITIZE_NUMBER_FLOAT,
        php_filter_number_float,
    ),
    (
        "add_slashes",
        FILTER_SANITIZE_ADD_SLASHES,
        php_filter_add_slashes,
    ),
    ("callback", FILTER_CALLBACK, php_filter_callback),
];

/// `php_find_filter`: the filter `id`, else unsafe_raw.
fn find_filter(id: i64) -> (&'static str, FilterFn) {
    let e = FILTER_LIST
        .iter()
        .find(|e| e.1 == id)
        .or_else(|| FILTER_LIST.iter().find(|e| e.1 == FILTER_DEFAULT))
        .expect("unsafe_raw is listed");
    (e.0, e.2)
}

// ---------------------------------------------------------------------------
// request input
// ---------------------------------------------------------------------------

/// The raw input arrays (`IF_G(get_array)` …); `None` is php's UNDEF.
#[derive(Default)]
struct Input {
    get: Option<PhpArray>,
    post: Option<PhpArray>,
    cookie: Option<PhpArray>,
    server: Option<PhpArray>,
    env: Option<PhpArray>,
}

thread_local! {
    static INPUT: RefCell<Input> = RefCell::new(Input::default());
}

/// The request's input as php_sapi_filter keeps it. php fills an array
/// only when a variable is registered into it, so an empty one stays
/// undefined. `env` is `$_ENV` (filter_input falls back to it).
pub fn set_request_input(
    get: &PhpArray,
    post: &PhpArray,
    cookie: &PhpArray,
    server: Option<PhpArray>,
    env: &PhpArray,
) {
    let some = |a: &PhpArray| (!a.is_empty()).then(|| a.clone());
    INPUT.with(|i| {
        *i.borrow_mut() = Input {
            get: some(get),
            post: some(post),
            cookie: some(cookie),
            server,
            env: Some(env.clone()),
        }
    });
}

/// The `$_SERVER` entries a SAPI registers through the input filter: the
/// script names under php-cli, `PHP_SELF` under the CGI-style SAPIs.
fn server_filter_keys() -> &'static [&'static [u8]] {
    if crate::main::sapi::sapi_name() == "cli" {
        &[
            b"PHP_SELF",
            b"SCRIPT_NAME",
            b"SCRIPT_FILENAME",
            b"PATH_TRANSLATED",
            b"DOCUMENT_ROOT",
        ]
    } else {
        &[b"PHP_SELF"]
    }
}

/// The part of `$_SERVER` php_sapi_filter keeps a raw copy of.
pub fn filtered_server_vars(server: &PhpArray) -> Option<PhpArray> {
    let mut out = PhpArray::new();
    for k in server_filter_keys() {
        let k = ArrayKey::from_bytes(k);
        if let Some(v) = server.get(&k) {
            out.insert(k, v.clone());
        }
    }
    (!out.is_empty()).then_some(out)
}

/// `filter.default` and `filter.default_flags` (UpdateDefaultFilter,
/// OnUpdateFlags): the filter php_sapi_filter runs over the input.
fn default_filter() -> (i64, i64) {
    let name = crate::main::ini::ini_value(b"filter.default").flatten();
    let id = name
        .and_then(|n| {
            FILTER_LIST
                .iter()
                .find(|e| e.0.as_bytes().eq_ignore_ascii_case(&n))
                .map(|e| e.1)
        })
        .unwrap_or(FILTER_DEFAULT);
    let flags = match crate::main::ini::ini_value(b"filter.default_flags").flatten() {
        Some(v) => crate::main::ini::c_atol(&v) as i32 as i64,
        None => FILTER_FLAG_NO_ENCODE_QUOTES,
    };
    (id, flags)
}

/// Whether `filter.default = name` deprecates (UpdateDefaultFilter): it
/// names a filter other than unsafe_raw.
pub(crate) fn default_is_deprecated(name: &[u8]) -> bool {
    FILTER_LIST
        .iter()
        .any(|e| e.0.as_bytes().eq_ignore_ascii_case(name) && e.1 != FILTER_DEFAULT)
}

/// php_sapi_filter's mangled value: a non-empty one goes through the
/// default filter unless that is unsafe_raw.
fn default_filter_value(host: &mut dyn Host, id: i64, flags: i64, v: &mut Value) {
    if matches!(v, Value::Str(s) if !s.is_empty()) {
        if let Err(t) = zval_filter(host, "Unknown", v, id, flags, None) {
            host.set_pending_throw(t);
        }
    }
}

fn default_filter_all(host: &mut dyn Host, id: i64, flags: i64, a: &mut PhpArray) {
    let keys: Vec<ArrayKey> = a.keys().collect();
    for k in keys {
        if let Some(v) = a.get_mut(&k) {
            let mut x = std::mem::replace(v, Value::Null);
            match &mut x {
                Value::Array(inner) => default_filter_all(host, id, flags, inner),
                x => default_filter_value(host, id, flags, x),
            }
            if let Some(v) = a.get_mut(&k) {
                *v = x;
            }
        }
    }
}

/// filter.default over a request array's values (GET, POST, COOKIE).
pub fn apply_default_filter(host: &mut dyn Host, a: &mut PhpArray) {
    let (id, flags) = default_filter();
    if id != FILTER_UNSAFE_RAW {
        default_filter_all(host, id, flags, a);
    }
}

/// filter.default over parse_str's values, each left a string.
pub fn apply_default_filter_strings(host: &mut dyn Host, a: &mut PhpArray) {
    fn stringify(a: &mut PhpArray) {
        let keys: Vec<ArrayKey> = a.keys().collect();
        for k in keys {
            if let Some(v) = a.get_mut(&k) {
                match v {
                    Value::Array(inner) => stringify(inner),
                    Value::Str(_) => {}
                    other => *other = Value::str(conv::to_string_bytes(other)),
                }
            }
        }
    }
    let (id, flags) = default_filter();
    if id != FILTER_UNSAFE_RAW {
        default_filter_all(host, id, flags, a);
        stringify(a);
    }
}

/// filter.default over the `$_SERVER` entries the SAPI filters.
pub fn apply_default_filter_server(host: &mut dyn Host, server: &mut PhpArray) {
    let (id, flags) = default_filter();
    if id == FILTER_UNSAFE_RAW {
        return;
    }
    for k in server_filter_keys() {
        if let Some(v) = server.get_mut(&ArrayKey::from_bytes(k)) {
            let mut x = std::mem::replace(v, Value::Null);
            default_filter_value(host, id, flags, &mut x);
            if let Some(v) = server.get_mut(&ArrayKey::from_bytes(k)) {
                *v = x;
            }
        }
    }
}

pub(crate) fn reset_state() {
    INPUT.with(|i| *i.borrow_mut() = Input::default());
}

/// `php_filter_get_storage`: `Err` for a bad INPUT_* type, `Ok(None)` for
/// an input php never filled.
fn get_storage(fname: &str, pname: &str, arg: i64) -> Result<Option<PhpArray>, Thrown> {
    INPUT.with(|i| {
        let i = i.borrow();
        Ok(match arg {
            PARSE_GET => i.get.clone(),
            PARSE_POST => i.post.clone(),
            PARSE_COOKIE => i.cookie.clone(),
            PARSE_SERVER => i.server.clone(),
            PARSE_ENV => Some(i.env.clone().unwrap_or_default()),
            _ => {
                return Err(Thrown::error(
                    "ValueError",
                    format!("{fname}(): Argument #1 (${pname}) must be an INPUT_* constant"),
                ))
            }
        })
    })
}

/// `zend_hash_find` with a string key: a numeric string never matches the
/// integer keys a request array has.
fn find_str<'a>(a: &'a PhpArray, name: &[u8]) -> Option<&'a Value> {
    match ArrayKey::from_bytes(name) {
        k @ ArrayKey::Str(_) => a.get(&k),
        ArrayKey::Int(_) => None,
    }
}

// ---------------------------------------------------------------------------
// the driver
// ---------------------------------------------------------------------------

/// A `Filter\FilterFailedException` with `msg`.
fn failed_exception(host: &mut dyn Host, msg: Vec<u8>) -> Thrown {
    const CLASS: &str = "Filter\\FilterFailedException";
    match host.reflect_new_instance(CLASS.as_bytes(), vec![Value::str(msg.clone())], true) {
        Ok(Value::Object(h)) => Thrown::Object(h),
        Ok(_) => Thrown::error(CLASS, String::from_utf8_lossy(&msg)),
        Err(t) => t,
    }
}

/// `%s` of a string: it stops at the first NUL.
fn c_str(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]
}

/// `php_zval_filter`: one scalar through filter `filter`.
fn zval_filter(
    host: &mut dyn Host,
    fname: &str,
    value: &mut Value,
    filter: i64,
    flags: i64,
    options: Option<&Value>,
) -> Result<(), Thrown> {
    let (name, func) = find_filter(filter);
    let obj_class = match &*value {
        Value::Object(h) => {
            let class = h.borrow().class;
            (!host.class_has_method(class, b"__tostring"))
                .then(|| host.class_name(class).unwrap_or_default())
        }
        Value::Closure(_) => Some(b"Closure".to_vec()),
        _ => None,
    };
    if let Some(class) = obj_class {
        // an object without __toString() fails (#49274)
        if flags & FILTER_THROW_ON_FAILURE != 0 {
            *value = Value::Null;
            let mut msg = b"filter validation failed: object of type ".to_vec();
            msg.extend_from_slice(c_str(&class));
            msg.extend_from_slice(b" has no __toString() method");
            return Err(failed_exception(host, msg));
        }
        *value = if flags & FILTER_NULL_ON_FAILURE != 0 {
            Value::Null
        } else {
            Value::Bool(false)
        };
    } else {
        // Here be strings
        let s = match &*value {
            Value::Str(_) => None,
            Value::Object(_) => crate::zend::object_handlers::to_string(host, value)?,
            v => Some(conv::to_string_bytes(v)),
        };
        if let Some(s) = s {
            *value = Value::str(s);
        }
        let copy = (flags & FILTER_THROW_ON_FAILURE != 0).then(|| match &*value {
            Value::Str(s) => s.as_bytes().to_vec(),
            _ => Vec::new(),
        });
        let ok = func(host, fname, value, flags, options)?;
        if let (Some(copy), false) = (copy, ok) {
            let mut msg =
                format!("filter validation failed: filter {name} not satisfied by '").into_bytes();
            msg.extend_from_slice(c_str(&copy));
            msg.push(b'\'');
            return Err(failed_exception(host, msg));
        }
    }
    // the "default" option replaces a failure
    if let Some(Value::Array(o)) = options {
        let null_on_failure = flags & FILTER_NULL_ON_FAILURE != 0;
        if (null_on_failure && matches!(value, Value::Null))
            || (!null_on_failure && matches!(value, Value::Bool(false)))
        {
            if let Some(d) = o.get(&ArrayKey::from_bytes(b"default")) {
                *value = deref(d);
            }
        }
    }
    Ok(())
}

/// `php_zval_filter_recursive`: every scalar inside an array, through
/// references, each array once.
fn zval_filter_recursive(
    host: &mut dyn Host,
    fname: &str,
    value: &mut Value,
    filter: i64,
    flags: i64,
    options: Option<&Value>,
    seen: &mut Vec<usize>,
) -> Result<(), Thrown> {
    let Value::Array(a) = value else {
        return zval_filter(host, fname, value, filter, flags, options);
    };
    let keys: Vec<ArrayKey> = a.keys().collect();
    for k in keys {
        let Some(slot) = a.get_mut(&k) else {
            continue;
        };
        if let Value::Ref(r) = slot {
            let r = r.clone();
            let ptr = std::rc::Rc::as_ptr(&r) as usize;
            let mut inner = r.borrow().clone();
            if matches!(inner, Value::Array(_)) {
                // a recursive array is filtered once
                if seen.contains(&ptr) {
                    continue;
                }
                seen.push(ptr);
                let res =
                    zval_filter_recursive(host, fname, &mut inner, filter, flags, options, seen);
                seen.pop();
                *r.borrow_mut() = inner;
                res?;
            } else {
                let res = zval_filter(host, fname, &mut inner, filter, flags, options);
                *r.borrow_mut() = inner;
                res?;
            }
        } else {
            let mut v = std::mem::replace(slot, Value::Null);
            let res = if matches!(v, Value::Array(_)) {
                zval_filter_recursive(host, fname, &mut v, filter, flags, options, seen)
            } else {
                zval_filter(host, fname, &mut v, filter, flags, options)
            };
            if let Some(slot) = a.get_mut(&k) {
                *slot = v;
            }
            res?;
        }
    }
    Ok(())
}

/// The `array|int $options` argument.
enum FilterArgs<'a> {
    Ht(&'a PhpArray),
    Long(i64),
}

/// `php_filter_call`. `filter` is -1 for the array handler, where a long
/// `args` is the filter id rather than the flags.
#[allow(clippy::too_many_arguments)]
fn filter_call(
    host: &mut dyn Host,
    fname: &str,
    filtered: &mut Value,
    mut filter: i64,
    args: &FilterArgs,
    mut flags: i64,
    options_arg: usize,
) -> Result<(), Thrown> {
    let mut options: Option<Value> = None;
    match args {
        FilterArgs::Long(l) => {
            if filter != -1 {
                flags = *l;
                if flags & (FILTER_REQUIRE_ARRAY | FILTER_FORCE_ARRAY) == 0 {
                    flags |= FILTER_REQUIRE_SCALAR;
                }
            } else {
                filter = *l;
            }
        }
        FilterArgs::Ht(ht) => {
            if let Some(f) = ht.get(&ArrayKey::from_bytes(b"filter")) {
                filter = crate::zend::zpp::zval_get_long(host, f)?;
            }
            if let Some(o) = ht.get(&ArrayKey::from_bytes(b"options")).map(deref) {
                if filter != FILTER_CALLBACK {
                    if matches!(o, Value::Array(_)) {
                        options = Some(o);
                    }
                } else {
                    options = Some(o);
                    flags = 0;
                }
            }
            if let Some(f) = ht.get(&ArrayKey::from_bytes(b"flags")) {
                flags = crate::zend::zpp::zval_get_long(host, f)?;
                if flags & (FILTER_REQUIRE_ARRAY | FILTER_FORCE_ARRAY) == 0 {
                    flags |= FILTER_REQUIRE_SCALAR;
                }
            }
        }
    }
    if flags & FILTER_NULL_ON_FAILURE != 0 && flags & FILTER_THROW_ON_FAILURE != 0 {
        *filtered = Value::Null;
        return Err(Thrown::error(
            "ValueError",
            format!(
                "{fname}(): Argument #{options_arg} ($options) cannot use both FILTER_NULL_ON_FAILURE and FILTER_THROW_ON_FAILURE"
            ),
        ));
    }
    let options = options.as_ref();
    if matches!(filtered, Value::Array(_)) {
        if flags & FILTER_REQUIRE_SCALAR != 0 {
            if flags & FILTER_THROW_ON_FAILURE != 0 {
                *filtered = Value::Null;
                return Err(failed_exception(
                    host,
                    b"filter validation failed: not a scalar value (got an array)".to_vec(),
                ));
            }
            *filtered = if flags & FILTER_NULL_ON_FAILURE != 0 {
                Value::Null
            } else {
                Value::Bool(false)
            };
            return Ok(());
        }
        return zval_filter_recursive(
            host,
            fname,
            filtered,
            filter,
            flags,
            options,
            &mut Vec::new(),
        );
    }
    if flags & FILTER_REQUIRE_ARRAY != 0 {
        if flags & FILTER_THROW_ON_FAILURE != 0 {
            let got = crate::zend::zpp::zval_value_name(host, filtered);
            *filtered = Value::Null;
            return Err(failed_exception(
                host,
                format!("filter validation failed: not an array (got {got})").into_bytes(),
            ));
        }
        *filtered = if flags & FILTER_NULL_ON_FAILURE != 0 {
            Value::Null
        } else {
            Value::Bool(false)
        };
        return Ok(());
    }
    zval_filter(host, fname, filtered, filter, flags, options)?;
    if flags & FILTER_FORCE_ARRAY != 0 {
        let mut a = PhpArray::new();
        a.push(std::mem::replace(filtered, Value::Null));
        *filtered = Value::Array(a);
    }
    Ok(())
}

/// `php_filter_array_handler`.
fn array_handler(
    host: &mut dyn Host,
    fname: &str,
    input: &PhpArray,
    op: &FilterArgs,
    add_empty: bool,
) -> Result<Value, Thrown> {
    let ht = match op {
        FilterArgs::Long(_) => {
            let mut rv = Value::Array(input.clone());
            filter_call(host, fname, &mut rv, -1, op, FILTER_REQUIRE_ARRAY, 2)?;
            return Ok(rv);
        }
        FilterArgs::Ht(ht) => ht,
    };
    let mut out = PhpArray::new();
    for (k, elm) in ht.iter() {
        let key = match &k {
            ArrayKey::Str(s) => s.as_bytes().to_vec(),
            ArrayKey::Int(_) => {
                return Err(Thrown::type_error(format!(
                    "{fname}(): Argument #2 ($options) must contain only string keys"
                )))
            }
        };
        if key.is_empty() {
            return Err(Thrown::error(
                "ValueError",
                format!("{fname}(): Argument #2 ($options) cannot contain empty keys"),
            ));
        }
        match input.get(&k) {
            None => {
                if add_empty {
                    out.insert(k.clone(), Value::Null);
                }
            }
            Some(v) => {
                let mut nval = deref(v);
                let elm = deref(elm);
                let args = match &elm {
                    Value::Array(a) => FilterArgs::Ht(a),
                    other => {
                        let id = crate::zend::zpp::zval_get_long(host, other)?;
                        if !filter_id_exists(id) {
                            crate::warn(host, fname, &format!("Unknown filter with ID {id}"))?;
                        }
                        FilterArgs::Long(id)
                    }
                };
                filter_call(host, fname, &mut nval, -1, &args, FILTER_REQUIRE_SCALAR, 2)?;
                out.insert(k.clone(), nval);
            }
        }
    }
    Ok(Value::Array(out))
}

// ---------------------------------------------------------------------------
// functions
// ---------------------------------------------------------------------------

fn arg(args: &[Value], i: usize) -> Option<Value> {
    args.get(i).map(deref)
}

fn int_arg(args: &[Value], i: usize, default: i64) -> i64 {
    arg(args, i).map_or(default, |v| conv::to_int(&v))
}

/// An `array|int` argument (zpp has checked it).
fn filter_args(v: &Option<Value>, default: i64) -> FilterArgs<'_> {
    match v {
        Some(Value::Array(a)) => FilterArgs::Ht(a),
        Some(v) => FilterArgs::Long(conv::to_int(v)),
        None => FilterArgs::Long(default),
    }
}

fn unknown_filter(host: &mut dyn Host, fname: &str, id: i64) -> Result<Value, Thrown> {
    crate::warn(host, fname, &format!("Unknown filter with ID {id}"))?;
    Ok(Value::Bool(false))
}

fn fn_filter_has_var(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "filter_has_var";
    let ty = int_arg(args, 0, 0);
    let var = crate::zend::zpp::str_param(host, args, 1, f, "var_name")?;
    let storage = get_storage(f, "input_type", ty)?;
    Ok(Value::Bool(
        storage.is_some_and(|a| find_str(&a, &var).is_some()),
    ))
}

fn fn_filter_input(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "filter_input";
    let ty = int_arg(args, 0, 0);
    let var = crate::zend::zpp::str_param(host, args, 1, f, "var_name")?;
    let filter = int_arg(args, 2, FILTER_DEFAULT);
    let opt = arg(args, 3);
    let fa = filter_args(&opt, 0);
    if !filter_id_exists(filter) {
        return unknown_filter(host, f, filter);
    }
    let input = get_storage(f, "type", ty)?;
    let found = input.as_ref().and_then(|a| find_str(a, &var)).cloned();
    let Some(v) = found else {
        let flags = match &fa {
            FilterArgs::Long(l) => *l,
            FilterArgs::Ht(ht) => match ht.get(&ArrayKey::from_bytes(b"flags")) {
                Some(v) => crate::zend::zpp::zval_get_long(host, v)?,
                None => 0,
            },
        };
        if flags & FILTER_NULL_ON_FAILURE != 0 && flags & FILTER_THROW_ON_FAILURE != 0 {
            return Err(Thrown::error(
                "ValueError",
                format!("{f}(): Argument #4 ($options) cannot use both FILTER_NULL_ON_FAILURE and FILTER_THROW_ON_FAILURE"),
            ));
        }
        if flags & FILTER_THROW_ON_FAILURE != 0 {
            let mut msg = b"input value '".to_vec();
            msg.extend_from_slice(c_str(&var));
            msg.extend_from_slice(b"' not found");
            return Err(failed_exception(host, msg));
        }
        if let FilterArgs::Ht(ht) = &fa {
            if let Some(Value::Array(o)) = ht.get(&ArrayKey::from_bytes(b"options")).map(deref) {
                if let Some(d) = o.get(&ArrayKey::from_bytes(b"default")) {
                    return Ok(deref(d));
                }
            }
        }
        // FILTER_NULL_ON_FAILURE swaps the usual null (missing) and false
        return Ok(if flags & FILTER_NULL_ON_FAILURE != 0 {
            Value::Bool(false)
        } else {
            Value::Null
        });
    };
    let mut rv = deref(&v);
    filter_call(host, f, &mut rv, filter, &fa, FILTER_REQUIRE_SCALAR, 4)?;
    Ok(rv)
}

fn fn_filter_var(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "filter_var";
    let filter = int_arg(args, 1, FILTER_DEFAULT);
    let opt = arg(args, 2);
    let fa = filter_args(&opt, 0);
    if !filter_id_exists(filter) {
        return unknown_filter(host, f, filter);
    }
    let mut rv = arg(args, 0).unwrap_or(Value::Null);
    filter_call(host, f, &mut rv, filter, &fa, FILTER_REQUIRE_SCALAR, 3)?;
    Ok(rv)
}

fn fn_filter_input_array(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "filter_input_array";
    let ty = int_arg(args, 0, 0);
    let opt = arg(args, 1);
    let fa = filter_args(&opt, FILTER_DEFAULT);
    let add_empty = arg(args, 2).is_none_or(|v| conv::to_bool(&v));
    if let FilterArgs::Long(id) = fa {
        if !filter_id_exists(id) {
            return unknown_filter(host, f, id);
        }
    }
    match get_storage(f, "type", ty)? {
        Some(input) => array_handler(host, f, &input, &fa, add_empty),
        None => Ok(Value::Null),
    }
}

fn fn_filter_var_array(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "filter_var_array";
    let input = crate::zend::zpp::array_param(host, args, 0, f)?;
    let opt = arg(args, 1);
    let fa = filter_args(&opt, FILTER_DEFAULT);
    let add_empty = arg(args, 2).is_none_or(|v| conv::to_bool(&v));
    if let FilterArgs::Long(id) = fa {
        if !filter_id_exists(id) {
            return unknown_filter(host, f, id);
        }
    }
    array_handler(host, f, &input, &fa, add_empty)
}

fn fn_filter_list(_host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    for (name, _, _) in FILTER_LIST {
        a.push(Value::str(name.as_bytes()));
    }
    Ok(Value::Array(a))
}

fn fn_filter_id(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = crate::zend::zpp::str_param(host, args, 0, "filter_id", "name")?;
    let name = c_str(&name);
    Ok(FILTER_LIST
        .iter()
        .find(|e| e.0.as_bytes() == name)
        .map_or(Value::Bool(false), |e| Value::Int(e.1)))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// `Filter\FilterException` and `Filter\FilterFailedException`.
pub const PRELUDE_PHP: &str = include_str!("prelude.php");

/// The E_DEPRECATED of the `#[\Deprecated]` constants.
pub(crate) fn constant_deprecation(name: &[u8]) -> Option<String> {
    matches!(
        name,
        b"FILTER_SANITIZE_STRING" | b"FILTER_SANITIZE_STRIPPED"
    )
    .then(|| {
        format!(
            "Constant {} is deprecated since 8.1, use htmlspecialchars() instead",
            String::from_utf8_lossy(name)
        )
    })
}

/// filter.stub.php's constants.
pub(crate) fn constants() -> Vec<(&'static str, Value)> {
    [
        ("INPUT_POST", PARSE_POST),
        ("INPUT_GET", PARSE_GET),
        ("INPUT_COOKIE", PARSE_COOKIE),
        ("INPUT_ENV", PARSE_ENV),
        ("INPUT_SERVER", PARSE_SERVER),
        ("FILTER_FLAG_NONE", FILTER_FLAG_NONE),
        ("FILTER_REQUIRE_SCALAR", FILTER_REQUIRE_SCALAR),
        ("FILTER_REQUIRE_ARRAY", FILTER_REQUIRE_ARRAY),
        ("FILTER_FORCE_ARRAY", FILTER_FORCE_ARRAY),
        ("FILTER_NULL_ON_FAILURE", FILTER_NULL_ON_FAILURE),
        ("FILTER_THROW_ON_FAILURE", FILTER_THROW_ON_FAILURE),
        ("FILTER_VALIDATE_INT", FILTER_VALIDATE_INT),
        ("FILTER_VALIDATE_BOOLEAN", FILTER_VALIDATE_BOOL),
        ("FILTER_VALIDATE_BOOL", FILTER_VALIDATE_BOOL),
        ("FILTER_VALIDATE_FLOAT", FILTER_VALIDATE_FLOAT),
        ("FILTER_VALIDATE_REGEXP", FILTER_VALIDATE_REGEXP),
        ("FILTER_VALIDATE_DOMAIN", FILTER_VALIDATE_DOMAIN),
        ("FILTER_VALIDATE_URL", FILTER_VALIDATE_URL),
        ("FILTER_VALIDATE_EMAIL", FILTER_VALIDATE_EMAIL),
        ("FILTER_VALIDATE_IP", FILTER_VALIDATE_IP),
        ("FILTER_VALIDATE_MAC", FILTER_VALIDATE_MAC),
        ("FILTER_DEFAULT", FILTER_DEFAULT),
        ("FILTER_UNSAFE_RAW", FILTER_UNSAFE_RAW),
        ("FILTER_SANITIZE_STRING", FILTER_SANITIZE_STRING),
        ("FILTER_SANITIZE_STRIPPED", FILTER_SANITIZE_STRING),
        ("FILTER_SANITIZE_ENCODED", FILTER_SANITIZE_ENCODED),
        (
            "FILTER_SANITIZE_SPECIAL_CHARS",
            FILTER_SANITIZE_SPECIAL_CHARS,
        ),
        (
            "FILTER_SANITIZE_FULL_SPECIAL_CHARS",
            FILTER_SANITIZE_FULL_SPECIAL_CHARS,
        ),
        ("FILTER_SANITIZE_EMAIL", FILTER_SANITIZE_EMAIL),
        ("FILTER_SANITIZE_URL", FILTER_SANITIZE_URL),
        ("FILTER_SANITIZE_NUMBER_INT", FILTER_SANITIZE_NUMBER_INT),
        ("FILTER_SANITIZE_NUMBER_FLOAT", FILTER_SANITIZE_NUMBER_FLOAT),
        ("FILTER_SANITIZE_ADD_SLASHES", FILTER_SANITIZE_ADD_SLASHES),
        ("FILTER_CALLBACK", FILTER_CALLBACK),
        ("FILTER_FLAG_ALLOW_OCTAL", FILTER_FLAG_ALLOW_OCTAL),
        ("FILTER_FLAG_ALLOW_HEX", FILTER_FLAG_ALLOW_HEX),
        ("FILTER_FLAG_STRIP_LOW", FILTER_FLAG_STRIP_LOW),
        ("FILTER_FLAG_STRIP_HIGH", FILTER_FLAG_STRIP_HIGH),
        ("FILTER_FLAG_STRIP_BACKTICK", FILTER_FLAG_STRIP_BACKTICK),
        ("FILTER_FLAG_ENCODE_LOW", FILTER_FLAG_ENCODE_LOW),
        ("FILTER_FLAG_ENCODE_HIGH", FILTER_FLAG_ENCODE_HIGH),
        ("FILTER_FLAG_ENCODE_AMP", FILTER_FLAG_ENCODE_AMP),
        ("FILTER_FLAG_NO_ENCODE_QUOTES", FILTER_FLAG_NO_ENCODE_QUOTES),
        (
            "FILTER_FLAG_EMPTY_STRING_NULL",
            FILTER_FLAG_EMPTY_STRING_NULL,
        ),
        ("FILTER_FLAG_ALLOW_FRACTION", FILTER_FLAG_ALLOW_FRACTION),
        ("FILTER_FLAG_ALLOW_THOUSAND", FILTER_FLAG_ALLOW_THOUSAND),
        ("FILTER_FLAG_ALLOW_SCIENTIFIC", FILTER_FLAG_ALLOW_SCIENTIFIC),
        ("FILTER_FLAG_PATH_REQUIRED", FILTER_FLAG_PATH_REQUIRED),
        ("FILTER_FLAG_QUERY_REQUIRED", FILTER_FLAG_QUERY_REQUIRED),
        ("FILTER_FLAG_IPV4", FILTER_FLAG_IPV4),
        ("FILTER_FLAG_IPV6", FILTER_FLAG_IPV6),
        ("FILTER_FLAG_NO_RES_RANGE", FILTER_FLAG_NO_RES_RANGE),
        ("FILTER_FLAG_NO_PRIV_RANGE", FILTER_FLAG_NO_PRIV_RANGE),
        ("FILTER_FLAG_GLOBAL_RANGE", FILTER_FLAG_GLOBAL_RANGE),
        ("FILTER_FLAG_HOSTNAME", FILTER_FLAG_HOSTNAME),
        ("FILTER_FLAG_EMAIL_UNICODE", FILTER_FLAG_EMAIL_UNICODE),
    ]
    .into_iter()
    .map(|(n, v)| (n, Value::Int(v)))
    .collect()
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("filter_has_var", fn_filter_has_var),
            ("filter_input", fn_filter_input),
            ("filter_var", fn_filter_var),
            ("filter_input_array", fn_filter_input_array),
            ("filter_var_array", fn_filter_var_array),
            ("filter_list", fn_filter_list),
            ("filter_id", fn_filter_id),
        ],
    );
}

#[cfg(test)]
mod tests;
