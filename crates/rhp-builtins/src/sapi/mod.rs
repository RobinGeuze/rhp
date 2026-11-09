//! php-src's sapi/: what a SAPI brings to the engine besides its callbacks
//! (`main::sapi::Sapi`): its own functions, and the modules only the
//! command-line SAPIs load. The running SAPI's name selects its
//! [`SapiDef`] (`main::sapi::set_sapi`); a SAPI without one (an embedding
//! server) adds nothing, or php-fpm's functions when it asks for them
//! ([`server`]). The registry holds every SAPI's functions, each
//! marked with the SAPIs that supply it.

use crate::NativeFn;

pub mod cgi;
pub mod cli;
pub mod cli_server;
pub mod server;

/// A SAPI module's contribution: php's `sapi_module.additional_functions`
/// and the SAPI's own extension module.
pub(crate) struct SapiDef {
    /// `php_sapi_name()`.
    pub name: &'static str,
    /// `additional_functions`, which php registers as part of `standard`.
    pub functions: &'static [(&'static str, NativeFn)],
    /// The SAPI's own module and its functions, in php's order
    /// (`get_loaded_extensions()` lists the module last).
    pub module: Option<(&'static str, &'static [(&'static str, NativeFn)])>,
    /// A command-line SAPI (php-cli, php-cgi): it loads the `cli_only`
    /// modules (pcntl).
    pub command_line: bool,
    /// Back to a fresh request: the SAPI's `thread_local!`s. Mandatory, as
    /// a module's; every SAPI's runs, whichever SAPI the next request uses.
    pub reset: fn(),
}

/// Every SAPI rhp has functions for; a SAPI's position is its bit in
/// `Builtin::sapis`.
pub(crate) static SAPIS: [&SapiDef; 4] = [&cli::SAPI, &cgi::SAPI, &cli_server::SAPI, &server::SAPI];

/// The SAPI named `name` and its bit, if rhp has one.
pub(crate) fn sapi_def(name: &str) -> Option<(u8, &'static SapiDef)> {
    SAPIS
        .iter()
        .position(|d| d.name == name && !name.is_empty())
        .map(|i| (1 << i, SAPIS[i]))
}

/// The embedding server's functions ([`server`]) and their bit.
pub(crate) fn server_def() -> (u8, &'static SapiDef) {
    let i = SAPIS.len() - 1;
    (1 << i, SAPIS[i])
}
