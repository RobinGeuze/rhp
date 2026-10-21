//! AST → bytecode compiler (Phase-1 subset).
//!
//! Lowers a `phrust_frontend::ParsedFile` into a `phrust_bytecode::Program`.
//! Owns the **structural refusals** (eval/include/backticks/`(unset)`-cast,
//! variable-variable writes): those nodes never produce an opcode — the
//! compiler returns `Err(Thrown::Unsupported)` at compile time (§4 of the spec).
//!
//! Register model (spec §2.2): a pre-pass assigns one CV register per named
//! `$var` so CVs occupy `[0, num_cvs)`; temporaries bump above and are freed at
//! statement boundaries; `num_registers` is the peak.

use std::rc::Rc;

use phrust_bytecode::{CIdx, CmpKind, FnIdx, FunctionProto, Instr, NIdx, Program, Reg};
use phrust_frontend::cst::{
    Argument, ArrayElement, AssignmentOperator, BinaryOperator, CompositeString, Construct,
    Expression, ForBody, ForeachBody, ForeachTarget, Identifier, IfBody, Literal, Match, MatchArm,
    Program as AstProgram, Statement, StringPart, SwitchBody, SwitchCase, UnaryPrefixOperator,
    Variable, WhileBody,
};

/// A step in an lvalue path (compiler-side; key regs are pre-evaluated).
enum CursorStep {
    Key(Reg),
    Append,
}
use phrust_values::{PhpArray, PhpStr, Thrown, Value};

/// Compile a parsed program (borrowed from a `phrust_frontend::Arena`) into an
/// owned, runnable bytecode `Program`. No `'arena` borrow escapes.
pub fn compile(program: &AstProgram) -> Result<Program, Thrown> {
    // Hoist top-level function declarations (compiled into their own protos).
    let mut functions = Vec::new();
    for stmt in program.statements.iter() {
        if let Statement::Function(func) = stmt {
            functions.push(Rc::new(compile_function(func)?));
        }
    }

    // Compile the script body ({main}); its CVs are the globals.
    let mut c = ProtoBuilder::new(PhpStr::from("{main}"));
    for stmt in program.statements.iter() {
        c.scan_stmt_vars(stmt);
    }
    c.num_cvs = c.cv_map.len() as u16;
    c.next_temp = c.num_cvs;
    c.high_water = c.num_cvs;
    c.temp_floor = c.num_cvs;

    for stmt in program.statements.iter() {
        c.compile_stmt(stmt)?;
    }
    c.emit(Instr::RetNull);

    Ok(Program { main: Rc::new(c.finish()), functions })
}

/// Compile one `function name(params) { body }` into a `FunctionProto`.
/// Parameters occupy the first CV registers, in order.
fn compile_function(func: &phrust_frontend::cst::Function) -> Result<FunctionProto, Thrown> {
    let fname = func.name.value;
    let mut c = ProtoBuilder::new(PhpStr::from_bytes(fname.to_vec()));

    let mut params = Vec::new();
    for p in func.parameter_list.parameters.iter() {
        let var = strip_dollar(p.variable.name);
        let cv = c.cv(var); // params get CV 0,1,2,... in order
        let name = c.intern_name(var);
        let default = match &p.default_value {
            Some(dv) => Some(c.const_default(dv.value)?),
            None => None,
        };
        params.push(phrust_bytecode::ParamInfo {
            name,
            cv,
            by_ref: p.ampersand.is_some(),
            variadic: p.ellipsis.is_some(),
            default,
        });
    }
    let num_params = params.len() as u16;

    // Reject by-ref params for now (semantics not wired through calls yet).
    if params.iter().any(|p| p.by_ref) {
        return Err(Thrown::unsupported(
            "by-reference parameter",
            "reference parameters arrive in the next phase",
        ));
    }

    // Scan the body for the rest of the function's local variables.
    for s in func.body.statements.iter() {
        c.scan_stmt_vars(s);
    }
    c.num_cvs = c.cv_map.len() as u16;
    c.next_temp = c.num_cvs;
    c.high_water = c.num_cvs;
    c.temp_floor = c.num_cvs;

    for s in func.body.statements.iter() {
        c.compile_stmt(s)?;
    }
    c.emit(Instr::RetNull);

    let mut proto = c.finish();
    proto.num_params = num_params;
    proto.params = params;
    Ok(proto)
}

