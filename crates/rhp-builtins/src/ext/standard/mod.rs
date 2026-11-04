//! php's standard extension (php-src ext/standard/), one file per php-src
//! file.

pub mod array;
pub mod crypt;
pub mod formatted_print;
pub mod html;
#[rustfmt::skip]
pub mod html_tables;
pub mod info;
pub mod math;
pub mod string;
pub mod r#type;
pub mod var;
