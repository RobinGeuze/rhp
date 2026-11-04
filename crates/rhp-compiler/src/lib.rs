//! AST → bytecode compiler.
//!
//! Lowers a parsed `rhp_frontend` program into a `rhp_bytecode::Program`.
//! A construct rhp does not implement yet never produces an opcode: the
//! compiler returns `Err(Thrown::Unsupported)` for the whole file.
//!
//! Register model (spec §2.2): a pre-pass assigns one CV register per named
//! `$var` so CVs occupy `[0, num_cvs)`; temporaries bump above and are freed at
//! statement boundaries; `num_registers` is the peak.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rhp_bytecode::{
    CIdx, ClassProto, ClassRef, CmpKind, ConstInitProto, ConstProto, FunctionProto, IncludeKind,
    Instr, MethodProto, NIdx, Program, PropDefaultProto, PropProto, Reg, StaticPropProto,
};
use rhp_frontend::cst::{
    Access, Argument, ArrayElement, AssignmentOperator, BinaryOperator, Class, ClassLikeMember,
    ClassLikeMemberSelector, CompositeString, Construct, Expression, ForBody, ForeachBody,
    ForeachTarget, Hint, Identifier, IfBody, Literal, MagicConstant, Match, MatchArm, MethodBody,
    Modifier, Node, Program as AstProgram, Property, PropertyItem, Statement, StringPart,
    SwitchBody, SwitchCase, UnaryPrefixOperator, Variable, WhileBody,
};
use rhp_values::{ClassFlags, ClassKind, Visibility};

/// A compiled member selector: a static interned name, or a runtime name in a
/// register (dynamic `$o->$x` / `$o->{expr}`).
#[derive(Clone, Copy)]
enum Sel {
    Static(NIdx),
    Dyn(Reg),
}

/// A step in an lvalue path (compiler-side; key regs are pre-evaluated).
enum CursorStep {
    Key(Reg),
    Append,
    Prop(NIdx),
    PropDyn(Reg),
}

/// Where an lvalue cursor roots.
enum LvalRoot {
    /// A register: a CV, or a temporary holding a call result.
    Cv(Reg),
    StaticProp(ClassRef, NIdx),
    /// `C::${expr}`: static property with a runtime name.
    StaticPropDyn(ClassRef, Reg),
    /// `$$name` / `${expr}`: the variable named by the register's value.
    VarVar(Reg),
    /// `$GLOBALS[<name reg>]`: a global variable by name.
    Global(Reg),
}

/// `$GLOBALS[<index>]`: the index expression when `e` is that form.
fn globals_index<'a>(e: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    match e {
        Expression::ArrayAccess(aa) if is_globals_var(aa.array) => Some(aa.index),
        _ => None,
    }
}

/// Is `e` the bare `$GLOBALS` variable?
fn is_globals_var(e: &Expression) -> bool {
    matches!(e, Expression::Variable(Variable::Direct(d)) if strip_dollar(d.name) == b"GLOBALS")
}

/// php's compile-time errors for `$GLOBALS` as (the root of) a write target:
/// only `$GLOBALS[$name]...` may be written.
fn check_globals_write(target: &Expression) -> Result<(), Thrown> {
    match target {
        e if is_globals_var(e) => Err(Thrown::error(
            "Error",
            "$GLOBALS can only be modified using the $GLOBALS[$name] = $value syntax",
        )),
        Expression::ArrayAppend(ap) if is_globals_var(ap.array) => {
            Err(Thrown::error("Error", "Cannot append to $GLOBALS"))
        }
        Expression::ArrayAccess(aa) if is_globals_var(aa.array) => Ok(()),
        Expression::ArrayAccess(aa) => check_globals_write(aa.array),
        Expression::ArrayAppend(ap) => check_globals_write(ap.array),
        Expression::Parenthesized(p) => check_globals_write(p.expression),
        _ => Ok(()),
    }
}

/// The body form of a closure being lowered.
enum ClosureBody<'a> {
    /// Arrow function: a single expression, implicitly `return`ed.
    Arrow(&'a Expression<'a>),
    /// `function(){ ... }`: a block of statements.
    Block(&'a rhp_frontend::cst::Block<'a>),
}
use rhp_values::{ArrayKey, PhpArray, PhpStr, Thrown, Value};

thread_local! {
    /// Sink for anonymous closure / arrow-function protos produced anywhere in
    /// the program. Compilation is single-threaded and non-reentrant per
    /// `compile()` call; the list is cleared on entry and drained into
    /// `Program::closures` on exit. A closure's `fnidx` is its position here.
    static CLOSURES: RefCell<Vec<Rc<FunctionProto>>> = const { RefCell::new(Vec::new()) };
    /// Anonymous classes (`new class {…}`) found anywhere in the program,
    /// declared with the program's classes under their generated names.
    static ANON_CLASSES: RefCell<Vec<ClassProto>> = const { RefCell::new(Vec::new()) };
    /// Sink for class declarations nested in control flow / function bodies
    /// (drained into `Program::cond_classes`), plus the file's count of user
    /// top-level class-likes (the `DeclareClass` index offset for them).
    static COND_CLASSES: RefCell<(Vec<ClassProto>, u32)> = const { RefCell::new((Vec::new(), 0)) };
}

/// The source bytes doc comments are looked up in, and its token stream
/// (lexed on the first lookup).
type DocSource = (Rc<Vec<u8>>, Option<Rc<Vec<rhp_frontend::Token>>>);

thread_local! {
    /// Method name (lowercase) -> per-position "could be by-reference" mask: true
    /// at position i if ANY method of that name declares `&` there. A call site
    /// passes those positions by reference; by-value callees simply deref, so
    /// over-approximation across classes is safe.
    static METHOD_REF: RefCell<HashMap<Vec<u8>, Vec<bool>>> = RefCell::new(HashMap::new());
    /// Byte offsets of each line start in the source currently being compiled
    /// (empty while compiling the prelude; whose line numbers are never surfaced).
    static LINE_STARTS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    /// The earliest compile error (source offset, line, message) recorded by
    /// [`compile_fatal`] in the file being compiled.
    static COMPILE_FATAL: RefCell<Option<(u32, u32, String)>> = const { RefCell::new(None) };
    /// See [`compile_error_line`].
    static COMPILE_ERROR_LINE: RefCell<u32> = const { RefCell::new(0) };
    /// The file name reported in debug info / exception getFile().
    static SRC_FILE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// The user source being compiled (empty for the prelude), for lexical
    /// details the AST drops (the newline a `?>` swallows).
    static SRC_BYTES: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// Whether the file being compiled declared `strict_types=1` (php's
    /// caller-decides coercion rule); stamped on every proto built from it.
    static STRICT_TYPES: RefCell<bool> = const { RefCell::new(false) };
    /// Whether a trait body is being compiled: `__CLASS__` there names the
    /// *using* class, so it is lowered to a runtime `self::class`.
    static IN_TRAIT: RefCell<bool> = const { RefCell::new(false) };
    /// Whether an interface body is being compiled (its property
    /// declarations, hooked or not, contribute no runtime members).
    static IN_INTERFACE: RefCell<bool> = const { RefCell::new(false) };
    /// The user source being compiled and its full token stream (comments
    /// included), for locating declarations' doc comments. Empty while
    /// compiling the prelude.
    static DOC_SRC: RefCell<DocSource> =
        RefCell::new((Rc::new(Vec::new()), None));
}

/// Lower a mago type hint to the bytecode [`rhp_bytecode::TypeHint`] (resolving
/// class names against the current namespace).
fn type_hint(h: &Hint) -> rhp_bytecode::TypeHint {
    let mut out = rhp_bytecode::TypeHint {
        nullable: false,
        intersection: false,
        kinds: Vec::new(),
        groups: Vec::new(),
        explicit_null: false,
    };
    collect_hint(h, &mut out);
    out
}

/// The class names of a (parenthesized) intersection hint `A&B&C`.
fn intersection_names(h: &Hint, out: &mut Vec<PhpStr>) {
    match h {
        Hint::Parenthesized(p) => intersection_names(p.hint, out),
        Hint::Intersection(i) => {
            intersection_names(i.left, out);
            intersection_names(i.right, out);
        }
        Hint::Identifier(id) => out.push(PhpStr::from_bytes(resolve_class_name(id))),
        _ => {}
    }
}

fn collect_hint(h: &Hint, out: &mut rhp_bytecode::TypeHint) {
    use rhp_bytecode::TypeKind as K;
    match h {
        Hint::Nullable(n) => {
            out.nullable = true;
            collect_hint(n.hint, out);
        }
        Hint::Parenthesized(p) => collect_hint(p.hint, out),
        Hint::Union(u) => {
            // A DNF member `(A&B)` of a union is an intersection group.
            for side in [u.left, u.right] {
                match side {
                    Hint::Parenthesized(p) if matches!(p.hint, Hint::Intersection(_)) => {
                        let mut g = Vec::new();
                        intersection_names(p.hint, &mut g);
                        out.groups.push(g);
                    }
                    Hint::Null(_) => {
                        out.nullable = true;
                        out.explicit_null = true;
                    }
                    other => collect_hint(other, out),
                }
            }
        }
        Hint::Intersection(i) => {
            out.intersection = true;
            collect_hint(i.left, out);
            collect_hint(i.right, out);
        }
        Hint::Null(_) => out.nullable = true,
        Hint::True(_) => out.kinds.push(K::True),
        Hint::False(_) => out.kinds.push(K::False),
        Hint::Array(_) => out.kinds.push(K::Array),
        Hint::Callable(_) => out.kinds.push(K::Callable),
        Hint::Static(_) => out.kinds.push(K::StaticType),
        Hint::Self_(_) => out.kinds.push(K::SelfType),
        Hint::Parent(_) => out.kinds.push(K::Parent),
        Hint::Void(_) => out.kinds.push(K::Void),
        Hint::Never(_) => out.kinds.push(K::Never),
        Hint::Float(_) => out.kinds.push(K::Float),
        Hint::Bool(_) => out.kinds.push(K::Bool),
        Hint::Integer(_) => out.kinds.push(K::Int),
        Hint::String(_) => out.kinds.push(K::String),
        Hint::Object(_) => out.kinds.push(K::Object),
        Hint::Mixed(_) => out.kinds.push(K::Mixed),
        Hint::Iterable(_) => out.kinds.push(K::Iterable),
        Hint::Identifier(id) => {
            check_type_name(id);
            out.kinds
                .push(K::Named(PhpStr::from_bytes(resolve_class_name(id))))
        }
    }
}

/// zend_compile_single_typename's compile errors for a class-name type: a
/// builtin type name written fully qualified (`\int`) or namespace-relative
/// (`namespace\static`) is "Type declaration '%s' must be unqualified" (php
/// 8.5.12 added `static` to the builtin list); `\self` is an invalid class
/// name; and a class name ending in a reserved word (`Foo\int`) is refused.
fn check_type_name(id: &Identifier) {
    use mago_span::HasSpan;
    const BUILTIN: [&[u8]; 13] = [
        b"null",
        b"true",
        b"false",
        b"int",
        b"float",
        b"string",
        b"bool",
        b"void",
        b"never",
        b"iterable",
        b"object",
        b"mixed",
        b"static",
    ];
    const RESERVED: [&[u8]; 17] = [
        b"bool",
        b"false",
        b"float",
        b"int",
        b"null",
        b"parent",
        b"self",
        b"static",
        b"string",
        b"true",
        b"void",
        b"never",
        b"iterable",
        b"object",
        b"mixed",
        b"array",
        b"callable",
    ];
    let at = id.span().start.offset;
    let (name, prefix): (Vec<u8>, &str) = match id {
        Identifier::FullyQualified(f) => (strip_lead_bs(f.value), "\\"),
        Identifier::Qualified(q) if starts_with_ci(q.value, b"namespace\\") => {
            (q.value[b"namespace\\".len()..].to_vec(), "namespace\\")
        }
        _ => (identifier_raw(id).to_vec(), ""),
    };
    let lc = name.to_ascii_lowercase();
    if !prefix.is_empty() && BUILTIN.contains(&lc.as_slice()) {
        let msg = format!(
            "Type declaration '{}' must be unqualified",
            String::from_utf8_lossy(&lc)
        );
        return compile_fatal(at, msg);
    }
    // zend_resolve_class_name: `\self` / `namespace\parent` name no class.
    if !prefix.is_empty() && (lc == b"self" || lc == b"parent") {
        let msg = format!(
            "'{prefix}{}' is an invalid class name",
            String::from_utf8_lossy(&name)
        );
        return compile_fatal(at, msg);
    }
    // zend_assert_valid_class_name(…, "a type name") on the resolved name,
    // whose unqualified tail must not be a reserved name (`Foo\int`).
    let resolved = resolve_class_name(id);
    let tail = resolved.rsplit(|&b| b == b'\\').next().unwrap_or(&resolved);
    if RESERVED.contains(&tail.to_ascii_lowercase().as_slice()) {
        let msg = format!(
            "Cannot use \"{}\" as a type name as it is reserved",
            String::from_utf8_lossy(&resolved)
        );
        compile_fatal(at, msg);
    }
}

/// Record an E_COMPILE_ERROR found at source `offset` of the file being
/// compiled. The compile still finishes (so callers need no error plumbing),
/// then fails with the earliest recorded error (see [`compile_program`]).
fn compile_fatal(offset: u32, message: String) {
    let line = line_of(offset);
    COMPILE_FATAL.with(|f| {
        let mut f = f.borrow_mut();
        if f.as_ref().is_none_or(|(o, _, _)| offset < *o) {
            *f = Some((offset, line, message));
        }
    });
}

/// The line of the compile error (E_COMPILE_ERROR) behind the last
/// `Thrown::Error { class: "CompileError", .. }` a compile entry point of this
/// thread returned. The SAPI reports it as a fatal error at that line.
pub fn compile_error_line() -> u32 {
    COMPILE_ERROR_LINE.with(|l| *l.borrow())
}

/// Whether the program's leading statement is `declare(strict_types=1)`.
fn detect_strict_types(program: &AstProgram) -> bool {
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

// --- namespaces --------------------------------------------------------------

/// The per-file, per-block namespace state threaded through compilation: the
/// current namespace and the `use` import aliases visible so far (textual order
/// — a reference only sees imports declared before it, matching PHP). Class and
/// function aliases are keyed case-insensitively (lowercased); constant aliases
/// are case-sensitive. Alias values are fully-qualified names WITHOUT a leading
/// backslash.
#[derive(Default, Clone)]
struct NsContext {
    ns: Vec<u8>, // "" = global namespace, else e.g. b"Foo\\Bar"
    use_class: HashMap<Vec<u8>, Vec<u8>>,
    use_func: HashMap<Vec<u8>, Vec<u8>>,
    use_const: HashMap<Vec<u8>, Vec<u8>>,
}

thread_local! {
    /// The namespace context active at the statement currently being compiled.
    /// Set from the flattened `(context, statement)` list before each statement
    /// in every compile pass, so deep name-resolution chokepoints can read it.
    static NS_CTX: RefCell<Rc<NsContext>> = RefCell::new(Rc::new(NsContext::default()));
}

/// Which symbol table an identifier resolves against (PHP resolves classes,
/// functions, and constants in separate namespace tables with different rules).
#[derive(Clone, Copy)]
enum NameKind {
    Class,
    Func,
    Const,
}

#[derive(Clone, Copy)]
enum UseKind {
    Class,
    Func,
    Const,
}

fn set_ns_ctx(ctx: &Rc<NsContext>) {
    NS_CTX.with(|c| *c.borrow_mut() = ctx.clone());
}

/// The current namespace (empty for the global namespace).
fn cur_ns() -> Vec<u8> {
    NS_CTX.with(|c| c.borrow().ns.clone())
}

/// Join a namespace prefix and a (relative) name with a backslash.
fn join_ns(ns: &[u8], name: &[u8]) -> Vec<u8> {
    if ns.is_empty() {
        return name.to_vec();
    }
    let mut v = Vec::with_capacity(ns.len() + 1 + name.len());
    v.extend_from_slice(ns);
    v.push(b'\\');
    v.extend_from_slice(name);
    v
}

/// Drop a single leading backslash, if present.
fn strip_lead_bs(name: &[u8]) -> Vec<u8> {
    if name.first() == Some(&b'\\') {
        name[1..].to_vec()
    } else {
        name.to_vec()
    }
}

fn starts_with_ci(hay: &[u8], pfx: &[u8]) -> bool {
    hay.len() >= pfx.len() && hay[..pfx.len()].eq_ignore_ascii_case(pfx)
}

/// The raw identifier bytes exactly as written (backslashes preserved), unlike
/// [`identifier_name`] which strips to the last segment.
fn identifier_raw<'a>(id: &'a Identifier<'a>) -> &'a [u8] {
    match id {
        Identifier::Local(l) => l.value,
        Identifier::Qualified(q) => q.value,
        Identifier::FullyQualified(f) => f.value,
    }
}

/// Resolve an identifier reference to `(primary_fqn, Option<global_fallback>)`
/// per PHP's namespace rules, using the current [`NsContext`]. Returned names
/// carry no leading backslash. A fallback is produced only for an unqualified
/// function/const reference inside a namespace (PHP's fall-back-to-global rule);
/// class references and everything qualified resolve to a single name.
fn resolve_name(id: &Identifier, kind: NameKind) -> (Vec<u8>, Option<Vec<u8>>) {
    NS_CTX.with(|c| resolve_in(&c.borrow(), id, kind))
}

fn resolve_in(ctx: &NsContext, id: &Identifier, kind: NameKind) -> (Vec<u8>, Option<Vec<u8>>) {
    match id {
        // `\Foo\Bar` — absolute; drop the leading backslash.
        Identifier::FullyQualified(f) => (strip_lead_bs(f.value), None),
        // `Foo\Bar` — qualified: `namespace\X` is relative to the current ns;
        // otherwise the first segment may be a class/namespace import alias, else
        // it is relative to the current namespace.
        Identifier::Qualified(q) => {
            let raw = q.value;
            if starts_with_ci(raw, b"namespace\\") {
                let rest = &raw[b"namespace\\".len()..];
                return (join_ns(&ctx.ns, rest), None);
            }
            let first_len = raw.iter().position(|&b| b == b'\\').unwrap_or(raw.len());
            let first_lc = raw[..first_len].to_ascii_lowercase();
            if let Some(fqn) = ctx.use_class.get(&first_lc) {
                let mut v = fqn.clone();
                v.extend_from_slice(&raw[first_len..]); // keep the remaining `\...`
                (v, None)
            } else {
                (join_ns(&ctx.ns, raw), None)
            }
        }
        // `Foo` — unqualified: import alias, else namespaced (with a global
        // fallback for functions/constants).
        Identifier::Local(l) => {
            let raw = l.value;
            match kind {
                NameKind::Class => match ctx.use_class.get(&raw.to_ascii_lowercase()) {
                    Some(fqn) => (fqn.clone(), None),
                    None => (join_ns(&ctx.ns, raw), None),
                },
                NameKind::Func => {
                    if let Some(fqn) = ctx.use_func.get(&raw.to_ascii_lowercase()) {
                        (fqn.clone(), None)
                    } else if ctx.ns.is_empty() {
                        (raw.to_vec(), None)
                    } else {
                        (join_ns(&ctx.ns, raw), Some(raw.to_vec()))
                    }
                }
                NameKind::Const => {
                    if let Some(fqn) = ctx.use_const.get(raw) {
                        (fqn.clone(), None)
                    } else if ctx.ns.is_empty() {
                        (raw.to_vec(), None)
                    } else {
                        (join_ns(&ctx.ns, raw), Some(raw.to_vec()))
                    }
                }
            }
        }
    }
}

/// Resolve a class-position identifier to its single fully-qualified name.
fn resolve_class_name(id: &Identifier) -> Vec<u8> {
    resolve_name(id, NameKind::Class).0
}

/// Flatten `namespace`/`use` statements into `(context, statement)` pairs in
/// source order: each `use` accumulates into the running context (so later
/// statements see it but earlier ones do not), and each `namespace` block starts
/// a fresh context. Every compile pass iterates this list so name resolution is
/// identical across passes.
fn flatten_namespaces<'a>(
    stmts: &'a rhp_frontend::cst::Sequence<'a, Statement<'a>>,
    ctx: Rc<NsContext>,
    out: &mut Vec<(Rc<NsContext>, &'a Statement<'a>)>,
) {
    let mut ctx = ctx;
    for stmt in stmts.iter() {
        match stmt {
            Statement::Namespace(ns) => {
                let name = ns.name.as_ref().map(identifier_raw).unwrap_or(b"");
                let base = Rc::new(NsContext {
                    ns: strip_lead_bs(name),
                    ..Default::default()
                });
                flatten_namespaces(ns.statements(), base, out);
            }
            Statement::Use(u) => {
                let mut c = (*ctx).clone();
                apply_use(&mut c, u);
                ctx = Rc::new(c);
            }
            other => out.push((Rc::clone(&ctx), other)),
        }
    }
}

fn apply_use(ctx: &mut NsContext, u: &rhp_frontend::cst::Use) {
    use rhp_frontend::cst::UseItems;
    match &u.items {
        UseItems::Sequence(seq) => {
            for it in seq.items.iter() {
                import_one(ctx, UseKind::Class, b"", it);
            }
        }
        UseItems::TypedSequence(ts) => {
            let k = use_kind(&ts.r#type);
            for it in ts.items.iter() {
                import_one(ctx, k, b"", it);
            }
        }
        UseItems::TypedList(tl) => {
            let k = use_kind(&tl.r#type);
            let pfx = strip_lead_bs(identifier_raw(&tl.namespace));
            for it in tl.items.iter() {
                import_one(ctx, k, &pfx, it);
            }
        }
        UseItems::MixedList(ml) => {
            let pfx = strip_lead_bs(identifier_raw(&ml.namespace));
            for mit in ml.items.iter() {
                let k = mit.r#type.as_ref().map(use_kind).unwrap_or(UseKind::Class);
                import_one(ctx, k, &pfx, &mit.item);
            }
        }
    }
}

fn use_kind(t: &rhp_frontend::cst::UseType) -> UseKind {
    use rhp_frontend::cst::UseType;
    match t {
        UseType::Function(_) => UseKind::Func,
        UseType::Const(_) => UseKind::Const,
    }
}

fn import_one(
    ctx: &mut NsContext,
    kind: UseKind,
    prefix: &[u8],
    item: &rhp_frontend::cst::UseItem,
) {
    let name_raw = strip_lead_bs(identifier_raw(&item.name));
    let full = if prefix.is_empty() {
        name_raw
    } else {
        join_ns(prefix, &name_raw)
    };
    let alias: Vec<u8> = match &item.alias {
        Some(a) => a.identifier.value.to_vec(),
        None => match full.iter().rposition(|&b| b == b'\\') {
            Some(p) => full[p + 1..].to_vec(),
            None => full.clone(),
        },
    };
    match kind {
        UseKind::Class => {
            ctx.use_class.insert(alias.to_ascii_lowercase(), full);
        }
        UseKind::Func => {
            ctx.use_func.insert(alias.to_ascii_lowercase(), full);
        }
        UseKind::Const => {
            ctx.use_const.insert(alias, full);
        }
    }
}

/// Sorted byte offsets of line starts (offset 0, then after each '\n').
fn build_line_starts(source: &[u8]) -> Vec<u32> {
    let mut v = vec![0u32];
    for (i, &b) in source.iter().enumerate() {
        if b == b'\n' {
            v.push((i + 1) as u32);
        }
    }
    v
}

/// The source file name currently being compiled (for `__FILE__`).
fn src_file() -> Vec<u8> {
    SRC_FILE.with(|f| f.borrow().clone())
}

/// If `e` is a `&$lvalue` reference-prefix expression, return the referenced
/// operand (used for by-reference array elements `[k => &$v]`).
fn as_reference<'a>(e: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    match e {
        Expression::UnaryPrefix(u) => match &u.operator {
            UnaryPrefixOperator::Reference(_) => Some(u.operand),
            _ => None,
        },
        _ => None,
    }
}

/// `dirname()` of a path, matching PHP for the file paths `__FILE__` yields:
/// no slash → ".", a single leading slash → "/", else everything before the
/// last slash.
fn dirname(path: &[u8]) -> Vec<u8> {
    match path.iter().rposition(|&b| b == b'/') {
        // php (zend_compile `__DIR__`): a "." directory — "Command line
        // code", "Standard input code" — becomes the current directory.
        None => std::env::current_dir()
            .map(|d| d.into_os_string().into_encoded_bytes())
            .unwrap_or_else(|_| b".".to_vec()),
        Some(0) => b"/".to_vec(),
        Some(i) => path[..i].to_vec(),
    }
}

/// The file being compiled (a class declaration's `info.user.filename`).
fn decl_file() -> PhpStr {
    SRC_FILE.with(|f| PhpStr::from_bytes(f.borrow().clone()))
}

