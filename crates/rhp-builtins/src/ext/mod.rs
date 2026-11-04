//! Discrete PHP extensions, one folder per extension — mirroring php-src's
//! `ext/<name>/` layout. Each extension is a self-contained [`ModuleDef`]
//! ([`modules`]) owning its functions, constants, PHP-source classes, and stream
//! wrappers; the registry / constants / prelude / wrapper plumbing in `lib.rs`
//! folds every module in.
//!
//! (The broad core library — string/array/math/etc., php's `ext/standard` — still
//! lives in the flat `*_fns` modules for now.)

use crate::module::{no_constants, no_wrappers, ModuleDef};

pub mod bcmath;
pub mod bz2;
pub mod ctype;
pub mod curl;
pub mod date;
pub mod dom;
pub mod gmp;
pub mod hash;
pub mod iconv;
pub mod intl;
pub mod libxml;
pub mod mbstring;
pub mod openssl;
pub mod password;
pub mod pcntl;
pub mod phar;
pub mod posix;
pub mod reflection;
pub mod simplexml;
pub mod sockets;
pub mod spl;
pub mod tokenizer;
pub mod xml;
pub mod xmlreader;
pub mod xmlwriter;
pub mod zip;
pub mod zlib;
pub mod zpp;

/// The object handlers the extensions register for their classes.
pub(crate) fn class_handlers() -> Vec<&'static crate::object::ClassHandlers> {
    [
        curl::CLASSES,
        gmp::CLASSES,
        openssl::CLASSES,
        simplexml::CLASSES,
        sockets::CLASSES,
        spl::CLASSES,
        xml::CLASSES,
        xmlreader::CLASSES,
        xmlwriter::CLASSES,
        zip::CLASSES,
        crate::dt_oop_fns::CLASSES,
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Every discrete extension, in registration order.
pub fn modules() -> Vec<ModuleDef> {
    vec![
        ModuleDef {
            name: "date",
            functions: date::register,
            constants: date::constants,
            prelude_php: "",
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "hash",
            functions: hash::register,
            constants: hash::constants,
            prelude_php: hash::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "bcmath",
            functions: bcmath::register,
            constants: no_constants,
            prelude_php: "",
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "ctype",
            functions: ctype::register,
            constants: no_constants,
            prelude_php: "",
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "gmp",
            functions: gmp::register,
            constants: gmp::constants,
            prelude_php: gmp::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "mbstring",
            functions: mbstring::register,
            constants: no_constants,
            prelude_php: "",
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "password",
            functions: password::register,
            constants: password::constants,
            prelude_php: "",
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "spl",
            functions: spl::register,
            constants: no_constants,
            prelude_php: spl::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "reflection",
            functions: reflection::register,
            constants: no_constants,
            prelude_php: reflection::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "zip",
            functions: zip::register,
            constants: no_constants,
            prelude_php: zip::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "bz2",
            functions: bz2::register,
            constants: no_constants,
            prelude_php: "",
            wrappers: bz2::wrappers,
        },
        ModuleDef {
            name: "tokenizer",
            functions: tokenizer::register,
            constants: tokenizer::constants,
            prelude_php: tokenizer::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "zlib",
            functions: zlib::register,
            constants: zlib::constants,
            prelude_php: "",
            wrappers: zlib::wrappers,
        },
        ModuleDef {
            name: "curl",
            functions: curl::register,
            constants: curl::constants,
            prelude_php: curl::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "openssl",
            functions: openssl::register,
            constants: openssl::constants,
            prelude_php: openssl::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "intl",
            functions: intl::register,
            constants: intl::constants,
            prelude_php: intl::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "iconv",
            functions: iconv::register,
            constants: iconv::constants,
            prelude_php: "",
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "phar",
            functions: phar::register,
            constants: no_constants,
            prelude_php: phar::PRELUDE_PHP,
            wrappers: phar::wrappers,
        },
        ModuleDef {
            name: "posix",
            functions: posix::register,
            constants: posix::constants,
            prelude_php: "",
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "pcntl",
            functions: pcntl::register,
            constants: pcntl::constants,
            prelude_php: "",
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "libxml",
            functions: libxml::register,
            constants: libxml::constants,
            prelude_php: libxml::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "dom",
            functions: dom::register,
            constants: dom::constants,
            prelude_php: dom::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "SimpleXML",
            functions: simplexml::register,
            constants: no_constants,
            prelude_php: simplexml::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "xml",
            functions: xml::register,
            constants: xml::constants,
            prelude_php: xml::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "xmlreader",
            functions: xmlreader::register,
            constants: no_constants,
            prelude_php: xmlreader::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "xmlwriter",
            functions: xmlwriter::register,
            constants: no_constants,
            prelude_php: xmlwriter::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "sockets",
            functions: sockets::register,
            constants: sockets::constants,
            prelude_php: sockets::PRELUDE_PHP,
            wrappers: no_wrappers,
        },
    ]
}
