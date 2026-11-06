//! Property hooks (`public $p { get => …; set { … } }`): each hook compiles
//! to a method named `$p::get` / `$p::set`, as php's
//! zend_compile_property_hooks does, plus the declaration checks php makes
//! at compile time.

use std::rc::Rc;

use crate::parse::cst::{
    ClassLikeMemberSelector, Expression, Hint, Literal, Modifier, Node, PropertyHook,
    PropertyHookBody, PropertyHookConcreteBody, PropertyHookList, Statement,
};
use rhp_bytecode::{HookKind, HookProto, Instr, ProtoFlags};
use rhp_values::{PhpStr, Thrown, Visibility};

use crate::builder::{ProtoBuilder, ScopeKind};
use crate::decl::{compile_attributes, decl_meta, span_of};
use crate::expr::{block_has_yield, expr_has_yield};
use crate::file::FileCtx;
use crate::hints::type_hint;
use crate::util::strip_dollar;

/// What a hooked property declaration says about its hooks.
pub(crate) struct HookCtx<'a> {
    pub(crate) class_name: &'a PhpStr,
    pub(crate) prop_name: &'a [u8],
    pub(crate) hint: Option<&'a Hint<'a>>,
    pub(crate) visibility: Visibility,
    pub(crate) is_abstract: bool,
    pub(crate) is_static: bool,
    pub(crate) is_readonly: bool,
    pub(crate) in_interface: bool,
    /// Where the property is declared (for errors without a hook).
    pub(crate) offset: u32,
}

/// zend_property_is_virtual: an interface property always is; a hooked
/// property is unless some hook body reads or writes `$this->prop` (a
/// short `set =>` body always does).
pub(crate) fn property_is_virtual(
    name: &[u8],
    hooks: &PropertyHookList,
    in_interface: bool,
) -> bool {
    if in_interface {
        return true;
    }
    !hooks.hooks.iter().any(|h| match &h.body {
        PropertyHookBody::Abstract(_) => false,
        PropertyHookBody::Concrete(PropertyHookConcreteBody::Expression(e)) => {
            h.name.value.eq_ignore_ascii_case(b"set")
                || uses_property(name, Node::Expression(e.expression))
        }
        PropertyHookBody::Concrete(PropertyHookConcreteBody::Block(b)) => b
            .statements
            .iter()
            .any(|s| uses_property(name, Node::Statement(s))),
    })
}

/// zend_property_hook_find_property_usage: a `$this->name` /
/// `$this?->name` anywhere in `node`, not looking into nested functions or
/// classes.
fn uses_property(name: &[u8], node: Node) -> bool {
    let (object, selector) = match node {
        Node::PropertyAccess(pa) => (Some(pa.object), Some(&pa.property)),
        Node::NullSafePropertyAccess(pa) => (Some(pa.object), Some(&pa.property)),
        Node::Closure(_)
        | Node::ArrowFunction(_)
        | Node::AnonymousClass(_)
        | Node::Function(_)
        | Node::Class(_)
        | Node::Interface(_)
        | Node::Trait(_)
        | Node::Enum(_) => return false,
        _ => (None, None),
    };
    if let (Some(Expression::Variable(crate::parse::cst::Variable::Direct(d))), Some(sel)) =
        (object, selector)
    {
        let prop: Option<&[u8]> = match sel {
            ClassLikeMemberSelector::Identifier(id) => Some(id.value),
            ClassLikeMemberSelector::Expression(e) => match e.expression {
                Expression::Literal(Literal::String(s)) => s.value,
                _ => None,
            },
            _ => None,
        };
        if d.name == b"$this" && prop == Some(name) {
            return true;
        }
    }
    let mut found = false;
    node.visit_children(|c| {
        if !found && uses_property(name, c) {
            found = true;
        }
    });
    found
}

fn hook_kind(name: &[u8]) -> Option<HookKind> {
    if name.eq_ignore_ascii_case(b"get") {
        Some(HookKind::Get)
    } else if name.eq_ignore_ascii_case(b"set") {
        Some(HookKind::Set)
    } else {
        None
    }
}