/// 1-based line number for a byte offset, via the current `LINE_STARTS`.
/// php's lexer folds one newline (`\n`, `\r\n` or `\r`) right after `?>`
/// into the closing tag: drop it from inline text that starts there.
fn strip_close_tag_newline(offset: u32, text: &[u8]) -> &[u8] {
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

fn line_of(offset: u32) -> u32 {
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
fn locate(e: Thrown, offset: u32) -> Thrown {
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
fn collect_method_refs(flat: &[(Rc<NsContext>, &Statement)]) -> HashMap<Vec<u8>, Vec<bool>> {
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
    Ok(Program {
        main: Rc::new(c.finish()?),
        functions,
        classes,
        closures,
        num_prelude_classes,
        num_prelude_functions,
        cond_classes,
    })
}

// --- OOP: class declaration compilation ------------------------------------

fn modifiers_visibility(mods: &rhp_frontend::cst::Sequence<Modifier>) -> Visibility {
    for m in mods.iter() {
        match m {
            // `private(set)` etc. only restrict writes (asymmetric visibility,
            // not enforced yet); the read visibility is the plain modifier,
            // defaulting to public.
            Modifier::Private(_) => return Visibility::Private,
            Modifier::Protected(_) => return Visibility::Protected,
            Modifier::Public(_) => return Visibility::Public,
            _ => {}
        }
    }
    Visibility::Public
}

/// The asymmetric write visibility (`private(set)` / `protected(set)`), when
/// it is narrower than the read visibility.
fn modifiers_set_visibility(mods: &rhp_frontend::cst::Sequence<Modifier>) -> Option<Visibility> {
    let read = modifiers_visibility(mods);
    let set = mods.iter().find_map(|m| match m {
        Modifier::PrivateSet(_) => Some(Visibility::Private),
        Modifier::ProtectedSet(_) => Some(Visibility::Protected),
        Modifier::PublicSet(_) => Some(Visibility::Public),
        _ => None,
    })?;
    let rank = |v: Visibility| match v {
        Visibility::Public => 0,
        Visibility::Protected => 1,
        Visibility::Private => 2,
    };
    (rank(set) > rank(read)).then_some(set)
}

fn has_modifier(mods: &rhp_frontend::cst::Sequence<Modifier>, want: fn(&Modifier) -> bool) -> bool {
    mods.iter().any(want)
}

/// Whether the attribute lists carry `#[AllowDynamicProperties]`.
fn has_allow_dynamic(
    lists: &rhp_frontend::cst::Sequence<rhp_frontend::cst::AttributeList>,
) -> bool {
    lists.iter().any(|l| {
        l.attributes.iter().any(|a| {
            let n = identifier_raw(&a.name);
            let n = n.strip_prefix(b"\\").unwrap_or(n);
            n.eq_ignore_ascii_case(b"AllowDynamicProperties")
        })
    })
}

/// php's scanner keeps the last `/** */` comment until a declaration claims
/// it (its "backup" point: a function/class name, a property / parameter
/// variable, a constant name) or a `}` resets it. Approximate that by walking
/// back from `pos` over the tokens that can sit between a doc comment and the
/// claiming point (whitespace, other comments, attribute groups, modifiers and
/// keywords, type hints): the first doc comment reached is the declaration's.
fn doc_comment_before(pos: u32) -> Option<PhpStr> {
    use rhp_frontend::TokenKind as T;
    let (src, toks) = DOC_SRC.with(|d| {
        let mut d = d.borrow_mut();
        if d.1.is_none() {
            let has_doc = d.0.windows(3).any(|w| w == b"/**");
            let toks = if has_doc {
                rhp_frontend::tokens(&d.0)
            } else {
                Vec::new()
            };
            d.1 = Some(Rc::new(toks));
        }
        (d.0.clone(), d.1.clone().unwrap_or_default())
    });
    if toks.is_empty() {
        return None;
    }
    let pos = pos as usize;
    let mut i = toks.partition_point(|t| t.end <= pos);
    let mut attr_depth = 0usize;
    let mut paren_depth = 0usize;
    while i > 0 {
        i -= 1;
        let t = toks[i];
        if attr_depth > 0 {
            match t.kind {
                T::RightBracket => attr_depth += 1,
                T::LeftBracket | T::HashLeftBracket => attr_depth -= 1,
                _ => {}
            }
            continue;
        }
        match t.kind {
            T::DocBlockComment => {
                let text = &src[t.start..t.end];
                // `"/**"{WHITESPACE}` — `/**/` is a plain comment.
                if text.len() > 4 && text[3].is_ascii_whitespace() {
                    return Some(PhpStr::from_bytes(text.to_vec()));
                }
            }
            T::Whitespace | T::SingleLineComment | T::HashComment | T::MultiLineComment => {}
            T::RightBracket => attr_depth = 1,
            T::RightParenthesis => paren_depth += 1,
            T::LeftParenthesis => {
                if paren_depth == 0 {
                    return None;
                }
                paren_depth -= 1;
            }
            T::Identifier
            | T::QualifiedIdentifier
            | T::FullyQualifiedIdentifier
            | T::Question
            | T::Pipe
            | T::Ampersand
            | T::DotDotDot => {}
            k if k.is_keyword() => {}
            _ => return None,
        }
    }
    None
}

/// Compile `#[...]` attribute groups: each attribute's resolved class name
/// and its argument list as an array (folded when constant, else a thunk run
/// in `class_name`'s scope when Reflection asks for the arguments).
fn compile_attributes(
    lists: &rhp_frontend::cst::Sequence<rhp_frontend::cst::AttributeList>,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
    class_name: Option<&PhpStr>,
) -> Result<Vec<rhp_bytecode::AttributeProto>, Thrown> {
    use rhp_frontend::cst::PartialArgument as PA;
    let mut out = Vec::new();
    for list in lists.iter() {
        for a in list.attributes.iter() {
            let name = PhpStr::from_bytes(resolve_class_name(&a.name));
            let args = match &a.argument_list {
                None => None,
                Some(al) => {
                    // (name, expr) pairs; positional args have no name.
                    let mut items: Vec<(Option<&[u8]>, &Expression)> = Vec::new();
                    for arg in al.arguments.iter() {
                        match arg {
                            PA::Positional(p) => items.push((None, p.value)),
                            PA::Named(n) => items.push((Some(n.name.value), n.value)),
                            _ => {}
                        }
                    }
                    let folded: Result<Vec<Value>, Thrown> =
                        items.iter().map(|(_, e)| fold_const(e)).collect();
                    match folded {
                        Ok(vals) => {
                            let mut arr = PhpArray::new();
                            for ((n, _), v) in items.iter().zip(vals) {
                                match n {
                                    Some(n) => arr.insert(ArrayKey::from_bytes(n), v),
                                    None => arr.push(v),
                                }
                            }
                            Some(ConstInitProto::Const(Value::Array(arr)))
                        }
                        Err(_) => {
                            let mut c = ProtoBuilder::new(PhpStr::from("{init}"));
                            c.sigs = sigs.clone();
                            c.class_name = class_name.cloned();
                            c.scope_kind = ScopeKind::Init;
                            for (_, e) in &items {
                                c.scan_expr_vars(e);
                            }
                            c.num_cvs = c.cv_map.len() as u16;
                            c.next_temp = c.num_cvs;
                            c.high_water = c.num_cvs;
                            c.temp_floor = c.num_cvs;
                            let dst = c.alloc_temp();
                            c.emit(Instr::NewArray { dst, cap: 0 });
                            for (n, e) in &items {
                                let val = c.compile_expr(e)?;
                                match n {
                                    Some(n) => {
                                        let key = c.load_string_const(n);
                                        c.emit(Instr::ArrSet { arr: dst, key, val });
                                    }
                                    None => c.emit(Instr::ArrPush { arr: dst, val }),
                                }
                            }
                            c.emit(Instr::Ret { src: dst });
                            Some(ConstInitProto::Thunk(Rc::new(c.finish()?)))
                        }
                    }
                }
            };
            out.push(rhp_bytecode::AttributeProto { name, args });
        }
    }
    Ok(out)
}

/// Reflection metadata of a declaration: its attributes, the doc comment
/// claimed at `doc_pos`, and the line span `[start, end]` (byte offsets).
fn decl_meta(
    lists: &rhp_frontend::cst::Sequence<rhp_frontend::cst::AttributeList>,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
    class_name: Option<&PhpStr>,
    doc_pos: u32,
    start: u32,
    end: u32,
) -> Result<rhp_bytecode::DeclMeta, Thrown> {
    Ok(rhp_bytecode::DeclMeta {
        attributes: compile_attributes(lists, sigs, class_name)?,
        doc_comment: doc_comment_before(doc_pos),
        start_line: line_of(start),
        offset: start,
        end_line: line_of(end.saturating_sub(1).max(start)),
    })
}

/// Reflection data for a parameter default: a standalone thunk for a
/// non-constant default, and the constant's name when the default is a bare
/// constant / class-constant reference.
fn param_default_meta(
    p: &rhp_frontend::cst::FunctionLikeParameter,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
    class_name: Option<&PhpStr>,
) -> (Option<Rc<FunctionProto>>, Option<PhpStr>) {
    let Some(dv) = &p.default_value else {
        return (None, None);
    };
    let e = dv.value;
    let cname = match e {
        Expression::ConstantAccess(ca) => {
            let n = identifier_raw(&ca.name);
            let n = n.strip_prefix(b"\\").unwrap_or(n);
            let lcn = n.to_ascii_lowercase();
            (!matches!(lcn.as_slice(), b"true" | b"false" | b"null"))
                .then(|| PhpStr::from_bytes(n.to_vec()))
        }
        Expression::Access(Access::ClassConstant(cca)) => {
            let class: Option<Vec<u8>> = match cca.class {
                Expression::Identifier(id) => Some(identifier_raw(id).to_vec()),
                Expression::Self_(_) => Some(b"self".to_vec()),
                Expression::Static(_) => Some(b"static".to_vec()),
                Expression::Parent(_) => Some(b"parent".to_vec()),
                _ => None,
            };
            match (&cca.constant, class) {
                (rhp_frontend::cst::ClassLikeConstantSelector::Identifier(c), Some(mut cl)) => {
                    cl.extend_from_slice(b"::");
                    cl.extend_from_slice(c.value);
                    Some(PhpStr::from_bytes(cl))
                }
                _ => None,
            }
        }
        _ => None,
    };
    let thunk = if fold_const(e).is_err() {
        compile_const_thunk(e, sigs, class_name).ok()
    } else {
        None
    };
    (thunk, cname)
}

/// php's `add_stringable_interface`: a class-like (not a trait) declaring
/// `__toString` implicitly implements `Stringable`.
fn add_stringable(interfaces: &mut Vec<PhpStr>, methods: &[MethodProto], self_name: &PhpStr) {
    if self_name.as_bytes().eq_ignore_ascii_case(b"Stringable") {
        return;
    }
    if methods
        .iter()
        .any(|m| m.name.as_bytes().eq_ignore_ascii_case(b"__toString"))
        && !interfaces
            .iter()
            .any(|i| i.as_bytes().eq_ignore_ascii_case(b"Stringable"))
    {
        interfaces.push(PhpStr::from("Stringable"));
    }
}

/// The span offsets `(start, end)` of an AST node.
fn span_of(n: &impl mago_span::HasSpan) -> (u32, u32) {
    let s = n.span();
    (s.start.offset, s.end.offset)
}

fn compile_class(
    class: &Class,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
) -> Result<ClassProto, Thrown> {
    let name = PhpStr::from_bytes(join_ns(&cur_ns(), class.name.value));
    let flags = ClassFlags {
        is_abstract: has_modifier(&class.modifiers, |m| matches!(m, Modifier::Abstract(_))),
        is_final: has_modifier(&class.modifiers, |m| matches!(m, Modifier::Final(_))),
        is_readonly: has_modifier(&class.modifiers, |m| matches!(m, Modifier::Readonly(_))),
        allow_dynamic: has_allow_dynamic(&class.attribute_lists),
        ..ClassFlags::default()
    };
    // Single inheritance: a class `extends` one parent.
    let parent = class.extends.as_ref().and_then(|e| {
        e.types
            .iter()
            .next()
            .map(|id| PhpStr::from_bytes(resolve_class_name(id)))
    });
    let interfaces: Vec<PhpStr> = class
        .implements
        .as_ref()
        .map(|i| {
            i.types
                .iter()
                .map(|id| PhpStr::from_bytes(resolve_class_name(id)))
                .collect()
        })
        .unwrap_or_default();

    let m = compile_members(&class.members, sigs, &name)?;
    let mut interfaces = interfaces;
    add_stringable(&mut interfaces, &m.methods, &name);
    let meta = decl_meta(
        &class.attribute_lists,
        sigs,
        Some(&name),
        class.name.span.start.offset,
        class.class.span.start.offset,
        class.right_brace.end.offset,
    )?;
    Ok(ClassProto {
        name,
        kind: ClassKind::Class,
        flags,
        parent,
        interfaces,
        traits: m.traits,
        methods: m.methods,
        props: m.props,
        static_props: m.static_props,
        constants: m.constants,
        enum_cases: Vec::new(),
        enum_backing: None,
        meta,
        decl_file: decl_file(),
        decl_line: line_of(class.class.span.start.offset),
    })
}

/// Compile an anonymous class body under its generated `name`
/// (`Prefix@anonymous\0file:line$n`, zend_generate_anon_class_name).
fn compile_anon_class(
    ac: &rhp_frontend::cst::AnonymousClass,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
) -> Result<ClassProto, Thrown> {
    use mago_span::HasSpan;
    let parent = ac.extends.as_ref().and_then(|e| {
        e.types
            .iter()
            .next()
            .map(|id| PhpStr::from_bytes(resolve_class_name(id)))
    });
    let interfaces: Vec<PhpStr> = ac
        .implements
        .as_ref()
        .map(|i| {
            i.types
                .iter()
                .map(|id| PhpStr::from_bytes(resolve_class_name(id)))
                .collect()
        })
        .unwrap_or_default();
    let mut name = parent
        .as_ref()
        .or(interfaces.first())
        .map(|n| n.as_bytes().to_vec())
        .unwrap_or_else(|| b"class".to_vec());
    name.extend_from_slice(b"@anonymous\0");
    name.extend_from_slice(&src_file());
    let n = ANON_CLASSES.with(|c| c.borrow().len());
    name.extend_from_slice(format!(":{}${:x}", line_of(ac.span().start.offset), n).as_bytes());
    let name = PhpStr::from_bytes(name);
    let flags = ClassFlags {
        is_abstract: false,
        is_final: true,
        is_readonly: has_modifier(&ac.modifiers, |m| matches!(m, Modifier::Readonly(_))),
        allow_dynamic: has_allow_dynamic(&ac.attribute_lists),
        // zend_compile_class_decl: every anonymous class.
        not_serializable: true,
        ..ClassFlags::default()
    };
    let m = compile_members(&ac.members, sigs, &name)?;
    let mut interfaces = interfaces;
    add_stringable(&mut interfaces, &m.methods, &name);
    let meta = decl_meta(
        &ac.attribute_lists,
        sigs,
        Some(&name),
        ac.class.span.start.offset,
        ac.class.span.start.offset,
        ac.right_brace.end.offset,
    )?;
    Ok(ClassProto {
        decl_file: decl_file(),
        decl_line: line_of(ac.class.span.start.offset),
        name,
        kind: ClassKind::Class,
        flags,
        parent,
        interfaces,
        traits: m.traits,
        methods: m.methods,
        props: m.props,
        static_props: m.static_props,
        constants: m.constants,
        enum_cases: Vec::new(),
        enum_backing: None,
        meta,
    })
}

/// Compile an `enum` declaration into a `ClassProto` of kind `Enum`.
///
/// Enum cases are collected and materialized by the VM at link time (one
/// singleton object per case, exposed as class constants). The enum implements
/// `UnitEnum` (pure) or `BackedEnum` (backed), plus any declared interfaces.
fn compile_enum(
    en: &rhp_frontend::cst::Enum,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
) -> Result<ClassProto, Thrown> {
    use rhp_bytecode::EnumBacking;
    let name = PhpStr::from_bytes(join_ns(&cur_ns(), en.name.value));

    let backing = match &en.backing_type_hint {
        None => None,
        Some(h) => match &h.hint {
            Hint::Integer(_) => Some(EnumBacking::Int),
            Hint::String(_) => Some(EnumBacking::Str),
            _ => {
                return Err(Thrown::unsupported(
                    "enum backing type",
                    "only int and string backings are valid",
                ))
            }
        },
    };

    let mut interfaces: Vec<PhpStr> = en
        .implements
        .as_ref()
        .map(|i| {
            i.types
                .iter()
                .map(|id| PhpStr::from_bytes(resolve_class_name(id)))
                .collect()
        })
        .unwrap_or_default();
    // Every enum implements UnitEnum; a backed enum also implements BackedEnum.
    interfaces.push(PhpStr::from("UnitEnum"));
    if backing.is_some() {
        interfaces.push(PhpStr::from("BackedEnum"));
    }

    let m = compile_members(&en.members, sigs, &name)?;
    add_stringable(&mut interfaces, &m.methods, &name);
    let meta = decl_meta(
        &en.attribute_lists,
        sigs,
        Some(&name),
        en.name.span.start.offset,
        en.r#enum.span.start.offset,
        en.right_brace.end.offset,
    )?;
    Ok(ClassProto {
        meta,
        name,
        kind: ClassKind::Enum,
        // Enums are implicitly final.
        flags: ClassFlags {
            is_final: true,
            ..ClassFlags::default()
        },
        parent: None,
        interfaces,
        traits: m.traits,
        methods: m.methods,
        props: m.props,
        static_props: m.static_props,
        constants: m.constants,
        enum_cases: m.enum_cases,
        enum_backing: backing,
        decl_file: decl_file(),
        decl_line: line_of(en.r#enum.span.start.offset),
    })
}

/// Compile a (possibly non-constant) initializer expression — a class constant,
/// static-property, or instance-property default — into a zero-argument thunk
/// the VM evaluates in the class's scope after all classes are linked. This is
/// how `const X = self::Y` / `private State $s = State::Idle` resolve.
fn compile_const_thunk(
    expr: &Expression,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
    class_name: Option<&PhpStr>,
) -> Result<Rc<FunctionProto>, Thrown> {
    let mut c = ProtoBuilder::new(PhpStr::from("{init}"));
    c.sigs = sigs.clone();
    c.class_name = class_name.cloned(); // for `__CLASS__`
    c.scope_kind = ScopeKind::Init;
    c.scan_expr_vars(expr); // const-exprs use no $vars, but be safe
    c.scan_node_vars(Node::Expression(expr));
    c.freeze_cvs();
    let r = c.compile_expr(expr)?;
    c.emit(Instr::Ret { src: r });
    Ok(Rc::new(c.finish()?))
}

/// Fold an initializer to a constant, or fall back to a runtime thunk.
fn const_init_or_thunk(
    expr: &Expression,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
    class_name: Option<&PhpStr>,
) -> Result<ConstInitProto, Thrown> {
    match fold_const(expr) {
        Ok(v) => Ok(ConstInitProto::Const(v)),
        Err(_) => Ok(ConstInitProto::Thunk(compile_const_thunk(
            expr, sigs, class_name,
        )?)),
    }
}

fn prop_default_or_thunk(
    expr: &Expression,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
    class_name: Option<&PhpStr>,
) -> Result<PropDefaultProto, Thrown> {
    match fold_const(expr) {
        Ok(v) => Ok(PropDefaultProto::Const(v)),
        Err(_) => Ok(PropDefaultProto::Init(compile_const_thunk(
            expr, sigs, class_name,
        )?)),
    }
}

/// php's `zend_is_valid_default_value`: a literal int initializer of a
/// property / class constant whose type admits float but not int becomes a
/// float at compile time.
fn widen_int_default(v: Value, hint: Option<&rhp_bytecode::TypeHint>) -> Value {
    use rhp_bytecode::TypeKind as K;
    match (v, hint) {
        (Value::Int(n), Some(h))
            if h.kinds.contains(&K::Float)
                && !h.kinds.iter().any(|k| matches!(k, K::Int | K::Mixed)) =>
        {
            Value::Float(n as f64)
        }
        (v, _) => v,
    }
}

fn coerce_prop_default(
    d: PropDefaultProto,
    hint: Option<&rhp_bytecode::TypeHint>,
) -> PropDefaultProto {
    match d {
        PropDefaultProto::Const(v) => PropDefaultProto::Const(widen_int_default(v, hint)),
        other => other,
    }
}

fn coerce_const_init(d: ConstInitProto, hint: Option<&rhp_bytecode::TypeHint>) -> ConstInitProto {
    match d {
        ConstInitProto::Const(v) => ConstInitProto::Const(widen_int_default(v, hint)),
        other => other,
    }
}

#[derive(Default)]
struct CompiledMembers {
    methods: Vec<MethodProto>,
    props: Vec<PropProto>,
    static_props: Vec<StaticPropProto>,
    constants: Vec<ConstProto>,
    traits: Vec<PhpStr>,
    enum_cases: Vec<rhp_bytecode::EnumCaseProto>,
}

fn compile_members(
    members: &rhp_frontend::cst::Sequence<ClassLikeMember>,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
    class_name: &PhpStr,
) -> Result<CompiledMembers, Thrown> {
    let mut out = CompiledMembers::default();
    for member in members.iter() {
        use mago_span::HasSpan;
        compile_member(&mut out, member, sigs, class_name)
            .map_err(|e| locate(e, member.span().start.offset))?;
    }
    Ok(out)
}

/// Compile one class-like member into `out`.
fn compile_member(
    out: &mut CompiledMembers,
    member: &ClassLikeMember,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
    class_name: &PhpStr,
) -> Result<(), Thrown> {
    match member {
        ClassLikeMember::Method(m) => {
            let (mp, promoted) = compile_method(m, sigs, class_name)?;
            out.methods.push(mp);
            out.props.extend(promoted);
        }
        ClassLikeMember::Property(_) if IN_INTERFACE.with(|f| *f.borrow()) => {}
        ClassLikeMember::Property(Property::Plain(p)) => {
            let visibility = modifiers_visibility(&p.modifiers);
            let set_visibility = modifiers_set_visibility(&p.modifiers);
            let is_static = has_modifier(&p.modifiers, |m| matches!(m, Modifier::Static(_)));
            let is_readonly = has_modifier(&p.modifiers, |m| matches!(m, Modifier::Readonly(_)));
            let hint = p.hint.as_ref().map(type_hint);
            let attributes = compile_attributes(&p.attribute_lists, sigs, Some(class_name))?;
            let (pstart, pend) = span_of(p);
            for item in p.items.iter() {
                let (var, default) = match item {
                    PropertyItem::Concrete(c) => (
                        &c.variable,
                        coerce_prop_default(
                            prop_default_or_thunk(c.value, sigs, Some(class_name))?,
                            hint.as_ref(),
                        ),
                    ),
                    PropertyItem::Abstract(a) => (&a.variable, PropDefaultProto::None),
                };
                let meta = rhp_bytecode::DeclMeta {
                    attributes: attributes.clone(),
                    doc_comment: doc_comment_before(var.span.start.offset),
                    start_line: line_of(pstart),
                    offset: var.span.start.offset,
                    end_line: line_of(pend),
                };
                let pname = PhpStr::from_bytes(strip_dollar(var.name).to_vec());
                if is_static {
                    let has_default = !matches!(default, PropDefaultProto::None);
                    let init = match default {
                        PropDefaultProto::Const(v) => ConstInitProto::Const(v),
                        PropDefaultProto::Init(t) => ConstInitProto::Thunk(t),
                        PropDefaultProto::None => ConstInitProto::Const(Value::Null),
                    };
                    out.static_props.push(StaticPropProto {
                        name: pname,
                        visibility,
                        is_readonly,
                        init,
                        type_hint: hint.clone(),
                        meta,
                        has_default,
                    });
                } else {
                    out.props.push(PropProto {
                        name: pname,
                        visibility,
                        set_visibility,
                        is_readonly,
                        default,
                        type_hint: hint.clone(),
                        meta,
                        promoted: false,
                    });
                }
            }
        }
        ClassLikeMember::Property(Property::Hooked(_)) => {
            return Err(Thrown::unsupported("property hooks", "deferred"))
        }
        ClassLikeMember::Constant(c) => {
            let visibility = modifiers_visibility(&c.modifiers);
            let hint = c.hint.as_ref().map(type_hint);
            let is_final = has_modifier(&c.modifiers, |m| matches!(m, Modifier::Final(_)));
            let attributes = compile_attributes(&c.attribute_lists, sigs, Some(class_name))?;
            let (cstart, cend) = span_of(c);
            for item in c.items.iter() {
                out.constants.push(ConstProto {
                    name: PhpStr::from_bytes(item.name.value.to_vec()),
                    visibility,
                    value: coerce_const_init(
                        const_init_or_thunk(item.value, sigs, Some(class_name))?,
                        hint.as_ref(),
                    ),
                    type_hint: hint.clone(),
                    meta: rhp_bytecode::DeclMeta {
                        attributes: attributes.clone(),
                        doc_comment: doc_comment_before(item.name.span.start.offset),
                        start_line: line_of(cstart),
                        offset: item.name.span.start.offset,
                        end_line: line_of(cend),
                    },
                    is_final,
                });
            }
        }
        ClassLikeMember::TraitUse(tu) => {
            for t in tu.trait_names.iter() {
                out.traits.push(PhpStr::from_bytes(resolve_class_name(t)));
            }
        }
        ClassLikeMember::EnumCase(ec) => {
            use rhp_frontend::cst::EnumCaseItem;
            let (name, value) = match &ec.item {
                EnumCaseItem::Unit(u) => (PhpStr::from_bytes(u.name.value.to_vec()), None),
                EnumCaseItem::Backed(b) => (
                    PhpStr::from_bytes(b.name.value.to_vec()),
                    Some(const_init_or_thunk(b.value, sigs, Some(class_name))?),
                ),
            };
            let name_off = match &ec.item {
                EnumCaseItem::Unit(u) => u.name.span.start.offset,
                EnumCaseItem::Backed(b) => b.name.span.start.offset,
            };
            let (cstart, cend) = span_of(ec);
            let meta = decl_meta(
                &ec.attribute_lists,
                sigs,
                Some(class_name),
                name_off,
                cstart,
                cend,
            )?;
            out.enum_cases
                .push(rhp_bytecode::EnumCaseProto { name, value, meta });
        }
    }
    Ok(())
}

fn compile_trait(
    t: &rhp_frontend::cst::Trait,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
) -> Result<ClassProto, Thrown> {
    let name = PhpStr::from_bytes(join_ns(&cur_ns(), t.name.value));
    let saved = IN_TRAIT.with(|f| std::mem::replace(&mut *f.borrow_mut(), true));
    let m = compile_members(&t.members, sigs, &name);
    IN_TRAIT.with(|f| *f.borrow_mut() = saved);
    let m = m?;
    let meta = decl_meta(
        &t.attribute_lists,
        sigs,
        Some(&name),
        t.name.span.start.offset,
        t.r#trait.span.start.offset,
        t.right_brace.end.offset,
    )?;
    Ok(ClassProto {
        meta,
        decl_file: decl_file(),
        decl_line: line_of(t.r#trait.span.start.offset),
        name,
        kind: ClassKind::Trait,
        flags: ClassFlags::default(),
        parent: None,
        interfaces: Vec::new(),
        traits: m.traits,
        methods: m.methods,
        props: m.props,
        static_props: m.static_props,
        constants: m.constants,
        enum_cases: Vec::new(),
        enum_backing: None,
    })
}

/// The built-in exception/throwable class hierarchy, as PHP source. Compiled
/// and prepended to every program so `throw`/`catch` and engine errors work.
const PRELUDE: &str = r##"<?php
interface Stringable { public function __toString(): string; }
interface Throwable extends Stringable {
    public function getMessage(): string;
    public function getCode();
    public function getFile(): string;
    public function getLine(): int;
    public function getTrace(): array;
    public function getPrevious(): ?Throwable;
    public function getTraceAsString(): string;
}
interface UnitEnum {}
interface BackedEnum extends UnitEnum {}
interface Traversable {}
interface Iterator extends Traversable {
    public function current(): mixed;
    public function next(): void;
    public function key(): mixed;
    public function valid(): bool;
    public function rewind(): void;
}
interface IteratorAggregate extends Traversable { public function getIterator(): Traversable; }
interface OuterIterator extends Iterator { public function getInnerIterator(): ?Iterator; }
interface RecursiveIterator extends Iterator {
    public function hasChildren(): bool;
    public function getChildren(): ?RecursiveIterator;
}
interface JsonSerializable { public function jsonSerialize(): mixed; }
interface Serializable {
    public function serialize();
    public function unserialize(string $data);
}
interface ArrayAccess {
    public function offsetExists(mixed $offset): bool;
    public function offsetGet(mixed $offset): mixed;
    public function offsetSet(mixed $offset, mixed $value): void;
    public function offsetUnset(mixed $offset): void;
}
interface Countable { public function count(): int; }
final class Generator implements Iterator {}
class stdClass {}
final class __PHP_Incomplete_Class {}
final class Closure {
    public static function fromCallable($callback) { return __closure_from_callable($callback); }
    public static function bind($closure, $newThis, $newScope = 'static') { return __closure_bind($closure, $newThis, $newScope); }
}
class Exception implements Throwable {
    protected $message = "";
    private $string = "";
    protected $code = 0;
    protected $file = "";
    protected $line = 0;
    private $trace = [];
    private $previous = null;
    public function __construct($message = "", $code = 0, $previous = null) {
        $n = func_num_args();
        if ($n >= 1) { $this->message = __zpp_arg("Exception::__construct", 1, "message", "string", $message); }
        if ($n >= 2) { $c = __zpp_arg("Exception::__construct", 2, "code", "int", $code); if ($c) { $this->code = $c; } }
        if ($n >= 3) { $p = __zpp_arg("Exception::__construct", 3, "previous", "?Throwable", $previous); if ($p !== null) { $this->previous = $p; } }
    }
    final public function getMessage() { return $this->message; }
    final public function getCode() { return $this->code; }
    final public function getPrevious() { return $this->previous; }
    final public function getFile() { return $this->file; }
    final public function getLine() { return $this->line; }
    final public function getTrace() { return $this->trace; }
    final public function getTraceAsString() { return __exception_trace_string($this->trace); }
    public function __toString() { return __throwable_string($this); }
}
class Error implements Throwable {
    protected $message = "";
    private $string = "";
    protected $code = 0;
    protected $file = "";
    protected $line = 0;
    private $trace = [];
    private $previous = null;
    public function __construct($message = "", $code = 0, $previous = null) {
        $n = func_num_args();
        if ($n >= 1) { $this->message = __zpp_arg("Error::__construct", 1, "message", "string", $message); }
        if ($n >= 2) { $c = __zpp_arg("Error::__construct", 2, "code", "int", $code); if ($c) { $this->code = $c; } }
        if ($n >= 3) { $p = __zpp_arg("Error::__construct", 3, "previous", "?Throwable", $previous); if ($p !== null) { $this->previous = $p; } }
    }
    final public function getMessage() { return $this->message; }
    final public function getCode() { return $this->code; }
    final public function getPrevious() { return $this->previous; }
    final public function getFile() { return $this->file; }
    final public function getLine() { return $this->line; }
    final public function getTrace() { return $this->trace; }
    final public function getTraceAsString() { return __exception_trace_string($this->trace); }
    public function __toString() { return __throwable_string($this); }
}
class ErrorException extends Exception {
    protected $severity = 1;
    public function __construct($message = "", $code = 0, $severity = 1, $filename = null, $line = null, $previous = null) {
        $n = func_num_args();
        $f = "ErrorException::__construct";
        if ($n >= 1) { $this->message = __zpp_arg($f, 1, "message", "string", $message); }
        if ($n >= 2) { $c = __zpp_arg($f, 2, "code", "int", $code); if ($c) { $this->code = $c; } }
        if ($n >= 3) { $severity = __zpp_arg($f, 3, "severity", "int", $severity); }
        if ($n >= 4) { $filename = __zpp_arg($f, 4, "filename", "?string", $filename); }
        if ($n >= 5) { $line = __zpp_arg($f, 5, "line", "?int", $line); }
        if ($n >= 6) { $p = __zpp_arg($f, 6, "previous", "?Throwable", $previous); if ($p !== null) { $this->previous = $p; } }
        $this->severity = $severity;
        if ($filename !== null) { $this->file = $filename; }
        if ($line !== null) { $this->line = $line; } elseif ($filename !== null) { $this->line = 0; }
    }
    final public function getSeverity() { return $this->severity; }
}
class TypeError extends Error {}
class ValueError extends Error {}
class ArithmeticError extends Error {}
class DivisionByZeroError extends ArithmeticError {}
class ArgumentCountError extends TypeError {}
class UnhandledMatchError extends Error {}
class CompileError extends Error {}
class ParseError extends CompileError {}
class AssertionError extends Error {}
class LogicException extends Exception {}
class InvalidArgumentException extends LogicException {}
class LengthException extends LogicException {}
class DomainException extends LogicException {}
class BadFunctionCallException extends LogicException {}
class BadMethodCallException extends BadFunctionCallException {}
class RuntimeException extends Exception {}
class OutOfBoundsException extends RuntimeException {}
class OutOfRangeException extends LogicException {}
class RangeException extends RuntimeException {}
class OverflowException extends RuntimeException {}
class UnderflowException extends RuntimeException {}
class UnexpectedValueException extends RuntimeException {}
class JsonException extends Exception {}
class RequestParseBodyException extends Exception {}
"##;

/// The compiled prelude: built-in classes, helper functions, and the
/// closures their bodies declare (always program closures `0..n`).
struct PreludeParts {
    classes: Vec<ClassProto>,
    functions: Vec<Rc<FunctionProto>>,
    closures: Vec<Rc<FunctionProto>>,
}

thread_local! {
    /// The prelude, compiled on first use and shared by every later compile on
    /// this thread (protos hold `Rc`s, so the cache is per thread — matching
    /// the VM-per-thread model). It depends on no per-file state: it is
    /// compiled in an isolated compiler context (see [`prelude_classes`]).
    static PRELUDE_CACHE: RefCell<Option<Rc<PreludeParts>>> = const { RefCell::new(None) };
}

/// The compiled prelude, compiling it on this thread's first call.
fn prelude_classes() -> Result<Rc<PreludeParts>, Thrown> {
    if let Some(p) = PRELUDE_CACHE.with(|c| c.borrow().clone()) {
        return Ok(p);
    }
    // Compiled once per thread, kept for every later request: not counted
    // against a request's memory.
    let parts = rhp_values::mem::uncounted(|| prelude_classes_isolated().map(Rc::new))?;
    PRELUDE_CACHE.with(|c| *c.borrow_mut() = Some(parts.clone()));
    Ok(parts)
}

/// Compile the prelude with the per-file compiler state (thread-locals) set to
/// a fixed, prelude-only context, restoring the caller's state afterwards, so
/// the result is independent of whichever file happened to trigger it.
fn prelude_classes_isolated() -> Result<PreludeParts, Thrown> {
    // Prelude / built-in classes are never strict-typed (coercive calls).
    let saved_strict = STRICT_TYPES.with(|s| std::mem::replace(&mut *s.borrow_mut(), false));
    // Prelude protos carry an empty file name: the VM treats them as internal
    // code (skipped for exception file/line, `[internal function]` in traces).
    let saved_file = SRC_FILE.with(|f| std::mem::take(&mut *f.borrow_mut()));
    // No line map: prelude line numbers are never surfaced.
    let saved_lines = LINE_STARTS.with(|l| std::mem::take(&mut *l.borrow_mut()));
    let saved_ns =
        NS_CTX.with(|c| std::mem::replace(&mut *c.borrow_mut(), Rc::new(NsContext::default())));
    let saved_trait = IN_TRAIT.with(|t| std::mem::replace(&mut *t.borrow_mut(), false));
    let saved_method_ref = METHOD_REF.with(|m| std::mem::take(&mut *m.borrow_mut()));
    let saved_closures = CLOSURES.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let saved_anon = ANON_CLASSES.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let saved_cond = COND_CLASSES.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let r = prelude_classes_inner();
    // Closures the prelude declares occupy closure indices `0..n`; a program
    // with the prelude re-seeds them first. Anonymous / nested classes would
    // index per-file tables the cached copy cannot carry.
    let closures = CLOSURES.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let leaked = ANON_CLASSES.with(|c| !c.borrow().is_empty())
        || COND_CLASSES.with(|c| !c.borrow().0.is_empty());
    STRICT_TYPES.with(|s| *s.borrow_mut() = saved_strict);
    SRC_FILE.with(|f| *f.borrow_mut() = saved_file);
    LINE_STARTS.with(|l| *l.borrow_mut() = saved_lines);
    NS_CTX.with(|c| *c.borrow_mut() = saved_ns);
    IN_TRAIT.with(|t| *t.borrow_mut() = saved_trait);
    METHOD_REF.with(|m| *m.borrow_mut() = saved_method_ref);
    CLOSURES.with(|c| *c.borrow_mut() = saved_closures);
    ANON_CLASSES.with(|c| *c.borrow_mut() = saved_anon);
    COND_CLASSES.with(|c| *c.borrow_mut() = saved_cond);
    assert!(
        !leaked,
        "prelude must not declare anonymous or nested classes"
    );
    let (classes, functions) = r?;
    Ok(PreludeParts {
        classes,
        functions,
        closures,
    })
}

#[allow(clippy::type_complexity)]
fn prelude_classes_inner() -> Result<(Vec<ClassProto>, Vec<Rc<FunctionProto>>), Thrown> {
    // The engine's core classes (PRELUDE) plus each extension module's own
    // PHP-source classes (GMP, Phar, …) and helper functions (iterator_*, …).
    let src = format!("{PRELUDE}\n{}", rhp_builtins::module_prelude());
    let arena = rhp_frontend::Arena::new();
    let parsed = arena.parse(b"<prelude>", src.as_bytes());
    // Method calls inside the prelude see the prelude's own by-ref signatures.
    let flat: Vec<(Rc<NsContext>, &Statement)> = parsed
        .program()
        .statements
        .iter()
        .map(|s| (Rc::new(NsContext::default()), s))
        .collect();
    METHOD_REF.with(|m| *m.borrow_mut() = collect_method_refs(&flat));
    let sigs = Rc::new(std::collections::HashMap::new());
    let mut classes = Vec::new();
    let mut functions = Vec::new();
    for stmt in parsed.program().statements.iter() {
        match stmt {
            Statement::Class(c) => classes.push(compile_class(c, &sigs)?),
            Statement::Interface(i) => classes.push(compile_interface(i, &sigs)?),
            Statement::Function(f) => functions.push(Rc::new(compile_function(f, sigs.clone())?)),
            _ => {}
        }
    }
    Ok((classes, functions))
}

fn compile_interface(
    iface: &rhp_frontend::cst::Interface,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
) -> Result<ClassProto, Thrown> {
    let name = PhpStr::from_bytes(join_ns(&cur_ns(), iface.name.value));
    let interfaces: Vec<PhpStr> = iface
        .extends
        .as_ref()
        .map(|e| {
            e.types
                .iter()
                .map(|id| PhpStr::from_bytes(resolve_class_name(id)))
                .collect()
        })
        .unwrap_or_default();
    // Constants and (abstract) method signatures; interface properties
    // contribute no runtime members.
    let saved = IN_INTERFACE.with(|f| std::mem::replace(&mut *f.borrow_mut(), true));
    let m = compile_members(&iface.members, sigs, &name);
    IN_INTERFACE.with(|f| *f.borrow_mut() = saved);
    let m = m?;
    let methods: Vec<MethodProto> = m
        .methods
        .into_iter()
        .map(|mut mp| {
            mp.is_abstract = true;
            mp
        })
        .collect();
    let mut interfaces = interfaces;
    add_stringable(&mut interfaces, &methods, &name);
    let meta = decl_meta(
        &iface.attribute_lists,
        sigs,
        Some(&name),
        iface.name.span.start.offset,
        iface.interface.span.start.offset,
        iface.right_brace.end.offset,
    )?;
    Ok(ClassProto {
        name,
        kind: ClassKind::Interface,
        flags: ClassFlags::default(),
        parent: None,
        interfaces,
        traits: Vec::new(),
        methods,
        props: Vec::new(),
        static_props: Vec::new(),
        constants: m.constants,
        enum_cases: Vec::new(),
        enum_backing: None,
        meta,
        decl_file: decl_file(),
        decl_line: line_of(iface.interface.span.start.offset),
    })
}

fn compile_method(
    m: &rhp_frontend::cst::Method,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
    class_name: &PhpStr,
) -> Result<(MethodProto, Vec<PropProto>), Thrown> {
    let name = PhpStr::from_bytes(m.name.value.to_vec());
    let visibility = modifiers_visibility(&m.modifiers);
    let is_static = has_modifier(&m.modifiers, |md| matches!(md, Modifier::Static(_)));
    let is_abstract = has_modifier(&m.modifiers, |md| matches!(md, Modifier::Abstract(_)));
    let is_final = has_modifier(&m.modifiers, |md| matches!(md, Modifier::Final(_)));

    let mut c = ProtoBuilder::new(name.clone());
    c.sigs = sigs.clone();
    c.class_name = Some(class_name.clone());
    c.scope_kind = ScopeKind::Method;
    c.returns_ref = m.ampersand.is_some();

    // Instance methods reserve CV 0 for `$this`.
    if !is_static {
        let this = c.cv(b"this");
        debug_assert_eq!(this, 0);
    }

    // Constructor property promotion: a param with a modifier declares a property.
    let mut promoted: Vec<PropProto> = Vec::new();
    let mut promo_stores: Vec<(NIdx, Reg)> = Vec::new(); // (prop name, param cv)

    let mut params = Vec::new();
    let mut nonconst_defaults: Vec<(u16, Reg, &Expression)> = Vec::new();
    for (idx, p) in m.parameter_list.parameters.iter().enumerate() {
        let var = strip_dollar(p.variable.name);
        let cv = c.cv(var);
        let pname = c.intern_name(var);
        let default = p
            .default_value
            .as_ref()
            .map(|dv| match c.const_default(dv.value) {
                Ok(d) => d,
                Err(_) => {
                    nonconst_defaults.push((idx as u16, cv, dv.value));
                    rhp_bytecode::ParamDefault::Init
                }
            });
        let attributes = compile_attributes(&p.attribute_lists, sigs, Some(class_name))?;
        if !p.modifiers.is_empty() {
            let vis = modifiers_visibility(&p.modifiers);
            let ro = has_modifier(&p.modifiers, |md| matches!(md, Modifier::Readonly(_)));
            let (pstart, pend) = span_of(p);
            promoted.push(PropProto {
                name: PhpStr::from_bytes(var.to_vec()),
                visibility: vis,
                set_visibility: modifiers_set_visibility(&p.modifiers),
                is_readonly: ro,
                default: PropDefaultProto::None,
                type_hint: p.hint.as_ref().map(type_hint),
                meta: rhp_bytecode::DeclMeta {
                    attributes: attributes.clone(),
                    doc_comment: doc_comment_before(p.variable.span.start.offset),
                    start_line: line_of(pstart),
                    offset: pstart,
                    end_line: line_of(pend),
                },
                promoted: true,
            });
            promo_stores.push((c.intern_name(var), cv));
        }
        let dmeta = param_default_meta(p, sigs, Some(class_name));
        params.push(rhp_bytecode::ParamInfo {
            name: pname,
            cv,
            by_ref: p.ampersand.is_some(),
            variadic: p.ellipsis.is_some(),
            default,
            type_hint: p.hint.as_ref().map(type_hint),
            line: {
                use mago_span::HasSpan;
                line_of(p.span().start.offset)
            },
            attributes,
            promoted: !p.modifiers.is_empty(),
            default_thunk: dmeta.0,
            default_const: dmeta.1,
        });
    }
    let num_params = params.len() as u16;

    let empty: &[Statement] = &[];
    let body_stmts: &[_] = match &m.body {
        MethodBody::Concrete(block) => block.statements.as_slice(),
        MethodBody::Abstract(_) => empty,
    };

    for s in body_stmts {
        c.scan_stmt_vars(s);
    }
    for s in body_stmts {
        c.scan_node_vars(Node::Statement(s));
    }
    c.freeze_cvs();

    // Non-constant parameter defaults (e.g. `= self::CONST`), before promotion/body.
    c.emit_param_defaults(&nonconst_defaults)?;

    // Emit `$this->name = $name` for each promoted param, before the body.
    for (prop_name, param_cv) in &promo_stores {
        c.emit(Instr::LvalCv { cv: 0 });
        c.emit(Instr::LvalProp { name: *prop_name });
        c.emit(Instr::AssignLval { src: *param_cv });
    }

    for s in body_stmts {
        c.compile_stmt(s)?;
    }
    c.emit(Instr::RetNull);

    let mut proto = c.finish()?;
    proto.num_params = num_params;
    proto.params = params;
    proto.this_cv = if is_static { None } else { Some(0) };
    proto.is_static = is_static;
    proto.is_generator = body_stmts.iter().any(stmt_has_yield);
    proto.return_type = m.return_type_hint.as_ref().map(|r| type_hint(&r.hint));
    proto.meta = decl_meta(
        &m.attribute_lists,
        sigs,
        Some(class_name),
        m.name.span.start.offset,
        m.function.span.start.offset,
        span_of(m).1,
    )?;

    let mp = MethodProto {
        name,
        proto: Rc::new(proto),
        visibility,
        is_static,
        is_abstract,
        is_final,
    };
    Ok((mp, promoted))
}

/// Compile one `function name(params) { body }` into a `FunctionProto`.
/// Parameters occupy the first CV registers, in order.
fn compile_function(
    func: &rhp_frontend::cst::Function,
    sigs: Rc<HashMap<Vec<u8>, Vec<bool>>>,
) -> Result<FunctionProto, Thrown> {
    let fname = join_ns(&cur_ns(), func.name.value);
    let mut c = ProtoBuilder::new(PhpStr::from_bytes(fname));
    let c_sigs = sigs.clone();
    c.sigs = sigs;
    c.returns_ref = func.ampersand.is_some();

    let mut params = Vec::new();
    let mut nonconst_defaults: Vec<(u16, Reg, &Expression)> = Vec::new();
    for (idx, p) in func.parameter_list.parameters.iter().enumerate() {
        let var = strip_dollar(p.variable.name);
        let cv = c.cv(var); // params get CV 0,1,2,... in order
        let name = c.intern_name(var);
        let default = p
            .default_value
            .as_ref()
            .map(|dv| match c.const_default(dv.value) {
                Ok(d) => d,
                Err(_) => {
                    nonconst_defaults.push((idx as u16, cv, dv.value));
                    rhp_bytecode::ParamDefault::Init
                }
            });
        let dmeta = param_default_meta(p, &c.sigs.clone(), None);
        params.push(rhp_bytecode::ParamInfo {
            name,
            cv,
            by_ref: p.ampersand.is_some(),
            variadic: p.ellipsis.is_some(),
            default,
            type_hint: p.hint.as_ref().map(type_hint),
            line: {
                use mago_span::HasSpan;
                line_of(p.span().start.offset)
            },
            attributes: compile_attributes(&p.attribute_lists, &c.sigs, None)?,
            promoted: false,
            default_thunk: dmeta.0,
            default_const: dmeta.1,
        });
    }
    let num_params = params.len() as u16;

    // Scan the body for the rest of the function's local variables.
    for s in func.body.statements.iter() {
        c.scan_stmt_vars(s);
    }
    for s in func.body.statements.iter() {
        c.scan_node_vars(Node::Statement(s));
    }
    c.freeze_cvs();

    c.emit_param_defaults(&nonconst_defaults)?;

    for s in func.body.statements.iter() {
        c.compile_stmt(s)?;
    }
    c.emit(Instr::RetNull);

    let mut proto = c.finish()?;
    proto.num_params = num_params;
    proto.params = params;
    proto.is_generator = block_has_yield(&func.body.statements);
    proto.return_type = func.return_type_hint.as_ref().map(|r| type_hint(&r.hint));
    proto.meta = decl_meta(
        &func.attribute_lists,
        &c_sigs,
        None,
        func.name.span.start.offset,
        func.function.span.start.offset,
        span_of(func).1,
    )?;
    Ok(proto)
}

/// What kind of scope a proto is, for the `__FUNCTION__`/`__METHOD__` magic
/// constants (which read differently at top level vs in a function/method/closure).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Main,
    Function,
    Method,
    Closure,
    Init,
}

struct ProtoBuilder {
    name: PhpStr,
    /// Enclosing class/trait/enum name for `__CLASS__`/`__METHOD__` (None outside).
    class_name: Option<PhpStr>,
    /// `function &f()` — this proto returns by reference (affects `return`).
    returns_ref: bool,
    /// The scope discriminator for `__FUNCTION__`/`__METHOD__`.
    scope_kind: ScopeKind,
    /// For a closure, PHP 8.5's generated name `{closure:<scope>:<line>}` used by
    /// `__FUNCTION__`/`__METHOD__` inside it (the proto name stays "{closure}").
    magic_fn_name: Option<PhpStr>,
    code: Vec<Instr>,
    consts: Vec<Value>,
    names: Vec<PhpStr>,
    cv_map: Vec<Vec<u8>>, // index = Reg, value = variable name bytes
    exception_table: Vec<rhp_bytecode::ExRegion>,
    /// Per enclosing try-with-finally: positions of pending-return jump
    /// placeholders to patch to the finally entry.
    finally_rets: Vec<Vec<usize>>,
    num_cvs: u16,
    next_temp: Reg,
    high_water: Reg,
    /// Temporaries below this are protected from `reset_temps` (e.g. a `switch`
    /// subject held live across fall-through case bodies).
    temp_floor: Reg,
    /// Stack of enclosing loops/switches, for `break`/`continue` resolution.
    loops: Vec<LoopCtx>,
    /// Function name (lowercase) -> per-parameter by-ref flags (for call sites).
    sigs: Rc<HashMap<Vec<u8>, Vec<bool>>>,
    /// Current source line (set per statement); recorded into `lines` on change.
    cur_line: u32,
    /// (pc, line) pairs, appended whenever `cur_line` changes at an emit point.
    lines: Vec<(rhp_bytecode::Pc, u32)>,
    /// Lines of the enclosing call / `new` expressions (innermost last): php
    /// gives the call opcode the line of the callee name (or `new`), not the
    /// statement's, which is what getLine() and traces report.
    call_lines: Vec<u32>,
    /// Set once the CV pre-pass is done: every `$var` of the body must already
    /// have its slot (a later slot would alias a temporary).
    cvs_frozen: bool,
    /// The enclosing loop/switch and finally-body scopes (unique ids; finally
    /// bodies have the high bit set), for checking `goto` jumps.
    goto_scope: Vec<u32>,
    goto_serial: u32,
    /// `label:` name -> (pc, scope path at the label).
    labels: HashMap<Vec<u8>, (u32, Vec<u32>)>,
    /// Pending `goto`s: placeholder position, label, scope path, line.
    gotos: Vec<(usize, Vec<u8>, Vec<u32>, u32)>,
}

/// A break/continue target under construction. Jump placeholders are recorded
/// as code positions and patched once the loop's exit/continue labels are known.
#[derive(Default)]
struct LoopCtx {
    /// `finally_rets.len()` when the loop was entered: a break/continue
    /// inside more try-with-finally levels than this must run them.
    finally_depth: usize,
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
}

impl ProtoBuilder {
    fn new(name: PhpStr) -> Self {
        ProtoBuilder {
            name,
            class_name: None,
            returns_ref: false,
            scope_kind: ScopeKind::Function,
            magic_fn_name: None,
            code: Vec::new(),
            consts: Vec::new(),
            names: Vec::new(),
            cv_map: Vec::new(),
            exception_table: Vec::new(),
            finally_rets: Vec::new(),
            num_cvs: 0,
            next_temp: 0,
            high_water: 0,
            temp_floor: 0,
            loops: Vec::new(),
            sigs: Rc::new(HashMap::new()),
            cur_line: 0,
            lines: Vec::new(),
            call_lines: Vec::new(),
            cvs_frozen: false,
            goto_scope: Vec::new(),
            goto_serial: 0,
            labels: HashMap::new(),
            gotos: Vec::new(),
        }
    }

    /// Enter a loop/switch (`finally == false`) or finally-body scope.
    fn enter_goto_scope(&mut self, finally: bool) {
        self.goto_serial += 1;
        let id = self.goto_serial | if finally { 1 << 31 } else { 0 };
        self.goto_scope.push(id);
    }

    /// Bind each `goto` to its label (php's zend_resolve_goto_label plus the
    /// finally checks of pass_two): the label must exist, must not be inside a
    /// loop/switch the `goto` is not in, and the jump may not cross a finally
    /// body boundary. A jump out of a try/catch with a finally runs it first.
    fn resolve_gotos(&mut self) -> Result<(), Thrown> {
        for (pos, name, scope, line) in std::mem::take(&mut self.gotos) {
            let label = String::from_utf8_lossy(&name).into_owned();
            let Some((target, lscope)) = self.labels.get(&name) else {
                return Err(compile_error(
                    format!("'goto' to undefined label '{label}'"),
                    line,
                ));
            };
            let common = scope
                .iter()
                .zip(lscope.iter())
                .take_while(|(a, b)| a == b)
                .count();
            if lscope[common..].iter().any(|id| id & (1 << 31) != 0) {
                return Err(compile_error(
                    "jump into a finally block is disallowed",
                    line,
                ));
            }
            if scope[common..].iter().any(|id| id & (1 << 31) != 0) {
                return Err(compile_error(
                    "jump out of a finally block is disallowed",
                    line,
                ));
            }
            if common < lscope.len() {
                return Err(compile_error(
                    "'goto' into loop or switch statement is disallowed",
                    line,
                ));
            }
            let target = *target;
            self.code[pos] = match self.code[pos] {
                Instr::JmpFinally { .. } => Instr::JmpFinally { target },
                _ => Instr::Jmp { target },
            };
        }
        Ok(())
    }

    fn finish(mut self) -> Result<FunctionProto, Thrown> {
        self.resolve_gotos()?;
        let mut proto = FunctionProto::new(self.name);
        proto.num_cvs = self.num_cvs;
        proto.num_registers = self.high_water.max(self.num_cvs);
        proto.code = self.code;
        proto.consts = self.consts;
        proto.names = self.names;
        proto.exception_table = self.exception_table;
        proto.cv_var_names = self
            .cv_map
            .iter()
            .map(|n| PhpStr::from_bytes(n.clone()))
            .collect();
        proto.debug.lines = self.lines;
        proto.debug.file = SRC_FILE.with(|f| PhpStr::from_bytes(f.borrow().clone()));
        proto.superglobals = self
            .cv_map
            .iter()
            .enumerate()
            .filter(|(_, n)| is_superglobal(n))
            .map(|(i, n)| (i as Reg, PhpStr::from_bytes(n.clone())))
            .collect();
        proto.strict_types = STRICT_TYPES.with(|s| *s.borrow());
        proto.returns_ref = self.returns_ref;
        Ok(proto)
    }

    // -- register / pool management --

    fn emit(&mut self, instr: Instr) {
        let line = match (&instr, self.call_lines.last()) {
            (
                Instr::Call { .. }
                | Instr::CallV { .. }
                | Instr::CallDyn { .. }
                | Instr::CallDynV { .. }
                | Instr::New { .. }
                | Instr::NewV { .. }
                | Instr::CallMethod { .. }
                | Instr::CallMethodV { .. }
                | Instr::CallMethodDyn { .. }
                | Instr::CallStatic { .. }
                | Instr::CallStaticV { .. }
                | Instr::CallStaticDyn { .. }
                | Instr::CallMethodDynV { .. }
                | Instr::CallStaticDynV { .. },
                Some(&l),
            ) if l != 0 => l,
            _ => self.cur_line,
        };
        // Record a line entry whenever the source line changes (for debug info).
        if self.lines.last().map(|(_, l)| *l) != Some(line) {
            self.lines.push((self.code.len() as rhp_bytecode::Pc, line));
        }
        self.code.push(instr);
    }

    fn alloc_temp(&mut self) -> Reg {
        let r = self.next_temp;
        self.next_temp += 1;
        self.high_water = self.high_water.max(self.next_temp);
        r
    }

    /// Allocate `n` contiguous temporaries (for a call arg window).
    fn alloc_temps(&mut self, n: u16) -> Reg {
        let base = self.next_temp;
        self.next_temp += n;
        self.high_water = self.high_water.max(self.next_temp);
        base
    }

    /// Free temporaries back to the current floor (end of a statement).
    fn reset_temps(&mut self) {
        self.next_temp = self.temp_floor;
    }

    /// Patch a placeholder `Nop` at `pos` into a jump to `target`.
    fn patch_jmp(&mut self, pos: usize, target: u32) {
        self.code[pos] = match self.code[pos] {
            Instr::JmpFinally { .. } => Instr::JmpFinally { target },
            _ => Instr::Jmp { target },
        };
    }

    fn here(&self) -> u32 {
        self.code.len() as u32
    }

    fn cv(&mut self, var_name: &[u8]) -> Reg {
        if let Some(i) = self.cv_map.iter().position(|n| n == var_name) {
            return i as Reg;
        }
        debug_assert!(
            !self.cvs_frozen,
            "CV ${} was missed by the pre-pass (its slot would alias a temporary)",
            String::from_utf8_lossy(var_name)
        );
        let r = self.cv_map.len() as Reg;
        self.cv_map.push(var_name.to_vec());
        r
    }

    fn intern_name(&mut self, bytes: &[u8]) -> NIdx {
        if let Some(i) = self.names.iter().position(|n| n.as_bytes() == bytes) {
            return i as NIdx;
        }
        let i = self.names.len() as NIdx;
        self.names.push(PhpStr::from_bytes(bytes.to_vec()));
        i
    }

    /// Intern an optional global-fallback name, or `NO_NAME` when there is none.
    fn intern_fallback(&mut self, fb: &Option<Vec<u8>>) -> NIdx {
        match fb {
            Some(f) => self.intern_name(f),
            None => rhp_bytecode::NO_NAME,
        }
    }

    /// Resolve a first-class-callable's target function to a single name. An
    /// unqualified reference inside a namespace prefers the namespaced user
    /// function when one is declared, else the global name (builtins are global).
    fn resolve_callable_name(&self, id: &Identifier) -> Vec<u8> {
        let (primary, fallback) = resolve_name(id, NameKind::Func);
        match fallback {
            None => primary,
            Some(global) => {
                if self.sigs.contains_key(&primary.to_ascii_lowercase()) {
                    primary
                } else {
                    global
                }
            }
        }
    }

    /// Evaluate a parameter default to a constant and intern it. Only
    /// compile-time constants are supported (literals, `[]`, true/false/null,
    /// negated numeric literals); other expressions are deferred.
    fn const_default(&mut self, e: &Expression) -> Result<rhp_bytecode::ParamDefault, Thrown> {
        let v = self.const_value(e)?;
        Ok(rhp_bytecode::ParamDefault::Const(self.intern_const(v)))
    }

    /// Emit prologue code for non-constant parameter defaults (e.g.
    /// `$v = self::CONST`): for each, `JmpIfArg index -> after` skips the init when
    /// the argument was passed, else the default expression is evaluated into the
    /// param's CV (in the callee's scope, so `self::`/`static::` resolve).
    fn emit_param_defaults(&mut self, defs: &[(u16, Reg, &Expression)]) -> Result<(), Thrown> {
        for &(index, cv, expr) in defs {
            let jpos = self.code.len();
            self.emit(Instr::Nop); // JmpIfArg index -> after (patched)
            let t = self.compile_expr(expr)?;
            self.emit(Instr::Mov { dst: cv, src: t });
            let after = self.here();
            self.code[jpos] = Instr::JmpIfArg {
                index,
                target: after,
            };
            self.reset_temps();
        }
        Ok(())
    }

    fn const_value(&mut self, e: &Expression) -> Result<Value, Thrown> {
        // Parameter defaults fold with the same rules as class consts / props.
        fold_const(e)
    }

    fn intern_const(&mut self, v: Value) -> CIdx {
        if let Some(i) = self.consts.iter().position(|c| const_eq(c, &v)) {
            return i as CIdx;
        }
        let i = self.consts.len() as CIdx;
        self.consts.push(v);
        i
    }

    // -- CV pre-pass --

    fn scan_stmt_vars(&mut self, stmt: &Statement) {
        use rhp_frontend::cst::{ForBody, IfBody, SwitchBody, SwitchCase, WhileBody};
        match stmt {
            Statement::Echo(e) => e.values.iter().for_each(|v| self.scan_expr_vars(v)),
            Statement::EchoTag(e) => e.values.iter().for_each(|v| self.scan_expr_vars(v)),
            Statement::Expression(s) => self.scan_expr_vars(s.expression),
            Statement::Block(b) => b.statements.iter().for_each(|s| self.scan_stmt_vars(s)),
            Statement::If(i) => {
                self.scan_expr_vars(i.condition);
                match &i.body {
                    IfBody::Statement(b) => {
                        self.scan_stmt_vars(b.statement);
                        for c in b.else_if_clauses.iter() {
                            self.scan_expr_vars(c.condition);
                            self.scan_stmt_vars(c.statement);
                        }
                        if let Some(e) = &b.else_clause {
                            self.scan_stmt_vars(e.statement);
                        }
                    }
                    IfBody::ColonDelimited(b) => {
                        b.statements.iter().for_each(|s| self.scan_stmt_vars(s));
                        for c in b.else_if_clauses.iter() {
                            self.scan_expr_vars(c.condition);
                            c.statements.iter().for_each(|s| self.scan_stmt_vars(s));
                        }
                        if let Some(e) = &b.else_clause {
                            e.statements.iter().for_each(|s| self.scan_stmt_vars(s));
                        }
                    }
                }
            }
            Statement::While(w) => {
                self.scan_expr_vars(w.condition);
                match &w.body {
                    WhileBody::Statement(s) => self.scan_stmt_vars(s),
                    WhileBody::ColonDelimited(b) => {
                        b.statements.iter().for_each(|s| self.scan_stmt_vars(s))
                    }
                }
            }
            Statement::DoWhile(d) => {
                self.scan_stmt_vars(d.statement);
                self.scan_expr_vars(d.condition);
            }
            Statement::For(f) => {
                f.initializations
                    .iter()
                    .for_each(|e| self.scan_expr_vars(e));
                f.conditions.iter().for_each(|e| self.scan_expr_vars(e));
                f.increments.iter().for_each(|e| self.scan_expr_vars(e));
                match &f.body {
                    ForBody::Statement(s) => self.scan_stmt_vars(s),
                    ForBody::ColonDelimited(b) => {
                        b.statements.iter().for_each(|s| self.scan_stmt_vars(s))
                    }
                }
            }
            Statement::Switch(sw) => {
                self.scan_expr_vars(sw.expression);
                let cases = match &sw.body {
                    SwitchBody::BraceDelimited(b) => &b.cases,
                    SwitchBody::ColonDelimited(b) => &b.cases,
                };
                for case in cases.iter() {
                    match case {
                        SwitchCase::Expression(c) => {
                            self.scan_expr_vars(c.expression);
                            c.statements.iter().for_each(|s| self.scan_stmt_vars(s));
                        }
                        SwitchCase::Default(c) => {
                            c.statements.iter().for_each(|s| self.scan_stmt_vars(s))
                        }
                    }
                }
            }
            Statement::Return(r) => {
                if let Some(v) = r.value {
                    self.scan_expr_vars(v);
                }
            }
            Statement::Foreach(fe) => {
                self.scan_expr_vars(fe.expression);
                match &fe.target {
                    ForeachTarget::Value(t) => self.scan_expr_vars(t.value),
                    // php compiles the value variable before the key's.
                    ForeachTarget::KeyValue(t) => {
                        self.scan_expr_vars(t.value);
                        self.scan_expr_vars(t.key);
                    }
                }
                match &fe.body {
                    ForeachBody::Statement(s) => self.scan_stmt_vars(s),
                    ForeachBody::ColonDelimited(b) => {
                        b.statements.iter().for_each(|s| self.scan_stmt_vars(s))
                    }
                }
            }
            Statement::Unset(u) => u.values.iter().for_each(|v| self.scan_expr_vars(v)),
            Statement::Global(g) => {
                for v in g.variables.iter() {
                    if let Variable::Direct(d) = v {
                        self.cv(strip_dollar(d.name));
                    }
                }
            }
            Statement::Static(s) => {
                for item in s.items.iter() {
                    self.cv(strip_dollar(item.variable().name));
                }
            }
            Statement::Try(t) => {
                t.block
                    .statements
                    .iter()
                    .for_each(|s| self.scan_stmt_vars(s));
                for clause in t.catch_clauses.iter() {
                    if let Some(v) = &clause.variable {
                        self.cv(strip_dollar(v.name));
                    }
                    clause
                        .block
                        .statements
                        .iter()
                        .for_each(|s| self.scan_stmt_vars(s));
                }
                if let Some(fin) = &t.finally_clause {
                    fin.block
                        .statements
                        .iter()
                        .for_each(|s| self.scan_stmt_vars(s));
                }
            }
            _ => {}
        }
    }

    fn scan_expr_vars(&mut self, expr: &Expression) {
        match expr {
            Expression::Variable(Variable::Direct(d)) => {
                self.cv(strip_dollar(d.name));
            }
            Expression::Parenthesized(p) => self.scan_expr_vars(p.expression),
            Expression::Binary(b) => {
                self.scan_expr_vars(b.lhs);
                self.scan_expr_vars(b.rhs);
            }
            Expression::UnaryPrefix(u) => self.scan_expr_vars(u.operand),
            Expression::UnaryPostfix(u) => self.scan_expr_vars(u.operand),
            Expression::Assignment(a) => {
                self.scan_expr_vars(a.lhs);
                self.scan_expr_vars(a.rhs);
            }
            Expression::Conditional(c) => {
                self.scan_expr_vars(c.condition);
                if let Some(t) = &c.then {
                    self.scan_expr_vars(t);
                }
                self.scan_expr_vars(c.r#else);
            }
            Expression::Array(arr) => self.scan_array_vars(arr.elements.iter()),
            Expression::LegacyArray(arr) => self.scan_array_vars(arr.elements.iter()),
            Expression::List(lst) => self.scan_array_vars(lst.elements.iter()),
            Expression::ArrayAccess(aa) => {
                self.scan_expr_vars(aa.array);
                self.scan_expr_vars(aa.index);
            }
            Expression::ArrayAppend(ap) => self.scan_expr_vars(ap.array),
            Expression::CompositeString(cs) => {
                let parts = match cs {
                    CompositeString::Interpolated(i) => Some(&i.parts),
                    CompositeString::Document(d) => Some(&d.parts),
                    CompositeString::ShellExecute(_) => None,
                };
                if let Some(parts) = parts {
                    for part in parts.iter() {
                        match part {
                            StringPart::Expression(e) => self.scan_expr_vars(e),
                            StringPart::BracedExpression(b) => self.scan_expr_vars(b.expression),
                            StringPart::Literal(_) => {}
                        }
                    }
                }
            }
            Expression::Match(m) => {
                self.scan_expr_vars(m.expression);
                for arm in m.arms.iter() {
                    match arm {
                        rhp_frontend::cst::MatchArm::Expression(a) => {
                            a.conditions.iter().for_each(|c| self.scan_expr_vars(c));
                            self.scan_expr_vars(a.expression);
                        }
                        rhp_frontend::cst::MatchArm::Default(a) => {
                            self.scan_expr_vars(a.expression)
                        }
                    }
                }
            }
            Expression::Instantiation(inst) => {
                self.scan_expr_vars(inst.class);
                if let Some(list) = &inst.argument_list {
                    self.scan_args(list);
                }
            }
            Expression::Access(acc) => match acc {
                Access::Property(pa) => self.scan_expr_vars(pa.object),
                Access::NullSafeProperty(pa) => self.scan_expr_vars(pa.object),
                // Note: in `C::$x` the `$x` is the static-property NAME, not a local.
                Access::StaticProperty(spa) => self.scan_expr_vars(spa.class),
                Access::ClassConstant(cca) => self.scan_expr_vars(cca.class),
            },
            Expression::Clone(cl) => self.scan_expr_vars(cl.object),
            Expression::Throw(t) => self.scan_expr_vars(t.exception),
            Expression::Construct(c) => self.scan_construct_vars(c),
            Expression::Call(call) => {
                use rhp_frontend::cst::Call;
                match call {
                    Call::Function(fc) => self.scan_args(&fc.argument_list),
                    Call::Method(mc) => {
                        self.scan_expr_vars(mc.object);
                        self.scan_args(&mc.argument_list);
                    }
                    Call::NullSafeMethod(mc) => {
                        self.scan_expr_vars(mc.object);
                        self.scan_args(&mc.argument_list);
                    }
                    Call::StaticMethod(smc) => {
                        self.scan_expr_vars(smc.class);
                        self.scan_args(&smc.argument_list);
                    }
                }
            }
            _ => {}
        }
    }

    fn scan_args(&mut self, list: &rhp_frontend::cst::ArgumentList) {
        for arg in list.arguments.iter() {
            match arg {
                Argument::Positional(p) => self.scan_expr_vars(p.value),
                Argument::Named(n) => self.scan_expr_vars(n.value),
            }
        }
    }

    fn scan_array_vars<'a>(&mut self, els: impl Iterator<Item = &'a ArrayElement<'a>>) {
        for el in els {
            match el {
                ArrayElement::KeyValue(kv) => {
                    self.scan_expr_vars(kv.key);
                    self.scan_expr_vars(kv.value);
                }
                ArrayElement::Value(v) => self.scan_expr_vars(v.value),
                ArrayElement::Variadic(v) => self.scan_expr_vars(v.value),
                ArrayElement::Missing(_) => {}
            }
        }
    }

    fn scan_construct_vars(&mut self, c: &Construct) {
        match c {
            Construct::Print(p) => self.scan_expr_vars(p.value),
            Construct::Empty(e) => self.scan_expr_vars(e.value),
            Construct::Isset(i) => i.values.iter().for_each(|v| self.scan_expr_vars(v)),
            Construct::Include(i) => self.scan_expr_vars(i.value),
            Construct::IncludeOnce(i) => self.scan_expr_vars(i.value),
            Construct::Require(i) => self.scan_expr_vars(i.value),
            Construct::RequireOnce(i) => self.scan_expr_vars(i.value),
            Construct::Eval(e) => self.scan_expr_vars(e.value),
            Construct::Exit(e) => {
                if let Some(list) = &e.arguments {
                    self.scan_args(list);
                }
            }
            Construct::Die(d) => {
                if let Some(list) = &d.arguments {
                    self.scan_args(list);
                }
            }
        }
    }

    /// Completeness backstop for the CV pre-pass: walk every node of the body
    /// (not entering nested function/class scopes, whose variables are their
    /// own) and give each `$name` still missing a slot. A closure contributes
    /// its `use` variables; an arrow function nothing (it captures only
    /// variables the enclosing scope already has).
    fn scan_node_vars(&mut self, node: Node) {
        match node {
            Node::DirectVariable(d) => {
                self.cv(strip_dollar(d.name));
                return;
            }
            Node::Closure(cl) => {
                if let Some(u) = &cl.use_clause {
                    for v in u.variables.iter() {
                        self.cv(strip_dollar(v.variable.name));
                    }
                }
                return;
            }
            Node::AnonymousClass(ac) => {
                if let Some(args) = &ac.argument_list {
                    self.scan_node_vars(Node::PartialArgumentList(args));
                }
                return;
            }
            // `C::$p`: the `$p` is the property name, not a local.
            Node::StaticPropertyAccess(spa) => {
                self.scan_node_vars(Node::Expression(spa.class));
                return;
            }
            Node::ArrowFunction(_)
            | Node::Function(_)
            | Node::Class(_)
            | Node::Interface(_)
            | Node::Trait(_)
            | Node::Enum(_)
            | Node::AttributeList(_) => return,
            _ => {}
        }
        node.visit_children(|ch| self.scan_node_vars(ch));
    }

    /// End the CV pre-pass: CVs occupy `[0, num_cvs)`, temporaries follow.
    fn freeze_cvs(&mut self) {
        self.num_cvs = self.cv_map.len() as u16;
        self.next_temp = self.num_cvs;
        self.high_water = self.num_cvs;
        self.temp_floor = self.num_cvs;
        self.cvs_frozen = true;
    }

    // -- statement lowering --

    /// Record a nested class declaration and emit its `DeclareClass`.
    fn emit_cond_class(&mut self, proto: ClassProto) {
        self.cur_line = proto.decl_line;
        let class_idx = COND_CLASSES.with(|c| {
            let mut c = c.borrow_mut();
            c.0.push(proto);
            c.1 + c.0.len() as u32 - 1
        });
        self.emit(Instr::DeclareClass { class_idx });
    }

    fn compile_stmt(&mut self, stmt: &Statement) -> Result<(), Thrown> {
        use mago_span::HasSpan;
        self.compile_stmt_inner(stmt)
            .map_err(|e| locate(e, stmt.span().start.offset))
    }

    fn compile_stmt_inner(&mut self, stmt: &Statement) -> Result<(), Thrown> {
        use mago_span::HasSpan;
        self.cur_line = line_of(stmt.span().start.offset);
        match stmt {
            Statement::OpeningTag(_) | Statement::ClosingTag(_) | Statement::Noop(_) => {}
            Statement::Inline(inline) => {
                if inline.kind.is_text() {
                    let text = strip_close_tag_newline(inline.span().start.offset, inline.value);
                    // php's `?>` token includes the newline: the HTML after
                    // it starts on the next line.
                    let skipped = inline.value.len() - text.len();
                    if skipped > 0 {
                        self.cur_line = line_of(inline.span().start.offset + skipped as u32);
                    }
                    if !text.is_empty() {
                        let k = self.intern_const(Value::str(text.to_vec()));
                        self.emit(Instr::EchoConst { k });
                    }
                }
            }
            Statement::Echo(e) => {
                for v in e.values.iter() {
                    let r = self.compile_expr(v)?;
                    self.emit(Instr::Echo { src: r });
                }
            }
            Statement::EchoTag(e) => {
                for v in e.values.iter() {
                    let r = self.compile_expr(v)?;
                    self.emit(Instr::Echo { src: r });
                }
            }
            Statement::Expression(s) => {
                // Track the temporaries this statement touches so they can be
                // released at its end (php frees an expression's temporaries,
                // running destructors, before the next statement).
                let from = self.next_temp;
                let saved_hw = self.high_water;
                self.high_water = self.next_temp;
                self.compile_expr(s.expression)?;
                let to = self.high_water;
                self.high_water = saved_hw.max(to);
                if to > from {
                    self.emit(Instr::ClearTemps { from, to });
                }
            }
            Statement::Block(b) => {
                for s in b.statements.iter() {
                    self.compile_stmt(s)?;
                }
            }
            Statement::HaltCompiler(_) => self.emit(Instr::Halt),
            // A function reached here is NOT a hoisted file-top-level declaration
            // (the {main} loop skips those) — it is nested in control flow or a
            // function body, so declare it at runtime (php conditional functions).
            Statement::Function(f) => {
                let proto = compile_function(f, self.sigs.clone())?;
                let idx = CLOSURES.with(|c| {
                    let mut v = c.borrow_mut();
                    let i = v.len() as u32;
                    v.push(Rc::new(proto));
                    i
                });
                self.emit(Instr::DeclareFunc { idx });
            }
            // A class-like reached here is nested in control flow or a function
            // body: declared at run time when execution reaches it.
            Statement::Class(cl) => {
                let proto = compile_class(cl, &self.sigs)?;
                self.emit_cond_class(proto);
            }
            Statement::Interface(i) => {
                let proto = compile_interface(i, &self.sigs)?;
                self.emit_cond_class(proto);
            }
            Statement::Trait(t) => {
                let proto = compile_trait(t, &self.sigs)?;
                self.emit_cond_class(proto);
            }
            Statement::Enum(e) => {
                let proto = compile_enum(e, &self.sigs)?;
                self.emit_cond_class(proto);
            }
            Statement::Constant(c) => {
                for item in c.items.iter() {
                    // zend_compile_const_decl: true/false/null in any case.
                    let short: &[u8] = item.name.value;
                    if [&b"true"[..], b"false", b"null"]
                        .iter()
                        .any(|s| short.eq_ignore_ascii_case(s))
                    {
                        let msg = format!(
                            "Cannot redeclare constant '{}'",
                            String::from_utf8_lossy(short)
                        );
                        compile_fatal(stmt.span().start.offset, msg);
                    }
                    let src = self.compile_expr(item.value)?;
                    let name = self.intern_name(&join_ns(&cur_ns(), item.name.value));
                    self.emit(Instr::DeclareConst { name, src });
                }
            }
            Statement::If(i) => self.compile_if(i)?,
            Statement::While(w) => self.compile_while(w)?,
            Statement::DoWhile(d) => self.compile_do_while(d)?,
            Statement::For(f) => self.compile_for(f)?,
            Statement::Switch(s) => self.compile_switch(s)?,
            Statement::Foreach(fe) => self.compile_foreach(fe)?,
            Statement::Try(t) => self.compile_try(t)?,
            Statement::Global(g) => {
                for v in g.variables.iter() {
                    if let Variable::Direct(d) = v {
                        let var = strip_dollar(d.name);
                        let cv = self.cv(var);
                        let name = self.intern_name(var);
                        self.emit(Instr::BindGlobal { cv, name });
                    } else {
                        let name = self.var_name_reg(v)?;
                        self.emit(Instr::BindGlobalDyn { name });
                    }
                }
            }
            Statement::Static(s) => {
                for item in s.items.iter() {
                    use rhp_frontend::cst::StaticItem;
                    let (dvar, init) = match item {
                        StaticItem::Abstract(a) => (&a.variable, u32::MAX),
                        StaticItem::Concrete(c) => match fold_const(c.value) {
                            Ok(v) => (&c.variable, self.intern_const(v)),
                            Err(_) => {
                                // php 8.3+: an arbitrary initializer, evaluated
                                // once on the first execution of the statement.
                                let var = strip_dollar(c.variable.name);
                                let cv = self.cv(var);
                                let name = self.intern_name(var);
                                let jpos = self.code.len();
                                self.emit(Instr::Nop); // BindStaticOrJmp (patched)
                                let src = self.compile_expr(c.value)?;
                                self.emit(Instr::BindStaticInit { cv, name, src });
                                let after = self.here();
                                self.code[jpos] = Instr::BindStaticOrJmp {
                                    cv,
                                    name,
                                    target: after,
                                };
                                continue;
                            }
                        },
                    };
                    let var = strip_dollar(dvar.name);
                    let cv = self.cv(var);
                    let name = self.intern_name(var);
                    self.emit(Instr::BindStatic { cv, name, init });
                }
            }
            Statement::Unset(u) => self.compile_unset(u)?,
            Statement::Return(r) => {
                if self.finally_rets.is_empty() {
                    match r.value {
                        // By-ref function: `return $lval` yields a reference.
                        Some(e) if self.returns_ref => {
                            let src = self.compile_ref_source(e)?;
                            self.emit(Instr::RetRef { src });
                        }
                        Some(e) => {
                            let src = self.compile_expr(e)?;
                            self.emit(Instr::Ret { src });
                        }
                        None => self.emit(Instr::RetNull),
                    }
                } else {
                    // Route the return through the innermost enclosing finally.
                    let src = match r.value {
                        Some(e) => self.compile_expr(e)?,
                        None => {
                            let t = self.alloc_temp();
                            self.emit(Instr::LoadNull { dst: t });
                            t
                        }
                    };
                    self.emit(Instr::SetPendingRet { src });
                    let pos = self.code.len();
                    self.emit(Instr::Nop); // Jmp -> finally (patched)
                    self.finally_rets.last_mut().unwrap().push(pos);
                }
            }
            Statement::Break(b) => self.compile_break_continue(b.level, true)?,
            Statement::Continue(c) => self.compile_break_continue(c.level, false)?,
            // `declare(strict_types=1|ticks=…|encoding=…)` — the directives are
            // no-ops for this engine; compile the body (usually just `;`).
            Statement::Declare(d) => {
                use rhp_frontend::cst::DeclareBody;
                match &d.body {
                    DeclareBody::Statement(s) => self.compile_stmt(s)?,
                    DeclareBody::ColonDelimited(c) => {
                        for s in c.statements.iter() {
                            self.compile_stmt(s)?;
                        }
                    }
                }
            }
            // A label marks a jump target; duplicates are php's compile error.
            Statement::Label(l) => {
                let name = l.name.value.to_vec();
                if self.labels.contains_key(&name) {
                    return Err(compile_error(
                        format!("Label '{}' already defined", String::from_utf8_lossy(&name)),
                        self.cur_line,
                    ));
                }
                let here = self.here();
                self.labels.insert(name, (here, self.goto_scope.clone()));
            }
            // Bound to its label when the function is finished. Inside a
            // try/catch with a finally, the jump runs the finally blocks it
            // leaves (JmpFinally leaves any that contain the target alone).
            Statement::Goto(g) => {
                let pos = self.code.len();
                if self.finally_rets.is_empty() {
                    self.emit(Instr::Nop);
                } else {
                    self.emit(Instr::JmpFinally { target: 0 });
                }
                self.gotos.push((
                    pos,
                    g.label.value.to_vec(),
                    self.goto_scope.clone(),
                    self.cur_line,
                ));
            }
            other => {
                return Err(Thrown::unsupported(
                    format!("{} statement", stmt_kind(other)),
                    "not implemented",
                ))
            }
        }
        self.reset_temps();
        Ok(())
    }

    // -- control-flow lowering --

    /// Compile a single statement or a colon-delimited statement sequence.
    fn compile_stmts<'a>(
        &mut self,
        stmts: impl Iterator<Item = &'a Statement<'a>>,
    ) -> Result<(), Thrown> {
        for s in stmts {
            self.compile_stmt(s)?;
        }
        Ok(())
    }

    fn compile_if(&mut self, i: &rhp_frontend::cst::If) -> Result<(), Thrown> {
        // Jumps to the end of the whole if/elseif/else chain, patched at the end.
        let mut end_jumps = Vec::new();

        // Emit one (condition, body) clause: returns nothing; leaves a JmpFalse
        // targeting the next clause's start.
        macro_rules! clause {
            ($cond:expr, $body:block) => {{
                let cond = self.compile_expr($cond)?;
                let jf = self.code.len();
                self.emit(Instr::Nop); // JmpFalse cond -> next clause
                $body
                end_jumps.push(self.code.len());
                self.emit(Instr::Nop); // Jmp -> end
                let next = self.here();
                self.code[jf] = Instr::JmpFalse { cond, target: next };
            }};
        }

        match &i.body {
            IfBody::Statement(b) => {
                clause!(i.condition, {
                    self.compile_stmt(b.statement)?;
                });
                for c in b.else_if_clauses.iter() {
                    clause!(c.condition, {
                        self.compile_stmt(c.statement)?;
                    });
                }
                if let Some(e) = &b.else_clause {
                    self.compile_stmt(e.statement)?;
                }
            }
            IfBody::ColonDelimited(b) => {
                clause!(i.condition, {
                    self.compile_stmts(b.statements.iter())?;
                });
                for c in b.else_if_clauses.iter() {
                    clause!(c.condition, {
                        self.compile_stmts(c.statements.iter())?;
                    });
                }
                if let Some(e) = &b.else_clause {
                    self.compile_stmts(e.statements.iter())?;
                }
            }
        }

        let end = self.here();
        for j in end_jumps {
            self.patch_jmp(j, end);
        }
        Ok(())
    }

    /// php compiles `while` and `for` with a jump to the condition at the
    /// bottom, and a jump checks for interrupts (a pending out-of-memory
    /// error is raised before the first iteration). The condition stays at
    /// the top here, behind a jump to it.
    fn emit_loop_entry(&mut self) {
        let next = self.here() + 1;
        self.emit(Instr::Jmp { target: next });
    }

    fn compile_while(&mut self, w: &rhp_frontend::cst::While) -> Result<(), Thrown> {
        self.emit_loop_entry();
        let top = self.here();
        let cond = self.compile_expr(w.condition)?;
        let jf = self.code.len();
        self.emit(Instr::Nop); // JmpFalse cond -> end
        self.loops.push(LoopCtx {
            finally_depth: self.finally_rets.len(),
            ..LoopCtx::default()
        });
        self.enter_goto_scope(false);
        match &w.body {
            WhileBody::Statement(s) => self.compile_stmt(s)?,
            WhileBody::ColonDelimited(b) => self.compile_stmts(b.statements.iter())?,
        }
        let ctx = self.loops.pop().unwrap();
        self.goto_scope.pop();
        self.emit(Instr::Jmp { target: top });
        let end = self.here();
        self.code[jf] = Instr::JmpFalse { cond, target: end };
        self.patch_all(ctx.break_jumps, end);
        self.patch_all(ctx.continue_jumps, top);
        Ok(())
    }

    fn compile_do_while(&mut self, d: &rhp_frontend::cst::DoWhile) -> Result<(), Thrown> {
        let top = self.here();
        self.loops.push(LoopCtx {
            finally_depth: self.finally_rets.len(),
            ..LoopCtx::default()
        });
        self.enter_goto_scope(false);
        self.compile_stmt(d.statement)?;
        let ctx = self.loops.pop().unwrap();
        self.goto_scope.pop();
        let cont = self.here(); // continue -> re-test the condition
        let cond = self.compile_expr(d.condition)?;
        self.emit(Instr::JmpTrue { cond, target: top });
        let end = self.here();
        self.patch_all(ctx.break_jumps, end);
        self.patch_all(ctx.continue_jumps, cont);
        Ok(())
    }

    fn compile_for(&mut self, f: &rhp_frontend::cst::For) -> Result<(), Thrown> {
        for e in f.initializations.iter() {
            self.compile_expr(e)?;
        }
        self.emit_loop_entry();
        let top = self.here();
        // All conditions evaluated for side effects; the LAST controls the loop.
        let jf = if f.conditions.is_empty() {
            None
        } else {
            let mut last = None;
            for e in f.conditions.iter() {
                last = Some(self.compile_expr(e)?);
            }
            let pos = self.code.len();
            self.emit(Instr::Nop); // JmpFalse last -> end
            Some((last.unwrap(), pos))
        };
        self.loops.push(LoopCtx {
            finally_depth: self.finally_rets.len(),
            ..LoopCtx::default()
        });
        self.enter_goto_scope(false);
        match &f.body {
            ForBody::Statement(s) => self.compile_stmt(s)?,
            ForBody::ColonDelimited(b) => self.compile_stmts(b.statements.iter())?,
        }
        let ctx = self.loops.pop().unwrap();
        self.goto_scope.pop();
        let cont = self.here(); // continue -> increments
        for e in f.increments.iter() {
            self.compile_expr(e)?;
        }
        self.emit(Instr::Jmp { target: top });
        let end = self.here();
        if let Some((cond, pos)) = jf {
            self.code[pos] = Instr::JmpFalse { cond, target: end };
        }
        self.patch_all(ctx.break_jumps, end);
        self.patch_all(ctx.continue_jumps, cont);
        Ok(())
    }

    fn compile_switch(&mut self, sw: &rhp_frontend::cst::Switch) -> Result<(), Thrown> {
        let subject = self.compile_expr(sw.expression)?;
        // Protect the subject register across the fall-through case bodies.
        let saved_floor = self.temp_floor;
        self.temp_floor = self.next_temp;

        let cases: Vec<&SwitchCase> = match &sw.body {
            SwitchBody::BraceDelimited(b) => b.cases.iter().collect(),
            SwitchBody::ColonDelimited(b) => b.cases.iter().collect(),
        };

        // Dispatch: compare the subject against each case (loose ==), in order.
        let mut dispatch: Vec<(usize, usize, Reg)> = Vec::new(); // (case idx, jmp pos, cond reg)
        for (idx, case) in cases.iter().enumerate() {
            if let SwitchCase::Expression(c) = case {
                let cexpr = self.compile_expr(c.expression)?;
                let t = self.alloc_temp();
                self.emit(Instr::Cmp {
                    dst: t,
                    a: subject,
                    b: cexpr,
                    kind: CmpKind::Eq,
                });
                let pos = self.code.len();
                self.emit(Instr::Nop); // JmpTrue t -> case body
                dispatch.push((idx, pos, t));
            }
        }
        let default_idx = cases
            .iter()
            .position(|c| matches!(c, SwitchCase::Default(_)));
        let no_match_pos = self.code.len();
        self.emit(Instr::Nop); // Jmp -> default body or end

        // Case bodies in source order (fall-through: no jumps between them).
        self.loops.push(LoopCtx {
            finally_depth: self.finally_rets.len(),
            ..LoopCtx::default()
        }); // switch is one break/continue level
        self.enter_goto_scope(false);
        let mut starts = vec![0u32; cases.len()];
        for (idx, case) in cases.iter().enumerate() {
            starts[idx] = self.here();
            match case {
                SwitchCase::Expression(c) => self.compile_stmts(c.statements.iter())?,
                SwitchCase::Default(c) => self.compile_stmts(c.statements.iter())?,
            }
        }
        let ctx = self.loops.pop().unwrap();
        self.goto_scope.pop();
        let end = self.here();

        for (idx, pos, cond) in dispatch {
            self.code[pos] = Instr::JmpTrue {
                cond,
                target: starts[idx],
            };
        }
        let no_match_target = default_idx.map(|i| starts[i]).unwrap_or(end);
        self.patch_jmp(no_match_pos, no_match_target);
        // In a switch, continue (level 1) behaves like break.
        self.patch_all(ctx.break_jumps, end);
        self.patch_all(ctx.continue_jumps, end);

        self.temp_floor = saved_floor;
        Ok(())
    }

    fn compile_foreach(&mut self, fe: &rhp_frontend::cst::Foreach) -> Result<(), Thrown> {
        // Resolve key + value targets. A value target may be a destructuring
        // pattern, which binds into a protected element register per iteration.
        // php compiles the iterated expression, then the value variable, then
        // the key variable: their CVs (so the global symbol table's and
        // get_defined_vars()'s order) are created in that order.
        let (key_expr, val_expr) = match &fe.target {
            ForeachTarget::Value(t) => (None, t.value),
            ForeachTarget::KeyValue(t) => (Some(t.key), t.value),
        };

        // By-reference foreach: `foreach ($a as &$v)` / `... as $k => &$v`.
        let by_ref = matches!(
            val_expr,
            Expression::UnaryPrefix(u) if matches!(u.operator, UnaryPrefixOperator::Reference(_))
        );

        if by_ref {
            let Expression::UnaryPrefix(u) = val_expr else {
                unreachable!()
            };
            let Expression::Variable(Variable::Direct(d)) = u.operand else {
                return Err(Thrown::unsupported(
                    "foreach by-reference target",
                    "only a simple `&$v` loop variable is supported",
                ));
            };
            // The source must be assignable so its elements can be referenced: a
            // simple variable uses its slot directly; a property / array-element /
            // static-property is promoted to a reference held in a temp (the VM's
            // `FeFetchRef` derefs an `arr_cv` that holds a `Ref`).
            let arr_cv = match fe.expression {
                Expression::Variable(Variable::Direct(sd)) => self.cv(strip_dollar(sd.name)),
                Expression::ArrayAccess(_)
                | Expression::ArrayAppend(_)
                | Expression::Access(Access::Property(_))
                | Expression::Access(Access::StaticProperty(_)) => {
                    let mut steps = Vec::new();
                    let root = self.collect_lvalue_steps(fe.expression, &mut steps)?;
                    self.emit_cursor(root, &steps);
                    let r = self.alloc_temp();
                    self.emit(Instr::MakeRefLval { dst: r });
                    r
                }
                // Any other expression (a call, `new`, a literal …): php
                // iterates the temporary itself (FE_RESET_RW on a TMP) — a
                // by-ref generator / its elements are referenced in place.
                _ => self.compile_expr(fe.expression)?,
            };
            let val_cv = self.cv(strip_dollar(d.name));
            let key_cv = match key_expr {
                Some(k) => self.foreach_target_cv(k)?,
                None => Reg::MAX,
            };
            let pos_reg = self.alloc_temp();
            let saved_floor = self.temp_floor;
            self.temp_floor = self.next_temp;

            let reset_pos = self.code.len();
            self.emit(Instr::Nop); // FeResetRef
            let fetch_label = self.here();
            let fetch_pos = self.code.len();
            self.emit(Instr::Nop); // FeFetchRef

            self.loops.push(LoopCtx {
                finally_depth: self.finally_rets.len(),
                ..LoopCtx::default()
            });

            self.enter_goto_scope(false);
            match &fe.body {
                ForeachBody::Statement(s) => self.compile_stmt(s)?,
                ForeachBody::ColonDelimited(b) => self.compile_stmts(b.statements.iter())?,
            }
            let ctx = self.loops.pop().unwrap();
            self.goto_scope.pop();
            self.emit(Instr::Jmp {
                target: fetch_label,
            });
            let end = self.here();
            self.code[reset_pos] = Instr::FeResetRef {
                arr_cv,
                pos_reg,
                end,
            };
            self.code[fetch_pos] = Instr::FeFetchRef {
                arr_cv,
                pos_reg,
                key_cv,
                val_cv,
                end,
            };
            self.patch_all(ctx.break_jumps, end);
            self.patch_all(ctx.continue_jumps, fetch_label);
            self.temp_floor = saved_floor;
            return Ok(());
        }

        let src = self.compile_expr(fe.expression)?;
        let arr_reg = self.alloc_temp();
        let pos_reg = self.alloc_temp();

        let (val_cv, val_pattern): (Reg, Option<&Expression>) = match val_expr {
            Expression::Variable(Variable::Direct(d)) => (self.cv(strip_dollar(d.name)), None),
            // Destructuring, `$$n`, `$a[k]`, `$o->p`: fetch into a temporary
            // and store it like a list() element.
            _ => (self.alloc_temp(), Some(val_expr)),
        };
        // The key variable is created after the value variable, as php does
        // (observable in the order of `$GLOBALS`).
        let key_cv = match key_expr {
            Some(k) => self.foreach_target_cv(k)?,
            None => Reg::MAX,
        };
        // php stores the value target before assigning the key, so a key
        // variable used inside the target (`$k => $$k`) is the previous one.
        let (fetch_key, key_after) = if val_pattern.is_some() && key_cv != Reg::MAX {
            (self.alloc_temp(), Some(key_cv))
        } else {
            (key_cv, None)
        };

        let saved_floor = self.temp_floor;
        self.temp_floor = self.next_temp; // protect iterator + element regs across the body

        let reset_pos = self.code.len();
        self.emit(Instr::Nop); // FeReset (needs end)
        let fetch_label = self.here();
        let fetch_pos = self.code.len();
        self.emit(Instr::Nop); // FeFetch (needs end)

        // Destructure the fetched element into the pattern, if any.
        if let Some(pattern) = val_pattern {
            match pattern {
                Expression::Array(arr) => self.destructure_into(arr.elements.iter(), val_cv)?,
                Expression::List(lst) => self.destructure_into(lst.elements.iter(), val_cv)?,
                other => self.store_to_target(other, val_cv)?,
            }
        }
        if let Some(cv) = key_after {
            self.emit(Instr::Assign { cv, src: fetch_key });
        }

        self.loops.push(LoopCtx {
            finally_depth: self.finally_rets.len(),
            ..LoopCtx::default()
        });

        self.enter_goto_scope(false);
        match &fe.body {
            ForeachBody::Statement(s) => self.compile_stmt(s)?,
            ForeachBody::ColonDelimited(b) => self.compile_stmts(b.statements.iter())?,
        }
        let ctx = self.loops.pop().unwrap();
        self.goto_scope.pop();
        self.emit(Instr::Jmp {
            target: fetch_label,
        });
        let end = self.here();
        self.code[reset_pos] = Instr::FeReset {
            arr_reg,
            pos_reg,
            src,
            end,
            delegate: false,
        };
        self.code[fetch_pos] = Instr::FeFetch {
            arr_reg,
            pos_reg,
            key_cv: fetch_key,
            val_cv,
            end,
        };
        self.patch_all(ctx.break_jumps, end);
        self.patch_all(ctx.continue_jumps, fetch_label);
        self.temp_floor = saved_floor;
        Ok(())
    }

    fn foreach_target_cv(&mut self, e: &Expression) -> Result<Reg, Thrown> {
        match e {
            Expression::Variable(Variable::Direct(d)) => Ok(self.cv(strip_dollar(d.name))),
            Expression::UnaryPrefix(u)
                if matches!(u.operator, UnaryPrefixOperator::Reference(_)) =>
            {
                Err(Thrown::unsupported(
                    "foreach by reference",
                    "references arrive in the next phase",
                ))
            }
            _ => Err(Thrown::unsupported(
                "foreach target",
                "only simple `$var` targets for now (no list()/destructuring)",
            )),
        }
    }

    fn compile_unset(&mut self, u: &rhp_frontend::cst::Unset) -> Result<(), Thrown> {
        for v in u.values.iter() {
            check_globals_write(v)?;
            match v {
                Expression::Variable(Variable::Direct(d)) => {
                    // A CV can't be removed; null is observably equivalent to
                    // unset for isset/??/empty.
                    let cv = self.cv(strip_dollar(d.name));
                    self.emit(Instr::LoadNull { dst: cv });
                }
                Expression::Variable(var) => {
                    let name = self.var_name_reg(var)?;
                    self.emit(Instr::UnsetVarVar { name });
                }
                Expression::ArrayAccess(_) if globals_index(v).is_some() => {
                    let name = self.compile_expr(globals_index(v).unwrap())?;
                    self.emit(Instr::UnsetGlobal { name });
                }
                Expression::ArrayAccess(_) => {
                    let mut steps = Vec::new();
                    let root = self.collect_lvalue_steps(v, &mut steps)?;
                    self.emit_cursor(root, &steps);
                    self.emit(Instr::UnsetLval);
                }
                // `unset($o->prop)`: remove a declared property, else `__unset`.
                Expression::Access(Access::Property(pa)) => {
                    let obj = self.compile_expr(pa.object)?;
                    match self.compile_selector(&pa.property)? {
                        Sel::Static(name) => self.emit(Instr::UnsetProp { obj, name }),
                        Sel::Dyn(name) => self.emit(Instr::UnsetPropDyn { obj, name }),
                    }
                }
                _ => {
                    return Err(Thrown::unsupported(
                        "unset target",
                        "only variables, array elements, and properties",
                    ))
                }
            }
        }
        Ok(())
    }

    fn compile_try(&mut self, t: &rhp_frontend::cst::Try) -> Result<(), Thrown> {
        use rhp_bytecode::{CatchArm, ExRegion};
        let has_finally = t.finally_clause.is_some();
        if has_finally {
            self.finally_rets.push(Vec::new());
        }
        let try_start = self.here();
        self.compile_stmts(t.block.statements.iter())?;
        let try_end = self.here();
        // Normal completion jumps past the catch bodies to finally/after.
        let norm_jmp = self.code.len();
        self.emit(Instr::Nop);

        let mut catch_arms = Vec::new();
        let mut end_jumps = vec![norm_jmp];
        for clause in t.catch_clauses.iter() {
            let target = self.here();
            let classes = hint_class_names(&clause.hint)?;
            let var_reg = clause
                .variable
                .as_ref()
                .map(|v| self.cv(strip_dollar(v.name)));
            self.compile_stmts(clause.block.statements.iter())?;
            let j = self.code.len();
            self.emit(Instr::Nop);
            end_jumps.push(j);
            catch_arms.push(CatchArm {
                classes,
                var_reg,
                target,
            });
        }

        let catches_end = self.here();
        let mut finally_end = 0;
        let finally_pc = if let Some(fin) = &t.finally_clause {
            // Stop routing returns into THIS finally while compiling its body.
            let pending_rets = self.finally_rets.pop().unwrap();
            let fpc = self.here();
            self.enter_goto_scope(true);
            self.compile_stmts(fin.block.statements.iter())?;
            self.goto_scope.pop();
            finally_end = self.here();
            self.emit(Instr::FinallyEnd);
            // Patch `return`s that crossed out of try/catch to enter the finally.
            for pos in pending_rets {
                self.code[pos] = Instr::Jmp { target: fpc };
            }
            Some(fpc)
        } else {
            None
        };

        let after = self.here();
        let norm_target = finally_pc.unwrap_or(after);
        for j in end_jumps {
            self.patch_jmp(j, norm_target);
        }

        let has_catches = !catch_arms.is_empty();
        self.exception_table.push(ExRegion {
            start: try_start,
            end: try_end,
            catches: catch_arms,
            finally: finally_pc,
            finally_end,
        });
        // An exception escaping a catch body still runs the finally.
        if finally_pc.is_some() && has_catches {
            self.exception_table.push(ExRegion {
                start: try_end,
                end: catches_end,
                catches: Vec::new(),
                finally: finally_pc,
                finally_end,
            });
        }
        Ok(())
    }

    /// Assign the elements of `src` (an array value) to a destructuring pattern.
    fn destructure_into<'a>(
        &mut self,
        elements: impl Iterator<Item = &'a ArrayElement<'a>>,
        src: Reg,
    ) -> Result<(), Thrown> {
        let mut idx: i64 = 0;
        for el in elements {
            match el {
                ArrayElement::Missing(_) => idx += 1,
                ArrayElement::Value(v) => {
                    let key = self.alloc_temp();
                    self.emit(Instr::LoadInt { dst: key, n: idx });
                    let val = self.alloc_temp();
                    self.emit(Instr::DimFetchList {
                        dst: val,
                        arr: src,
                        key,
                    });
                    self.store_to_target(v.value, val)?;
                    idx += 1;
                }
                ArrayElement::KeyValue(kv) => {
                    let key = self.compile_expr(kv.key)?;
                    let val = self.alloc_temp();
                    self.emit(Instr::DimFetchList {
                        dst: val,
                        arr: src,
                        key,
                    });
                    self.store_to_target(kv.value, val)?;
                }
                ArrayElement::Variadic(_) => {
                    return Err(Thrown::unsupported("spread in destructuring", "deferred"))
                }
            }
        }
        Ok(())
    }

    /// Store `src` into a single destructuring target (variable, nested pattern,
    /// array element, or property).
    fn store_to_target(&mut self, target: &Expression, src: Reg) -> Result<(), Thrown> {
        check_globals_write(target)?;
        match target {
            Expression::Variable(Variable::Direct(d)) => {
                let cv = self.cv(strip_dollar(d.name));
                self.emit(Instr::Assign { cv, src });
            }
            Expression::Array(arr) => self.destructure_into(arr.elements.iter(), src)?,
            Expression::List(lst) => self.destructure_into(lst.elements.iter(), src)?,
            Expression::ArrayAccess(_)
            | Expression::ArrayAppend(_)
            | Expression::Variable(_)
            | Expression::Access(Access::Property(_))
            | Expression::Access(Access::StaticProperty(_)) => {
                let mut steps = Vec::new();
                let root = self.collect_lvalue_steps(target, &mut steps)?;
                self.emit_cursor(root, &steps);
                self.emit(Instr::AssignLval { src });
            }
            _ => return Err(Thrown::unsupported("destructuring target", "deferred")),
        }
        Ok(())
    }

    fn compile_break_continue(
        &mut self,
        level: Option<&Expression>,
        is_break: bool,
    ) -> Result<(), Thrown> {
        let n = match level {
            None => 1usize,
            Some(Expression::Literal(Literal::Integer(i))) => i.value.unwrap_or(1).max(1) as usize,
            Some(_) => {
                return Err(Thrown::error(
                    "Error",
                    "'break'/'continue' operator accepts only a constant integer level",
                ))
            }
        };
        let kw = if is_break { "break" } else { "continue" };
        if n == 0 || self.loops.len() < n {
            return Err(Thrown::error(
                "Error",
                format!("Cannot '{kw}' {n} level(s)"),
            ));
        }
        let idx = self.loops.len() - n;
        let pos = self.code.len();
        if self.finally_rets.len() > self.loops[idx].finally_depth {
            self.emit(Instr::JmpFinally { target: 0 }); // patched at loop close
        } else {
            self.emit(Instr::Nop); // patched to Jmp at loop close
        }
        if is_break {
            self.loops[idx].break_jumps.push(pos);
        } else {
            self.loops[idx].continue_jumps.push(pos);
        }
        Ok(())
    }

    fn patch_all(&mut self, positions: Vec<usize>, target: u32) {
        for p in positions {
            self.patch_jmp(p, target);
        }
    }

    fn compile_match(&mut self, m: &Match) -> Result<Reg, Thrown> {
        let subject = self.compile_expr(m.expression)?;
        let dst = self.alloc_temp();
        let arms: Vec<&MatchArm> = m.arms.iter().collect();

        // Dispatch: strict === against each arm condition, in order.
        let mut dispatch: Vec<(usize, usize, Reg)> = Vec::new();
        let mut default_idx = None;
        for (idx, arm) in arms.iter().enumerate() {
            match arm {
                MatchArm::Expression(a) => {
                    for cond in a.conditions.iter() {
                        let c = self.compile_expr(cond)?;
                        let t = self.alloc_temp();
                        self.emit(Instr::Cmp {
                            dst: t,
                            a: subject,
                            b: c,
                            kind: CmpKind::Identical,
                        });
                        let pos = self.code.len();
                        self.emit(Instr::Nop); // JmpTrue t -> arm body
                        dispatch.push((idx, pos, t));
                    }
                }
                MatchArm::Default(_) => default_idx = Some(idx),
            }
        }
        let no_match_pos = self.code.len();
        self.emit(Instr::Nop); // Jmp -> default arm or throw

        // Arm bodies.
        let mut starts = vec![0u32; arms.len()];
        let mut end_jumps = Vec::new();
        for (idx, arm) in arms.iter().enumerate() {
            starts[idx] = self.here();
            let e = match arm {
                MatchArm::Expression(a) => a.expression,
                MatchArm::Default(a) => a.expression,
            };
            let r = self.compile_expr(e)?;
            self.emit(Instr::Mov { dst, src: r });
            end_jumps.push(self.code.len());
            self.emit(Instr::Nop); // Jmp -> end
        }
        let throw_label = self.here();
        self.emit(Instr::ThrowMatchError { subject });
        let end = self.here();

        for (idx, pos, cond) in dispatch {
            self.code[pos] = Instr::JmpTrue {
                cond,
                target: starts[idx],
            };
        }
        let nm_target = default_idx.map(|i| starts[i]).unwrap_or(throw_label);
        self.patch_jmp(no_match_pos, nm_target);
        for j in end_jumps {
            self.patch_jmp(j, end);
        }
        Ok(dst)
    }

    // -- expression lowering (returns the register holding the result) --

    fn compile_expr(&mut self, expr: &Expression) -> Result<Reg, Thrown> {
        use mago_span::HasSpan;
        self.compile_expr_inner(expr)
            .map_err(|e| locate(e, expr.span().start.offset))
    }

    fn compile_expr_inner(&mut self, expr: &Expression) -> Result<Reg, Thrown> {
        match expr {
            Expression::Literal(lit) => self.compile_literal(lit),
            Expression::Parenthesized(p) => self.compile_expr(p.expression),
            Expression::Variable(Variable::Direct(d)) if strip_dollar(d.name) == b"GLOBALS" => {
                let dst = self.alloc_temp();
                self.emit(Instr::FetchGlobals { dst });
                Ok(dst)
            }
            Expression::Variable(Variable::Direct(d)) => Ok(self.cv(strip_dollar(d.name))),
            Expression::Variable(v) => self.compile_variable(v),
            Expression::Binary(b) => self.compile_binary(b),
            Expression::UnaryPrefix(u) => self.compile_unary_prefix(u),
            Expression::UnaryPostfix(u) => self.compile_unary_postfix(u),
            Expression::Assignment(a) => self.compile_assignment(a),
            Expression::Conditional(c) => self.compile_conditional(c),
            Expression::Array(arr) => self.compile_array(arr.elements.iter()),
            Expression::LegacyArray(arr) => self.compile_array(arr.elements.iter()),
            Expression::ArrayAccess(aa) if is_globals_var(aa.array) => {
                let name = self.compile_expr(aa.index)?;
                let dst = self.alloc_temp();
                self.emit(Instr::FetchGlobal { dst, name });
                Ok(dst)
            }
            Expression::ArrayAccess(aa) => {
                let arr = self.compile_expr(aa.array)?;
                let key = self.compile_expr(aa.index)?;
                let dst = self.alloc_temp();
                self.emit(Instr::DimFetch { dst, arr, key });
                Ok(dst)
            }
            Expression::ConstantAccess(ca) => self.compile_constant(&ca.name),
            Expression::Identifier(id) => self.compile_constant(id),
            Expression::CompositeString(cs) => self.compile_composite(cs),
            Expression::Match(m) => self.compile_match(m),
            Expression::Instantiation(inst) => {
                use mago_span::HasSpan;
                self.call_lines.push(line_of(inst.span().start.offset));
                let r = self.compile_new(inst);
                self.call_lines.pop();
                r
            }
            Expression::AnonymousClass(ac) => {
                use mago_span::HasSpan;
                self.call_lines.push(line_of(ac.span().start.offset));
                let r = self.compile_new_anon(ac);
                self.call_lines.pop();
                r
            }
            Expression::Access(acc) => self.compile_access(acc),
            Expression::Clone(cl) => {
                let src = self.compile_expr(cl.object)?;
                let dst = self.alloc_temp();
                self.emit(Instr::CloneObj { dst, src });
                Ok(dst)
            }
            Expression::Throw(t) => {
                let src = self.compile_expr(t.exception)?;
                self.emit(Instr::Throw { src });
                // `throw` never returns; the dst is unreachable.
                let dst = self.alloc_temp();
                Ok(dst)
            }
            Expression::Construct(c) => self.compile_construct(c),
            Expression::Call(call) => {
                use mago_span::HasSpan;
                use rhp_frontend::cst::Call;
                let at = match call {
                    Call::Function(fc) => fc.function.span(),
                    Call::Method(mc) => mc.method.span(),
                    Call::NullSafeMethod(mc) => mc.method.span(),
                    Call::StaticMethod(smc) => smc.method.span(),
                };
                self.call_lines.push(line_of(at.start.offset));
                let r = self.compile_call(call);
                self.call_lines.pop();
                r
            }
            Expression::ArrowFunction(af) => self.compile_arrow(af),
            Expression::Closure(cl) => self.compile_closure(cl),
            Expression::PartialApplication(pa) => self.compile_fcc(pa),
            Expression::Yield(y) => self.compile_yield(y),
            Expression::MagicConstant(mc) => self.compile_magic_const(mc),
            other => Err(Thrown::unsupported(
                format!("{} expression", expr_kind(other)),
                "not implemented",
            )),
        }
    }

    /// Lower a magic constant (`__DIR__`, `__FILE__`, `__LINE__`, `__FUNCTION__`,
    /// `__CLASS__`, `__METHOD__`, `__NAMESPACE__`) — all resolved at compile time.
    fn compile_magic_const(&mut self, mc: &MagicConstant) -> Result<Reg, Thrown> {
        use mago_span::HasSpan;
        let dst = self.alloc_temp();
        if let MagicConstant::Line(_) = mc {
            let n = line_of(mc.span().start.offset) as i64;
            self.emit(Instr::LoadInt { dst, n });
            return Ok(dst);
        }
        if matches!(mc, MagicConstant::Class(_))
            && self.class_name.is_some()
            && IN_TRAIT.with(|f| *f.borrow())
        {
            // Inside a trait, `__CLASS__` is the class using it (php resolves
            // it at runtime, like `self::class`).
            let name = self.intern_name(b"class");
            self.emit(Instr::FetchClassConst {
                dst,
                class: ClassRef::SelfClass,
                name,
            });
            return Ok(dst);
        }
        if let MagicConstant::Property(_) = mc {
            return Err(Thrown::unsupported(
                "__PROPERTY__",
                "only valid in property hooks",
            ));
        }
        let s: Vec<u8> = match mc {
            MagicConstant::File(_) => src_file(),
            MagicConstant::Directory(_) => dirname(&src_file()),
            MagicConstant::Function(_) => self.magic_function_name(),
            MagicConstant::Method(_) => self.magic_method_name(),
            MagicConstant::Class(_) | MagicConstant::Trait(_) => self
                .class_name
                .as_ref()
                .map(|c| c.as_bytes().to_vec())
                .unwrap_or_default(),
            MagicConstant::Namespace(_) => cur_ns(),
            MagicConstant::Line(_) | MagicConstant::Property(_) => unreachable!(),
        };
        let k = self.intern_const(Value::str(s));
        self.emit(Instr::LoadConst { dst, k });
        Ok(dst)
    }

    /// `__FUNCTION__`: "" at top level, the function/method name, or a closure's
    /// generated `{closure:…}` name.
    fn magic_function_name(&self) -> Vec<u8> {
        match self.scope_kind {
            ScopeKind::Main | ScopeKind::Init => Vec::new(),
            ScopeKind::Function | ScopeKind::Method => self.name.as_bytes().to_vec(),
            ScopeKind::Closure => self
                .magic_fn_name
                .as_ref()
                .map(|n| n.as_bytes().to_vec())
                .unwrap_or_else(|| b"{closure}".to_vec()),
        }
    }

    /// `__METHOD__`: "Class::method" in a method, the function name in a free
    /// function, the closure name in a closure, "" at top level.
    fn magic_method_name(&self) -> Vec<u8> {
        match self.scope_kind {
            ScopeKind::Method => {
                let mut v = self
                    .class_name
                    .as_ref()
                    .map(|c| c.as_bytes().to_vec())
                    .unwrap_or_default();
                v.extend_from_slice(b"::");
                v.extend_from_slice(self.name.as_bytes());
                v
            }
            ScopeKind::Function | ScopeKind::Closure => self.magic_function_name(),
            ScopeKind::Main | ScopeKind::Init => Vec::new(),
        }
    }

    /// PHP 8.5's closure name `{closure:<enclosing>:<line>}`, built from the scope
    /// that defines the closure (`self`). Top-level/init closures embed the file.
    fn closure_magic_name(&self, offset: u32) -> PhpStr {
        let line = line_of(offset);
        let qual: Vec<u8> = match self.scope_kind {
            ScopeKind::Method => {
                let mut v = self
                    .class_name
                    .as_ref()
                    .map(|c| c.as_bytes().to_vec())
                    .unwrap_or_default();
                v.extend_from_slice(b"::");
                v.extend_from_slice(self.name.as_bytes());
                v.extend_from_slice(b"()");
                v
            }
            ScopeKind::Function => {
                let mut v = self.name.as_bytes().to_vec();
                v.extend_from_slice(b"()");
                v
            }
            ScopeKind::Closure => self
                .magic_fn_name
                .as_ref()
                .map(|n| n.as_bytes().to_vec())
                .unwrap_or_default(),
            ScopeKind::Main | ScopeKind::Init => src_file(),
        };
        let mut s = b"{closure:".to_vec();
        s.extend_from_slice(&qual);
        s.push(b':');
        s.extend_from_slice(line.to_string().as_bytes());
        s.push(b'}');
        PhpStr::from_bytes(s)
    }

    fn compile_literal(&mut self, lit: &Literal) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        match literal_value(lit) {
            Value::Null => self.emit(Instr::LoadNull { dst }),
            Value::Bool(b) => self.emit(Instr::LoadBool { dst, b }),
            Value::Int(n) => self.emit(Instr::LoadInt { dst, n }),
            v => {
                let k = self.intern_const(v);
                self.emit(Instr::LoadConst { dst, k });
            }
        }
        Ok(dst)
    }

    fn compile_binary(&mut self, b: &rhp_frontend::cst::Binary) -> Result<Reg, Thrown> {
        use BinaryOperator as Op;
        // Short-circuit operators lower to branches.
        match &b.operator {
            Op::And(_) | Op::LowAnd(_) => return self.compile_logical(b, true),
            Op::Or(_) | Op::LowOr(_) => return self.compile_logical(b, false),
            Op::NullCoalesce(_) => return self.compile_coalesce(b),
            Op::Instanceof(_) => {
                let val = self.compile_expr(b.lhs)?;
                let class = self.class_ref(b.rhs)?;
                let dst = self.alloc_temp();
                self.emit(Instr::InstanceOf { dst, val, class });
                return Ok(dst);
            }
            _ => {}
        }

        let a = self.compile_expr(b.lhs)?;
        let r = self.compile_expr(b.rhs)?;
        // zend_vm's SPEC_RULE_COMMUTATIVE: php swaps the operands of these
        // opcodes when op1's kind is lower than op2's, which decides whose
        // object handler runs first.
        let commutative = matches!(
            &b.operator,
            Op::Multiplication(_)
                | Op::BitwiseAnd(_)
                | Op::BitwiseOr(_)
                | Op::BitwiseXor(_)
                | Op::Equal(_)
                | Op::NotEqual(_)
                | Op::AngledNotEqual(_)
                | Op::Identical(_)
                | Op::NotIdentical(_)
        );
        let (a, r) = if commutative && php_swaps_operands(b.lhs, b.rhs) {
            (r, a)
        } else {
            (a, r)
        };
        let dst = self.alloc_temp();
        let cmp = |k| Instr::Cmp {
            dst,
            a,
            b: r,
            kind: k,
        };
        let instr = match &b.operator {
            Op::Addition(_) => Instr::Add { dst, a, b: r },
            Op::Subtraction(_) => Instr::Sub { dst, a, b: r },
            Op::Multiplication(_) => Instr::Mul { dst, a, b: r },
            Op::Division(_) => Instr::Div { dst, a, b: r },
            Op::Modulo(_) => Instr::Mod { dst, a, b: r },
            Op::Exponentiation(_) => Instr::Pow { dst, a, b: r },
            Op::BitwiseAnd(_) => Instr::BitAnd { dst, a, b: r },
            Op::BitwiseOr(_) => Instr::BitOr { dst, a, b: r },
            Op::BitwiseXor(_) => Instr::BitXor { dst, a, b: r },
            Op::LeftShift(_) => Instr::Shl { dst, a, b: r },
            Op::RightShift(_) => Instr::Shr { dst, a, b: r },
            Op::StringConcat(_) => Instr::Concat { dst, a, b: r },
            Op::Equal(_) => cmp(CmpKind::Eq),
            Op::NotEqual(_) | Op::AngledNotEqual(_) => cmp(CmpKind::Ne),
            Op::Identical(_) => cmp(CmpKind::Identical),
            Op::NotIdentical(_) => cmp(CmpKind::NotIdentical),
            Op::LessThan(_) => cmp(CmpKind::Lt),
            Op::LessThanOrEqual(_) => cmp(CmpKind::Le),
            Op::GreaterThan(_) => cmp(CmpKind::Gt),
            Op::GreaterThanOrEqual(_) => cmp(CmpKind::Ge),
            Op::Spaceship(_) => cmp(CmpKind::Spaceship),
            Op::LowXor(_) => {
                // a xor b == (bool)a != (bool)b, via cast+compare
                Instr::Cmp {
                    dst,
                    a,
                    b: r,
                    kind: CmpKind::Ne,
                }
            }
            Op::And(_)
            | Op::Or(_)
            | Op::LowAnd(_)
            | Op::LowOr(_)
            | Op::NullCoalesce(_)
            | Op::Instanceof(_) => {
                unreachable!("handled above")
            }
        };
        self.emit(instr);
        Ok(dst)
    }

    /// `&&`/`and` (is_and=true) or `||`/`or` lowered to short-circuit branches,
    /// materializing a bool in `dst`. On the short path `&&`→false, `||`→true;
    /// on the full path `dst = (bool)rhs`.
    fn compile_logical(
        &mut self,
        b: &rhp_frontend::cst::Binary,
        is_and: bool,
    ) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        let a = self.compile_expr(b.lhs)?;
        let jcond = self.code.len();
        self.emit(Instr::Nop); // JmpFalse(&&)/JmpTrue(||) a -> short
        let rb = self.compile_expr(b.rhs)?;
        self.emit(Instr::CastBool { dst, a: rb });
        let jend = self.code.len();
        self.emit(Instr::Nop); // Jmp end
        let short = self.code.len() as u32;
        self.emit(Instr::LoadBool { dst, b: !is_and });
        let end = self.code.len() as u32;
        self.code[jcond] = if is_and {
            Instr::JmpFalse {
                cond: a,
                target: short,
            }
        } else {
            Instr::JmpTrue {
                cond: a,
                target: short,
            }
        };
        self.code[jend] = Instr::Jmp { target: end };
        Ok(dst)
    }

    fn compile_coalesce(&mut self, b: &rhp_frontend::cst::Binary) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        let a = self.compile_quiet(b.lhs)?;
        self.emit(Instr::Mov { dst, src: a });
        let jnn = self.code.len();
        self.emit(Instr::Nop); // JmpNotNull dst -> end
        let r = self.compile_expr(b.rhs)?;
        self.emit(Instr::Mov { dst, src: r });
        let end = self.code.len() as u32;
        self.code[jnn] = Instr::JmpNotNull {
            src: dst,
            target: end,
        };
        Ok(dst)
    }

    fn compile_unary_prefix(&mut self, u: &rhp_frontend::cst::UnaryPrefix) -> Result<Reg, Thrown> {
        use UnaryPrefixOperator as Op;
        match &u.operator {
            Op::PreIncrement(_) => return self.compile_inc_dec(u.operand, true, 1),
            Op::PreDecrement(_) => return self.compile_inc_dec(u.operand, true, -1),
            Op::UnsetCast(..) | Op::VoidCast(..) | Op::RealCast(..) => {
                return Err(Thrown::unsupported(
                    "(unset)/(void)/(real) cast",
                    "removed/refused in PHP 8",
                ))
            }
            Op::Reference(_) => return Err(Thrown::unsupported("& reference", "not implemented")),
            Op::ErrorControl(_) => {
                self.emit(Instr::BeginSilence);
                let a = self.compile_expr(u.operand)?;
                let dst = self.alloc_temp();
                self.emit(Instr::Mov { dst, src: a });
                self.emit(Instr::EndSilence);
                return Ok(dst);
            }
            _ => {}
        }
        let a = self.compile_expr(u.operand)?;
        let dst = self.alloc_temp();
        let instr = match &u.operator {
            Op::Negation(_) => Instr::Neg { dst, a },
            Op::Plus(_) => Instr::CastInt { dst, a }, // +x coerces to number; refined below
            Op::Not(_) => Instr::Not { dst, a },
            Op::BitwiseNot(_) => Instr::BitNot { dst, a },
            Op::IntCast(..) | Op::IntegerCast(..) => Instr::CastInt { dst, a },
            Op::FloatCast(..) | Op::DoubleCast(..) => Instr::CastFloat { dst, a },
            Op::StringCast(..) | Op::BinaryCast(..) => Instr::CastString { dst, a },
            Op::BoolCast(..) | Op::BooleanCast(..) => Instr::CastBool { dst, a },
            Op::ArrayCast(..) => Instr::CastArray { dst, a },
            Op::ObjectCast(..) => Instr::CastObject { dst, a },
            _ => unreachable!("handled above"),
        };
        // `+x` is "unary plus" = numeric coercion, not (int). Use Neg-of-Neg? No:
        // emit Mul by 1 would change int->int; simplest faithful lowering is a
        // dedicated path — represent +x as (x * 1) via Add with 0.
        if matches!(u.operator, Op::Plus(_)) {
            // +x == 0 + x  → preserves int/float per ops::add numeric coercion
            let zero = self.alloc_temp();
            self.emit(Instr::LoadInt { dst: zero, n: 0 });
            self.emit(Instr::Add { dst, a: zero, b: a });
            return Ok(dst);
        }
        self.emit(instr);
        Ok(dst)
    }

    fn compile_unary_postfix(
        &mut self,
        u: &rhp_frontend::cst::UnaryPostfix,
    ) -> Result<Reg, Thrown> {
        use rhp_frontend::cst::UnaryPostfixOperator as Op;
        match &u.operator {
            Op::PostIncrement(_) => self.compile_inc_dec(u.operand, false, 1),
            Op::PostDecrement(_) => self.compile_inc_dec(u.operand, false, -1),
        }
    }

    fn compile_inc_dec(
        &mut self,
        target: &Expression,
        pre: bool,
        delta: i8,
    ) -> Result<Reg, Thrown> {
        check_globals_write(target)?;
        match target {
            Expression::Variable(Variable::Direct(d)) => {
                let cv = self.cv(strip_dollar(d.name));
                let dst = self.alloc_temp();
                self.emit(Instr::IncDec {
                    dst,
                    cv,
                    pre,
                    delta,
                });
                Ok(dst)
            }
            Expression::Parenthesized(p) => self.compile_inc_dec(p.expression, pre, delta),
            Expression::ArrayAccess(_)
            | Expression::Variable(_)
            | Expression::Access(Access::Property(_))
            | Expression::Access(Access::StaticProperty(_)) => {
                let mut steps = Vec::new();
                let root = self.collect_lvalue_steps(target, &mut steps)?;
                self.emit_cursor(root, &steps);
                let dst = self.alloc_temp();
                self.emit(Instr::IncDecLval { dst, pre, delta });
                Ok(dst)
            }
            _ => Err(Thrown::unsupported(
                "++/-- target",
                "variables, array elements, properties",
            )),
        }
    }

    fn compile_conditional(&mut self, c: &rhp_frontend::cst::Conditional) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        let cond = self.compile_expr(c.condition)?;
        let jf = self.code.len();
        self.emit(Instr::Nop); // JmpFalse cond -> else
        match &c.then {
            Some(then) => {
                let t = self.compile_expr(then)?;
                self.emit(Instr::Mov { dst, src: t });
            }
            None => {
                // Elvis: reuse the condition value when truthy.
                self.emit(Instr::Mov { dst, src: cond });
            }
        }
        let jend = self.code.len();
        self.emit(Instr::Nop); // Jmp end
        let else_label = self.code.len() as u32;
        let e = self.compile_expr(c.r#else)?;
        self.emit(Instr::Mov { dst, src: e });
        let end = self.code.len() as u32;
        self.code[jf] = Instr::JmpFalse {
            cond,
            target: else_label,
        };
        self.code[jend] = Instr::Jmp { target: end };
        Ok(dst)
    }

    fn compile_array<'a>(
        &mut self,
        els: impl Iterator<Item = &'a ArrayElement<'a>> + Clone,
    ) -> Result<Reg, Thrown> {
        // An array of plain literals (CallMap-style data files) is one
        // constant: no per-element temporaries, no runtime building.
        if let Some(v) = literal_array(els.clone()) {
            let k = self.intern_const(v);
            let dst = self.alloc_temp();
            self.emit(Instr::LoadConst { dst, k });
            return Ok(dst);
        }
        let dst = self.alloc_temp();
        self.emit(Instr::NewArray { dst, cap: 0 });
        for el in els {
            match el {
                ArrayElement::KeyValue(kv) => {
                    let key = self.compile_expr(kv.key)?;
                    if let Some(inner) = as_reference(kv.value) {
                        let val = self.compile_ref_source(inner)?;
                        self.emit(Instr::ArrSetRef { arr: dst, key, val });
                    } else {
                        let val = self.compile_expr(kv.value)?;
                        self.emit(Instr::ArrSet { arr: dst, key, val });
                    }
                }
                ArrayElement::Value(v) => {
                    if let Some(inner) = as_reference(v.value) {
                        let val = self.compile_ref_source(inner)?;
                        self.emit(Instr::ArrPushRef { arr: dst, val });
                    } else {
                        let val = self.compile_expr(v.value)?;
                        self.emit(Instr::ArrPush { arr: dst, val });
                    }
                }
                ArrayElement::Variadic(v) => {
                    let src = self.compile_expr(v.value)?;
                    self.emit(Instr::ArrSpread { arr: dst, src });
                }
                ArrayElement::Missing(_) => {}
            }
        }
        Ok(dst)
    }

    /// Interpolated (`"...$x..."`) and heredoc strings: concatenate the parts,
    /// stringifying each via `Concat` (which routes to `ops::concat` →
    /// `conv::to_string_bytes`). Backticks (`ShellExecute`) are refused.
    fn compile_composite(&mut self, cs: &CompositeString) -> Result<Reg, Thrown> {
        let parts = match cs {
            CompositeString::Interpolated(i) => &i.parts,
            CompositeString::Document(d) => &d.parts,
            CompositeString::ShellExecute(_) => {
                return Err(Thrown::unsupported(
                    "backtick shell execution",
                    "never spawns a shell; refused by design",
                ))
            }
        };
        // Start from "" so every part is stringified uniformly.
        let dst = self.alloc_temp();
        let empty = self.intern_const(Value::str(""));
        self.emit(Instr::LoadConst { dst, k: empty });
        for part in parts.iter() {
            let pr = match part {
                StringPart::Literal(l) => {
                    let r = self.alloc_temp();
                    let k = self.intern_const(Value::str(l.value.unwrap_or(l.raw).to_vec()));
                    self.emit(Instr::LoadConst { dst: r, k });
                    r
                }
                // Simple syntax `"$a[key]"`: an unquoted key is a string
                // literal, not a constant.
                StringPart::Expression(Expression::ArrayAccess(aa))
                    if matches!(aa.index, Expression::Identifier(_)) =>
                {
                    let Expression::Identifier(id) = aa.index else {
                        unreachable!()
                    };
                    let k = self.intern_const(Value::str(id.value().to_vec()));
                    let key = self.alloc_temp();
                    self.emit(Instr::LoadConst { dst: key, k });
                    let r = self.alloc_temp();
                    if is_globals_var(aa.array) {
                        self.emit(Instr::FetchGlobal { dst: r, name: key });
                    } else {
                        let arr = self.compile_expr(aa.array)?;
                        self.emit(Instr::DimFetch { dst: r, arr, key });
                    }
                    r
                }
                StringPart::Expression(e) => self.compile_expr(e)?,
                StringPart::BracedExpression(b) => self.compile_expr(b.expression)?,
            };
            self.emit(Instr::Concat { dst, a: dst, b: pr });
        }
        Ok(dst)
    }

    fn compile_constant(&mut self, id: &Identifier) -> Result<Reg, Thrown> {
        let name = identifier_name(id);
        let dst = self.alloc_temp();
        match name {
            b"true" | b"TRUE" | b"True" => self.emit(Instr::LoadBool { dst, b: true }),
            b"false" | b"FALSE" | b"False" => self.emit(Instr::LoadBool { dst, b: false }),
            b"null" | b"NULL" | b"Null" => self.emit(Instr::LoadNull { dst }),
            _ => {
                let (primary, fallback) = resolve_name(id, NameKind::Const);
                let n = self.intern_name(&primary);
                let fb = self.intern_fallback(&fallback);
                self.emit(Instr::FetchConst {
                    dst,
                    name: n,
                    fallback: fb,
                });
            }
        }
        Ok(dst)
    }

    /// Lower `include`/`require`/`*_once $path` to an `Include` opcode; the VM
    /// resolves, compiles and runs the file at runtime, leaving its return in `dst`.
    fn compile_include(
        &mut self,
        path_expr: &Expression,
        kind: IncludeKind,
    ) -> Result<Reg, Thrown> {
        let path = self.compile_expr(path_expr)?;
        let dst = self.alloc_temp();
        self.emit(Instr::Include { dst, path, kind });
        Ok(dst)
    }

    fn compile_construct(&mut self, c: &Construct) -> Result<Reg, Thrown> {
        match c {
            Construct::Print(p) => {
                let src = self.compile_expr(p.value)?;
                let dst = self.alloc_temp();
                self.emit(Instr::Print { dst, src });
                Ok(dst)
            }
            Construct::Eval(e) => {
                let src = self.compile_expr(e.value)?;
                let dst = self.alloc_temp();
                self.emit(Instr::Eval { dst, src });
                Ok(dst)
            }
            Construct::Include(i) => self.compile_include(i.value, IncludeKind::Include),
            Construct::IncludeOnce(i) => self.compile_include(i.value, IncludeKind::IncludeOnce),
            Construct::Require(i) => self.compile_include(i.value, IncludeKind::Require),
            Construct::RequireOnce(i) => self.compile_include(i.value, IncludeKind::RequireOnce),
            Construct::Isset(i) => {
                // AND over each operand: true iff every value is non-null.
                let dst = self.alloc_temp();
                let mut jumps = Vec::new();
                for v in i.values.iter() {
                    // `isset($o->prop)` must consult `__isset`, not `__get`.
                    // A nullsafe `$o?->prop` too: a null object makes IssetProp false.
                    let prop = match v {
                        Expression::Access(Access::Property(pa)) => Some((pa.object, &pa.property)),
                        Expression::Access(Access::NullSafeProperty(pa)) => {
                            Some((pa.object, &pa.property))
                        }
                        _ => None,
                    };
                    if let Some((object, property)) = prop {
                        let obj = self.compile_quiet(object)?;
                        let r = self.alloc_temp();
                        match self.compile_selector(property)? {
                            Sel::Static(name) => self.emit(Instr::IssetProp { dst: r, obj, name }),
                            Sel::Dyn(name) => self.emit(Instr::IssetPropDyn { dst: r, obj, name }),
                        }
                        // IssetProp yields a bool; false short-circuits.
                        let pos = self.code.len();
                        self.emit(Instr::Nop); // JmpFalse r -> false
                        jumps.push((pos, r, true));
                        continue;
                    }
                    // `isset($c[$k])`: a string offset out of range reads as
                    // "" (not null), so dimension isset has its own opcode.
                    if let Expression::ArrayAccess(aa) = v {
                        let arr = self.compile_quiet(aa.array)?;
                        let key = self.compile_expr(aa.index)?;
                        let r = self.alloc_temp();
                        self.emit(Instr::IssetDim { dst: r, arr, key });
                        let pos = self.code.len();
                        self.emit(Instr::Nop); // JmpFalse r -> false
                        jumps.push((pos, r, true));
                        continue;
                    }
                    let r = self.compile_quiet(v)?; // null if absent (DimFetch returns null)
                    let pos = self.code.len();
                    self.emit(Instr::Nop); // JmpNull r -> false
                    jumps.push((pos, r, false));
                }
                self.emit(Instr::LoadBool { dst, b: true });
                let jend = self.code.len();
                self.emit(Instr::Nop); // Jmp end
                let lfalse = self.here();
                self.emit(Instr::LoadBool { dst, b: false });
                let lend = self.here();
                for (pos, r, is_bool) in jumps {
                    self.code[pos] = if is_bool {
                        Instr::JmpFalse {
                            cond: r,
                            target: lfalse,
                        }
                    } else {
                        Instr::JmpNull {
                            src: r,
                            target: lfalse,
                        }
                    };
                }
                self.code[jend] = Instr::Jmp { target: lend };
                Ok(dst)
            }
            Construct::Empty(e) => {
                // `empty($o->prop)` = !isset($o->prop) || !$o->prop: an
                // inaccessible / unset property is empty (no error), via
                // `__isset` then `__get` like php.
                let prop = match e.value {
                    Expression::Access(Access::Property(pa)) => Some((pa.object, &pa.property)),
                    Expression::Access(Access::NullSafeProperty(pa)) => {
                        Some((pa.object, &pa.property))
                    }
                    _ => None,
                };
                if let Some((object, property)) = prop {
                    let obj = self.compile_quiet(object)?;
                    let dst = self.alloc_temp();
                    match self.compile_selector(property)? {
                        Sel::Static(name) => self.emit(Instr::EmptyProp { dst, obj, name }),
                        Sel::Dyn(name) => self.emit(Instr::EmptyPropDyn { dst, obj, name }),
                    }
                    return Ok(dst);
                }
                // empty(x) == !to_bool(x); absent/null -> true (Not handles it).
                let r = self.compile_quiet(e.value)?;
                let dst = self.alloc_temp();
                self.emit(Instr::Not { dst, a: r });
                Ok(dst)
            }
            Construct::Exit(e) => self.compile_exit(e.arguments.as_ref()),
            Construct::Die(d) => self.compile_exit(d.arguments.as_ref()),
        }
    }

    /// `exit`/`die` with an optional single argument (a status int or a message
    /// string to print). Evaluates the argument (or null) and emits `Exit`.
    fn compile_exit(
        &mut self,
        arguments: Option<&rhp_frontend::cst::ArgumentList>,
    ) -> Result<Reg, Thrown> {
        let arg = match arguments.and_then(|a| a.arguments.first()) {
            Some(Argument::Positional(p)) => self.compile_expr(p.value)?,
            _ => {
                let r = self.alloc_temp();
                self.emit(Instr::LoadNull { dst: r });
                r
            }
        };
        self.emit(Instr::Exit { arg });
        // `exit` never returns a value, but expressions expect a result register.
        let dst = self.alloc_temp();
        self.emit(Instr::LoadNull { dst });
        Ok(dst)
    }

    fn compile_call(&mut self, call: &rhp_frontend::cst::Call) -> Result<Reg, Thrown> {
        use rhp_frontend::cst::Call;
        match call {
            Call::Function(fc) => self.compile_function_call(fc),
            Call::Method(mc) => {
                self.compile_method_call(mc.object, &mc.method, &mc.argument_list, false)
            }
            Call::StaticMethod(smc) => self.compile_static_call(smc),
            Call::NullSafeMethod(mc) => {
                self.compile_method_call(mc.object, &mc.method, &mc.argument_list, true)
            }
        }
    }

    /// `throw new \Error($msg)` at the current line; the returned register
    /// is never written.
    fn emit_throw_error(&mut self, msg: &str) -> Reg {
        let base = self.alloc_temps(1);
        let k = self.intern_const(Value::str(msg));
        self.emit(Instr::LoadConst { dst: base, k });
        let class = ClassRef::Name(self.intern_name(b"Error"));
        let dst = self.alloc_temp();
        self.emit(Instr::New {
            dst,
            class,
            base,
            argc: 1,
        });
        self.emit(Instr::Throw { src: dst });
        dst
    }

    fn compile_function_call(
        &mut self,
        fc: &rhp_frontend::cst::FunctionCall,
    ) -> Result<Reg, Thrown> {
        let callee_id: &Identifier = match fc.function {
            Expression::Identifier(id) => id,
            Expression::ConstantAccess(ca) => &ca.name,
            // `$f(...)`, `($expr)(...)`, `$arr[0](...)`: the callee is a runtime
            // value (closure / string name / `[obj, method]` / __invoke object).
            _ => return self.compile_dynamic_call(fc),
        };
        // Resolve the (possibly namespaced) callee to a primary name + optional
        // global fallback (PHP's fall-back-to-global for unqualified calls).
        let (name_bytes, fallback) = resolve_name(callee_id, NameKind::Func);

        // Spread / named-argument call `foo(...$args)` / `foo(b: 1)`: marshal
        // args into an array, emit CallV (the VM places named arguments).
        if has_spread(&fc.argument_list) || has_named(&fc.argument_list) {
            let args = self.build_args_array(Some(&fc.argument_list))?;
            let name = self.intern_name(&name_bytes);
            let fb = self.intern_fallback(&fallback);
            let dst = self.alloc_temp();
            self.emit(Instr::CallV {
                dst,
                name,
                args,
                fallback: fb,
            });
            return Ok(dst);
        }

        // Per-parameter by-ref flags of the (statically-known) callee: a user
        // function's recorded signature (tried under the namespaced name, then the
        // global fallback), or a known by-ref builtin (always global).
        let name_lc = name_bytes.to_ascii_lowercase();
        let global_lc = fallback.as_ref().map(|f| f.to_ascii_lowercase());
        let sig = self
            .sigs
            .get(&name_lc)
            .cloned()
            .or_else(|| global_lc.as_ref().and_then(|g| self.sigs.get(g).cloned()))
            .or_else(|| {
                builtin_by_ref(
                    global_lc.as_deref().unwrap_or(&name_lc),
                    fc.argument_list.arguments.len(),
                )
            });
        // The builtin whose stub gave `sig` (not a user function of the file).
        let builtin = (!self.sigs.contains_key(&name_lc)
            && !global_lc
                .as_ref()
                .is_some_and(|g| self.sigs.contains_key(g)))
        .then(|| builtin_sig(global_lc.as_deref().unwrap_or(&name_lc)))
        .flatten();
        // `@prefer-ref` positions of the builtin (array_multisort, extract): a
        // non-variable argument is passed by value, silently.
        let prefer_ref = |i: usize| {
            use rhp_builtins::signatures::Signature;
            builtin.is_some_and(|f| f.prefer_ref(i))
        };

        // Compile each arg into a value reg, or (for a by-ref param) note the
        // variable's CV so it can be materialized as a reference.
        enum ArgSlot<'e> {
            Value(Reg),
            Ref(Reg),
            /// A by-ref argument that is a property / array-element lvalue.
            RefLval(&'e Expression<'e>),
            /// A call result for a by-ref parameter (passed with a notice).
            NoRef(Reg),
        }
        let mut slots = Vec::new();
        for (i, arg) in fc.argument_list.arguments.iter().enumerate() {
            let p = match arg {
                Argument::Positional(p) if p.ellipsis.is_none() => p,
                Argument::Positional(_) => {
                    return Err(Thrown::unsupported(
                        "argument spread",
                        "call-site spread is deferred",
                    ))
                }
                Argument::Named(_) => {
                    return Err(Thrown::unsupported("named arguments", "deferred"))
                }
            };
            let by_ref = sig
                .as_ref()
                .and_then(|s| s.get(i).copied())
                .unwrap_or(false);
            if by_ref {
                match p.value {
                    Expression::Variable(Variable::Direct(d)) => {
                        slots.push(ArgSlot::Ref(self.cv(strip_dollar(d.name))))
                    }
                    // `sort($a['k'])`, `array_push($this->items, …)`.
                    Expression::ArrayAccess(_)
                    | Expression::ArrayAppend(_)
                    | Expression::Access(Access::Property(_))
                    | Expression::Access(Access::StaticProperty(_)) => {
                        slots.push(ArgSlot::RefLval(p.value))
                    }
                    // A call result for a by-ref parameter: php passes it
                    // anyway with "Only variables should be passed by reference"
                    // (ZEND_SEND_VAR_NO_REF / SEND_FUNC_ARG). prefer-ref
                    // parameters accept a temporary silently.
                    Expression::Call(_) => {
                        let r = self.compile_expr(p.value)?;
                        if prefer_ref(i) {
                            slots.push(ArgSlot::Value(r))
                        } else {
                            slots.push(ArgSlot::NoRef(r))
                        }
                    }
                    _ if prefer_ref(i) => slots.push(ArgSlot::Value(self.compile_expr(p.value)?)),
                    // zend_cannot_pass_by_reference: a catchable Error raised
                    // when the argument is sent, after its expression runs.
                    // A variadic parameter is not named.
                    _ if builtin.is_some() => {
                        let f = builtin.expect("checked");
                        self.compile_expr(p.value)?;
                        let named = f
                            .params
                            .get(i)
                            .filter(|p| !p.variadic())
                            .map(|p| format!(" (${})", p.name))
                            .unwrap_or_default();
                        let msg = format!(
                            "{}(): Argument #{}{named} could not be passed by reference",
                            f.name,
                            i + 1
                        );
                        slots.push(ArgSlot::Value(self.emit_throw_error(&msg)));
                    }
                    _ => {
                        return Err(Thrown::error(
                            "Error",
                            format!(
                                "{}(): Argument #{} could not be passed by reference",
                                String::from_utf8_lossy(&name_bytes),
                                i + 1
                            ),
                        ))
                    }
                }
            } else {
                // A variable's value is taken when its argument is evaluated
                // (php's SEND_VAR), before later arguments can change it.
                let r = self.compile_expr(p.value)?;
                let r = self.snapshot_arg(&fc.argument_list, i, p.value, r);
                slots.push(ArgSlot::Value(r));
            }
        }
        let argc = slots.len() as u16;
        let base = self.alloc_temps(argc);
        for (i, slot) in slots.into_iter().enumerate() {
            let dst = base + i as Reg;
            match slot {
                ArgSlot::Value(r) => self.emit(Instr::Mov { dst, src: r }),
                ArgSlot::Ref(cv) => self.emit(Instr::MakeRef { dst, cv }),
                ArgSlot::NoRef(src) => self.emit(Instr::SendNoRef { dst, src }),
                ArgSlot::RefLval(expr) => {
                    let mut steps = Vec::new();
                    let root = self.collect_lvalue_steps(expr, &mut steps)?;
                    self.emit_cursor(root, &steps);
                    self.emit(Instr::MakeRefLval { dst });
                }
            }
        }
        let name = self.intern_name(&name_bytes);
        let fb = self.intern_fallback(&fallback);
        let dst = self.alloc_temp();
        self.emit(Instr::Call {
            dst,
            name,
            base,
            argc,
            fallback: fb,
        });
        Ok(dst)
    }

    /// `$f(args)` — call a runtime callable value. The callee is evaluated to a
    /// register; arguments are passed by value (the signature is unknown at the
    /// call site, so by-ref parameters are not honoured for dynamic calls).
    fn compile_dynamic_call(
        &mut self,
        fc: &rhp_frontend::cst::FunctionCall,
    ) -> Result<Reg, Thrown> {
        let callee = self.compile_expr(fc.function)?;

        if has_spread(&fc.argument_list) || has_named(&fc.argument_list) {
            let args = self.build_args_array(Some(&fc.argument_list))?;
            let dst = self.alloc_temp();
            self.emit(Instr::CallDynV { dst, callee, args });
            return Ok(dst);
        }

        // The callee is only known at run time, so (as for method calls) a
        // simple-variable argument travels as a reference; a by-value
        // parameter dereferences it.
        let (base, argc) = self.compile_arg_window_ex(Some(&fc.argument_list), &[], true)?;
        let dst = self.alloc_temp();
        self.emit(Instr::CallDyn {
            dst,
            callee,
            base,
            argc,
        });
        Ok(dst)
    }

    /// `yield` / `yield $v` / `yield $k => $v`. The result register receives the
    /// value sent in via `Generator::send()` when execution resumes.
    fn compile_yield(&mut self, y: &rhp_frontend::cst::Yield) -> Result<Reg, Thrown> {
        use rhp_frontend::cst::Yield;
        let (key, value) = match y {
            Yield::Value(v) => {
                let value = match v.value {
                    // A by-reference generator yields a reference to the
                    // variable / element / property (ZEND_YIELD, BP_VAR_W).
                    Some(e) if self.returns_ref => self.compile_yield_ref(e)?,
                    Some(e) => self.compile_expr(e)?,
                    None => {
                        let t = self.alloc_temp();
                        self.emit(Instr::LoadNull { dst: t });
                        if self.returns_ref {
                            // A bare `yield` yields null, without the
                            // "Only variable references" notice.
                            let r = self.alloc_temp();
                            self.emit(Instr::MakeRefVal { dst: r, src: t });
                            r
                        } else {
                            t
                        }
                    }
                };
                (Reg::MAX, value)
            }
            Yield::Pair(p) => {
                let key = self.compile_expr(p.key)?;
                let value = if self.returns_ref {
                    self.compile_yield_ref(p.value)?
                } else {
                    self.compile_expr(p.value)?
                };
                (key, value)
            }
            Yield::From(_) if self.returns_ref => {
                return Err(Thrown::error(
                    "Error",
                    "Cannot use \"yield from\" inside a by-reference generator",
                ))
            }
            Yield::From(yf) => return self.compile_yield_from(yf.iterator),
        };
        let dst = self.alloc_temp();
        self.emit(Instr::Yield { dst, key, value });
        Ok(dst)
    }

    /// The value a by-reference generator yields: a reference to a variable
    /// / element / property, or a by-reference call's result; anything else
    /// is a plain value, which the VM's `Yield` answers with php's "Only
    /// variable references should be yielded by reference" notice.
    fn compile_yield_ref(&mut self, e: &Expression) -> Result<Reg, Thrown> {
        self.compile_ref_source(e)
    }

    /// `yield from $iterable`: re-yield each element (keys preserved); the
    /// expression evaluates to the inner generator's return value (null for an
    /// array). Lowered as a foreach-style loop over existing opcodes.
    fn compile_yield_from(&mut self, iterable: &Expression) -> Result<Reg, Thrown> {
        let src = self.compile_expr(iterable)?;
        let arr_reg = self.alloc_temp();
        let pos_reg = self.alloc_temp();
        let key_cv = self.alloc_temp();
        let val_cv = self.alloc_temp();

        let reset_pos = self.code.len();
        self.emit(Instr::Nop); // FeReset
        let fetch_label = self.here();
        let fetch_pos = self.code.len();
        self.emit(Instr::Nop); // FeFetch
        let ysink = self.alloc_temp();
        self.emit(Instr::Yield {
            dst: ysink,
            key: key_cv,
            value: val_cv,
        });
        self.emit(Instr::Jmp {
            target: fetch_label,
        });
        let end = self.here();
        self.code[reset_pos] = Instr::FeReset {
            arr_reg,
            pos_reg,
            src,
            end,
            delegate: true,
        };
        self.code[fetch_pos] = Instr::FeFetch {
            arr_reg,
            pos_reg,
            key_cv,
            val_cv,
            end,
        };

        // Result: `$src instanceof Generator ? $src->getReturn() : null`.
        let dst = self.alloc_temp();
        let is_gen = self.alloc_temp();
        let gen_class = ClassRef::Name(self.intern_name(b"Generator"));
        self.emit(Instr::InstanceOf {
            dst: is_gen,
            val: src,
            class: gen_class,
        });
        let jf = self.code.len();
        self.emit(Instr::Nop); // JmpFalse is_gen -> lnull
        let base = self.alloc_temps(1);
        let name = self.intern_name(b"getReturn");
        self.emit(Instr::CallMethod {
            dst,
            obj: src,
            name,
            base,
            argc: 0,
        });
        let jend = self.code.len();
        self.emit(Instr::Nop); // Jmp -> lend
        let lnull = self.here();
        self.emit(Instr::LoadNull { dst });
        let lend = self.here();
        self.code[jf] = Instr::JmpFalse {
            cond: is_gen,
            target: lnull,
        };
        self.code[jend] = Instr::Jmp { target: lend };
        Ok(dst)
    }

    /// First-class callable syntax: `f(...)`, `$o->m(...)`, `C::m(...)`. Builds
    /// the underlying callable value, then wraps it as a `Closure` via `MakeFcc`.
    fn compile_fcc(&mut self, pa: &rhp_frontend::cst::PartialApplication) -> Result<Reg, Thrown> {
        use rhp_frontend::cst::PartialApplication as Pa;
        if !pa.is_first_class_callable() {
            return Err(Thrown::unsupported(
                "partial application with placeholders",
                "only the first-class callable form `f(...)` is supported",
            ));
        }
        // Build the callable Value into `callee`.
        let callee = match pa {
            Pa::Function(f) => match f.function {
                // Named function: the callable is its (resolved) name string.
                Expression::Identifier(id) => {
                    let n = self.resolve_callable_name(id);
                    self.load_string_const(&n)
                }
                Expression::ConstantAccess(ca) => {
                    let n = self.resolve_callable_name(&ca.name);
                    self.load_string_const(&n)
                }
                // Dynamic `$f(...)`: wrap the existing callable value.
                other => self.compile_expr(other)?,
            },
            Pa::Method(m) => {
                let obj = self.compile_expr(m.object)?;
                let name = self.selector_string(&m.method)?;
                self.build_pair(obj, &name)
            }
            Pa::StaticMethod(s) => {
                let class = match s.class {
                    Expression::Identifier(id) => {
                        let n = resolve_class_name(id);
                        self.load_string_const(&n)
                    }
                    Expression::ConstantAccess(ca) => {
                        let n = resolve_class_name(&ca.name);
                        self.load_string_const(&n)
                    }
                    // Resolved against the creation scope when called.
                    Expression::Self_(_) => self.load_string_const(b"self"),
                    Expression::Static(_) => self.load_string_const(b"static"),
                    Expression::Parent(_) => self.load_string_const(b"parent"),
                    other => self.compile_expr(other)?,
                };
                let name = self.selector_string(&s.method)?;
                self.build_pair(class, &name)
            }
        };
        let dst = self.alloc_temp();
        self.emit(Instr::MakeFcc { dst, callee });
        Ok(dst)
    }

    /// Load a byte-string constant into a fresh register.
    fn load_string_const(&mut self, bytes: &[u8]) -> Reg {
        let k = self.intern_const(Value::str(bytes.to_vec()));
        let dst = self.alloc_temp();
        self.emit(Instr::LoadConst { dst, k });
        dst
    }

    /// Build a two-element `[first, second-string]` array (a callable pair).
    fn build_pair(&mut self, first: Reg, second: &[u8]) -> Reg {
        let arr = self.alloc_temp();
        self.emit(Instr::NewArray { dst: arr, cap: 2 });
        self.emit(Instr::ArrPush { arr, val: first });
        let s = self.load_string_const(second);
        self.emit(Instr::ArrPush { arr, val: s });
        arr
    }

    /// The raw identifier bytes of a member selector (non-dynamic only).
    fn selector_string(
        &self,
        sel: &rhp_frontend::cst::ClassLikeMemberSelector,
    ) -> Result<Vec<u8>, Thrown> {
        match sel {
            ClassLikeMemberSelector::Identifier(li) => Ok(li.value.to_vec()),
            _ => Err(Thrown::unsupported(
                "dynamic member name",
                "`->$x(...)` deferred",
            )),
        }
    }

    /// `fn(params) => expr` — arrow function. Auto-captures (by value) every
    /// free variable of the body that is a CV in the enclosing scope, plus
    /// `$this` when defined and the arrow is non-static.
    fn compile_arrow(&mut self, af: &rhp_frontend::cst::ArrowFunction) -> Result<Reg, Thrown> {
        let is_static = af.r#static.is_some();
        // Parameter names, to exclude from auto-capture.
        let params: Vec<Vec<u8>> = af
            .parameter_list
            .parameters
            .iter()
            .map(|p| strip_dollar(p.variable.name).to_vec())
            .collect();

        let mut used = Vec::new();
        collect_vars(af.expression, &mut used);
        let mut captures: Vec<(Vec<u8>, bool)> = Vec::new();
        for name in used {
            if name == b"this" {
                continue; // handled below
            }
            if params.iter().any(|p| p == &name) || captures.iter().any(|c| c.0 == name) {
                continue;
            }
            // Arrow functions auto-capture by value only.
            if self.cv_map.iter().any(|n| n == &name) {
                captures.push((name, false));
            }
        }
        // Bind `$this` by value when we're inside an instance context.
        if !is_static && self.cv_map.iter().any(|n| n == b"this") {
            captures.insert(0, (b"this".to_vec(), false));
        }

        self.build_closure(
            is_static,
            &af.parameter_list,
            captures,
            ClosureBody::Arrow(af.expression),
            {
                use mago_span::HasSpan;
                af.span().start.offset
            },
            af.return_type_hint.as_ref().map(|r| &r.hint),
            af.ampersand.is_some(),
            decl_meta(
                &af.attribute_lists,
                &self.sigs,
                self.class_name.as_ref(),
                af.r#fn.span.end.offset,
                af.r#fn.span.start.offset,
                span_of(af).1,
            )?,
        )
    }

    /// `function(params) use(...) { body }` — closure with an explicit capture
    /// list. `use(&$x)` captures by reference (shares a cell with the outer var).
    fn compile_closure(&mut self, cl: &rhp_frontend::cst::Closure) -> Result<Reg, Thrown> {
        let is_static = cl.r#static.is_some();
        let mut captures: Vec<(Vec<u8>, bool)> = Vec::new();
        if !is_static && self.cv_map.iter().any(|n| n == b"this") {
            captures.push((b"this".to_vec(), false));
        }
        if let Some(uses) = &cl.use_clause {
            for u in uses.variables.iter() {
                let name = strip_dollar(u.variable.name).to_vec();
                let by_ref = u.ampersand.is_some();
                if let Some(existing) = captures.iter_mut().find(|c| c.0 == name) {
                    existing.1 |= by_ref;
                } else {
                    captures.push((name, by_ref));
                }
            }
        }
        self.build_closure(
            is_static,
            &cl.parameter_list,
            captures,
            ClosureBody::Block(&cl.body),
            {
                use mago_span::HasSpan;
                cl.span().start.offset
            },
            cl.return_type_hint.as_ref().map(|r| &r.hint),
            cl.ampersand.is_some(),
            decl_meta(
                &cl.attribute_lists,
                &self.sigs,
                self.class_name.as_ref(),
                cl.function.span.end.offset,
                cl.function.span.start.offset,
                span_of(cl).1,
            )?,
        )
    }

    /// Shared closure lowering: compile the body into a fresh proto whose CVs are
    /// `[captures.., params.., locals..]`, register it, then emit the capture
    /// window + `MakeClosure` in the enclosing frame.
    #[allow(clippy::too_many_arguments)] // closure lowering needs the full frame context
    fn build_closure(
        &mut self,
        is_static: bool,
        param_list: &rhp_frontend::cst::FunctionLikeParameterList,
        captures: Vec<(Vec<u8>, bool)>,
        body: ClosureBody,
        closure_offset: u32,
        ret_hint: Option<&Hint>,
        returns_ref: bool,
        meta: rhp_bytecode::DeclMeta,
    ) -> Result<Reg, Thrown> {
        let nc = captures.len() as u16;

        // PHP 8.5 names a closure `{closure:<enclosing scope>:<line>}`, computed
        // from the scope that *defines* it (the current builder).
        let magic_name = self.closure_magic_name(closure_offset);

        // --- compile the closure proto in an isolated builder ---
        // The proto carries the 8.5 name too: traces and error messages print it.
        let mut c = ProtoBuilder::new(magic_name.clone());
        c.sigs = self.sigs.clone();
        c.scope_kind = ScopeKind::Closure;
        c.class_name = self.class_name.clone(); // __CLASS__ in a closure = enclosing class
        c.magic_fn_name = Some(magic_name);
        c.returns_ref = returns_ref;
        for (name, _) in &captures {
            c.cv(name); // captures occupy CVs 0..nc
        }
        let mut params = Vec::new();
        let mut nonconst_defaults: Vec<(u16, Reg, &Expression)> = Vec::new();
        for (idx, p) in param_list.parameters.iter().enumerate() {
            let var = strip_dollar(p.variable.name);
            let cv = c.cv(var);
            let name = c.intern_name(var);
            let default = p
                .default_value
                .as_ref()
                .map(|dv| match c.const_default(dv.value) {
                    Ok(d) => d,
                    Err(_) => {
                        nonconst_defaults.push((idx as u16, cv, dv.value));
                        rhp_bytecode::ParamDefault::Init
                    }
                });
            let dmeta = param_default_meta(p, &c.sigs.clone(), c.class_name.clone().as_ref());
            params.push(rhp_bytecode::ParamInfo {
                name,
                cv,
                by_ref: p.ampersand.is_some(),
                variadic: p.ellipsis.is_some(),
                default,
                type_hint: p.hint.as_ref().map(type_hint),
                line: {
                    use mago_span::HasSpan;
                    line_of(p.span().start.offset)
                },
                attributes: compile_attributes(&p.attribute_lists, &c.sigs, c.class_name.as_ref())?,
                promoted: false,
                default_thunk: dmeta.0,
                default_const: dmeta.1,
            });
        }
        let num_params = params.len() as u16;

        match body {
            ClosureBody::Arrow(e) => {
                c.scan_expr_vars(e);
                c.scan_node_vars(Node::Expression(e));
            }
            ClosureBody::Block(b) => {
                b.statements.iter().for_each(|s| c.scan_stmt_vars(s));
                b.statements
                    .iter()
                    .for_each(|s| c.scan_node_vars(Node::Statement(s)));
            }
        }
        c.freeze_cvs();

        c.emit_param_defaults(&nonconst_defaults)?;

        match body {
            ClosureBody::Arrow(e) => {
                c.cur_line = {
                    use mago_span::HasSpan;
                    line_of(e.span().start.offset)
                };
                let r = c.compile_expr(e)?;
                c.emit(Instr::Ret { src: r });
            }
            ClosureBody::Block(b) => {
                for s in b.statements.iter() {
                    c.compile_stmt(s)?;
                }
                c.emit(Instr::RetNull);
            }
        }

        // `$this` in a non-static closure: its CV receives a rebound object
        // (`Closure::bind` / `bindTo` / `call`) even when the closure was
        // created outside any instance context and so captured nothing.
        let this_cv = if is_static {
            None
        } else {
            c.cv_map.iter().position(|n| n == b"this").map(|i| i as Reg)
        };
        let mut proto = c.finish()?;
        proto.this_cv = this_cv;
        proto.num_params = num_params;
        proto.params = params;
        proto.num_captures = nc;
        proto.is_static = is_static;
        proto.return_type = ret_hint.map(type_hint);
        proto.meta = meta;
        proto.is_generator = match body {
            ClosureBody::Block(b) => block_has_yield(&b.statements),
            ClosureBody::Arrow(e) => expr_has_yield(e),
        };

        let fnidx = CLOSURES.with(|cls| {
            let mut v = cls.borrow_mut();
            let idx = v.len() as u32;
            v.push(Rc::new(proto));
            idx
        });

        // --- emit the capture window + MakeClosure in the enclosing frame ---
        let base = self.alloc_temps(nc.max(1));
        for (i, (name, by_ref)) in captures.iter().enumerate() {
            let dst = base + i as Reg;
            if *by_ref {
                // Share a cell with the outer variable (created if absent).
                let cv = self.cv(name);
                self.emit(Instr::MakeRef { dst, cv });
            } else {
                match self.cv_map.iter().position(|n| n == name) {
                    Some(src) => self.emit(Instr::Mov {
                        dst,
                        src: src as Reg,
                    }),
                    None => self.emit(Instr::LoadNull { dst }),
                }
            }
        }
        let dst = self.alloc_temp();
        self.emit(Instr::MakeClosure {
            dst,
            fnidx,
            base,
            ncaptures: nc,
        });
        Ok(dst)
    }

    /// Collect an lvalue path (root CV + pre-evaluated key/append steps). All
    /// index expressions are compiled into registers here, so the subsequent
    /// cursor opcode sequence is contiguous (no nested cursors).
    fn collect_lvalue_steps(
        &mut self,
        target: &Expression,
        steps: &mut Vec<CursorStep>,
    ) -> Result<LvalRoot, Thrown> {
        match target {
            Expression::Variable(Variable::Direct(d)) => {
                Ok(LvalRoot::Cv(self.cv(strip_dollar(d.name))))
            }
            Expression::ArrayAccess(aa) if is_globals_var(aa.array) => {
                let name = self.compile_expr(aa.index)?;
                Ok(LvalRoot::Global(name))
            }
            Expression::ArrayAccess(aa) => {
                let root = self.collect_lvalue_steps(aa.array, steps)?;
                let key = self.compile_expr(aa.index)?;
                steps.push(CursorStep::Key(key));
                Ok(root)
            }
            Expression::ArrayAppend(ap) => {
                let root = self.collect_lvalue_steps(ap.array, steps)?;
                steps.push(CursorStep::Append);
                Ok(root)
            }
            Expression::Access(Access::Property(pa)) => {
                let root = self.collect_lvalue_steps(pa.object, steps)?;
                match self.compile_selector(&pa.property)? {
                    Sel::Static(name) => steps.push(CursorStep::Prop(name)),
                    Sel::Dyn(r) => steps.push(CursorStep::PropDyn(r)),
                }
                Ok(root)
            }
            Expression::Access(Access::StaticProperty(spa)) => {
                // Static property is the cursor root, not a step.
                let class = self.class_ref(spa.class)?;
                Ok(match self.static_prop_sel(&spa.property)? {
                    Sel::Static(name) => LvalRoot::StaticProp(class, name),
                    Sel::Dyn(name) => LvalRoot::StaticPropDyn(class, name),
                })
            }
            Expression::Variable(v) => Ok(LvalRoot::VarVar(self.var_name_reg(v)?)),
            Expression::Parenthesized(p) => self.collect_lvalue_steps(p.expression, steps),
            Expression::Access(Access::NullSafeProperty(_))
            | Expression::Call(rhp_frontend::cst::Call::NullSafeMethod(_)) => Err(Thrown::error(
                "Error",
                "Can't use nullsafe operator in write context",
            )),
            // `f()[$k] = v`, `A::g()->p[] = v`: php allows a call result as the
            // container (zend_delayed_compile_var on a call). The cursor roots
            // at the temporary; objects are handles, so writes through one land
            // in the shared object (an `ArrayAccess` root goes to `offsetSet`).
            Expression::Call(_) => Ok(LvalRoot::Cv(self.compile_expr(target)?)),
            _ => Err(Thrown::error(
                "Error",
                "Cannot use temporary expression in write context",
            )),
        }
    }

    fn emit_cursor(&mut self, root: LvalRoot, steps: &[CursorStep]) {
        match root {
            LvalRoot::Cv(cv) => self.emit(Instr::LvalCv { cv }),
            LvalRoot::StaticProp(class, name) => self.emit(Instr::LvalStaticProp { class, name }),
            LvalRoot::StaticPropDyn(class, name) => {
                self.emit(Instr::LvalStaticPropDyn { class, name })
            }
            LvalRoot::VarVar(name) => self.emit(Instr::LvalVarVar { name }),
            LvalRoot::Global(name) => self.emit(Instr::LvalGlobal { name }),
        }
        for step in steps {
            match step {
                CursorStep::Key(r) => self.emit(Instr::LvalKey { key: *r }),
                CursorStep::Append => self.emit(Instr::LvalAppend),
                CursorStep::Prop(n) => self.emit(Instr::LvalProp { name: *n }),
                CursorStep::PropDyn(r) => self.emit(Instr::LvalPropDyn { name: *r }),
            }
        }
    }

    // --- OOP expression lowering ---

    /// Resolve the class part of `new`/`::`/`instanceof` to a `ClassRef`.
    fn class_ref(&mut self, expr: &Expression) -> Result<ClassRef, Thrown> {
        match expr {
            Expression::Identifier(id) => {
                Ok(ClassRef::Name(self.intern_name(&resolve_class_name(id))))
            }
            Expression::Self_(_) => Ok(ClassRef::SelfClass),
            Expression::Static(_) => Ok(ClassRef::StaticClass),
            Expression::Parent(_) => Ok(ClassRef::ParentClass),
            other => Ok(ClassRef::Dynamic(self.compile_expr(other)?)),
        }
    }

    fn selector_name(
        &mut self,
        sel: &rhp_frontend::cst::ClassLikeMemberSelector,
    ) -> Result<NIdx, Thrown> {
        match sel {
            ClassLikeMemberSelector::Identifier(li) => Ok(self.intern_name(li.value)),
            _ => Err(Thrown::unsupported(
                "dynamic member name",
                "`->$x`/`->{expr}` deferred",
            )),
        }
    }

    /// Produce a register holding a `Ref` to a reference-assignment source: a
    /// simple variable's slot, or a property / array-element / static-prop lvalue.
    fn compile_ref_source(&mut self, src: &Expression) -> Result<Reg, Thrown> {
        if is_globals_var(src) {
            return Err(Thrown::error(
                "Error",
                "Cannot acquire reference to $GLOBALS",
            ));
        }
        check_globals_write(src)?;
        match src {
            Expression::Variable(Variable::Direct(v)) => {
                let name = strip_dollar(v.name);
                // `$t =& $this` must NOT let `$t = null` null out `$this` (php
                // protects $this): bind to a fresh ref cell holding the object,
                // not the $this slot itself. The object handle is still shared.
                if name == b"this" {
                    let obj = self.compile_expr(src)?;
                    let r = self.alloc_temp();
                    self.emit(Instr::MakeRefVal { dst: r, src: obj });
                    return Ok(r);
                }
                let cv = self.cv(name);
                let r = self.alloc_temp();
                self.emit(Instr::MakeRef { dst: r, cv });
                Ok(r)
            }
            Expression::ArrayAccess(_)
            | Expression::ArrayAppend(_)
            | Expression::Access(Access::Property(_))
            | Expression::Access(Access::StaticProperty(_)) => {
                let mut steps = Vec::new();
                let root = self.collect_lvalue_steps(src, &mut steps)?;
                self.emit_cursor(root, &steps);
                let r = self.alloc_temp();
                self.emit(Instr::MakeRefLval { dst: r });
                Ok(r)
            }
            // `$x = &func()` / other non-lvalue source: we don't implement
            // return-by-reference, so bind by value (matches php's fallback for a
            // non-ref-returning callee; object handles are shared regardless).
            other => self.compile_expr(other),
        }
    }

    /// A member selector as either a static interned name or a runtime name
    /// register (`$o->$x` / `$o->{expr}`).
    fn compile_selector(
        &mut self,
        sel: &rhp_frontend::cst::ClassLikeMemberSelector,
    ) -> Result<Sel, Thrown> {
        match sel {
            ClassLikeMemberSelector::Identifier(li) => Ok(Sel::Static(self.intern_name(li.value))),
            ClassLikeMemberSelector::Variable(Variable::Direct(d)) => {
                Ok(Sel::Dyn(self.cv(strip_dollar(d.name))))
            }
            ClassLikeMemberSelector::Expression(e) => {
                Ok(Sel::Dyn(self.compile_expr(e.expression)?))
            }
            _ => Err(Thrown::unsupported(
                "dynamic member name",
                "variable-variable member names deferred",
            )),
        }
    }

    /// The name of `C::$name` (static) or `C::${expr}` / `C::$$x` (runtime).
    fn static_prop_sel(&mut self, v: &Variable) -> Result<Sel, Thrown> {
        match v {
            Variable::Direct(d) => Ok(Sel::Static(self.intern_name(strip_dollar(d.name)))),
            other => Ok(Sel::Dyn(self.var_name_reg(other)?)),
        }
    }

    /// The register holding a variable-variable's *name*: `${expr}` is the
    /// expression's value, `$$x` the value of `$x` (recursively). For a direct
    /// `$x` (only reached as the root of `C::$x` lowering) the name itself.
    fn var_name_reg(&mut self, v: &Variable) -> Result<Reg, Thrown> {
        match v {
            Variable::Indirect(iv) => self.compile_expr(iv.expression),
            Variable::Nested(nv) => self.compile_variable(nv.variable),
            Variable::Direct(d) => Ok(self.load_string_const(strip_dollar(d.name))),
        }
    }

    /// A variable in value context: a CV, or a variable-variable read.
    fn compile_variable(&mut self, v: &Variable) -> Result<Reg, Thrown> {
        match v {
            Variable::Direct(d) => Ok(self.cv(strip_dollar(d.name))),
            other => {
                let name = self.var_name_reg(other)?;
                let dst = self.alloc_temp();
                self.emit(Instr::FetchVarVar { dst, name });
                Ok(dst)
            }
        }
    }

    /// Build a `PhpArray` of the call arguments (used when the call has a
    /// spread `...$x`): positional args are pushed, spreads are merged.
    fn build_args_array(
        &mut self,
        args: Option<&rhp_frontend::cst::ArgumentList>,
    ) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        self.emit(Instr::NewArray { dst, cap: 0 });
        if let Some(list) = args {
            for arg in list.arguments.iter() {
                match arg {
                    Argument::Positional(p) => {
                        let r = self.compile_expr(p.value)?;
                        if p.ellipsis.is_some() {
                            self.emit(Instr::ArgSpread { arr: dst, src: r });
                        } else {
                            self.emit(Instr::ArrPush { arr: dst, val: r });
                        }
                    }
                    // A named argument travels under its name (the callee's
                    // parameter list places it at run time).
                    Argument::Named(n) => {
                        let r = self.compile_expr(n.value)?;
                        let k = self.intern_const(Value::str(n.name.value.to_vec()));
                        let key = self.alloc_temp();
                        self.emit(Instr::LoadConst { dst: key, k });
                        self.emit(Instr::ArrSet {
                            arr: dst,
                            key,
                            val: r,
                        });
                    }
                }
            }
        }
        Ok(dst)
    }

    /// Marshal positional args into a contiguous register window.
    /// Compile a method/constructor argument list into a contiguous window.
    /// `ref_mask[i]` true means position `i` may be by-reference for some method
    /// of this name: if the argument is a simple variable, pass it as a `Ref`
    /// (by-value callees deref it in `bind_params`).
    /// `over_approx`: pass every simple-variable argument by reference (used for
    /// method / constructor calls, where the callee may live in another file so
    /// its by-ref signature is unknown at compile time; a by-value parameter just
    /// dereferences a copy, so this is safe and matches php's runtime by-ref).
    fn compile_arg_window_ex(
        &mut self,
        args: Option<&rhp_frontend::cst::ArgumentList>,
        ref_mask: &[bool],
        over_approx: bool,
    ) -> Result<(Reg, u16), Thrown> {
        self.compile_arg_window_t(args, ref_mask, over_approx, None)
    }

    /// [`compile_arg_window_ex`] for a call whose callee is resolved at run
    /// time: a property / array-element argument asks the callee (`target`)
    /// whether that parameter is by-reference (php's SEND_FUNC_ARG) and is
    /// then passed as a reference to the lvalue, else by value.
    fn compile_arg_window_t(
        &mut self,
        args: Option<&rhp_frontend::cst::ArgumentList>,
        ref_mask: &[bool],
        over_approx: bool,
        target: Option<ArgRefTarget>,
    ) -> Result<(Reg, u16), Thrown> {
        enum Slot {
            Value(Reg),
            Ref(Reg),
        }
        let mut slots = Vec::new();
        if let Some(list) = args {
            for (i, arg) in list.arguments.iter().enumerate() {
                let p = match arg {
                    Argument::Positional(p) if p.ellipsis.is_none() => p,
                    Argument::Positional(_) => {
                        return Err(Thrown::unsupported("argument spread", "deferred"))
                    }
                    Argument::Named(_) => {
                        return Err(Thrown::unsupported("named arguments", "deferred"))
                    }
                };
                if let Expression::Variable(Variable::Direct(d)) = p.value {
                    if over_approx || ref_mask.get(i).copied().unwrap_or(false) {
                        slots.push(Slot::Ref(self.cv(strip_dollar(d.name))));
                        continue;
                    }
                }
                if let Some(t) = target {
                    if matches!(
                        p.value,
                        Expression::ArrayAccess(_)
                            | Expression::Access(Access::Property(_))
                            | Expression::Access(Access::StaticProperty(_))
                    ) && variable_rooted_lvalue(p.value)
                    {
                        let r = self.compile_func_arg(p.value, t, i as u16)?;
                        slots.push(Slot::Ref(r));
                        continue;
                    }
                }
                let r = self.compile_expr(p.value)?;
                let r = self.snapshot_arg(list, i, p.value, r);
                slots.push(Slot::Value(r));
            }
        }
        let argc = slots.len() as u16;
        let base = self.alloc_temps(argc);
        for (i, slot) in slots.into_iter().enumerate() {
            let dst = base + i as Reg;
            match slot {
                Slot::Value(r) => self.emit(Instr::Mov { dst, src: r }),
                Slot::Ref(cv) => self.emit(Instr::MakeRef { dst, cv }),
            }
        }
        Ok((base, argc))
    }

    /// One SEND_FUNC_ARG argument: `$t = callee-param-is-ref ? &lvalue :
    /// lvalue` into a temp (the caller passes it on with `MakeRef`, which
    /// a by-value parameter dereferences).
    fn compile_func_arg(
        &mut self,
        e: &Expression,
        target: ArgRefTarget,
        idx: u16,
    ) -> Result<Reg, Thrown> {
        let flag = self.alloc_temp();
        match target {
            ArgRefTarget::Method(obj, name) => self.emit(Instr::MethodArgIsRef {
                dst: flag,
                obj,
                name,
                idx,
            }),
            ArgRefTarget::Static(class, name) => self.emit(Instr::StaticArgIsRef {
                dst: flag,
                class,
                name,
                idx,
            }),
        }
        let t = self.alloc_temp();
        let jf = self.code.len();
        self.emit(Instr::Nop); // JmpFalse flag -> by_value
        let mut steps = Vec::new();
        let root = self.collect_lvalue_steps(e, &mut steps)?;
        self.emit_cursor(root, &steps);
        self.emit(Instr::MakeRefLval { dst: t });
        let jend = self.code.len();
        self.emit(Instr::Nop); // Jmp -> end
        let by_value = self.here();
        self.code[jf] = Instr::JmpFalse {
            cond: flag,
            target: by_value,
        };
        let v = self.compile_expr(e)?;
        self.emit(Instr::Mov { dst: t, src: v });
        let end = self.here();
        self.code[jend] = Instr::Jmp { target: end };
        Ok(t)
    }

    /// A by-value argument that is a plain variable compiles to the variable's
    /// own register, which is only read when the argument window is filled
    /// after every argument ran. Snapshot it into a temp now when a later
    /// argument could change it (`f($m, g($m))` with a by-ref `g`): php
    /// evaluates arguments strictly left to right.
    fn snapshot_arg(
        &mut self,
        list: &rhp_frontend::cst::ArgumentList,
        i: usize,
        expr: &Expression,
        r: Reg,
    ) -> Reg {
        let later_may_write = list.arguments.iter().skip(i + 1).any(|a| {
            !matches!(
                a,
                Argument::Positional(q) if matches!(
                    q.value,
                    Expression::Literal(_) | Expression::Variable(Variable::Direct(_))
                )
            )
        });
        if !later_may_write || !matches!(expr, Expression::Variable(Variable::Direct(_))) {
            return r;
        }
        let t = self.alloc_temp();
        self.emit(Instr::Mov { dst: t, src: r });
        t
    }

    /// The by-ref mask for a method/constructor name (empty if none).
    fn method_ref_mask(&self, name_lc: &[u8]) -> Vec<bool> {
        METHOD_REF.with(|m| m.borrow().get(name_lc).cloned().unwrap_or_default())
    }

    /// The by-ref mask for an interned method name.
    fn name_ref_mask(&self, name: NIdx) -> Vec<bool> {
        let lc = self.names[name as usize].as_bytes().to_ascii_lowercase();
        self.method_ref_mask(&lc)
    }

    /// `new class(args) extends P implements I { … }`: declare the class under
    /// its generated name and instantiate it.
    fn compile_new_anon(&mut self, ac: &rhp_frontend::cst::AnonymousClass) -> Result<Reg, Thrown> {
        use rhp_frontend::cst::PartialArgument;
        let proto = compile_anon_class(ac, &self.sigs)?;
        let class = ClassRef::Name(self.intern_name(proto.name.as_bytes()));
        ANON_CLASSES.with(|c| c.borrow_mut().push(proto));
        let mut regs = Vec::new();
        if let Some(list) = &ac.argument_list {
            for arg in list.arguments.iter() {
                match arg {
                    PartialArgument::Positional(p) if p.ellipsis.is_none() => {
                        regs.push(self.compile_expr(p.value)?)
                    }
                    _ => {
                        return Err(Thrown::unsupported(
                            "anonymous class argument",
                            "spread / named / placeholder arguments deferred",
                        ))
                    }
                }
            }
        }
        let argc = regs.len() as u16;
        let base = self.alloc_temps(argc);
        for (i, r) in regs.into_iter().enumerate() {
            self.emit(Instr::Mov {
                dst: base + i as Reg,
                src: r,
            });
        }
        let dst = self.alloc_temp();
        self.emit(Instr::New {
            dst,
            class,
            base,
            argc,
        });
        Ok(dst)
    }

    fn compile_new(&mut self, inst: &rhp_frontend::cst::Instantiation) -> Result<Reg, Thrown> {
        if let Expression::AnonymousClass(ac) = inst.class {
            return self.compile_new_anon(ac);
        }
        let class = self.class_ref(inst.class)?;
        if inst
            .argument_list
            .as_ref()
            .is_some_and(|l| has_spread(l) || has_named(l))
        {
            let args = self.build_args_array(inst.argument_list.as_ref())?;
            let dst = self.alloc_temp();
            self.emit(Instr::NewV { dst, class, args });
            return Ok(dst);
        }
        let mask = self.method_ref_mask(b"__construct");
        let ctor = self.intern_name(b"__construct");
        let (base, argc) = self.compile_arg_window_t(
            inst.argument_list.as_ref(),
            &mask,
            true,
            Some(ArgRefTarget::Static(class, ctor)),
        )?;
        let dst = self.alloc_temp();
        self.emit(Instr::New {
            dst,
            class,
            base,
            argc,
        });
        Ok(dst)
    }

    /// Compile `e` in php's BP_VAR_IS mode (an isset / empty / `??` operand):
    /// the property, static-property and dim fetches along its container chain
    /// are quiet (an inaccessible or missing property is null, no access
    /// error, `__isset` consulted before `__get`). Keys, dynamic member names
    /// and anything that is not a fetch compile normally.
    fn compile_quiet(&mut self, e: &Expression) -> Result<Reg, Thrown> {
        match e {
            Expression::Parenthesized(p) => self.compile_quiet(p.expression),
            Expression::ArrayAccess(aa) => {
                let arr = self.compile_quiet(aa.array)?;
                let key = self.compile_expr(aa.index)?;
                let dst = self.alloc_temp();
                self.emit(Instr::DimFetchIs { dst, arr, key });
                Ok(dst)
            }
            Expression::Access(Access::Property(pa)) => {
                let obj = self.compile_quiet(pa.object)?;
                let dst = self.alloc_temp();
                match self.compile_selector(&pa.property)? {
                    Sel::Static(name) => self.emit(Instr::FetchPropIs { dst, obj, name }),
                    Sel::Dyn(name) => self.emit(Instr::FetchPropDynIs { dst, obj, name }),
                }
                Ok(dst)
            }
            Expression::Access(Access::NullSafeProperty(npa)) => {
                let obj = self.compile_quiet(npa.object)?;
                let dst = self.alloc_temp();
                self.emit(Instr::Mov { dst, src: obj });
                let jn = self.code.len();
                self.emit(Instr::Nop); // JmpNull dst -> end
                let t = self.alloc_temp();
                match self.compile_selector(&npa.property)? {
                    Sel::Static(name) => self.emit(Instr::FetchPropIs {
                        dst: t,
                        obj: dst,
                        name,
                    }),
                    Sel::Dyn(name) => self.emit(Instr::FetchPropDynIs {
                        dst: t,
                        obj: dst,
                        name,
                    }),
                }
                self.emit(Instr::Mov { dst, src: t });
                let end = self.here();
                self.code[jn] = Instr::JmpNull {
                    src: dst,
                    target: end,
                };
                Ok(dst)
            }
            Expression::Access(Access::StaticProperty(spa)) => {
                let class = self.class_ref(spa.class)?;
                let dst = self.alloc_temp();
                match self.static_prop_sel(&spa.property)? {
                    Sel::Static(name) => self.emit(Instr::FetchStaticPropIs { dst, class, name }),
                    Sel::Dyn(name) => self.emit(Instr::FetchStaticPropDynIs { dst, class, name }),
                }
                Ok(dst)
            }
            _ => self.compile_expr(e),
        }
    }

    fn compile_access(&mut self, acc: &Access) -> Result<Reg, Thrown> {
        match acc {
            Access::Property(pa) => {
                let obj = self.compile_expr(pa.object)?;
                let dst = self.alloc_temp();
                match self.compile_selector(&pa.property)? {
                    Sel::Static(name) => {
                        self.emit(Instr::FetchProp { dst, obj, name });
                    }
                    Sel::Dyn(name) => self.emit(Instr::FetchPropDyn { dst, obj, name }),
                }
                Ok(dst)
            }
            Access::NullSafeProperty(npa) => {
                // `$o?->p`: null short-circuits the whole access to null.
                let obj = self.compile_expr(npa.object)?;
                let dst = self.alloc_temp();
                self.emit(Instr::Mov { dst, src: obj });
                let jn = self.code.len();
                self.emit(Instr::Nop); // JmpNull dst -> Lnull
                let name = self.selector_name(&npa.property)?;
                let t = self.alloc_temp();
                self.emit(Instr::FetchProp {
                    dst: t,
                    obj: dst,
                    name,
                });
                self.emit(Instr::Mov { dst, src: t });
                let jend = self.code.len();
                self.emit(Instr::Nop); // Jmp end
                let lnull = self.here();
                self.emit(Instr::LoadNull { dst });
                let end = self.here();
                self.code[jn] = Instr::JmpNull {
                    src: dst,
                    target: lnull,
                };
                self.code[jend] = Instr::Jmp { target: end };
                Ok(dst)
            }
            Access::StaticProperty(spa) => {
                let class = self.class_ref(spa.class)?;
                let dst = self.alloc_temp();
                match self.static_prop_sel(&spa.property)? {
                    Sel::Static(name) => {
                        self.emit(Instr::FetchStaticProp { dst, class, name });
                    }
                    Sel::Dyn(name) => self.emit(Instr::FetchStaticPropDyn { dst, class, name }),
                }
                Ok(dst)
            }
            Access::ClassConstant(cca) => {
                // `Name::class` is a COMPILE-TIME fully-qualified-name string — the
                // class need not exist (php uses `Foo::class` as a bareword FQN,
                // e.g. `function_exists(u::class)`). `self/static/parent::class`
                // and `$obj::class` stay runtime.
                if let rhp_frontend::cst::ClassLikeConstantSelector::Identifier(li) = &cca.constant
                {
                    if li.value.eq_ignore_ascii_case(b"class") {
                        if let Expression::Identifier(id) = cca.class {
                            let k = self.intern_const(Value::str(resolve_class_name(id)));
                            let dst = self.alloc_temp();
                            self.emit(Instr::LoadConst { dst, k });
                            return Ok(dst);
                        }
                    }
                }
                let class = self.class_ref(cca.class)?;
                let name = match &cca.constant {
                    rhp_frontend::cst::ClassLikeConstantSelector::Identifier(li) => {
                        self.intern_name(li.value)
                    }
                    _ => return Err(Thrown::unsupported("dynamic class constant", "deferred")),
                };
                let dst = self.alloc_temp();
                self.emit(Instr::FetchClassConst { dst, class, name });
                Ok(dst)
            }
        }
    }

    fn compile_method_call(
        &mut self,
        object: &Expression,
        selector: &rhp_frontend::cst::ClassLikeMemberSelector,
        args: &rhp_frontend::cst::ArgumentList,
        null_safe: bool,
    ) -> Result<Reg, Thrown> {
        let obj = self.compile_expr(object)?;
        if !null_safe {
            match self.compile_selector(selector)? {
                Sel::Static(name) => {
                    if has_spread(args) || has_named(args) {
                        let args = self.build_args_array(Some(args))?;
                        let dst = self.alloc_temp();
                        self.emit(Instr::CallMethodV {
                            dst,
                            obj,
                            name,
                            args,
                        });
                        return Ok(dst);
                    }
                    let mask = self.name_ref_mask(name);
                    let (base, argc) = self.compile_arg_window_t(
                        Some(args),
                        &mask,
                        true,
                        Some(ArgRefTarget::Method(obj, name)),
                    )?;
                    let dst = self.alloc_temp();
                    self.emit(Instr::CallMethod {
                        dst,
                        obj,
                        name,
                        base,
                        argc,
                    });
                    return Ok(dst);
                }
                Sel::Dyn(name) => {
                    if has_spread(args) || has_named(args) {
                        let args = self.build_args_array(Some(args))?;
                        let dst = self.alloc_temp();
                        self.emit(Instr::CallMethodDynV {
                            dst,
                            obj,
                            name,
                            args,
                        });
                        return Ok(dst);
                    }
                    let (base, argc) = self.compile_arg_window_ex(Some(args), &[], true)?;
                    let dst = self.alloc_temp();
                    self.emit(Instr::CallMethodDyn {
                        dst,
                        obj,
                        name,
                        base,
                        argc,
                    });
                    return Ok(dst);
                }
            }
        }
        // `$o?->m(...)`: if $o is null, the whole call is null and args are not
        // evaluated (the guard jumps over both the arg marshalling and the call).
        let dst = self.alloc_temp();
        self.emit(Instr::Mov { dst, src: obj });
        let jn = self.code.len();
        self.emit(Instr::Nop); // JmpNull dst -> Lnull
        let sel = self.compile_selector(selector)?;
        let t = self.alloc_temp();
        if has_spread(args) || has_named(args) {
            let args = self.build_args_array(Some(args))?;
            match sel {
                Sel::Static(name) => self.emit(Instr::CallMethodV {
                    dst: t,
                    obj: dst,
                    name,
                    args,
                }),
                Sel::Dyn(name) => self.emit(Instr::CallMethodDynV {
                    dst: t,
                    obj: dst,
                    name,
                    args,
                }),
            }
        } else {
            match sel {
                Sel::Static(name) => {
                    let mask = self.name_ref_mask(name);
                    let (base, argc) = self.compile_arg_window_t(
                        Some(args),
                        &mask,
                        true,
                        Some(ArgRefTarget::Method(dst, name)),
                    )?;
                    self.emit(Instr::CallMethod {
                        dst: t,
                        obj: dst,
                        name,
                        base,
                        argc,
                    });
                }
                Sel::Dyn(name) => {
                    let (base, argc) = self.compile_arg_window_ex(Some(args), &[], true)?;
                    self.emit(Instr::CallMethodDyn {
                        dst: t,
                        obj: dst,
                        name,
                        base,
                        argc,
                    });
                }
            }
        }
        self.emit(Instr::Mov { dst, src: t });
        let jend = self.code.len();
        self.emit(Instr::Nop); // Jmp end
        let lnull = self.here();
        self.emit(Instr::LoadNull { dst });
        let end = self.here();
        self.code[jn] = Instr::JmpNull {
            src: dst,
            target: lnull,
        };
        self.code[jend] = Instr::Jmp { target: end };
        Ok(dst)
    }

    fn compile_static_call(
        &mut self,
        smc: &rhp_frontend::cst::StaticMethodCall,
    ) -> Result<Reg, Thrown> {
        let class = self.class_ref(smc.class)?;
        match self.compile_selector(&smc.method)? {
            Sel::Static(name) => {
                if has_spread(&smc.argument_list) || has_named(&smc.argument_list) {
                    let args = self.build_args_array(Some(&smc.argument_list))?;
                    let dst = self.alloc_temp();
                    self.emit(Instr::CallStaticV {
                        dst,
                        class,
                        name,
                        args,
                    });
                    return Ok(dst);
                }
                let mask = self.name_ref_mask(name);
                let (base, argc) = self.compile_arg_window_t(
                    Some(&smc.argument_list),
                    &mask,
                    true,
                    Some(ArgRefTarget::Static(class, name)),
                )?;
                let dst = self.alloc_temp();
                self.emit(Instr::CallStatic {
                    dst,
                    class,
                    name,
                    base,
                    argc,
                });
                Ok(dst)
            }
            Sel::Dyn(name) => {
                if has_spread(&smc.argument_list) || has_named(&smc.argument_list) {
                    let args = self.build_args_array(Some(&smc.argument_list))?;
                    let dst = self.alloc_temp();
                    self.emit(Instr::CallStaticDynV {
                        dst,
                        class,
                        name,
                        args,
                    });
                    return Ok(dst);
                }
                let (base, argc) =
                    self.compile_arg_window_ex(Some(&smc.argument_list), &[], true)?;
                let dst = self.alloc_temp();
                self.emit(Instr::CallStaticDyn {
                    dst,
                    class,
                    name,
                    base,
                    argc,
                });
                Ok(dst)
            }
        }
    }

    fn compile_assignment(&mut self, a: &rhp_frontend::cst::Assignment) -> Result<Reg, Thrown> {
        check_globals_write(a.lhs)?;
        // Destructuring: `[$a, $b] = ...` / `list($a, $b) = ...`.
        if matches!(a.operator, AssignmentOperator::Assign(_)) {
            if let Expression::Array(arr) = a.lhs {
                let rhs = self.compile_expr(a.rhs)?;
                self.destructure_into(arr.elements.iter(), rhs)?;
                return Ok(rhs);
            }
            if let Expression::List(lst) = a.lhs {
                let rhs = self.compile_expr(a.rhs)?;
                self.destructure_into(lst.elements.iter(), rhs)?;
                return Ok(rhs);
            }
        }

        // Reference assignment `$b =& $a` — rhs is `&<lvalue>`.
        if let (AssignmentOperator::Assign(_), Expression::UnaryPrefix(u)) = (&a.operator, a.rhs) {
            if matches!(u.operator, UnaryPrefixOperator::Reference(_)) {
                // `$dst =& $src` between simple variables: alias the slots.
                // Exception: `$dst =& $this` must not alias the $this slot (php
                // protects $this), so it goes through compile_ref_source below,
                // which binds a fresh ref cell holding the object.
                if let (
                    Expression::Variable(Variable::Direct(dst)),
                    Expression::Variable(Variable::Direct(src)),
                ) = (a.lhs, u.operand)
                {
                    if strip_dollar(src.name) == b"GLOBALS" {
                        return Err(Thrown::error(
                            "Error",
                            "Cannot acquire reference to $GLOBALS",
                        ));
                    }
                    if strip_dollar(src.name) != b"this" {
                        let dst_cv = self.cv(strip_dollar(dst.name));
                        let src_cv = self.cv(strip_dollar(src.name));
                        self.emit(Instr::BindRef { dst_cv, src_cv });
                        return Ok(dst_cv);
                    }
                }
                // `$dst =& <lvalue>` (property / array element / static prop): take
                // a reference to the source lvalue and alias the destination var.
                if let Expression::Variable(Variable::Direct(dst)) = a.lhs {
                    let r = self.compile_ref_source(u.operand)?;
                    let dst_cv = self.cv(strip_dollar(dst.name));
                    self.emit(Instr::RefAssign { dst_cv, src: r });
                    return Ok(dst_cv);
                }
                // `$$n =& <src>`: alias the variable named at run time.
                if let Expression::Variable(var) = a.lhs {
                    let name = self.var_name_reg(var)?;
                    let r = self.compile_ref_source(u.operand)?;
                    self.emit(Instr::BindVarVarRef { name, src: r });
                    return Ok(r);
                }
                // `<lvalue> =& <src>`: make the destination element/property a
                // reference to the source.
                if matches!(
                    a.lhs,
                    Expression::ArrayAccess(_)
                        | Expression::ArrayAppend(_)
                        | Expression::Access(Access::Property(_))
                        | Expression::Access(Access::StaticProperty(_))
                ) {
                    let r = self.compile_ref_source(u.operand)?;
                    let mut steps = Vec::new();
                    let root = self.collect_lvalue_steps(a.lhs, &mut steps)?;
                    self.emit_cursor(root, &steps);
                    self.emit(Instr::AssignLvalRef { src: r });
                    return Ok(r);
                }
                return Err(Thrown::unsupported(
                    "reference binding target",
                    "unsupported `=&` source/target form",
                ));
            }
        }

        if matches!(a.operator, AssignmentOperator::Coalesce(_)) {
            return self.compile_coalesce_assign(a.lhs, a.rhs);
        }

        // Simple single-level `$o->prop = v`: a dedicated opcode so the VM can
        // route undefined/inaccessible writes to `__set` (the cursor can't).
        if let (AssignmentOperator::Assign(_), Expression::Access(Access::Property(pa))) =
            (&a.operator, a.lhs)
        {
            if let ClassLikeMemberSelector::Identifier(li) = &pa.property {
                let obj = self.compile_expr(pa.object)?;
                let name = self.intern_name(li.value);
                let rhs = self.compile_expr(a.rhs)?;
                self.emit(Instr::StoreProp {
                    obj,
                    name,
                    src: rhs,
                });
                return Ok(rhs);
            }
            // Dynamic `$o->$x = v` / `$o->{expr} = v`.
            if matches!(
                &pa.property,
                ClassLikeMemberSelector::Variable(_) | ClassLikeMemberSelector::Expression(_)
            ) {
                let obj = self.compile_expr(pa.object)?;
                let name = match self.compile_selector(&pa.property)? {
                    Sel::Dyn(r) => r,
                    Sel::Static(_) => unreachable!(),
                };
                let rhs = self.compile_expr(a.rhs)?;
                self.emit(Instr::StorePropDyn {
                    obj,
                    name,
                    src: rhs,
                });
                return Ok(rhs);
            }
        }

        // Array-element / property / static-property / variable-variable
        // target: use the lvalue cursor.
        if matches!(
            a.lhs,
            Expression::ArrayAccess(_)
                | Expression::ArrayAppend(_)
                | Expression::Access(Access::Property(_))
                | Expression::Access(Access::StaticProperty(_))
                | Expression::Variable(Variable::Indirect(_) | Variable::Nested(_))
        ) {
            let mut steps = Vec::new();
            let root = self.collect_lvalue_steps(a.lhs, &mut steps)?;
            let rhs = self.compile_expr(a.rhs)?;
            return match &a.operator {
                AssignmentOperator::Assign(_) => {
                    self.emit_cursor(root, &steps);
                    self.emit(Instr::AssignLval { src: rhs });
                    Ok(rhs)
                }
                op => {
                    self.emit_cursor(root, &steps);
                    let cur = self.alloc_temp();
                    self.emit(Instr::FetchLvalRw { dst: cur });
                    let res = self.alloc_temp();
                    self.emit(compound_op(op, res, cur, rhs, lvalue_desc(a.lhs))?);
                    self.emit(Instr::AssignLval { src: res });
                    Ok(res)
                }
            };
        }

        let Expression::Variable(Variable::Direct(d)) = a.lhs else {
            return Err(Thrown::unsupported(
                "assignment target",
                "property targets arrive with OOP",
            ));
        };
        let cv = self.cv(strip_dollar(d.name));
        let rhs = self.compile_expr(a.rhs)?;

        let value_reg = match &a.operator {
            AssignmentOperator::Assign(_) => rhs,
            op => {
                // compound: dst = cv <op> rhs
                let dst = self.alloc_temp();
                let instr = match op {
                    AssignmentOperator::Addition(_) => Instr::Add { dst, a: cv, b: rhs },
                    AssignmentOperator::Subtraction(_) => Instr::Sub { dst, a: cv, b: rhs },
                    AssignmentOperator::Multiplication(_) => Instr::Mul { dst, a: cv, b: rhs },
                    AssignmentOperator::Division(_) => Instr::Div { dst, a: cv, b: rhs },
                    AssignmentOperator::Modulo(_) => Instr::Mod { dst, a: cv, b: rhs },
                    AssignmentOperator::Exponentiation(_) => Instr::Pow { dst, a: cv, b: rhs },
                    AssignmentOperator::Concat(_) => Instr::Concat { dst, a: cv, b: rhs },
                    AssignmentOperator::BitwiseAnd(_) => Instr::BitAnd { dst, a: cv, b: rhs },
                    AssignmentOperator::BitwiseOr(_) => Instr::BitOr { dst, a: cv, b: rhs },
                    AssignmentOperator::BitwiseXor(_) => Instr::BitXor { dst, a: cv, b: rhs },
                    AssignmentOperator::LeftShift(_) => Instr::Shl { dst, a: cv, b: rhs },
                    AssignmentOperator::RightShift(_) => Instr::Shr { dst, a: cv, b: rhs },
                    AssignmentOperator::Coalesce(_) => {
                        // $x ??= rhs : assign only if cv is null
                        self.emit(Instr::Mov { dst, src: cv });
                        let jnn = self.code.len();
                        self.emit(Instr::Nop);
                        self.emit(Instr::Mov { dst, src: rhs });
                        let end = self.code.len() as u32;
                        self.code[jnn] = Instr::JmpNotNull {
                            src: dst,
                            target: end,
                        };
                        self.emit(Instr::Assign { cv, src: dst });
                        return Ok(dst);
                    }
                    AssignmentOperator::Assign(_) => unreachable!(),
                };
                self.emit(instr);
                dst
            }
        };
        self.emit(Instr::Assign { cv, src: value_reg });
        Ok(value_reg)
    }

    /// `lhs ??= rhs` (php's ZEND_ASSIGN_COALESCE): fetch `lhs` quietly (isset
    /// semantics: no undefined-key/property diagnostics, `ArrayAccess` via
    /// `offsetExists`/`offsetGet`, `__isset`/`__get`); when it is not null that
    /// is the result and nothing is written or evaluated further. Otherwise
    /// evaluate `rhs` and assign it. Sub-expressions of `lhs` (keys, objects)
    /// are evaluated exactly once.
    fn compile_coalesce_assign(
        &mut self,
        lhs: &Expression,
        rhs: &Expression,
    ) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        match lhs {
            Expression::Parenthesized(p) => return self.compile_coalesce_assign(p.expression, rhs),
            Expression::Variable(Variable::Direct(d)) => {
                let cv = self.cv(strip_dollar(d.name));
                self.emit(Instr::Mov { dst, src: cv });
                let jnn = self.code.len();
                self.emit(Instr::Nop); // JmpNotNull dst -> end
                let r = self.compile_expr(rhs)?;
                self.emit(Instr::Assign { cv, src: r });
                self.emit(Instr::Mov { dst, src: cv });
                let end = self.here();
                self.code[jnn] = Instr::JmpNotNull {
                    src: dst,
                    target: end,
                };
            }
            // Single-level `$o->p ??= v`: the write is a plain property store,
            // so an inaccessible/undefined property routes to `__set`.
            // The container is fetched quietly too (php compiles the whole
            // left side in BP_VAR_IS mode for the check).
            Expression::Access(Access::Property(pa)) => {
                let obj = self.compile_quiet(pa.object)?;
                let sel = self.compile_selector(&pa.property)?;
                match sel {
                    Sel::Static(name) => self.emit(Instr::FetchPropIs { dst, obj, name }),
                    Sel::Dyn(name) => self.emit(Instr::FetchPropDynIs { dst, obj, name }),
                }
                let jnn = self.code.len();
                self.emit(Instr::Nop);
                let r = self.compile_expr(rhs)?;
                match sel {
                    Sel::Static(name) => self.emit(Instr::StoreProp { obj, name, src: r }),
                    Sel::Dyn(name) => self.emit(Instr::StorePropDyn { obj, name, src: r }),
                }
                self.emit(Instr::Mov { dst, src: r });
                let end = self.here();
                self.code[jnn] = Instr::JmpNotNull {
                    src: dst,
                    target: end,
                };
            }
            _ => {
                let mut steps = Vec::new();
                let root = self.collect_lvalue_steps(lhs, &mut steps)?;
                // Quiet read along the same (pre-evaluated) path.
                let mut cur = match &root {
                    LvalRoot::Cv(r) => *r,
                    LvalRoot::StaticProp(class, name) => {
                        let t = self.alloc_temp();
                        self.emit(Instr::FetchStaticPropIs {
                            dst: t,
                            class: *class,
                            name: *name,
                        });
                        t
                    }
                    LvalRoot::StaticPropDyn(class, name) => {
                        let t = self.alloc_temp();
                        self.emit(Instr::FetchStaticPropDynIs {
                            dst: t,
                            class: *class,
                            name: *name,
                        });
                        t
                    }
                    LvalRoot::VarVar(name) => {
                        let t = self.alloc_temp();
                        self.emit(Instr::FetchVarVar {
                            dst: t,
                            name: *name,
                        });
                        t
                    }
                    // `$GLOBALS[$k] ??= v`: a quiet read of the global.
                    LvalRoot::Global(name) => {
                        let all = self.alloc_temp();
                        self.emit(Instr::FetchGlobals { dst: all });
                        let t = self.alloc_temp();
                        self.emit(Instr::DimFetchIs {
                            dst: t,
                            arr: all,
                            key: *name,
                        });
                        t
                    }
                };
                for step in &steps {
                    let t = self.alloc_temp();
                    match step {
                        CursorStep::Key(key) => self.emit(Instr::DimFetchIs {
                            dst: t,
                            arr: cur,
                            key: *key,
                        }),
                        CursorStep::Prop(name) => self.emit(Instr::FetchPropIs {
                            dst: t,
                            obj: cur,
                            name: *name,
                        }),
                        CursorStep::PropDyn(name) => self.emit(Instr::FetchPropDynIs {
                            dst: t,
                            obj: cur,
                            name: *name,
                        }),
                        CursorStep::Append => {
                            return Err(Thrown::error("Error", "Cannot use [] for reading"))
                        }
                    }
                    cur = t;
                }
                self.emit(Instr::Mov { dst, src: cur });
                let jnn = self.code.len();
                self.emit(Instr::Nop);
                let r = self.compile_expr(rhs)?;
                self.emit_cursor(root, &steps);
                self.emit(Instr::AssignLval { src: r });
                self.emit(Instr::Mov { dst, src: r });
                let end = self.here();
                self.code[jnn] = Instr::JmpNotNull {
                    src: dst,
                    target: end,
                };
            }
        }
        Ok(dst)
    }
}

