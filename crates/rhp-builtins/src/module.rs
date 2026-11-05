//! The module abstraction, php's `zend_module_entry`: everything rhp
//! registers for one php module (`Core`, `standard`, `date`, …) and the
//! hooks the engine calls into it. [`crate::registry::modules`] lists
//! every module; the registry, constants, prelude, stream wrappers, the
//! per-request reset and the object free hook are all built from that list.

use rhp_values::Value;

use crate::main::streams::WrapperReg;

/// One php module.
pub struct ModuleDef {
    /// php's module name, as `get_loaded_extensions()` and
    /// `extension_loaded()` know it.
    pub name: &'static str,
    /// PHP_NEW_EXTENSION's `cli` SAPI class: php builds the module into the
    /// command-line SAPIs (and embed) only, and rhp loads it only under
    /// php-cli and php-cgi (pcntl: signal handlers and fork belong to the
    /// whole process, so a server that embeds rhp never gets them). The
    /// SAPI module supplies its functions (sapi/).
    pub cli_only: bool,
    /// Insert the module's native functions into the registry.
    pub functions: fn(&mut crate::Registry),
    /// The module's `(name, value)` constants (MINIT's
    /// `REGISTER_*_CONSTANT`). Each constant is defined by one module.
    pub constants: fn() -> Vec<(&'static str, Value)>,
    /// PHP-source class/interface declarations (compiled into the prelude).
    /// Methods may delegate to the module's own `__<ext>_*` native functions.
    pub prelude_php: &'static str,
    /// Built-in stream wrappers installed at VM start.
    pub wrappers: fn() -> Vec<WrapperReg>,
    /// Back to a fresh request (php's RSHUTDOWN and RINIT): reset every
    /// `thread_local!` of the module that holds request state (see
    /// docs/design/request-state.md). Mandatory: a module without such
    /// state says so with [`no_state`].
    pub reset: fn(),
    /// The object with this id was freed: drop what the module keeps for
    /// it by object id. Mandatory: `None` when it keeps nothing (the hook
    /// runs on every object free, so only the modules that have one are
    /// called).
    pub object_freed: Option<fn(u32)>,
}

/// No-op constant provider (module declares no constants).
pub fn no_constants() -> Vec<(&'static str, Value)> {
    Vec::new()
}

/// No-op wrapper provider (module registers no stream wrappers).
pub fn no_wrappers() -> Vec<WrapperReg> {
    Vec::new()
}

/// The reset of a module that keeps no request state.
pub fn no_state() {}

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
