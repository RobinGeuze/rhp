//! AST → bytecode compiler.
//!
//! Lowers a parsed `rhp_frontend` program into a `rhp_bytecode::Program`.
//! A construct rhp does not implement yet never produces an opcode: the
//! compiler returns `Err(Thrown::Unsupported)` for the whole file.
//!
//! Register model (spec §2.2): a pre-pass assigns one CV register per named
//! `$var` so CVs occupy `[0, num_cvs)`; temporaries bump above and are freed at
//! statement boundaries; `num_registers` is the peak.

use std::collections::HashMap;
use std::rc::Rc;

use rhp_bytecode::{Instr, Program};
use rhp_frontend::cst::{Node, Program as AstProgram, Statement};
use rhp_values::{PhpStr, Thrown};

use crate::builder::{ProtoBuilder, ScopeKind};
use crate::decl::{compile_class, compile_enum, compile_interface, compile_trait};
use crate::file::{
    build_line_starts, collect_method_refs, detect_strict_types, line_of, locate, ANON_CLASSES,
    CLOSURES, COMPILE_ERROR_LINE, COMPILE_FATAL, COND_CLASSES, DOC_SRC, IN_INTERFACE, IN_TRAIT,
    LINE_STARTS, METHOD_REF, SRC_BYTES, SRC_FILE, STRICT_TYPES,
};
use crate::functions::compile_function;
use crate::names::{flatten_namespaces, join_ns, set_ns_ctx, NsContext, NS_CTX};
use crate::prelude::prelude_classes;

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
mod prelude;
mod stmt;
mod util;

/// The line of the compile error (E_COMPILE_ERROR) behind the last
/// `Thrown::Error { class: "CompileError", .. }` a compile entry point of this
/// thread returned. The SAPI reports it as a fatal error at that line.
pub fn compile_error_line() -> u32 {
    COMPILE_ERROR_LINE.with(|l| *l.borrow())
}

/// Compile a parsed program (borrowed from a `rhp_frontend::Arena`) into an
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
    COMPILE_FATAL.with(|f| *f.borrow_mut() = None);
    let r = compile_program_inner(program, source, file, with_prelude);
    clear_file_state();
    // A recorded compile error came first in source order (compilation stops
    // at the first one in php), so it wins over a later refusal too.
    match COMPILE_FATAL.with(|f| f.borrow_mut().take()) {
        Some((_, line, message)) => {
            COMPILE_ERROR_LINE.with(|l| *l.borrow_mut() = line);
            Err(Thrown::error("CompileError", message))
        }
        None => r,
    }
}

/// Drop the per-file compiler state, so a thread does not keep the last
/// compiled file's source, tokens and tables alive (an error return can
/// also leave the class and closure sinks filled).
fn clear_file_state() {
    SRC_BYTES.with(|b| std::mem::take(&mut *b.borrow_mut()));
    DOC_SRC.with(|d| *d.borrow_mut() = (Rc::new(Vec::new()), None));
    LINE_STARTS.with(|l| std::mem::take(&mut *l.borrow_mut()));
    METHOD_REF.with(|m| std::mem::take(&mut *m.borrow_mut()));
    NS_CTX.with(|c| *c.borrow_mut() = Rc::new(NsContext::default()));
    SRC_FILE.with(|f| std::mem::take(&mut *f.borrow_mut()));
    STRICT_TYPES.with(|s| *s.borrow_mut() = false);
    IN_TRAIT.with(|t| *t.borrow_mut() = false);
    IN_INTERFACE.with(|i| *i.borrow_mut() = false);
    CLOSURES.with(|c| std::mem::take(&mut *c.borrow_mut()));
    ANON_CLASSES.with(|c| std::mem::take(&mut *c.borrow_mut()));
    COND_CLASSES.with(|c| std::mem::take(&mut *c.borrow_mut()));
}

