//! php's standard extension (php-src ext/standard/), one file per php-src
//! file.

pub mod array;
pub mod assert;
pub mod crypt;
pub mod dns;
pub mod formatted_print;
pub mod hrtime;
pub mod html;
#[rustfmt::skip]
pub mod html_tables;
pub mod info;
pub mod levenshtein;
pub mod math;
pub mod microtime;
pub mod pack;
pub mod pageinfo;
pub mod scanf;
pub mod soundex;
pub mod string;
pub mod r#type;
pub mod uniqid;
pub mod var;
pub mod versioning;
