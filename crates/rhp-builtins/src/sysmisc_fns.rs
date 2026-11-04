//! Small standard-library / system builtins: `array_first`/`array_last`,
//! `str_increment`/`str_decrement`, `ip2long`/`long2ip`, `gettimeofday`,
//! `lcg_value`, `zend_version`, `get_cfg_var`, `get_include_path`/
//! `set_include_path`, `error_clear_last`, `cli_set_process_title`/
//! `cli_get_process_title`, `gethostname`, `gethostbyname`, `getmyuid`/
//! `getmygid`/`getmyinode`, `get_current_user`.

use rhp_values::Value;

use crate::zend::zpp::str_param;
use crate::{Host, Thrown};

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

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("lcg_value", fn_lcg_value),
            ("cli_set_process_title", fn_cli_set_process_title),
            ("cli_get_process_title", fn_cli_get_process_title),
        ],
    );
}
