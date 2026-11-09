//! Per-file compiler state: the file's name, source and line map, its
//! signature tables, the sinks its declarations land in, and its flags.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::parse::cst::{ClassLikeMember, Program as AstProgram, Statement};
use rhp_bytecode::{ClassProto, FunctionProto};
use rhp_values::{PhpStr, Thrown, Value};

use crate::fold::fold_const;
use crate::names::{join_ns, NsContext};
use crate::Builtins;

/// The file being compiled. Each compile builds one (the prelude its own) and
/// every builder borrows it, so nothing outlives the compile or leaks into
/// the next one on the thread.
pub(crate) struct FileCtx<'s> {
    /// The builtins the file is compiled against.
    pub(crate) builtins: &'s Builtins,
    /// The file name for debug info, getFile() and `__FILE__`. Empty for the
    /// prelude.
    pub(crate) file: PhpStr,
    /// The source (empty for the prelude), for what the AST drops: doc
    /// comments and the newline a `?>` swallows.
    pub(crate) src: &'s [u8],
    /// The source's tokens, comments included, lexed on the first doc
    /// comment lookup.
    tokens: OnceCell<Vec<crate::parse::Token>>,
    /// The variables the file may alias, scanned on first use
    /// ([`FileCtx::may_alias`]).
    alias_marks: OnceCell<AliasMarks>,
    /// Each anonymous class's CG(rtd_key_counter) value, by its offset
    /// ([`crate::decl::assign_rtd_keys`]).
    pub(crate) anon_keys: RefCell<HashMap<u32, u32>>,
    /// Each anonymous class's CG(rtd_key_counter) value, by its start
    /// offset ([`crate::decl::assign_rtd_keys`]).
    pub(crate) anon_keys: RefCell<HashMap<u32, u32>>,
    /// Byte offset of each line start. Empty for the prelude, whose line
    /// numbers are never surfaced.
    line_starts: Vec<u32>,
    /// The prelude itself is being compiled: every builtin it calls exists,
    /// and its protos are internal ([`rhp_bytecode::ProtoFlags::INTERNAL`]).
    pub(crate) is_prelude: bool,
    /// `declare(strict_types=1)` (php's caller-decides coercion rule),
    /// stamped on every proto of the file.
    pub(crate) strict_types: bool,
    /// Function name (fully qualified, lowercase) -> per-parameter by-ref
    /// flags, so a call site can pass by reference to a function it names.
    pub(crate) sigs: HashMap<Vec<u8>, Vec<bool>>,
    /// Same keys: the parameter names (no `$`), and whether the last one
    /// is variadic (a by-reference variadic covers every later position;
    /// the names place named arguments at compile time).
    pub(crate) sig_names: HashMap<Vec<u8>, (Vec<Vec<u8>>, bool)>,
    /// The same functions that return by reference.
    pub(crate) ref_returning: std::collections::HashSet<Vec<u8>>,
    /// The end offset of each function in `sigs`: php knows a function
    /// at compile time only after its declaration was compiled.
    pub(crate) sig_ends: HashMap<Vec<u8>, u32>,
    /// The byte offset after `__halt_compiler();`: this file's
    /// `__COMPILER_HALT_OFFSET__`.
    pub(crate) halt_offset: Option<i64>,
    /// Method name (lowercase) -> per-position "could be by-reference" mask:
    /// true at position i if ANY method of that name declares `&` there. A
    /// call site passes those positions by reference; by-value callees simply
    /// deref, so over-approximation across classes is safe.
    method_ref: HashMap<Vec<u8>, Vec<bool>>,
    /// The namespace context of the statement being compiled.
    ns: RefCell<Rc<NsContext>>,
    /// A trait body is being compiled: `__CLASS__` there names the *using*
    /// class, so it is lowered to a runtime `self::class`.
    pub(crate) in_trait: Cell<bool>,
    /// An interface body is being compiled.
    pub(crate) in_interface: Cell<bool>,
    /// The class-like whose body is being compiled (php's
    /// `CG(active_class_entry)`), for `self`/`parent`/`static` in types.
    pub(crate) class_scope: RefCell<Option<Rc<ClassScope>>>,
    /// A `readonly class` body is being compiled.
    pub(crate) in_readonly_class: Cell<bool>,
    /// The property whose declaration is being compiled (`__PROPERTY__` in
    /// its default value and attributes).
    pub(crate) cur_property: RefCell<Option<Vec<u8>>>,
    /// The function or method (`f()`, `C::m()`) whose attributes are being
    /// compiled: a closure in them is named after it (else the file).
    pub(crate) attr_fn_scope: RefCell<Option<Vec<u8>>>,
    /// Closure and arrow-function protos, and functions declared in control
    /// flow, from anywhere in the file: `Program::closures`. A closure's
    /// `fnidx` is its position here.
    pub(crate) closures: RefCell<Vec<Rc<FunctionProto>>>,
    /// Anonymous classes (`new class {…}`), declared with the program's
    /// classes under their generated names.
    pub(crate) anon_classes: RefCell<Vec<ClassProto>>,
    /// Class declarations nested in control flow or function bodies:
    /// `Program::cond_classes`.
    pub(crate) cond_classes: RefCell<Vec<ClassProto>>,
    /// The file's count of top-level class-likes: the `DeclareClass` index
    /// of `cond_classes[0]`.
    pub(crate) num_top_classes: u32,
    /// The earliest compile error (source offset, line, message) recorded by
    /// [`FileCtx::fatal`].
    fatal: RefCell<Option<(u32, u32, String)>>,
    /// Diagnostics php raises while compiling (level, message, line):
    /// `Program::compile_diags`.
    pub(crate) compile_diags: RefCell<Vec<(i64, String, u32)>>,
    /// Diagnostics php's scanner raises (level, message, byte offset): the
    /// whole file is scanned before anything compiles, so they come first.
    pub(crate) scan_diags: RefCell<Vec<(i64, String, u32)>>,
    /// The leading run of top-level `declare`s (zend_is_first_statement):
    /// each one's offset, and whether an empty statement came before it.
    pub(crate) first_declares: Vec<(u32, bool)>,
    /// `declare(ticks)`'s value through the file, as php's file-level
    /// FC(declarables).ticks changes in source order: (offset, ticks) at
    /// each change, sorted. Empty when the file never declares ticks.
    pub(crate) ticks: Vec<(u32, u32)>,
}

