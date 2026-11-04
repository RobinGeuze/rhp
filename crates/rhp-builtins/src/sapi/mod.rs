//! php-src's sapi/: builtins a SAPI module defines itself.
//! [`crate::main::sapi::builtin_hidden`] hides them under the other SAPIs.

pub mod cgi;
pub mod cli;
