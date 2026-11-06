//! Every module ([`modules`]), and what the engine builds from them: the
//! builtin function table (php names to native functions, with php's
//! arity), the constant table, the PHP prelude, the stream wrappers, and
//! the hooks it calls into every module: request reset, object freed,
//! constant deprecation.

use std::collections::HashMap;
use std::sync::OnceLock;

use rhp_values::Value;

use crate::module::{no_wrappers, ModuleDef};
use crate::{ext, main, zend, Host, Thrown, WrapperReg};

/// Every module, in startup order: Core, standard, then the extensions.
/// The prelude compiles in this order (Core first, so its class indices
/// never change), and [`reset_request_state`] resets them in it. Built
/// once per process, outside any request's memory.
pub(crate) fn modules() -> &'static [ModuleDef] {
    static MODULES: OnceLock<Vec<ModuleDef>> = OnceLock::new();
    MODULES.get_or_init(|| {
        rhp_values::mem::uncounted(|| {
            let mut v = vec![core()];
            v.extend(ext::modules());
            v
        })
    })
}

/// php's Core: the engine's own functions (zend/), and the constants and
/// request state of main/, which main.c registers under Core.
fn core() -> ModuleDef {
    ModuleDef {
        name: "Core",
        cli_only: false,
        functions: |m| {
            zend::builtin_functions::register(m);
            zend::exceptions::register(m);
            zend::closures::register(m);
            zend::zpp::register(m);
        },
        constants: || {
            [
                zend::constants::constants(),
                main::constants(),
                main::output::constants(),
                main::rfc1867::constants(),
            ]
            .concat()
        },
        prelude_php: include_str!("core_prelude.php"),
        wrappers: no_wrappers,
        reset: || {
            zend::virtual_cwd::reset_state();
            main::reset_state();
        },
        object_freed: None,
    }
}

/// Whether module `md` is loaded under the running SAPI (a `cli_only` one
/// only under php-cli and php-cgi).
fn loaded(md: &ModuleDef) -> bool {
    !md.cli_only || main::sapi::is_command_line()
}

/// Whether the module php calls `name` is loaded under the running SAPI.
pub(crate) fn module_loaded(name: &str) -> bool {
    modules().iter().any(|md| md.name == name && loaded(md))
}

/// The E_DEPRECATED message for reading a `#[\Deprecated]` extension
/// constant, if `name` is one.
pub fn constant_deprecation(name: &[u8]) -> Option<String> {
    if let b"FILE_BINARY" | b"FILE_TEXT" = name {
        return Some(format!(
            "Constant {} is deprecated since 8.1, as the constant has no effect",
            String::from_utf8_lossy(name)
        ));
    }
    ext::curl::constant_deprecation(name)
        .or_else(|| ext::date::constant_deprecation(name))
        .or_else(|| ext::dom::constant_deprecation(name))
        .or_else(|| ext::standard::assert::constant_deprecation(name))
}

/// The value php's compiler substitutes for the global constant `name`
/// (zend_try_ct_eval_const): a builtin constant that is not deprecated.
pub fn ct_constant(name: &[u8]) -> Option<Value> {
    if constant_deprecation(name).is_some() {
        return None;
    }
    constants()
        .into_iter()
        .find(|(n, _)| n.as_bytes() == name)
        .map(|(_, v)| v)
}

/// The E_DEPRECATED a fetch of a built-in class's `#[\Deprecated]`
/// constant raises (`class` is the declaring class).
pub fn class_constant_deprecation(class: &[u8], name: &[u8]) -> Option<String> {
    ext::date::class_constant_deprecation(class, name)
        .or_else(|| ext::zip::class_constant_deprecation(class, name))
}

/// The engine's object free hook (`rhp_values::set_object_free_hook`): the
/// modules that keep state by object id drop it.
pub fn object_freed(id: u32) {
    static HOOKS: OnceLock<Vec<fn(u32)>> = OnceLock::new();
    let hooks = HOOKS.get_or_init(|| modules().iter().filter_map(|md| md.object_freed).collect());
    for hook in hooks {
        hook(id);
    }
}

/// The refcount-0 destructor of a native (non-stream) resource, given its
/// payload: a process closes its pipes and reaps the child if it already
/// exited (proc_open_rsrc_dtor); anything else is just dropped.
pub fn native_resource_dtor(host: &mut dyn Host, data: Box<dyn std::any::Any>) {
    ext::standard::proc_open::proc_dtor(host, data);
}

/// A native builtin function over owned values.
pub type NativeFn = fn(&mut dyn Host, &[Value]) -> Result<Value, Thrown>;

