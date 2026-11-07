//! Class-like declarations: classes, interfaces, traits, enums and their members, attributes and doc comments.

use std::rc::Rc;

use mago_span::HasSpan;

use crate::parse::cst::{
    Access, Class, ClassLikeMember, Expression, Hint, Identifier, Modifier, Node, Property,
    PropertyItem,
};
use rhp_bytecode::{
    AttrArgShow, ClassProto, ConstInitProto, ConstProto, FunctionProto, Instr, MethodProto,
    PropDefaultProto, PropProto, StaticPropProto,
};
use rhp_values::{ArrayKey, PhpArray, PhpStr, Thrown, Value};
use rhp_values::{ClassFlags, ClassKind, Visibility};

use crate::attrs::AttrTarget;
use crate::builder::{ProtoBuilder, ScopeKind};
use crate::checks::{check_decl, Decl, DeclKind};
use crate::file::{ClassScope, FileCtx};
use crate::fold::fold_const;
use crate::functions::compile_method;
use crate::functions::const_type_name;
use crate::hints::{compile_type, PhpType, MAY_CALLABLE, MAY_NEVER, MAY_VOID};
use crate::names::{identifier_raw, join_ns};
use crate::util::strip_dollar;

pub(crate) fn modifiers_visibility(mods: &crate::parse::cst::Sequence<Modifier>) -> Visibility {
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
/// it is narrower than the read visibility; an explicit `public(set)` on a
/// public property is kept (it keeps a readonly one publicly writable).
pub(crate) fn modifiers_set_visibility(
    mods: &crate::parse::cst::Sequence<Modifier>,
) -> Option<Visibility> {
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
    (rank(set) > rank(read) || (set == Visibility::Public && read == Visibility::Public))
        .then_some(set)
}

/// [`modifiers_set_visibility`], plus an explicit `public(set)` on a
/// readonly property (which is otherwise implicitly protected(set)).
pub(crate) fn readonly_set_visibility(
    mods: &crate::parse::cst::Sequence<Modifier>,
    readonly: bool,
) -> Option<Visibility> {
    let public_set = || mods.iter().any(|m| matches!(m, Modifier::PublicSet(_)));
    modifiers_set_visibility(mods)
        .or_else(|| (readonly && public_set()).then_some(Visibility::Public))
}

pub(crate) fn has_modifier(
    mods: &crate::parse::cst::Sequence<Modifier>,
    want: fn(&Modifier) -> bool,
) -> bool {
    mods.iter().any(want)
}

/// Whether the attribute lists carry `#[AllowDynamicProperties]`.
fn has_allow_dynamic(
    lists: &crate::parse::cst::Sequence<crate::parse::cst::AttributeList>,
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
impl FileCtx<'_> {
    pub(crate) fn doc_comment_before(&self, pos: u32) -> Option<PhpStr> {
        use crate::parse::TokenKind as T;
        let (src, toks) = self.doc_tokens();
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
                        return Some(PhpStr::from_bytes(text));
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
}

/// Compile `#[...]` attribute groups: each attribute's resolved class name
/// and its argument list as an array (folded when constant, else a thunk run
/// in `class_name`'s scope when Reflection asks for the arguments).
pub(crate) fn compile_attributes(
    cx: &FileCtx,
    lists: &crate::parse::cst::Sequence<crate::parse::cst::AttributeList>,
    class_name: Option<&PhpStr>,
    target: &crate::attrs::AttrTarget,
) -> Result<Vec<rhp_bytecode::AttributeProto>, Thrown> {
    use crate::parse::cst::PartialArgument as PA;
    let mut out = Vec::new();
    for list in lists.iter() {
        for a in list.attributes.iter() {
            let name = PhpStr::from_bytes(cx.resolve_class_name(&a.name));
            if crate::attrs::skip_promoted(&name.as_bytes().to_ascii_lowercase(), target) {
                continue;
            }
            let mut arg_show = Vec::new();
            let args = match &a.argument_list {
                None => None,
                Some(al) => {
                    // (name, expr) pairs; positional args have no name.
                    let mut items: Vec<(Option<&[u8]>, &Expression)> = Vec::new();
                    if al
                        .arguments
                        .iter()
                        .any(|a| matches!(a, PA::VariadicPlaceholder(_)))
                    {
                        use mago_span::HasSpan;
                        // Reported at the declaration's line: the first
                        // token after the attribute list.
                        let end = list.span().end.offset as usize;
                        let src = cx.doc_tokens().0;
                        let skip = src[end.min(src.len())..]
                            .iter()
                            .take_while(|b| b.is_ascii_whitespace())
                            .count();
                        cx.fatal(
                            (end + skip) as u32,
                            "Cannot create Closure as attribute argument".into(),
                        );
                    }
                    for arg in al.arguments.iter() {
                        let at = arg.span().start.offset;
                        match arg {
                            PA::Positional(p) if p.ellipsis.is_some() => {
                                cx.fatal(
                                    at,
                                    "Cannot use unpacking in attribute argument list".into(),
                                );
                            }
                            PA::Positional(_) if items.iter().any(|(n, _)| n.is_some()) => {
                                cx.fatal(
                                    at,
                                    "Cannot use positional argument after named argument".into(),
                                );
                            }
                            PA::Positional(p) => items.push((None, p.value)),
                            PA::Named(n) if items.iter().any(|(k, _)| *k == Some(n.name.value)) => {
                                cx.fatal(
                                    at,
                                    format!(
                                        "Duplicate named parameter ${}",
                                        String::from_utf8_lossy(n.name.value)
                                    ),
                                );
                            }
                            PA::Named(n) => items.push((Some(n.name.value), n.value)),
                            _ => {}
                        }
                    }
                    let known = member_scope_known(cx);
                    for (_, e) in &items {
                        if !crate::constexpr::check_const_expr(cx, e, true, known) {
                            break;
                        }
                    }
                    let names = AttrNames { cx, class_name };
                    for (n, e) in &items {
                        use crate::ast_export::ConstExprNames;
                        let show = match names.fold(e) {
                            Some(v) => AttrArgShow::Value(v),
                            None => AttrArgShow::Ast(PhpStr::from_bytes(
                                crate::ast_export::const_expr(cx.src, e, &names),
                            )),
                        };
                        arg_show.push((n.map(PhpStr::from_bytes), show));
                    }
                    let folded: Result<Vec<Value>, Thrown> =
                        items.iter().map(|(_, e)| fold_const(cx, e)).collect();
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
                            let mut c = ProtoBuilder::new(cx, PhpStr::from("{init}"));
                            c.class_name = class_name.cloned();
                            c.scope_kind = ScopeKind::Init;
                            c.closure_qual = cx.attr_fn_scope.borrow().clone();
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
                            let mut proto = c.finish()?;
                            proto.flags.insert(rhp_bytecode::ProtoFlags::CONST_EXPR);
                            if class_name.is_none() {
                                // No class scope: zend_ast_evaluate_ex sets
                                // no file/line override.
                                proto
                                    .flags
                                    .insert(rhp_bytecode::ProtoFlags::CONST_EXPR_PLAIN);
                            }
                            Some(ConstInitProto::Thunk(Rc::new(proto)))
                        }
                    }
                }
            };
            out.push(rhp_bytecode::AttributeProto {
                name,
                args,
                validation_error: None,
                arg_show,
                file: cx.file.clone(),
                line: cx.line_of(a.name.span().start.offset),
                strict_types: cx.strict_types,
            });
        }
    }
    crate::attrs::validate(cx, &mut out, target);
    Ok(out)
}