/// zend_compile_property_hooks: compile the hook list of one property.
/// Errors go to `cx.fatal`; the hooks compiled so far are returned anyway.
pub(crate) fn compile_hooks(
    cx: &FileCtx,
    p: &HookCtx,
    list: &PropertyHookList,
) -> Result<[Option<HookProto>; 2], Thrown> {
    let mut out: [Option<HookProto>; 2] = [None, None];
    let cname = String::from_utf8_lossy(p.class_name.as_bytes()).into_owned();
    let pname = String::from_utf8_lossy(p.prop_name).into_owned();
    if p.is_readonly {
        cx.fatal(p.offset, "Hooked properties cannot be readonly".into());
        return Ok(out);
    }
    if list.hooks.is_empty() {
        cx.fatal(p.offset, "Property hook list must not be empty".into());
        return Ok(out);
    }
    for hook in list.hooks.iter() {
        let at = span_of(hook).0;
        let fail = |msg: String| {
            cx.fatal(at, msg);
        };
        let mut is_final = false;
        for m in hook.modifiers.iter() {
            match m {
                Modifier::Final(_) => is_final = true,
                other => {
                    let word = String::from_utf8_lossy(other.get_keyword().value).to_lowercase();
                    fail(format!("Cannot use the {word} modifier on a property hook"));
                    return Ok(out);
                }
            }
        }
        let private = p.visibility == Visibility::Private;
        let has_body = matches!(hook.body, PropertyHookBody::Concrete(_));
        if p.is_static {
            fail("Cannot declare hooks for static property".into());
            return Ok(out);
        }
        if is_final && private {
            fail("Property hook cannot be both final and private".into());
            return Ok(out);
        }
        let is_abstract = p.in_interface || (p.is_abstract && !has_body);
        if is_abstract {
            if has_body {
                fail("Abstract property hook cannot have body".into());
                return Ok(out);
            }
            if private {
                fail("Property hook cannot be both abstract and private".into());
                return Ok(out);
            }
            if is_final {
                fail("Property hook cannot be both abstract and final".into());
                return Ok(out);
            }
        } else if !has_body {
            fail("Non-abstract property hook must have a body".into());
            return Ok(out);
        }
        let Some(kind) = hook_kind(hook.name.value) else {
            fail(format!(
                "Unknown hook \"{}\" for property {cname}::${pname}, expected \"get\" or \"set\"",
                String::from_utf8_lossy(hook.name.value)
            ));
            return Ok(out);
        };
        let hname = hook.name.value;
        match kind {
            HookKind::Get if hook.parameter_list.is_some() => {
                fail(format!(
                    "get hook of property {cname}::${pname} must not have a parameter list"
                ));
                return Ok(out);
            }
            HookKind::Set => {
                if let Some(pl) = &hook.parameter_list {
                    let params: Vec<_> = pl.parameters.iter().collect();
                    let hn = String::from_utf8_lossy(hname);
                    if params.len() != 1 {
                        fail(format!(
                            "{hn} hook of property {cname}::${pname} must accept exactly one parameters"
                        ));
                        return Ok(out);
                    }
                    let vp = params[0];
                    let vname = String::from_utf8_lossy(strip_dollar(vp.variable.name));
                    if vp.ampersand.is_some() {
                        fail(format!(
                            "Parameter ${vname} of {hn} hook {cname}::${pname} must not be pass-by-reference"
                        ));
                        return Ok(out);
                    }
                    if vp.ellipsis.is_some() {
                        fail(format!(
                            "Parameter ${vname} of {hn} hook {cname}::${pname} must not be variadic"
                        ));
                        return Ok(out);
                    }
                    if vp.default_value.is_some() {
                        fail(format!(
                            "Parameter ${vname} of {hn} hook {cname}::${pname} must not have a default value"
                        ));
                        return Ok(out);
                    }
                    if p.hint.is_some() != vp.hint.is_some() {
                        fail(format!(
                            "Type of parameter ${vname} of hook {cname}::${pname}::set must be compatible with property type"
                        ));
                        return Ok(out);
                    }
                }
            }
            HookKind::Get => {}
        }
        if out[kind as usize].is_some() {
            fail(format!(
                "Cannot redeclare property hook \"{}\"",
                String::from_utf8_lossy(hname)
            ));
            return Ok(out);
        }
        let proto = compile_hook_body(cx, p, hook, kind)?;
        out[kind as usize] = Some(HookProto {
            proto: Rc::new(proto),
            is_final,
            is_abstract,
        });
    }
    Ok(out)
}

