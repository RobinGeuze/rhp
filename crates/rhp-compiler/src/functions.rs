//! Functions, methods and closures: parameters, captures and bodies.

use std::rc::Rc;

use crate::parse::cst::{Expression, Hint, MethodBody, Modifier, Node, Statement};
use rhp_bytecode::{FunctionProto, Instr, MethodProto, NIdx, PropDefaultProto, PropProto, Reg};
use rhp_values::{PhpStr, Thrown, Value};

use crate::attrs::AttrTarget;
use crate::builder::{ProtoBuilder, ScopeKind};
use crate::cvs::collect_vars;
use crate::decl::{
    compile_attributes, decl_meta, has_modifier, modifiers_visibility, param_default_meta,
    readonly_set_visibility, span_of,
};
use crate::expr::{block_has_yield, expr_has_yield, stmt_has_yield};
use crate::file::FileCtx;
use crate::hints::{compile_type, compile_type_ex, PhpType};
use crate::names::join_ns;
use crate::util::{compile_error, strip_dollar};

/// The body form of a closure being lowered.
enum ClosureBody<'a> {
    /// Arrow function: a single expression, implicitly `return`ed.
    Arrow(&'a Expression<'a>),
    /// `function(){ ... }`: a block of statements.
    Block(&'a crate::parse::cst::Block<'a>),
}

