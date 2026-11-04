//! php's standard extension (php-src ext/standard/), one file per php-src
//! file.

pub mod array;
pub mod assert;
pub mod basic_functions;
pub mod crypt;
pub mod dir;
pub mod dns;
pub mod exec;
pub mod file;
pub mod filestat;
pub mod formatted_print;
pub mod fsock;
pub mod head;
pub mod hrtime;
pub mod html;
#[rustfmt::skip]
pub mod html_tables;
pub mod http;
pub mod http_fopen_wrapper;
pub mod info;
pub mod levenshtein;
pub mod link;
pub mod math;
pub mod microtime;
pub mod pack;
pub mod pageinfo;
pub mod proc_open;
pub mod scanf;
pub mod soundex;
pub mod streamsfuncs;
pub mod string;
pub mod r#type;
pub mod uniqid;
pub mod url;
pub mod var;
pub mod versioning;