struct ProtoBuilder {
    name: PhpStr,
    code: Vec<Instr>,
    consts: Vec<Value>,
    names: Vec<PhpStr>,
    cv_map: Vec<Vec<u8>>, // index = Reg, value = variable name bytes
    num_cvs: u16,
    next_temp: Reg,
    high_water: Reg,
    /// Temporaries below this are protected from `reset_temps` (e.g. a `switch`
    /// subject held live across fall-through case bodies).
    temp_floor: Reg,
    /// Stack of enclosing loops/switches, for `break`/`continue` resolution.
    loops: Vec<LoopCtx>,
}

/// A break/continue target under construction. Jump placeholders are recorded
/// as code positions and patched once the loop's exit/continue labels are known.
#[derive(Default)]
struct LoopCtx {
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
}

impl ProtoBuilder {
    fn new(name: PhpStr) -> Self {
        ProtoBuilder {
            name,
            code: Vec::new(),
            consts: Vec::new(),
            names: Vec::new(),
            cv_map: Vec::new(),
            num_cvs: 0,
            next_temp: 0,
            high_water: 0,
            temp_floor: 0,
            loops: Vec::new(),
        }
    }

    fn finish(self) -> FunctionProto {
        let mut proto = FunctionProto::new(self.name);
        proto.num_cvs = self.num_cvs;
        proto.num_registers = self.high_water.max(self.num_cvs);
        proto.code = self.code;
        proto.consts = self.consts;
        proto.names = self.names;
        proto.debug.cv_names = (0..self.cv_map.len() as u32).collect();
        proto
    }

    // -- register / pool management --

    fn emit(&mut self, instr: Instr) {
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
        self.code[pos] = Instr::Jmp { target };
    }

    fn here(&self) -> u32 {
        self.code.len() as u32
    }

