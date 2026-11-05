//! File-level name checks, in php's compile order: namespace declaration
//! rules (zend_compile_namespace, zend_verify_namespace), `use` imports
//! (zend_compile_use) and the declarations they may clash with
//! (FC(seen_symbols), the "previously declared as local import" checks).

use std::collections::HashMap;

use crate::parse::cst::{Node, Program, Sequence, Statement, UseItem, UseItems, UseType};
use mago_span::HasSpan;
use rhp_values::E_WARNING;

use crate::checks::is_reserved_class_name;
use crate::file::FileCtx;
use crate::names::identifier_raw;

const CLASS: u8 = 1;
const FUNCTION: u8 = 2;
const CONST: u8 = 4;

#[derive(Default)]
struct State {
    /// FC(current_namespace).
    ns: Option<Vec<u8>>,
    in_namespace: bool,
    has_bracketed: bool,
    /// Import tables: lookup name -> imported (old) name.
    imports: [HashMap<Vec<u8>, Vec<u8>>; 3],
    /// FC(seen_symbols): lowercase name (case kept for constants) -> kinds.
    seen: HashMap<Vec<u8>, u8>,
    /// Functions declared at the top level: lowercase name -> line.
    top_functions: HashMap<Vec<u8>, u32>,
    /// `__halt_compiler();` was reached: php compiles nothing after it.
    halted: bool,
    /// The declare statements before the file's first other statement
    /// (where strict_types may be set: zend_is_first_statement).
    leading_declares: Vec<usize>,
}

