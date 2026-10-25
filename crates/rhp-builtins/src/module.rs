//! The extension-module abstraction. A module is self-contained: it owns its
//! native functions, its constants, its PHP-source classes (compiled into the
//! prelude), and any stream wrappers it registers. The discrete extensions under
//! `ext/<name>/` each expose a [`ModuleDef`]; [`crate::ext::modules`] lists them,
//! and the registry / constants / prelude / wrapper plumbing folds them in.
//!
//! Core stdlib groups (`str_fns`, `math_fns`, …) stay registered directly for
//! now — only discrete extensions use this richer contract.

use std::collections::HashMap;

use rhp_values::Value;

use crate::streams::WrapperReg;
use crate::Builtin;

/// A full extension description.
pub struct ModuleDef {
    pub name: &'static str,
    /// Insert the module's native functions into the registry.
    pub functions: fn(&mut HashMap<&'static str, Builtin>),
    /// The module's `(name, value)` constants.
    pub constants: fn() -> Vec<(&'static str, Value)>,
    /// PHP-source class/interface declarations (compiled into the prelude).
    /// Methods may delegate to the module's own `__<ext>_*` native functions.
    pub prelude_php: &'static str,
    /// Built-in stream wrappers installed at VM start.
    pub wrappers: fn() -> Vec<WrapperReg>,
}

/// No-op constant provider (module declares no constants).
pub fn no_constants() -> Vec<(&'static str, Value)> {
    Vec::new()
}

/// No-op wrapper provider (module registers no stream wrappers).
pub fn no_wrappers() -> Vec<WrapperReg> {
    Vec::new()
}