fn compile_program_inner(
    program: &AstProgram,
    source: &[u8],
    file: &str,
    with_prelude: bool,
) -> Result<Program, Thrown> {
    LINE_STARTS.with(|ls| ls.borrow_mut().clear());
    DOC_SRC.with(|d| *d.borrow_mut() = (Rc::new(Vec::new()), None));
    SRC_BYTES.with(|b| b.borrow_mut().clear());
    SRC_FILE.with(|f| *f.borrow_mut() = file.as_bytes().to_vec());
    STRICT_TYPES.with(|s| *s.borrow_mut() = detect_strict_types(program));
    let user_line_starts = build_line_starts(source);
    CLOSURES.with(|c| c.borrow_mut().clear());
    ANON_CLASSES.with(|c| c.borrow_mut().clear());
    COND_CLASSES.with(|c| *c.borrow_mut() = (Vec::new(), 0));
    NS_CTX.with(|c| *c.borrow_mut() = Rc::new(NsContext::default()));
    // Flatten namespace/use into (context, statement) pairs in source order; every
    // pass below iterates this so name resolution is consistent across passes.
    let mut flat: Vec<(Rc<NsContext>, &Statement)> = Vec::new();
    flatten_namespaces(
        &program.statements,
        Rc::new(NsContext::default()),
        &mut flat,
    );

    // Signature pre-pass: fully-qualified name -> per-parameter by-ref flags, so a
    // call site can marshal by-ref arguments even though the callee is resolved by
    // name. Keyed by FQN (lowercased) to match the call-site resolution.
    let mut sigs: HashMap<Vec<u8>, Vec<bool>> = HashMap::new();
    for (ctx, stmt) in &flat {
        if let Statement::Function(func) = stmt {
            let fqn = join_ns(&ctx.ns, func.name.value).to_ascii_lowercase();
            let flags: Vec<bool> = func
                .parameter_list
                .parameters
                .iter()
                .map(|p| p.ampersand.is_some())
                .collect();
            sigs.insert(fqn, flags);
        }
    }
    let sigs = Rc::new(sigs);
    METHOD_REF.with(|m| *m.borrow_mut() = collect_method_refs(&flat));

    // Hoist top-level function + class declarations. Start the class list with
    // the built-in exception hierarchy (the "prelude"), compiled once per
    // thread and copied from the cache.
    let (mut classes, mut functions) = if with_prelude {
        let p = prelude_classes()?;
        CLOSURES.with(|c| *c.borrow_mut() = p.closures.clone());
        (p.classes.clone(), p.functions.clone())
    } else {
        (Vec::new(), Vec::new())
    };
    let num_prelude_classes = classes.len();
    let num_prelude_functions = functions.len();
    let is_class_like = |st: &Statement| {
        matches!(
            st,
            Statement::Class(_)
                | Statement::Interface(_)
                | Statement::Trait(_)
                | Statement::Enum(_)
        )
    };
    let num_top_classes = flat.iter().filter(|(_, st)| is_class_like(st)).count() as u32;
    COND_CLASSES.with(|c| c.borrow_mut().1 = num_top_classes);
    // From here on we compile user code, so install its line map.
    LINE_STARTS.with(|ls| *ls.borrow_mut() = user_line_starts);
    DOC_SRC.with(|d| *d.borrow_mut() = (Rc::new(source.to_vec()), None));
    SRC_BYTES.with(|b| *b.borrow_mut() = source.to_vec());
    for (ctx, stmt) in &flat {
        use mago_span::HasSpan;
        set_ns_ctx(ctx);
        let at = |e| locate(e, stmt.span().start.offset);
        match stmt {
            Statement::Function(func) => {
                functions.push(Rc::new(compile_function(func, sigs.clone()).map_err(at)?));
            }
            Statement::Class(class) => classes.push(compile_class(class, &sigs).map_err(at)?),
            Statement::Interface(iface) => {
                classes.push(compile_interface(iface, &sigs).map_err(at)?)
            }
            Statement::Trait(tr) => classes.push(compile_trait(tr, &sigs).map_err(at)?),
            Statement::Enum(en) => classes.push(compile_enum(en, &sigs).map_err(at)?),
            // Top-level `const NAME = <expr>;` is a runtime statement (php
            // declares it when execution reaches it), compiled in {main}.
            _ => {}
        }
    }

    // Compile the script body ({main}); its CVs are the globals.
    let mut c = ProtoBuilder::new(PhpStr::from("{main}"));
    c.sigs = sigs.clone();
    c.scope_kind = ScopeKind::Main;
    for (_ctx, stmt) in &flat {
        c.scan_stmt_vars(stmt);
    }
    for (_ctx, stmt) in &flat {
        c.scan_node_vars(Node::Statement(stmt));
    }
    c.freeze_cvs();

    let mut top_class_idx = 0u32;
    for (ctx, stmt) in &flat {
        set_ns_ctx(ctx);
        // A bare top-level function is already hoisted into `functions`; skip it
        // so compile_stmt doesn't also runtime-declare (double) it. Functions
        // nested in control flow flow through compile_stmt and runtime-declare.
        if matches!(stmt, Statement::Function(_)) {
            continue;
        }
        // A top-level class is hoisted at load when its parent/interfaces
        // are available; otherwise it is declared when this point is reached.
        if is_class_like(stmt) {
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

    let closures = CLOSURES.with(|c| std::mem::take(&mut *c.borrow_mut()));
    classes.extend(ANON_CLASSES.with(|c| std::mem::take(&mut *c.borrow_mut())));
    let cond_classes = COND_CLASSES.with(|c| std::mem::take(&mut c.borrow_mut().0));
    let compile_diags = hrh::scan(&flat, line_of)
        .into_iter()
        .map(|line| (8192, hrh::MESSAGE.to_string(), line))
        .collect();
    Ok(Program {
        main: Rc::new(c.finish()?),
        functions,
        classes,
        closures,
        num_prelude_classes,
        num_prelude_functions,
        cond_classes,
        compile_diags,
        compile_diags_reported: Default::default(),
    })
}
