//! Statement lowering.

use crate::parse::cst::{Expression, Statement, Variable};
use rhp_bytecode::{ClassProto, Instr, Reg};
use rhp_values::{Thrown, Value};

use crate::builder::ProtoBuilder;
use crate::decl::{compile_class, compile_enum, compile_interface, compile_trait};
use crate::fold::fold_const;
use crate::functions::compile_function;
use crate::names::join_ns;
use crate::util::{compile_error, strip_dollar};

impl ProtoBuilder<'_> {
    /// Record a nested class declaration and emit its `DeclareClass`.
    fn emit_cond_class(&mut self, proto: ClassProto) {
        self.cur_line = proto.decl_line;
        let class_idx = self.cx.add_cond_class(proto);
        self.emit(Instr::DeclareClass { class_idx });
    }

    pub(crate) fn compile_stmt(&mut self, stmt: &Statement) -> Result<(), Thrown> {
        use mago_span::HasSpan;
        if matches!(
            stmt,
            Statement::Class(_)
                | Statement::Interface(_)
                | Statement::Trait(_)
                | Statement::Enum(_)
        ) && self.cx.class_scope.borrow().is_some()
        {
            return Err(compile_error(
                "Class declarations may not be nested",
                self.cx.line_of(stmt.span().start.offset),
            ));
        }
        self.compile_stmt_inner(stmt)
            .map_err(|e| self.cx.locate(e, stmt.span().start.offset))
    }

    fn compile_stmt_inner(&mut self, stmt: &Statement) -> Result<(), Thrown> {
        use mago_span::HasSpan;
        self.cur_line = self.cx.line_of(stmt.span().start.offset);
        match stmt {
            Statement::OpeningTag(_) | Statement::ClosingTag(_) | Statement::Noop(_) => {}
            Statement::Inline(inline) => {
                if inline.kind.is_text() {
                    let text = self
                        .cx
                        .strip_close_tag_newline(inline.span().start.offset, inline.value);
                    // php's `?>` token includes the newline: the HTML after
                    // it starts on the next line.
                    let skipped = inline.value.len() - text.len();
                    if skipped > 0 {
                        self.cur_line =
                            self.cx.line_of(inline.span().start.offset + skipped as u32);
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
                self.result_unused = matches!(s.expression, Expression::Assignment(_));
                let r = self.compile_discarded(s.expression);
                self.result_unused = false;
                r?;
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
                let proto = compile_function(self.cx, f)?;
                let idx = self.cx.add_closure(proto);
                self.emit(Instr::DeclareFunc { idx });
            }
            // A class-like reached here is nested in control flow or a function
            // body: declared at run time when execution reaches it.
            Statement::Class(cl) => {
                let proto = compile_class(self.cx, cl)?;
                self.emit_cond_class(proto);
            }
            Statement::Interface(i) => {
                let proto = compile_interface(self.cx, i)?;
                self.emit_cond_class(proto);
            }
            Statement::Trait(t) => {
                let proto = compile_trait(self.cx, t)?;
                self.emit_cond_class(proto);
            }
            Statement::Enum(e) => {
                let proto = compile_enum(self.cx, e)?;
                self.emit_cond_class(proto);
            }
            Statement::Constant(c) => {
                // zend_compile_const_decl: attributes take one constant.
                let attrs = if c.attribute_lists.is_empty() {
                    None
                } else {
                    if c.items.len() > 1 {
                        self.cx.fatal(
                            c.r#const.span.start.offset,
                            "Cannot apply attributes to multiple constants at once".into(),
                        );
                    }
                    let at = c
                        .items
                        .iter()
                        .next()
                        .map_or(c.r#const.span.start.offset, |i| i.name.span.start.offset);
                    let list = crate::decl::compile_attributes(
                        self.cx,
                        &c.attribute_lists,
                        self.class_name.as_ref(),
                        &crate::attrs::AttrTarget::new(crate::attrs::TARGET_CONST, at),
                    )?;
                    self.const_attrs.push(std::rc::Rc::new(list));
                    Some((self.const_attrs.len() - 1) as u32)
                };
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
                        self.cx.fatal(stmt.span().start.offset, msg);
                    }
                    if !crate::constexpr::check_const_expr(
                        self.cx,
                        item.value,
                        true,
                        self.scope_known(),
                    ) {
                        continue;
                    }
                    self.in_const_decl = true;
                    let src = self.compile_expr(item.value);
                    self.in_const_decl = false;
                    let src = src?;
                    let name = self.intern_name(&join_ns(&self.cx.cur_ns(), item.name.value));
                    match attrs {
                        Some(attrs) => self.emit(Instr::DeclareAttrConst { name, src, attrs }),
                        None => self.emit(Instr::DeclareConst { name, src }),
                    }
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
                    if matches!(v, Variable::Direct(d) if strip_dollar(d.name) == b"this") {
                        return Err(compile_error(
                            "Cannot use $this as global variable",
                            self.cur_line,
                        ));
                    }
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
                    use crate::parse::cst::StaticItem;
                    let var = match item {
                        StaticItem::Abstract(a) => &a.variable,
                        StaticItem::Concrete(c) => &c.variable,
                    };
                    let vname = strip_dollar(var.name);
                    if vname == b"this" {
                        return Err(compile_error(
                            "Cannot use $this as static variable",
                            self.cur_line,
                        ));
                    }
                    if self.static_vars.iter().any(|n| n == vname) {
                        return Err(compile_error(
                            format!(
                                "Duplicate declaration of static variable ${}",
                                String::from_utf8_lossy(vname)
                            ),
                            self.cur_line,
                        ));
                    }
                    self.static_vars.push(vname.to_vec());
                    let (dvar, init) = match item {
                        StaticItem::Abstract(a) => (&a.variable, u32::MAX),
                        StaticItem::Concrete(c) => match fold_const(self.cx, c.value) {
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
                let line = self.cur_line;
                if self.finally_rets.is_empty() {
                    match r.value {
                        // By-ref function: `return $lval` yields a reference.
                        Some(e) if self.returns_ref => {
                            let src = self.compile_ref_source(e)?;
                            self.check_return(r.value, line)?;
                            self.emit(Instr::RetRef { src });
                        }
                        Some(e) => {
                            let src = self.compile_expr(e)?;
                            self.check_return(r.value, line)?;
                            self.emit(Instr::Ret { src });
                        }
                        None => {
                            self.check_return(None, line)?;
                            self.emit(Instr::RetNull)
                        }
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
                    self.check_return(r.value, line)?;
                    self.emit(Instr::SetPendingRet { src });
                    let pos = self.code.len();
                    self.emit(Instr::Nop); // Jmp -> finally (patched)
                    self.finally_rets.last_mut().unwrap().push(pos);
                }
            }
            Statement::Break(b) => {
                self.compile_break_continue(b.level, true, b.span().start.offset)?
            }
            Statement::Continue(c) => {
                self.compile_break_continue(c.level, false, c.span().start.offset)?
            }
            // `declare(strict_types=1|ticks=…|encoding=…)` — the directives are
            // no-ops for this engine; compile the body (usually just `;`).
            Statement::Declare(d) => {
                use crate::parse::cst::DeclareBody;
                self.check_declare(d);
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
}

impl ProtoBuilder<'_> {
    /// zend_emit_return_type_check's compile-time errors for `return`.
    fn check_return(&self, value: Option<&Expression>, line: u32) -> Result<(), Thrown> {
        use crate::checks::{type_mask, NEVER, NULL, VOID};
        let Some(t) = &self.ret_type else {
            return Ok(());
        };
        if self.is_generator {
            return Ok(());
        }
        let what = if self.cx.class_scope.borrow().is_some() {
            "method"
        } else {
            "function"
        };
        let (mask, _) = type_mask(t);
        if mask & VOID != 0 {
            return match value {
                None => Ok(()),
                Some(e) if matches!(fold_const(self.cx, e), Ok(Value::Null)) => Err(compile_error(
                    format!("A void {what} must not return a value (did you mean \"return;\" instead of \"return null;\"?)"),
                    line,
                )),
                Some(_) => Err(compile_error(
                    format!("A void {what} must not return a value"),
                    line,
                )),
            };
        }
        if mask & NEVER != 0 {
            return Err(compile_error(
                format!("A never-returning {what} must not return"),
                line,
            ));
        }
        if value.is_none() {
            let hint = if mask & NULL != 0 {
                " (did you mean \"return null;\" instead of \"return;\"?)"
            } else {
                ""
            };
            return Err(compile_error(
                format!("A {what} with return type must return a value{hint}"),
                line,
            ));
        }
        Ok(())
    }

    /// zend_compile_declare's checks: literal values, where encoding and
    /// strict_types may appear, strict_types' value and form, unknown
    /// directives.
    fn check_declare(&mut self, d: &crate::parse::cst::Declare) {
        use crate::parse::cst::DeclareBody;
        use mago_span::HasSpan;
        let cx = self.cx;
        let at = d.declare.span.start.offset;
        let first = cx
            .first_declares
            .iter()
            .find(|(o, _)| *o == at)
            .map(|&(_, nop)| nop);
        for item in d.items.iter() {
            let name = item.name.value;
            let shown = String::from_utf8_lossy(name);
            let off = item.span().start.offset;
            if !declare_literal(item.value) {
                cx.fatal(off, format!("declare({shown}) value must be a literal"));
                return;
            }
            if name.eq_ignore_ascii_case(b"ticks") {
            } else if name.eq_ignore_ascii_case(b"encoding") {
                if first != Some(false) {
                    cx.fatal(
                        off,
                        "Encoding declaration pragma must be the very first statement in the script".into(),
                    );
                    return;
                }
            } else if name.eq_ignore_ascii_case(b"strict_types") {
                if first.is_none() {
                    cx.fatal(
                        off,
                        "strict_types declaration must be the very first statement in the script"
                            .into(),
                    );
                    return;
                }
                let block = !matches!(&d.body, DeclareBody::Statement(Statement::Noop(_)));
                if block {
                    cx.fatal(
                        off,
                        "strict_types declaration must not use block mode".into(),
                    );
                    return;
                }
                if !matches!(fold_const(cx, item.value), Ok(Value::Int(0 | 1))) {
                    cx.fatal(
                        off,
                        "strict_types declaration must have 0 or 1 as its value".into(),
                    );
                    return;
                }
            } else {
                cx.diag(
                    rhp_values::E_COMPILE_WARNING,
                    off,
                    format!("Unsupported declare '{shown}'"),
                );
            }
        }
    }
}

/// Whether php's parser leaves a declare value as a ZEND_AST_ZVAL: a
/// number, plain string or heredoc, parenthesised or concatenated
/// (zend_ast_create_concat_op folds those). `true`, `-1` and magic
/// constants are not.
fn declare_literal(e: &Expression) -> bool {
    use crate::parse::cst::{BinaryOperator, CompositeString, Literal, StringPart};
    match e {
        Expression::Literal(Literal::String(_) | Literal::Integer(_) | Literal::Float(_)) => true,
        Expression::CompositeString(CompositeString::Document(d)) => {
            d.parts.iter().all(|p| matches!(p, StringPart::Literal(_)))
        }
        Expression::Parenthesized(p) => declare_literal(p.expression),
        Expression::Binary(b) if matches!(b.operator, BinaryOperator::StringConcat(_)) => {
            declare_literal(b.lhs) && declare_literal(b.rhs)
        }
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

impl ProtoBuilder<'_> {
    /// Compile an expression whose value is freed (an expression statement,
    /// a `for` list entry): php's zend_do_free marks a call compiled last
    /// as having an unused result, which `#[\NoDiscard]` warns about. A
    /// `(void)` cast frees its operand without that mark.
    pub(crate) fn compile_discarded(&mut self, e: &Expression) -> Result<Reg, Thrown> {
        use crate::parse::cst::UnaryPrefixOperator;
        if let Expression::UnaryPrefix(u) = e {
            if matches!(u.operator, UnaryPrefixOperator::VoidCast(..)) {
                return self.compile_expr(u.operand);
            }
        }
        let r = self.compile_expr(e)?;
        // A nullsafe chain ends `call; Jmp end; LoadNull` (php's call is
        // still the last opline there).
        let mut at = self.code.len().wrapping_sub(1);
        if let [.., Instr::Jmp { target }, Instr::LoadNull { dst }] = self.code[..] {
            if dst == r && target as usize == self.code.len() {
                at = self.code.len() - 3;
            }
        }
        let dst = match self.code.get(at) {
            Some(
                Instr::Call { dst, .. }
                | Instr::CallV { dst, .. }
                | Instr::CallDyn { dst, .. }
                | Instr::CallDynV { dst, .. }
                | Instr::CallMethod { dst, .. }
                | Instr::CallMethodV { dst, .. }
                | Instr::CallMethodDyn { dst, .. }
                | Instr::CallMethodDynV { dst, .. }
                | Instr::CallStatic { dst, .. }
                | Instr::CallStaticV { dst, .. }
                | Instr::CallStaticDyn { dst, .. }
                | Instr::CallStaticDynV { dst, .. },
            ) => Some(*dst),
            _ => None,
        };
        if dst == Some(r) {
            self.discard_calls.push(at as rhp_bytecode::Pc);
        }
        Ok(r)
    }
}
