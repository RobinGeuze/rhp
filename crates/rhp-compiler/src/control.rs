//! Control-flow lowering: conditionals, loops, switch/match, try and jumps.

use rhp_bytecode::{CmpKind, Instr, Reg};
use rhp_frontend::cst::{
    Access, Expression, ForBody, ForeachBody, ForeachTarget, IfBody, Literal, Match, MatchArm,
    Statement, SwitchBody, SwitchCase, UnaryPrefixOperator, Variable, WhileBody,
};
use rhp_values::Thrown;

use crate::assign::{check_globals_write, globals_index};
use crate::builder::{LoopCtx, ProtoBuilder};
use crate::hints::hint_class_names;
use crate::oop::Sel;
use crate::util::strip_dollar;

impl ProtoBuilder<'_> {
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

    pub(crate) fn compile_if(&mut self, i: &rhp_frontend::cst::If) -> Result<(), Thrown> {
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

    pub(crate) fn compile_while(&mut self, w: &rhp_frontend::cst::While) -> Result<(), Thrown> {
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

    pub(crate) fn compile_do_while(
        &mut self,
        d: &rhp_frontend::cst::DoWhile,
    ) -> Result<(), Thrown> {
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

    pub(crate) fn compile_for(&mut self, f: &rhp_frontend::cst::For) -> Result<(), Thrown> {
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

    pub(crate) fn compile_switch(&mut self, sw: &rhp_frontend::cst::Switch) -> Result<(), Thrown> {
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

    pub(crate) fn compile_foreach(
        &mut self,
        fe: &rhp_frontend::cst::Foreach,
    ) -> Result<(), Thrown> {
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

    pub(crate) fn compile_unset(&mut self, u: &rhp_frontend::cst::Unset) -> Result<(), Thrown> {
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
                    let obj = self.compile_unset_object(pa.object)?;
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

    pub(crate) fn compile_try(&mut self, t: &rhp_frontend::cst::Try) -> Result<(), Thrown> {
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
            let classes = hint_class_names(self.cx, &clause.hint)?;
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

    pub(crate) fn compile_break_continue(
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

    pub(crate) fn compile_match(&mut self, m: &Match) -> Result<Reg, Thrown> {
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
}
