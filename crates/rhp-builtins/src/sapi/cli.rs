//! sapi/cli/php_cli_process_title.c: `cli_set_process_title` and
//! `cli_get_process_title`.

use rhp_values::Value;

use crate::zend::zpp::str_param;
use crate::{Host, Thrown};

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
            ("cli_set_process_title", fn_cli_set_process_title),
            ("cli_get_process_title", fn_cli_get_process_title),
        ],
    );
}