fn s(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn idx(kind: u8) -> usize {
    match kind {
        CLASS => 0,
        FUNCTION => 1,
        _ => 2,
    }
}

impl State {
    fn reset_imports(&mut self) {
        for t in &mut self.imports {
            t.clear();
        }
        self.seen.clear();
    }

    fn prefixed(&self, name: &[u8]) -> Vec<u8> {
        match &self.ns {
            Some(ns) => {
                let mut v = ns.clone();
                v.push(b'\\');
                v.extend_from_slice(name);
                v
            }
            None => name.to_vec(),
        }
    }
}

/// Run the checks over a whole file. Errors go to [`FileCtx::fatal`], the
/// non-compound `use` warning to the compile diagnostics.
pub(crate) fn check_file(cx: &FileCtx, program: &Program) {
    if cx.is_prelude {
        return;
    }
    let mut st = State::default();
    for stmt in program.statements.iter() {
        match stmt {
            Statement::OpeningTag(_) | Statement::ClosingTag(_) => {}
            Statement::Inline(i) if cx.inline_is_empty(i) => {}
            Statement::Declare(d) => {
                st.leading_declares.push(d as *const _ as usize);
            }
            _ => break,
        }
    }
    // Stops at the first error.
    let _ = top_stmts(cx, program, &program.statements, &mut st);
}

/// zend_compile_top_stmt over `stmts`; `None` once an error is recorded
/// (php stops at the first).
fn top_stmts(
    cx: &FileCtx,
    program: &Program,
    stmts: &Sequence<Statement>,
    st: &mut State,
) -> Option<()> {
    for stmt in stmts.iter() {
        if st.halted {
            break;
        }
        match stmt {
            Statement::Namespace(ns) => {
                let with_bracket =
                    matches!(ns.body, crate::parse::cst::NamespaceBody::BraceDelimited(_));
                let at = stmt.span().start.offset;
                let err = |msg: &str| {
                    cx.fatal(at, msg.to_string());
                    None::<()>
                };
                if !st.has_bracketed {
                    if st.ns.is_some() && with_bracket {
                        return err("Cannot mix bracketed namespace declarations with unbracketed namespace declarations");
                    }
                } else if !with_bracket {
                    return err("Cannot mix bracketed namespace declarations with unbracketed namespace declarations");
                } else if st.ns.is_some() || st.in_namespace {
                    return err("Namespace declarations cannot be nested");
                }
                let first =
                    (!with_bracket && st.ns.is_none()) || (with_bracket && !st.has_bracketed);
                if first && !is_first_statement(cx, program, stmt) {
                    return err("Namespace declaration statement has to be the very first statement or after any declare call in the script");
                }
                st.ns = match &ns.name {
                    Some(n) => {
                        let name = identifier_raw(n);
                        if name.eq_ignore_ascii_case(b"namespace") {
                            return err(&format!("Cannot use '{}' as namespace name", s(name)));
                        }
                        Some(name.to_vec())
                    }
                    None => None,
                };
                st.reset_imports();
                st.in_namespace = true;
                if with_bracket {
                    st.has_bracketed = true;
                }
                top_stmts(cx, program, ns.statements(), st)?;
                if with_bracket {
                    // zend_end_namespace.
                    st.in_namespace = false;
                    st.reset_imports();
                    st.ns = None;
                }
            }
            Statement::Use(u) => compile_use(cx, u, st)?,
            Statement::OpeningTag(_) | Statement::ClosingTag(_) | Statement::Noop(_) => {}
            Statement::HaltCompiler(_) => st.halted = true,
            Statement::Inline(i) if cx.inline_is_empty(i) => {}
            _ => {
                declarations(cx, Node::Statement(stmt), st, true)?;
                if st.has_bracketed && !st.in_namespace {
                    cx.fatal(
                        stmt.span().start.offset,
                        "No code may exist outside of namespace {}".into(),
                    );
                    return None;
                }
            }
        }
    }
    Some(())
}

/// zend_is_first_statement (allow_nop): only declare statements before it.
fn is_first_statement(cx: &FileCtx, program: &Program, target: &Statement) -> bool {
    for stmt in program.statements.iter() {
        if std::ptr::eq(stmt, target) {
            return true;
        }
        match stmt {
            Statement::Declare(_)
            | Statement::OpeningTag(_)
            | Statement::ClosingTag(_)
            | Statement::Noop(_) => {}
            Statement::Inline(i) if cx.inline_is_empty(i) => {}
            _ => return false,
        }
    }
    false
}

fn use_kind(t: &UseType) -> u8 {
    match t {
        UseType::Function(_) => FUNCTION,
        UseType::Const(_) => CONST,
    }
}

fn compile_use(cx: &FileCtx, u: &crate::parse::cst::Use, st: &mut State) -> Option<()> {
    let mut items: Vec<(u8, Vec<u8>, &UseItem)> = Vec::new();
    match &u.items {
        UseItems::Sequence(seq) => items.extend(seq.items.iter().map(|it| (CLASS, Vec::new(), it))),
        UseItems::TypedSequence(ts) => {
            let k = use_kind(&ts.r#type);
            items.extend(ts.items.iter().map(|it| (k, Vec::new(), it)));
        }
        UseItems::TypedList(tl) => {
            let k = use_kind(&tl.r#type);
            let pfx = identifier_raw(&tl.namespace).to_vec();
            items.extend(tl.items.iter().map(|it| (k, pfx.clone(), it)));
        }
        UseItems::MixedList(ml) => {
            let pfx = identifier_raw(&ml.namespace).to_vec();
            for mit in ml.items.iter() {
                let k = mit.r#type.as_ref().map(use_kind).unwrap_or(CLASS);
                items.push((k, pfx.clone(), &mit.item));
            }
        }
    }
    // php reports at the statement's line: its first name's (the prefix of
    // a group use).
    let at = match &u.items {
        UseItems::TypedList(tl) => tl.namespace.span().start.offset,
        UseItems::MixedList(ml) => ml.namespace.span().start.offset,
        _ => items.first().map_or(0, |i| i.2.name.span().start.offset),
    };
    for (kind, pfx, it) in items {
        use_one(cx, kind, &pfx, it, at, st)?;
    }
    Some(())
}

fn use_one(
    cx: &FileCtx,
    kind: u8,
    pfx: &[u8],
    it: &UseItem,
    at: u32,
    st: &mut State,
) -> Option<()> {
    let raw = identifier_raw(&it.name);
    let mut old = pfx.strip_prefix(b"\\").unwrap_or(pfx).to_vec();
    if !old.is_empty() {
        old.push(b'\\');
    }
    old.extend_from_slice(raw);
    let old = old.strip_prefix(b"\\").unwrap_or(&old).to_vec();
    let new: Vec<u8> = match &it.alias {
        Some(a) => a.identifier.value.to_vec(),
        None => match old.iter().rposition(|&b| b == b'\\') {
            Some(p) => old[p + 1..].to_vec(),
            None => {
                if st.ns.is_none() {
                    cx.diag(
                        E_WARNING,
                        at,
                        format!(
                            "The use statement with non-compound name '{}' has no effect",
                            s(&old)
                        ),
                    );
                }
                old.clone()
            }
        },
    };
    let lookup = if kind == CONST {
        new.clone()
    } else {
        new.to_ascii_lowercase()
    };
    if kind == CLASS && is_reserved_class_name(&new) {
        cx.fatal(
            at,
            format!(
                "Cannot use {} as {} because '{}' is a special class name",
                s(&old),
                s(&new),
                s(&new)
            ),
        );
        return None;
    }
    let type_str = match kind {
        CLASS => "",
        FUNCTION => " function",
        _ => " const",
    };
    let in_use = || {
        format!(
            "Cannot use{type_str} {} as {} because the name is already in use",
            s(&old),
            s(&new)
        )
    };
    let check = match &st.ns {
        Some(ns) => {
            let mut v = ns.to_ascii_lowercase();
            v.push(b'\\');
            v.extend_from_slice(&lookup);
            v
        }
        None => lookup.clone(),
    };
    if st.seen.get(&check).is_some_and(|k| k & kind != 0) && !old.eq_ignore_ascii_case(&check) {
        cx.fatal(at, in_use());
        return None;
    }
    let table = &mut st.imports[idx(kind)];
    if table.contains_key(&lookup) {
        cx.fatal(at, in_use());
        return None;
    }
    table.insert(lookup, old);
    Some(())
}

/// The declarations in `node`, in compile order: check them against the
/// imports and record them as seen.
fn declarations(cx: &FileCtx, node: Node, st: &mut State, top: bool) -> Option<()> {
    let (kind, short, at): (u8, &[u8], u32) = match node {
        // php's decl line is the `function` keyword's, after any attributes.
        Node::Function(f) => (FUNCTION, f.name.value, f.function.span().start.offset),
        Node::Class(c) => (CLASS, c.name.value, c.span().start.offset),
        Node::Interface(i) => (CLASS, i.name.value, i.span().start.offset),
        Node::Trait(t) => (CLASS, t.name.value, t.span().start.offset),
        Node::Enum(e) => (CLASS, e.name.value, e.span().start.offset),
        Node::Constant(c) => {
            for item in c.items.iter() {
                let name = st.prefixed(item.name.value);
                if let Some(imp) = st.imports[idx(CONST)].get(item.name.value) {
                    if *imp != name {
                        cx.fatal(
                            c.span().start.offset,
                            format!(
                                "Cannot declare const {} because the name is already in use",
                                s(&name)
                            ),
                        );
                        return None;
                    }
                }
                *st.seen.entry(name).or_default() |= CONST;
            }
            return Some(());
        }
        Node::Declare(d) => {
            // zend_compile_declare's strict_types placement rules.
            let strict = d
                .items
                .iter()
                .any(|i| i.name.value.eq_ignore_ascii_case(b"strict_types"));
            if strict {
                let msg = if !st.leading_declares.contains(&(d as *const _ as usize)) {
                    Some("strict_types declaration must be the very first statement in the script")
                } else if !matches!(
                    &d.body,
                    crate::parse::cst::DeclareBody::Statement(Statement::Noop(_))
                ) {
                    Some("strict_types declaration must not use block mode")
                } else {
                    None
                };
                if let Some(msg) = msg {
                    cx.fatal(d.span().start.offset, msg.into());
                    return None;
                }
            }
            for child in node.children() {
                declarations(cx, child, st, false)?;
            }
            return Some(());
        }
        // Closures and anonymous classes declare no names.
        Node::Closure(_) | Node::ArrowFunction(_) | Node::AnonymousClass(_) => {
            return Some(());
        }
        _ => {
            for child in node.children() {
                declarations(cx, child, st, top && matches!(node, Node::Statement(_)))?;
            }
            return Some(());
        }
    };
    let name = st.prefixed(short);
    let lc = name.to_ascii_lowercase();
    if let Some(imp) = st.imports[idx(kind)].get(&short.to_ascii_lowercase()) {
        if !imp.eq_ignore_ascii_case(&lc) {
            let msg = if kind == FUNCTION {
                format!(
                    "Cannot redeclare function {}() (previously declared as local import)",
                    s(&name)
                )
            } else {
                format!(
                    "Cannot redeclare class {} (previously declared as local import)",
                    s(&name)
                )
            };
            cx.fatal(at, msg);
            return None;
        }
    }
    if kind == FUNCTION {
        if short.eq_ignore_ascii_case(b"assert") {
            cx.fatal(
                at,
                "Defining a custom assert() function is not allowed, as the function has special semantics".into(),
            );
            return None;
        }
        if top {
            let line = cx.line_of(at);
            if let Some(prev) = st.top_functions.get(&lc) {
                cx.fatal(
                    at,
                    format!(
                        "Cannot redeclare function {}() (previously declared in {}:{})",
                        s(&name),
                        s(&cx.file_name()),
                        prev
                    ),
                );
                return None;
            }
            st.top_functions.insert(lc.clone(), line);
        }
    }
    *st.seen.entry(lc).or_default() |= kind;
    for child in node.children() {
        declarations(cx, child, st, false)?;
    }
    Some(())
}
