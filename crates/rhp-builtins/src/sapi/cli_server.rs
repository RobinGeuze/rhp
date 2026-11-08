//! sapi/cli/php_cli_server.c: the built-in web server's SAPI module
//! (`cli-server`), its `server_additional_functions` and the `cli_server`
//! module (the `cli_server.color` directive).

use crate::module::no_state;
use crate::sapi::SapiDef;
use crate::sapi::{cgi, cli};
use crate::NativeFn;

/// php -S: its additional functions and the `cli_server` module, which has
/// none of its own.
pub(crate) static SAPI: SapiDef = SapiDef {
    name: "cli-server",
    functions: FUNCTIONS,
    module: Some(("cli_server", &[])),
    command_line: true,
    reset: no_state,
};

/// `server_additional_functions`, in its order.
const FUNCTIONS: &[(&str, NativeFn)] = &[
    ("cli_set_process_title", cli::fn_cli_set_process_title),
    ("cli_get_process_title", cli::fn_cli_get_process_title),
    ("apache_request_headers", cgi::fn_apache_request_headers),
    ("apache_response_headers", cgi::fn_apache_response_headers),
    ("getallheaders", cgi::fn_apache_request_headers),
];