/// The class-like a declaration sits in, as type compilation needs it.
pub(crate) struct ClassScope {
    /// Resolved class name.
    pub(crate) name: Vec<u8>,
    /// Resolved parent name (`extends` of a class; none for interfaces).
    pub(crate) parent: Option<Vec<u8>>,
    pub(crate) is_trait: bool,
    pub(crate) is_anon: bool,
    /// The constants compiled so far with a value known at compile time
    /// (php's active class's constants_table, which
    /// zend_try_ct_eval_class_const substitutes).
    pub(crate) consts: RefCell<HashMap<Vec<u8>, Value>>,
}

impl<'s> FileCtx<'s> {
    /// Run `f` with `scope` as the active class-like.
    pub(crate) fn with_class_scope<R>(&self, scope: ClassScope, f: impl FnOnce() -> R) -> R {
        let saved = self.class_scope.replace(Some(Rc::new(scope)));
        let r = f();
        *self.class_scope.borrow_mut() = saved;
        r
    }

    fn empty(builtins: &'s Builtins, file: PhpStr, src: &'s [u8]) -> Self {
        FileCtx {
            builtins,
            file,
            src,
            tokens: OnceCell::new(),
            alias_marks: OnceCell::new(),
            anon_keys: RefCell::new(HashMap::new()),
            anon_keys: RefCell::new(HashMap::new()),
            line_starts: Vec::new(),
            is_prelude: false,
            strict_types: false,
            sigs: HashMap::new(),
            sig_names: HashMap::new(),
            ref_returning: Default::default(),
            sig_ends: HashMap::new(),
            halt_offset: None,
            method_ref: HashMap::new(),
            ns: RefCell::new(Rc::new(NsContext::default())),
            in_trait: Cell::new(false),
            in_interface: Cell::new(false),
            class_scope: RefCell::new(None),
            in_readonly_class: Cell::new(false),
            cur_property: RefCell::new(None),
            attr_fn_scope: RefCell::new(None),
            closures: RefCell::new(Vec::new()),
            anon_classes: RefCell::new(Vec::new()),
            cond_classes: RefCell::new(Vec::new()),
            num_top_classes: 0,
            fatal: RefCell::new(None),
            compile_diags: RefCell::new(Vec::new()),
            scan_diags: RefCell::new(Vec::new()),
            first_declares: Vec::new(),
            ticks: Vec::new(),
        }
    }

