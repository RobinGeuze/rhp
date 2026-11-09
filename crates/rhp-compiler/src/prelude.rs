//! The built-in classes and helper functions written in PHP, compiled once per thread.

use std::cell::RefCell;
use std::rc::Rc;

use crate::parse::cst::Statement;
use rhp_bytecode::{ClassProto, FunctionProto};
use rhp_values::{Thrown, Value};

use crate::decl::{compile_class, compile_interface};
use crate::file::FileCtx;
use crate::functions::compile_function;
use crate::names::NsContext;
use crate::Builtins;

/// The compiled prelude: built-in classes, helper functions, and the
/// closures their bodies declare (always program closures `0..n`).
pub(crate) struct PreludeParts {
    pub(crate) classes: Vec<Rc<ClassProto>>,
    pub(crate) functions: Vec<Rc<FunctionProto>>,
    pub(crate) closures: Vec<Rc<FunctionProto>>,
    /// The classes' lowercase names.
    pub(crate) class_names: std::collections::HashSet<Vec<u8>>,
}

thread_local! {
    /// The prelude, compiled on first use and shared by every later compile on
    /// this thread (protos hold `Rc`s, so the cache is per thread — matching
    /// the VM-per-thread model). It depends on no per-file state: it is
    /// compiled in a [`FileCtx`] of its own.
    static PRELUDE_CACHE: RefCell<Option<(Key, Rc<PreludeParts>)>> =
        const { RefCell::new(None) };
}

/// What a compiled prelude depends on: its source, and the builtins' answers
/// its compile asked for (constants it folds, native functions it calls).
struct Key {
    src: &'static str,
    native_function: fn(&str) -> bool,
    constant: fn(&[u8]) -> Option<Value>,
    prelude_flags: fn(Option<&[u8]>, &[u8]) -> rhp_bytecode::ProtoFlags,
}

impl Key {
    fn of(builtins: &Builtins) -> Key {
        Key {
            src: (builtins.prelude)(),
            native_function: builtins.native_function,
            constant: builtins.constant,
            prelude_flags: builtins.prelude_flags,
        }
    }

    /// The same source and functions. Functions share an address only when
    /// their code is the same, so they answer alike; one function at two
    /// addresses only costs a recompile.
    fn same(&self, other: &Key) -> bool {
        std::ptr::eq(self.src, other.src)
            && std::ptr::fn_addr_eq(self.native_function, other.native_function)
            && std::ptr::fn_addr_eq(self.constant, other.constant)
            && std::ptr::fn_addr_eq(self.prelude_flags, other.prelude_flags)
    }
}

/// The compiled prelude of `builtins`, compiling it on this thread's first
/// call for them.
pub(crate) fn prelude_classes(builtins: &Builtins) -> Result<Rc<PreludeParts>, Thrown> {
    let key = Key::of(builtins);
    let cached = PRELUDE_CACHE.with(|c| {
        c.borrow()
            .as_ref()
            .filter(|(k, _)| k.same(&key))
            .map(|(_, p)| p.clone())
    });
    if let Some(p) = cached {
        return Ok(p);
    }
    // Compiled once per thread, kept for every later request: not counted
    // against a request's memory.
    let parts =
        rhp_values::mem::uncounted(|| prelude_classes_isolated(builtins, key.src).map(Rc::new))?;
    PRELUDE_CACHE.with(|c| *c.borrow_mut() = Some((key, parts.clone())));
    Ok(parts)
}

/// Compile the prelude in a [`FileCtx`] of its own, so the result is
/// independent of whichever file happened to trigger it.
fn prelude_classes_isolated(builtins: &Builtins, src: &str) -> Result<PreludeParts, Thrown> {
    let arena = crate::parse::Arena::new();
    let parsed = arena.parse(b"<prelude>", src.as_bytes());
    // `namespace X { ... }` blocks declare namespaced classes (ext/random's).
    let mut flat: Vec<(Rc<NsContext>, &Statement)> = Vec::new();
    crate::names::flatten_namespaces(
        &parsed.program().statements,
        Rc::new(NsContext::default()),
        &mut flat,
    );
    // Never strict-typed (coercive calls). No file name: the VM treats the
    // protos as internal code (skipped for exception file/line, `[internal
    // function]` in traces). No line map: prelude line numbers are never
    // surfaced. Method calls see the prelude's own by-ref signatures.
    let cx = FileCtx::prelude(builtins, &flat);
    let compile = || -> Result<(Vec<ClassProto>, Vec<Rc<FunctionProto>>), Thrown> {
        let mut classes = Vec::new();
        let mut functions = Vec::new();
        for (ns, stmt) in &flat {
            cx.set_ns(ns);
            match stmt {
                Statement::Class(c) => classes.push(compile_class(&cx, c)?),
                Statement::Interface(i) => classes.push(compile_interface(&cx, i)?),
                Statement::Enum(e) => classes.push(crate::decl::compile_enum(&cx, e)?),
                Statement::Function(f) => functions.push(Rc::new(compile_function(&cx, f)?)),
                _ => {}
            }
        }
        Ok((classes, functions))
    };
    let r = compile();
    // Closures the prelude declares occupy closure indices `0..n`; a program
    // with the prelude re-seeds them first. Anonymous / nested classes would
    // index per-file tables the cached copy cannot carry.
    assert!(
        cx.anon_classes.borrow().is_empty() && cx.cond_classes.borrow().is_empty(),
        "prelude must not declare anonymous or nested classes"
    );
    assert!(
        cx.take_fatal().is_none(),
        "prelude must compile without errors"
    );
    let (classes, functions) = r?;
    Ok(PreludeParts {
        class_names: classes
            .iter()
            .map(|c| c.name.as_bytes().to_ascii_lowercase())
            .collect(),
        classes: classes.into_iter().map(Rc::new).collect(),
        functions,
        closures: cx.closures.take(),
    })
}
