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
pub mod ctype;
pub mod gmp;
pub mod hash;
pub mod mbstring;
pub mod openssl;
pub mod password;
pub mod phar;
pub mod reflection;
pub mod spl;
pub mod zip;
pub mod zlib;

/// Every discrete extension, in registration order.
pub fn modules() -> Vec<ModuleDef> {
    vec![
        ModuleDef {
            name: "hash",
            functions: hash::register,
            constants: no_constants,
            prelude_php: "",
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
            name: "zlib",
            functions: zlib::register,
            constants: zlib::constants,
            prelude_php: "",
            wrappers: no_wrappers,
        },
        ModuleDef {
            name: "openssl",
            functions: openssl::register,
            constants: openssl::constants,
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
    ]
}
