//! The tree-walking interpreter.
//!
//! Walks the mago CST (`rhp_frontend::ParsedFile`), producing owned
//! `rhp_values::Value`s. Statement execution returns [`Flow`] (control
//! transfer) in `Ok`, with real throws + engine refusals in `Err(Thrown)`.

use std::collections::HashMap;

use mago_syntax::cst::{
    Argument, ArrayAccess, ArrayElement, AssignmentOperator, BinaryOperator, CompositeString,
    Construct, Expression, ForBody, Foreach, ForeachBody, ForeachTarget, Function, Identifier,
    IfBody, Literal, Match, MatchArm, Program, Statement, StringPart, SwitchBody, SwitchCase,
    UnaryPrefixOperator, Variable, WhileBody,
};

pub use rhp_builtins::Thrown;
use rhp_builtins::{self as builtins, Host};
use rhp_semantics::{cast_to_array, deref, inc_dec_value, ops};
use rhp_values::conv;
use rhp_values::{ArrayKey, PhpArray, Value};

/// Control-flow signal carried up the statement-exec stack (see
/// `docs/design/README.md` §3). Real throws use `Err(Thrown)` instead.
pub enum Flow {
    Normal,
    Break(u32),
    Continue(u32),
    Return(Value),
}

type ExecResult = Result<Flow, Thrown>;
type EvalResult = Result<Value, Thrown>;

/// The interpreter state. The lifetime `'a` ties it to the parsed program whose
/// function declarations it references.
pub struct Interp<'a> {
    out: Vec<u8>,
    vars: HashMap<Vec<u8>, Value>,
    constants: HashMap<Vec<u8>, Value>,
    functions: HashMap<Vec<u8>, &'a Function<'a>>, // lowercase name -> declaration
    bc_scale: i64,
}

impl Host for Interp<'_> {
    fn write_out(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    fn bc_scale(&self) -> i64 {
        self.bc_scale
    }

    fn set_bc_scale(&mut self, scale: i64) {
        self.bc_scale = scale;
    }
}

