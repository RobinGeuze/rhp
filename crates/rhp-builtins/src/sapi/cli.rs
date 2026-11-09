//! sapi/cli/: php-cli's SAPI module, and php_cli_process_title.c's
//! `cli_set_process_title` and `cli_get_process_title`.

use rhp_values::Value;

use crate::sapi::SapiDef;
use crate::zend::zpp::str_param;
use crate::{Host, NativeFn, Thrown};

/// php-cli: its additional functions.
pub(crate) static SAPI: SapiDef = SapiDef {
    name: "cli",
    functions: FUNCTIONS,
    module: None,
    command_line: true,
    reset: reset_state,
};

const FUNCTIONS: &[(&str, NativeFn)] = &[
    ("dl", crate::ext::standard::dl::fn_dl),
    ("cli_set_process_title", fn_cli_set_process_title),
    ("cli_get_process_title", fn_cli_get_process_title),
];

pub(crate) fn fn_cli_set_process_title(
    host: &mut dyn Host,
    args: &[Value],
) -> Result<Value, Thrown> {
    let t = str_param(host, args, 0, "cli_set_process_title", "title")?;
    TITLE.with(|c| *c.borrow_mut() = Some(t));
    Ok(Value::Bool(true))
}

thread_local! {
    static TITLE: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
}

fn reset_state() {
    TITLE.with(|c| *c.borrow_mut() = None);
}

pub(crate) fn fn_cli_get_process_title(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::str(
        TITLE.with(|c| c.borrow().clone()).unwrap_or_default(),
    ))
}
