//! Per-file compiler state: the file's source, line map, sinks and flags.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rhp_bytecode::{ClassProto, FunctionProto};
use rhp_frontend::cst::{ClassLikeMember, Program as AstProgram, Statement};
use rhp_values::{PhpStr, Thrown, Value};

use crate::fold::fold_const;
use crate::names::NsContext;

thread_local! {
    /// Sink for anonymous closure / arrow-function protos produced anywhere in
    /// the program. Compilation is single-threaded and non-reentrant per
    /// `compile()` call; the list is cleared on entry and drained into
    /// `Program::closures` on exit. A closure's `fnidx` is its position here.
    pub(crate) static CLOSURES: RefCell<Vec<Rc<FunctionProto>>> = const { RefCell::new(Vec::new()) };
    /// Anonymous classes (`new class {…}`) found anywhere in the program,
    /// declared with the program's classes under their generated names.
    pub(crate) static ANON_CLASSES: RefCell<Vec<ClassProto>> = const { RefCell::new(Vec::new()) };
    /// Sink for class declarations nested in control flow / function bodies
    /// (drained into `Program::cond_classes`), plus the file's count of user
    /// top-level class-likes (the `DeclareClass` index offset for them).
    pub(crate) static COND_CLASSES: RefCell<(Vec<ClassProto>, u32)> = const { RefCell::new((Vec::new(), 0)) };
}

/// The source bytes doc comments are looked up in, and its token stream
/// (lexed on the first lookup).
type DocSource = (Rc<Vec<u8>>, Option<Rc<Vec<rhp_frontend::Token>>>);

thread_local! {
    /// Method name (lowercase) -> per-position "could be by-reference" mask: true
    /// at position i if ANY method of that name declares `&` there. A call site
    /// passes those positions by reference; by-value callees simply deref, so
    /// over-approximation across classes is safe.
    pub(crate) static METHOD_REF: RefCell<HashMap<Vec<u8>, Vec<bool>>> = RefCell::new(HashMap::new());
    /// Byte offsets of each line start in the source currently being compiled
    /// (empty while compiling the prelude; whose line numbers are never surfaced).
    pub(crate) static LINE_STARTS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    /// The earliest compile error (source offset, line, message) recorded by
    /// [`compile_fatal`] in the file being compiled.
    pub(crate) static COMPILE_FATAL: RefCell<Option<(u32, u32, String)>> = const { RefCell::new(None) };
    /// See [`compile_error_line`].
    pub(crate) static COMPILE_ERROR_LINE: RefCell<u32> = const { RefCell::new(0) };
    /// The file name reported in debug info / exception getFile().
    pub(crate) static SRC_FILE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// The user source being compiled (empty for the prelude), for lexical
    /// details the AST drops (the newline a `?>` swallows).
    pub(crate) static SRC_BYTES: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// Whether the file being compiled declared `strict_types=1` (php's
    /// caller-decides coercion rule); stamped on every proto built from it.
    pub(crate) static STRICT_TYPES: RefCell<bool> = const { RefCell::new(false) };
    /// Whether a trait body is being compiled: `__CLASS__` there names the
    /// *using* class, so it is lowered to a runtime `self::class`.
    pub(crate) static IN_TRAIT: RefCell<bool> = const { RefCell::new(false) };
    /// Whether an interface body is being compiled (its property
    /// declarations, hooked or not, contribute no runtime members).
    pub(crate) static IN_INTERFACE: RefCell<bool> = const { RefCell::new(false) };
    /// The user source being compiled and its full token stream (comments
    /// included), for locating declarations' doc comments. Empty while
    /// compiling the prelude.
    pub(crate) static DOC_SRC: RefCell<DocSource> =
        RefCell::new((Rc::new(Vec::new()), None));
}

/// Record an E_COMPILE_ERROR found at source `offset` of the file being
/// compiled. The compile still finishes (so callers need no error plumbing),
/// then fails with the earliest recorded error (see [`compile_program`]).
pub(crate) fn compile_fatal(offset: u32, message: String) {
    let line = line_of(offset);
    COMPILE_FATAL.with(|f| {
        let mut f = f.borrow_mut();
        if f.as_ref().is_none_or(|(o, _, _)| offset < *o) {
            *f = Some((offset, line, message));
        }
    });
}