// --- free helpers ---------------------------------------------------------

/// A php compile-time fatal error (E_COMPILE_ERROR) at `line`.
fn compile_error(msg: impl Into<String>, _line: u32) -> Thrown {
    Thrown::error("Error", msg)
}

fn strip_dollar(name: &[u8]) -> &[u8] {
    name.strip_prefix(b"$").unwrap_or(name)
}

/// The stub signature of a builtin rhp implements: a native one, or a
/// function of the prelude.
fn builtin_sig(name_lc: &[u8]) -> Option<&'static rhp_builtins::signatures::Func> {
    let f = rhp_builtins::signatures::function(name_lc)?;
    let name = std::str::from_utf8(name_lc).ok()?;
    if rhp_builtins::registry().contains_key(name) {
        return Some(f);
    }
    // While the prelude itself compiles, every function it calls exists.
    let in_prelude = PRELUDE_CACHE.with(|c| match &*c.borrow() {
        Some(p) => p
            .functions
            .iter()
            .any(|pf| pf.name.as_bytes().eq_ignore_ascii_case(name_lc)),
        None => true,
    });
    in_prelude.then_some(f)
}

/// By-reference flags for the first `argc` arguments of a builtin call, from
/// its stub signature. The call site emits `MakeRef` for each flagged
/// position.
fn builtin_by_ref(name_lc: &[u8], argc: usize) -> Option<Vec<bool>> {
    use rhp_builtins::signatures::Signature;
    if !rhp_builtins::signatures::function(name_lc)?.has_by_ref() {
        return None;
    }
    let f = builtin_sig(name_lc)?;
    Some((0..argc.max(f.params.len())).map(|i| f.by_ref(i)).collect())
}

