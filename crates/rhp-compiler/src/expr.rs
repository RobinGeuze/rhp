//! Expression lowering: each returns the register holding the result.

use crate::parse::cst::{
    Access, Argument, ArrayElement, BinaryOperator, ClassLikeMemberSelector, CompositeString,
    Construct, Expression, Identifier, Literal, MagicConstant, Node, Statement, StringPart,
    UnaryPrefixOperator, Variable,
};
use rhp_bytecode::{ClassRef, CmpKind, IncludeKind, Instr, Reg};
use rhp_values::{Thrown, Value};

use crate::assign::{as_reference, check_globals_write, is_globals_var};
use crate::builder::{ProtoBuilder, ScopeKind};
use crate::fold::{literal_array, literal_value, php_swaps_operands};
use crate::names::{identifier_name, NameKind};
use crate::oop::Sel;
use crate::util::{dirname, strip_dollar};

impl ProtoBuilder<'_> {
    pub(crate) fn compile_expr(&mut self, expr: &Expression) -> Result<Reg, Thrown> {
        use mago_span::HasSpan;
        self.compile_expr_inner(expr)
            .map_err(|e| self.cx.locate(e, expr.span().start.offset))
    }

    fn compile_expr_inner(&mut self, expr: &Expression) -> Result<Reg, Thrown> {
        match expr {
            Expression::Literal(lit) => self.compile_literal(lit),
            Expression::Parenthesized(p) => self.compile_expr(p.expression),
            Expression::Variable(Variable::Direct(d)) if strip_dollar(d.name) == b"GLOBALS" => {
                let dst = self.alloc_temp();
                self.emit(Instr::FetchGlobals { dst });
                Ok(dst)
            }
            Expression::Variable(Variable::Direct(d)) => Ok(self.cv(strip_dollar(d.name))),
            Expression::Variable(v) => self.compile_variable(v),
            Expression::Binary(b) => self.compile_binary(b),
            Expression::UnaryPrefix(u) => self.compile_unary_prefix(u),
            Expression::UnaryPostfix(u) => self.compile_unary_postfix(u),
            Expression::Assignment(a) => self.compile_assignment(a),
            Expression::Conditional(c) => self.compile_conditional(c),
            Expression::Array(arr) => self.compile_array(arr.elements.iter()),
            Expression::LegacyArray(arr) => self.compile_array(arr.elements.iter()),
            Expression::ArrayAccess(aa) if is_globals_var(aa.array) => {
                let name = self.compile_expr(aa.index)?;
                let dst = self.alloc_temp();
                self.emit(Instr::FetchGlobal { dst, name });
                Ok(dst)
            }
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
            Expression::Instantiation(inst) => {
                use mago_span::HasSpan;
                self.call_lines
                    .push(self.cx.line_of(inst.span().start.offset));
                let r = self.compile_new(inst);
                self.call_lines.pop();
                r
            }
            Expression::AnonymousClass(ac) => {
                use mago_span::HasSpan;
                self.call_lines
                    .push(self.cx.line_of(ac.span().start.offset));
                let r = self.compile_new_anon(ac);
                self.call_lines.pop();
                r
            }
            Expression::Access(acc) => self.compile_access(acc),
            Expression::Clone(cl) => {
                let src = self.compile_expr(cl.object)?;
                let dst = self.alloc_temp();
                self.emit(Instr::CloneObj { dst, src });
                Ok(dst)
            }
            Expression::Throw(t) => {
                let src = self.compile_expr(t.exception)?;
                self.emit(Instr::Throw { src });
                // `throw` never returns; the dst is unreachable.
                let dst = self.alloc_temp();
                Ok(dst)
            }
            Expression::Construct(c) => self.compile_construct(c),
            Expression::Call(call) => {
                use crate::parse::cst::Call;
                use mago_span::HasSpan;
                let at = match call {
                    Call::Function(fc) => fc.function.span(),
                    Call::Method(mc) => mc.method.span(),
                    Call::NullSafeMethod(mc) => mc.method.span(),
                    Call::StaticMethod(smc) => smc.method.span(),
                };
                self.call_lines.push(self.cx.line_of(at.start.offset));
                let r = self.compile_call(call);
                self.call_lines.pop();
                r
            }
            Expression::ArrowFunction(af) => self.compile_arrow(af),
            Expression::Closure(cl) => self.compile_closure(cl),
            Expression::PartialApplication(pa) => self.compile_fcc(pa),
            Expression::Yield(y) => self.compile_yield(y),
            Expression::MagicConstant(mc) => self.compile_magic_const(mc),
            other => Err(Thrown::unsupported(
                format!("{} expression", expr_kind(other)),
                "not implemented",
            )),
        }
    }

    /// Lower a magic constant (`__DIR__`, `__FILE__`, `__LINE__`, `__FUNCTION__`,
    /// `__CLASS__`, `__METHOD__`, `__NAMESPACE__`) — all resolved at compile time.
    fn compile_magic_const(&mut self, mc: &MagicConstant) -> Result<Reg, Thrown> {
        use mago_span::HasSpan;
        let dst = self.alloc_temp();
        if let MagicConstant::Line(_) = mc {
            let n = self.cx.line_of(mc.span().start.offset) as i64;
            self.emit(Instr::LoadInt { dst, n });
            return Ok(dst);
        }
        if matches!(mc, MagicConstant::Class(_))
            && self.class_name.is_some()
            && self.cx.in_trait.get()
        {
            // Inside a trait, `__CLASS__` is the class using it (php resolves
            // it at runtime, like `self::class`).
            let name = self.intern_name(b"class");
            self.emit(Instr::FetchClassConst {
                dst,
                class: ClassRef::SelfClass,
                name,
            });
            return Ok(dst);
        }
        if let MagicConstant::Property(_) = mc {
            return Err(Thrown::unsupported(
                "__PROPERTY__",
                "only valid in property hooks",
            ));
        }
        let s: Vec<u8> = match mc {
            MagicConstant::File(_) => self.cx.file_name(),
            MagicConstant::Directory(_) => dirname(self.cx.builtins, &self.cx.file_name()),
            MagicConstant::Function(_) => self.magic_function_name(),
            MagicConstant::Method(_) => self.magic_method_name(),
            MagicConstant::Class(_) | MagicConstant::Trait(_) => self
                .class_name
                .as_ref()
                .map(|c| c.as_bytes().to_vec())
                .unwrap_or_default(),
            MagicConstant::Namespace(_) => self.cx.cur_ns(),
            MagicConstant::Line(_) | MagicConstant::Property(_) => unreachable!(),
        };
        let k = self.intern_const(Value::str(s));
        self.emit(Instr::LoadConst { dst, k });
        Ok(dst)
    }

    /// `__FUNCTION__`: "" at top level, the function/method name, or a closure's
    /// generated `{closure:…}` name.
    fn magic_function_name(&self) -> Vec<u8> {
        match self.scope_kind {
            ScopeKind::Main | ScopeKind::Init => Vec::new(),
            ScopeKind::Function | ScopeKind::Method => self.name.as_bytes().to_vec(),
            ScopeKind::Closure => self
                .magic_fn_name
                .as_ref()
                .map(|n| n.as_bytes().to_vec())
                .unwrap_or_else(|| b"{closure}".to_vec()),
        }
    }

    /// `__METHOD__`: "Class::method" in a method, the function name in a free
    /// function, the closure name in a closure, "" at top level.
    fn magic_method_name(&self) -> Vec<u8> {
        match self.scope_kind {
            ScopeKind::Method => {
                let mut v = self
                    .class_name
                    .as_ref()
                    .map(|c| c.as_bytes().to_vec())
                    .unwrap_or_default();
                v.extend_from_slice(b"::");
                v.extend_from_slice(self.name.as_bytes());
                v
            }
            ScopeKind::Function | ScopeKind::Closure => self.magic_function_name(),
            ScopeKind::Main | ScopeKind::Init => Vec::new(),
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

    fn compile_binary(&mut self, b: &crate::parse::cst::Binary) -> Result<Reg, Thrown> {
        use BinaryOperator as Op;
        // Short-circuit operators lower to branches.
        match &b.operator {
            Op::And(_) | Op::LowAnd(_) => return self.compile_logical(b, true),
            Op::Or(_) | Op::LowOr(_) => return self.compile_logical(b, false),
            Op::NullCoalesce(_) => return self.compile_coalesce(b),
            Op::Instanceof(_) => {
                let val = self.compile_expr(b.lhs)?;
                let class = self.class_ref(b.rhs)?;
                let dst = self.alloc_temp();
                self.emit(Instr::InstanceOf { dst, val, class });
                return Ok(dst);
            }
            _ => {}
        }

        let a = self.compile_expr(b.lhs)?;
        let r = self.compile_expr(b.rhs)?;
        // zend_vm's SPEC_RULE_COMMUTATIVE: php swaps the operands of these
        // opcodes when op1's kind is lower than op2's, which decides whose
        // object handler runs first.
        let commutative = matches!(
            &b.operator,
            Op::Multiplication(_)
                | Op::BitwiseAnd(_)
                | Op::BitwiseOr(_)
                | Op::BitwiseXor(_)
                | Op::Equal(_)
                | Op::NotEqual(_)
                | Op::AngledNotEqual(_)
                | Op::Identical(_)
                | Op::NotIdentical(_)
        );
        let (a, r) = if commutative && php_swaps_operands(self.cx, b.lhs, b.rhs) {
            (r, a)
        } else {
            (a, r)
        };
        let dst = self.alloc_temp();
        let cmp = |k| Instr::Cmp {
            dst,
            a,
            b: r,
            kind: k,
        };
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
                Instr::Cmp {
                    dst,
                    a,
                    b: r,
                    kind: CmpKind::Ne,
                }
            }
            Op::And(_)
            | Op::Or(_)
            | Op::LowAnd(_)
            | Op::LowOr(_)
            | Op::NullCoalesce(_)
            | Op::Instanceof(_) => {
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
        b: &crate::parse::cst::Binary,
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
            Instr::JmpFalse {
                cond: a,
                target: short,
            }
        } else {
            Instr::JmpTrue {
                cond: a,
                target: short,
            }
        };
        self.code[jend] = Instr::Jmp { target: end };
        Ok(dst)
    }

    fn compile_coalesce(&mut self, b: &crate::parse::cst::Binary) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        let a = self.compile_quiet(b.lhs)?;
        self.emit(Instr::Mov { dst, src: a });
        let jnn = self.code.len();
        self.emit(Instr::Nop); // JmpNotNull dst -> end
        let r = self.compile_expr(b.rhs)?;
        self.emit(Instr::Mov { dst, src: r });
        let end = self.code.len() as u32;
        self.code[jnn] = Instr::JmpNotNull {
            src: dst,
            target: end,
        };
        Ok(dst)
    }

    fn compile_unary_prefix(&mut self, u: &crate::parse::cst::UnaryPrefix) -> Result<Reg, Thrown> {
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
            Op::Reference(_) => return Err(Thrown::unsupported("& reference", "not implemented")),
            Op::ErrorControl(_) => {
                self.emit(Instr::BeginSilence);
                let a = self.compile_expr(u.operand)?;
                let dst = self.alloc_temp();
                self.emit(Instr::Mov { dst, src: a });
                self.emit(Instr::EndSilence);
                return Ok(dst);
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
            Op::IntCast(..) | Op::IntegerCast(..) => Instr::CastInt { dst, a },
            Op::FloatCast(..) | Op::DoubleCast(..) => Instr::CastFloat { dst, a },
            Op::StringCast(..) | Op::BinaryCast(..) => Instr::CastString { dst, a },
            Op::BoolCast(..) | Op::BooleanCast(..) => Instr::CastBool { dst, a },
            Op::ArrayCast(..) => Instr::CastArray { dst, a },
            Op::ObjectCast(..) => Instr::CastObject { dst, a },
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
        u: &crate::parse::cst::UnaryPostfix,
    ) -> Result<Reg, Thrown> {
        use crate::parse::cst::UnaryPostfixOperator as Op;
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
        check_globals_write(target)?;
        match target {
            Expression::Variable(Variable::Direct(d)) => {
                let cv = self.cv(strip_dollar(d.name));
                let dst = self.alloc_temp();
                self.emit(Instr::IncDec {
                    dst,
                    cv,
                    pre,
                    delta,
                });
                Ok(dst)
            }
            Expression::Parenthesized(p) => self.compile_inc_dec(p.expression, pre, delta),
            Expression::ArrayAccess(_)
            | Expression::Variable(_)
            | Expression::Access(Access::Property(_))
            | Expression::Access(Access::StaticProperty(_)) => {
                let mut steps = Vec::new();
                let root = self.collect_lvalue_steps(target, &mut steps)?;
                self.emit_cursor(root, &steps);
                let dst = self.alloc_temp();
                self.emit(Instr::IncDecLval { dst, pre, delta });
                Ok(dst)
            }
            _ => Err(Thrown::unsupported(
                "++/-- target",
                "variables, array elements, properties",
            )),
        }
    }

    fn compile_conditional(&mut self, c: &crate::parse::cst::Conditional) -> Result<Reg, Thrown> {
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
        self.code[jf] = Instr::JmpFalse {
            cond,
            target: else_label,
        };
        self.code[jend] = Instr::Jmp { target: end };
        Ok(dst)
    }

    fn compile_array<'a>(
        &mut self,
        els: impl Iterator<Item = &'a ArrayElement<'a>> + Clone,
    ) -> Result<Reg, Thrown> {
        // An array of plain literals (CallMap-style data files) is one
        // constant: no per-element temporaries, no runtime building.
        if let Some(v) = literal_array(els.clone()) {
            let k = self.intern_const(v);
            let dst = self.alloc_temp();
            self.emit(Instr::LoadConst { dst, k });
            return Ok(dst);
        }
        let dst = self.alloc_temp();
        self.emit(Instr::NewArray { dst, cap: 0 });
        for el in els {
            match el {
                ArrayElement::KeyValue(kv) => {
                    let key = self.compile_expr(kv.key)?;
                    if let Some(inner) = as_reference(kv.value) {
                        let val = self.compile_ref_source(inner)?;
                        self.emit(Instr::ArrSetRef { arr: dst, key, val });
                    } else {
                        let val = self.compile_expr(kv.value)?;
                        self.emit(Instr::ArrSet { arr: dst, key, val });
                    }
                }
                ArrayElement::Value(v) => {
                    if let Some(inner) = as_reference(v.value) {
                        let val = self.compile_ref_source(inner)?;
                        self.emit(Instr::ArrPushRef { arr: dst, val });
                    } else {
                        let val = self.compile_expr(v.value)?;
                        self.emit(Instr::ArrPush { arr: dst, val });
                    }
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
                // Simple syntax `"$a[key]"`: an unquoted key is a string
                // literal, not a constant.
                StringPart::Expression(Expression::ArrayAccess(aa))
                    if matches!(aa.index, Expression::Identifier(_)) =>
                {
                    let Expression::Identifier(id) = aa.index else {
                        unreachable!()
                    };
                    let k = self.intern_const(Value::str(id.value().to_vec()));
                    let key = self.alloc_temp();
                    self.emit(Instr::LoadConst { dst: key, k });
                    let r = self.alloc_temp();
                    if is_globals_var(aa.array) {
                        self.emit(Instr::FetchGlobal { dst: r, name: key });
                    } else {
                        let arr = self.compile_expr(aa.array)?;
                        self.emit(Instr::DimFetch { dst: r, arr, key });
                    }
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
                let (primary, fallback) = self.cx.resolve_name(id, NameKind::Const);
                let n = self.intern_name(&primary);
                let fb = self.intern_fallback(&fallback);
                self.emit(Instr::FetchConst {
                    dst,
                    name: n,
                    fallback: fb,
                });
            }
        }
        Ok(dst)
    }

    /// Lower `include`/`require`/`*_once $path` to an `Include` opcode; the VM
    /// resolves, compiles and runs the file at runtime, leaving its return in `dst`.
    fn compile_include(
        &mut self,
        path_expr: &Expression,
        kind: IncludeKind,
    ) -> Result<Reg, Thrown> {
        let path = self.compile_expr(path_expr)?;
        let dst = self.alloc_temp();
        self.emit(Instr::Include { dst, path, kind });
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
            Construct::Eval(e) => {
                let src = self.compile_expr(e.value)?;
                let dst = self.alloc_temp();
                self.emit(Instr::Eval { dst, src });
                Ok(dst)
            }
            Construct::Include(i) => self.compile_include(i.value, IncludeKind::Include),
            Construct::IncludeOnce(i) => self.compile_include(i.value, IncludeKind::IncludeOnce),
            Construct::Require(i) => self.compile_include(i.value, IncludeKind::Require),
            Construct::RequireOnce(i) => self.compile_include(i.value, IncludeKind::RequireOnce),
            Construct::Isset(i) => {
                // AND over each operand: true iff every value is non-null.
                let dst = self.alloc_temp();
                let mut jumps = Vec::new();
                for v in i.values.iter() {
                    // `isset($o->prop)` must consult `__isset`, not `__get`.
                    // A nullsafe `$o?->prop` too: a null object makes IssetProp false.
                    let prop = match v {
                        Expression::Access(Access::Property(pa)) => Some((pa.object, &pa.property)),
                        Expression::Access(Access::NullSafeProperty(pa)) => {
                            Some((pa.object, &pa.property))
                        }
                        _ => None,
                    };
                    if let Some((object, property)) = prop {
                        let obj = self.compile_quiet(object)?;
                        let r = self.alloc_temp();
                        match self.compile_selector(property)? {
                            Sel::Static(name) => self.emit(Instr::IssetProp { dst: r, obj, name }),
                            Sel::Dyn(name) => self.emit(Instr::IssetPropDyn { dst: r, obj, name }),
                        }
                        // IssetProp yields a bool; false short-circuits.
                        let pos = self.code.len();
                        self.emit(Instr::Nop); // JmpFalse r -> false
                        jumps.push((pos, r, true));
                        continue;
                    }
                    // `isset($c[$k])`: a string offset out of range reads as
                    // "" (not null), so dimension isset has its own opcode.
                    if let Expression::ArrayAccess(aa) = v {
                        let arr = self.compile_quiet(aa.array)?;
                        let key = self.compile_expr(aa.index)?;
                        let r = self.alloc_temp();
                        self.emit(Instr::IssetDim { dst: r, arr, key });
                        let pos = self.code.len();
                        self.emit(Instr::Nop); // JmpFalse r -> false
                        jumps.push((pos, r, true));
                        continue;
                    }
                    let r = self.compile_quiet(v)?; // null if absent (DimFetch returns null)
                    let pos = self.code.len();
                    self.emit(Instr::Nop); // JmpNull r -> false
                    jumps.push((pos, r, false));
                }
                self.emit(Instr::LoadBool { dst, b: true });
                let jend = self.code.len();
                self.emit(Instr::Nop); // Jmp end
                let lfalse = self.here();
                self.emit(Instr::LoadBool { dst, b: false });
                let lend = self.here();
                for (pos, r, is_bool) in jumps {
                    self.code[pos] = if is_bool {
                        Instr::JmpFalse {
                            cond: r,
                            target: lfalse,
                        }
                    } else {
                        Instr::JmpNull {
                            src: r,
                            target: lfalse,
                        }
                    };
                }
                self.code[jend] = Instr::Jmp { target: lend };
                Ok(dst)
            }
            Construct::Empty(e) => {
                // `empty($o->prop)` = !isset($o->prop) || !$o->prop: an
                // inaccessible / unset property is empty (no error), via
                // `__isset` then `__get` like php.
                let prop = match e.value {
                    Expression::Access(Access::Property(pa)) => Some((pa.object, &pa.property)),
                    Expression::Access(Access::NullSafeProperty(pa)) => {
                        Some((pa.object, &pa.property))
                    }
                    _ => None,
                };
                if let Some((object, property)) = prop {
                    let obj = self.compile_quiet(object)?;
                    let dst = self.alloc_temp();
                    match self.compile_selector(property)? {
                        Sel::Static(name) => self.emit(Instr::EmptyProp { dst, obj, name }),
                        Sel::Dyn(name) => self.emit(Instr::EmptyPropDyn { dst, obj, name }),
                    }
                    return Ok(dst);
                }
                if let Expression::ArrayAccess(aa) = e.value {
                    if !is_globals_var(aa.array) {
                        let arr = self.compile_quiet(aa.array)?;
                        let key = self.compile_expr(aa.index)?;
                        let dst = self.alloc_temp();
                        self.emit(Instr::EmptyDim { dst, arr, key });
                        return Ok(dst);
                    }
                }
                // empty(x) == !to_bool(x); absent/null -> true (Not handles it).
                let r = self.compile_quiet(e.value)?;
                let dst = self.alloc_temp();
                self.emit(Instr::Not { dst, a: r });
                Ok(dst)
            }
            Construct::Exit(e) => self.compile_exit(e.arguments.as_ref()),
            Construct::Die(d) => self.compile_exit(d.arguments.as_ref()),
        }
    }

    /// `exit`/`die` with an optional single argument (a status int or a message
    /// string to print). Evaluates the argument (or null) and emits `Exit`.
    fn compile_exit(
        &mut self,
        arguments: Option<&crate::parse::cst::ArgumentList>,
    ) -> Result<Reg, Thrown> {
        let arg = match arguments.and_then(|a| a.arguments.first()) {
            Some(Argument::Positional(p)) => self.compile_expr(p.value)?,
            _ => {
                let r = self.alloc_temp();
                self.emit(Instr::LoadNull { dst: r });
                r
            }
        };
        self.emit(Instr::Exit { arg });
        // `exit` never returns a value, but expressions expect a result register.
        let dst = self.alloc_temp();
        self.emit(Instr::LoadNull { dst });
        Ok(dst)
    }

    /// `yield` / `yield $v` / `yield $k => $v`. The result register receives the
    /// value sent in via `Generator::send()` when execution resumes.
    fn compile_yield(&mut self, y: &crate::parse::cst::Yield) -> Result<Reg, Thrown> {
        use crate::parse::cst::Yield;
        let (key, value) = match y {
            Yield::Value(v) => {
                let value = match v.value {
                    // A by-reference generator yields a reference to the
                    // variable / element / property (ZEND_YIELD, BP_VAR_W).
                    Some(e) if self.returns_ref => self.compile_yield_ref(e)?,
                    Some(e) => self.compile_expr(e)?,
                    None => {
                        let t = self.alloc_temp();
                        self.emit(Instr::LoadNull { dst: t });
                        if self.returns_ref {
                            // A bare `yield` yields null, without the
                            // "Only variable references" notice.
                            let r = self.alloc_temp();
                            self.emit(Instr::MakeRefVal { dst: r, src: t });
                            r
                        } else {
                            t
                        }
                    }
                };
                (Reg::MAX, value)
            }
            Yield::Pair(p) => {
                let key = self.compile_expr(p.key)?;
                let value = if self.returns_ref {
                    self.compile_yield_ref(p.value)?
                } else {
                    self.compile_expr(p.value)?
                };
                (key, value)
            }
            Yield::From(_) if self.returns_ref => {
                return Err(Thrown::error(
                    "Error",
                    "Cannot use \"yield from\" inside a by-reference generator",
                ))
            }
            Yield::From(yf) => return self.compile_yield_from(yf.iterator),
        };
        let dst = self.alloc_temp();
        self.emit(Instr::Yield { dst, key, value });
        Ok(dst)
    }

    /// The value a by-reference generator yields: a reference to a variable
    /// / element / property, or a by-reference call's result; anything else
    /// is a plain value, which the VM's `Yield` answers with php's "Only
    /// variable references should be yielded by reference" notice.
    fn compile_yield_ref(&mut self, e: &Expression) -> Result<Reg, Thrown> {
        self.compile_ref_source(e)
    }

    /// `yield from $iterable`: re-yield each element (keys preserved); the
    /// expression evaluates to the inner generator's return value (null for an
    /// array). Lowered as a foreach-style loop over existing opcodes.
    fn compile_yield_from(&mut self, iterable: &Expression) -> Result<Reg, Thrown> {
        let src = self.compile_expr(iterable)?;
        let arr_reg = self.alloc_temp();
        let pos_reg = self.alloc_temp();
        let key_cv = self.alloc_temp();
        let val_cv = self.alloc_temp();

        let reset_pos = self.code.len();
        self.emit(Instr::Nop); // FeReset
        let fetch_label = self.here();
        let fetch_pos = self.code.len();
        self.emit(Instr::Nop); // FeFetch
        let ysink = self.alloc_temp();
        self.emit(Instr::Yield {
            dst: ysink,
            key: key_cv,
            value: val_cv,
        });
        self.emit(Instr::Jmp {
            target: fetch_label,
        });
        let end = self.here();
        self.code[reset_pos] = Instr::FeReset {
            arr_reg,
            pos_reg,
            src,
            end,
            delegate: true,
        };
        self.code[fetch_pos] = Instr::FeFetch {
            arr_reg,
            pos_reg,
            key_cv,
            val_cv,
            end,
        };

        // Result: `$src instanceof Generator ? $src->getReturn() : null`.
        let dst = self.alloc_temp();
        let is_gen = self.alloc_temp();
        let gen_class = ClassRef::Name(self.intern_name(b"Generator"));
        self.emit(Instr::InstanceOf {
            dst: is_gen,
            val: src,
            class: gen_class,
        });
        let jf = self.code.len();
        self.emit(Instr::Nop); // JmpFalse is_gen -> lnull
        let base = self.alloc_temps(1);
        let name = self.intern_name(b"getReturn");
        self.emit(Instr::CallMethod {
            dst,
            obj: src,
            name,
            base,
            argc: 0,
        });
        let jend = self.code.len();
        self.emit(Instr::Nop); // Jmp -> lend
        let lnull = self.here();
        self.emit(Instr::LoadNull { dst });
        let lend = self.here();
        self.code[jf] = Instr::JmpFalse {
            cond: is_gen,
            target: lnull,
        };
        self.code[jend] = Instr::Jmp { target: lend };
        Ok(dst)
    }

    /// Load a byte-string constant into a fresh register.
    pub(crate) fn load_string_const(&mut self, bytes: &[u8]) -> Reg {
        let k = self.intern_const(Value::str(bytes.to_vec()));
        let dst = self.alloc_temp();
        self.emit(Instr::LoadConst { dst, k });
        dst
    }

    /// Build a two-element `[first, second-string]` array (a callable pair).
    pub(crate) fn build_pair(&mut self, first: Reg, second: &[u8]) -> Reg {
        let arr = self.alloc_temp();
        self.emit(Instr::NewArray { dst: arr, cap: 2 });
        self.emit(Instr::ArrPush { arr, val: first });
        let s = self.load_string_const(second);
        self.emit(Instr::ArrPush { arr, val: s });
        arr
    }

    /// The raw identifier bytes of a member selector (non-dynamic only).
    pub(crate) fn selector_string(
        &self,
        sel: &crate::parse::cst::ClassLikeMemberSelector,
    ) -> Result<Vec<u8>, Thrown> {
        match sel {
            ClassLikeMemberSelector::Identifier(li) => Ok(li.value.to_vec()),
            _ => Err(Thrown::unsupported(
                "dynamic member name",
                "`->$x(...)` deferred",
            )),
        }
    }
}

