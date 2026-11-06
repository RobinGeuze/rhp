//! AST → bytecode compiler.
//!
//! Lowers a program parsed by [`parse`] into a `rhp_bytecode::Program`.
//! A construct rhp does not implement yet never produces an opcode: the
//! compiler returns `Err(Thrown::Unsupported)` for the whole file.
//!
//! Register model (spec §2.2): a pre-pass assigns one CV register per named
//! `$var` so CVs occupy `[0, num_cvs)`; temporaries bump above and are freed at
//! statement boundaries; `num_registers` is the peak.

use std::cell::Cell;
use std::rc::Rc;

use crate::parse::cst::{Node, Program as AstProgram, Statement};
use rhp_bytecode::{Instr, Program};
use rhp_values::{PhpStr, Thrown, Value};

use crate::builder::{ProtoBuilder, ScopeKind};
use crate::decl::{compile_class, compile_enum, compile_interface, compile_trait};
use crate::file::FileCtx;
use crate::functions::compile_function;
use crate::names::{flatten_namespaces, NsContext};
use crate::prelude::prelude_classes;
use crate::shrink::shrink_program;

mod assign;
mod attrs;
mod builder;
mod calls;
mod checks;
mod constexpr;
mod control;
mod cvs;
mod decl;
mod expr;
mod file;
mod fold;
mod functions;
mod hints;
mod hooks;
mod hrh;
mod names;
mod oop;
pub mod parse;
mod prelude;
mod shrink;
mod stmt;
mod strings;
mod symbols;
mod util;

thread_local! {
    /// See [`compile_error_line`]: the result channel of a failed compile,
    /// not per-file state.
    static COMPILE_ERROR_LINE: Cell<u32> = const { Cell::new(0) };
    /// See [`take_compile_error_diags`].
    static COMPILE_ERROR_DIAGS: std::cell::RefCell<Vec<(i64, String, u32)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The diagnostics (level, message, line) the compiler raised before the
/// compile error behind [`compile_error_line`]: php displays them first.
pub fn take_compile_error_diags() -> Vec<(i64, String, u32)> {
    COMPILE_ERROR_DIAGS.with(|d| d.take())
}

/// The line of the compile error (E_COMPILE_ERROR) behind the last
/// `Thrown::Error { class: "CompileError", .. }` a compile entry point of this
/// thread returned. The SAPI reports it as a fatal error at that line.
pub fn compile_error_line() -> u32 {
    COMPILE_ERROR_LINE.with(Cell::get)
}

pub(crate) fn set_compile_error_line(line: u32) {
    COMPILE_ERROR_LINE.with(|l| l.set(line));
}

/// What a file is compiled for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// A request's main script: the program starts with the prelude.
    Main,
    /// A file `include`d, `require`d or `eval`ed into a running VM, which
    /// has the prelude already: the program carries none and reports a
    /// prelude count of zero.
    Include,
}

/// What the compiler needs to know about the engine's builtins.
/// rhp-builtins has the answers and the request layer passes them in
/// (`rhp_sapi::compile_file`), so the compiler does not depend on
/// rhp-builtins and builds alongside it.
pub struct Builtins {
    /// The PHP prelude: the core classes first, then each module's classes
    /// and helper functions. Compiled once per thread for each prelude
    /// source (the same `&str`) and pair of the functions below, and the
    /// start of every [`Mode::Main`] program.
    pub prelude: fn() -> &'static str,
    /// Is `name` (lowercase) a native builtin function?
    pub native_function: fn(&str) -> bool,
    /// The value php's compiler substitutes for the global constant `name`
    /// (zend_try_ct_eval_const): a builtin constant that is not deprecated.
    pub constant: fn(&[u8]) -> Option<Value>,
    /// The request's working directory, php's `getcwd()`: `__DIR__` of code
    /// without a file ("Command line code", "Standard input code").
    pub cwd: fn() -> Option<Vec<u8>>,
}

/// Compile a parsed program into an owned, runnable bytecode `Program` (no
/// `'arena` borrow escapes). `source` and `file` give protos their lines
/// and file name (exception `getLine()`/`getFile()`, diagnostics).
pub fn compile(
    builtins: &Builtins,
    program: &AstProgram,
    source: &[u8],
    file: &str,
    mode: Mode,
) -> Result<Program, Thrown> {
    compile_program(builtins, program, source, file, mode == Mode::Main)
}