/// Registry entry: a native function plus its arity bounds.
pub struct Builtin {
    pub name: &'static str,
    pub func: NativeFn,
    pub min_args: usize,
    /// `None` = variadic (no upper bound).
    pub max_args: Option<usize>,
    /// The SAPIs that supply the function, one bit per position in
    /// `sapi::SAPIS`; 0 for a function every SAPI has.
    pub sapis: u8,
}

/// The builtin table under construction, keyed by lowercase name.
pub(crate) type Registry = HashMap<&'static str, Builtin>;

/// Register engine helpers: `__` builtins only prelude code calls, with no
/// stub row, so their arity (min, max; `None` = variadic) is given here.
pub(crate) fn register_helpers(
    m: &mut Registry,
    fns: &[(&'static str, NativeFn, usize, Option<usize>)],
) {
    for &(name, func, min_args, max_args) in fns {
        debug_assert!(name.starts_with("__"), "{name} is not an engine helper");
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args,
                max_args,
                sapis: 0,
            },
        );
    }
}

/// Functions whose php body requires more arguments than their arginfo
/// (`array_udiff(): expects at least 2 arguments`; openssl_cms_verify's zpp
/// wants `$flags`).
const BODY_MIN: &[(&str, usize)] = &[
    ("array_diff_uassoc", 2),
    ("array_diff_ukey", 2),
    ("array_intersect_uassoc", 2),
    ("array_intersect_ukey", 2),
    ("array_udiff", 2),
    ("array_udiff_assoc", 2),
    ("array_udiff_uassoc", 3),
    ("array_uintersect", 2),
    ("array_uintersect_assoc", 2),
    ("array_uintersect_uassoc", 3),
    ("openssl_cms_verify", 2),
];

/// Reset every module's per-request state (its `reset` hook) and every
/// SAPI module's, then the engine's list of dropped streams, which the
/// resets may add to. Engines call this on construction and when they
/// drop, so state never leaks from one script run to the next on the same
/// thread.
pub fn reset_request_state() {
    for md in modules() {
        (md.reset)();
    }
    for sapi in &crate::sapi::SAPIS {
        (sapi.reset)();
    }
    rhp_values::reset_dropped_streams();
}

/// Every builtin function, keyed by lowercase name (PHP function names are
/// case-insensitive): every module's but the `cli_only` ones, for every
/// SAPI; then each SAPI's own functions and, for a command-line SAPI, the
/// `cli_only` modules', marked with the SAPIs that have them
/// ([`Builtin::sapis`]). [`lookup_lc`] finds only the running SAPI's.
pub fn registry() -> &'static HashMap<&'static str, Builtin> {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(|| {
        let mut m = Registry::new();
        for md in modules().iter().filter(|md| !md.cli_only) {
            (md.functions)(&mut m);
        }
        for (i, sapi) in crate::sapi::SAPIS.iter().enumerate() {
            let bit = 1 << i;
            let mut own = Registry::new();
            register(&mut own, sapi.functions);
            if let Some((_, fns)) = sapi.module {
                register(&mut own, fns);
            }
            if sapi.command_line {
                for md in modules().iter().filter(|md| md.cli_only) {
                    (md.functions)(&mut own);
                }
            }
            for (name, b) in own {
                m.entry(name)
                    .and_modify(|e| e.sapis |= bit)
                    .or_insert(Builtin { sapis: bit, ..b });
            }
        }
        m
    })
}

/// Whether the running SAPI has builtin `b`, and disable_functions left it.
fn available(b: &Builtin) -> bool {
    (b.sapis == 0 || b.sapis & main::sapi::sapi_bit() != 0) && !is_disabled(b.name)
}

thread_local! {
    /// Functions removed by disable_functions (zend_disable_functions), by
    /// lowercase name. Startup state: set before anything compiles.
    static DISABLED: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(Vec::new()) };
    static ANY_DISABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether disable_functions removed builtin `name_lc`.
fn is_disabled(name_lc: &str) -> bool {
    ANY_DISABLED.with(|a| a.get()) && DISABLED.with(|d| d.borrow().contains(&name_lc))
}

