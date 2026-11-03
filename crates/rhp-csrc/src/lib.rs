//! C that rhp compiles from source and links statically: php-src's own C at
//! the pinned tag (`vendor/php-src`, written by `tools/sync-php-src-c.sh`)
//! and frozen libraries whose only release is the one php links
//! (`vendor/argon2`). All php-src C shares one Zend shim, `shim/zend_shim.h`.
//!
//! One cargo feature per library. Each module wraps its C in safe functions
//! that take what php passes and return what php's C returns.

#[cfg(feature = "argon2")]
pub mod argon2;
#[cfg(feature = "crypt")]
pub mod crypt;
