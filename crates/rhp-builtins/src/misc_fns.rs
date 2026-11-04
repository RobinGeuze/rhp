//! `is_iterable` / `is_countable`.
//!
//! (The `ctype_*` tests moved to `ext::ctype`; the JSON builtins live in
//! `json_fns`.) All behavior verified byte-for-byte against the local php 8.5.8.

use rhp_values::Value;

use crate::{inet, Host, Thrown};

// ---------------------------------------------------------------------------
// inet_pton / inet_ntop
// ---------------------------------------------------------------------------

/// `inet_pton(string $ip): string|false` (basic_functions.c): AF_INET6 when the
/// text has a ':', otherwise AF_INET if it has a '.'.
fn fn_inet_pton(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let ip = crate::zend::zpp::path_param(host, args, 0, "inet_pton", "ip")?;
    let packed = if ip.contains(&b':') {
        inet::pton6(&ip).map(|a| a.to_vec())
    } else if ip.contains(&b'.') {
        inet::pton4(&ip).map(|a| a.to_vec())
    } else {
        None
    };
    Ok(packed.map_or(Value::Bool(false), Value::str))
}

/// `inet_ntop(string $ip): string|false`: a 4- or 16-byte address as text.
fn fn_inet_ntop(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let b = crate::zend::zpp::str_param(host, args, 0, "inet_ntop", "ip")?;
    Ok(inet::ntop(&b).map_or(Value::Bool(false), Value::str))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

/// Register this module's builtins into the engine's table.
pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[("inet_pton", fn_inet_pton), ("inet_ntop", fn_inet_ntop)],
    );
}
