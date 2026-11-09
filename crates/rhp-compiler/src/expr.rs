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
use crate::util::{compile_error, dirname, strip_dollar};

impl ProtoBuilder<'_> {
    pub(crate) fn compile_expr(&mut self, expr: &Expression) -> Result<Reg, Thrown> {
        use mago_span::HasSpan;
        if self.scope_kind == ScopeKind::Init {
            // A constant expression reports at the line of the node being
            // evaluated (zend_ast_evaluate_ex's EG(lineno_override)).
            let line = self.cx.line_of(expr.span().start.offset);
            let saved = std::mem::replace(&mut self.cur_line, line);
            let r = self.compile_expr_at(expr);
            self.cur_line = saved;
            return r;
        }
        self.compile_expr_at(expr)
    }

    fn compile_expr_at(&mut self, expr: &Expression) -> Result<Reg, Thrown> {
        use mago_span::HasSpan;
        let addr = expr as *const Expression as usize;
        let inner = self.sc_inner == addr;
        let link = chain_object(expr);
        if let Some(obj) = link {
            self.sc_inner = obj as *const Expression as usize;
        } else if inner {
            self.sc_inner = 0;
        }
        if link.is_none() || inner {
            return self
                .compile_expr_inner(expr)
                .map_err(|e| self.cx.locate(e, expr.span().start.offset));
        }
        // The outermost link of a chain: a nullsafe guard anywhere in it
        // makes the whole chain null.
        let saved = std::mem::take(&mut self.sc_jumps);
        let r = self.compile_expr_inner(expr);
        let jumps = std::mem::replace(&mut self.sc_jumps, saved);
        let r = r.map_err(|e| self.cx.locate(e, expr.span().start.offset))?;
        if jumps.is_empty() {
            return Ok(r);
        }
        let jend = self.code.len();
        self.emit(Instr::Nop);
        let lnull = self.here();
        self.emit(Instr::LoadNull { dst: r });
        let end = self.here();
        self.code[jend] = Instr::Jmp { target: end };
        for (pos, src) in jumps {
            self.code[pos] = Instr::JmpNull { src, target: lnull };
        }
        Ok(r)
    }

    /// The scanner's "Octal escape sequence overflow" warnings for a string
    /// body starting at source `offset`.
    fn octal_overflow_warnings(&self, raw: &[u8], offset: u32) {
        for (at, digits) in crate::strings::octal_overflows(raw) {
            self.cx.scan_diag(
                rhp_values::E_COMPILE_WARNING,
                offset + at as u32,
                format!("Octal escape sequence overflow \\{digits} is greater than \\377"),
            );
        }
    }

    /// The scanner's 8.5 deprecation of the long cast spellings.
    fn cast_deprecation(&self, op: &UnaryPrefixOperator) {
        use mago_span::HasSpan;
        use UnaryPrefixOperator as Op;
        let (from, to) = match op {
            Op::BinaryCast(..) => ("binary", "string"),
            Op::BooleanCast(..) => ("boolean", "bool"),
            Op::DoubleCast(..) => ("double", "float"),
            Op::IntegerCast(..) => ("integer", "int"),
            _ => return,
        };
        self.cx.scan_diag(
            rhp_values::E_DEPRECATED,
            op.span().start.offset,
            format!("Non-canonical cast ({from}) is deprecated, use the ({to}) cast instead"),
        );
    }

    /// The scanner's diagnostics in code php scans but never compiles
    /// (the right side of `false && ...`, assert() under
    /// zend.assertions=-1): long cast spellings and octal overflows.
    pub(crate) fn scanner_diags(&self, node: crate::parse::cst::Node) {
        use crate::parse::cst::DocumentKind;
        use mago_span::HasSpan;
        match node {
            Node::UnaryPrefix(u) => self.cast_deprecation(&u.operator),
            Node::LiteralString(s) if matches!(s.raw.first(), Some(b'"' | b'b' | b'B')) => {
                self.octal_overflow_warnings(s.raw, s.span().start.offset)
            }
            Node::CompositeString(cs) => {
                let (parts, escapes) = match cs {
                    CompositeString::Interpolated(i) => (&i.parts, true),
                    CompositeString::Document(d) => (&d.parts, d.kind != DocumentKind::Nowdoc),
                    CompositeString::ShellExecute(s) => (&s.parts, true),
                };
                for part in parts.iter() {
                    match part {
                        StringPart::Literal(l) => {
                            if escapes {
                                self.octal_overflow_warnings(l.raw, l.span.start.offset);
                            }
                        }
                        p => self.scanner_diags(Node::StringPart(p)),
                    }
                }
                return;
            }
            _ => {}
        }
        for child in node.children() {
            self.scanner_diags(child);
        }
    }

    /// A read of a chain of dimensions and properties on a variable
    /// (`$a->$b->{$c[1]}`, `$a[f()][g()]`) as php's delayed compilation
    /// orders it: every key and name first, then the fetches, so a key's
    /// side effects and warnings come before an inner fetch's. `None` when
    /// the chain has nothing to reorder (or a form this does not handle).
    fn compile_delayed_chain(&mut self, expr: &Expression) -> Result<Option<Reg>, Thrown> {
        enum Link<'a> {
            Dim(&'a Expression<'a>),
            Prop(&'a crate::parse::cst::ClassLikeMemberSelector<'a>),
        }
        let simple = |e: &Expression| {
            matches!(
                e,
                Expression::Literal(_) | Expression::Variable(Variable::Direct(_))
            )
        };
        let mut links = Vec::new();
        let mut complex = false;
        let mut e = expr;
        loop {
            match e {
                Expression::ArrayAccess(aa) if !is_globals_var(aa.array) => {
                    complex |= !simple(aa.index);
                    links.push(Link::Dim(aa.index));
                    e = aa.array;
                }
                Expression::Access(Access::Property(pa)) => {
                    use crate::parse::cst::ClassLikeMemberSelector as S;
                    complex |= matches!(&pa.property, S::Expression(x) if !simple(x.expression));
                    links.push(Link::Prop(&pa.property));
                    e = pa.object;
                }
                _ => break,
            }
        }
        let base = match e {
            Expression::Variable(Variable::Direct(d))
                if !matches!(strip_dollar(d.name), b"this" | b"GLOBALS") =>
            {
                strip_dollar(d.name)
            }
            _ => return Ok(None),
        };
        if links.len() < 2 || !complex || self.in_const_expr() {
            return Ok(None);
        }
        links.reverse();
        let mut keys = Vec::with_capacity(links.len());
        for link in &links {
            keys.push(match link {
                Link::Dim(k) => Sel::Dyn(self.compile_expr(k)?),
                Link::Prop(s) => self.compile_prop_selector(s)?,
            });
        }
        let mut cur = self.cv(base);
        for (link, key) in links.iter().zip(keys) {
            let dst = self.alloc_temp();
            self.emit(match (link, key) {
                (Link::Dim(_), Sel::Dyn(key)) => Instr::DimFetch { dst, arr: cur, key },
                (Link::Prop(_), Sel::Dyn(name)) => Instr::FetchPropDyn {
                    dst,
                    obj: cur,
                    name,
                },
                (_, Sel::Static(name)) => Instr::FetchProp {
                    dst,
                    obj: cur,
                    name,
                },
            });
            cur = dst;
        }
        Ok(Some(cur))
    }

    /// A nullsafe guard on `src` for the open chain.
    pub(crate) fn nullsafe_guard(&mut self, src: Reg) {
        // JMP_NULL reads its operand (an undefined variable warns).
        if src < self.num_cvs {
            self.emit(Instr::CheckCv { cv: src });
        }
        let pos = self.code.len();
        self.emit(Instr::Nop);
        self.sc_jumps.push((pos, src));
    }

    fn compile_expr_inner(&mut self, expr: &Expression) -> Result<Reg, Thrown> {
        if !self.expr_regs.is_empty() {
            let addr = expr as *const Expression as usize;
            if let Some(&(_, r)) = self.expr_regs.iter().find(|(a, _)| *a == addr) {
                return Ok(r);
            }
        }
        if matches!(
            expr,
            Expression::ArrayAccess(_) | Expression::Access(Access::Property(_))
        ) {
            if let Some(r) = self.compile_delayed_chain(expr)? {
                return Ok(r);
            }
        }
        match expr {
            Expression::Literal(lit) => self.compile_literal(lit),
            Expression::Parenthesized(p) => self.compile_expr(p.expression),
            Expression::Variable(Variable::Direct(d)) if strip_dollar(d.name) == b"GLOBALS" => {
                let dst = self.alloc_temp();
                self.emit(Instr::FetchGlobals { dst });
                Ok(dst)
            }
            Expression::Variable(Variable::Direct(d)) if strip_dollar(d.name) == b"this" => {
                Ok(self.this_reg())
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
                if self.in_const_expr() {
                    self.emit(Instr::ConstExprDim { arr });
                }
                let dst = self.alloc_temp();
                self.emit(Instr::DimFetch { dst, arr, key });
                // FETCH_DIM_R frees a call's result once the element is
                // out, so the rest of the statement no longer holds it.
                if matches!(aa.array, Expression::Call(_)) && arr >= self.num_cvs {
                    self.emit(Instr::LoadNull { dst: arr });
                }
                Ok(dst)
            }
            Expression::ConstantAccess(ca) => self.compile_constant(&ca.name),
            Expression::Identifier(id) => self.compile_constant(id),
            Expression::CompositeString(cs) => self.compile_composite(cs),
            Expression::Match(m) => self.compile_match(m),
            Expression::Instantiation(inst) => {
                use mago_span::HasSpan;
                // ZEND_AST_NEW takes its class name's line.
                self.call_lines
                    .push(self.cx.line_of(inst.class.span().start.offset));
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
                    // php's `callable_expr argument_list`: the line the
                    // argument list starts on (a plain name keeps its own).
                    Call::Function(fc) if !matches!(fc.function, Expression::Identifier(_)) => {
                        fc.argument_list.left_parenthesis
                    }
                    Call::Function(fc) => fc.function.span(),
                    Call::Method(mc) => mc.method.span(),
                    Call::NullSafeMethod(mc) => mc.method.span(),
                    Call::StaticMethod(smc) => smc.method.span(),
                };
                let args = match call {
                    Call::Function(fc) => &fc.argument_list,
                    Call::Method(mc) => &mc.argument_list,
                    Call::NullSafeMethod(mc) => &mc.argument_list,
                    Call::StaticMethod(smc) => &smc.argument_list,
                };
                self.check_arg_order(args);
                self.call_lines.push(self.cx.line_of(at.start.offset));
                let r = self.compile_call(call);
                self.call_lines.pop();
                r
            }
            Expression::Pipe(p) => self.compile_pipe(p),
            // zend_compile_func_decl leaves CG(zend_lineno) at the
            // function's last line: what follows reports there.
            Expression::ArrowFunction(af) => {
                use mago_span::HasSpan;
                let r = self.compile_arrow(af);
                self.cur_line = self.cx.line_of(af.span().end.offset.saturating_sub(1));
                r
            }
            Expression::Closure(cl) => {
                use mago_span::HasSpan;
                let r = self.compile_closure(cl);
                self.cur_line = self.cx.line_of(cl.span().end.offset.saturating_sub(1));
                r
            }
            Expression::PartialApplication(pa) => self.compile_fcc(pa),
            Expression::Yield(y) => self.compile_yield(y),
            Expression::MagicConstant(mc) => self.compile_magic_const(mc),
            Expression::ArrayAppend(_) if self.func_arg_read => {
                let k = self.intern_const(Value::str("Cannot use [] for reading"));
                self.emit(Instr::ThrowError { k });
                Ok(self.alloc_temp())
            }
            Expression::ArrayAppend(_) => {
                Err(compile_error("Cannot use [] for reading", self.cur_line))
            }
            Expression::List(_) => Err(compile_error(
                "Cannot use list() as standalone expression",
                self.cur_line,
            )),
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
            let name = match (&self.hook, self.scope_kind) {
                (Some(h), _) => h.0.clone(),
                (None, ScopeKind::Init) => {
                    self.cx.cur_property.borrow().clone().unwrap_or_default()
                }
                _ => Vec::new(),
            };
            let k = self.intern_const(Value::str(name));
            self.emit(Instr::LoadConst { dst, k });
            return Ok(dst);
        }
        let s: Vec<u8> = match mc {
            MagicConstant::File(_) => self.cx.file_name(),
            MagicConstant::Directory(_) => dirname(self.cx.builtins, &self.cx.file_name()),
            MagicConstant::Function(_) => self.magic_function_name(),
            MagicConstant::Method(_) => self.magic_method_name(),
            // __TRAIT__ is "" outside a trait.
            MagicConstant::Trait(_) if !self.cx.in_trait.get() => Vec::new(),
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
        if let Literal::String(s) = lit {
            if matches!(s.raw.first(), Some(b'"') | Some(b'b' | b'B')) {
                use mago_span::HasSpan;
                self.octal_overflow_warnings(s.raw, s.span().start.offset);
            }
        }
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

    /// After both operands of `.` are compiled: php casts a constant array
    /// operand to string (ZEND_CAST, which warns) before the concat.
    fn concat_const_array_cast(&mut self, e: &Expression, r: Reg) -> Reg {
        if matches!(e, Expression::Array(_) | Expression::LegacyArray(_))
            && matches!(crate::fold::fold_const(self.cx, e), Ok(Value::Array(_)))
        {
            let dst = self.alloc_temp();
            self.emit(Instr::CastString { dst, a: r });
            return dst;
        }
        r
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

        let (mut a, mut r) = (self.compile_expr(b.lhs)?, self.compile_expr(b.rhs)?);
        if let Op::StringConcat(_) = &b.operator {
            a = self.concat_const_array_cast(b.lhs, a);
            r = self.concat_const_array_cast(b.rhs, r);
        }
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
                // ZEND_BOOL_XOR: (bool)a !== (bool)b.
                let (ta, tb) = (self.alloc_temp(), self.alloc_temp());
                self.emit(Instr::CastBool { dst: ta, a });
                self.emit(Instr::CastBool { dst: tb, a: r });
                Instr::Cmp {
                    dst,
                    a: ta,
                    b: tb,
                    kind: CmpKind::NotIdentical,
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
        // zend_compile_short_circuiting: a constant left side that decides
        // the result leaves the right side uncompiled (no diagnostics).
        if let Some(v) = literal_bool(b.lhs) {
            if v != is_and {
                self.scanner_diags(crate::parse::cst::Node::Expression(b.lhs));
                self.scanner_diags(crate::parse::cst::Node::Expression(b.rhs));
                let dst = self.alloc_temp();
                self.emit(Instr::LoadBool { dst, b: v });
                return Ok(dst);
            }
        }
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
        self.emit(Instr::MovIs { dst, src: a });
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
            Op::UnsetCast(..) => {
                return Err(compile_error(
                    "The (unset) cast is no longer supported",
                    self.cur_line,
                ))
            }
            Op::BinaryCast(..) | Op::BooleanCast(..) | Op::DoubleCast(..) | Op::IntegerCast(..) => {
                self.cast_deprecation(&u.operator)
            }
            // Only a statement (php's grammar refuses it elsewhere): see
            // `compile_discarded`.
            Op::VoidCast(..) => return self.compile_expr(u.operand),
            Op::RealCast(..) => {
                return Err(Thrown::unsupported(
                    "(real) cast",
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
        // php compiles `+x` to `x * 1`.
        if matches!(u.operator, Op::Plus(_)) {
            let one = self.alloc_temp();
            self.emit(Instr::LoadInt { dst: one, n: 1 });
            self.emit(Instr::Mul { dst, a, b: one });
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
        check_globals_write(target, self.cur_line)?;
        self.check_write_target(target, self.cur_line)?;
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
            | Expression::ArrayAppend(_)
            | Expression::Variable(_)
            | Expression::Access(Access::Property(_))
            | Expression::Access(Access::StaticProperty(_)) => {
                let mut steps = Vec::new();
                let root = self.collect_lvalue_steps(target, &mut steps)?;
                self.emit_cursor_mode(root, &steps, crate::assign::LvalMode::Rw);
                let dst = self.alloc_temp();
                self.emit(Instr::IncDecLval { dst, pre, delta });
                Ok(dst)
            }
            // zend_ensure_writable_variable.
            Expression::Call(c) => {
                use crate::parse::cst::Call;
                use mago_span::HasSpan;
                let msg = match c {
                    Call::Function(_) => "Can't use function return value in write context",
                    _ => "Can't use method return value in write context",
                };
                self.cx.fatal(target.span().start.offset, msg.into());
                Ok(self.alloc_temp())
            }
            _ => Err(Thrown::unsupported(
                "++/-- target",
                "variables, array elements, properties",
            )),
        }
    }

    fn compile_conditional(&mut self, c: &crate::parse::cst::Conditional) -> Result<Reg, Thrown> {
        // zend_compile_conditional: nested unparenthesized ternaries.
        if let Expression::Conditional(inner) = c.condition {
            let msg = match (inner.then.is_some(), c.then.is_some()) {
                (true, true) => Some("Unparenthesized `a ? b : c ? d : e` is not supported. Use either `(a ? b : c) ? d : e` or `a ? b : (c ? d : e)`"),
                (true, false) => Some("Unparenthesized `a ? b : c ?: d` is not supported. Use either `(a ? b : c) ?: d` or `a ? b : (c ?: d)`"),
                (false, true) => Some("Unparenthesized `a ?: b ? c : d` is not supported. Use either `(a ?: b) ? c : d` or `a ?: (b ? c : d)`"),
                (false, false) => None,
            };
            if let Some(msg) = msg {
                return Err(compile_error(msg, self.cur_line));
            }
        }
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

    /// zend_try_ct_eval_array: "Cannot use empty array elements in arrays",
    /// at the line of the last element before the gap.
    pub(crate) fn check_no_empty_elements<'a>(
        &self,
        els: impl Iterator<Item = &'a ArrayElement<'a>>,
    ) -> Result<(), Thrown> {
        use mago_span::HasSpan;
        let mut line = self.cur_line;
        for el in els {
            if let ArrayElement::Missing(_) = el {
                return Err(compile_error(
                    "Cannot use empty array elements in arrays",
                    line,
                ));
            }
            line = self.cx.line_of(el.span().start.offset);
        }
        Ok(())
    }

    fn compile_array<'a>(
        &mut self,
        els: impl Iterator<Item = &'a ArrayElement<'a>> + Clone,
    ) -> Result<Reg, Thrown> {
        self.check_no_empty_elements(els.clone())?;
        // zend_try_ct_eval_array: a constant array unpacking a non-array.
        let has_spread = els
            .clone()
            .any(|el| matches!(el, ArrayElement::Variadic(_)));
        let folded: Option<Vec<(bool, rhp_values::Value)>> = has_spread
            .then(|| els.clone())
            .into_iter()
            .flatten()
            .map(|el| match el {
                ArrayElement::Value(v) if as_reference(v.value).is_none() => {
                    crate::fold::fold_const(self.cx, v.value)
                        .ok()
                        .map(|x| (false, x))
                }
                ArrayElement::KeyValue(kv) if as_reference(kv.value).is_none() => {
                    crate::fold::fold_const(self.cx, kv.key).ok()?;
                    crate::fold::fold_const(self.cx, kv.value)
                        .ok()
                        .map(|x| (false, x))
                }
                ArrayElement::Variadic(v) => crate::fold::fold_const(self.cx, v.value)
                    .ok()
                    .map(|x| (true, x)),
                _ => None,
            })
            .collect();
        if let Some(vals) = folded.filter(|_| has_spread) {
            if let Some((_, v)) = vals
                .iter()
                .find(|(spread, v)| *spread && !matches!(v, rhp_values::Value::Array(_)))
            {
                return Err(compile_error(
                    format!(
                        "Only arrays and Traversables can be unpacked, {} given",
                        crate::oop::value_name(v)
                    ),
                    self.cur_line,
                ));
            }
        }
        // An array of plain literals (CallMap-style data files) is one
        // constant: no per-element temporaries, no runtime building.
        if let Some(v) = literal_array(els.clone()) {
            let k = self.intern_const(v);
            let dst = self.alloc_temp();
            self.emit(Instr::LoadConst { dst, k });
            return Ok(dst);
        }
        let dst = self.alloc_temp();
        // ZEND_INIT_ARRAY's size: the number of elements.
        let cap = els.clone().count() as u32;
        self.emit(Instr::NewArray { dst, cap });
        for el in els {
            match el {
                ArrayElement::KeyValue(kv) => {
                    let key = self.compile_expr(kv.key)?;
                    if let Some(inner) = as_reference(kv.value).filter(|i| !is_this_var(i)) {
                        let val = self.compile_ref_source(inner)?;
                        self.emit(Instr::ArrSetRef { arr: dst, key, val });
                    } else {
                        let v = as_reference(kv.value).unwrap_or(kv.value);
                        let val = self.compile_expr(v)?;
                        self.emit(Instr::ArrSet { arr: dst, key, val });
                    }
                }
                ArrayElement::Value(v) => {
                    if let Some(inner) = as_reference(v.value).filter(|i| !is_this_var(i)) {
                        let val = self.compile_ref_source(inner)?;
                        self.emit(Instr::ArrPushRef { arr: dst, val });
                    } else {
                        let e = as_reference(v.value).unwrap_or(v.value);
                        let val = self.compile_expr(e)?;
                        self.emit(Instr::ArrPush { arr: dst, val });
                    }
                }
                ArrayElement::Variadic(v) => {
                    let src = self.compile_expr(v.value)?;
                    if self.in_const_expr() {
                        self.const_unpack_check(src);
                    }
                    self.emit(Instr::ArrSpread { arr: dst, src });
                }
                ArrayElement::Missing(_) => {}
            }
        }
        Ok(dst)
    }

    /// zend_ast_add_unpacked_element: a constant expression unpacks
    /// arrays only (`is_array($src)` or throw).
    fn const_unpack_check(&mut self, src: Reg) {
        let base = self.alloc_temps(1);
        self.emit(Instr::Mov { dst: base, src });
        let name = self.intern_name(b"is_array");
        let fallback = self.intern_fallback(&None);
        let ok = self.alloc_temp();
        self.emit(Instr::Call {
            dst: ok,
            name,
            base,
            argc: 1,
            fallback,
            flags: rhp_bytecode::CallFlags::default(),
        });
        let jpos = self.code.len();
        self.emit(Instr::Nop);
        let k = self.intern_const(Value::str(
            "Only arrays can be unpacked in constant expression",
        ));
        self.emit(Instr::ThrowError { k });
        let after = self.here();
        self.code[jpos] = Instr::JmpTrue {
            cond: ok,
            target: after,
        };
    }

    /// zend_compile_shell_exec: `` `cmd` `` is deprecated and calls the
    /// global `shell_exec()` (a fully qualified name) with the string.
    fn compile_shell_exec(&mut self, cs: &CompositeString) -> Result<Reg, Thrown> {
        use mago_span::HasSpan;
        self.cx.diag(
            rhp_values::E_DEPRECATED,
            cs.span().start.offset,
            "The backtick (`) operator is deprecated, use shell_exec() instead".into(),
        );
        let cmd = self.compile_composite_parts(cs)?;
        let (name, fallback) = (b"shell_exec".to_vec(), None);
        let base = self.alloc_temps(1);
        self.emit(Instr::MovTake {
            dst: base,
            src: cmd,
        });
        let name = self.intern_name(&name);
        let fallback = self.intern_fallback(&fallback);
        let dst = self.alloc_temp();
        self.emit(Instr::Call {
            dst,
            name,
            base,
            argc: 1,
            fallback,
            flags: rhp_bytecode::CallFlags::default(),
        });
        Ok(dst)
    }

    /// Interpolated (`"...$x..."`) and heredoc strings: concatenate the parts,
    /// stringifying each via `Concat` (which routes to `ops::concat` →
    /// `conv::to_string_bytes`).
    fn compile_composite(&mut self, cs: &CompositeString) -> Result<Reg, Thrown> {
        if matches!(cs, CompositeString::ShellExecute(_)) {
            return self.compile_shell_exec(cs);
        }
        self.compile_composite_parts(cs)
    }

    fn compile_composite_parts(&mut self, cs: &CompositeString) -> Result<Reg, Thrown> {
        let parts = match cs {
            CompositeString::Interpolated(i) => &i.parts,
            CompositeString::Document(d) => &d.parts,
            CompositeString::ShellExecute(s) => &s.parts,
        };
        // Start from "" so every part is stringified uniformly.
        let dst = self.alloc_temp();
        let empty = self.intern_const(Value::str(""));
        self.emit(Instr::LoadConst { dst, k: empty });
        use crate::parse::cst::DocumentKind;
        let escapes = !matches!(cs, CompositeString::Document(d) if d.kind == DocumentKind::Nowdoc);
        for part in parts.iter() {
            let pr = match part {
                StringPart::Literal(l) => {
                    if escapes {
                        self.octal_overflow_warnings(l.raw, l.span.start.offset);
                    }
                    let v = l.value.unwrap_or(l.raw).to_vec();
                    let r = self.alloc_temp();
                    let k = self.intern_const(Value::str(v));
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
                    let k = self.intern_const(Value::str(id.value()));
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
                StringPart::Expression(Expression::Variable(Variable::Indirect(iv))) => {
                    use mago_span::HasSpan;
                    // The encaps list's lineno: its first part's.
                    let list_line = parts
                        .first()
                        .map_or(self.cur_line, |p| self.cx.line_of(p.span().start.offset));
                    self.compile_dollar_brace(iv, list_line)?
                }
                StringPart::Expression(e) => self.compile_expr(e)?,
                StringPart::BracedExpression(b) => self.compile_expr(b.expression)?,
            };
            self.emit(Instr::Concat { dst, a: dst, b: pr });
        }
        Ok(dst)
    }

    /// `"${name}"`, `"${name[expr]}"` (php 8.2: "Using ${var} in strings is
    /// deprecated") and `"${expr}"` (a variable variable, deprecated too).
    fn compile_dollar_brace(
        &mut self,
        iv: &crate::parse::cst::IndirectVariable,
        list_line: u32,
    ) -> Result<Reg, Thrown> {
        let at = iv.dollar_left_brace.start.offset;
        let src = self.cx.src;
        let name =
            crate::strings::dollar_brace_varname(src, iv.dollar_left_brace.end.offset as usize);
        let Some(name) = name else {
            // zend_compile_encaps_list, at the list's line; the fetch runs
            // at the variable's own line.
            self.cx.compile_diags.borrow_mut().push((
                rhp_values::E_DEPRECATED,
                "Using ${expr} (variable variables) in strings is deprecated, use {${expr}} instead".into(),
                list_line,
            ));
            let saved = std::mem::replace(&mut self.cur_line, self.cx.line_of(at));
            let r = self.compile_variable(&Variable::Indirect(iv.clone()));
            self.cur_line = saved;
            return r;
        };
        self.cx.diag(
            rhp_values::E_DEPRECATED,
            at,
            "Using ${var} in strings is deprecated, use {$var} instead".into(),
        );
        let var = |this: &mut Self| -> Reg {
            if name == b"this" || name == b"GLOBALS" {
                let k = this.intern_const(Value::str(name));
                let n = this.alloc_temp();
                this.emit(Instr::LoadConst { dst: n, k });
                let dst = this.alloc_temp();
                this.emit(Instr::FetchVarVar { dst, name: n });
                dst
            } else {
                this.cv(name)
            }
        };
        match iv.expression {
            Expression::ArrayAccess(aa) => {
                let arr = var(self);
                let key = self.compile_expr(aa.index)?;
                let dst = self.alloc_temp();
                self.emit(Instr::DimFetch { dst, arr, key });
                Ok(dst)
            }
            _ => Ok(var(self)),
        }
    }

    fn compile_constant(&mut self, id: &Identifier) -> Result<Reg, Thrown> {
        let name = identifier_name(id);
        let dst = self.alloc_temp();
        // `namespace\true` and `a\true` are ordinary constants.
        if matches!(id, Identifier::Qualified(_)) {
            let (primary, _) = self.cx.resolve_name(id, NameKind::Const);
            let n = self.intern_name(&primary);
            self.emit(Instr::FetchConst {
                dst,
                name: n,
                fallback: rhp_bytecode::NO_NAME,
            });
            return Ok(dst);
        }
        // zend_get_halt_offset_constant: the file's own halt offset.
        if let Some(n) = self.cx.halt_offset {
            let halt = match id {
                Identifier::Local(l) => l.value == b"__COMPILER_HALT_OFFSET__",
                Identifier::FullyQualified(f) => f.value == b"\\__COMPILER_HALT_OFFSET__",
                Identifier::Qualified(_) => false,
            };
            if halt {
                self.emit(Instr::LoadInt { dst, n });
                return Ok(dst);
            }
        }
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
        self.emit_at_operand_line(path_expr, Instr::Include { dst, path, kind });
        Ok(dst)
    }

    /// Emit an include/eval opcode at its operand's line: php's
    /// ZEND_AST_INCLUDE_OR_EVAL takes its child's lineno (a heredoc's is
    /// the line its body starts on).
    fn emit_at_operand_line(&mut self, operand: &Expression, instr: Instr) {
        use crate::parse::cst::{CompositeString, Literal};
        use mago_span::HasSpan;
        let mut line = self.cx.line_of(operand.span().start.offset);
        let doc = match operand {
            Expression::CompositeString(CompositeString::Document(_)) => true,
            Expression::Literal(Literal::String(s)) => s.raw.starts_with(b"<<<"),
            _ => false,
        };
        if doc {
            line += 1;
        }
        let saved = std::mem::replace(&mut self.cur_line, line);
        self.emit(instr);
        self.cur_line = saved;
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
                self.emit_at_operand_line(e.value, Instr::Eval { dst, src });
                Ok(dst)
            }
            Construct::Include(i) => self.compile_include(i.value, IncludeKind::Include),
            Construct::IncludeOnce(i) => self.compile_include(i.value, IncludeKind::IncludeOnce),
            Construct::Require(i) => self.compile_include(i.value, IncludeKind::Require),
            Construct::RequireOnce(i) => self.compile_include(i.value, IncludeKind::RequireOnce),
            Construct::Isset(i) => {
                // AND over each operand: true iff every value is non-null.
                for v in i.values.iter() {
                    let mut e: &Expression = v;
                    while let Expression::Parenthesized(p) = e {
                        e = p.expression;
                    }
                    if !crate::assign::is_variable(e) {
                        return Err(compile_error(
                            "Cannot use isset() on the result of an expression (you can use \"null !== expression\" instead)",
                            self.cur_line,
                        ));
                    }
                }
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
                        match self.compile_prop_selector(property)? {
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
                    // isset($this): ZEND_ISSET_ISEMPTY_THIS, no error.
                    let r = if crate::util::is_this(v) {
                        self.cv(b"this")
                    } else {
                        self.compile_quiet(v)? // null if absent (DimFetch returns null)
                    };
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
                    match self.compile_prop_selector(property)? {
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
                let r = if crate::util::is_this(e.value) {
                    self.cv(b"this")
                } else {
                    self.compile_quiet(e.value)?
                };
                // A variable is read quietly (ZEND_ISSET_ISEMPTY_CV / _THIS).
                let r = if r < self.num_cvs {
                    let t = self.alloc_temp();
                    self.emit(Instr::MovIs { dst: t, src: r });
                    t
                } else {
                    r
                };
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
        // php 8.4: exit(int|string $status = 0), named arguments included.
        let arg = match arguments.and_then(|a| a.arguments.first()) {
            Some(Argument::Positional(p)) => self.compile_expr(p.value)?,
            Some(Argument::Named(n)) if n.name.value == b"status" => self.compile_expr(n.value)?,
            _ => Reg::MAX,
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
        self.mark_generator()?;
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
                return Err(compile_error(
                    "Cannot use \"yield from\" inside a by-reference generator",
                    self.cur_line,
                ))
            }
            Yield::From(yf) => return self.compile_yield_from(yf.iterator),
        };
        let dst = self.alloc_temp();
        self.emit(Instr::Yield { dst, key, value });
        Ok(dst)
    }

    /// zend_mark_function_as_generator: `yield` needs a function whose
    /// return type admits a Generator.
    fn mark_generator(&mut self) -> Result<(), Thrown> {
        if self.scope_kind == ScopeKind::Main {
            return Err(compile_error(
                "The \"yield\" expression can only be used inside a function",
                self.cur_line,
            ));
        }
        Ok(())
    }

    /// zend_compile_params: "Returning by reference from a void function
    /// is deprecated" (`function &f(): void`).
    pub(crate) fn check_void_by_ref(&self, display: &str, hint: Option<&crate::parse::cst::Hint>) {
        use mago_span::HasSpan;
        let (Some(t), Some(h)) = (&self.ret_type, hint) else {
            return;
        };
        if self.returns_ref && t.kinds.contains(&rhp_bytecode::TypeKind::Void) {
            self.cx.diag(
                rhp_values::E_DEPRECATED,
                h.span().start.offset,
                format!("{display}(): Returning by reference from a void function is deprecated"),
            );
        }
    }

    /// zend_mark_function_as_generator's return type check. php runs it
    /// after the parameters, at the declaration's line.
    pub(crate) fn check_generator_type(&self, line: u32) -> Result<(), Thrown> {
        if !self.is_generator {
            return Ok(());
        }
        let Some(t) = &self.ret_type else {
            return Ok(());
        };
        let (mask, _) = crate::checks::type_mask(t);
        let named = t.kinds.iter().any(|k| match k {
            rhp_bytecode::TypeKind::Named(n) => [&b"traversable"[..], b"iterator", b"generator"]
                .iter()
                .any(|g| n.as_bytes().eq_ignore_ascii_case(g)),
            rhp_bytecode::TypeKind::Iterable => true,
            _ => false,
        });
        if mask & crate::checks::OBJECT == 0 && !named {
            return Err(compile_error(
                format!(
                    "Generator return type must be a supertype of Generator, {} given",
                    crate::checks::type_string(t)
                ),
                line,
            ));
        }
        Ok(())
    }

    /// The value a by-reference generator yields: a reference to a variable
    /// / element / property, or a by-reference call's result; anything else
    /// is a plain value, which the VM's `Yield` answers with php's "Only
    /// variable references should be yielded by reference" notice.
    fn compile_yield_ref(&mut self, e: &Expression) -> Result<Reg, Thrown> {
        self.compile_ref_source(e)
    }

    /// `yield from $iterable` (ZEND_YIELD_FROM): `YieldFrom` delegates to a
    /// Generator (the frame resumes at `end` with its return value in
    /// `dst`); an array or other Traversable falls through to a
    /// foreach-style loop re-yielding each element, keys preserved, and
    /// evaluates to null.
    fn compile_yield_from(&mut self, iterable: &Expression) -> Result<Reg, Thrown> {
        let src = self.compile_expr(iterable)?;
        let dst = self.alloc_temp();
        let arr_reg = self.alloc_temp();
        let pos_reg = self.alloc_temp();
        self.alloc_temp(); // pos_reg + 1: the iterator handle (see FeReset)
        let key_cv = self.alloc_temp();
        let val_cv = self.alloc_temp();

        let yf_pos = self.code.len();
        self.emit(Instr::Nop); // YieldFrom
        let reset_pos = self.code.len();
        self.emit(Instr::Nop); // FeReset
        let fetch_label = self.here();
        let fetch_pos = self.code.len();
        self.emit(Instr::Nop); // FeFetch
        self.emit(Instr::YieldValues {
            key: key_cv,
            value: val_cv,
        });
        self.emit(Instr::Jmp {
            target: fetch_label,
        });
        let end = self.here();
        self.code[yf_pos] = Instr::YieldFrom { dst, src, end };
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
        // The delegation is over: the iterator's handle goes.
        self.emit(Instr::ClearTemps {
            from: pos_reg + 1,
            to: pos_reg + 2,
        });
        Ok(dst)
    }

    /// Load a byte-string constant into a fresh register.
    pub(crate) fn load_string_const(&mut self, bytes: &[u8]) -> Reg {
        let k = self.intern_const(Value::str(bytes));
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

    /// [`Self::build_pair`] with the method named by a selector, which may
    /// be dynamic (`$o->$name(...)`, `$o->{expr}(...)`).
    pub(crate) fn build_pair_sel(
        &mut self,
        first: Reg,
        sel: &crate::parse::cst::ClassLikeMemberSelector,
    ) -> Result<Reg, Thrown> {
        if let ClassLikeMemberSelector::Identifier(li) = sel {
            return Ok(self.build_pair(first, li.value));
        }
        let name = match self.compile_selector(sel)? {
            crate::oop::Sel::Dyn(r) => r,
            crate::oop::Sel::Static(_) => unreachable!("identifier handled above"),
        };
        let arr = self.alloc_temp();
        self.emit(Instr::NewArray { dst: arr, cap: 2 });
        self.emit(Instr::ArrPush { arr, val: first });
        self.emit(Instr::ArrPush { arr, val: name });
        Ok(arr)
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

/// The object (or class, or array) of a chain link that nullsafe
/// short-circuiting runs through.
fn chain_object<'a>(e: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    use crate::parse::cst::Call;
    match e {
        Expression::Access(Access::Property(p)) => Some(p.object),
        Expression::Access(Access::NullSafeProperty(p)) => Some(p.object),
        Expression::Access(Access::StaticProperty(p)) => Some(p.class),
        Expression::Call(Call::Method(c)) => Some(c.object),
        Expression::Call(Call::NullSafeMethod(c)) => Some(c.object),
        Expression::Call(Call::StaticMethod(c)) => Some(c.class),
        Expression::ArrayAccess(a) => Some(a.array),
        _ => None,
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

/// `$this`: php's `&$this` (ZEND_FETCH_THIS) yields a plain value, so an
/// array element `&$this` is not a reference.
fn is_this_var(e: &Expression) -> bool {
    matches!(e, Expression::Variable(Variable::Direct(d)) if d.name == b"$this")
}

/// The truth value of a literal (`0`, `'x'`, `true`, `null`, ...), which
/// php's compiler knows as a constant, or of `!` and `&&` / `||` over
/// such constants (zend_compile_short_circuiting's constant result).
fn literal_bool(e: &Expression) -> Option<bool> {
    let mut e = e;
    while let Expression::Parenthesized(p) = e {
        e = p.expression;
    }
    match e {
        Expression::UnaryPrefix(u) if matches!(u.operator, UnaryPrefixOperator::Not(_)) => {
            literal_bool(u.operand).map(|v| !v)
        }
        Expression::Binary(b) => {
            let is_and = match b.operator {
                BinaryOperator::And(_) | BinaryOperator::LowAnd(_) => true,
                BinaryOperator::Or(_) | BinaryOperator::LowOr(_) => false,
                _ => return None,
            };
            match literal_bool(b.lhs)? {
                v if v != is_and => Some(v),
                _ => literal_bool(b.rhs),
            }
        }
        Expression::Literal(l) => Some(rhp_values::conv::to_bool(&crate::fold::literal_value(l))),
        Expression::ConstantAccess(ca) => {
            let n = ca.name.value();
            let n = n.strip_prefix(b"\\").unwrap_or(n);
            if n.eq_ignore_ascii_case(b"true") {
                Some(true)
            } else if n.eq_ignore_ascii_case(b"false") || n.eq_ignore_ascii_case(b"null") {
                Some(false)
            } else {
                None
            }
        }
        _ => None,
    }
}