/// Whether the program's leading statement is `declare(strict_types=1)`.
pub(crate) fn detect_strict_types(program: &AstProgram) -> bool {
    for stmt in program.statements.iter() {
        match stmt {
            // skip the opening `<?php` / inline-html prelude
            Statement::OpeningTag(_) | Statement::Inline(_) | Statement::Noop(_) => continue,
            Statement::Declare(d) => {
                for item in d.items.iter() {
                    if item.name.value.eq_ignore_ascii_case(b"strict_types") {
                        return matches!(fold_const(item.value), Ok(Value::Int(1)));
                    }
                }
                return false;
            }
            _ => return false, // strict_types must be the first statement
        }
    }
    false
}

/// Sorted byte offsets of line starts (offset 0, then after each '\n').
pub(crate) fn build_line_starts(source: &[u8]) -> Vec<u32> {
    let mut v = vec![0u32];
    for (i, &b) in source.iter().enumerate() {
        if b == b'\n' {
            v.push((i + 1) as u32);
        }
    }
    v
}

/// The source file name currently being compiled (for `__FILE__`).
pub(crate) fn src_file() -> Vec<u8> {
    SRC_FILE.with(|f| f.borrow().clone())
}

/// The file being compiled (a class declaration's `info.user.filename`).
pub(crate) fn decl_file() -> PhpStr {
    SRC_FILE.with(|f| PhpStr::from_bytes(f.borrow().clone()))
}

/// 1-based line number for a byte offset, via the current `LINE_STARTS`.
/// php's lexer folds one newline (`\n`, `\r\n` or `\r`) right after `?>`
/// into the closing tag: drop it from inline text that starts there.
pub(crate) fn strip_close_tag_newline(offset: u32, text: &[u8]) -> &[u8] {
    let after_close = SRC_BYTES.with(|b| {
        let b = b.borrow();
        let o = offset as usize;
        o >= 2 && o <= b.len() && &b[o - 2..o] == b"?>"
    });
    if !after_close {
        return text;
    }
    match text {
        [b'\r', b'\n', rest @ ..] | [b'\n', rest @ ..] | [b'\r', rest @ ..] => rest,
        _ => text,
    }
}

pub(crate) fn line_of(offset: u32) -> u32 {
    LINE_STARTS.with(|ls| {
        let ls = ls.borrow();
        if ls.is_empty() {
            return 0;
        }
        // Largest index whose start <= offset, +1 for 1-based.
        match ls.binary_search(&offset) {
            Ok(i) => (i + 1) as u32,
            Err(i) => i as u32, // i = count of starts <= offset
        }
    })
}

/// Stamp a compile-time refusal with the file being compiled and the line of
/// `offset`, unless an inner construct already located it.
pub(crate) fn locate(e: Thrown, offset: u32) -> Thrown {
    if !matches!(e, Thrown::Unsupported { location: None, .. }) {
        return e;
    }
    let line = line_of(offset);
    e.with_location(
        || SRC_FILE.with(|f| String::from_utf8_lossy(&f.borrow()).into_owned()),
        line,
    )
}

/// Scan every class-like's methods, OR-ing their by-ref parameter flags by name.
/// (Method names are not namespaced, so this ignores the namespace context.)
pub(crate) fn collect_method_refs(
    flat: &[(Rc<NsContext>, &Statement)],
) -> HashMap<Vec<u8>, Vec<bool>> {
    let mut map: HashMap<Vec<u8>, Vec<bool>> = HashMap::new();
    let mut add = |name: &[u8], pl: &rhp_frontend::cst::FunctionLikeParameterList| {
        let entry = map.entry(name.to_ascii_lowercase()).or_default();
        for (i, p) in pl.parameters.iter().enumerate() {
            if entry.len() <= i {
                entry.resize(i + 1, false);
            }
            entry[i] |= p.ampersand.is_some();
        }
    };
    for (_ctx, stmt) in flat {
        let members = match stmt {
            Statement::Class(c) => &c.members,
            Statement::Trait(t) => &t.members,
            Statement::Enum(e) => &e.members,
            Statement::Interface(i) => &i.members,
            _ => continue,
        };
        for m in members.iter() {
            if let ClassLikeMember::Method(method) = m {
                add(method.name.value, &method.parameter_list);
            }
        }
    }
    map
}