    /// A user file: `flat` is its statements with their namespace contexts.
    pub(crate) fn new(
        builtins: &'s Builtins,
        file: &str,
        src: &'s [u8],
        program: &AstProgram,
        flat: &[(Rc<NsContext>, &Statement)],
    ) -> Self {
        let mut cx = Self::empty(builtins, PhpStr::from_bytes(file.as_bytes()), src);
        // Before the line map: `__LINE__` folds to 0 here.
        cx.strict_types = detect_strict_types(&cx, program);
        cx.first_declares = first_declares(&cx, program);
        cx.ticks = tick_points(&cx, program);
        cx.line_starts = build_line_starts(src);
        // Keyed by fully qualified name (lowercased), as call sites resolve.
        for (ns, stmt) in flat {
            if let Statement::Function(func) = stmt {
                let fqn = join_ns(&ns.ns, func.name.value).to_ascii_lowercase();
                let flags = func
                    .parameter_list
                    .parameters
                    .iter()
                    .map(|p| p.ampersand.is_some())
                    .collect();
                let params = &func.parameter_list.parameters;
                let names = params
                    .iter()
                    .map(|p| crate::util::strip_dollar(p.variable.name).to_vec())
                    .collect();
                let variadic = params.iter().last().is_some_and(|p| p.ellipsis.is_some());
                cx.sig_names.insert(fqn.clone(), (names, variadic));
                if func.ampersand.is_some() {
                    cx.ref_returning.insert(fqn.clone());
                }
                use mago_span::HasSpan;
                cx.sig_ends.insert(fqn.clone(), func.span().end.offset);
                cx.sigs.insert(fqn, flags);
            }
        }
        cx.method_ref = collect_method_refs(flat);
        cx.halt_offset = flat.iter().find_map(|(_, st)| match st {
            Statement::HaltCompiler(h) => {
                use mago_span::HasSpan;
                let end = h.span().end.offset as usize;
                // A closing `?>` takes one newline with it.
                let rest = &src[end.min(src.len())..];
                let nl = if !src[..end.min(src.len())].ends_with(b"?>") {
                    0
                } else if rest.starts_with(b"\r\n") {
                    2
                } else if rest.starts_with(b"\n") || rest.starts_with(b"\r") {
                    1
                } else {
                    0
                };
                Some((end + nl) as i64)
            }
            _ => None,
        });
        cx.num_top_classes = flat.iter().filter(|(_, st)| is_class_like(st)).count() as u32;
        cx
    }

