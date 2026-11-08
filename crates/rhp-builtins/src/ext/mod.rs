//! php's extensions, one folder per extension as in php-src's `ext/`.
//! Each is a [`ModuleDef`] ([`modules`]) owning its functions, constants,
//! PHP-source classes, stream wrappers, request state and object state.

use crate::module::{no_constants, no_state, no_wrappers, ModuleDef};

pub mod bcmath;
pub mod bz2;
pub mod ctype;
pub mod curl;
pub mod date;
pub mod dom;
pub mod fileinfo;
pub mod filter;
pub mod gmp;
pub mod hash;
pub mod iconv;
pub mod intl;
pub mod json;
pub mod libxml;
#[cfg(feature = "mbstring-libmbfl")]
pub mod mbstring;
pub mod opcache;
pub mod openssl;
pub mod pcntl;
pub mod pcre;
pub mod pdo;
pub mod pdo_sqlite;
pub mod phar;
pub mod posix;
pub mod random;
pub mod readline;
pub mod reflection;
pub mod session;
pub mod simplexml;
pub mod sockets;
pub mod spl;
pub mod standard;
pub mod tokenizer;
pub mod xml;
pub mod xmlreader;
pub mod xmlwriter;
pub mod zip;
pub mod zlib;

/// The object handlers the extensions register for their classes.
pub(crate) fn class_handlers() -> Vec<&'static crate::zend::object_handlers::ClassHandlers> {
    [
        curl::CLASSES,
        dom::CLASSES,
        fileinfo::CLASSES,
        gmp::CLASSES,
        openssl::CLASSES,
        random::CLASSES,
        simplexml::CLASSES,
        sockets::CLASSES,
        spl::CLASSES,
        xml::CLASSES,
        zlib::context::CLASSES,
        xmlreader::CLASSES,
        xmlwriter::CLASSES,
        zip::CLASSES,
        intl::CLASSES,
        pdo::CLASSES,
        date::CLASSES,
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The standard module, then every extension, in startup order: their
/// prelude classes compile in this order.
pub(crate) fn modules() -> Vec<ModuleDef> {
    vec![
        standard::module(),
        ModuleDef {
            name: "date",
            cli_only: false,
            functions: date::register,
            constants: date::constants,
            prelude_php: date::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: date::reset_state,
            object_freed: None,
        },
        ModuleDef {
            name: "hash",
            cli_only: false,
            functions: hash::register,
            constants: hash::constants,
            prelude_php: hash::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "bcmath",
            cli_only: false,
            functions: bcmath::register,
            constants: no_constants,
            prelude_php: "",
            wrappers: no_wrappers,
            reset: bcmath::reset_state,
            object_freed: None,
        },
        ModuleDef {
            name: "ctype",
            cli_only: false,
            functions: ctype::register,
            constants: no_constants,
            prelude_php: "",
            wrappers: no_wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "gmp",
            cli_only: false,
            functions: gmp::register,
            constants: gmp::constants,
            prelude_php: gmp::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: gmp::reset_state,
            object_freed: None,
        },
        #[cfg(feature = "mbstring-libmbfl")]
        ModuleDef {
            name: "mbstring",
            cli_only: false,
            functions: mbstring::register,
            constants: mbstring::constants,
            prelude_php: "",
            wrappers: no_wrappers,
            reset: mbstring::reset_state,
            object_freed: None,
        },
        ModuleDef {
            name: "SPL",
            cli_only: false,
            functions: spl::register,
            constants: no_constants,
            prelude_php: spl::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "Reflection",
            cli_only: false,
            functions: reflection::register,
            constants: no_constants,
            prelude_php: reflection::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "zip",
            cli_only: false,
            functions: zip::register,
            constants: no_constants,
            prelude_php: zip::prelude(),
            wrappers: zip::wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "bz2",
            cli_only: false,
            functions: bz2::register,
            constants: no_constants,
            prelude_php: "",
            wrappers: bz2::wrappers,
            reset: bz2::reset_state,
            object_freed: None,
        },
        ModuleDef {
            name: "tokenizer",
            cli_only: false,
            functions: tokenizer::register,
            constants: tokenizer::constants,
            prelude_php: tokenizer::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "zlib",
            cli_only: false,
            functions: zlib::register,
            constants: zlib::constants,
            prelude_php: zlib::context::PRELUDE_PHP,
            wrappers: zlib::wrappers,
            reset: zlib::output::reset_state,
            object_freed: None,
        },
        ModuleDef {
            name: "curl",
            cli_only: false,
            functions: curl::register,
            constants: curl::constants,
            prelude_php: curl::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: curl::reset_state,
            object_freed: Some(curl::object_freed),
        },
        ModuleDef {
            name: "openssl",
            cli_only: false,
            functions: openssl::register,
            constants: openssl::constants,
            prelude_php: openssl::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: openssl::reset_state,
            object_freed: Some(openssl::object_freed),
        },
        ModuleDef {
            name: "intl",
            cli_only: false,
            functions: intl::register,
            constants: intl::constants,
            prelude_php: intl::prelude(),
            wrappers: no_wrappers,
            reset: intl::error_reset,
            object_freed: None,
        },
        ModuleDef {
            name: "iconv",
            cli_only: false,
            functions: iconv::register,
            constants: iconv::constants,
            prelude_php: "",
            wrappers: no_wrappers,
            reset: iconv::reset_state,
            object_freed: None,
        },
        ModuleDef {
            name: "session",
            cli_only: false,
            functions: session::register,
            constants: session::constants,
            prelude_php: session::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: session::reset_state,
            object_freed: None,
        },
        ModuleDef {
            name: "Phar",
            cli_only: false,
            functions: phar::register,
            constants: no_constants,
            prelude_php: phar::PRELUDE_PHP,
            wrappers: phar::wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "posix",
            cli_only: false,
            functions: posix::register,
            constants: posix::constants,
            prelude_php: "",
            wrappers: no_wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "pcntl",
            cli_only: true,
            functions: pcntl::register,
            constants: pcntl::constants,
            prelude_php: "",
            wrappers: no_wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "PDO",
            cli_only: false,
            functions: pdo::register,
            constants: no_constants,
            prelude_php: pdo::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "pdo_sqlite",
            cli_only: false,
            functions: pdo_sqlite::register,
            constants: no_constants,
            prelude_php: pdo_sqlite::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "libxml",
            cli_only: false,
            functions: libxml::register,
            constants: libxml::constants,
            prelude_php: libxml::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: libxml::reset_state,
            object_freed: None,
        },
        ModuleDef {
            name: "dom",
            cli_only: false,
            functions: dom::register,
            constants: dom::constants,
            prelude_php: dom::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "SimpleXML",
            cli_only: false,
            functions: simplexml::register,
            constants: no_constants,
            prelude_php: simplexml::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "xml",
            cli_only: false,
            functions: xml::register,
            constants: xml::constants,
            prelude_php: xml::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: xml::reset_state,
            object_freed: None,
        },
        ModuleDef {
            name: "xmlreader",
            cli_only: false,
            functions: xmlreader::register,
            constants: no_constants,
            prelude_php: xmlreader::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "xmlwriter",
            cli_only: false,
            functions: xmlwriter::register,
            constants: no_constants,
            prelude_php: xmlwriter::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "sockets",
            cli_only: false,
            functions: sockets::register,
            constants: sockets::constants,
            prelude_php: sockets::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: sockets::reset_state,
            object_freed: None,
        },
        ModuleDef {
            name: "pcre",
            cli_only: false,
            functions: pcre::php_pcre::register,
            constants: pcre::php_pcre::preg_constants,
            prelude_php: "",
            wrappers: no_wrappers,
            reset: pcre::php_pcre::reset_state,
            object_freed: None,
        },
        ModuleDef {
            name: "json",
            cli_only: false,
            functions: json::register,
            constants: json::json_constants,
            prelude_php: "",
            wrappers: no_wrappers,
            reset: json::reset_state,
            object_freed: None,
        },
        ModuleDef {
            name: "fileinfo",
            cli_only: false,
            functions: fileinfo::register,
            constants: fileinfo::constants,
            prelude_php: fileinfo::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: no_state,
            object_freed: None,
        },
        ModuleDef {
            name: "filter",
            cli_only: false,
            functions: filter::register,
            constants: filter::constants,
            prelude_php: filter::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: filter::reset_state,
            object_freed: None,
        },
        ModuleDef {
            name: "random",
            cli_only: false,
            functions: random::register,
            constants: random::mt_rand_constants,
            prelude_php: random::PRELUDE_PHP,
            wrappers: no_wrappers,
            reset: random::reset_state,
            object_freed: None,
        },
        ModuleDef {
            name: "readline",
            cli_only: false,
            functions: readline::register,
            constants: readline::constants,
            prelude_php: "",
            wrappers: no_wrappers,
            reset: readline::reset_state,
            object_freed: None,
        },
        ModuleDef {
            name: "Zend OPcache",
            cli_only: false,
            functions: opcache::register,
            constants: no_constants,
            prelude_php: "",
            wrappers: no_wrappers,
            reset: no_state,
            object_freed: None,
        },
    ]
}