/// Does a function body contain a top-level `yield` (making it a generator)?
/// Does not descend into nested function-likes (their yields are their own).
fn block_has_yield(stmts: &rhp_frontend::cst::Sequence<Statement>) -> bool {
    stmts.iter().any(stmt_has_yield)
}

fn stmt_has_yield(s: &Statement) -> bool {
    node_has_yield(Node::Statement(s))
}

fn expr_has_yield(e: &Expression) -> bool {
    node_has_yield(Node::Expression(e))
}

/// Any `yield` / `yield from` below `node`, not counting nested function-likes
/// and class bodies (their yields are their own).
fn node_has_yield(node: Node) -> bool {
    match node {
        Node::Expression(Expression::Yield(_)) => return true,
        Node::Closure(_)
        | Node::ArrowFunction(_)
        | Node::Function(_)
        | Node::Class(_)
        | Node::Interface(_)
        | Node::Trait(_)
        | Node::Enum(_)
        | Node::AttributeList(_) => return false,
        Node::AnonymousClass(ac) => {
            return ac
                .argument_list
                .as_ref()
                .is_some_and(|l| node_has_yield(Node::PartialArgumentList(l)));
        }
        _ => {}
    }
    let mut found = false;
    node.visit_children(|ch| found = found || node_has_yield(ch));
    found
}