impl Default for Interp<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Interp<'a> {
    pub fn new() -> Self {
        let mut constants = HashMap::new();
        for (name, value) in builtins::constants() {
            constants.insert(name.as_bytes().to_vec(), value);
        }
        Interp {
            out: Vec::new(),
            vars: HashMap::new(),
            constants,
            functions: HashMap::new(),
            bc_scale: 0,
        }
    }

    /// The accumulated output buffer.
    pub fn output(&self) -> &[u8] {
        &self.out
    }

    /// Run a parsed program to completion. Returns the process exit code.
    /// `program` borrows a `rhp_frontend::Arena` that must outlive the call.
    pub fn run(&mut self, program: &'a Program<'a>) -> Result<i32, Thrown> {
        // Hoist top-level function declarations.
        for stmt in program.statements.iter() {
            if let Statement::Function(func) = stmt {
                self.functions
                    .insert(func.name.value.to_ascii_lowercase(), func);
            }
        }
        for stmt in program.statements.iter() {
            match self.exec_stmt(stmt) {
                Ok(Flow::Normal) => {}
                Ok(Flow::Return(_)) => break, // top-level return ends the script
                Ok(Flow::Break(_)) | Ok(Flow::Continue(_)) => {} // ignored at top level
                Err(Thrown::Exit(code)) => return Ok(code),
                Err(other) => return Err(other),
            }
        }
        Ok(0)
    }

    // -- statements --------------------------------------------------------

    fn exec_stmt(&mut self, stmt: &'a Statement<'a>) -> ExecResult {
        match stmt {
            Statement::OpeningTag(_) | Statement::ClosingTag(_) | Statement::Noop(_) => {
                Ok(Flow::Normal)
            }
            Statement::Inline(inline) => {
                // Inline HTML is emitted verbatim; a shebang line is not.
                if inline.kind.is_text() {
                    let bytes = inline.value.to_vec();
                    self.write_out(&bytes);
                }
                Ok(Flow::Normal)
            }
            Statement::Echo(echo) => {
                for value in echo.values.iter() {
                    let v = self.walk_expr(value)?;
                    let bytes = conv::to_string_bytes(&v);
                    self.write_out(&bytes);
                }
                Ok(Flow::Normal)
            }
            Statement::EchoTag(echo) => {
                for value in echo.values.iter() {
                    let v = self.walk_expr(value)?;
                    let bytes = conv::to_string_bytes(&v);
                    self.write_out(&bytes);
                }
                Ok(Flow::Normal)
            }
            Statement::Expression(stmt) => {
                self.walk_expr(stmt.expression)?;
                Ok(Flow::Normal)
            }
            Statement::Block(block) => {
                for s in block.statements.iter() {
                    match self.exec_stmt(s)? {
                        Flow::Normal => {}
                        other => return Ok(other),
                    }
                }
                Ok(Flow::Normal)
            }
            Statement::HaltCompiler(_) => Ok(Flow::Return(Value::Null)),
            Statement::Function(_) => Ok(Flow::Normal), // hoisted in run()
            Statement::If(i) => self.exec_if(i),
            Statement::While(w) => self.exec_while(w),
            Statement::DoWhile(d) => self.exec_do_while(d),
            Statement::For(f) => self.exec_for(f),
            Statement::Foreach(fe) => self.exec_foreach(fe),
            Statement::Switch(s) => self.exec_switch(s),
            Statement::Unset(u) => {
                for v in u.values.iter() {
                    let (root, steps) = self.collect_path(v)?;
                    if steps.is_empty() {
                        self.vars.remove(&root);
                    } else if let Some(slot) = self.vars.get_mut(&root) {
                        unset_path(slot, &steps);
                    }
                }
                Ok(Flow::Normal)
            }
            Statement::Return(r) => {
                let v = match r.value {
                    Some(e) => self.walk_expr(e)?,
                    None => Value::Null,
                };
                Ok(Flow::Return(v))
            }
            Statement::Break(b) => Ok(Flow::Break(flow_level(b.level)?)),
            Statement::Continue(c) => Ok(Flow::Continue(flow_level(c.level)?)),
            other => Err(Thrown::unsupported(
                format!("{} statement", stmt_name(other)),
                "not implemented in the phase-1 scaffold",
            )),
        }
    }

    /// Execute a sequence of statements, propagating the first non-Normal flow.
    fn exec_stmts(&mut self, stmts: impl Iterator<Item = &'a Statement<'a>>) -> ExecResult {
        for s in stmts {
            match self.exec_stmt(s)? {
                Flow::Normal => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_if(&mut self, i: &'a mago_syntax::cst::If<'a>) -> ExecResult {
        match &i.body {
            IfBody::Statement(b) => {
                if conv::to_bool(&self.walk_expr(i.condition)?) {
                    return self.exec_stmt(b.statement);
                }
                for c in b.else_if_clauses.iter() {
                    if conv::to_bool(&self.walk_expr(c.condition)?) {
                        return self.exec_stmt(c.statement);
                    }
                }
                if let Some(e) = &b.else_clause {
                    return self.exec_stmt(e.statement);
                }
                Ok(Flow::Normal)
            }
            IfBody::ColonDelimited(b) => {
                if conv::to_bool(&self.walk_expr(i.condition)?) {
                    return self.exec_stmts(b.statements.iter());
                }
                for c in b.else_if_clauses.iter() {
                    if conv::to_bool(&self.walk_expr(c.condition)?) {
                        return self.exec_stmts(c.statements.iter());
                    }
                }
                if let Some(e) = &b.else_clause {
                    return self.exec_stmts(e.statements.iter());
                }
                Ok(Flow::Normal)
            }
        }
    }

    fn exec_while(&mut self, w: &'a mago_syntax::cst::While<'a>) -> ExecResult {
        while conv::to_bool(&self.walk_expr(w.condition)?) {
            let flow = match &w.body {
                WhileBody::Statement(s) => self.exec_stmt(s)?,
                WhileBody::ColonDelimited(b) => self.exec_stmts(b.statements.iter())?,
            };
            match loop_decision(flow) {
                LoopDecision::Iterate => {}
                LoopDecision::Stop => break,
                LoopDecision::Propagate(f) => return Ok(f),
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_do_while(&mut self, d: &'a mago_syntax::cst::DoWhile<'a>) -> ExecResult {
        loop {
            let flow = self.exec_stmt(d.statement)?;
            match loop_decision(flow) {
                LoopDecision::Iterate => {}
                LoopDecision::Stop => break,
                LoopDecision::Propagate(f) => return Ok(f),
            }
            if !conv::to_bool(&self.walk_expr(d.condition)?) {
                break;
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_for(&mut self, f: &'a mago_syntax::cst::For<'a>) -> ExecResult {
        for e in f.initializations.iter() {
            self.walk_expr(e)?;
        }
        loop {
            // all conditions evaluated; the last controls the loop
            let mut run = true;
            let mut last = None;
            for e in f.conditions.iter() {
                last = Some(self.walk_expr(e)?);
            }
            if let Some(v) = last {
                run = conv::to_bool(&v);
            }
            if !run {
                break;
            }
            let flow = match &f.body {
                ForBody::Statement(s) => self.exec_stmt(s)?,
                ForBody::ColonDelimited(b) => self.exec_stmts(b.statements.iter())?,
            };
            match loop_decision(flow) {
                LoopDecision::Iterate => {}
                LoopDecision::Stop => break,
                LoopDecision::Propagate(f) => return Ok(f),
            }
            for e in f.increments.iter() {
                self.walk_expr(e)?;
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_foreach(&mut self, fe: &'a Foreach<'a>) -> ExecResult {
        let subject = self.walk_expr(fe.expression)?;
        let Value::Array(arr) = deref(&subject) else {
            // php warns "foreach() argument must be of type array|object"; skip.
            return Ok(Flow::Normal);
        };
        // Iterate a snapshot (COW): body mutations of the source don't affect it.
        let entries: Vec<(ArrayKey, Value)> =
            arr.iter().map(|(k, v)| (k.clone(), deref(v))).collect();
        for (k, v) in entries {
            match &fe.target {
                ForeachTarget::Value(t) => self.bind_foreach(t.value, v)?,
                ForeachTarget::KeyValue(t) => {
                    let key = match k {
                        ArrayKey::Int(n) => Value::Int(n),
                        ArrayKey::Str(s) => Value::Str(s),
                    };
                    self.bind_foreach(t.key, key)?;
                    self.bind_foreach(t.value, v)?;
                }
            }
            let flow = match &fe.body {
                ForeachBody::Statement(s) => self.exec_stmt(s)?,
                ForeachBody::ColonDelimited(b) => self.exec_stmts(b.statements.iter())?,
            };
            match loop_decision(flow) {
                LoopDecision::Iterate => {}
                LoopDecision::Stop => break,
                LoopDecision::Propagate(f) => return Ok(f),
            }
        }
        Ok(Flow::Normal)
    }

    /// Bind one `foreach` target (key or value) to a produced value.
    fn bind_foreach(&mut self, target: &'a Expression<'a>, value: Value) -> Result<(), Thrown> {
        if let Expression::UnaryPrefix(u) = target {
            if matches!(u.operator, UnaryPrefixOperator::Reference(_)) {
                return Err(Thrown::unsupported(
                    "foreach by reference",
                    "references arrive in the next phase",
                ));
            }
        }
        match target {
            Expression::Variable(Variable::Direct(d)) => {
                self.vars.insert(strip_dollar(d.name).to_vec(), value);
                Ok(())
            }
            Expression::ArrayAccess(_) | Expression::ArrayAppend(_) => {
                let (root, steps) = self.collect_path(target)?;
                let slot = self.vars.entry(root).or_insert(Value::Null);
                assign_path(slot, &steps, value)
            }
            Expression::List(_) => Err(Thrown::unsupported(
                "list() destructuring",
                "arrives in a later phase",
            )),
            _ => Err(Thrown::unsupported("foreach target", "unsupported target")),
        }
    }

    fn exec_switch(&mut self, sw: &'a mago_syntax::cst::Switch<'a>) -> ExecResult {
        let subject = self.walk_expr(sw.expression)?;
        let cases: Vec<&SwitchCase> = match &sw.body {
            SwitchBody::BraceDelimited(b) => b.cases.iter().collect(),
            SwitchBody::ColonDelimited(b) => b.cases.iter().collect(),
        };
        // First matching case (loose ==), else default.
        let mut start = None;
        for (idx, case) in cases.iter().enumerate() {
            if let SwitchCase::Expression(c) = case {
                let cv = self.walk_expr(c.expression)?;
                if ops::loose_eq(&subject, &cv) {
                    start = Some(idx);
                    break;
                }
            }
        }
        let start = start.or_else(|| {
            cases
                .iter()
                .position(|c| matches!(c, SwitchCase::Default(_)))
        });
        let Some(start) = start else {
            return Ok(Flow::Normal);
        };
        // Execute from there with fall-through.
        for case in &cases[start..] {
            let stmts = match case {
                SwitchCase::Expression(c) => &c.statements,
                SwitchCase::Default(c) => &c.statements,
            };
            match self.exec_stmts(stmts.iter())? {
                Flow::Normal => {}
                // break/continue at level 1 both leave the switch
                Flow::Break(1) | Flow::Continue(1) => return Ok(Flow::Normal),
                Flow::Break(n) => return Ok(Flow::Break(n - 1)),
                Flow::Continue(n) => return Ok(Flow::Continue(n - 1)),
                Flow::Return(v) => return Ok(Flow::Return(v)),
            }
        }
        Ok(Flow::Normal)
    }

    // -- expressions -------------------------------------------------------

    fn walk_expr(&mut self, expr: &'a Expression<'a>) -> EvalResult {
        match expr {
            Expression::Literal(lit) => Ok(walk_literal(lit)),
            Expression::Parenthesized(p) => self.walk_expr(p.expression),
            Expression::Binary(b) => self.walk_binary(b),
            Expression::UnaryPrefix(u) => self.walk_unary_prefix(u),
            Expression::UnaryPostfix(u) => self.walk_unary_postfix(u),
            Expression::Variable(v) => self.read_variable(v),
            Expression::Assignment(a) => self.walk_assignment(a),
            Expression::Conditional(c) => self.walk_conditional(c),
            Expression::Array(arr) => self.walk_array(arr.elements.iter()),
            Expression::LegacyArray(arr) => self.walk_array(arr.elements.iter()),
            Expression::ConstantAccess(ca) => self.walk_constant(&ca.name),
            Expression::Identifier(id) => self.walk_constant(id),
            Expression::ArrayAccess(aa) => self.walk_array_access(aa),
            Expression::CompositeString(cs) => self.walk_composite(cs),
            Expression::Match(m) => self.walk_match(m),
            Expression::Construct(c) => self.walk_construct(c),
            Expression::Call(call) => self.walk_call(call),
            other => Err(Thrown::unsupported(
                format!("{} expression", expr_name(other)),
                "not implemented in the phase-1 scaffold",
            )),
        }
    }

    fn walk_binary(&mut self, b: &'a mago_syntax::cst::Binary<'a>) -> EvalResult {
        use BinaryOperator as Op;
        // Short-circuiting operators evaluate the rhs lazily.
        match &b.operator {
            Op::And(_) | Op::LowAnd(_) => {
                let l = self.walk_expr(b.lhs)?;
                return Ok(Value::Bool(
                    conv::to_bool(&l) && conv::to_bool(&self.walk_expr(b.rhs)?),
                ));
            }
            Op::Or(_) | Op::LowOr(_) => {
                let l = self.walk_expr(b.lhs)?;
                return Ok(Value::Bool(
                    conv::to_bool(&l) || conv::to_bool(&self.walk_expr(b.rhs)?),
                ));
            }
            Op::NullCoalesce(_) => {
                // `??`: lhs if set and non-null, else rhs. (Full isset-style
                // suppression of undefined comes with array access.)
                let l = self.walk_expr(b.lhs).unwrap_or(Value::Null);
                return if matches!(deref(&l), Value::Null) {
                    self.walk_expr(b.rhs)
                } else {
                    Ok(l)
                };
            }
            _ => {}
        }

        let l = self.walk_expr(b.lhs)?;
        let r = self.walk_expr(b.rhs)?;
        Ok(match &b.operator {
            Op::Addition(_) => ops::add(&l, &r)?,
            Op::Subtraction(_) => ops::sub(&l, &r)?,
            Op::Multiplication(_) => ops::mul(&l, &r)?,
            Op::Division(_) => ops::div(&l, &r)?,
            Op::Modulo(_) => ops::modulo(&l, &r)?,
            Op::Exponentiation(_) => ops::pow(&l, &r)?,
            Op::BitwiseAnd(_) => ops::bitwise(&l, &r, ops::BitOp::And),
            Op::BitwiseOr(_) => ops::bitwise(&l, &r, ops::BitOp::Or),
            Op::BitwiseXor(_) => ops::bitwise(&l, &r, ops::BitOp::Xor),
            Op::LeftShift(_) => ops::bitwise(&l, &r, ops::BitOp::Shl),
            Op::RightShift(_) => ops::bitwise(&l, &r, ops::BitOp::Shr),
            Op::StringConcat(_) => ops::concat(&l, &r),
            Op::Equal(_) => Value::Bool(ops::loose_eq(&l, &r)),
            Op::NotEqual(_) | Op::AngledNotEqual(_) => Value::Bool(!ops::loose_eq(&l, &r)),
            Op::Identical(_) => Value::Bool(ops::strict_eq(&l, &r)),
            Op::NotIdentical(_) => Value::Bool(!ops::strict_eq(&l, &r)),
            Op::LessThan(_) => Value::Bool(ops::loose_compare(&l, &r).is_lt()),
            Op::LessThanOrEqual(_) => Value::Bool(ops::loose_compare(&l, &r).is_le()),
            Op::GreaterThan(_) => Value::Bool(ops::loose_compare(&l, &r).is_gt()),
            Op::GreaterThanOrEqual(_) => Value::Bool(ops::loose_compare(&l, &r).is_ge()),
            Op::Spaceship(_) => ops::spaceship(&l, &r),
            Op::And(_) | Op::Or(_) | Op::LowAnd(_) | Op::LowOr(_) | Op::NullCoalesce(_) => {
                unreachable!("handled above")
            }
            Op::LowXor(_) => Value::Bool(conv::to_bool(&l) ^ conv::to_bool(&r)),
            Op::Instanceof(_) => {
                return Err(Thrown::unsupported(
                    "instanceof",
                    "objects arrive in phase 7",
                ))
            }
        })
    }

    fn walk_unary_prefix(&mut self, u: &'a mago_syntax::cst::UnaryPrefix<'a>) -> EvalResult {
        use UnaryPrefixOperator as Op;
        // inc/dec need an lvalue, handled before evaluating the operand as a value.
        match &u.operator {
            Op::PreIncrement(_) => return self.pre_inc_dec(u.operand, 1),
            Op::PreDecrement(_) => return self.pre_inc_dec(u.operand, -1),
            Op::UnsetCast(..) | Op::VoidCast(..) | Op::RealCast(..) => {
                return Err(Thrown::unsupported(
                    "(unset)/(void)/(real) cast",
                    "removed/refused in PHP 8",
                ))
            }
            Op::Reference(_) => {
                return Err(Thrown::unsupported(
                    "& reference",
                    "references arrive in phase 6",
                ))
            }
            _ => {}
        }
        let v = self.walk_expr(u.operand)?;
        Ok(match &u.operator {
            Op::Negation(_) => ops::negate(&v)?,
            Op::Plus(_) => conv::to_number(&deref(&v)).0.to_value(),
            Op::Not(_) => Value::Bool(!conv::to_bool(&v)),
            Op::BitwiseNot(_) => ops::bitwise_not(&v),
            Op::ErrorControl(_) => v, // `@` suppresses warnings; value passes through
            Op::IntCast(..) | Op::IntegerCast(..) => Value::Int(conv::to_int(&v)),
            Op::FloatCast(..) | Op::DoubleCast(..) => Value::Float(conv::to_float(&v)),
            Op::StringCast(..) | Op::BinaryCast(..) => Value::str(conv::to_string_bytes(&v)),
            Op::BoolCast(..) | Op::BooleanCast(..) => Value::Bool(conv::to_bool(&v)),
            Op::ArrayCast(..) => cast_to_array(v),
            Op::ObjectCast(..) => {
                return Err(Thrown::unsupported(
                    "(object) cast",
                    "objects arrive in phase 7",
                ))
            }
            _ => unreachable!("handled above"),
        })
    }

    fn walk_unary_postfix(&mut self, u: &'a mago_syntax::cst::UnaryPostfix<'a>) -> EvalResult {
        use mago_syntax::cst::UnaryPostfixOperator as Op;
        match &u.operator {
            Op::PostIncrement(_) => self.post_inc_dec(u.operand, 1),
            Op::PostDecrement(_) => self.post_inc_dec(u.operand, -1),
        }
    }

    fn walk_conditional(&mut self, c: &'a mago_syntax::cst::Conditional<'a>) -> EvalResult {
        let cond = self.walk_expr(c.condition)?;
        match &c.then {
            Some(then) => {
                if conv::to_bool(&cond) {
                    self.walk_expr(then)
                } else {
                    self.walk_expr(c.r#else)
                }
            }
            // Elvis `?:` — reuse the condition value when truthy.
            None => {
                if conv::to_bool(&cond) {
                    Ok(cond)
                } else {
                    self.walk_expr(c.r#else)
                }
            }
        }
    }

    fn walk_array(&mut self, elements: impl Iterator<Item = &'a ArrayElement<'a>>) -> EvalResult {
        let mut arr = PhpArray::new();
        for el in elements {
            match el {
                ArrayElement::KeyValue(kv) => {
                    let key = self.walk_expr(kv.key)?;
                    let val = self.walk_expr(kv.value)?;
                    let k = ArrayKey::from_value(&key)
                        .ok_or_else(|| Thrown::type_error("Illegal offset type"))?;
                    arr.insert(k, val);
                }
                ArrayElement::Value(v) => {
                    let val = self.walk_expr(v.value)?;
                    arr.push(val);
                }
                ArrayElement::Variadic(v) => {
                    // array spread: merge entries (int keys renumbered).
                    let val = self.walk_expr(v.value)?;
                    if let Value::Array(inner) = deref(&val) {
                        for (k, iv) in inner.iter() {
                            match k {
                                ArrayKey::Int(_) => arr.push(iv.clone()),
                                ArrayKey::Str(_) => arr.insert(k.clone(), iv.clone()),
                            }
                        }
                    }
                }
                ArrayElement::Missing(_) => {}
            }
        }
        Ok(Value::Array(arr))
    }

    fn walk_match(&mut self, m: &'a Match<'a>) -> EvalResult {
        let subject = self.walk_expr(m.expression)?;
        let mut default_expr = None;
        for arm in m.arms.iter() {
            match arm {
                MatchArm::Expression(a) => {
                    for cond in a.conditions.iter() {
                        let c = self.walk_expr(cond)?;
                        if ops::strict_eq(&subject, &c) {
                            return self.walk_expr(a.expression);
                        }
                    }
                }
                MatchArm::Default(a) => default_expr = Some(a.expression),
            }
        }
        match default_expr {
            Some(e) => self.walk_expr(e),
            None => Err(Thrown::error("UnhandledMatchError", "Unhandled match case")),
        }
    }

    fn walk_composite(&mut self, cs: &'a CompositeString<'a>) -> EvalResult {
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
        let mut bytes = Vec::new();
        for part in parts.iter() {
            match part {
                StringPart::Literal(l) => bytes.extend_from_slice(l.value.unwrap_or(l.raw)),
                StringPart::Expression(e) => {
                    let v = self.walk_expr(e)?;
                    bytes.extend_from_slice(&conv::to_string_bytes(&v));
                }
                StringPart::BracedExpression(b) => {
                    let v = self.walk_expr(b.expression)?;
                    bytes.extend_from_slice(&conv::to_string_bytes(&v));
                }
            }
        }
        Ok(Value::str(bytes))
    }

    fn walk_constant(&mut self, id: &'a Identifier<'a>) -> EvalResult {
        let name = identifier_name(id);
        // case-sensitive user/engine constants; keyword literals are elsewhere.
        if let Some(v) = self.constants.get(name) {
            return Ok(v.clone());
        }
        match name {
            b"true" | b"TRUE" | b"True" => Ok(Value::Bool(true)),
            b"false" | b"FALSE" | b"False" => Ok(Value::Bool(false)),
            b"null" | b"NULL" | b"Null" => Ok(Value::Null),
            _ => Err(Thrown::error(
                "Error",
                format!("Undefined constant \"{}\"", String::from_utf8_lossy(name)),
            )),
        }
    }

    fn walk_construct(&mut self, c: &'a Construct<'a>) -> EvalResult {
        match c {
            Construct::Print(p) => {
                let v = self.walk_expr(p.value)?;
                let bytes = conv::to_string_bytes(&v);
                self.write_out(&bytes);
                Ok(Value::Int(1))
            }
            Construct::Exit(e) => {
                let code = self.exit_code_from(e.arguments.as_ref())?;
                Err(Thrown::Exit(code))
            }
            Construct::Die(d) => {
                let code = self.exit_code_from(d.arguments.as_ref())?;
                Err(Thrown::Exit(code))
            }
            Construct::Isset(i) => {
                // isset($x): true if the variable exists and is not null.
                let mut all = true;
                for v in i.values.iter() {
                    let set = match v {
                        Expression::Variable(var) => {
                            matches!(self.read_variable(var), Ok(ref val) if !matches!(deref(val), Value::Null))
                        }
                        other => !matches!(deref(&self.walk_expr(other)?), Value::Null),
                    };
                    all &= set;
                }
                Ok(Value::Bool(all))
            }
            Construct::Empty(e) => {
                let v = self.walk_expr(e.value).unwrap_or(Value::Null);
                Ok(Value::Bool(!conv::to_bool(&v)))
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
        }
    }

    fn exit_code_from(
        &mut self,
        args: Option<&'a mago_syntax::cst::ArgumentList<'a>>,
    ) -> Result<i32, Thrown> {
        let Some(list) = args else { return Ok(0) };
        let Some(first) = list.arguments.iter().next() else {
            return Ok(0);
        };
        let value = match first {
            Argument::Positional(p) => self.walk_expr(p.value)?,
            Argument::Named(n) => self.walk_expr(n.value)?,
        };
        Ok(match deref(&value) {
            Value::Int(n) => (n & 0xFF) as i32,
            Value::Str(s) => {
                self.write_out(s.as_bytes());
                0
            }
            _ => 0,
        })
    }

    fn walk_call(&mut self, call: &'a mago_syntax::cst::Call<'a>) -> EvalResult {
        use mago_syntax::cst::Call;
        let Call::Function(fc) = call else {
            return Err(Thrown::unsupported(
                "method/static call",
                "objects arrive in phase 7",
            ));
        };
        let name = match fc.function {
            Expression::Identifier(id) => identifier_name(id).to_vec(),
            Expression::ConstantAccess(ca) => identifier_name(&ca.name).to_vec(),
            _ => {
                return Err(Thrown::unsupported(
                    "dynamic function call",
                    "callable values arrive with closures",
                ))
            }
        };
        let name_str = String::from_utf8_lossy(&name).into_owned();

        if let Some(reason) = builtins::is_refused(&name_str) {
            return Err(Thrown::unsupported(format!("{name_str}()"), reason));
        }

        // Program execution and OS-stream functions need the VM's resource
        // table; the frozen oracle refuses them (the harness then skips).
        if matches!(
            name_str.to_ascii_lowercase().as_str(),
            "shell_exec"
                | "proc_get_status"
                | "proc_close"
                | "proc_terminate"
                | "stream_isatty"
                | "posix_isatty"
                | "putenv"
        ) {
            return Err(Thrown::unsupported(
                format!("{name_str}()"),
                "process control is not supported by the oracle",
            ));
        }

        // By-reference builtins (sort/array_push/…) mutate their array argument
        // in place; the frozen oracle has no by-ref call path, so it refuses
        // them (the differential harness then skips those fixtures).
        if is_byref_builtin(&name_str) {
            return Err(Thrown::unsupported(
                format!("{name_str}()"),
                "by-reference builtins are not supported by the oracle",
            ));
        }

        // The oracle has no runtime-constant table, so it cannot model
        // define/defined/constant; it refuses them (the harness then skips).
        if matches!(
            name_str.to_ascii_lowercase().as_str(),
            "define" | "defined" | "constant"
        ) {
            return Err(Thrown::unsupported(
                format!("{name_str}()"),
                "runtime constants are not supported by the oracle",
            ));
        }

        // Evaluate positional arguments (named/variadic deferred).
        let mut argv: Vec<Value> = Vec::new();
        for arg in fc.argument_list.arguments.iter() {
            match arg {
                Argument::Positional(p) => {
                    if p.ellipsis.is_some() {
                        if let Value::Array(a) = deref(&self.walk_expr(p.value)?) {
                            argv.extend(a.iter().map(|(_, v)| v.clone()));
                        }
                    } else {
                        argv.push(self.walk_expr(p.value)?);
                    }
                }
                Argument::Named(_) => {
                    return Err(Thrown::unsupported(
                        "named arguments",
                        "arrive with user functions in phase 5",
                    ))
                }
            }
        }

        // User-defined function (hoisted)?
        if let Some(func) = self
            .functions
            .get(name_str.to_ascii_lowercase().as_bytes())
            .copied()
        {
            return self.call_user(func, argv);
        }

        let Some(builtin) = builtins::lookup(&name_str) else {
            return Err(Thrown::error(
                "Error",
                format!("Call to undefined function {name_str}()"),
            ));
        };

        // Arity check.
        if argv.len() < builtin.min_args {
            return Err(Thrown::error(
                "ArgumentCountError",
                format!(
                    "Too few arguments to function {name_str}(), {} passed and at least {} expected",
                    argv.len(),
                    builtin.min_args
                ),
            ));
        }
        if let Some(max) = builtin.max_args {
            if argv.len() > max {
                return Err(Thrown::error(
                    "ArgumentCountError",
                    format!("Too many arguments to function {name_str}()"),
                ));
            }
        }

        (builtin.func)(self, &argv)
    }

    // -- variables / lvalues ----------------------------------------------

    /// Call a user-defined function: bind params into a fresh scope, run the
    /// body, return its value (by-value params + constant defaults + variadic).
    fn call_user(&mut self, func: &'a Function<'a>, argv: Vec<Value>) -> EvalResult {
        let mut scope: HashMap<Vec<u8>, Value> = HashMap::new();
        for (i, p) in func.parameter_list.parameters.iter().enumerate() {
            if p.ampersand.is_some() {
                return Err(Thrown::unsupported(
                    "by-reference parameter",
                    "reference parameters arrive in the next phase",
                ));
            }
            let var = strip_dollar(p.variable.name).to_vec();
            if p.ellipsis.is_some() {
                let mut arr = PhpArray::new();
                if i < argv.len() {
                    for a in &argv[i..] {
                        arr.push(a.clone());
                    }
                }
                scope.insert(var, Value::Array(arr));
                break;
            } else if i < argv.len() {
                scope.insert(var, argv[i].clone());
            } else if let Some(dv) = &p.default_value {
                let v = self.walk_expr(dv.value)?;
                scope.insert(var, v);
            } else {
                return Err(Thrown::error(
                    "ArgumentCountError",
                    format!(
                        "Too few arguments to function {}()",
                        String::from_utf8_lossy(func.name.value)
                    ),
                ));
            }
        }

        // Swap in the call-local scope; functions have their own variable scope.
        let saved = std::mem::replace(&mut self.vars, scope);
        let mut result = Ok(Value::Null);
        for s in func.body.statements.iter() {
            match self.exec_stmt(s) {
                Ok(Flow::Normal) => {}
                Ok(Flow::Return(v)) => {
                    result = Ok(v);
                    break;
                }
                Ok(_) => {} // stray break/continue at function scope: ignore
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        self.vars = saved;
        result
    }

    /// Read `$a[$i]`: array element (null if absent), or a single byte from a
    /// string (negative offsets count from the end; out of range → "").
    fn walk_array_access(&mut self, aa: &'a ArrayAccess<'a>) -> EvalResult {
        let base = self.walk_expr(aa.array)?;
        let idx = self.walk_expr(aa.index)?;
        match deref(&base) {
            Value::Array(a) => {
                let key = ArrayKey::from_value(&idx)
                    .ok_or_else(|| Thrown::type_error("Illegal offset type"))?;
                Ok(a.get(&key).map(deref).unwrap_or(Value::Null))
            }
            Value::Str(s) => {
                let bytes = s.as_bytes();
                let len = bytes.len() as i64;
                let i = conv::to_int(&idx);
                let actual = if i < 0 { len + i } else { i };
                if actual >= 0 && actual < len {
                    Ok(Value::str(vec![bytes[actual as usize]]))
                } else {
                    Ok(Value::str(Vec::new()))
                }
            }
            _ => Ok(Value::Null),
        }
    }

    /// Resolve an lvalue (`$x`, `$a[$i]`, `$a[$i][$j]`, `$a[]`) into its root
    /// variable name plus a sequence of descent steps. Index expressions are
    /// evaluated here (left to right, outer array first).
    fn collect_path(
        &mut self,
        target: &'a Expression<'a>,
    ) -> Result<(Vec<u8>, Vec<PathStep>), Thrown> {
        match target {
            Expression::Variable(Variable::Direct(d)) => {
                Ok((strip_dollar(d.name).to_vec(), Vec::new()))
            }
            Expression::ArrayAccess(aa) => {
                let (root, mut steps) = self.collect_path(aa.array)?;
                let idx = self.walk_expr(aa.index)?;
                let key = ArrayKey::from_value(&idx)
                    .ok_or_else(|| Thrown::type_error("Illegal offset type"))?;
                steps.push(PathStep::Key(key));
                Ok((root, steps))
            }
            Expression::ArrayAppend(aa) => {
                let (root, mut steps) = self.collect_path(aa.array)?;
                steps.push(PathStep::Append);
                Ok((root, steps))
            }
            _ => Err(Thrown::unsupported(
                "assignment target",
                "only variables and array elements are supported",
            )),
        }
    }

    /// Read the value at a resolved path, or `None` if any step is missing /
    /// not an array. Never autovivifies.
    fn read_opt(&self, root: &[u8], steps: &[PathStep]) -> Option<Value> {
        let mut cur = deref(self.vars.get(root)?);
        for step in steps {
            let Value::Array(a) = &cur else { return None };
            let PathStep::Key(key) = step else {
                return None;
            };
            cur = deref(a.get(key)?);
        }
        Some(cur)
    }

    fn read_variable(&mut self, var: &'a Variable<'a>) -> EvalResult {
        match var {
            Variable::Direct(d) => {
                let key = strip_dollar(d.name);
                Ok(self.vars.get(key).cloned().unwrap_or(Value::Null))
            }
            _ => Err(Thrown::unsupported(
                "indirect/nested variable",
                "variable-variables are refused / deferred",
            )),
        }
    }

    fn walk_assignment(&mut self, a: &'a mago_syntax::cst::Assignment<'a>) -> EvalResult {
        match a.lhs {
            Expression::Variable(Variable::Direct(d)) => {
                let key = strip_dollar(d.name).to_vec();
                let new = match &a.operator {
                    AssignmentOperator::Assign(_) => self.walk_expr(a.rhs)?,
                    AssignmentOperator::Coalesce(_) => {
                        let cur = self.vars.get(&key).cloned().unwrap_or(Value::Null);
                        if !matches!(deref(&cur), Value::Null) {
                            return Ok(cur);
                        }
                        self.walk_expr(a.rhs)?
                    }
                    op => {
                        let cur = self.vars.get(&key).cloned().unwrap_or(Value::Null);
                        let rhs = self.walk_expr(a.rhs)?;
                        apply_compound_op(op, &cur, &rhs)?
                    }
                };
                self.vars.insert(key, new.clone());
                Ok(new)
            }
            Expression::ArrayAccess(_) | Expression::ArrayAppend(_) => self.assign_to_path(a),
            _ => Err(Thrown::unsupported(
                "assignment target",
                "property targets arrive with OOP",
            )),
        }
    }

    /// Assign to an array-element lvalue (`$a[$i] = v`, `$a[] = v`, compounds).
    fn assign_to_path(&mut self, a: &'a mago_syntax::cst::Assignment<'a>) -> EvalResult {
        let (root, steps) = self.collect_path(a.lhs)?;
        let new = match &a.operator {
            AssignmentOperator::Assign(_) => self.walk_expr(a.rhs)?,
            AssignmentOperator::Coalesce(_) => {
                if let Some(v) = self.read_opt(&root, &steps) {
                    if !matches!(v, Value::Null) {
                        return Ok(v);
                    }
                }
                self.walk_expr(a.rhs)?
            }
            op => {
                let cur = self.read_opt(&root, &steps).unwrap_or(Value::Null);
                let rhs = self.walk_expr(a.rhs)?;
                apply_compound_op(op, &cur, &rhs)?
            }
        };
        let slot = self.vars.entry(root).or_insert(Value::Null);
        assign_path(slot, &steps, new.clone())?;
        Ok(new)
    }

    fn pre_inc_dec(&mut self, target: &'a Expression<'a>, delta: i64) -> EvalResult {
        match target {
            Expression::Variable(Variable::Direct(d)) => {
                let key = strip_dollar(d.name).to_vec();
                let cur = self.vars.get(&key).cloned().unwrap_or(Value::Null);
                let new = inc_dec_value(&cur, delta);
                self.vars.insert(key, new.clone());
                Ok(new)
            }
            Expression::ArrayAccess(_) => {
                let (root, steps) = self.collect_path(target)?;
                let cur = self.read_opt(&root, &steps).unwrap_or(Value::Null);
                let new = inc_dec_value(&cur, delta);
                let slot = self.vars.entry(root).or_insert(Value::Null);
                assign_path(slot, &steps, new.clone())?;
                Ok(new)
            }
            _ => Err(Thrown::unsupported("++/-- target", "unsupported target")),
        }
    }

    fn post_inc_dec(&mut self, target: &'a Expression<'a>, delta: i64) -> EvalResult {
        match target {
            Expression::Variable(Variable::Direct(d)) => {
                let key = strip_dollar(d.name).to_vec();
                let cur = self.vars.get(&key).cloned().unwrap_or(Value::Null);
                let new = inc_dec_value(&cur, delta);
                self.vars.insert(key, new);
                Ok(cur)
            }
            Expression::ArrayAccess(_) => {
                let (root, steps) = self.collect_path(target)?;
                let cur = self.read_opt(&root, &steps).unwrap_or(Value::Null);
                let new = inc_dec_value(&cur, delta);
                let slot = self.vars.entry(root).or_insert(Value::Null);
                assign_path(slot, &steps, new)?;
                Ok(cur)
            }
            _ => Err(Thrown::unsupported("++/-- target", "unsupported target")),
        }
    }
}

// ---------------------------------------------------------------------------
// free helpers
// ---------------------------------------------------------------------------

/// What a loop should do after its body returns a given `Flow`.
enum LoopDecision {
    Iterate,
    Stop,
    Propagate(Flow),
}

fn loop_decision(flow: Flow) -> LoopDecision {
    match flow {
        Flow::Normal => LoopDecision::Iterate,
        Flow::Continue(1) => LoopDecision::Iterate,
        Flow::Continue(n) => LoopDecision::Propagate(Flow::Continue(n - 1)),
        Flow::Break(1) => LoopDecision::Stop,
        Flow::Break(n) => LoopDecision::Propagate(Flow::Break(n - 1)),
        Flow::Return(v) => LoopDecision::Propagate(Flow::Return(v)),
    }
}

/// Resolve a `break`/`continue` level (default 1; must be a positive literal).
fn flow_level(level: Option<&Expression>) -> Result<u32, Thrown> {
    match level {
        None => Ok(1),
        Some(Expression::Literal(Literal::Integer(i))) => Ok(i.value.unwrap_or(1).max(1) as u32),
        Some(_) => Err(Thrown::error(
            "Error",
            "'break'/'continue' operator accepts only a constant integer level",
        )),
    }
}

/// One descent step of an array lvalue path.
enum PathStep {
    Key(ArrayKey),
    Append,
}

/// Write `value` at `steps` under `slot`, autovivifying intermediate arrays
/// (COW via `make_mut`). `null`/`""` become a fresh array; other scalars error.
fn assign_path(slot: &mut Value, steps: &[PathStep], value: Value) -> Result<(), Thrown> {
    if steps.is_empty() {
        *slot = value;
        return Ok(());
    }
    if matches!(slot, Value::Null) || matches!(slot, Value::Str(s) if s.is_empty()) {
        *slot = Value::Array(PhpArray::new());
    }
    let Value::Array(arr) = slot else {
        return Err(Thrown::error(
            "Error",
            "Cannot use a scalar value as an array",
        ));
    };
    let data = arr.make_mut();
    let key = match &steps[0] {
        PathStep::Key(k) => k.clone(),
        PathStep::Append => ArrayKey::Int(data.next_int.unwrap_or(0)),
    };
    // Maintain the next-free-integer high-water mark for integer keys.
    if let ArrayKey::Int(n) = &key {
        let n = *n;
        data.next_int = Some(match data.next_int {
            None => n + 1,
            Some(x) => x.max(n + 1),
        });
    }
    if steps.len() == 1 {
        data.entries.insert(key, value);
    } else {
        let child = data.entries.entry(key).or_insert(Value::Null);
        assign_path(child, &steps[1..], value)?;
    }
    Ok(())
}

/// Remove the leaf element at `steps` (order-preserving; never lowers next_int).
fn unset_path(slot: &mut Value, steps: &[PathStep]) {
    let Value::Array(arr) = slot else { return };
    let PathStep::Key(key) = &steps[0] else {
        return;
    };
    if steps.len() == 1 {
        arr.make_mut().entries.shift_remove(key);
    } else if let Some(child) = arr.make_mut().entries.get_mut(key) {
        unset_path(child, &steps[1..]);
    }
}

/// Apply a compound-assignment operator (`+=`, `.=`, `&=`, …) — not `=`/`??=`.
fn apply_compound_op(op: &AssignmentOperator, cur: &Value, rhs: &Value) -> Result<Value, Thrown> {
    Ok(match op {
        AssignmentOperator::Addition(_) => ops::add(cur, rhs)?,
        AssignmentOperator::Subtraction(_) => ops::sub(cur, rhs)?,
        AssignmentOperator::Multiplication(_) => ops::mul(cur, rhs)?,
        AssignmentOperator::Division(_) => ops::div(cur, rhs)?,
        AssignmentOperator::Modulo(_) => ops::modulo(cur, rhs)?,
        AssignmentOperator::Exponentiation(_) => ops::pow(cur, rhs)?,
        AssignmentOperator::Concat(_) => ops::concat(cur, rhs),
        AssignmentOperator::BitwiseAnd(_) => ops::bitwise(cur, rhs, ops::BitOp::And),
        AssignmentOperator::BitwiseOr(_) => ops::bitwise(cur, rhs, ops::BitOp::Or),
        AssignmentOperator::BitwiseXor(_) => ops::bitwise(cur, rhs, ops::BitOp::Xor),
        AssignmentOperator::LeftShift(_) => ops::bitwise(cur, rhs, ops::BitOp::Shl),
        AssignmentOperator::RightShift(_) => ops::bitwise(cur, rhs, ops::BitOp::Shr),
        AssignmentOperator::Assign(_) | AssignmentOperator::Coalesce(_) => unreachable!(),
    })
}

/// Builtins with by-reference parameters (sort/stack ops, `preg_*` out-params).
/// The frozen oracle has no by-ref call path, so it refuses them.
fn is_byref_builtin(name: &str) -> bool {
    !rhp_bytecode::builtin_ref_params(&name.to_ascii_lowercase()).is_empty()
}

fn strip_dollar(name: &[u8]) -> &[u8] {
    name.strip_prefix(b"$").unwrap_or(name)
}

fn identifier_name<'a>(id: &'a Identifier<'a>) -> &'a [u8] {
    // Take the final `\`-separated segment (namespace resolution is minimal).
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

fn walk_literal(lit: &Literal) -> Value {
    match lit {
        Literal::Integer(i) => match i.value {
            Some(u) if u <= i64::MAX as u64 => Value::Int(u as i64),
            Some(u) => Value::Float(u as f64), // overflows i64 => float
            None => {
                // too large even for u64, or unparsed: fall back to float.
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

fn stmt_name(s: &Statement) -> &'static str {
    match s {
        Statement::Namespace(_) => "namespace",
        Statement::Use(_) => "use",
        Statement::Class(_) => "class",
        Statement::Interface(_) => "interface",
        Statement::Trait(_) => "trait",
        Statement::Enum(_) => "enum",
        Statement::Constant(_) => "const",
        Statement::Function(_) => "function",
        Statement::Declare(_) => "declare",
        Statement::Goto(_) => "goto",
        Statement::Try(_) => "try",
        Statement::Foreach(_) => "foreach",
        Statement::For(_) => "for",
        Statement::While(_) => "while",
        Statement::DoWhile(_) => "do-while",
        Statement::Switch(_) => "switch",
        Statement::If(_) => "if",
        Statement::Return(_) => "return",
        Statement::Global(_) => "global",
        Statement::Static(_) => "static",
        Statement::Unset(_) => "unset",
        Statement::Break(_) => "break",
        Statement::Continue(_) => "continue",
        Statement::Label(_) => "label",
        _ => "this",
    }
}

fn expr_name(e: &Expression) -> &'static str {
    match e {
        Expression::CompositeString(_) => "interpolated-string",
        Expression::Match(_) => "match",
        Expression::Closure(_) => "closure",
        Expression::ArrowFunction(_) => "arrow-function",
        Expression::List(_) => "list()",
        Expression::ArrayAccess(_) => "array-access",
        Expression::ArrayAppend(_) => "array-append",
        Expression::Throw(_) => "throw",
        Expression::Clone(_) => "clone",
        Expression::Instantiation(_) => "new",
        Expression::Access(_) => "member-access",
        Expression::Yield(_) => "yield",
        Expression::MagicConstant(_) => "magic-constant",
        Expression::Pipe(_) => "pipe",
        _ => "this",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_str(src: &str) -> String {
        let arena = rhp_frontend::Arena::new();
        let parsed = arena.parse(b"test.php", src.as_bytes());
        let mut interp = Interp::new();
        interp.run(parsed.program()).expect("run ok");
        String::from_utf8_lossy(interp.output()).into_owned()
    }

    // --- array access + foreach (ground truth from php 8.5.8) ---

    #[test]
    fn array_read() {
        assert_eq!(
            run_str(
                r#"<?php $a=[10,20,30];$m=["x"=>1,"y"=>2];$n=[[1,2],[3,4]];
                   echo $a[0],$a[2],$m["x"],$n[0][1],$n[1][0],($a[99]??"Z");"#
            ),
            "1030123Z"
        );
    }

    #[test]
    fn array_write() {
        assert_eq!(
            run_str(r#"<?php $a=[];$a[0]="z";$a[]="one";$a["k"]="v";$a[0]="Z";var_dump($a);"#),
            "array(3) {\n  [0]=>\n  string(1) \"Z\"\n  [1]=>\n  string(3) \"one\"\n  [\"k\"]=>\n  string(1) \"v\"\n}\n"
        );
    }

    #[test]
    fn nested_autovivify() {
        assert_eq!(
            run_str(r#"<?php $g=[];$g[2][3]="deep";var_dump($g);"#),
            "array(1) {\n  [2]=>\n  array(1) {\n    [3]=>\n    string(4) \"deep\"\n  }\n}\n"
        );
    }

    #[test]
    fn element_compound_and_incdec() {
        assert_eq!(
            run_str(
                r#"<?php $a=[10,20,30];$a[0]+=5;$a[1]*=2;$a[2]++;
                   $c=[];$c["x"]=0;$c["x"]++;$c["x"]++;
                   echo $a[0],",",$a[1],",",$a[2],",",$c["x"];"#
            ),
            "15,40,31,2"
        );
    }

    #[test]
    fn isset_empty_unset_dims() {
        assert_eq!(
            run_str(
                r#"<?php $a=["x"=>1,"y"=>null,"z"=>0];
                   var_dump(isset($a["x"]));var_dump(isset($a["y"]));var_dump(isset($a["q"]));
                   var_dump(empty($a["z"]));var_dump(empty($a["x"]));"#
            ),
            "bool(true)\nbool(false)\nbool(false)\nbool(true)\nbool(false)\n"
        );
        assert_eq!(
            run_str(r#"<?php $a=["x"=>1,"y"=>null,"z"=>0];unset($a["x"]);var_dump($a);"#),
            "array(2) {\n  [\"y\"]=>\n  NULL\n  [\"z\"]=>\n  int(0)\n}\n"
        );
    }

    #[test]
    fn foreach_value_and_kv() {
        assert_eq!(
            run_str(
                r#"<?php $a=[10,20,30];$s=0;foreach($a as $v){$s+=$v;}
                   $m=["a"=>1,"b"=>2];$o="";foreach($m as $k=>$v){$o.="$k$v";}
                   echo $s,"|",$o;"#
            ),
            "60|a1b2"
        );
    }

    #[test]
    fn foreach_nested_and_break() {
        assert_eq!(
            run_str(
                r#"<?php $n=[[1,2],[3,4]];$o="";
                   foreach($n as $row){foreach($row as $x){if($x==3)break 2;$o.=$x;}}
                   echo $o;"#
            ),
            "12"
        );
    }

    #[test]
    fn string_index() {
        assert_eq!(
            run_str(r#"<?php $s="hello";echo $s[0],$s[4],$s[-1],"|",strlen($s);"#),
            "hoo|5"
        );
    }

    #[test]
    fn append_highwater_after_unset() {
        assert_eq!(
            run_str(
                r#"<?php $a=[0,1,2];unset($a[1]);$a[]=9;$o="";
                   foreach($a as $k=>$v){$o.="$k:$v ";}echo $o;"#
            ),
            "0:0 2:2 3:9 "
        );
    }

    #[test]
    fn echo_and_inline() {
        assert_eq!(run_str("<?php echo 'hi';"), "hi");
        assert_eq!(run_str("A<?php echo 1+2; ?>B"), "A3B");
        assert_eq!(run_str("<?php echo 1, 2, 3;"), "123");
    }

    #[test]
    fn arithmetic_and_concat() {
        assert_eq!(run_str("<?php echo 7 % 3;"), "1");
        assert_eq!(run_str("<?php echo 'r=' . (0.1 + 0.2);"), "r=0.3");
        assert_eq!(run_str("<?php echo 2 ** 10;"), "1024");
    }

    #[test]
    fn variables() {
        assert_eq!(run_str("<?php $x = 5; $x += 3; echo $x;"), "8");
        assert_eq!(run_str("<?php $a = 1; echo $a++; echo $a;"), "12");
    }
}