    fn cv(&mut self, var_name: &[u8]) -> Reg {
        if let Some(i) = self.cv_map.iter().position(|n| n == var_name) {
            return i as Reg;
        }
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

    /// Evaluate a parameter default to a constant and intern it. Only
    /// compile-time constants are supported (literals, `[]`, true/false/null,
    /// negated numeric literals); other expressions are deferred.
    fn const_default(&mut self, e: &Expression) -> Result<phrust_bytecode::Default, Thrown> {
        let v = self.const_value(e)?;
        Ok(phrust_bytecode::Default::Const(self.intern_const(v)))
    }

    fn const_value(&mut self, e: &Expression) -> Result<Value, Thrown> {
        let v = match e {
            Expression::Literal(l) => literal_value(l),
            Expression::Parenthesized(p) => return self.const_value(p.expression),
            Expression::Array(a) if a.elements.is_empty() => Value::Array(PhpArray::new()),
            Expression::ConstantAccess(ca) => match identifier_name(&ca.name) {
                b"true" | b"TRUE" | b"True" => Value::Bool(true),
                b"false" | b"FALSE" | b"False" => Value::Bool(false),
                b"null" | b"NULL" | b"Null" => Value::Null,
                _ => {
                    return Err(Thrown::unsupported(
                        "non-constant default value",
                        "only literals / [] / true/false/null are supported for now",
                    ))
                }
            },
            Expression::UnaryPrefix(u) => {
                use UnaryPrefixOperator as Op;
                let inner = self.const_value(u.operand)?;
                match (&u.operator, inner) {
                    (Op::Negation(_), Value::Int(n)) => Value::Int(-n),
                    (Op::Negation(_), Value::Float(f)) => Value::Float(-f),
                    (Op::Plus(_), v @ (Value::Int(_) | Value::Float(_))) => v,
                    _ => {
                        return Err(Thrown::unsupported(
                            "non-constant default value",
                            "only literals / [] / true/false/null are supported for now",
                        ))
                    }
                }
            }
            _ => {
                return Err(Thrown::unsupported(
                    "non-constant default value",
                    "only literals / [] / true/false/null are supported for now",
                ))
            }
        };
        Ok(v)
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
        use phrust_frontend::cst::{ForBody, IfBody, SwitchBody, SwitchCase, WhileBody};
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
                f.initializations.iter().for_each(|e| self.scan_expr_vars(e));
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
                    ForeachTarget::KeyValue(t) => {
                        self.scan_expr_vars(t.key);
                        self.scan_expr_vars(t.value);
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
                        phrust_frontend::cst::MatchArm::Expression(a) => {
                            a.conditions.iter().for_each(|c| self.scan_expr_vars(c));
                            self.scan_expr_vars(a.expression);
                        }
                        phrust_frontend::cst::MatchArm::Default(a) => self.scan_expr_vars(a.expression),
                    }
                }
            }
            Expression::Construct(c) => self.scan_construct_vars(c),
            Expression::Call(call) => {
                if let phrust_frontend::cst::Call::Function(fc) = call {
                    for arg in fc.argument_list.arguments.iter() {
                        match arg {
                            Argument::Positional(p) => self.scan_expr_vars(p.value),
                            Argument::Named(n) => self.scan_expr_vars(n.value),
                        }
                    }
                }
            }
            _ => {}
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
            _ => {}
        }
    }

    // -- statement lowering --

    fn compile_stmt(&mut self, stmt: &Statement) -> Result<(), Thrown> {
        match stmt {
            Statement::OpeningTag(_) | Statement::ClosingTag(_) | Statement::Noop(_) => {}
            Statement::Inline(inline) => {
                if inline.kind.is_text() {
                    let k = self.intern_const(Value::str(inline.value.to_vec()));
                    self.emit(Instr::EchoConst { k });
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
                self.compile_expr(s.expression)?;
            }
            Statement::Block(b) => {
                for s in b.statements.iter() {
                    self.compile_stmt(s)?;
                }
            }
            Statement::HaltCompiler(_) => self.emit(Instr::Halt),
            // Top-level function declarations are hoisted/compiled separately.
            Statement::Function(_) => {}
            Statement::If(i) => self.compile_if(i)?,
            Statement::While(w) => self.compile_while(w)?,
            Statement::DoWhile(d) => self.compile_do_while(d)?,
            Statement::For(f) => self.compile_for(f)?,
            Statement::Switch(s) => self.compile_switch(s)?,
            Statement::Foreach(fe) => self.compile_foreach(fe)?,
            Statement::Unset(u) => self.compile_unset(u)?,
            Statement::Return(r) => match r.value {
                Some(e) => {
                    let src = self.compile_expr(e)?;
                    self.emit(Instr::Ret { src });
                }
                None => self.emit(Instr::RetNull),
            },
            Statement::Break(b) => self.compile_break_continue(b.level, true)?,
            Statement::Continue(c) => self.compile_break_continue(c.level, false)?,
            other => {
                return Err(Thrown::unsupported(
                    format!("{} statement", stmt_kind(other)),
                    "not implemented in the phase-1 compiler",
                ))
            }
        }
        self.reset_temps();
        Ok(())
    }

    // -- control-flow lowering --

    /// Compile a single statement or a colon-delimited statement sequence.
    fn compile_stmts<'a>(&mut self, stmts: impl Iterator<Item = &'a Statement<'a>>) -> Result<(), Thrown> {
        for s in stmts {
            self.compile_stmt(s)?;
        }
        Ok(())
    }

    fn compile_if(&mut self, i: &phrust_frontend::cst::If) -> Result<(), Thrown> {
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

    fn compile_while(&mut self, w: &phrust_frontend::cst::While) -> Result<(), Thrown> {
        let top = self.here();
        let cond = self.compile_expr(w.condition)?;
        let jf = self.code.len();
        self.emit(Instr::Nop); // JmpFalse cond -> end
        self.loops.push(LoopCtx::default());
        match &w.body {
            WhileBody::Statement(s) => self.compile_stmt(s)?,
            WhileBody::ColonDelimited(b) => self.compile_stmts(b.statements.iter())?,
        }
        let ctx = self.loops.pop().unwrap();
        self.emit(Instr::Jmp { target: top });
        let end = self.here();
        self.code[jf] = Instr::JmpFalse { cond, target: end };
        self.patch_all(ctx.break_jumps, end);
        self.patch_all(ctx.continue_jumps, top);
        Ok(())
    }

    fn compile_do_while(&mut self, d: &phrust_frontend::cst::DoWhile) -> Result<(), Thrown> {
        let top = self.here();
        self.loops.push(LoopCtx::default());
        self.compile_stmt(d.statement)?;
        let ctx = self.loops.pop().unwrap();
        let cont = self.here(); // continue -> re-test the condition
        let cond = self.compile_expr(d.condition)?;
        self.emit(Instr::JmpTrue { cond, target: top });
        let end = self.here();
        self.patch_all(ctx.break_jumps, end);
        self.patch_all(ctx.continue_jumps, cont);
        Ok(())
    }

    fn compile_for(&mut self, f: &phrust_frontend::cst::For) -> Result<(), Thrown> {
        for e in f.initializations.iter() {
            self.compile_expr(e)?;
        }
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
        self.loops.push(LoopCtx::default());
        match &f.body {
            ForBody::Statement(s) => self.compile_stmt(s)?,
            ForBody::ColonDelimited(b) => self.compile_stmts(b.statements.iter())?,
        }
        let ctx = self.loops.pop().unwrap();
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

    fn compile_switch(&mut self, sw: &phrust_frontend::cst::Switch) -> Result<(), Thrown> {
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
                self.emit(Instr::Cmp { dst: t, a: subject, b: cexpr, kind: CmpKind::Eq });
                let pos = self.code.len();
                self.emit(Instr::Nop); // JmpTrue t -> case body
                dispatch.push((idx, pos, t));
            }
        }
        let default_idx = cases.iter().position(|c| matches!(c, SwitchCase::Default(_)));
        let no_match_pos = self.code.len();
        self.emit(Instr::Nop); // Jmp -> default body or end

        // Case bodies in source order (fall-through: no jumps between them).
        self.loops.push(LoopCtx::default()); // switch is one break/continue level
        let mut starts = vec![0u32; cases.len()];
        for (idx, case) in cases.iter().enumerate() {
            starts[idx] = self.here();
            match case {
                SwitchCase::Expression(c) => self.compile_stmts(c.statements.iter())?,
                SwitchCase::Default(c) => self.compile_stmts(c.statements.iter())?,
            }
        }
        let ctx = self.loops.pop().unwrap();
        let end = self.here();

        for (idx, pos, cond) in dispatch {
            self.code[pos] = Instr::JmpTrue { cond, target: starts[idx] };
        }
        let no_match_target = default_idx.map(|i| starts[i]).unwrap_or(end);
        self.patch_jmp(no_match_pos, no_match_target);
        // In a switch, continue (level 1) behaves like break.
        self.patch_all(ctx.break_jumps, end);
        self.patch_all(ctx.continue_jumps, end);

        self.temp_floor = saved_floor;
        Ok(())
    }

    fn compile_foreach(&mut self, fe: &phrust_frontend::cst::Foreach) -> Result<(), Thrown> {
        let src = self.compile_expr(fe.expression)?;
        let arr_reg = self.alloc_temp();
        let pos_reg = self.alloc_temp();
        let saved_floor = self.temp_floor;
        self.temp_floor = self.next_temp; // protect src/arr_reg/pos_reg across the body

        let (key_cv, val_cv) = match &fe.target {
            ForeachTarget::Value(t) => (Reg::MAX, self.foreach_target_cv(t.value)?),
            ForeachTarget::KeyValue(t) => {
                (self.foreach_target_cv(t.key)?, self.foreach_target_cv(t.value)?)
            }
        };

        let reset_pos = self.code.len();
        self.emit(Instr::Nop); // FeReset (needs end)
        let fetch_label = self.here();
        let fetch_pos = self.code.len();
        self.emit(Instr::Nop); // FeFetch (needs end)

        self.loops.push(LoopCtx::default());
        match &fe.body {
            ForeachBody::Statement(s) => self.compile_stmt(s)?,
            ForeachBody::ColonDelimited(b) => self.compile_stmts(b.statements.iter())?,
        }
        let ctx = self.loops.pop().unwrap();
        self.emit(Instr::Jmp { target: fetch_label });
        let end = self.here();
        self.code[reset_pos] = Instr::FeReset { arr_reg, pos_reg, src, end };
        self.code[fetch_pos] = Instr::FeFetch { arr_reg, pos_reg, key_cv, val_cv, end };
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

    fn compile_unset(&mut self, u: &phrust_frontend::cst::Unset) -> Result<(), Thrown> {
        for v in u.values.iter() {
            match v {
                Expression::Variable(Variable::Direct(d)) => {
                    // A CV can't be removed; null is observably equivalent to
                    // unset for isset/??/empty.
                    let cv = self.cv(strip_dollar(d.name));
                    self.emit(Instr::LoadNull { dst: cv });
                }
                Expression::ArrayAccess(_) => {
                    let mut steps = Vec::new();
                    let root = self.collect_lvalue_steps(v, &mut steps)?;
                    self.emit_cursor(root, &steps);
                    self.emit(Instr::UnsetLval);
                }
                _ => {
                    return Err(Thrown::unsupported(
                        "unset target",
                        "only variables and array elements",
                    ))
                }
            }
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
        self.emit(Instr::Nop); // patched to Jmp at loop close
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
                        self.emit(Instr::Cmp { dst: t, a: subject, b: c, kind: CmpKind::Identical });
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
            self.code[pos] = Instr::JmpTrue { cond, target: starts[idx] };
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
        match expr {
            Expression::Literal(lit) => self.compile_literal(lit),
            Expression::Parenthesized(p) => self.compile_expr(p.expression),
            Expression::Variable(Variable::Direct(d)) => Ok(self.cv(strip_dollar(d.name))),
            Expression::Variable(_) => Err(Thrown::unsupported(
                "indirect/nested variable",
                "variable-variables are refused / deferred",
            )),
            Expression::Binary(b) => self.compile_binary(b),
            Expression::UnaryPrefix(u) => self.compile_unary_prefix(u),
            Expression::UnaryPostfix(u) => self.compile_unary_postfix(u),
            Expression::Assignment(a) => self.compile_assignment(a),
            Expression::Conditional(c) => self.compile_conditional(c),
            Expression::Array(arr) => self.compile_array(arr.elements.iter()),
            Expression::LegacyArray(arr) => self.compile_array(arr.elements.iter()),
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
            Expression::Construct(c) => self.compile_construct(c),
            Expression::Call(call) => self.compile_call(call),
            other => Err(Thrown::unsupported(
                format!("{} expression", expr_kind(other)),
                "not implemented in the phase-1 compiler",
            )),
        }
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

    fn compile_binary(&mut self, b: &phrust_frontend::cst::Binary) -> Result<Reg, Thrown> {
        use BinaryOperator as Op;
        // Short-circuit operators lower to branches.
        match &b.operator {
            Op::And(_) | Op::LowAnd(_) => return self.compile_logical(b, true),
            Op::Or(_) | Op::LowOr(_) => return self.compile_logical(b, false),
            Op::NullCoalesce(_) => return self.compile_coalesce(b),
            _ => {}
        }

        let a = self.compile_expr(b.lhs)?;
        let r = self.compile_expr(b.rhs)?;
        let dst = self.alloc_temp();
        let cmp = |k| Instr::Cmp { dst, a, b: r, kind: k };
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
                Instr::Cmp { dst, a, b: r, kind: CmpKind::Ne }
            }
            Op::Instanceof(_) => {
                return Err(Thrown::unsupported("instanceof", "objects arrive in phase 7"))
            }
            Op::And(_) | Op::Or(_) | Op::LowAnd(_) | Op::LowOr(_) | Op::NullCoalesce(_) => {
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
        b: &phrust_frontend::cst::Binary,
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
            Instr::JmpFalse { cond: a, target: short }
        } else {
            Instr::JmpTrue { cond: a, target: short }
        };
        self.code[jend] = Instr::Jmp { target: end };
        Ok(dst)
    }

    fn compile_coalesce(&mut self, b: &phrust_frontend::cst::Binary) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        let a = self.compile_expr(b.lhs)?;
        self.emit(Instr::Mov { dst, src: a });
        let jnn = self.code.len();
        self.emit(Instr::Nop); // JmpNotNull dst -> end
        let r = self.compile_expr(b.rhs)?;
        self.emit(Instr::Mov { dst, src: r });
        let end = self.code.len() as u32;
        self.code[jnn] = Instr::JmpNotNull { src: dst, target: end };
        Ok(dst)
    }

    fn compile_unary_prefix(
        &mut self,
        u: &phrust_frontend::cst::UnaryPrefix,
    ) -> Result<Reg, Thrown> {
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
            Op::Reference(_) => {
                return Err(Thrown::unsupported("& reference", "references arrive in phase 6"))
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
            Op::ErrorControl(_) => Instr::Mov { dst, src: a },
            Op::IntCast(..) | Op::IntegerCast(..) => Instr::CastInt { dst, a },
            Op::FloatCast(..) | Op::DoubleCast(..) => Instr::CastFloat { dst, a },
            Op::StringCast(..) | Op::BinaryCast(..) => Instr::CastString { dst, a },
            Op::BoolCast(..) | Op::BooleanCast(..) => Instr::CastBool { dst, a },
            Op::ArrayCast(..) => Instr::CastArray { dst, a },
            Op::ObjectCast(..) => {
                return Err(Thrown::unsupported("(object) cast", "objects arrive in phase 7"))
            }
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
        u: &phrust_frontend::cst::UnaryPostfix,
    ) -> Result<Reg, Thrown> {
        use phrust_frontend::cst::UnaryPostfixOperator as Op;
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
        match target {
            Expression::Variable(Variable::Direct(d)) => {
                let cv = self.cv(strip_dollar(d.name));
                let dst = self.alloc_temp();
                self.emit(Instr::IncDec { dst, cv, pre, delta });
                Ok(dst)
            }
            Expression::ArrayAccess(_) => {
                let mut steps = Vec::new();
                let root = self.collect_lvalue_steps(target, &mut steps)?;
                self.emit_cursor(root, &steps);
                let dst = self.alloc_temp();
                self.emit(Instr::IncDecLval { dst, pre, delta });
                Ok(dst)
            }
            _ => Err(Thrown::unsupported("++/-- target", "only variables and array elements")),
        }
    }

    fn compile_conditional(
        &mut self,
        c: &phrust_frontend::cst::Conditional,
    ) -> Result<Reg, Thrown> {
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
        self.code[jf] = Instr::JmpFalse { cond, target: else_label };
        self.code[jend] = Instr::Jmp { target: end };
        Ok(dst)
    }

    fn compile_array<'a>(
        &mut self,
        els: impl Iterator<Item = &'a ArrayElement<'a>>,
    ) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        self.emit(Instr::NewArray { dst, cap: 0 });
        for el in els {
            match el {
                ArrayElement::KeyValue(kv) => {
                    let key = self.compile_expr(kv.key)?;
                    let val = self.compile_expr(kv.value)?;
                    self.emit(Instr::ArrSet { arr: dst, key, val });
                }
                ArrayElement::Value(v) => {
                    let val = self.compile_expr(v.value)?;
                    self.emit(Instr::ArrPush { arr: dst, val });
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
                let n = self.intern_name(name);
                self.emit(Instr::FetchConst { dst, name: n });
            }
        }
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
            Construct::Eval(_) => Err(Thrown::unsupported(
                "eval()",
                "executes a string as PHP code; refused by design",
            )),
            Construct::Include(_)
            | Construct::IncludeOnce(_)
            | Construct::Require(_)
            | Construct::RequireOnce(_) => Err(Thrown::unsupported(
                "include/require",
                "file inclusion arrives in a later phase",
            )),
            Construct::Isset(i) => {
                // AND over each operand: true iff every value is non-null.
                let dst = self.alloc_temp();
                let mut jumps = Vec::new();
                for v in i.values.iter() {
                    let r = self.compile_expr(v)?; // null if absent (DimFetch returns null)
                    let pos = self.code.len();
                    self.emit(Instr::Nop); // JmpNull r -> false
                    jumps.push((pos, r));
                }
                self.emit(Instr::LoadBool { dst, b: true });
                let jend = self.code.len();
                self.emit(Instr::Nop); // Jmp end
                let lfalse = self.here();
                self.emit(Instr::LoadBool { dst, b: false });
                let lend = self.here();
                for (pos, r) in jumps {
                    self.code[pos] = Instr::JmpNull { src: r, target: lfalse };
                }
                self.code[jend] = Instr::Jmp { target: lend };
                Ok(dst)
            }
            Construct::Empty(e) => {
                // empty(x) == !to_bool(x); absent/null -> true (Not handles it).
                let r = self.compile_expr(e.value)?;
                let dst = self.alloc_temp();
                self.emit(Instr::Not { dst, a: r });
                Ok(dst)
            }
            Construct::Exit(_) | Construct::Die(_) => Err(Thrown::unsupported(
                "exit/die",
                "arrive with the SAPI exit path",
            )),
        }
    }

    fn compile_call(&mut self, call: &phrust_frontend::cst::Call) -> Result<Reg, Thrown> {
        use phrust_frontend::cst::Call;
        let Call::Function(fc) = call else {
            return Err(Thrown::unsupported("method/static call", "objects arrive in phase 7"));
        };
        let name_bytes = match fc.function {
            Expression::Identifier(id) => identifier_name(id).to_vec(),
            Expression::ConstantAccess(ca) => identifier_name(&ca.name).to_vec(),
            _ => {
                return Err(Thrown::unsupported(
                    "dynamic function call",
                    "callable values arrive with closures",
                ))
            }
        };

        // Compile args, then marshal into a contiguous window.
        let mut arg_regs = Vec::new();
        for arg in fc.argument_list.arguments.iter() {
            match arg {
                Argument::Positional(p) if p.ellipsis.is_none() => {
                    arg_regs.push(self.compile_expr(p.value)?);
                }
                Argument::Positional(_) => {
                    return Err(Thrown::unsupported(
                        "argument spread",
                        "variadic call sites arrive with user functions",
                    ))
                }
                Argument::Named(_) => {
                    return Err(Thrown::unsupported(
                        "named arguments",
                        "arrive with user functions in phase 5",
                    ))
                }
            }
        }
        let argc = arg_regs.len() as u16;
        let base = self.alloc_temps(argc);
        for (i, r) in arg_regs.into_iter().enumerate() {
            self.emit(Instr::Mov { dst: base + i as Reg, src: r });
        }
        let name = self.intern_name(&name_bytes);
        let dst = self.alloc_temp();
        self.emit(Instr::Call { dst, name, base, argc });
        Ok(dst)
    }

    /// Collect an lvalue path (root CV + pre-evaluated key/append steps). All
    /// index expressions are compiled into registers here, so the subsequent
    /// cursor opcode sequence is contiguous (no nested cursors).
    fn collect_lvalue_steps(
        &mut self,
        target: &Expression,
        steps: &mut Vec<CursorStep>,
    ) -> Result<Reg, Thrown> {
        match target {
            Expression::Variable(Variable::Direct(d)) => Ok(self.cv(strip_dollar(d.name))),
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
            _ => Err(Thrown::unsupported(
                "assignment target",
                "only variables and array elements are supported",
            )),
        }
    }

    fn emit_cursor(&mut self, root: Reg, steps: &[CursorStep]) {
        self.emit(Instr::LvalCv { cv: root });
        for step in steps {
            match step {
                CursorStep::Key(r) => self.emit(Instr::LvalKey { key: *r }),
                CursorStep::Append => self.emit(Instr::LvalAppend),
            }
        }
    }

    fn compile_assignment(
        &mut self,
        a: &phrust_frontend::cst::Assignment,
    ) -> Result<Reg, Thrown> {
        // Array-element / append target: use the lvalue cursor.
        if matches!(a.lhs, Expression::ArrayAccess(_) | Expression::ArrayAppend(_)) {
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
                    self.emit(compound_op(op, res, cur, rhs)?);
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
                        self.code[jnn] = Instr::JmpNotNull { src: dst, target: end };
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
}

// --- free helpers ---------------------------------------------------------

fn strip_dollar(name: &[u8]) -> &[u8] {
    name.strip_prefix(b"$").unwrap_or(name)
}

/// Build the instruction for a compound-assignment operator `dst = a <op> b`.
fn compound_op(op: &AssignmentOperator, dst: Reg, a: Reg, b: Reg) -> Result<Instr, Thrown> {
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
                "??= on an array element",
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

/// Fold a literal AST node to its runtime value (same rules as the oracle).
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
        Statement::For(_) => "for",
        Statement::Foreach(_) => "foreach",
        Statement::Switch(_) => "switch",
        Statement::Function(_) => "function",
        Statement::Return(_) => "return",
        Statement::Class(_) => "class",
        Statement::Try(_) => "try",
        _ => "this",
    }
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

// FnIdx is reserved for user-function references (phase 5+).
const _: fn() = || {
    let _ = FnIdx::default;
};
