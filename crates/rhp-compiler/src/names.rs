//! Namespaces: `namespace`/`use` contexts and php's name resolution rules.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rhp_frontend::cst::{Identifier, Statement};

/// The per-file, per-block namespace state threaded through compilation: the
/// current namespace and the `use` import aliases visible so far (textual order
/// — a reference only sees imports declared before it, matching PHP). Class and
/// function aliases are keyed case-insensitively (lowercased); constant aliases
/// are case-sensitive. Alias values are fully-qualified names WITHOUT a leading
/// backslash.
#[derive(Default, Clone)]
pub(crate) struct NsContext {
    pub(crate) ns: Vec<u8>, // "" = global namespace, else e.g. b"Foo\\Bar"
    pub(crate) use_class: HashMap<Vec<u8>, Vec<u8>>,
    pub(crate) use_func: HashMap<Vec<u8>, Vec<u8>>,
    pub(crate) use_const: HashMap<Vec<u8>, Vec<u8>>,
}

thread_local! {
    /// The namespace context active at the statement currently being compiled.
    /// Set from the flattened `(context, statement)` list before each statement
    /// in every compile pass, so deep name-resolution chokepoints can read it.
    pub(crate) static NS_CTX: RefCell<Rc<NsContext>> = RefCell::new(Rc::new(NsContext::default()));
}

/// Which symbol table an identifier resolves against (PHP resolves classes,
/// functions, and constants in separate namespace tables with different rules).
#[derive(Clone, Copy)]
pub(crate) enum NameKind {
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

pub(crate) fn set_ns_ctx(ctx: &Rc<NsContext>) {
    NS_CTX.with(|c| *c.borrow_mut() = ctx.clone());
}

/// The current namespace (empty for the global namespace).
pub(crate) fn cur_ns() -> Vec<u8> {
    NS_CTX.with(|c| c.borrow().ns.clone())
}

/// Join a namespace prefix and a (relative) name with a backslash.
pub(crate) fn join_ns(ns: &[u8], name: &[u8]) -> Vec<u8> {
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
pub(crate) fn strip_lead_bs(name: &[u8]) -> Vec<u8> {
    if name.first() == Some(&b'\\') {
        name[1..].to_vec()
    } else {
        name.to_vec()
    }
}

pub(crate) fn starts_with_ci(hay: &[u8], pfx: &[u8]) -> bool {
    hay.len() >= pfx.len() && hay[..pfx.len()].eq_ignore_ascii_case(pfx)
}

/// The raw identifier bytes exactly as written (backslashes preserved), unlike
/// [`identifier_name`] which strips to the last segment.
pub(crate) fn identifier_raw<'a>(id: &'a Identifier<'a>) -> &'a [u8] {
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
pub(crate) fn resolve_name(id: &Identifier, kind: NameKind) -> (Vec<u8>, Option<Vec<u8>>) {
    NS_CTX.with(|c| resolve_in(&c.borrow(), id, kind))
}

pub(crate) fn resolve_in(
    ctx: &NsContext,
    id: &Identifier,
    kind: NameKind,
) -> (Vec<u8>, Option<Vec<u8>>) {
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
pub(crate) fn resolve_class_name(id: &Identifier) -> Vec<u8> {
    resolve_name(id, NameKind::Class).0
}

/// Flatten `namespace`/`use` statements into `(context, statement)` pairs in
/// source order: each `use` accumulates into the running context (so later
/// statements see it but earlier ones do not), and each `namespace` block starts
/// a fresh context. Every compile pass iterates this list so name resolution is
/// identical across passes.
pub(crate) fn flatten_namespaces<'a>(
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

pub(crate) fn identifier_name<'a>(id: &'a Identifier<'a>) -> &'a [u8] {
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