/// How zend_compile_const_expr leaves the names in an attribute argument.
struct AttrNames<'c, 's> {
    cx: &'c FileCtx<'s>,
    class_name: Option<&'c PhpStr>,
}

impl AttrNames<'_, '_> {
    /// `self`, `parent` or `static`, which stay as written.
    fn special(raw: &[u8]) -> bool {
        [&b"self"[..], b"parent", b"static"]
            .iter()
            .any(|k| raw.eq_ignore_ascii_case(k))
    }

    fn fq(name: Vec<u8>) -> Vec<u8> {
        let mut v = b"\\".to_vec();
        v.extend(name);
        v
    }
}

impl crate::ast_export::ConstExprNames for AttrNames<'_, '_> {
    fn fold(&self, e: &Expression) -> Option<Value> {
        use crate::parse::cst::{ClassLikeConstantSelector as Sel, MagicConstant};
        // The class names zend_eval_const_expr resolves at compile time:
        // `X::class`, and `self::class` / `__CLASS__` outside a trait.
        let own = || match self.class_name {
            Some(c) if !self.cx.in_trait.get() => Some(Value::Str(c.clone())),
            _ => None,
        };
        match crate::util::unparen(e) {
            Expression::Access(Access::ClassConstant(c)) if matches!(&c.constant, Sel::Identifier(id) if id.value.eq_ignore_ascii_case(b"class")) =>
            {
                return match crate::util::unparen(c.class) {
                    Expression::Identifier(id) if !Self::special(identifier_raw(id)) => Some(
                        Value::Str(PhpStr::from_bytes(self.cx.resolve_class_name(id))),
                    ),
                    Expression::Self_(_) => own(),
                    Expression::Identifier(id)
                        if identifier_raw(id).eq_ignore_ascii_case(b"self") =>
                    {
                        own()
                    }
                    _ => None,
                };
            }
            Expression::MagicConstant(MagicConstant::Class(_)) => {
                return match self.class_name {
                    None => Some(Value::str(Vec::new())),
                    Some(_) => own(),
                };
            }
            _ => {}
        }
        fold_const(self.cx, e).ok()
    }

    fn class_ref(&self, id: &Identifier) -> Vec<u8> {
        let raw = identifier_raw(id);
        if Self::special(raw) {
            return raw.to_vec();
        }
        Self::fq(self.cx.resolve_class_name(id))
    }

    fn class_const_class(&self, id: &Identifier) -> Vec<u8> {
        let raw = identifier_raw(id);
        if Self::special(raw) {
            return raw.to_vec();
        }
        // ZEND_NAME_FQ only when resolution produced a new name.
        let ns = self.cx.ns();
        let first = raw.split(|&b| b == b'\\').next().unwrap_or_default();
        let unchanged = match id {
            Identifier::FullyQualified(_) => false,
            _ => {
                ns.ns.is_empty()
                    && !ns.use_class.contains_key(&first.to_ascii_lowercase())
                    && !crate::names::starts_with_ci(raw, b"namespace\\")
            }
        };
        drop(ns);
        if unchanged {
            raw.to_vec()
        } else {
            Self::fq(self.cx.resolve_class_name(id))
        }
    }

    fn function(&self, id: &Identifier) -> Vec<u8> {
        let (name, fallback) = self.cx.resolve_name(id, crate::names::NameKind::Func);
        let fully_qualified = match id {
            Identifier::Local(l) => {
                fallback.is_none()
                    && self
                        .cx
                        .ns()
                        .use_func
                        .contains_key(&l.value.to_ascii_lowercase())
            }
            _ => true,
        };
        if fully_qualified {
            Self::fq(name)
        } else {
            name
        }
    }

    fn constant(&self, id: &Identifier) -> Vec<u8> {
        self.cx.resolve_name(id, crate::names::NameKind::Const).0
    }

