//! Small standard-library / system builtins: `array_first`/`array_last`,
//! `str_increment`/`str_decrement`, `ip2long`/`long2ip`, `gettimeofday`,
//! `lcg_value`, `zend_version`, `get_cfg_var`, `get_include_path`/
//! `set_include_path`, `error_clear_last`, `cli_set_process_title`/
//! `cli_get_process_title`, `gethostname`, `gethostbyname`, `getmyuid`/
//! `getmygid`/`getmyinode`, `get_current_user`.

use std::collections::HashMap;

use rhp_semantics::deref;
use rhp_values::{ArrayKey, PhpArray, Value};

use crate::argparse::{bool_param, int_param, str_param, value_name};
use crate::{Builtin, Host, Thrown};

fn want_array(host: &dyn Host, args: &[Value], func: &str) -> Result<PhpArray, Thrown> {
    match deref(&args[0]) {
        Value::Array(a) => Ok(a),
        other => Err(Thrown::type_error(format!(
            "{func}(): Argument #1 ($array) must be of type array, {} given",
            value_name(host, &other)
        ))),
    }
}

fn fn_array_first(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = want_array(host, args, "array_first")?;
    Ok(a.iter()
        .next()
        .map(|(_, v)| v.clone())
        .unwrap_or(Value::Null))
}

fn fn_array_last(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let a = want_array(host, args, "array_last")?;
    Ok(a.iter()
        .last()
        .map(|(_, v)| v.clone())
        .unwrap_or(Value::Null))
}

fn check_alnum(s: &[u8], func: &str) -> Result<(), Thrown> {
    if s.is_empty() {
        return Err(Thrown::error(
            "ValueError",
            format!("{func}(): Argument #1 ($string) must not be empty"),
        ));
    }
    if !s.iter().all(|c| c.is_ascii_alphanumeric()) {
        return Err(Thrown::error(
            "ValueError",
            format!("{func}(): Argument #1 ($string) must be composed only of alphanumeric ASCII characters"),
        ));
    }
    Ok(())
}

fn fn_str_increment(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = str_param(host, args, 0, "str_increment", "string")?;
    check_alnum(&s, "str_increment")?;
    let mut v = s.clone();
    let mut pos = v.len();
    let mut carry = true;
    while carry && pos > 0 {
        pos -= 1;
        match v[pos] {
            b'z' | b'Z' => {
                v[pos] -= 25;
            }
            b'9' => v[pos] = b'0',
            _ => {
                v[pos] += 1;
                carry = false;
            }
        }
    }
    if carry {
        let first = if v[0] == b'0' { b'1' } else { v[0] };
        v.insert(0, first);
    }
    Ok(Value::str(v))
}

fn out_of_range(s: &[u8]) -> Thrown {
    Thrown::error(
        "ValueError",
        format!(
            "str_decrement(): Argument #1 ($string) \"{}\" is out of decrement range",
            String::from_utf8_lossy(s)
        ),
    )
}

fn fn_str_decrement(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = str_param(host, args, 0, "str_decrement", "string")?;
    check_alnum(&s, "str_decrement")?;
    if s[0] == b'0' {
        return Err(out_of_range(&s));
    }
    let mut v = s.clone();
    let mut pos = v.len();
    let mut carry = true;
    while carry && pos > 0 {
        pos -= 1;
        match v[pos] {
            b'a' | b'A' => v[pos] += 25,
            b'0' => v[pos] = b'9',
            _ => {
                v[pos] -= 1;
                carry = false;
            }
        }
    }
    if carry || (v[0] == b'0' && v.len() > 1) {
        if v.len() == 1 {
            return Err(out_of_range(&s));
        }
        v.remove(0);
    }
    Ok(Value::str(v))
}

fn fn_ip2long(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = str_param(host, args, 0, "ip2long", "ip")?;
    if s.is_empty() || s.contains(&0) {
        return Ok(Value::Bool(false));
    }
    // inet_pton(AF_INET): exactly four dot-separated decimal octets <= 255.
    let mut parts = 0;
    let mut n: u32 = 0;
    for p in s.split(|&b| b == b'.') {
        parts += 1;
        if p.is_empty() || parts > 4 || !p.iter().all(u8::is_ascii_digit) {
            return Ok(Value::Bool(false));
        }
        let mut o: u32 = 0;
        for &d in p {
            o = o * 10 + (d - b'0') as u32;
            if o > 255 {
                return Ok(Value::Bool(false));
            }
        }
        n = (n << 8) | o;
    }
    if parts != 4 {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Int(n as i64))
}

fn fn_long2ip(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = int_param(host, args, 0, "long2ip", "ip", 0)? as u64 as u32;
    let ip = std::net::Ipv4Addr::from(n);
    Ok(Value::str(ip.to_string().into_bytes()))
}

fn fn_gettimeofday(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let as_float = bool_param(host, args, 0, "gettimeofday", "as_float", false)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    if as_float {
        return Ok(Value::Float(
            now.as_secs() as f64 + now.subsec_micros() as f64 / 1_000_000.0,
        ));
    }
    let mut a = PhpArray::new();
    a.insert(
        ArrayKey::from_bytes(b"sec"),
        Value::Int(now.as_secs() as i64),
    );
    a.insert(
        ArrayKey::from_bytes(b"usec"),
        Value::Int(now.subsec_micros() as i64),
    );
    a.insert(ArrayKey::from_bytes(b"minuteswest"), Value::Int(0));
    a.insert(ArrayKey::from_bytes(b"dsttime"), Value::Int(0));
    Ok(Value::Array(a))
}

