//! Zend engine parts generated from or ported off php-src's Zend/: the
//! language and INI parsers' Bison automata, which decide what parses and
//! how a syntax error is worded.

pub mod bison;
#[rustfmt::skip]
pub mod ini_parser_tables;
mod language_parser;
#[rustfmt::skip]
mod language_parser_tables;
pub mod object_handlers;
pub mod virtual_cwd;
pub(crate) mod zpp;

pub use language_parser::{syntax_error, ParseFailure};