    fn closure_name(&self, c: &crate::parse::cst::Closure) -> Vec<u8> {
        let qual = self
            .cx
            .attr_fn_scope
            .borrow()
            .clone()
            .unwrap_or_else(|| self.cx.file_name());
        let mut s = b"{closure:".to_vec();
        s.extend(qual);
        s.push(b':');
        s.extend(
            self.cx
                .line_of(c.span().start.offset)
                .to_string()
                .into_bytes(),
        );
        s.push(b'}');
        s
    }
}

/// The attribute target of a class-like declaration.
fn class_target<'a>(
    kind: DeclKind,
    name: &'a PhpStr,
    is_abstract: bool,
    is_readonly: bool,
    at: u32,
) -> AttrTarget<'a> {
    // An anonymous class's name is printed up to its NUL byte.
    let n = name.as_bytes();
    let n = &n[..n.iter().position(|&b| b == 0).unwrap_or(n.len())];
    AttrTarget {
        target: crate::attrs::TARGET_CLASS,
        promoted: 0,
        class: Some(crate::attrs::ClassInfo {
            kind,
            name: n,
            is_abstract,
            is_readonly,
        }),
        hook: false,
        at,
    }
}

/// Reflection metadata of a declaration: its attributes, the doc comment
/// claimed at `doc_pos`, and the line span `[start, end]` (byte offsets).
pub(crate) fn decl_meta(
    cx: &FileCtx,
    lists: &crate::parse::cst::Sequence<crate::parse::cst::AttributeList>,
    class_name: Option<&PhpStr>,
    target: &crate::attrs::AttrTarget,
    doc_pos: u32,
    start: u32,
    end: u32,
) -> Result<rhp_bytecode::DeclMeta, Thrown> {
    Ok(rhp_bytecode::DeclMeta {
        attributes: compile_attributes(cx, lists, class_name, target)?,
        doc_comment: cx.doc_comment_before(doc_pos),
        start_line: cx.line_of(start),
        offset: start,
        end_line: cx.line_of(end.saturating_sub(1).max(start)),
    })
}

/// Reflection data for a parameter default: a standalone thunk for a
/// non-constant default, and the constant's name when the default is a bare
/// constant / class-constant reference.
pub(crate) fn param_default_meta(
    cx: &FileCtx,
    p: &crate::parse::cst::FunctionLikeParameter,
    class_name: Option<&PhpStr>,
) -> (Option<Rc<FunctionProto>>, Option<PhpStr>) {
    let Some(dv) = &p.default_value else {
        return (None, None);
    };
    let e = dv.value;
    let cname = match e {
        // Names as php's compiled AST holds them: namespace-resolved.
        Expression::ConstantAccess(ca) => {
            let n = identifier_raw(&ca.name);
            let n = n.strip_prefix(b"\\").unwrap_or(n);
            let lcn = n.to_ascii_lowercase();
            (!matches!(lcn.as_slice(), b"true" | b"false" | b"null")).then(|| {
                PhpStr::from_bytes(cx.resolve_name(&ca.name, crate::names::NameKind::Const).0)
            })
        }
        Expression::Access(Access::ClassConstant(cca)) => {
            let class: Option<Vec<u8>> = match cca.class {
                Expression::Identifier(id) => Some(cx.resolve_class_name(id)),
                Expression::Self_(_) => Some(b"self".to_vec()),
                Expression::Static(_) => Some(b"static".to_vec()),
                Expression::Parent(_) => Some(b"parent".to_vec()),
                _ => None,
            };
            match (&cca.constant, class) {
                (crate::parse::cst::ClassLikeConstantSelector::Identifier(c), Some(mut cl)) => {
                    cl.extend_from_slice(b"::");
                    cl.extend_from_slice(c.value);
                    Some(PhpStr::from_bytes(cl))
                }
                _ => None,
            }
        }
        _ => None,
    };
    let thunk = if fold_const(cx, e).is_err() {
        compile_const_thunk(cx, e, class_name).ok()
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
pub(crate) fn span_of(n: &impl mago_span::HasSpan) -> (u32, u32) {
    let s = n.span();
    (s.start.offset, s.end.offset)
}

pub(crate) fn compile_class(cx: &FileCtx, class: &Class) -> Result<ClassProto, Thrown> {
    let name = PhpStr::from_bytes(join_ns(&cx.cur_ns(), class.name.value));
    check_decl(
        cx,
        &Decl {
            kind: DeclKind::Class,
            name: &name,
            short: Some(class.name.value),
            at: class.class.span.start.offset,
            extends: class.extends.as_ref(),
            implements: class.implements.as_ref(),
            is_abstract: has_modifier(&class.modifiers, |m| matches!(m, Modifier::Abstract(_))),
            is_readonly: has_modifier(&class.modifiers, |m| matches!(m, Modifier::Readonly(_))),
            backing: None,
            members: &class.members,
        },
    );
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
            .map(|id| PhpStr::from_bytes(cx.resolve_class_name(id)))
    });
    let interfaces: Vec<PhpStr> = class
        .implements
        .as_ref()
        .map(|i| {
            i.types
                .iter()
                .map(|id| PhpStr::from_bytes(cx.resolve_class_name(id)))
                .collect()
        })
        .unwrap_or_default();

    let scope = ClassScope {
        name: name.as_bytes().to_vec(),
        parent: parent.as_ref().map(|p| p.as_bytes().to_vec()),
        is_trait: false,
        is_anon: false,
    };
    let saved = cx.in_readonly_class.replace(flags.is_readonly);
    let m = cx.with_class_scope(scope, || compile_members(cx, &class.members, &name));
    cx.in_readonly_class.set(saved);
    let m = m?;
    let mut interfaces = interfaces;
    add_stringable(&mut interfaces, &m.methods, &name);
    let meta = decl_meta(
        cx,
        &class.attribute_lists,
        Some(&name),
        &class_target(
            DeclKind::Class,
            &name,
            has_modifier(&class.modifiers, |m| matches!(m, Modifier::Abstract(_))),
            has_modifier(&class.modifiers, |m| matches!(m, Modifier::Readonly(_))),
            class.class.span.start.offset,
        ),
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
        trait_rules: m.trait_rules(),
        traits: m.traits,
        methods: m.methods,
        props: m.props,
        static_props: m.static_props,
        constants: m.constants,
        enum_cases: Vec::new(),
        enum_backing: None,
        meta,
        decl_file: cx.file.clone(),
        decl_line: cx.line_of(class.class.span.start.offset),
    })
}

