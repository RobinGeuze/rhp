//! Functions, methods and closures: parameters, captures and bodies.

use std::rc::Rc;

use crate::parse::cst::{Expression, Hint, MethodBody, Modifier, Node, Statement};
use rhp_bytecode::{FunctionProto, Instr, MethodProto, NIdx, PropDefaultProto, PropProto, Reg};
use rhp_values::{PhpStr, Thrown};

use crate::builder::{ProtoBuilder, ScopeKind};
use crate::cvs::collect_vars;
use crate::decl::{
    compile_attributes, decl_meta, has_modifier, modifiers_set_visibility, modifiers_visibility,
    param_default_meta, span_of,
};
use crate::expr::{block_has_yield, expr_has_yield, stmt_has_yield};
use crate::file::FileCtx;
use crate::hints::type_hint;
use crate::names::join_ns;
use crate::util::strip_dollar;

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
    let name = PhpStr::from_bytes(m.name.value.to_vec());
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
    }

    // Constructor property promotion: a param with a modifier declares a property.
    let mut promoted: Vec<PropProto> = Vec::new();
    let mut promo_stores: Vec<(NIdx, Reg, bool)> = Vec::new(); // (prop name, param cv, by ref)

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
        let attributes = compile_attributes(cx, &p.attribute_lists, Some(class_name))?;
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
                type_hint: p.hint.as_ref().map(|h| type_hint(cx, h)),
                meta: rhp_bytecode::DeclMeta {
                    attributes: attributes.clone(),
                    doc_comment: cx.doc_comment_before(p.variable.span.start.offset),
                    start_line: cx.line_of(pstart),
                    offset: pstart,
                    end_line: cx.line_of(pend),
                },
                promoted: true,
            });
            promo_stores.push((c.intern_name(var), cv, p.ampersand.is_some()));
        }
        let dmeta = param_default_meta(cx, p, Some(class_name));
        params.push(rhp_bytecode::ParamInfo {
            name: pname,
            cv,
            by_ref: p.ampersand.is_some(),
            variadic: p.ellipsis.is_some(),
            default,
            type_hint: p.hint.as_ref().map(|h| type_hint(cx, h)),
            line: {
                use mago_span::HasSpan;
                cx.line_of(p.span().start.offset)
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
            c.emit(Instr::AssignLval { src: param_cv });
        }
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
    proto.return_type = m.return_type_hint.as_ref().map(|r| type_hint(cx, &r.hint));
    proto.meta = decl_meta(
        cx,
        &m.attribute_lists,
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
pub(crate) fn compile_function(
    cx: &FileCtx,
    func: &crate::parse::cst::Function,
) -> Result<FunctionProto, Thrown> {
    crate::checks::check_function_params(cx, &func.parameter_list);
    let fname = join_ns(&cx.cur_ns(), func.name.value);
    let mut c = ProtoBuilder::new(cx, PhpStr::from_bytes(fname));
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
        let dmeta = param_default_meta(cx, p, None);
        params.push(rhp_bytecode::ParamInfo {
            name,
            cv,
            by_ref: p.ampersand.is_some(),
            variadic: p.ellipsis.is_some(),
            default,
            type_hint: p.hint.as_ref().map(|h| type_hint(cx, h)),
            line: {
                use mago_span::HasSpan;
                cx.line_of(p.span().start.offset)
            },
            attributes: compile_attributes(cx, &p.attribute_lists, None)?,
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
    proto.return_type = func
        .return_type_hint
        .as_ref()
        .map(|r| type_hint(cx, &r.hint));
    proto.meta = decl_meta(
        cx,
        &func.attribute_lists,
        None,
        func.name.span.start.offset,
        func.function.span.start.offset,
        span_of(func).1,
    )?;
    Ok(proto)
}

impl ProtoBuilder<'_> {
    /// PHP 8.5's closure name `{closure:<enclosing>:<line>}`, built from the scope
    /// that defines the closure (`self`). Top-level/init closures embed the file.
    fn closure_magic_name(&self, offset: u32) -> PhpStr {
        let line = self.cx.line_of(offset);
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
                self.cx,
                &af.attribute_lists,
                self.class_name.as_ref(),
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
            let dmeta = param_default_meta(self.cx, p, c.class_name.clone().as_ref());
            params.push(rhp_bytecode::ParamInfo {
                name,
                cv,
                by_ref: p.ampersand.is_some(),
                variadic: p.ellipsis.is_some(),
                default,
                type_hint: p.hint.as_ref().map(|h| type_hint(self.cx, h)),
                line: {
                    use mago_span::HasSpan;
                    self.cx.line_of(p.span().start.offset)
                },
                attributes: compile_attributes(self.cx, &p.attribute_lists, c.class_name.as_ref())?,
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
                    self.cx.line_of(e.span().start.offset)
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
        proto.return_type = ret_hint.map(|h| type_hint(self.cx, h));
        proto.meta = meta;
        proto.is_generator = match body {
            ClosureBody::Block(b) => block_has_yield(&b.statements),
            ClosureBody::Arrow(e) => expr_has_yield(e),
        };

        let fnidx = self.cx.add_closure(proto);

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
}
