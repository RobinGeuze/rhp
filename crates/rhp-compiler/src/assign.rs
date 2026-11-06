//! Assignments and lvalues: destructuring, references, compound and `??=` assignment.

use crate::parse::cst::{
    Access, ArrayElement, AssignmentOperator, ClassLikeMemberSelector, Expression,
    UnaryPrefixOperator, Variable,
};
use rhp_bytecode::{ClassRef, Instr, NIdx, Reg};
use rhp_values::Thrown;

use crate::builder::ProtoBuilder;
use crate::oop::Sel;
use crate::util::{compile_error, is_this, reassign_this, strip_dollar};

/// A step in an lvalue path (compiler-side; key regs are pre-evaluated).
pub(crate) enum CursorStep {
    Key(Reg),
    Append,
    Prop(NIdx),
    PropDyn(Reg),
}

/// How a cursor's root variable is fetched.
#[derive(Clone, Copy)]
pub(crate) enum LvalMode {
    W,
    Rw,
    Unset,
}

/// Where an lvalue cursor roots.
pub(crate) enum LvalRoot {
    /// A register: a CV, or a temporary holding a call result.
    Cv(Reg),
    StaticProp(ClassRef, NIdx),
    /// `C::${expr}`: static property with a runtime name.
    StaticPropDyn(ClassRef, Reg),
    /// `$$name` / `${expr}`: the variable named by the register's value.
    VarVar(Reg),
    /// `$GLOBALS[<name reg>]`: a global variable by name.
    Global(Reg),
}

/// `$GLOBALS[<index>]`: the index expression when `e` is that form.
pub(crate) fn globals_index<'a>(e: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    match e {
        Expression::ArrayAccess(aa) if is_globals_var(aa.array) => Some(aa.index),
        _ => None,
    }
}

/// Is `e` the bare `$GLOBALS` variable?
pub(crate) fn is_globals_var(e: &Expression) -> bool {
    matches!(e, Expression::Variable(Variable::Direct(d)) if strip_dollar(d.name) == b"GLOBALS")
}

/// php's compile-time errors for `$GLOBALS` as (the root of) a write target:
/// only `$GLOBALS[$name]...` may be written.
pub(crate) fn check_globals_write(target: &Expression, line: u32) -> Result<(), Thrown> {
    match target {
        e if is_globals_var(e) => Err(compile_error(
            "$GLOBALS can only be modified using the $GLOBALS[$name] = $value syntax",
            line,
        )),
        Expression::ArrayAppend(ap) if is_globals_var(ap.array) => {
            Err(compile_error("Cannot append to $GLOBALS", line))
        }
        Expression::ArrayAccess(aa) if is_globals_var(aa.array) => Ok(()),
        Expression::ArrayAccess(aa) => check_globals_write(aa.array, line),
        Expression::ArrayAppend(ap) => check_globals_write(ap.array, line),
        Expression::Parenthesized(p) => check_globals_write(p.expression, line),
        _ => Ok(()),
    }
}

/// zend_verify_list_assign_target.
fn verify_list_target(target: &Expression, list_style: bool, line: u32) -> Result<(), Thrown> {
    match target {
        Expression::LegacyArray(_) => Err(compile_error(
            "Cannot assign to array(), use [] instead",
            line,
        )),
        Expression::Array(_) if list_style => Err(compile_error("Cannot mix [] and list()", line)),
        Expression::List(_) if !list_style => Err(compile_error("Cannot mix [] and list()", line)),
        Expression::Array(_) | Expression::List(_) => Ok(()),
        // `[&$x]`: by-reference elements are checked where they are compiled.
        e if as_reference(e).is_some() => Ok(()),
        e if !can_write_to_variable(e) => Err(compile_error(
            "Assignments can only happen to writable values",
            line,
        )),
        _ => Ok(()),
    }
}