fn compile_program(
    builtins: &Builtins,
    program: &AstProgram,
    source: &[u8],
    file: &str,
    with_prelude: bool,
) -> Result<Program, Thrown> {
    // Flatten namespace/use into (context, statement) pairs in source order; every
    // pass below iterates this so name resolution is consistent across passes.
    let mut flat: Vec<(Rc<NsContext>, &Statement)> = Vec::new();
    flatten_namespaces(
        &program.statements,
        Rc::new(NsContext::default()),
        &mut flat,
    );
    let cx = FileCtx::new(builtins, file, source, program, &flat);
    symbols::check_file(&cx, program);
    let r = compile_file(&cx, &flat, with_prelude);
    // A recorded compile error came first in source order (compilation stops
    // at the first one in php), so it wins over a later refusal too, and over
    // a compile error that unwound the compile from a later line.
    let unwound = match &r {
        Err(Thrown::Error {
            class: "CompileError",
            ..
        }) => Some(compile_error_line()),
        _ => None,
    };
    let (r, mut diags) = match r {
        Ok(p) => {
            let d = p.compile_diags.clone();
            (Ok(p), d)
        }
        Err(e) => (Err(e), cx.take_diags(Vec::new())),
    };
    let r = match cx.take_fatal() {
        Some((line, message)) if unwound.is_none_or(|l| line <= l) => {
            set_compile_error_line(line);
            Err(Thrown::error("CompileError", message))
        }
        _ => r,
    };
    if let Err(Thrown::Error {
        class: "CompileError",
        ..
    }) = &r
    {
        // The diagnostics raised before the error, for the SAPI to show.
        let line = compile_error_line();
        diags.retain(|d| d.2 <= line);
        diags.sort_by_key(|d| d.2);
        COMPILE_ERROR_DIAGS.with(|d| *d.borrow_mut() = diags);
    }
    r
}

fn compile_file(
    cx: &FileCtx,
    flat: &[(Rc<NsContext>, &Statement)],
    with_prelude: bool,
) -> Result<Program, Thrown> {
    // Hoist top-level function + class declarations. Start the class list with
    // the built-in exception hierarchy (the "prelude"), compiled once per
    // thread and shared from the cache.
    let (mut classes, mut functions) = if with_prelude {
        let p = prelude_classes(cx.builtins)?;
        *cx.closures.borrow_mut() = p.closures.clone();
        (p.classes.clone(), p.functions.clone())
    } else {
        (Vec::new(), Vec::new())
    };
    let num_prelude_classes = classes.len();
    let num_prelude_functions = functions.len();
    for (ns, stmt) in flat {
        use mago_span::HasSpan;
        cx.set_ns(ns);
        let at = |e| cx.locate(e, stmt.span().start.offset);
        match stmt {
            Statement::Function(func) => {
                functions.push(Rc::new(compile_function(cx, func).map_err(at)?));
            }
            Statement::Class(class) => classes.push(Rc::new(compile_class(cx, class).map_err(at)?)),
            Statement::Interface(iface) => {
                classes.push(Rc::new(compile_interface(cx, iface).map_err(at)?))
            }
            Statement::Trait(tr) => classes.push(Rc::new(compile_trait(cx, tr).map_err(at)?)),
            Statement::Enum(en) => classes.push(Rc::new(compile_enum(cx, en).map_err(at)?)),
            // Top-level `const NAME = <expr>;` is a runtime statement (php
            // declares it when execution reaches it), compiled in {main}.
            _ => {}
        }
    }

    // Compile the script body ({main}); its CVs are the globals.
    let mut c = ProtoBuilder::new(cx, PhpStr::from("{main}"));
    c.scope_kind = ScopeKind::Main;
    for (_ns, stmt) in flat {
        c.scan_stmt_vars(stmt);
    }
    for (_ns, stmt) in flat {
        c.scan_node_vars(Node::Statement(stmt));
    }
    c.freeze_cvs();

    let mut top_class_idx = 0u32;
    for (ns, stmt) in flat {
        cx.set_ns(ns);
        // A bare top-level function is already hoisted into `functions`; skip it
        // so compile_stmt doesn't also runtime-declare (double) it. Functions
        // nested in control flow flow through compile_stmt and runtime-declare.
        if matches!(stmt, Statement::Function(_)) {
            continue;
        }
        // A top-level class is hoisted at load when its parent/interfaces
        // are available; otherwise it is declared when this point is reached.
        if matches!(
            stmt,
            Statement::Class(_)
                | Statement::Interface(_)
                | Statement::Trait(_)
                | Statement::Enum(_)
        ) {
            // php's DECLARE_CLASS line: the class keyword's (the decl's
            // line_start), not the attributes' or modifiers'.
            c.cur_line = classes[num_prelude_classes + top_class_idx as usize].decl_line;
            c.emit(Instr::DeclareClass {
                class_idx: top_class_idx,
            });
            top_class_idx += 1;
            continue;
        }
        c.compile_stmt(stmt)?;
    }
    c.emit(Instr::RetNull);

    let closures = cx.closures.take();
    classes.extend(cx.anon_classes.take().into_iter().map(Rc::new));
    let cond_classes = cx.cond_classes.take().into_iter().map(Rc::new).collect();
    let compile_diags = cx.take_diags(
        hrh::scan(flat, &|o| cx.line_of(o))
            .into_iter()
            .map(|line| (rhp_values::E_DEPRECATED, hrh::MESSAGE.to_string(), line))
            .collect(),
    );
    let mut prog = Program {
        main: Rc::new(c.finish()?),
        functions,
        classes,
        closures,
        num_prelude_classes,
        num_prelude_functions,
        cond_classes,
        compile_diags,
        compile_diags_reported: Default::default(),
    };
    shrink_program(&mut prog);
    Ok(prog)
}
