//! php-src's Zend/: the engine's builtin functions, zend_parse_parameters,
//! object handlers, exceptions, closures, the virtual cwd, and the INI
//! scanner and parser. The language and INI parsers' Bison automata decide
//! what parses and how a syntax error is worded.

pub mod bison;
pub mod builtin_functions;
pub mod closures;
pub(crate) mod constants;
pub mod exceptions;
pub(crate) mod ini_parser;
#[rustfmt::skip]
pub mod ini_parser_tables;
pub(crate) mod ini_scanner;
mod language_parser;
#[rustfmt::skip]
mod language_parser_tables;
pub mod object_handlers;
pub mod operators;
pub mod virtual_cwd;
pub(crate) mod zpp;

pub use language_parser::{syntax_error, ParseFailure};
