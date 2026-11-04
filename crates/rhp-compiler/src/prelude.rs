//! The built-in classes and helper functions written in PHP, compiled once per thread.

use std::cell::RefCell;
use std::rc::Rc;

use crate::parse::cst::Statement;
use rhp_bytecode::{ClassProto, FunctionProto};
use rhp_values::Thrown;

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
}

thread_local! {
    /// The prelude, compiled on first use and shared by every later compile on
    /// this thread (protos hold `Rc`s, so the cache is per thread — matching
    /// the VM-per-thread model), keyed on its source. It depends on no
    /// per-file state: it is compiled in a [`FileCtx`] of its own.
    static PRELUDE_CACHE: RefCell<Option<(&'static str, Rc<PreludeParts>)>> =
        const { RefCell::new(None) };
}

/// The compiled prelude of `builtins`, compiling it on this thread's first
/// call for its source.
pub(crate) fn prelude_classes(builtins: &Builtins) -> Result<Rc<PreludeParts>, Thrown> {
    let src = (builtins.prelude)();
    let cached = PRELUDE_CACHE.with(|c| {
        c.borrow()
            .as_ref()
            .filter(|(s, _)| std::ptr::eq(*s, src))
            .map(|(_, p)| p.clone())
    });
    if let Some(p) = cached {
        return Ok(p);
    }
    // Compiled once per thread, kept for every later request: not counted
    // against a request's memory.
    let parts =
        rhp_values::mem::uncounted(|| prelude_classes_isolated(builtins, src).map(Rc::new))?;
    PRELUDE_CACHE.with(|c| *c.borrow_mut() = Some((src, parts.clone())));
    Ok(parts)
}

/// Compile the prelude in a [`FileCtx`] of its own, so the result is
/// independent of whichever file happened to trigger it.
fn prelude_classes_isolated(builtins: &Builtins, src: &str) -> Result<PreludeParts, Thrown> {
    let arena = crate::parse::Arena::new();
    let parsed = arena.parse(b"<prelude>", src.as_bytes());
    let flat: Vec<(Rc<NsContext>, &Statement)> = parsed
        .program()
        .statements
        .iter()
        .map(|s| (Rc::new(NsContext::default()), s))
        .collect();
    // Never strict-typed (coercive calls). No file name: the VM treats the
    // protos as internal code (skipped for exception file/line, `[internal
    // function]` in traces). No line map: prelude line numbers are never
    // surfaced. Method calls see the prelude's own by-ref signatures.
    let cx = FileCtx::prelude(builtins, &flat);
    let compile = || -> Result<(Vec<ClassProto>, Vec<Rc<FunctionProto>>), Thrown> {
        let mut classes = Vec::new();
        let mut functions = Vec::new();
        for (_, stmt) in &flat {
            match stmt {
                Statement::Class(c) => classes.push(compile_class(&cx, c)?),
                Statement::Interface(i) => classes.push(compile_interface(&cx, i)?),
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
        classes: classes.into_iter().map(Rc::new).collect(),
        functions,
        closures: cx.closures.take(),
    })
}