fn has_named(list: &rhp_frontend::cst::ArgumentList) -> bool {
    list.arguments
        .iter()
        .any(|a| matches!(a, Argument::Named(_)))
}

fn has_spread(list: &rhp_frontend::cst::ArgumentList) -> bool {
    list.arguments
        .iter()
        .any(|a| matches!(a, Argument::Positional(p) if p.ellipsis.is_some()))
}

/// What an assignment target is, for diagnostics ("an array element", …).
fn lvalue_desc(e: &Expression) -> &'static str {
    match e {
        Expression::ArrayAccess(_) | Expression::ArrayAppend(_) => "an array element",
        Expression::Access(Access::Property(_)) => "a property",
        Expression::Access(Access::NullSafeProperty(_)) => "a nullsafe property",
        Expression::Access(Access::StaticProperty(_)) => "a static property",
        Expression::Variable(_) => "a variable",
        _ => "this target",
    }
}

/// Build the instruction for a compound-assignment operator `dst = a <op> b`;
/// `target` describes the assigned lvalue (see [`lvalue_desc`]).
fn compound_op(
    op: &AssignmentOperator,
    dst: Reg,
    a: Reg,
    b: Reg,
    target: &str,
) -> Result<Instr, Thrown> {
    use AssignmentOperator as Ao;
    Ok(match op {
        Ao::Addition(_) => Instr::Add { dst, a, b },
        Ao::Subtraction(_) => Instr::Sub { dst, a, b },
        Ao::Multiplication(_) => Instr::Mul { dst, a, b },
        Ao::Division(_) => Instr::Div { dst, a, b },
        Ao::Modulo(_) => Instr::Mod { dst, a, b },
        Ao::Exponentiation(_) => Instr::Pow { dst, a, b },
        Ao::Concat(_) => Instr::Concat { dst, a, b },
        Ao::BitwiseAnd(_) => Instr::BitAnd { dst, a, b },
        Ao::BitwiseOr(_) => Instr::BitOr { dst, a, b },
        Ao::BitwiseXor(_) => Instr::BitXor { dst, a, b },
        Ao::LeftShift(_) => Instr::Shl { dst, a, b },
        Ao::RightShift(_) => Instr::Shr { dst, a, b },
        Ao::Coalesce(_) => {
            return Err(Thrown::unsupported(
                format!("??= on {target}"),
                "use a full assignment for now",
            ))
        }
        Ao::Assign(_) => unreachable!("handled by caller"),
    })
}