    /// The prelude: no file name, source or line map, and only its own
    /// method signatures (`flat` is its statements).
    pub(crate) fn prelude(
        builtins: &'s Builtins,
        flat: &[(Rc<NsContext>, &Statement)],
    ) -> FileCtx<'s> {
        let mut cx = FileCtx::empty(builtins, PhpStr::from_bytes(Vec::new()), b"");
        cx.is_prelude = true;
        cx.method_ref = collect_method_refs(flat);
        cx
    }

    /// Record an E_COMPILE_ERROR found at source `offset`. The compile still
    /// finishes (so callers need no error plumbing), then fails with the
    /// earliest recorded error (see [`FileCtx::take_fatal`]).
    pub(crate) fn fatal(&self, offset: u32, message: String) {
        let line = self.line_of(offset);
        let mut f = self.fatal.borrow_mut();
        if f.as_ref().is_none_or(|(o, _, _)| offset < *o) {
            *f = Some((offset, line, message));
        }
    }

    /// Run `f` with the compile errors and diagnostics it records dropped
    /// (a check that only needs a value, such as a lowered type).
    pub(crate) fn quietly<T>(&self, f: impl FnOnce() -> T) -> T {
        let fatal = self.fatal.borrow().clone();
        let ndiags = self.compile_diags.borrow().len();
        let r = f();
        *self.fatal.borrow_mut() = fatal;
        self.compile_diags.borrow_mut().truncate(ndiags);
        r
    }

    /// [`FileCtx::fatal`] reported at `line`, ordered as if found at source
    /// `order` (php raises some errors at a declaration's first line only
    /// after compiling its body).
    pub(crate) fn fatal_line(&self, order: u32, line: u32, message: String) {
        let mut f = self.fatal.borrow_mut();
        if f.as_ref().is_none_or(|(o, _, _)| order < *o) {
            *f = Some((order, line, message));
        }
    }

    /// Record a diagnostic php raises while compiling, at the line of `offset`.
    pub(crate) fn diag(&self, level: i64, offset: u32, message: String) {
        let line = self.line_of(offset);
        self.compile_diags.borrow_mut().push((level, message, line));
    }

    /// A diagnostic of php's scanner at byte `offset`.
    pub(crate) fn scan_diag(&self, level: i64, offset: u32, message: String) {
        self.scan_diags.borrow_mut().push((level, message, offset));
    }

    /// All diagnostics in php's order: the scanner's in source order, then
    /// the compiler's by line.
    pub(crate) fn take_diags(&self, extra: Vec<(i64, String, u32)>) -> Vec<(i64, String, u32)> {
        let mut scan = self.scan_diags.take();
        scan.sort_by_key(|d| d.2);
        // An expression compiled twice is still scanned once.
        scan.dedup();
        let mut rest: Vec<_> = extra.into_iter().chain(self.compile_diags.take()).collect();
        rest.sort_by_key(|d| d.2);
        scan.into_iter()
            .map(|(l, m, o)| (l, m, self.line_of(o)))
            .chain(rest)
            .collect()
    }

    /// The earliest recorded compile error: `(line, message)`.
    pub(crate) fn take_fatal(&self) -> Option<(u32, String)> {
        self.fatal
            .borrow_mut()
            .take()
            .map(|(_, line, message)| (line, message))
    }

    /// 1-based line number of a byte offset (0 without a line map).
    pub(crate) fn line_of(&self, offset: u32) -> u32 {
        let ls = &self.line_starts;
        if ls.is_empty() {
            return 0;
        }
        // Largest index whose start <= offset, +1 for 1-based.
        match ls.binary_search(&offset) {
            Ok(i) => (i + 1) as u32,
            Err(i) => i as u32, // i = count of starts <= offset
        }
    }

    /// The file name's bytes (for `__FILE__` and generated names).
    pub(crate) fn file_name(&self) -> Vec<u8> {
        self.file.as_bytes().to_vec()
    }

    /// Stamp a compile-time refusal with the file being compiled and the
    /// line of `offset`, unless an inner construct already located it.
    pub(crate) fn locate(&self, e: Thrown, offset: u32) -> Thrown {
        if !matches!(e, Thrown::Unsupported { location: None, .. }) {
            return e;
        }
        let line = self.line_of(offset);
        e.with_location(
            || String::from_utf8_lossy(self.file.as_bytes()).into_owned(),
            line,
        )
    }

    /// Inline text that is no statement for php: empty after the newline
    /// folded into `?>`, or the shebang line of the file.
    pub(crate) fn inline_is_empty(&self, i: &crate::parse::cst::Inline) -> bool {
        use mago_span::HasSpan;
        let at = i.span().start.offset;
        if at == 0 && i.value.starts_with(b"#!") {
            let rest = i
                .value
                .iter()
                .position(|&b| b == b'\n')
                .map_or(&b""[..], |p| &i.value[p + 1..]);
            return rest.is_empty();
        }
        self.strip_close_tag_newline(at, i.value).is_empty()
    }

    /// php's lexer folds one newline (`\n`, `\r\n` or `\r`) right after `?>`
    /// into the closing tag: drop it from inline text that starts there.
    pub(crate) fn strip_close_tag_newline<'t>(&self, offset: u32, text: &'t [u8]) -> &'t [u8] {
        let b = self.src;
        let o = offset as usize;
        if !(o >= 2 && o <= b.len() && &b[o - 2..o] == b"?>") {
            return text;
        }
        match text {
            [b'\r', b'\n', rest @ ..] | [b'\n', rest @ ..] | [b'\r', rest @ ..] => rest,
            _ => text,
        }
    }

    /// The source and its token stream for doc comment lookups (no tokens
    /// when the source has no `/**`).
    pub(crate) fn doc_tokens(&self) -> (&[u8], &[crate::parse::Token]) {
        let toks = self.tokens.get_or_init(|| {
            if self.src.windows(3).any(|w| w == b"/**") {
                crate::parse::tokens(self.src)
            } else {
                Vec::new()
            }
        });
        (self.src, toks)
    }

    /// Switch to the namespace context of the next statement.
    pub(crate) fn set_ns(&self, ctx: &Rc<NsContext>) {
        *self.ns.borrow_mut() = ctx.clone();
    }

    /// Name closures in attributes after function `qual` until the guard
    /// drops.
    pub(crate) fn attr_scope(&self, qual: Vec<u8>) -> AttrScope<'_, 's> {
        let outer = self.attr_fn_scope.replace(Some(qual));
        AttrScope { cx: self, outer }
    }

    /// The namespace context of the statement being compiled.
    pub(crate) fn ns(&self) -> std::cell::Ref<'_, Rc<NsContext>> {
        self.ns.borrow()
    }

    /// Add a closure (or a function declared in control flow): its index.
    pub(crate) fn add_closure(&self, proto: FunctionProto) -> u32 {
        let mut v = self.closures.borrow_mut();
        v.push(Rc::new(proto));
        v.len() as u32 - 1
    }

    /// Add a class declared in control flow: its `DeclareClass` index.
    pub(crate) fn add_cond_class(&self, proto: ClassProto) -> u32 {
        let mut v = self.cond_classes.borrow_mut();
        v.push(proto);
        self.num_top_classes + v.len() as u32 - 1
    }

    /// The by-ref mask for a method/constructor name (empty if none).
    pub(crate) fn method_ref_mask(&self, name_lc: &[u8]) -> Vec<bool> {
        self.method_ref.get(name_lc).cloned().unwrap_or_default()
    }
}

