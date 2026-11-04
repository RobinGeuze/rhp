//! Small standard-library / system builtins: `array_first`/`array_last`,
//! `str_increment`/`str_decrement`, `ip2long`/`long2ip`, `gettimeofday`,
//! `lcg_value`, `zend_version`, `get_cfg_var`, `get_include_path`/
//! `set_include_path`, `error_clear_last`, `cli_set_process_title`/
//! `cli_get_process_title`, `gethostname`, `gethostbyname`, `getmyuid`/
//! `getmygid`/`getmyinode`, `get_current_user`.

use rhp_values::Value;

use crate::ext::standard::pageinfo::script_stat;
use crate::zend::zpp::{int_param, str_param};
use crate::{Host, Thrown};

fn fn_ip2long(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = crate::zend::zpp::path_param(host, args, 0, "ip2long", "ip")?;
    Ok(match crate::inet::pton4(&s) {
        Some(a) if !s.is_empty() => Value::Int(u32::from_be_bytes(a) as i64),
        _ => Value::Bool(false),
    })
}

fn fn_long2ip(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let n = int_param(host, args, 0, "long2ip", "ip", 0)? as u64 as u32;
    let text = crate::inet::ntop(&n.to_be_bytes()).expect("inet_ntop of an IPv4 address");
    Ok(Value::str(text))
}

fn fn_lcg_value(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    crate::deprecated_fn(
        host,
        "lcg_value",
        "8.4",
        "use \\Random\\Randomizer::getFloat() instead",
    )?;
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    // 53 random bits → [0, 1).
    let n = u64::from_le_bytes(b) >> 11;
    Ok(Value::Float(n as f64 / (1u64 << 53) as f64))
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

fn fn_cli_set_process_title(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let t = str_param(host, args, 0, "cli_set_process_title", "title")?;
    TITLE.with(|c| *c.borrow_mut() = Some(t));
    Ok(Value::Bool(true))
}

thread_local! {
    static TITLE: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn reset_state() {
    TITLE.with(|c| *c.borrow_mut() = None);
}

fn fn_cli_get_process_title(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(
        TITLE.with(|c| c.borrow().clone()).unwrap_or_default(),
    ))
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

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("ip2long", fn_ip2long),
            ("long2ip", fn_long2ip),
            ("lcg_value", fn_lcg_value),
            ("get_cfg_var", fn_get_cfg_var),
            ("get_include_path", fn_get_include_path),
            ("set_include_path", fn_set_include_path),
            ("cli_set_process_title", fn_cli_set_process_title),
            ("cli_get_process_title", fn_cli_get_process_title),
            ("get_current_user", fn_get_current_user),
        ],
    );
}