fn identifier_name<'a>(id: &'a Identifier<'a>) -> &'a [u8] {
    let raw = match id {
        Identifier::Local(l) => l.value,
        Identifier::Qualified(q) => q.value,
        Identifier::FullyQualified(f) => f.value,
    };
    match raw.iter().rposition(|&b| b == b'\\') {
        Some(pos) => &raw[pos + 1..],
        None => raw,
    }
}

/// Fold a compile-time-constant expression (literals, const arrays, true/false/
/// null, negated numeric literals) to a `Value`. Errors on anything requiring
/// runtime evaluation (named constants, class constants, arithmetic).
fn fold_const(e: &Expression) -> Result<Value, Thrown> {
    let v = match e {
        Expression::Literal(l) => literal_value(l),
        Expression::Parenthesized(p) => return fold_const(p.expression),
        // Context-free magic constants fold in const / default-value positions
        // (`const ROOT = __DIR__;`, `function f($p = __FILE__)`). The scope-
        // dependent ones (__CLASS__/__FUNCTION__/__METHOD__) are handled by the
        // ProtoBuilder path, not here.
        Expression::MagicConstant(mc) => {
            use mago_span::HasSpan;
            match mc {
                MagicConstant::Line(_) => Value::Int(line_of(mc.span().start.offset) as i64),
                MagicConstant::File(_) => Value::str(src_file()),
                MagicConstant::Directory(_) => Value::str(dirname(&src_file())),
                MagicConstant::Namespace(_) => Value::str(cur_ns()),
                _ => return Err(non_const_err()),
            }
        }
        Expression::Array(arr) => return fold_const_array(arr.elements.iter()),
        Expression::LegacyArray(arr) => return fold_const_array(arr.elements.iter()),
        Expression::ConstantAccess(ca) => match identifier_name(&ca.name) {
            b"true" | b"TRUE" | b"True" => Value::Bool(true),
            b"false" | b"FALSE" | b"False" => Value::Bool(false),
            b"null" | b"NULL" | b"Null" => Value::Null,
            _ => return Err(non_const_err()),
        },
        Expression::UnaryPrefix(u) => {
            use UnaryPrefixOperator as Op;
            let inner = fold_const(u.operand)?;
            match (&u.operator, inner) {
                (Op::Negation(_), Value::Int(n)) => Value::Int(-n),
                (Op::Negation(_), Value::Float(f)) => Value::Float(-f),
                (Op::Plus(_), v @ (Value::Int(_) | Value::Float(_))) => v,
                (Op::BitwiseNot(_), v) => rhp_semantics::ops::bitwise_not(&v),
                (Op::Not(_), v) => Value::Bool(!rhp_values::conv::to_bool(&v)),
                _ => return Err(non_const_err()),
            }
        }
        Expression::Binary(b) => {
            use rhp_frontend::cst::BinaryOperator as Bop;
            use rhp_semantics::ops::{self, BitOp};
            let l = fold_const(b.lhs)?;
            let r = fold_const(b.rhs)?;
            match &b.operator {
                Bop::Addition(_) => ops::add(&l, &r)?,
                Bop::Subtraction(_) => ops::sub(&l, &r)?,
                Bop::Multiplication(_) => ops::mul(&l, &r)?,
                Bop::Division(_) => ops::div(&l, &r)?,
                Bop::Modulo(_) => ops::modulo(&l, &r)?,
                Bop::Exponentiation(_) => ops::pow(&l, &r)?,
                Bop::StringConcat(_) => ops::concat(&l, &r),
                Bop::BitwiseAnd(_) => ops::bitwise(&l, &r, BitOp::And),
                Bop::BitwiseOr(_) => ops::bitwise(&l, &r, BitOp::Or),
                Bop::BitwiseXor(_) => ops::bitwise(&l, &r, BitOp::Xor),
                Bop::LeftShift(_) => ops::bitwise(&l, &r, BitOp::Shl),
                Bop::RightShift(_) => ops::bitwise(&l, &r, BitOp::Shr),
                Bop::Equal(_) => Value::Bool(ops::loose_eq(&l, &r)),
                Bop::NotEqual(_) | Bop::AngledNotEqual(_) => Value::Bool(!ops::loose_eq(&l, &r)),
                Bop::Identical(_) => Value::Bool(ops::strict_eq(&l, &r)),
                Bop::NotIdentical(_) => Value::Bool(!ops::strict_eq(&l, &r)),
                Bop::LessThan(_) => Value::Bool(ops::loose_compare(&l, &r).is_lt()),
                Bop::LessThanOrEqual(_) => Value::Bool(ops::loose_compare(&l, &r).is_le()),
                Bop::GreaterThan(_) => Value::Bool(ops::loose_compare(&r, &l).is_lt()),
                Bop::GreaterThanOrEqual(_) => Value::Bool(ops::loose_compare(&r, &l).is_le()),
                Bop::Spaceship(_) => ops::spaceship(&l, &r),
                Bop::And(_) | Bop::LowAnd(_) => {
                    Value::Bool(rhp_values::conv::to_bool(&l) && rhp_values::conv::to_bool(&r))
                }
                Bop::Or(_) | Bop::LowOr(_) => {
                    Value::Bool(rhp_values::conv::to_bool(&l) || rhp_values::conv::to_bool(&r))
                }
                Bop::NullCoalesce(_) => {
                    if matches!(l, Value::Null) {
                        r
                    } else {
                        l
                    }
                }
                _ => return Err(non_const_err()),
            }
        }
        _ => return Err(non_const_err()),
    };
    Ok(v)
}