fn fn_lcg_value(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    // 53 random bits → [0, 1).
    let n = u64::from_le_bytes(b) >> 11;
    Ok(Value::Float(n as f64 / (1u64 << 53) as f64))
}

fn fn_zend_version(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str("4.5.8"))
}

fn fn_get_cfg_var(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    // No php.ini is loaded, so the configuration hash is empty.
    let _ = str_param(host, args, 0, "get_cfg_var", "option")?;
    Ok(Value::Bool(false))
}

fn fn_get_include_path(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(
        match crate::runtime_fns::ini_value(b"include_path").flatten() {
            Some(v) => Value::str(v),
            None => Value::Bool(false),
        },
    )
}

fn fn_set_include_path(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let p = str_param(host, args, 0, "set_include_path", "include_path")?;
    if p.is_empty() {
        return Ok(Value::Bool(false));
    }
    let old = crate::runtime_fns::ini_value(b"include_path").flatten();
    crate::runtime_fns::ini_store(b"include_path", p);
    Ok(old.map(Value::str).unwrap_or(Value::Bool(false)))
}

fn fn_null(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Null)
}

fn fn_cli_set_process_title(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let t = str_param(host, args, 0, "cli_set_process_title", "title")?;
    TITLE.with(|c| *c.borrow_mut() = Some(t));
    Ok(Value::Bool(true))
}

thread_local! {
    static TITLE: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
}

fn fn_cli_get_process_title(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(
        TITLE.with(|c| c.borrow().clone()).unwrap_or_default(),
    ))
}

fn fn_gethostname(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut buf = [0u8; 256];
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc != 0 {
        return Ok(Value::Bool(false));
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    Ok(Value::str(buf[..end].to_vec()))
}

/// `gethostbyname`: the first IPv4 address, or the input unchanged on failure.
fn fn_gethostbyname(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    use std::net::ToSocketAddrs;
    let name = str_param(host, args, 0, "gethostbyname", "hostname")?;
    if name.len() > 255 {
        return Ok(Value::Bool(false));
    }
    let s = String::from_utf8_lossy(&name).into_owned();
    if let Ok(addrs) = (s.as_str(), 0u16).to_socket_addrs() {
        for a in addrs {
            if let std::net::SocketAddr::V4(v4) = a {
                return Ok(Value::str(v4.ip().to_string().into_bytes()));
            }
        }
    }
    Ok(Value::str(name))
}

fn fn_getmyuid(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(script_stat(host)
        .map(|m| Value::Int(std::os::unix::fs::MetadataExt::uid(&m) as i64))
        .unwrap_or(Value::Bool(false)))
}

fn fn_getmygid(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(script_stat(host)
        .map(|m| Value::Int(std::os::unix::fs::MetadataExt::gid(&m) as i64))
        .unwrap_or(Value::Bool(false)))
}

fn fn_getmyinode(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(script_stat(host)
        .map(|m| Value::Int(std::os::unix::fs::MetadataExt::ino(&m) as i64))
        .unwrap_or(Value::Bool(false)))
}

/// Stat of the main script (php's `sapi_get_stat`).
fn script_stat(host: &dyn Host) -> Option<std::fs::Metadata> {
    let p = host.running_main_path()?;
    std::fs::metadata(String::from_utf8_lossy(&p).into_owned()).ok()
}

/// `get_current_user`: the owner of the main script file.
fn fn_get_current_user(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let Some(m) = script_stat(host) else {
        return Ok(Value::str(""));
    };
    let uid = std::os::unix::fs::MetadataExt::uid(&m);
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut res: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut res) };
    if rc != 0 || res.is_null() {
        return Ok(Value::str(""));
    }
    let name = unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) };
    Ok(Value::str(name.to_bytes().to_vec()))
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    let mut put = |name: &'static str, func: crate::NativeFn, min: usize, max: usize| {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: Some(max),
            },
        );
    };
    put("array_first", fn_array_first, 1, 1);
    put("array_last", fn_array_last, 1, 1);
    put("str_increment", fn_str_increment, 1, 1);
    put("str_decrement", fn_str_decrement, 1, 1);
    put("ip2long", fn_ip2long, 1, 1);
    put("long2ip", fn_long2ip, 1, 1);
    put("gettimeofday", fn_gettimeofday, 0, 1);
    put("lcg_value", fn_lcg_value, 0, 0);
    put("zend_version", fn_zend_version, 0, 0);
    put("get_cfg_var", fn_get_cfg_var, 1, 1);
    put("get_include_path", fn_get_include_path, 0, 0);
    put("set_include_path", fn_set_include_path, 1, 1);
    put("error_clear_last", fn_null, 0, 0);
    put("cli_set_process_title", fn_cli_set_process_title, 1, 1);
    put("cli_get_process_title", fn_cli_get_process_title, 0, 0);
    put("gethostname", fn_gethostname, 0, 0);
    put("gethostbyname", fn_gethostbyname, 1, 1);
    put("getmyuid", fn_getmyuid, 0, 0);
    put("getmygid", fn_getmygid, 0, 0);
    put("getmyinode", fn_getmyinode, 0, 0);
    put("get_current_user", fn_get_current_user, 0, 0);
}