/// Does a function body contain a top-level `yield` (making it a generator)?
/// Does not descend into nested function-likes (their yields are their own).
pub(crate) fn block_has_yield(stmts: &crate::parse::cst::Sequence<Statement>) -> bool {
    stmts.iter().any(stmt_has_yield)
}

pub(crate) fn stmt_has_yield(s: &Statement) -> bool {
    node_has_yield(Node::Statement(s))
}

pub(crate) fn expr_has_yield(e: &Expression) -> bool {
    node_has_yield(Node::Expression(e))
}

/// Any `yield` / `yield from` below `node`, not counting nested function-likes
/// and class bodies (their yields are their own).
fn node_has_yield(node: Node) -> bool {
    match node {
        Node::Expression(Expression::Yield(_)) => return true,
        Node::Closure(_)
        | Node::ArrowFunction(_)
        | Node::Function(_)
        | Node::Class(_)
        | Node::Interface(_)
        | Node::Trait(_)
        | Node::Enum(_)
        | Node::AttributeList(_) => return false,
        Node::AnonymousClass(ac) => {
            return ac
                .argument_list
                .as_ref()
                .is_some_and(|l| node_has_yield(Node::PartialArgumentList(l)));
        }
        _ => {}
    }
    let mut found = false;
    node.visit_children(|ch| found = found || node_has_yield(ch));
    found
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