fn fold_const_array<'a>(els: impl Iterator<Item = &'a ArrayElement<'a>>) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    for el in els {
        match el {
            ArrayElement::KeyValue(kv) => {
                let k = fold_const(kv.key)?;
                let v = fold_const(kv.value)?;
                let key = ArrayKey::from_value(&k)
                    .ok_or_else(|| Thrown::type_error("Illegal offset type"))?;
                a.insert(key, v);
            }
            ArrayElement::Value(v) => a.push(fold_const(v.value)?),
            ArrayElement::Variadic(_) => return Err(non_const_err()),
            ArrayElement::Missing(_) => {}
        }
    }
    Ok(Value::Array(a))
}

/// An array literal built only from int/string-keyed scalar literals,
/// `true`/`false`/`null` and nested such arrays, as its value (`None` when
/// building it could raise a diagnostic or needs runtime evaluation).
fn literal_array<'a>(els: impl Iterator<Item = &'a ArrayElement<'a>>) -> Option<Value> {
    fn scalar(e: &Expression) -> Option<Value> {
        match e {
            Expression::Literal(l) => Some(literal_value(l)),
            Expression::Array(arr) => literal_array(arr.elements.iter()),
            Expression::LegacyArray(arr) => literal_array(arr.elements.iter()),
            Expression::ConstantAccess(ca) => match identifier_name(&ca.name) {
                b"true" | b"TRUE" | b"True" => Some(Value::Bool(true)),
                b"false" | b"FALSE" | b"False" => Some(Value::Bool(false)),
                b"null" | b"NULL" | b"Null" => Some(Value::Null),
                _ => None,
            },
            Expression::UnaryPrefix(u) => match (&u.operator, scalar(u.operand)?) {
                (UnaryPrefixOperator::Negation(_), Value::Int(n)) => {
                    n.checked_neg().map(Value::Int)
                }
                (UnaryPrefixOperator::Negation(_), Value::Float(f)) => Some(Value::Float(-f)),
                _ => None,
            },
            _ => None,
        }
    }
    let mut a = PhpArray::new();
    for el in els {
        match el {
            ArrayElement::KeyValue(kv) => {
                // Only int / string keys: float, bool and null keys convert
                // with diagnostics or php-specific rules at run time.
                let key = match scalar(kv.key)? {
                    Value::Int(n) => ArrayKey::Int(n),
                    k @ Value::Str(_) => ArrayKey::from_value(&k)?,
                    _ => return None,
                };
                // A PHP_INT_MAX key makes a later append fail at run time.
                if matches!(key, ArrayKey::Int(i64::MAX)) {
                    return None;
                }
                a.insert(key, scalar(kv.value)?);
            }
            ArrayElement::Value(v) => a.push(scalar(v.value)?),
            _ => return None,
        }
    }
    Some(Value::Array(a))
}