fn is_class_like(st: &Statement) -> bool {
    matches!(
        st,
        Statement::Class(_) | Statement::Interface(_) | Statement::Trait(_) | Statement::Enum(_)
    )
}

/// Whether the program's leading declares set `declare(strict_types=1)`.
fn detect_strict_types(cx: &FileCtx, program: &AstProgram) -> bool {
    for stmt in program.statements.iter() {
        match stmt {
            // skip the opening `<?php` / inline-html prelude
            Statement::OpeningTag(_) | Statement::ClosingTag(_) | Statement::Noop(_) => continue,
            Statement::Inline(i) if cx.inline_is_empty(i) => continue,
            Statement::Declare(d) => {
                // Other declares may come first (zend_is_first_statement).
                for item in d.items.iter() {
                    if item.name.value.eq_ignore_ascii_case(b"strict_types") {
                        return matches!(fold_const(cx, item.value), Ok(Value::Int(1)));
                    }
                }
            }
            _ => return false, // strict_types must be the first statement
        }
    }
    false
}

/// The top-level `declare`s before any other statement, with whether an
/// empty statement (`;`, `?>`) came before each.
fn first_declares(cx: &FileCtx, program: &AstProgram) -> Vec<(u32, bool)> {
    let mut out = Vec::new();
    let mut nop = false;
    for stmt in program.statements.iter() {
        match stmt {
            // The scanner skips a leading `#!` line.
            Statement::OpeningTag(_) => {}
            Statement::Inline(i) if matches!(i.kind, crate::parse::cst::InlineKind::Shebang) => {}
            // `?>` takes its newline, so `?>\n<?php` leaves no text.
            Statement::Inline(i) if cx.inline_is_empty(i) => {}
            Statement::Noop(_) | Statement::ClosingTag(_) => nop = true,
            Statement::Declare(d) => out.push((d.declare.span.start.offset, nop)),
            _ => break,
        }
    }
    out
}

/// [`FileCtx::ticks`]: each `declare(ticks=n)` sets the value from its
/// start; one with a body restores the previous value at its end (php
/// restores it once the body is compiled; `declare(ticks=n);` has none).
fn tick_points(cx: &FileCtx, program: &AstProgram) -> Vec<(u32, u32)> {
    use crate::parse::cst::{DeclareBody, Node};
    use mago_span::HasSpan;
    if !cx.src.windows(5).any(|w| w.eq_ignore_ascii_case(b"ticks")) {
        return Vec::new();
    }
    // (start, end of the body or None, value), in source order.
    let mut decls: Vec<(u32, Option<u32>, u32)> = Vec::new();
    fn walk(node: Node, cx: &FileCtx, out: &mut Vec<(u32, Option<u32>, u32)>) {
        if let Node::Declare(d) = node {
            for item in d.items.iter() {
                if item.name.value.eq_ignore_ascii_case(b"ticks") {
                    let v = crate::fold::fold_const(cx, item.value)
                        .map_or(0, |v| rhp_values::conv::to_int(&v));
                    let block = !matches!(
                        &d.body,
                        DeclareBody::Statement(Statement::Noop(_) | Statement::ClosingTag(_))
                    );
                    // The restore point is the declare's last byte: the
                    // statements in its body all end before it.
                    let end = block.then(|| d.span().end.offset.saturating_sub(1));
                    out.push((d.span().start.offset, end, v as u32));
                }
            }
        }
        node.visit_children(|ch| walk(ch, cx, out));
    }
    walk(Node::Program(program), cx, &mut decls);
    decls.sort_by_key(|d| d.0);
    let mut points = Vec::new();
    let mut cur = 0u32;
    // Block declares still open: (end, the value before them).
    let mut open: Vec<(u32, u32)> = Vec::new();
    for (start, end, v) in decls {
        while open.last().is_some_and(|&(e, _)| e <= start) {
            let (e, orig) = open.pop().unwrap();
            cur = orig;
            points.push((e, cur));
        }
        if let Some(end) = end {
            open.push((end, cur));
        }
        cur = v;
        points.push((start, cur));
    }
    while let Some((e, orig)) = open.pop() {
        points.push((e, orig));
    }
    points
}

