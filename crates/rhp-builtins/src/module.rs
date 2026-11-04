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

/// An internal output handler an extension registers under a name
/// (php_output_handler_alias_register): `ob_start("name")` and the
/// `output_handler` directive start it instead of looking up a function. It
/// refuses to start on top of a handler named in `conflicts`
/// (php_output_handler_conflict_register).
pub struct OutputHandlerAlias {
    pub name: &'static str,
    pub func: OutputHandlerFn,
    pub conflicts: &'static [&'static str],
}

/// php_output_handler_func_t: run the handler; `false` is FAILURE.
pub type OutputHandlerFn = fn(&mut dyn crate::Host, &mut OutputHandlerCtx) -> bool;

/// php_output_context, plus what an internal handler asks of the output
/// layer.
pub struct OutputHandlerCtx<'a> {
    /// The PHP_OUTPUT_HANDLER_* op bits.
    pub op: i64,
    /// The handler's buffered input.
    pub input: &'a [u8],
    /// `php_output_get_status() & PHP_OUTPUT_SENT`.
    pub output_sent: bool,
    /// What the handler passes on.
    pub out: Vec<u8>,
    /// php_output_handler_hook(PHP_OUTPUT_HANDLER_HOOK_IMMUTABLE): the
    /// handler can no longer be removed or cleaned.
    pub immutable: bool,
    /// Diagnostics (level, message without the `func(): ` prefix) for the
    /// output layer to raise as php_error_docref does, while the handler
    /// still counts as running.
    pub diags: Vec<(i64, String)>,
}

/// The internal output handler registered as `name` (case-sensitive, like
/// php's alias table).
pub fn output_handler_alias(name: &[u8]) -> Option<&'static OutputHandlerAlias> {
    crate::ext::iconv::OUTPUT_HANDLERS
        .iter()
        .find(|a| a.name.as_bytes() == name)
}