/// Extract the class name(s) a `catch` clause's type hint names (`A` or `A|B`).
fn hint_class_names(hint: &Hint) -> Result<Vec<PhpStr>, Thrown> {
    fn collect(hint: &Hint, out: &mut Vec<PhpStr>) -> Result<(), Thrown> {
        match hint {
            Hint::Identifier(id) => {
                // Resolve against the current namespace + `use` imports (php
                // treats `catch (Foo)` in namespace N as `N\Foo`), matching how
                // `instanceof`/`new` resolve class names.
                out.push(PhpStr::from_bytes(resolve_class_name(id)));
                Ok(())
            }
            Hint::Union(u) => {
                collect(u.left, out)?;
                collect(u.right, out)
            }
            Hint::Parenthesized(p) => collect(p.hint, out),
            _ => Err(Thrown::unsupported(
                "catch type",
                "only class-name catch types",
            )),
        }
    }
    let mut out = Vec::new();
    collect(hint, &mut out)?;
    Ok(out)
}

/// Whether php's VM swaps the operands of a commutative opcode: op1's
/// kind is lower than op2's, in IS_CONST < IS_TMP_VAR < IS_VAR < IS_CV.
/// Reads of elements and properties are TMP (zend_adjust_for_fetch_type);
/// calls, `new`, `yield` and `include`/`eval` are VAR. (php's frameless
/// and special-cased function calls give TMP; they count as VAR here.)
fn php_swaps_operands(lhs: &Expression, rhs: &Expression) -> bool {
    match (php_operand_kind(lhs), php_operand_kind(rhs)) {
        (l, r) if l != r => l < r,
        // Both CONST-or-TMP: only a CONST op1 against a TMP op2 swaps.
        (Kind::ConstOrTmp, Kind::ConstOrTmp) => !is_ct_const(rhs) && is_ct_const(lhs),
        _ => false,
    }
}

/// Whether php's compiler folds `e` to a constant. A binary operation
/// checks its right operand first, so the left spine of a long chain is
/// not walked at every level.
fn is_ct_const(e: &Expression) -> bool {
    match e {
        Expression::Parenthesized(p) => is_ct_const(p.expression),
        Expression::MagicConstant(_) => true,
        Expression::Binary(b) => is_ct_const(b.rhs) && is_ct_const(b.lhs) && fold_const(e).is_ok(),
        _ => fold_const(e).is_ok(),
    }
}

#[derive(PartialEq, PartialOrd)]
enum Kind {
    ConstOrTmp,
    Var,
    Cv,
}

fn php_operand_kind(e: &Expression) -> Kind {
    match e {
        Expression::Parenthesized(p) => php_operand_kind(p.expression),
        Expression::Variable(Variable::Direct(d)) => {
            let n = strip_dollar(d.name);
            if n == b"this" || is_superglobal(n) {
                Kind::ConstOrTmp
            } else {
                Kind::Cv
            }
        }
        Expression::Call(_)
        | Expression::Instantiation(_)
        | Expression::AnonymousClass(_)
        | Expression::Yield(_) => Kind::Var,
        Expression::Construct(
            Construct::Eval(_)
            | Construct::Include(_)
            | Construct::IncludeOnce(_)
            | Construct::Require(_)
            | Construct::RequireOnce(_),
        ) => Kind::Var,
        _ => Kind::ConstOrTmp,
    }
}

/// PHP's superglobals, available in every scope without `global`.
fn is_superglobal(name: &[u8]) -> bool {
    matches!(
        name,
        b"GLOBALS"
            | b"_SERVER"
            | b"_GET"
            | b"_POST"
            | b"_REQUEST"
            | b"_SESSION"
            | b"_ENV"
            | b"_COOKIE"
            | b"_FILES"
    )
}

fn non_const_err() -> Thrown {
    Thrown::unsupported(
        "non-constant initializer",
        "only literal / array / true/false/null constant initializers for now",
    )
}

/// Fold a literal AST node to its runtime value (php's scanner rules: an
/// integer literal past `i64::MAX` becomes a float).
fn literal_value(lit: &Literal) -> Value {
    match lit {
        Literal::Integer(i) => match i.value {
            Some(u) if u <= i64::MAX as u64 => Value::Int(u as i64),
            Some(u) => Value::Float(u as f64),
            None => {
                let cleaned: Vec<u8> = i.raw.iter().copied().filter(|&b| b != b'_').collect();
                std::str::from_utf8(&cleaned)
                    .ok()
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(Value::Float)
                    .unwrap_or(Value::Int(0))
            }
        },
        Literal::Float(f) => Value::Float(f.value.0),
        Literal::String(s) => Value::str(s.value.unwrap_or(s.raw).to_vec()),
        Literal::True(_) => Value::Bool(true),
        Literal::False(_) => Value::Bool(false),
        Literal::Null(_) => Value::Null,
    }
}

/// Structural equality for constant-pool deduplication.
fn const_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
        (Value::Str(x), Value::Str(y)) => x == y,
        _ => false,
    }
}

fn stmt_kind(s: &Statement) -> &'static str {
    match s {
        Statement::If(_) => "if",
        Statement::While(_) => "while",
        Statement::DoWhile(_) => "do-while",
        Statement::For(_) => "for",
        Statement::Foreach(_) => "foreach",
        Statement::Switch(_) => "switch",
        Statement::Function(_) => "function",
        Statement::Return(_) => "return",
        Statement::Class(_) => "class",
        Statement::Interface(_) => "interface",
        Statement::Trait(_) => "trait",
        Statement::Enum(_) => "enum",
        Statement::Try(_) => "try",
        Statement::Namespace(_) => "namespace",
        Statement::Use(_) => "use",
        Statement::Declare(_) => "declare",
        Statement::Goto(_) => "goto",
        Statement::Label(_) => "label",
        Statement::Block(_) => "block",
        Statement::Global(_) => "global",
        Statement::Static(_) => "static",
        Statement::Unset(_) => "unset",
        Statement::Constant(_) => "const",
        Statement::Echo(_) => "echo",
        Statement::Expression(_) => "expression",
        Statement::HaltCompiler(_) => "__halt_compiler",
        _ => "this",
    }
}

/// Read-only collection of every `$var` name referenced in an expression, for
/// arrow-function auto-capture. Descends into nested closures/arrows so their
/// free variables propagate upward (over-collection is harmless: the caller
/// keeps only names that are CVs in the enclosing scope). Does not mutate the
/// builder, unlike [`ProtoBuilder::scan_expr_vars`].
fn collect_vars(expr: &Expression, out: &mut Vec<Vec<u8>>) {
    collect_node_vars(Node::Expression(expr), out);
}

/// [`collect_vars`] over any node: every `$name` below it (in source order,
/// deduplicated), entering nested arrow functions but not other function or
/// class scopes; a closure contributes its `use` variables.
fn collect_node_vars(node: Node, out: &mut Vec<Vec<u8>>) {
    let push = |name: &[u8], out: &mut Vec<Vec<u8>>| {
        if !out.iter().any(|n| n == name) {
            out.push(name.to_vec());
        }
    };
    match node {
        Node::DirectVariable(d) => {
            push(strip_dollar(d.name), out);
            return;
        }
        Node::Closure(cl) => {
            if let Some(uses) = &cl.use_clause {
                for u in uses.variables.iter() {
                    push(strip_dollar(u.variable.name), out);
                }
            }
            return;
        }
        Node::AnonymousClass(ac) => {
            if let Some(args) = &ac.argument_list {
                collect_node_vars(Node::PartialArgumentList(args), out);
            }
            return;
        }
        // `C::$p`: the `$p` is the property name, not a variable.
        Node::StaticPropertyAccess(spa) => {
            collect_node_vars(Node::Expression(spa.class), out);
            return;
        }
        Node::Function(_)
        | Node::Class(_)
        | Node::Interface(_)
        | Node::Trait(_)
        | Node::Enum(_)
        | Node::AttributeList(_) => return,
        _ => {}
    }
    node.visit_children(|ch| collect_node_vars(ch, out));
}

fn expr_kind(e: &Expression) -> &'static str {
    match e {
        Expression::CompositeString(_) => "interpolated-string",
        Expression::Match(_) => "match",
        Expression::Closure(_) => "closure",
        Expression::ArrayAccess(_) => "array-access",
        Expression::Throw(_) => "throw",
        _ => "this",
    }
}

/// The callee a SEND_FUNC_ARG check asks: a method on the object in a
/// register, or a static method / constructor of a class.
#[derive(Clone, Copy)]
enum ArgRefTarget {
    Method(Reg, NIdx),
    Static(ClassRef, NIdx),
}

/// Whether `e` is a `$var[...]` / `$var->p` / `C::$p[...]` chain rooted at a
/// variable other than `$GLOBALS` or at a static property: the arguments
/// SEND_FUNC_ARG can bind by reference (a chain on a call result or other
/// temporary is passed by value).
fn variable_rooted_lvalue(e: &Expression) -> bool {
    match e {
        Expression::Variable(Variable::Direct(_)) => !is_globals_var(e),
        Expression::Access(Access::StaticProperty(_)) => true,
        Expression::ArrayAccess(a) => variable_rooted_lvalue(a.array),
        Expression::Access(Access::Property(p)) => variable_rooted_lvalue(p.object),
        _ => false,
    }
}
