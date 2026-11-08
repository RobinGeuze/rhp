//! php-src's Zend/: the engine's builtin functions, zend_parse_parameters,
//! object handlers, exceptions, closures, the virtual cwd, and the INI
//! scanner and parser. The language and INI parsers' Bison automata decide
//! what parses and how a syntax error is worded.

pub mod bison;
pub mod builtin_functions;
pub mod call_stack;
pub mod closures;
pub(crate) mod constants;
pub mod exceptions;
pub mod fibers;
pub mod generators;
pub mod highlight;
pub(crate) mod ini_parser;
#[rustfmt::skip]
pub mod ini_parser_tables;
pub(crate) mod ini_scanner;
pub mod internal_iterator;
mod language_parser;
pub mod multibyte;
#[rustfmt::skip]
mod language_parser_tables;
pub mod object_handlers;
pub mod operators;
pub mod timeout;
pub mod virtual_cwd;
pub mod weakrefs;
pub(crate) mod zpp;

pub use language_parser::{syntax_error, syntax_error_ex, take_parse_warnings, ParseFailure};

thread_local! {
    /// EG(assertions): the zend.assertions ini as a number.
    static ASSERTIONS: std::cell::Cell<i64> = const { std::cell::Cell::new(1) };
}

/// EG(assertions): 1 runs `assert()`, 0 skips it at run time, -1 compiles
/// it away.
pub fn assertions() -> i64 {
    ASSERTIONS.with(|a| a.get())
}

pub(crate) fn set_assertions(v: i64) {
    ASSERTIONS.with(|a| a.set(v));
}