thread_local! {
    /// CG(rtd_key_counter) as zend_generate_anon_class_name uses it: one
    /// count per thread, so every compile (each eval, a file included
    /// twice) declares classes of its own.
    static ANON_CLASS_COUNTER: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Compile an anonymous class body under its generated `name`
/// (`Prefix@anonymous\0file:line$n`, zend_generate_anon_class_name).
pub(crate) fn compile_anon_class(
    cx: &FileCtx,
    ac: &crate::parse::cst::AnonymousClass,
) -> Result<ClassProto, Thrown> {
    use mago_span::HasSpan;
    let parent = ac.extends.as_ref().and_then(|e| {
        e.types
            .iter()
            .next()
            .map(|id| PhpStr::from_bytes(cx.resolve_class_name(id)))
    });
    let interfaces: Vec<PhpStr> = ac
        .implements
        .as_ref()
        .map(|i| {
            i.types
                .iter()
                .map(|id| PhpStr::from_bytes(cx.resolve_class_name(id)))
                .collect()
        })
        .unwrap_or_default();
    let mut name = parent
        .as_ref()
        .or(interfaces.first())
        .map(|n| n.as_bytes().to_vec())
        .unwrap_or_else(|| b"class".to_vec());
    name.extend_from_slice(b"@anonymous\0");
    name.extend_from_slice(&cx.file_name());
    let n = ANON_CLASS_COUNTER.with(|c| {
        let n = c.get();
        c.set(n.wrapping_add(1));
        n
    });
    name.extend_from_slice(format!(":{}${:x}", cx.line_of(ac.span().start.offset), n).as_bytes());
    let name = PhpStr::from_bytes(name);
    check_decl(
        cx,
        &Decl {
            kind: DeclKind::Anon,
            name: &name,
            short: None,
            at: ac.class.span.start.offset,
            extends: ac.extends.as_ref(),
            implements: ac.implements.as_ref(),
            is_abstract: false,
            is_readonly: has_modifier(&ac.modifiers, |m| matches!(m, Modifier::Readonly(_))),
            backing: None,
            members: &ac.members,
        },
    );
    let flags = ClassFlags {
        is_abstract: false,
        is_final: true,
        is_readonly: has_modifier(&ac.modifiers, |m| matches!(m, Modifier::Readonly(_))),
        allow_dynamic: has_allow_dynamic(&ac.attribute_lists),
        // zend_compile_class_decl: every anonymous class.
        not_serializable: true,
        ..ClassFlags::default()
    };
    let scope = ClassScope {
        name: name.as_bytes().to_vec(),
        parent: parent.as_ref().map(|p| p.as_bytes().to_vec()),
        is_trait: false,
        is_anon: true,
    };
    let saved = cx.in_readonly_class.replace(flags.is_readonly);
    let m = cx.with_class_scope(scope, || compile_members(cx, &ac.members, &name));
    cx.in_readonly_class.set(saved);
    let m = m?;
    let mut interfaces = interfaces;
    add_stringable(&mut interfaces, &m.methods, &name);
    let meta = decl_meta(
        cx,
        &ac.attribute_lists,
        Some(&name),
        &class_target(
            DeclKind::Anon,
            &name,
            false,
            has_modifier(&ac.modifiers, |m| matches!(m, Modifier::Readonly(_))),
            ac.class.span.start.offset,
        ),
        ac.class.span.start.offset,
        ac.class.span.start.offset,
        ac.right_brace.end.offset,
    )?;
    Ok(ClassProto {
        decl_file: cx.file.clone(),
        decl_line: cx.line_of(ac.class.span.start.offset),
        name,
        kind: ClassKind::Class,
        flags,
        parent,
        interfaces,
        trait_rules: m.trait_rules(),
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
pub(crate) fn compile_enum(
    cx: &FileCtx,
    en: &crate::parse::cst::Enum,
) -> Result<ClassProto, Thrown> {
    use rhp_bytecode::EnumBacking;
    let name = PhpStr::from_bytes(join_ns(&cx.cur_ns(), en.name.value));
    check_decl(
        cx,
        &Decl {
            kind: DeclKind::Enum,
            name: &name,
            short: Some(en.name.value),
            at: en.r#enum.span.start.offset,
            extends: None,
            implements: en.implements.as_ref(),
            is_abstract: false,
            is_readonly: false,
            backing: en.backing_type_hint.as_ref().map(|h| &h.hint),
            members: &en.members,
        },
    );

    // An invalid backing type is a compile error (check_decl).
    let backing = en.backing_type_hint.as_ref().map(|h| match &h.hint {
        Hint::String(_) => EnumBacking::Str,
        _ => EnumBacking::Int,
    });

    let mut interfaces: Vec<PhpStr> = en
        .implements
        .as_ref()
        .map(|i| {
            i.types
                .iter()
                .map(|id| PhpStr::from_bytes(cx.resolve_class_name(id)))
                .collect()
        })
        .unwrap_or_default();
    // Every enum implements UnitEnum; a backed enum also implements BackedEnum.
    interfaces.push(PhpStr::from("UnitEnum"));
    if backing.is_some() {
        interfaces.push(PhpStr::from("BackedEnum"));
    }

    let scope = ClassScope {
        name: name.as_bytes().to_vec(),
        parent: None,
        is_trait: false,
        is_anon: false,
    };
    let m = cx.with_class_scope(scope, || compile_members(cx, &en.members, &name))?;
    add_stringable(&mut interfaces, &m.methods, &name);
    let meta = decl_meta(
        cx,
        &en.attribute_lists,
        Some(&name),
        &class_target(
            DeclKind::Enum,
            &name,
            false,
            false,
            en.r#enum.span.start.offset,
        ),
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
        trait_rules: m.trait_rules(),
        traits: m.traits,
        methods: m.methods,
        props: m.props,
        static_props: m.static_props,
        constants: m.constants,
        enum_cases: m.enum_cases,
        enum_backing: backing,
        decl_file: cx.file.clone(),
        decl_line: cx.line_of(en.r#enum.span.start.offset),
    })
}

/// Compile a (possibly non-constant) initializer expression — a class constant,
/// static-property, or instance-property default — into a zero-argument thunk
/// the VM evaluates in the class's scope after all classes are linked. This is
/// how `const X = self::Y` / `private State $s = State::Idle` resolve.
fn compile_const_thunk(
    cx: &FileCtx,
    expr: &Expression,
    class_name: Option<&PhpStr>,
) -> Result<Rc<FunctionProto>, Thrown> {
    let mut c = ProtoBuilder::new(cx, PhpStr::from("{init}"));
    c.class_name = class_name.cloned(); // for `__CLASS__`
    c.scope_kind = ScopeKind::Init;
    c.scan_expr_vars(expr); // const-exprs use no $vars, but be safe
    c.scan_node_vars(Node::Expression(expr));
    c.freeze_cvs();
    let r = c.compile_expr(expr)?;
    c.emit(Instr::Ret { src: r });
    let mut proto = c.finish()?;
    proto.flags.insert(rhp_bytecode::ProtoFlags::CONST_EXPR);
    // A plain global constant name, or no class scope (a function's
    // parameter default): php sets no file/line override.
    if matches!(expr, Expression::ConstantAccess(_)) || class_name.is_none() {
        proto
            .flags
            .insert(rhp_bytecode::ProtoFlags::CONST_EXPR_PLAIN);
    }
    Ok(Rc::new(proto))
}

/// zend_is_scope_known while compiling a class member.
fn member_scope_known(cx: &FileCtx) -> bool {
    cx.class_scope
        .borrow()
        .as_ref()
        .is_some_and(|cs| !cs.is_trait)
}

/// Fold an initializer to a constant, or fall back to a runtime thunk.
fn const_init_or_thunk(
    cx: &FileCtx,
    expr: &Expression,
    class_name: Option<&PhpStr>,
) -> Result<ConstInitProto, Thrown> {
    crate::constexpr::check_const_expr(cx, expr, false, member_scope_known(cx));
    match fold_const(cx, expr) {
        Ok(v) => Ok(ConstInitProto::Const(v)),
        Err(_) => Ok(ConstInitProto::Thunk(compile_const_thunk(
            cx, expr, class_name,
        )?)),
    }
}

fn prop_default_or_thunk(
    cx: &FileCtx,
    expr: &Expression,
    class_name: Option<&PhpStr>,
) -> Result<PropDefaultProto, Thrown> {
    crate::constexpr::check_const_expr(cx, expr, false, member_scope_known(cx));
    match fold_const(cx, expr) {
        Ok(v) => Ok(PropDefaultProto::Const(v)),
        Err(_) => Ok(PropDefaultProto::Init(compile_const_thunk(
            cx, expr, class_name,
        )?)),
    }
}

/// zend_compile_prop_decl's type checks: the type itself, then a constant
/// default against it (an int default of a float type becomes a float).
fn check_prop_default(
    cx: &FileCtx,
    ty: &PhpType,
    default: PropDefaultProto,
    class_name: &PhpStr,
    var: &crate::parse::cst::DirectVariable,
    at: u32,
) -> PropDefaultProto {
    let cname = String::from_utf8_lossy(class_name.as_bytes());
    let pname = String::from_utf8_lossy(strip_dollar(var.name));
    if ty.has(MAY_VOID | MAY_NEVER | MAY_CALLABLE) {
        cx.fatal(
            at,
            format!(
                "Property {cname}::${pname} cannot have type {}",
                ty.to_php_string()
            ),
        );
        return default;
    }
    let PropDefaultProto::Const(mut v) = default else {
        return default;
    };
    if !ty.accepts_default(&mut v) {
        let t = ty.to_php_string();
        let msg = if matches!(v, Value::Null) && !ty.is_intersection() {
            format!(
                "Default value for property of type {t} may not be null. Use the nullable type {} to allow null default value",
                ty.with_null().to_php_string()
            )
        } else {
            let vname = match v {
                Value::Bool(true) => "true",
                Value::Bool(false) => "false",
                ref other => const_type_name(other),
            };
            format!(
                "Cannot use {vname} as default value for property {cname}::${pname} of type {t}"
            )
        };
        cx.fatal(at, msg);
    }
    PropDefaultProto::Const(v)
}

/// zend_compile_class_const_decl's type checks.
fn check_const_value(
    cx: &FileCtx,
    ty: &PhpType,
    value: ConstInitProto,
    class_name: &PhpStr,
    name: &[u8],
    at: u32,
) -> ConstInitProto {
    let cname = String::from_utf8_lossy(class_name.as_bytes());
    let name = String::from_utf8_lossy(name);
    if !ty.is_mixed() && ty.has(MAY_VOID | MAY_NEVER | MAY_CALLABLE) {
        cx.fatal(
            at,
            format!(
                "Class constant {cname}::{name} cannot have type {}",
                ty.to_php_string()
            ),
        );
        return value;
    }
    let ConstInitProto::Const(mut v) = value else {
        return value;
    };
    if !ty.accepts_default(&mut v) {
        cx.fatal(
            at,
            format!(
                "Cannot use {} as value for class constant {cname}::{name} of type {}",
                const_type_name(&v),
                ty.to_php_string()
            ),
        );
    }
    ConstInitProto::Const(v)
}

#[derive(Default)]
struct CompiledMembers {
    methods: Vec<MethodProto>,
    props: Vec<PropProto>,
    static_props: Vec<StaticPropProto>,
    constants: Vec<ConstProto>,
    traits: Vec<PhpStr>,
    trait_rules: rhp_bytecode::TraitRules,
    enum_cases: Vec<rhp_bytecode::EnumCaseProto>,
}

impl CompiledMembers {
    fn trait_rules(&self) -> Option<Box<rhp_bytecode::TraitRules>> {
        let r = &self.trait_rules;
        (!r.precedences.is_empty() || !r.aliases.is_empty()).then(|| Box::new(r.clone()))
    }
}

fn compile_members(
    cx: &FileCtx,
    members: &crate::parse::cst::Sequence<ClassLikeMember>,
    class_name: &PhpStr,
) -> Result<CompiledMembers, Thrown> {
    let mut out = CompiledMembers::default();
    for member in members.iter() {
        use mago_span::HasSpan;
        compile_member(cx, &mut out, member, class_name)
            .map_err(|e| cx.locate(e, member.span().start.offset))?;
    }
    Ok(out)
}

/// Compile one class-like member into `out`.
fn compile_member(
    cx: &FileCtx,
    out: &mut CompiledMembers,
    member: &ClassLikeMember,
    class_name: &PhpStr,
) -> Result<(), Thrown> {
    match member {
        ClassLikeMember::Method(m) => {
            let (mp, promoted) = compile_method(cx, m, class_name)?;
            out.methods.push(mp);
            out.props.extend(promoted);
        }
        ClassLikeMember::Property(Property::Plain(p)) => {
            let items: Vec<&PropertyItem> = p.items.iter().collect();
            compile_property(
                cx,
                out,
                PropDecl {
                    attribute_lists: &p.attribute_lists,
                    modifiers: &p.modifiers,
                    hint: p.hint.as_ref(),
                    items: &items,
                    hooks: None,
                    span: span_of(p),
                },
                class_name,
            )?;
        }
        ClassLikeMember::Property(Property::Hooked(p)) => {
            compile_property(
                cx,
                out,
                PropDecl {
                    attribute_lists: &p.attribute_lists,
                    modifiers: &p.modifiers,
                    hint: p.hint.as_ref(),
                    items: &[&p.item],
                    hooks: Some(&p.hook_list),
                    span: span_of(p),
                },
                class_name,
            )?;
        }
        ClassLikeMember::Constant(c) => {
            let visibility = modifiers_visibility(&c.modifiers);
            let compiled = c.hint.as_ref().and_then(|h| compile_type(cx, h, false));
            let hint = compiled.as_ref().map(|t| t.hint.clone());
            let is_final = has_modifier(&c.modifiers, |m| matches!(m, Modifier::Final(_)));
            let (cstart, cend) = span_of(c);
            let first = c
                .items
                .iter()
                .next()
                .map_or(cstart, |i| i.name.span.start.offset);
            let attributes = compile_attributes(
                cx,
                &c.attribute_lists,
                Some(class_name),
                &AttrTarget::new(crate::attrs::TARGET_CLASS_CONST, first),
            )?;
            for item in c.items.iter() {
                let value = const_init_or_thunk(cx, item.value, Some(class_name))?;
                let value = match &compiled {
                    Some(t) => {
                        check_const_value(cx, &t.ty, value, class_name, item.name.value, cstart)
                    }
                    None => value,
                };
                out.constants.push(ConstProto {
                    name: PhpStr::from_bytes(item.name.value),
                    visibility,
                    value,
                    type_hint: hint.clone(),
                    meta: rhp_bytecode::DeclMeta {
                        attributes: attributes.clone(),
                        doc_comment: cx.doc_comment_before(item.name.span.start.offset),
                        start_line: cx.line_of(cstart),
                        offset: item.name.span.start.offset,
                        end_line: cx.line_of(cend),
                    },
                    is_final,
                });
            }
        }
        ClassLikeMember::TraitUse(tu) => {
            use crate::parse::cst::{
                TraitUseAdaptation, TraitUseMethodReference, TraitUseSpecification,
            };
            for t in tu.trait_names.iter() {
                out.traits
                    .push(PhpStr::from_bytes(cx.resolve_class_name(t)));
            }
            // zend_compile_trait_precedence / zend_compile_trait_alias.
            if let TraitUseSpecification::Concrete(c) = &tu.specification {
                let name = |t| PhpStr::from_bytes(cx.resolve_class_name(t));
                for a in c.adaptations.iter() {
                    match a {
                        TraitUseAdaptation::Precedence(p) => {
                            let r = &p.method_reference;
                            out.trait_rules
                                .precedences
                                .push(rhp_bytecode::TraitPrecedence {
                                    trait_name: name(&r.trait_name),
                                    method: PhpStr::from_bytes(r.method_name.value),
                                    excludes: p.trait_names.iter().map(name).collect(),
                                });
                        }
                        TraitUseAdaptation::Alias(al) => {
                            let (trait_name, method) = match &al.method_reference {
                                TraitUseMethodReference::Identifier(m) => (None, m.value),
                                TraitUseMethodReference::Absolute(r) => {
                                    (Some(name(&r.trait_name)), r.method_name.value)
                                }
                            };
                            let (visibility, is_final) = match &al.modifier {
                                Some(Modifier::Public(_)) => (Some(Visibility::Public), false),
                                Some(Modifier::Protected(_)) => {
                                    (Some(Visibility::Protected), false)
                                }
                                Some(Modifier::Private(_)) => (Some(Visibility::Private), false),
                                Some(Modifier::Final(_)) => (None, true),
                                _ => (None, false),
                            };
                            out.trait_rules.aliases.push(rhp_bytecode::TraitAlias {
                                trait_name,
                                method: PhpStr::from_bytes(method),
                                alias: al.alias.as_ref().map(|a| PhpStr::from_bytes(a.value)),
                                visibility,
                                is_final,
                            });
                        }
                    }
                }
            }
        }
        ClassLikeMember::EnumCase(ec) => {
            use crate::parse::cst::EnumCaseItem;
            let (name, value) = match &ec.item {
                EnumCaseItem::Unit(u) => (PhpStr::from_bytes(u.name.value), None),
                EnumCaseItem::Backed(b) => (
                    PhpStr::from_bytes(b.name.value),
                    Some(const_init_or_thunk(cx, b.value, Some(class_name))?),
                ),
            };
            let name_off = match &ec.item {
                EnumCaseItem::Unit(u) => u.name.span.start.offset,
                EnumCaseItem::Backed(b) => b.name.span.start.offset,
            };
            let (cstart, cend) = span_of(ec);
            let meta = decl_meta(
                cx,
                &ec.attribute_lists,
                Some(class_name),
                &AttrTarget::new(crate::attrs::TARGET_CLASS_CONST, cstart),
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

/// One property declaration statement (php's ZEND_AST_PROP_GROUP).
struct PropDecl<'a> {
    attribute_lists: &'a crate::parse::cst::Sequence<'a, crate::parse::cst::AttributeList<'a>>,
    modifiers: &'a crate::parse::cst::Sequence<'a, Modifier<'a>>,
    hint: Option<&'a Hint<'a>>,
    items: &'a [&'a PropertyItem<'a>],
    hooks: Option<&'a crate::parse::cst::PropertyHookList<'a>>,
    span: (u32, u32),
}

/// zend_compile_prop_decl: the declaration checks, then one property (or
/// static property) per item, with its hooks.
fn compile_property(
    cx: &FileCtx,
    out: &mut CompiledMembers,
    p: PropDecl,
    class_name: &PhpStr,
) -> Result<(), Thrown> {
    let visibility = modifiers_visibility(p.modifiers);
    let is_static = has_modifier(p.modifiers, |m| matches!(m, Modifier::Static(_)));
    let is_readonly = has_modifier(p.modifiers, |m| matches!(m, Modifier::Readonly(_)));
    let set_visibility =
        readonly_set_visibility(p.modifiers, is_readonly || cx.in_readonly_class.get());
    let is_final = has_modifier(p.modifiers, |m| matches!(m, Modifier::Final(_)));
    let mut is_abstract = has_modifier(p.modifiers, |m| matches!(m, Modifier::Abstract(_)));
    let in_interface = cx.in_interface.get();
    let (pstart, _) = p.span;
    if is_final && visibility == Visibility::Private {
        cx.fatal(pstart, "Property cannot be both final and private".into());
        return Ok(());
    }
    if in_interface {
        if is_final {
            cx.fatal(pstart, "Property in interface cannot be final".into());
            return Ok(());
        }
        if visibility != Visibility::Public {
            cx.fatal(
                pstart,
                "Property in interface cannot be protected or private".into(),
            );
            return Ok(());
        }
        if is_abstract {
            cx.fatal(
                pstart,
                "Property in interface cannot be explicitly abstract. \
                 All interface members are implicitly abstract"
                    .into(),
            );
            return Ok(());
        }
        is_abstract = true;
    }
    let compiled = p.hint.and_then(|h| compile_type(cx, h, false));
    let first = p
        .items
        .first()
        .map(|i| strip_dollar(i.variable().name).to_vec());
    *cx.cur_property.borrow_mut() = first;
    let r = compile_property_items(
        cx,
        out,
        &p,
        class_name,
        compiled.as_ref(),
        is_abstract,
        visibility,
        set_visibility,
        is_static,
        is_readonly,
        is_final,
        in_interface,
    );
    *cx.cur_property.borrow_mut() = None;
    r
}

#[allow(clippy::too_many_arguments)]
fn compile_property_items(
    cx: &FileCtx,
    out: &mut CompiledMembers,
    p: &PropDecl,
    class_name: &PhpStr,
    compiled: Option<&crate::hints::CompiledType>,
    is_abstract: bool,
    visibility: Visibility,
    set_visibility: Option<Visibility>,
    is_static: bool,
    is_readonly: bool,
    is_final: bool,
    in_interface: bool,
) -> Result<(), Thrown> {
    let (pstart, pend) = p.span;
    let hint = compiled.map(|t| t.hint.clone());
    let first = p
        .items
        .first()
        .map_or(pstart, |i| i.variable().span.start.offset);
    let attributes = compile_attributes(
        cx,
        p.attribute_lists,
        Some(class_name),
        &AttrTarget::new(crate::attrs::TARGET_PROPERTY, first),
    )?;
    for item in p.items {
        *cx.cur_property.borrow_mut() = Some(strip_dollar(item.variable().name).to_vec());
        let (var, default) = match item {
            PropertyItem::Concrete(c) => (
                &c.variable,
                prop_default_or_thunk(cx, c.value, Some(class_name))?,
            ),
            PropertyItem::Abstract(a) => (&a.variable, PropDefaultProto::None),
        };
        let default = match compiled {
            Some(t) => check_prop_default(cx, &t.ty, default, class_name, var, pstart),
            None => default,
        };
        let name_bytes = strip_dollar(var.name);
        if p.hooks.is_none() {
            if in_interface {
                cx.fatal(
                    pstart,
                    "Interfaces may only include hooked properties".into(),
                );
                return Ok(());
            }
            if is_abstract {
                cx.fatal(
                    pstart,
                    "Only hooked properties may be declared abstract".into(),
                );
                return Ok(());
            }
        }
        let meta = rhp_bytecode::DeclMeta {
            attributes: attributes.clone(),
            doc_comment: cx.doc_comment_before(var.span.start.offset),
            start_line: cx.line_of(pstart),
            offset: var.span.start.offset,
            end_line: cx.line_of(pend),
        };
        let pname = PhpStr::from_bytes(name_bytes);
        let (hooks, is_virtual) = match p.hooks {
            Some(list) => {
                let hcx = crate::hooks::HookCtx {
                    class_name,
                    prop_name: name_bytes,
                    hint: p.hint,
                    visibility,
                    is_abstract,
                    is_static,
                    is_readonly: is_readonly || cx.in_readonly_class.get(),
                    in_interface,
                    offset: pstart,
                };
                (
                    crate::hooks::compile_hooks(cx, &hcx, list)?,
                    crate::hooks::property_is_virtual(name_bytes, list, in_interface),
                )
            }
            None => ([None, None], false),
        };
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
                set_visibility,
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
                hooks,
                is_virtual,
                is_abstract,
                is_final,
            });
        }
    }
    Ok(())
}

pub(crate) fn compile_trait(
    cx: &FileCtx,
    t: &crate::parse::cst::Trait,
) -> Result<ClassProto, Thrown> {
    let name = PhpStr::from_bytes(join_ns(&cx.cur_ns(), t.name.value));
    check_decl(
        cx,
        &Decl {
            kind: DeclKind::Trait,
            name: &name,
            short: Some(t.name.value),
            at: t.r#trait.span.start.offset,
            extends: None,
            implements: None,
            is_abstract: false,
            is_readonly: false,
            backing: None,
            members: &t.members,
        },
    );
    let saved = cx.in_trait.replace(true);
    let scope = ClassScope {
        name: name.as_bytes().to_vec(),
        parent: None,
        is_trait: true,
        is_anon: false,
    };
    let m = cx.with_class_scope(scope, || compile_members(cx, &t.members, &name));
    cx.in_trait.set(saved);
    let m = m?;
    let meta = decl_meta(
        cx,
        &t.attribute_lists,
        Some(&name),
        &class_target(
            DeclKind::Trait,
            &name,
            false,
            false,
            t.r#trait.span.start.offset,
        ),
        t.name.span.start.offset,
        t.r#trait.span.start.offset,
        t.right_brace.end.offset,
    )?;
    Ok(ClassProto {
        meta,
        decl_file: cx.file.clone(),
        decl_line: cx.line_of(t.r#trait.span.start.offset),
        name,
        kind: ClassKind::Trait,
        flags: ClassFlags::default(),
        parent: None,
        interfaces: Vec::new(),
        trait_rules: m.trait_rules(),
        traits: m.traits,
        methods: m.methods,
        props: m.props,
        static_props: m.static_props,
        constants: m.constants,
        enum_cases: Vec::new(),
        enum_backing: None,
    })
}

pub(crate) fn compile_interface(
    cx: &FileCtx,
    iface: &crate::parse::cst::Interface,
) -> Result<ClassProto, Thrown> {
    let name = PhpStr::from_bytes(join_ns(&cx.cur_ns(), iface.name.value));
    check_decl(
        cx,
        &Decl {
            kind: DeclKind::Interface,
            name: &name,
            short: Some(iface.name.value),
            at: iface.interface.span.start.offset,
            extends: iface.extends.as_ref(),
            implements: None,
            is_abstract: false,
            is_readonly: false,
            backing: None,
            members: &iface.members,
        },
    );
    let interfaces: Vec<PhpStr> = iface
        .extends
        .as_ref()
        .map(|e| {
            e.types
                .iter()
                .map(|id| PhpStr::from_bytes(cx.resolve_class_name(id)))
                .collect()
        })
        .unwrap_or_default();
    // Constants, (abstract) method signatures and (abstract, hooked)
    // properties.
    let saved = cx.in_interface.replace(true);
    let scope = ClassScope {
        name: name.as_bytes().to_vec(),
        parent: None,
        is_trait: false,
        is_anon: false,
    };
    let m = cx.with_class_scope(scope, || compile_members(cx, &iface.members, &name));
    cx.in_interface.set(saved);
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
        cx,
        &iface.attribute_lists,
        Some(&name),
        &class_target(
            DeclKind::Interface,
            &name,
            false,
            false,
            iface.interface.span.start.offset,
        ),
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
        trait_rules: None,
        methods,
        props: m.props,
        static_props: Vec::new(),
        constants: m.constants,
        enum_cases: Vec::new(),
        enum_backing: None,
        meta,
        decl_file: cx.file.clone(),
        decl_line: cx.line_of(iface.interface.span.start.offset),
    })
}
