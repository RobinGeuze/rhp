//! Discrete PHP extensions, one folder per extension — mirroring php-src's
//! `ext/<name>/` layout. Each submodule owns its functions and exposes a
//! `register(&mut HashMap<&str, Builtin>)`; [`register`] here aggregates them.
//!
//! (The broad core library — string/array/math/etc., php's `ext/standard` — still
//! lives in the flat `*_fns` modules for now.)

use std::collections::HashMap;

use crate::Builtin;

pub mod bcmath;
pub mod ctype;
pub mod hash;

/// Register every extension's builtins into the shared table.
pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    hash::register(m);
    bcmath::register(m);
    ctype::register(m);
}