/// What [`FileCtx::may_alias`] scans for in the source text.
#[derive(Default)]
struct AliasMarks {
    /// Names (with `$`) after a `&` or in a `global` statement.
    names: std::collections::HashSet<Vec<u8>>,
    /// `$$`, `eval(` or `extract(`: any variable may change.
    dynamic: bool,
}

impl FileCtx<'_> {
    /// Whether the variable `name` (with `$`) may be written through
    /// another name somewhere in this file: a reference (`&$name` anywhere:
    /// a by-reference parameter, assignment, closure use or foreach), a
    /// `global`, or a scope written dynamically (`$$`, eval, extract). A
    /// text scan, so it over-approximates (`$a & $b`, comments).
    pub(crate) fn may_alias(&self, name: &[u8]) -> bool {
        let m = self.alias_marks.get_or_init(|| scan_alias_marks(self.src));
        m.dynamic || m.names.contains(name)
    }

    /// The ticks value a statement ending at `end` is compiled under: the
    /// last change before it.
    pub(crate) fn ticks_at(&self, end: u32) -> u32 {
        let i = self.ticks.partition_point(|&(o, _)| o < end);
        if i == 0 {
            0
        } else {
            self.ticks[i - 1].1
        }
    }
}

fn scan_alias_marks(src: &[u8]) -> AliasMarks {
    let mut m = AliasMarks::default();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    let skip_ws = |mut i: usize| {
        while i < src.len() && src[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let var_at = |i: usize| -> Option<(Vec<u8>, usize)> {
        if src.get(i) != Some(&b'$') {
            return None;
        }
        let n = src[i + 1..].iter().take_while(|&&c| ident(c)).count();
        (n > 0).then(|| (src[i..i + 1 + n].to_vec(), i + 1 + n))
    };
    let word_at = |i: usize, w: &[u8]| {
        src.len() >= i + w.len()
            && src[i..i + w.len()].eq_ignore_ascii_case(w)
            && (i == 0 || !ident(src[i - 1]) && src[i - 1] != b'$')
            && src.get(i + w.len()).is_none_or(|&c| !ident(c))
    };
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'&' => {
                if let Some((name, _)) = var_at(skip_ws(i + 1)) {
                    m.names.insert(name);
                }
            }
            b'$' if src.get(i + 1) == Some(&b'$') => m.dynamic = true,
            b'e' | b'E' => {
                for w in [&b"eval"[..], b"extract"] {
                    if word_at(i, w) && src.get(skip_ws(i + w.len())) == Some(&b'(') {
                        m.dynamic = true;
                    }
                }
            }
            b'g' | b'G' if word_at(i, b"global") => {
                let mut j = skip_ws(i + 6);
                while let Some((name, end)) = var_at(j) {
                    m.names.insert(name);
                    j = skip_ws(end);
                    if src.get(j) != Some(&b',') {
                        break;
                    }
                    j = skip_ws(j + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    m
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

/// Scan every class-like's methods, OR-ing their by-ref parameter flags by name.
/// (Method names are not namespaced, so this ignores the namespace context.)
fn collect_method_refs(flat: &[(Rc<NsContext>, &Statement)]) -> HashMap<Vec<u8>, Vec<bool>> {
    let mut map: HashMap<Vec<u8>, Vec<bool>> = HashMap::new();
    let mut add = |name: &[u8], pl: &crate::parse::cst::FunctionLikeParameterList| {
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

/// See [`FileCtx::attr_scope`].
pub(crate) struct AttrScope<'c, 's> {
    cx: &'c FileCtx<'s>,
    outer: Option<Vec<u8>>,
}

impl Drop for AttrScope<'_, '_> {
    fn drop(&mut self) {
        *self.cx.attr_fn_scope.borrow_mut() = self.outer.take();
    }
}