/// One hook as a method: `get` takes no parameter and returns the property
/// type; `set` takes `$value` (or its declared parameter) and returns void.
fn compile_hook_body(
    cx: &FileCtx,
    p: &HookCtx,
    hook: &PropertyHook,
    kind: HookKind,
) -> Result<rhp_bytecode::FunctionProto, Thrown> {
    let mut name = b"$".to_vec();
    name.extend_from_slice(p.prop_name);
    name.extend_from_slice(b"::");
    name.extend_from_slice(kind.name().as_bytes());
    let mut c = ProtoBuilder::new(cx, PhpStr::from_bytes(name));
    c.class_name = Some(p.class_name.clone());
    c.scope_kind = ScopeKind::Method;
    c.returns_ref = hook.ampersand.is_some();
    c.hook = Some((p.prop_name.to_vec(), kind));
    let this = c.cv(b"this");
    debug_assert_eq!(this, 0);
    c.this_bound = true;

    let mut params = Vec::new();
    if kind == HookKind::Set {
        let (var, hint, attrs, line) = match hook
            .parameter_list
            .as_ref()
            .and_then(|pl| pl.parameters.iter().next())
        {
            Some(vp) => {
                use mago_span::HasSpan;
                (
                    strip_dollar(vp.variable.name).to_vec(),
                    vp.hint
                        .as_ref()
                        .and_then(|h| crate::hints::compile_type(cx, h, false).map(|t| t.hint)),
                    compile_attributes(
                        cx,
                        &vp.attribute_lists,
                        Some(p.class_name),
                        &crate::attrs::AttrTarget::new(
                            crate::attrs::TARGET_PARAMETER,
                            vp.variable.span.start.offset,
                        ),
                    )?,
                    cx.line_of(vp.span().start.offset),
                )
            }
            None => (
                b"value".to_vec(),
                p.hint.and_then(|h| type_hint(cx, h)),
                Vec::new(),
                cx.line_of(span_of(hook).0),
            ),
        };
        let cv = c.cv(&var);
        let pname = c.intern_name(&var);
        params.push(rhp_bytecode::ParamInfo {
            name: pname,
            cv,
            by_ref: false,
            variadic: false,
            default: None,
            type_hint: hint,
            line,
            attributes: attrs,
            promoted: false,
            default_thunk: None,
            default_const: None,
        });
    }

    let mut is_generator = false;
    match &hook.body {
        PropertyHookBody::Abstract(_) => {
            c.freeze_cvs();
        }
        PropertyHookBody::Concrete(PropertyHookConcreteBody::Block(b)) => {
            let stmts: &[Statement] = b.statements.as_slice();
            for s in stmts {
                c.scan_stmt_vars(s);
            }
            for s in stmts {
                c.scan_node_vars(Node::Statement(s));
            }
            c.freeze_cvs();
            for s in stmts {
                c.compile_stmt(s)?;
            }
            is_generator = block_has_yield(&b.statements);
        }
        PropertyHookBody::Concrete(PropertyHookConcreteBody::Expression(e)) => {
            let expr = e.expression;
            c.scan_expr_vars(expr);
            c.scan_node_vars(Node::Expression(expr));
            c.freeze_cvs();
            {
                use mago_span::HasSpan;
                c.cur_line = cx.line_of(expr.span().start.offset);
            }
            match kind {
                // `get => expr` is `return expr;`.
                HookKind::Get if c.returns_ref => c.compile_arrow_return(expr)?,
                HookKind::Get => {
                    let src = c.compile_expr(expr)?;
                    c.emit(Instr::Ret { src });
                }
                // `set => expr` is `$this->prop = expr;`.
                HookKind::Set => {
                    let src = c.compile_expr(expr)?;
                    let name = c.intern_name(p.prop_name);
                    c.emit(Instr::LvalCv { cv: 0 });
                    c.emit(Instr::LvalProp { name });
                    c.emit(Instr::AssignLval { src, dst: src });
                }
            }
            is_generator = expr_has_yield(expr);
        }
    }
    c.emit(Instr::RetNull);

    let mut proto = c.finish()?;
    proto.num_params = params.len() as u16;
    proto.params = params;
    proto.this_cv = Some(0);
    proto.is_static = false;
    proto.is_generator = is_generator;
    proto.flags.insert(ProtoFlags::HOOK);
    proto.return_type = match kind {
        HookKind::Get => p.hint.and_then(|h| type_hint(cx, h)),
        HookKind::Set => Some(rhp_bytecode::TypeHint {
            nullable: false,
            intersection: false,
            kinds: vec![rhp_bytecode::TypeKind::Void],
            groups: Vec::new(),
            group_pos: Vec::new(),
            explicit_null: false,
        }),
    };
    let (start, end) = span_of(hook);
    proto.meta = decl_meta(
        cx,
        &hook.attribute_lists,
        Some(p.class_name),
        &crate::attrs::AttrTarget {
            hook: true,
            ..crate::attrs::AttrTarget::new(
                crate::attrs::TARGET_METHOD,
                hook.name.span.start.offset,
            )
        },
        hook.name.span.start.offset,
        start,
        end,
    )?;
    // php's line_start for a hook: its name's line (after attributes).
    proto.meta.start_line = cx.line_of(hook.name.span.start.offset);
    crate::attrs::apply_fn_flags(cx, &mut proto, true, true, start);
    Ok(proto)
}
