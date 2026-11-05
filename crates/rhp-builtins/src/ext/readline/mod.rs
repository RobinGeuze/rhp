//! ext/readline: `readline` and its history functions, as php without a
//! terminal library runs them.

use rhp_values::ops::deref;
use rhp_values::Value;

use crate::zend::zpp::str_param;
use crate::{Host, Thrown};

// --- readline / stream knobs --------------------------------------------------

/// `readline($prompt)`: one line from stdin without its newline, or false at
/// EOF. With a non-terminal stdin (libedit's non-interactive mode) the
/// prompt is not echoed.
fn fn_readline(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let prompt = match args.first().map(deref) {
        None | Some(Value::Null) => Vec::new(),
        Some(_) => str_param(host, args, 0, "readline", "prompt")?,
    };
    let tty = unsafe { libc::isatty(0) } == 1;
    if tty && !prompt.is_empty() {
        use std::io::Write;
        let mut so = std::io::stdout();
        let _ = so.write_all(&prompt);
        let _ = so.flush();
    }
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    loop {
        let n = unsafe { libc::read(0, b.as_mut_ptr() as *mut libc::c_void, 1) };
        if n <= 0 {
            if line.is_empty() {
                return Ok(Value::Bool(false));
            }
            break;
        }
        if b[0] == b'\n' {
            break;
        }
        line.push(b[0]);
    }
    Ok(Value::str(line))
}

fn fn_readline_add_history(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let _ = str_param(host, args, 0, "readline_add_history", "prompt")?;
    Ok(Value::Bool(true))
}

fn fn_true(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(true))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            ("readline", fn_readline),
            ("readline_add_history", fn_readline_add_history),
            ("readline_clear_history", fn_true),
        ],
    );
}