impl ProtoBuilder<'_> {
    /// A call php compiles to an opcode with a temporary result in a write
    /// context (zend_try_compile_special_func with BP_VAR_W): no container
    /// to write to. Each builtin has its own shape; any other call returns
    /// a VAR that php separates.
    pub(crate) fn is_builtin_call(&self, e: &Expression) -> bool {
        use crate::parse::cst::{Argument, Call, Identifier, Literal};
        let Expression::Call(Call::Function(fc)) = e else {
            return false;
        };
        let name: &[u8] = match fc.function {
            Expression::Identifier(Identifier::FullyQualified(f)) => &f.value[1..],
            Expression::Identifier(Identifier::Local(l)) if self.cx.cur_ns().is_empty() => l.value,
            _ => return false,
        };
        let mut args = Vec::new();
        for a in fc.argument_list.arguments.iter() {
            match a {
                Argument::Positional(p) if p.ellipsis.is_none() => args.push(p.value),
                _ => return false,
            }
        }
        let n = args.len();
        let in_function = self.scope_kind != crate::builder::ScopeKind::Main;
        let lit = |e: &Expression| matches!(e, Expression::Literal(_));
        match name.to_ascii_lowercase().as_slice() {
            b"strlen" | b"is_null" | b"is_bool" | b"is_long" | b"is_int" | b"is_integer"
            | b"is_float" | b"is_double" | b"is_string" | b"is_array" | b"is_object"
            | b"is_resource" | b"is_scalar" | b"boolval" | b"intval" | b"floatval"
            | b"doubleval" | b"strval" | b"count" | b"sizeof" | b"gettype" => n == 1,
            b"defined" => {
                n == 1
                    && match args[0] {
                        Expression::Literal(Literal::String(s)) => !s
                            .value
                            .unwrap_or(s.raw)
                            .iter()
                            .any(|&b| b == b'\\' || b == b':'),
                        other => lit(other),
                    }
            }
            b"get_class" => n <= 1,
            b"get_called_class" => n == 0,
            b"func_num_args" | b"func_get_args" => n == 0 && in_function,
            b"array_key_exists" => n == 2,
            b"array_slice" => {
                in_function
                    && n == 2
                    && matches!(args[1], Expression::Literal(Literal::Integer(_)))
                    && matches!(args[0], Expression::Call(Call::Function(inner))
                        if inner.argument_list.arguments.is_empty()
                            && matches!(inner.function, Expression::Identifier(id)
                                if { let n = crate::names::identifier_name(id);
                                    n.strip_prefix(b"\\").unwrap_or(n)
                                        .eq_ignore_ascii_case(b"func_get_args") }))
            }
            b"in_array" => {
                let strict = match n {
                    2 => false,
                    3 => match args[2] {
                        Expression::Literal(l) => {
                            rhp_values::conv::to_bool(&crate::fold::literal_value(l))
                        }
                        Expression::ConstantAccess(ca) => match self.ct_constant(&ca.name) {
                            Some(v) => rhp_values::conv::to_bool(&v),
                            None => return false,
                        },
                        _ => return false,
                    },
                    _ => return false,
                };
                self.in_array_haystack_ok(args[1], strict)
            }
            b"sprintf" => n >= 1 && sprintf_is_rope(self.cx, args[0], n - 1),
            _ => false,
        }
    }
}

