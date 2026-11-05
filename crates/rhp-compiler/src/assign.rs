//! Assignments and lvalues: destructuring, references, compound and `??=` assignment.

use crate::parse::cst::{
    Access, ArrayElement, AssignmentOperator, ClassLikeMemberSelector, Expression,
    UnaryPrefixOperator, Variable,
};
use rhp_bytecode::{ClassRef, Instr, NIdx, Reg};
use rhp_values::Thrown;

use crate::builder::ProtoBuilder;
use crate::oop::Sel;
use crate::util::strip_dollar;

/// A step in an lvalue path (compiler-side; key regs are pre-evaluated).
pub(crate) enum CursorStep {
    Key(Reg),
    Append,
    Prop(NIdx),
    PropDyn(Reg),
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
pub(crate) fn check_globals_write(target: &Expression) -> Result<(), Thrown> {
    match target {
        e if is_globals_var(e) => Err(Thrown::error(
            "Error",
            "$GLOBALS can only be modified using the $GLOBALS[$name] = $value syntax",
        )),
        Expression::ArrayAppend(ap) if is_globals_var(ap.array) => {
            Err(Thrown::error("Error", "Cannot append to $GLOBALS"))
        }
        Expression::ArrayAccess(aa) if is_globals_var(aa.array) => Ok(()),
        Expression::ArrayAccess(aa) => check_globals_write(aa.array),
        Expression::ArrayAppend(ap) => check_globals_write(ap.array),
        Expression::Parenthesized(p) => check_globals_write(p.expression),
        _ => Ok(()),
    }
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
    /// Assign the elements of `src` (an array value) to a destructuring pattern.
    pub(crate) fn destructure_into<'a>(
        &mut self,
        elements: impl Iterator<Item = &'a ArrayElement<'a>>,
        src: Reg,
    ) -> Result<(), Thrown> {
        let mut idx: i64 = 0;
        for el in elements {
            match el {
                ArrayElement::Missing(_) => idx += 1,
                ArrayElement::Value(v) => {
                    let key = self.alloc_temp();
                    self.emit(Instr::LoadInt { dst: key, n: idx });
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
                    let key = self.compile_expr(kv.key)?;
                    let val = self.alloc_temp();
                    self.emit(Instr::DimFetchList {
                        dst: val,
                        arr: src,
                        key,
                    });
                    self.store_to_target(kv.value, val)?;
                }
                ArrayElement::Variadic(_) => {
                    return Err(Thrown::unsupported("spread in destructuring", "deferred"))
                }
            }
        }
        Ok(())
    }

    /// Store `src` into a single destructuring target (variable, nested pattern,
    /// array element, or property).
    pub(crate) fn store_to_target(&mut self, target: &Expression, src: Reg) -> Result<(), Thrown> {
        check_globals_write(target)?;
        match target {
            Expression::Variable(Variable::Direct(d)) => {
                let cv = self.cv(strip_dollar(d.name));
                self.emit(Instr::Assign { cv, src });
            }
            Expression::Array(arr) => self.destructure_into(arr.elements.iter(), src)?,
            Expression::List(lst) => self.destructure_into(lst.elements.iter(), src)?,
            Expression::ArrayAccess(_)
            | Expression::ArrayAppend(_)
            | Expression::Variable(_)
            | Expression::Access(Access::Property(_))
            | Expression::Access(Access::StaticProperty(_)) => {
                let mut steps = Vec::new();
                let root = self.collect_lvalue_steps(target, &mut steps)?;
                self.emit_cursor(root, &steps);
                self.emit(Instr::AssignLval { src });
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
            _ => Err(Thrown::error(
                "Error",
                "Cannot use temporary expression in write context",
            )),
        }
    }

    pub(crate) fn emit_cursor(&mut self, root: LvalRoot, steps: &[CursorStep]) {
        match root {
            LvalRoot::Cv(cv) => self.emit(Instr::LvalCv { cv }),
            LvalRoot::StaticProp(class, name) => self.emit(Instr::LvalStaticProp { class, name }),
            LvalRoot::StaticPropDyn(class, name) => {
                self.emit(Instr::LvalStaticPropDyn { class, name })
            }
            LvalRoot::VarVar(name) => self.emit(Instr::LvalVarVar { name }),
            LvalRoot::Global(name) => self.emit(Instr::LvalGlobal { name }),
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
            return Err(Thrown::error(
                "Error",
                "Cannot acquire reference to $GLOBALS",
            ));
        }
        check_globals_write(src)?;
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
            // `$x = &func()` / other non-lvalue source: we don't implement
            // return-by-reference, so bind by value (matches php's fallback for a
            // non-ref-returning callee; object handles are shared regardless).
            other => self.compile_expr(other),
        }
    }

    pub(crate) fn compile_assignment(
        &mut self,
        a: &crate::parse::cst::Assignment,
    ) -> Result<Reg, Thrown> {
        check_globals_write(a.lhs)?;
        // Destructuring: `[$a, $b] = ...` / `list($a, $b) = ...`.
        if matches!(a.operator, AssignmentOperator::Assign(_)) {
            if let Expression::Array(arr) = a.lhs {
                let rhs = self.compile_expr(a.rhs)?;
                self.destructure_into(arr.elements.iter(), rhs)?;
                return Ok(rhs);
            }
            if let Expression::List(lst) = a.lhs {
                let rhs = self.compile_expr(a.rhs)?;
                self.destructure_into(lst.elements.iter(), rhs)?;
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
                        return Err(Thrown::error(
                            "Error",
                            "Cannot acquire reference to $GLOBALS",
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
                    self.emit(Instr::AssignLval { src: rhs });
                    Ok(rhs)
                }
                op => {
                    self.emit_cursor(root, &steps);
                    let cur = self.alloc_temp();
                    self.emit(Instr::FetchLvalRw { dst: cur });
                    let res = self.alloc_temp();
                    self.emit(compound_op(op, res, cur, rhs, lvalue_desc(a.lhs))?);
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
                self.emit(Instr::Mov { dst, src: cv });
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
                        self.emit(Instr::FetchVarVar {
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
                            return Err(Thrown::error("Error", "Cannot use [] for reading"))
                        }
                    }
                    cur = t;
                }
                self.emit(Instr::Mov { dst, src: cur });
                let jnn = self.code.len();
                self.emit(Instr::Nop);
                let r = self.compile_expr(rhs)?;
                self.emit_cursor(root, &steps);
                self.emit(Instr::AssignLval { src: r });
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
