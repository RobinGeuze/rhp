//! Statement lowering.

use crate::parse::cst::{Expression, Statement, Variable};
use rhp_bytecode::{ClassProto, Instr};
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
                self.compile_expr(s.expression)?;
                self.result_unused = false;
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
                    let src = self.compile_expr(item.value)?;
                    let name = self.intern_name(&join_ns(&self.cx.cur_ns(), item.name.value));
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