/// zend_compile_func_sprintf's shape: a constant format under 256 bytes of
/// only `%s`, `%d` and `%%`, with one value per placeholder.
fn sprintf_is_rope(cx: &crate::file::FileCtx, format: &Expression, values: usize) -> bool {
    let Ok(rhp_values::Value::Str(f)) = crate::fold::fold_const(cx, format) else {
        return false;
    };
    let f = f.as_bytes();
    if f.len() >= 256 {
        return false;
    }
    let mut count = 0;
    let mut i = 0;
    while i < f.len() {
        if f[i] == b'%' {
            match f.get(i + 1) {
                Some(b's' | b'd') => count += 1,
                Some(b'%') => {}
                _ => return false,
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    count == values
}

/// zend_is_variable.
pub(crate) fn is_variable(e: &Expression) -> bool {
    matches!(
        e,
        Expression::Variable(_)
            | Expression::ArrayAccess(_)
            | Expression::ArrayAppend(_)
            | Expression::Access(
                Access::Property(_) | Access::NullSafeProperty(_) | Access::StaticProperty(_)
            )
    )
}

/// zend_is_call.
pub(crate) fn is_call(e: &Expression) -> bool {
    matches!(e, Expression::Call(_) | Expression::Pipe(_))
}

/// zend_ast_is_short_circuited: a nullsafe operator in the chain.
pub(crate) fn is_short_circuited(e: &Expression) -> bool {
    use crate::parse::cst::Call;
    match e {
        Expression::ArrayAccess(aa) => is_short_circuited(aa.array),
        Expression::ArrayAppend(ap) => is_short_circuited(ap.array),
        Expression::Access(Access::Property(pa)) => is_short_circuited(pa.object),
        Expression::Access(Access::StaticProperty(sp)) => is_short_circuited(sp.class),
        Expression::Access(Access::NullSafeProperty(_)) => true,
        Expression::Call(Call::NullSafeMethod(_)) => true,
        Expression::Call(Call::Method(mc)) => is_short_circuited(mc.object),
        Expression::Call(Call::StaticMethod(sc)) => is_short_circuited(sc.class),
        _ => false,
    }
}

/// zend_can_write_to_variable.
pub(crate) fn can_write_to_variable(mut e: &Expression) -> bool {
    loop {
        match e {
            Expression::ArrayAccess(aa) => e = aa.array,
            Expression::ArrayAppend(ap) => e = ap.array,
            Expression::Access(Access::Property(pa)) => e = pa.object,
            _ => break,
        }
    }
    (is_variable(e) || is_call(e)) && !is_short_circuited(e)
}

/// If `e` is a `&$lvalue` reference-prefix expression, return the referenced
/// operand (used for by-reference array elements `[k => &$v]`).
pub(crate) fn as_reference<'a>(e: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    match e {
        Expression::UnaryPrefix(u) => match &u.operator {
            UnaryPrefixOperator::Reference(_) => Some(u.operand),
            _ => None,
        },
        _ => None,
    }
}

impl ProtoBuilder<'_> {
    /// zend_ensure_writable_variable.
    pub(crate) fn ensure_writable(&self, e: &Expression, line: u32) -> Result<(), Thrown> {
        use crate::parse::cst::Call;
        match e {
            Expression::Call(Call::Function(_)) | Expression::Pipe(_) => Err(compile_error(
                "Can't use function return value in write context",
                line,
            )),
            Expression::Call(_) => Err(compile_error(
                "Can't use method return value in write context",
                line,
            )),
            e if is_short_circuited(e) => Err(compile_error(
                "Can't use nullsafe operator in write context",
                line,
            )),
            e if is_globals_var(e) => Err(compile_error(
                "$GLOBALS can only be modified using the $GLOBALS[$name] = $value syntax",
                line,
            )),
            _ => Ok(()),
        }
    }

    /// The container checks of compiling `e` for writing (BP_VAR_W, or
    /// BP_VAR_UNSET when `unset`): zend_delayed_compile_dim/prop on their
    /// base, zend_separate_if_call_and_write.
    pub(crate) fn check_write_chain(
        &self,
        e: &Expression,
        unset: bool,
        line: u32,
    ) -> Result<(), Thrown> {
        let base = match e {
            Expression::Parenthesized(p) => {
                return self.check_write_chain(p.expression, unset, line)
            }
            Expression::ArrayAccess(aa) => aa.array,
            Expression::ArrayAppend(ap) => {
                if unset {
                    return Err(compile_error("Cannot use [] for unsetting", line));
                }
                ap.array
            }
            Expression::Access(Access::Property(pa)) => pa.object,
            _ => return Ok(()),
        };
        self.check_write_base(base, unset, line)
    }

    fn check_write_base(&self, b: &Expression, unset: bool, line: u32) -> Result<(), Thrown> {
        match b {
            Expression::Parenthesized(p) => self.check_write_base(p.expression, unset, line),
            Expression::Variable(_) | Expression::Access(Access::StaticProperty(_)) => Ok(()),
            Expression::ArrayAccess(_)
            | Expression::ArrayAppend(_)
            | Expression::Access(Access::Property(_)) => self.check_write_chain(b, unset, line),
            // Reported by ensure_writable on the whole target.
            Expression::Access(Access::NullSafeProperty(_)) => Ok(()),
            e if self.is_builtin_call(e) => Err(compile_error(
                "Cannot use result of built-in function in write context",
                line,
            )),
            Expression::Call(_) | Expression::Pipe(_) => Ok(()),
            _ => Err(compile_error(
                "Cannot use temporary expression in write context",
                line,
            )),
        }
    }

    /// Both checks for an assignment-like target.
    pub(crate) fn check_write_target(&self, e: &Expression, line: u32) -> Result<(), Thrown> {
        self.ensure_writable(e, line)?;
        self.check_write_chain(e, false, line)
    }

    /// Assign the elements of `src` (an array value) to a destructuring
    /// pattern, with zend_compile_list_assign's compile-time checks;
    /// `list_style` is `list(...)` rather than `[...]`.
    pub(crate) fn destructure_into<'a>(
        &mut self,
        elements: impl Iterator<Item = &'a ArrayElement<'a>> + Clone,
        src: Reg,
        list_style: bool,
    ) -> Result<(), Thrown> {
        let line = self.cur_line;
        let is_keyed = elements
            .clone()
            .find(|e| !matches!(e, ArrayElement::Missing(_)))
            .is_some_and(|e| matches!(e, ArrayElement::KeyValue(_)));
        let mixed = || {
            compile_error(
                "Cannot mix keyed and unkeyed array entries in assignments",
                line,
            )
        };
        // The source is read once (each element is fetched quietly).
        if src < self.num_cvs {
            self.emit(Instr::CheckCv { cv: src });
        }
        let mut has_elems = false;
        let mut idx: i64 = 0;
        for el in elements {
            match el {
                ArrayElement::Missing(_) if is_keyed => {
                    return Err(compile_error(
                        "Cannot use empty array entries in keyed array assignment",
                        line,
                    ))
                }
                ArrayElement::Missing(_) => idx += 1,
                ArrayElement::Variadic(_) => {
                    return Err(compile_error(
                        "Spread operator is not supported in assignments",
                        line,
                    ))
                }
                ArrayElement::Value(_) if is_keyed => return Err(mixed()),
                ArrayElement::KeyValue(_) if !is_keyed => return Err(mixed()),
                ArrayElement::Value(v) => {
                    has_elems = true;
                    let key = self.alloc_temp();
                    self.emit(Instr::LoadInt { dst: key, n: idx });
                    verify_list_target(v.value, list_style, line)?;
                    let val = self.alloc_temp();
                    self.emit(Instr::DimFetchList {
                        dst: val,
                        arr: src,
                        key,
                    });
                    self.store_to_target(v.value, val)?;
                    idx += 1;
                }
                ArrayElement::KeyValue(kv) => {
                    has_elems = true;
                    let key = self.compile_expr(kv.key)?;
                    verify_list_target(kv.value, list_style, line)?;
                    let val = self.alloc_temp();
                    self.emit(Instr::DimFetchList {
                        dst: val,
                        arr: src,
                        key,
                    });
                    self.store_to_target(kv.value, val)?;
                }
            }
        }
        if !has_elems {
            return Err(compile_error("Cannot use empty list", line));
        }
        Ok(())
    }

    /// The source of a destructuring assignment. A plain variable is read
    /// into a temporary first, as zend_compile_assign does (QM_ASSIGN), so
    /// `[$a, $b] = $a` reads every element from the old `$a`.
    fn list_source(&mut self, rhs: &Expression) -> Result<Reg, Thrown> {
        let src = self.compile_expr(rhs)?;
        if !matches!(rhs, Expression::Variable(Variable::Direct(_))) {
            return Ok(src);
        }
        let dst = self.alloc_temp();
        self.emit(Instr::Mov { dst, src });
        Ok(dst)
    }

    /// Store `src` into a single destructuring target (variable, nested pattern,
    /// array element, or property).
    pub(crate) fn store_to_target(&mut self, target: &Expression, src: Reg) -> Result<(), Thrown> {
        check_globals_write(target, self.cur_line)?;
        if is_this(target) {
            return Err(reassign_this(self.cur_line));
        }
        if !matches!(target, Expression::Array(_) | Expression::List(_)) {
            self.check_write_target(target, self.cur_line)?;
        }
        match target {
            Expression::Variable(Variable::Direct(d)) => {
                let cv = self.cv(strip_dollar(d.name));
                self.emit(Instr::Assign { cv, src });
            }
            Expression::Array(arr) => self.destructure_into(arr.elements.iter(), src, false)?,
            Expression::List(lst) => self.destructure_into(lst.elements.iter(), src, true)?,
            Expression::ArrayAccess(_)
            | Expression::ArrayAppend(_)
            | Expression::Variable(_)
            | Expression::Access(Access::Property(_))
            | Expression::Access(Access::StaticProperty(_)) => {
                let mut steps = Vec::new();
                let root = self.collect_lvalue_steps(target, &mut steps)?;
                self.emit_cursor(root, &steps);
                self.emit(Instr::AssignLval { src, dst: src });
            }
            _ => return Err(Thrown::unsupported("destructuring target", "deferred")),
        }
        Ok(())
    }

    /// Collect an lvalue path (root CV + pre-evaluated key/append steps). All
    /// index expressions are compiled into registers here, so the subsequent
    /// cursor opcode sequence is contiguous (no nested cursors).
    pub(crate) fn collect_lvalue_steps(
        &mut self,
        target: &Expression,
        steps: &mut Vec<CursorStep>,
    ) -> Result<LvalRoot, Thrown> {
        match target {
            Expression::Variable(Variable::Direct(d)) => {
                Ok(LvalRoot::Cv(self.cv(strip_dollar(d.name))))
            }
            Expression::ArrayAccess(aa) if is_globals_var(aa.array) => {
                let name = self.compile_expr(aa.index)?;
                Ok(LvalRoot::Global(name))
            }
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
            Expression::Access(Access::Property(pa)) => {
                let root = self.collect_lvalue_steps(pa.object, steps)?;
                match self.compile_selector(&pa.property)? {
                    Sel::Static(name) => steps.push(CursorStep::Prop(name)),
                    Sel::Dyn(r) => steps.push(CursorStep::PropDyn(r)),
                }
                Ok(root)
            }
            Expression::Access(Access::StaticProperty(spa)) => {
                // Static property is the cursor root, not a step.
                let class = self.class_ref(spa.class)?;
                Ok(match self.static_prop_sel(&spa.property)? {
                    Sel::Static(name) => LvalRoot::StaticProp(class, name),
                    Sel::Dyn(name) => LvalRoot::StaticPropDyn(class, name),
                })
            }
            Expression::Variable(v) => Ok(LvalRoot::VarVar(self.var_name_reg(v)?)),
            Expression::Parenthesized(p) => self.collect_lvalue_steps(p.expression, steps),
            Expression::Access(Access::NullSafeProperty(_))
            | Expression::Call(crate::parse::cst::Call::NullSafeMethod(_)) => Err(Thrown::error(
                "Error",
                "Can't use nullsafe operator in write context",
            )),
            // `f()[$k] = v`, `A::g()->p[] = v`: php allows a call result as the
            // container (zend_delayed_compile_var on a call). The cursor roots
            // at the temporary; objects are handles, so writes through one land
            // in the shared object (an `ArrayAccess` root goes to `offsetSet`).
            Expression::Call(_) => Ok(LvalRoot::Cv(self.compile_expr(target)?)),
            _ => Err(compile_error(
                "Cannot use temporary expression in write context",
                self.cur_line,
            )),
        }
    }

    pub(crate) fn emit_cursor(&mut self, root: LvalRoot, steps: &[CursorStep]) {
        self.emit_cursor_mode(root, steps, LvalMode::W);
    }

    /// [`Self::emit_cursor`] for a read-modify-write or an unset, which treat
    /// an undefined root variable differently (php's BP_VAR_RW / _UNSET).
    pub(crate) fn emit_cursor_mode(
        &mut self,
        root: LvalRoot,
        steps: &[CursorStep],
        mode: LvalMode,
    ) {
        match root {
            LvalRoot::Cv(cv) if cv < self.num_cvs => self.emit(match mode {
                LvalMode::W => Instr::LvalCv { cv },
                LvalMode::Rw => Instr::LvalCvRw { cv },
                LvalMode::Unset => Instr::LvalCvUnset { cv },
            }),
            LvalRoot::Cv(cv) => self.emit(Instr::LvalCv { cv }),
            LvalRoot::StaticProp(class, name) => self.emit(Instr::LvalStaticProp { class, name }),
            LvalRoot::StaticPropDyn(class, name) => {
                self.emit(Instr::LvalStaticPropDyn { class, name })
            }
            LvalRoot::VarVar(name) => self.emit(match mode {
                LvalMode::Rw => Instr::LvalVarVarRw { name },
                _ => Instr::LvalVarVar { name },
            }),
            LvalRoot::Global(name) => self.emit(match mode {
                LvalMode::Rw => Instr::LvalGlobalRw { name },
                _ => Instr::LvalGlobal { name },
            }),
        }
        for step in steps {
            match step {
                CursorStep::Key(r) => self.emit(Instr::LvalKey { key: *r }),
                CursorStep::Append => self.emit(Instr::LvalAppend),
                CursorStep::Prop(n) => self.emit(Instr::LvalProp { name: *n }),
                CursorStep::PropDyn(r) => self.emit(Instr::LvalPropDyn { name: *r }),
            }
        }
    }

    /// Produce a register holding a `Ref` to a reference-assignment source: a
    /// simple variable's slot, or a property / array-element / static-prop lvalue.
    pub(crate) fn compile_ref_source(&mut self, src: &Expression) -> Result<Reg, Thrown> {
        if is_globals_var(src) {
            return Err(compile_error(
                "Cannot acquire reference to $GLOBALS",
                self.cur_line,
            ));
        }
        check_globals_write(src, self.cur_line)?;
        if is_variable(src) {
            if is_short_circuited(src) {
                return Err(compile_error(
                    "Cannot take reference of a nullsafe chain",
                    self.cur_line,
                ));
            }
            self.check_write_chain(src, false, self.cur_line)?;
        } else if self.is_builtin_call(src) {
            return Err(compile_error(
                "Cannot use result of built-in function in write context",
                self.cur_line,
            ));
        }
        match src {
            Expression::Variable(Variable::Direct(v)) => {
                let name = strip_dollar(v.name);
                // `$t =& $this` must NOT let `$t = null` null out `$this` (php
                // protects $this): bind to a fresh ref cell holding the object,
                // not the $this slot itself. The object handle is still shared.
                if name == b"this" {
                    let obj = self.compile_expr(src)?;
                    let r = self.alloc_temp();
                    self.emit(Instr::MakeRefVal { dst: r, src: obj });
                    return Ok(r);
                }
                let cv = self.cv(name);
                let r = self.alloc_temp();
                self.emit(Instr::MakeRef { dst: r, cv });
                Ok(r)
            }
            Expression::ArrayAccess(_)
            | Expression::ArrayAppend(_)
            | Expression::Access(Access::Property(_))
            | Expression::Access(Access::StaticProperty(_)) => {
                let mut steps = Vec::new();
                let root = self.collect_lvalue_steps(src, &mut steps)?;
                self.emit_cursor(root, &steps);
                let r = self.alloc_temp();
                self.emit(Instr::MakeRefLval { dst: r });
                Ok(r)
            }
            Expression::Parenthesized(p) => self.compile_ref_source(p.expression),
            // `$x =& f()`: the reference a by-reference function returns, or
            // (ZEND_RETURNS_FUNCTION) a value with php's notice.
            other if is_call(other) => {
                let r = self.compile_expr(other)?;
                self.emit(Instr::RefFuncResult { src: r });
                Ok(r)
            }
            other => self.compile_expr(other),
        }
    }

    pub(crate) fn compile_assignment(
        &mut self,
        a: &crate::parse::cst::Assignment,
    ) -> Result<Reg, Thrown> {
        let result_unused = std::mem::take(&mut self.result_unused);
        check_globals_write(a.lhs, self.cur_line)?;
        if is_this(a.lhs)
            && matches!(
                a.operator,
                AssignmentOperator::Assign(_) | AssignmentOperator::Coalesce(_)
            )
        {
            return Err(reassign_this(self.cur_line));
        }
        if !matches!(a.lhs, Expression::Array(_) | Expression::List(_)) {
            self.check_write_target(a.lhs, self.cur_line)?;
        }
        // Destructuring: `[$a, $b] = ...` / `list($a, $b) = ...`.
        if matches!(a.operator, AssignmentOperator::Assign(_)) {
            if let Expression::Array(arr) = a.lhs {
                let rhs = self.list_source(a.rhs)?;
                self.destructure_into(arr.elements.iter(), rhs, false)?;
                return Ok(rhs);
            }
            if let Expression::List(lst) = a.lhs {
                let rhs = self.list_source(a.rhs)?;
                self.destructure_into(lst.elements.iter(), rhs, true)?;
                return Ok(rhs);
            }
        }

        // Reference assignment `$b =& $a` — rhs is `&<lvalue>`.
        if let (AssignmentOperator::Assign(_), Expression::UnaryPrefix(u)) = (&a.operator, a.rhs) {
            if matches!(u.operator, UnaryPrefixOperator::Reference(_)) {
                // `$dst =& $src` between simple variables: alias the slots.
                // Exception: `$dst =& $this` must not alias the $this slot (php
                // protects $this), so it goes through compile_ref_source below,
                // which binds a fresh ref cell holding the object.
                if let (
                    Expression::Variable(Variable::Direct(dst)),
                    Expression::Variable(Variable::Direct(src)),
                ) = (a.lhs, u.operand)
                {
                    if strip_dollar(src.name) == b"GLOBALS" {
                        return Err(compile_error(
                            "Cannot acquire reference to $GLOBALS",
                            self.cur_line,
                        ));
                    }
                    if strip_dollar(src.name) != b"this" {
                        let dst_cv = self.cv(strip_dollar(dst.name));
                        let src_cv = self.cv(strip_dollar(src.name));
                        self.emit(Instr::BindRef { dst_cv, src_cv });
                        return Ok(dst_cv);
                    }
                }
                // `$dst =& <lvalue>` (property / array element / static prop): take
                // a reference to the source lvalue and alias the destination var.
                if let Expression::Variable(Variable::Direct(dst)) = a.lhs {
                    let r = self.compile_ref_source(u.operand)?;
                    let dst_cv = self.cv(strip_dollar(dst.name));
                    self.emit(Instr::RefAssign { dst_cv, src: r });
                    // A call's result is what was assigned (a destructor
                    // run by the assignment may unset the variable).
                    if is_call(crate::util::unparen(u.operand)) {
                        return Ok(r);
                    }
                    return Ok(dst_cv);
                }
                // `$$n =& <src>`: alias the variable named at run time.
                if let Expression::Variable(var) = a.lhs {
                    let name = self.var_name_reg(var)?;
                    let r = self.compile_ref_source(u.operand)?;
                    self.emit(Instr::BindVarVarRef { name, src: r });
                    return Ok(r);
                }
                // `<lvalue> =& <src>`: make the destination element/property a
                // reference to the source.
                if matches!(
                    a.lhs,
                    Expression::ArrayAccess(_)
                        | Expression::ArrayAppend(_)
                        | Expression::Access(Access::Property(_))
                        | Expression::Access(Access::StaticProperty(_))
                ) {
                    let r = self.compile_ref_source(u.operand)?;
                    let mut steps = Vec::new();
                    let root = self.collect_lvalue_steps(a.lhs, &mut steps)?;
                    self.emit_cursor(root, &steps);
                    self.emit(Instr::AssignLvalRef { src: r });
                    return Ok(r);
                }
                return Err(Thrown::unsupported(
                    "reference binding target",
                    "unsupported `=&` source/target form",
                ));
            }
        }

        if matches!(a.operator, AssignmentOperator::Coalesce(_)) {
            return self.compile_coalesce_assign(a.lhs, a.rhs);
        }

        // Simple single-level `$o->prop = v`: a dedicated opcode so the VM can
        // route undefined/inaccessible writes to `__set` (the cursor can't).
        if let (AssignmentOperator::Assign(_), Expression::Access(Access::Property(pa))) =
            (&a.operator, a.lhs)
        {
            if let ClassLikeMemberSelector::Identifier(li) = &pa.property {
                let obj = self.compile_write_object(pa.object)?;
                let name = self.intern_name(li.value);
                let rhs = self.compile_expr(a.rhs)?;
                self.emit(Instr::StoreProp {
                    obj,
                    name,
                    src: rhs,
                });
                return Ok(rhs);
            }
            // Dynamic `$o->$x = v` / `$o->{expr} = v`.
            if matches!(
                &pa.property,
                ClassLikeMemberSelector::Variable(_) | ClassLikeMemberSelector::Expression(_)
            ) {
                let obj = self.compile_write_object(pa.object)?;
                let name = match self.compile_selector(&pa.property)? {
                    Sel::Dyn(r) => r,
                    Sel::Static(_) => unreachable!(),
                };
                let rhs = self.compile_expr(a.rhs)?;
                self.emit(Instr::StorePropDyn {
                    obj,
                    name,
                    src: rhs,
                });
                return Ok(rhs);
            }
        }

        // Array-element / property / static-property / variable-variable
        // target: use the lvalue cursor.
        if matches!(
            a.lhs,
            Expression::ArrayAccess(_)
                | Expression::ArrayAppend(_)
                | Expression::Access(Access::Property(_))
                | Expression::Access(Access::StaticProperty(_))
                | Expression::Variable(Variable::Indirect(_) | Variable::Nested(_))
        ) {
            let mut steps = Vec::new();
            let root = self.collect_lvalue_steps(a.lhs, &mut steps)?;
            let rhs = self.compile_expr(a.rhs)?;
            return match &a.operator {
                AssignmentOperator::Assign(_) => {
                    self.emit_cursor(root, &steps);
                    let dst = if result_unused {
                        rhs
                    } else {
                        self.alloc_temp()
                    };
                    self.emit(Instr::AssignLval { src: rhs, dst });
                    Ok(dst)
                }
                op => {
                    self.emit_cursor_mode(root, &steps, LvalMode::Rw);
                    let cur = self.alloc_temp();
                    self.emit(Instr::FetchLvalRw { dst: cur });
                    let res = self.alloc_temp();
                    self.emit(compound_op(op, res, cur, rhs, lvalue_desc(a.lhs))?);
                    self.emit(Instr::AssignLval { src: res, dst: res });
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
                        self.code[jnn] = Instr::JmpNotNull {
                            src: dst,
                            target: end,
                        };
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

    /// `lhs ??= rhs` (php's ZEND_ASSIGN_COALESCE): fetch `lhs` quietly (isset
    /// semantics: no undefined-key/property diagnostics, `ArrayAccess` via
    /// `offsetExists`/`offsetGet`, `__isset`/`__get`); when it is not null that
    /// is the result and nothing is written or evaluated further. Otherwise
    /// evaluate `rhs` and assign it. Sub-expressions of `lhs` (keys, objects)
    /// are evaluated exactly once.
    fn compile_coalesce_assign(
        &mut self,
        lhs: &Expression,
        rhs: &Expression,
    ) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        match lhs {
            Expression::Parenthesized(p) => return self.compile_coalesce_assign(p.expression, rhs),
            Expression::Variable(Variable::Direct(d)) => {
                let cv = self.cv(strip_dollar(d.name));
                self.emit(Instr::MovIs { dst, src: cv });
                let jnn = self.code.len();
                self.emit(Instr::Nop); // JmpNotNull dst -> end
                let r = self.compile_expr(rhs)?;
                self.emit(Instr::Assign { cv, src: r });
                self.emit(Instr::Mov { dst, src: cv });
                let end = self.here();
                self.code[jnn] = Instr::JmpNotNull {
                    src: dst,
                    target: end,
                };
            }
            // Single-level `$o->p ??= v`: the write is a plain property store,
            // so an inaccessible/undefined property routes to `__set`.
            // The container is fetched quietly too (php compiles the whole
            // left side in BP_VAR_IS mode for the check).
            Expression::Access(Access::Property(pa)) => {
                let obj = self.compile_quiet(pa.object)?;
                let sel = self.compile_selector(&pa.property)?;
                match sel {
                    Sel::Static(name) => self.emit(Instr::FetchPropIs { dst, obj, name }),
                    Sel::Dyn(name) => self.emit(Instr::FetchPropDynIs { dst, obj, name }),
                }
                let jnn = self.code.len();
                self.emit(Instr::Nop);
                let r = self.compile_expr(rhs)?;
                match sel {
                    Sel::Static(name) => self.emit(Instr::StoreProp { obj, name, src: r }),
                    Sel::Dyn(name) => self.emit(Instr::StorePropDyn { obj, name, src: r }),
                }
                self.emit(Instr::Mov { dst, src: r });
                let end = self.here();
                self.code[jnn] = Instr::JmpNotNull {
                    src: dst,
                    target: end,
                };
            }
            _ => {
                let mut steps = Vec::new();
                let root = self.collect_lvalue_steps(lhs, &mut steps)?;
                // Quiet read along the same (pre-evaluated) path.
                let mut cur = match &root {
                    LvalRoot::Cv(r) => *r,
                    LvalRoot::StaticProp(class, name) => {
                        let t = self.alloc_temp();
                        self.emit(Instr::FetchStaticPropIs {
                            dst: t,
                            class: *class,
                            name: *name,
                        });
                        t
                    }
                    LvalRoot::StaticPropDyn(class, name) => {
                        let t = self.alloc_temp();
                        self.emit(Instr::FetchStaticPropDynIs {
                            dst: t,
                            class: *class,
                            name: *name,
                        });
                        t
                    }
                    LvalRoot::VarVar(name) => {
                        let t = self.alloc_temp();
                        self.emit(Instr::FetchVarVarIs {
                            dst: t,
                            name: *name,
                        });
                        t
                    }
                    // `$GLOBALS[$k] ??= v`: a quiet read of the global.
                    LvalRoot::Global(name) => {
                        let all = self.alloc_temp();
                        self.emit(Instr::FetchGlobals { dst: all });
                        let t = self.alloc_temp();
                        self.emit(Instr::DimFetchIs {
                            dst: t,
                            arr: all,
                            key: *name,
                        });
                        t
                    }
                };
                for step in &steps {
                    let t = self.alloc_temp();
                    match step {
                        CursorStep::Key(key) => self.emit(Instr::DimFetchIs {
                            dst: t,
                            arr: cur,
                            key: *key,
                        }),
                        CursorStep::Prop(name) => self.emit(Instr::FetchPropIs {
                            dst: t,
                            obj: cur,
                            name: *name,
                        }),
                        CursorStep::PropDyn(name) => self.emit(Instr::FetchPropDynIs {
                            dst: t,
                            obj: cur,
                            name: *name,
                        }),
                        CursorStep::Append => {
                            return Err(compile_error("Cannot use [] for reading", self.cur_line))
                        }
                    }
                    cur = t;
                }
                self.emit(Instr::Mov { dst, src: cur });
                let jnn = self.code.len();
                self.emit(Instr::Nop);
                let r = self.compile_expr(rhs)?;
                self.emit_cursor(root, &steps);
                self.emit(Instr::AssignLval { src: r, dst: r });
                self.emit(Instr::Mov { dst, src: r });
                let end = self.here();
                self.code[jnn] = Instr::JmpNotNull {
                    src: dst,
                    target: end,
                };
            }
        }
        Ok(dst)
    }
}

/// What an assignment target is, for diagnostics ("an array element", …).
fn lvalue_desc(e: &Expression) -> &'static str {
    match e {
        Expression::ArrayAccess(_) | Expression::ArrayAppend(_) => "an array element",
        Expression::Access(Access::Property(_)) => "a property",
        Expression::Access(Access::NullSafeProperty(_)) => "a nullsafe property",
        Expression::Access(Access::StaticProperty(_)) => "a static property",
        Expression::Variable(_) => "a variable",
        _ => "this target",
    }
}

/// Build the instruction for a compound-assignment operator `dst = a <op> b`;
/// `target` describes the assigned lvalue (see [`lvalue_desc`]).
fn compound_op(
    op: &AssignmentOperator,
    dst: Reg,
    a: Reg,
    b: Reg,
    target: &str,
) -> Result<Instr, Thrown> {
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
                format!("??= on {target}"),
                "use a full assignment for now",
            ))
        }
        Ao::Assign(_) => unreachable!("handled by caller"),
    })
}
