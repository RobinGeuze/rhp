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
use rhp_values::{PhpStr, Thrown};

use crate::builder::{ProtoBuilder, ScopeKind};
use crate::decl::{compile_class, compile_enum, compile_interface, compile_trait};
use crate::file::FileCtx;
use crate::functions::compile_function;
use crate::names::{flatten_namespaces, NsContext};
use crate::prelude::prelude_classes;
use crate::shrink::shrink_program;

mod assign;
mod builder;
mod calls;
mod control;
mod cvs;
mod decl;
mod expr;
mod file;
mod fold;
mod functions;
mod hints;
mod hrh;
mod names;
mod oop;
pub mod parse;
mod prelude;
mod shrink;
mod stmt;
mod util;

thread_local! {
    /// See [`compile_error_line`]: the result channel of a failed compile,
    /// not per-file state.
    static COMPILE_ERROR_LINE: Cell<u32> = const { Cell::new(0) };
}

/// The line of the compile error (E_COMPILE_ERROR) behind the last
/// `Thrown::Error { class: "CompileError", .. }` a compile entry point of this
/// thread returned. The SAPI reports it as a fatal error at that line.
pub fn compile_error_line() -> u32 {
    COMPILE_ERROR_LINE.with(Cell::get)
}

/// Compile a parsed program (borrowed from a `crate::parse::Arena`) into an
/// owned, runnable bytecode `Program`. No `'arena` borrow escapes.
pub fn compile(program: &AstProgram) -> Result<Program, Thrown> {
    compile_with_source(program, b"", "")
}

/// Compile with the original source + file name, so protos carry accurate line
/// numbers (for exception `getLine()`/`getFile()` and diagnostics).
pub fn compile_with_source(
    program: &AstProgram,
    source: &[u8],
    file: &str,
) -> Result<Program, Thrown> {
    compile_program(program, source, file, true)
}

/// Compile a file for `include`/`require`/`eval` into an already-running VM:
/// like [`compile_with_source`] but without the built-in prelude (the VM
/// skips an included program's first `num_prelude_*` entries anyway, so
/// this program carries none and reports a prelude count of zero).
pub fn compile_include(program: &AstProgram, source: &[u8], file: &str) -> Result<Program, Thrown> {
    compile_program(program, source, file, false)
}

fn compile_program(
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
    let cx = FileCtx::new(file, source, program, &flat);
    let r = compile_file(&cx, &flat, with_prelude);
    // A recorded compile error came first in source order (compilation stops
    // at the first one in php), so it wins over a later refusal too.
    match cx.take_fatal() {
        Some((line, message)) => {
            COMPILE_ERROR_LINE.with(|l| l.set(line));
            Err(Thrown::error("CompileError", message))
        }
        None => r,
    }
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
        let p = prelude_classes()?;
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
    let mut compile_diags: Vec<_> = hrh::scan(flat, &|o| cx.line_of(o))
        .into_iter()
        .map(|line| (rhp_builtins::E_DEPRECATED, hrh::MESSAGE.to_string(), line))
        .chain(cx.compile_diags.take())
        .collect();
    compile_diags.sort_by_key(|d| d.2);
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