pub(crate) fn compile_method(
    cx: &FileCtx,
    m: &crate::parse::cst::Method,
    class_name: &PhpStr,
) -> Result<(MethodProto, Vec<PropProto>), Thrown> {
    let name = PhpStr::from_bytes(m.name.value);
    let visibility = modifiers_visibility(&m.modifiers);
    let is_static = has_modifier(&m.modifiers, |md| matches!(md, Modifier::Static(_)));
    let is_abstract = has_modifier(&m.modifiers, |md| matches!(md, Modifier::Abstract(_)));
    let is_final = has_modifier(&m.modifiers, |md| matches!(md, Modifier::Final(_)));

    let mut c = ProtoBuilder::new(cx, name.clone());
    c.class_name = Some(class_name.clone());
    c.scope_kind = ScopeKind::Method;
    c.returns_ref = m.ampersand.is_some();

    // Instance methods reserve CV 0 for `$this`.
    if !is_static {
        let this = c.cv(b"this");
        debug_assert_eq!(this, 0);
        c.this_bound = true;
    }

    // Constructor property promotion: a param with a modifier declares a property.
    let mut promoted: Vec<PropProto> = Vec::new();
    let mut promo_stores: Vec<(NIdx, Reg, bool)> = Vec::new(); // (prop name, param cv, by ref)

    let display = format!(
        "{}::{}",
        String::from_utf8_lossy(class_name.as_bytes()),
        String::from_utf8_lossy(m.name.value)
    );
    // php compiles the return type before the parameters.
    let mut return_type = m
        .return_type_hint
        .as_ref()
        .and_then(|r| compile_type(cx, &r.hint, false).map(|t| t.hint));
    if m.return_type_hint.is_none() && m.name.value.eq_ignore_ascii_case(b"__tostring") {
        // zend_compile_params' fallback return type: `__toString(): string`.
        return_type = Some(rhp_bytecode::TypeHint {
            nullable: false,
            intersection: false,
            kinds: vec![rhp_bytecode::TypeKind::String],
            groups: Vec::new(),
            group_pos: Vec::new(),
            explicit_null: false,
        });
    }
    c.ret_type = return_type.clone();
    c.is_generator = match &m.body {
        MethodBody::Concrete(block) => block.statements.iter().any(stmt_has_yield),
        MethodBody::Abstract(_) => false,
    };
    c.check_void_by_ref(&display, m.return_type_hint.as_ref().map(|r| &r.hint));
    let _attr_scope = cx.attr_scope(format!("{display}()").into_bytes());
    let (params, nonconst_defaults, types) =
        c.lower_params(&m.parameter_list, &display, false, Some(class_name))?;
    c.check_generator_type(cx.line_of(m.function.span.start.offset))?;
    for ((p, info), ty) in m.parameter_list.parameters.iter().zip(&params).zip(&types) {
        if p.modifiers.is_empty() && p.hooks.is_none() {
            continue;
        }
        let var = strip_dollar(p.variable.name);
        if let Some(ty) = ty.as_ref().filter(|t| t.has(crate::hints::MAY_CALLABLE)) {
            use mago_span::HasSpan;
            cx.fatal(
                p.span().start.offset,
                format!(
                    "Property {}::${} cannot have type {}",
                    String::from_utf8_lossy(class_name.as_bytes()),
                    String::from_utf8_lossy(var),
                    ty.to_php_string()
                ),
            );
        }
        let vis = modifiers_visibility(&p.modifiers);
        let ro = has_modifier(&p.modifiers, |md| matches!(md, Modifier::Readonly(_)));
        let is_final = has_modifier(&p.modifiers, |md| matches!(md, Modifier::Final(_)));
        let (pstart, pend) = span_of(p);
        let (hooks, is_virtual) = match &p.hooks {
            Some(list) => {
                let hcx = crate::hooks::HookCtx {
                    class_name,
                    prop_name: var,
                    hint: p.hint.as_ref(),
                    visibility: vis,
                    is_abstract: false,
                    is_static: false,
                    is_readonly: ro || cx.in_readonly_class.get(),
                    in_interface: false,
                    offset: pstart,
                };
                *cx.cur_property.borrow_mut() = Some(var.to_vec());
                let hooks = crate::hooks::compile_hooks(cx, &hcx, list);
                *cx.cur_property.borrow_mut() = None;
                (hooks?, crate::hooks::property_is_virtual(var, list, false))
            }
            None => ([None, None], false),
        };
        // php compiles the list again for the property: internal attributes
        // that only fit a property (#[\Override]) are kept here.
        *cx.cur_property.borrow_mut() = Some(var.to_vec());
        let attributes = compile_attributes(
            cx,
            &p.attribute_lists,
            Some(class_name),
            &AttrTarget {
                promoted: crate::attrs::TARGET_PARAMETER,
                ..AttrTarget::new(crate::attrs::TARGET_PROPERTY, p.variable.span.start.offset)
            },
        );
        *cx.cur_property.borrow_mut() = None;
        let attributes = attributes?;
        promoted.push(PropProto {
            name: PhpStr::from_bytes(var),
            visibility: vis,
            set_visibility: readonly_set_visibility(&p.modifiers, ro || cx.in_readonly_class.get()),
            is_readonly: ro,
            default: PropDefaultProto::None,
            type_hint: info.type_hint.clone(),
            meta: rhp_bytecode::DeclMeta {
                attributes,
                doc_comment: cx.doc_comment_before(p.variable.span.start.offset),
                start_line: cx.line_of(pstart),
                offset: pstart,
                end_line: cx.line_of(pend),
            },
            promoted: true,
            hooks,
            is_virtual,
            is_abstract: false,
            is_final,
        });
        promo_stores.push((c.intern_name(var), info.cv, p.ampersand.is_some()));
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
    // (`$this->name = &$name` for a by-reference parameter.)
    for &(prop_name, param_cv, by_ref) in &promo_stores {
        if by_ref {
            let r = c.alloc_temp();
            c.emit(Instr::MakeRef {
                dst: r,
                cv: param_cv,
            });
            c.emit(Instr::LvalCv { cv: 0 });
            c.emit(Instr::LvalProp { name: prop_name });
            c.emit(Instr::AssignLvalRef { src: r });
        } else {
            c.emit(Instr::LvalCv { cv: 0 });
            c.emit(Instr::LvalProp { name: prop_name });
            c.emit(Instr::AssignLval {
                src: param_cv,
                dst: param_cv,
            });
        }
    }

    for s in body_stmts {
        c.compile_stmt(s)?;
    }
    let end = match &m.body {
        MethodBody::Concrete(block) => block.right_brace.start.offset,
        MethodBody::Abstract(_) => mago_span::HasSpan::span(m).end.offset,
    };
    c.emit_final_return(end);

    let mut proto = c.finish()?;
    proto.num_params = num_params;
    proto.params = params;
    proto.this_cv = if is_static { None } else { Some(0) };
    proto.is_static = is_static;
    proto.is_generator = body_stmts.iter().any(stmt_has_yield);
    proto.return_type = return_type;
    proto.meta = decl_meta(
        cx,
        &m.attribute_lists,
        Some(class_name),
        &AttrTarget::new(crate::attrs::TARGET_METHOD, m.function.span.start.offset),
        m.name.span.start.offset,
        m.function.span.start.offset,
        span_of(m).1,
    )?;
    crate::attrs::apply_fn_flags(cx, &mut proto, true, false, m.function.span.start.offset);

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
pub(crate) fn compile_function(
    cx: &FileCtx,
    func: &crate::parse::cst::Function,
) -> Result<FunctionProto, Thrown> {
    crate::checks::check_function_params(cx, &func.parameter_list);
    let fname = join_ns(&cx.cur_ns(), func.name.value);
    let mut c = ProtoBuilder::new(cx, PhpStr::from_bytes(fname));
    c.returns_ref = func.ampersand.is_some();

    let return_type = func
        .return_type_hint
        .as_ref()
        .and_then(|r| compile_type(cx, &r.hint, false).map(|t| t.hint));
    c.ret_type = return_type.clone();
    c.is_generator = block_has_yield(&func.body.statements);
    let display = String::from_utf8_lossy(c.name.as_bytes()).into_owned();
    c.check_void_by_ref(&display, func.return_type_hint.as_ref().map(|r| &r.hint));
    let _attr_scope = cx.attr_scope(format!("{display}()").into_bytes());
    let (params, nonconst_defaults, _) =
        c.lower_params(&func.parameter_list, &display, false, None)?;
    c.check_generator_type(cx.line_of(func.function.span.start.offset))?;
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
    c.emit_final_return(func.body.right_brace.start.offset);

    let mut proto = c.finish()?;
    proto.num_params = num_params;
    proto.params = params;
    proto.is_generator = block_has_yield(&func.body.statements);
    proto.return_type = return_type;
    proto.meta = decl_meta(
        cx,
        &func.attribute_lists,
        None,
        &AttrTarget::new(
            crate::attrs::TARGET_FUNCTION,
            func.function.span.start.offset,
        ),
        func.name.span.start.offset,
        func.function.span.start.offset,
        span_of(func).1,
    )?;
    crate::attrs::apply_fn_flags(
        cx,
        &mut proto,
        false,
        false,
        func.function.span.start.offset,
    );
    Ok(proto)
}

impl ProtoBuilder<'_> {
    /// PHP 8.5's closure name `{closure:<enclosing>:<line>}`, built from the scope
    /// that defines the closure (`self`). Top-level/init closures embed the file.
    fn closure_magic_name(&self, offset: u32) -> PhpStr {
        let line = self.cx.line_of(offset);
        let qual: Vec<u8> = match self.scope_kind {
            _ if self.closure_qual.is_some() => self.closure_qual.clone().unwrap_or_default(),
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
            ScopeKind::Main | ScopeKind::Init => self.cx.file_name(),
        };
        let mut s = b"{closure:".to_vec();
        s.extend_from_slice(&qual);
        s.push(b':');
        s.extend_from_slice(line.to_string().as_bytes());
        s.push(b'}');
        PhpStr::from_bytes(s)
    }

    /// `fn(params) => expr` — arrow function. Auto-captures (by value) every
    /// free variable of the body that is a CV in the enclosing scope, plus
    /// `$this` when defined and the arrow is non-static.
    pub(crate) fn compile_arrow(
        &mut self,
        af: &crate::parse::cst::ArrowFunction,
    ) -> Result<Reg, Thrown> {
        let is_static = af.r#static.is_some();
        // Parameter names, to exclude from auto-capture.
        let params: Vec<Vec<u8>> = af
            .parameter_list
            .parameters
            .iter()
            .map(|p| strip_dollar(p.variable.name).to_vec())
            .collect();

        let mut used = Vec::new();
        collect_vars(af.expression, self.cx.src, &mut used);
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
                self.cx,
                &af.attribute_lists,
                self.class_name.as_ref(),
                &AttrTarget::new(crate::attrs::TARGET_FUNCTION, af.r#fn.span.start.offset),
                af.r#fn.span.end.offset,
                af.r#fn.span.start.offset,
                span_of(af).1,
            )?,
        )
    }

    /// `function(params) use(...) { body }` — closure with an explicit capture
    /// list. `use(&$x)` captures by reference (shares a cell with the outer var).
    pub(crate) fn compile_closure(
        &mut self,
        cl: &crate::parse::cst::Closure,
    ) -> Result<Reg, Thrown> {
        let is_static = cl.r#static.is_some();
        let mut captures: Vec<(Vec<u8>, bool)> = Vec::new();
        if !is_static && self.cv_map.iter().any(|n| n == b"this") {
            captures.push((b"this".to_vec(), false));
        }
        if let Some(uses) = &cl.use_clause {
            // zend_compile_closure_binding / zend_compile_closure_uses.
            let mut seen: Vec<&[u8]> = Vec::new();
            for u in uses.variables.iter() {
                use mago_span::HasSpan;
                let name = strip_dollar(u.variable.name);
                let line = self.cx.line_of(u.span().start.offset);
                let msg = if name == b"this" {
                    "Cannot use $this as lexical variable".to_string()
                } else if crate::util::is_superglobal(name) {
                    "Cannot use auto-global as lexical variable".to_string()
                } else if seen.contains(&name) {
                    format!(
                        "Cannot use variable ${} twice",
                        String::from_utf8_lossy(name)
                    )
                } else {
                    seen.push(name);
                    continue;
                };
                return Err(compile_error(msg, line));
            }
            for u in uses.variables.iter() {
                use mago_span::HasSpan;
                let name = strip_dollar(u.variable.name);
                if cl
                    .parameter_list
                    .parameters
                    .iter()
                    .any(|p| strip_dollar(p.variable.name) == name)
                {
                    return Err(compile_error(
                        format!(
                            "Cannot use lexical variable ${} as a parameter name",
                            String::from_utf8_lossy(name)
                        ),
                        self.cx.line_of(u.span().start.offset),
                    ));
                }
            }
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
                self.cx,
                &cl.attribute_lists,
                self.class_name.as_ref(),
                &AttrTarget::new(crate::attrs::TARGET_FUNCTION, cl.function.span.start.offset),
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
        param_list: &crate::parse::cst::FunctionLikeParameterList,
        captures: Vec<(Vec<u8>, bool)>,
        body: ClosureBody,
        closure_offset: u32,
        ret_hint: Option<&Hint>,
        returns_ref: bool,
        meta: rhp_bytecode::DeclMeta,
    ) -> Result<Reg, Thrown> {
        crate::checks::check_function_params(self.cx, param_list);
        let nc = captures.len() as u16;

        // PHP 8.5 names a closure `{closure:<enclosing scope>:<line>}`, computed
        // from the scope that *defines* it (the current builder).
        let magic_name = self.closure_magic_name(closure_offset);

        // --- compile the closure proto in an isolated builder ---
        // The proto carries the 8.5 name too: traces and error messages print it.
        let mut c = ProtoBuilder::new(self.cx, magic_name.clone());
        c.scope_kind = ScopeKind::Closure;
        c.class_name = self.class_name.clone(); // __CLASS__ in a closure = enclosing class
        c.magic_fn_name = Some(magic_name);
        c.returns_ref = returns_ref;
        for (name, _) in &captures {
            c.cv(name); // captures occupy CVs 0..nc
            if name != b"this" {
                c.static_vars.push(name.clone());
            }
        }
        let return_type = ret_hint.and_then(|h| compile_type(self.cx, h, true).map(|t| t.hint));
        c.ret_type = return_type.clone();
        c.is_generator = match body {
            ClosureBody::Block(b) => block_has_yield(&b.statements),
            ClosureBody::Arrow(e) => expr_has_yield(e),
        };
        // A closure has no scope while it is compiled: its name alone.
        let display = String::from_utf8_lossy(c.name.as_bytes()).into_owned();
        c.check_void_by_ref(&display, ret_hint);
        let cname = c.class_name.clone();
        let (params, nonconst_defaults, _) =
            c.lower_params(param_list, &display, true, cname.as_ref())?;
        c.check_generator_type(meta.start_line)?;
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
                    self.cx.line_of(e.span().start.offset)
                };
                c.compile_arrow_return(e)?;
            }
            ClosureBody::Block(b) => {
                for s in b.statements.iter() {
                    c.compile_stmt(s)?;
                }
                c.emit_final_return(b.right_brace.start.offset);
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
        proto.return_type = return_type;
        proto.meta = meta;
        let at = proto.meta.offset;
        crate::attrs::apply_fn_flags(self.cx, &mut proto, self.class_name.is_some(), false, at);
        proto.is_generator = match body {
            ClosureBody::Block(b) => block_has_yield(&b.statements),
            ClosureBody::Arrow(e) => expr_has_yield(e),
        };

        let fnidx = self.cx.add_closure(proto);

        // --- emit the capture window + MakeClosure in the enclosing frame ---
        // `use ($x)` reads the variable (php's BIND_LEXICAL warns when it is
        // undefined); an arrow function's implicit binding leaves it
        // undefined inside, as does `$this`'s.
        let implicit = matches!(body, ClosureBody::Arrow(_));
        let base = self.alloc_temps(nc.max(1));
        for (i, (name, by_ref)) in captures.iter().enumerate() {
            let dst = base + i as Reg;
            if *by_ref {
                // Share a cell with the outer variable (created if absent).
                let cv = self.cv(name);
                self.emit(Instr::MakeRef { dst, cv });
            } else {
                match self.cv_map.iter().position(|n| n == name) {
                    Some(src) if implicit || name == b"this" => self.emit(Instr::MovIs {
                        dst,
                        src: src as Reg,
                    }),
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
}

/// One parameter list's lowered parameters, their non-constant defaults
/// (index, cv, expression) and php's view of each declared type.
type LoweredParams<'a> = (
    Vec<rhp_bytecode::ParamInfo>,
    Vec<(u16, Reg, &'a Expression<'a>, Option<u32>)>,
    Vec<Option<PhpType>>,
);

impl ProtoBuilder<'_> {
    /// zend_compile_params: the parameters' CVs, types and defaults, with
    /// php's compile-time checks. `display` names the function in messages;
    /// `closure`: the scope is not known at compile time.
    fn lower_params<'a>(
        &mut self,
        list: &'a crate::parse::cst::FunctionLikeParameterList<'a>,
        display: &str,
        closure: bool,
        class_name: Option<&PhpStr>,
    ) -> Result<LoweredParams<'a>, Thrown> {
        use mago_span::HasSpan;
        let cx = self.cx;
        let mut params = Vec::new();
        let mut nonconst = Vec::new();
        let mut types = Vec::new();
        // The last parameter without a default (not variadic).
        let last_required = list
            .parameters
            .iter()
            .rposition(|p| p.default_value.is_none() && p.ellipsis.is_none());
        for (idx, p) in list.parameters.iter().enumerate() {
            let var = strip_dollar(p.variable.name);
            let pline = cx.line_of(p.span().start.offset);
            if var == b"this" {
                return Err(compile_error("Cannot use $this as parameter", pline));
            }
            if idx > 0
                && list
                    .parameters
                    .iter()
                    .nth(idx - 1)
                    .is_some_and(|q| q.ellipsis.is_some())
            {
                return Err(compile_error(
                    "Only the last parameter can be variadic",
                    pline,
                ));
            }
            if p.ellipsis.is_some() && p.default_value.is_some() {
                return Err(compile_error(
                    "Variadic parameter cannot have a default value",
                    pline,
                ));
            }
            let cv = self.cv(var); // params get CVs in order
            let name = self.intern_name(var);
            let at = p.span().start.offset;
            let line = cx.line_of(at);
            let promoted = !p.modifiers.is_empty() || p.hooks.is_some();
            let mut folded = p
                .default_value
                .as_ref()
                .map(|dv| crate::fold::fold_const(cx, dv.value).map_err(|_| dv.value));
            let mut type_hint = None;
            let mut php_type = None;
            let mut forced_nullable = false;
            if let Some(h) = &p.hint {
                let force_null = matches!(folded, Some(Ok(Value::Null))) && !promoted;
                if let Some((t, forced)) = compile_type_ex(cx, h, closure, force_null) {
                    let pname = String::from_utf8_lossy(var);
                    forced_nullable = forced;
                    if forced {
                        cx.compile_diags.borrow_mut().push((
                            rhp_values::E_DEPRECATED,
                            format!(
                                "{display}(): Implicitly marking parameter ${pname} as nullable is deprecated, the explicit nullable type must be used instead"
                            ),
                            line,
                        ));
                    }
                    for (bit, word) in [
                        (crate::hints::MAY_VOID, "void"),
                        (crate::hints::MAY_NEVER, "never"),
                    ] {
                        if t.ty.has(bit) {
                            cx.fatal(at, format!("{word} cannot be used as a parameter type"));
                        }
                    }
                    if let Some(Ok(v)) = &mut folded {
                        if !force_null && !t.ty.accepts_default(v) {
                            cx.fatal(
                                at,
                                format!(
                                    "Cannot use {} as default value for parameter ${pname} of type {}",
                                    const_type_name(v),
                                    t.ty.to_php_string()
                                ),
                            );
                        }
                    }
                    type_hint = Some(t.hint);
                    php_type = Some(t.ty);
                }
            }
            // An optional parameter before a required one is required: it
            // loses its default (php compiles it to RECV).
            let mut dmeta = param_default_meta(cx, p, class_name);
            if folded.is_some() && last_required.is_some_and(|r| idx < r) {
                if !forced_nullable {
                    let r = &list.parameters.iter().nth(last_required.unwrap_or(0));
                    let rname = r.map(|r| strip_dollar(r.variable.name)).unwrap_or_default();
                    cx.compile_diags.borrow_mut().push((
                        rhp_values::E_DEPRECATED,
                        format!(
                            "{display}(): Optional parameter ${} declared before required parameter ${} is implicitly treated as a required parameter",
                            String::from_utf8_lossy(var),
                            String::from_utf8_lossy(rname)
                        ),
                        line,
                    ));
                }
                folded = None;
                dmeta = (None, None);
            }
            let default = folded.map(|d| match d {
                Ok(v) => rhp_bytecode::ParamDefault::Const(self.intern_const(v)),
                Err(e) => {
                    crate::constexpr::check_const_expr(cx, e, true, self.scope_known());
                    nonconst.push((idx as u16, cv, e, type_hint.is_some().then_some(line)));
                    rhp_bytecode::ParamDefault::Init
                }
            });
            params.push(rhp_bytecode::ParamInfo {
                name,
                cv,
                by_ref: p.ampersand.is_some(),
                variadic: p.ellipsis.is_some(),
                default,
                type_hint,
                line,
                attributes: {
                    // `__PROPERTY__` in a promoted parameter's attributes.
                    if promoted {
                        *cx.cur_property.borrow_mut() = Some(var.to_vec());
                    }
                    let a = compile_attributes(
                        cx,
                        &p.attribute_lists,
                        class_name,
                        &AttrTarget {
                            promoted: if promoted {
                                crate::attrs::TARGET_PROPERTY
                            } else {
                                0
                            },
                            ..AttrTarget::new(
                                crate::attrs::TARGET_PARAMETER,
                                p.variable.span.start.offset,
                            )
                        },
                    );
                    *cx.cur_property.borrow_mut() = None;
                    a?
                },
                promoted,
                default_thunk: dmeta.0,
                default_const: dmeta.1,
            });
            types.push(php_type);
        }
        Ok((params, nonconst, types))
    }
}

/// zend_get_type_by_const for a constant default's type.
pub(crate) fn const_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        _ => "object",
    }
}