/// zend_disable_functions: drop each name in `list` (separated by spaces
/// or commas) from the function table. exit, die and clone can't be
/// disabled: the warnings (printed with the rest of the list, as php's
/// `%s` does) are returned.
pub fn disable_functions(list: &[u8]) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut start = None;
    let mut disable = |s: usize, e: usize| {
        let name = &list[s..e];
        if matches!(name, b"exit" | b"die" | b"clone") {
            let rest = &list[s..];
            let rest = rest.split(|&b| b == 0).next().unwrap_or(rest);
            warnings.push(format!(
                "Cannot disable function {}()",
                String::from_utf8_lossy(rest)
            ));
            return;
        }
        // The function table is keyed by lowercase name: an exact match.
        let Ok(name) = std::str::from_utf8(name) else {
            return;
        };
        if let Some((&key, _)) = registry().get_key_value(name) {
            DISABLED.with(|d| d.borrow_mut().push(key));
            ANY_DISABLED.with(|a| a.set(true));
        }
    };
    for (i, &c) in list.iter().enumerate() {
        match c {
            b' ' | b',' => {
                if let Some(s) = start.take() {
                    disable(s, i);
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
    }
    if let Some(s) = start {
        disable(s, list.len());
    }
    warnings
}

/// Forget disable_functions (a new engine).
pub(crate) fn reset_disabled() {
    DISABLED.with(|d| d.borrow_mut().clear());
    ANY_DISABLED.with(|a| a.set(false));
}

/// Every builtin function of the running SAPI.
pub fn functions() -> impl Iterator<Item = &'static Builtin> {
    registry().values().filter(|b| available(b))
}

/// The running SAPI's own module, when `name_lc` is one of its functions
/// (`getallheaders` is php-cgi's `cgi-fcgi`'s): the stubs file those under
/// `standard`.
pub fn sapi_function_module(name_lc: &str) -> Option<&'static str> {
    let (module, fns) = main::sapi::sapi_module()?.module?;
    fns.iter().any(|(n, _)| *n == name_lc).then_some(module)
}

/// Whether some SAPI has a builtin `name_lc` (lowercase): what the compiler
/// asks, so that compiling does not depend on the SAPI.
pub fn native_function(name_lc: &str) -> bool {
    registry().contains_key(name_lc) && !is_disabled(name_lc)
}

/// Case-insensitive builtin lookup.
pub fn lookup(name: &str) -> Option<&'static Builtin> {
    let lower = name.to_ascii_lowercase();
    lookup_lc(&lower)
}

/// [`lookup`] for a name that is already lowercase (no allocation), among
/// the running SAPI's functions.
pub fn lookup_lc(name_lc: &str) -> Option<&'static Builtin> {
    registry().get(name_lc).filter(|b| available(b))
}

/// The constants of every module loaded under the running SAPI.
pub fn constants() -> Vec<(&'static str, Value)> {
    modules()
        .iter()
        .filter(|md| loaded(md))
        .flat_map(|md| (md.constants)())
        .collect()
}

/// The PHP prelude the compiler compiles into every program: each module's
/// PHP-source classes (core_prelude.php's first, then GMP, Phar, …) and
/// helper functions (iterator_*, …). Built once per process, outside any
/// request's memory.
pub fn prelude() -> &'static str {
    static PRELUDE: OnceLock<String> = OnceLock::new();
    PRELUDE.get_or_init(|| rhp_values::mem::uncounted(build_prelude))
}

fn build_prelude() -> String {
    let mut s = String::from("<?php\n");
    for md in modules() {
        if !md.prelude_php.is_empty() {
            s.push_str(md.prelude_php);
            s.push('\n');
        }
    }
    s
}

/// Built-in stream wrappers contributed by modules (e.g. `phar://`),
/// installed into the VM's per-run registry at startup.
pub fn stream_wrappers() -> Vec<WrapperReg> {
    modules().iter().flat_map(|md| (md.wrappers)()).collect()
}

/// If `name` is a deliberately-refused high-risk function, return why.
pub fn is_refused(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "extract" => Some("writes arbitrary variables into the local scope"),
        "compact" => Some("resolves variable names dynamically from strings"),
        "create_function" => Some("compiles a string into a function (eval-equivalent)"),
        _ => None,
    }
}

/// Register php functions by name. Their arity is php's, from the stub
/// signatures (every registered php function has a stub row; the
/// `php_builtins_have_stubs` test checks), raised to [`BODY_MIN`].
pub(crate) fn register(m: &mut Registry, fns: &[(&'static str, NativeFn)]) {
    use rhp_bytecode::signatures::{self, Signature};
    for &(name, func) in fns {
        let (min_args, max_args) = signatures::function(name.as_bytes())
            .map_or((0, None), |f| (f.min_args(), f.max_args()));
        let min_args = BODY_MIN
            .iter()
            .find(|(n, _)| *n == name)
            .map_or(min_args, |&(_, min)| min);
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args,
                max_args,
                sapis: 0,
            },
        );
    }
}

#[cfg(test)]
mod tests;
