//! The functions an embedding server's SAPI can ask for
//! ([`Sapi::server_functions`](crate::main::sapi::Sapi::server_functions)):
//! php-fpm's (sapi/fpm/fpm/fpm_main.c), which FrankenPHP has too:
//! `fastcgi_finish_request`, `apache_request_headers` (`getallheaders`) and
//! FrankenPHP's `apache_response_headers`, plus FrankenPHP's worker-mode
//! `frankenphp_handle_request` and `frankenphp_finish_request`, so the
//! runtimes written for it (Symfony Runtime, Laravel Octane) run as they
//! are. A SAPI with a name of its own gets them in place of the none it
//! would have.

use rhp_values::{Thrown, Value};

use crate::module::no_state;
use crate::sapi::{cgi, SapiDef};
use crate::{Host, NativeFn};

/// An embedding server's SAPI with [`Sapi::server_functions`]: its
/// additional functions.
///
/// [`Sapi::server_functions`]: crate::main::sapi::Sapi::server_functions
pub(crate) static SAPI: SapiDef = SapiDef {
    name: "",
    functions: FUNCTIONS,
    module: None,
    command_line: false,
    reset: no_state,
};

const FUNCTIONS: &[(&str, NativeFn)] = &[
    ("fastcgi_finish_request", fn_fastcgi_finish_request),
    ("apache_request_headers", cgi::fn_apache_request_headers),
    ("getallheaders", cgi::fn_apache_request_headers),
    ("apache_response_headers", cgi::fn_apache_response_headers),
    ("frankenphp_finish_request", fn_fastcgi_finish_request),
    ("frankenphp_handle_request", fn_frankenphp_handle_request),
];

/// fpm_main.c's `fastcgi_finish_request` (FrankenPHP's
/// `frankenphp_finish_request`): flush every output buffer, send the
/// headers and complete the response; the script runs on with its output
/// discarded. False when the response was already complete.
fn fn_fastcgi_finish_request(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    Ok(Value::Bool(host.finish_response()))
}

/// FrankenPHP's `frankenphp_handle_request(callable $callback): bool`:
/// wait for the worker's next request, run `$callback` for it with the
/// request's superglobals, then complete its response. False when the
/// worker should stop; outside worker mode a RuntimeException.
fn fn_frankenphp_handle_request(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = "frankenphp_handle_request";
    if args.len() != 1 {
        return Err(Thrown::error(
            "ArgumentCountError",
            format!("{name}() expects exactly 1 argument, {} given", args.len()),
        ));
    }
    let handler = rhp_values::ops::deref(&args[0]);
    crate::zend::zpp::require_callback(host, name, 1, &handler)?;
    host.worker_handle_request(&handler).map(Value::Bool)
}
