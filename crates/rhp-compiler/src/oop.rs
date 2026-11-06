//! Object expressions: class references, member names, property access and `new`.

use crate::parse::cst::{Access, ClassLikeMemberSelector, Expression, Variable};
use rhp_bytecode::{ClassRef, Instr, NIdx, Reg};
use rhp_values::{Thrown, Value};

use crate::assign::is_globals_var;
use crate::builder::ProtoBuilder;
use crate::builder::ScopeKind;
use crate::calls::{has_named, has_spread, ArgRefTarget};
use crate::decl::compile_anon_class;
use crate::fold::fold_const;
use crate::util::{compile_error, strip_dollar};

/// zend_zval_value_name of a compile-time constant.
pub(crate) fn value_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(true) => "true",
        Value::Bool(false) => "false",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        _ => "object",
    }
}

/// A compiled member selector: a static interned name, or a runtime name in a
/// register (dynamic `$o->$x` / `$o->{expr}`).
#[derive(Clone, Copy)]
pub(crate) enum Sel {
    Static(NIdx),
    Dyn(Reg),
}

impl ProtoBuilder<'_> {
    /// Resolve the class part of `new`/`::`/`instanceof` to a `ClassRef`.
    pub(crate) fn class_ref(&mut self, expr: &Expression) -> Result<ClassRef, Thrown> {
        match expr {
            Expression::Identifier(id) => Ok(ClassRef::Name(
                self.intern_name(&self.cx.resolve_class_name(id)),
            )),
            Expression::Self_(_) => {
                self.ensure_valid_class_fetch("self")?;
                Ok(ClassRef::SelfClass)
            }
            Expression::Static(_) => {
                self.ensure_valid_class_fetch("static")?;
                Ok(ClassRef::StaticClass)
            }
            Expression::Parent(_) => {
                self.ensure_valid_class_fetch("parent")?;
                Ok(ClassRef::ParentClass)
            }
            other => {
                // zend_compile_class_ref: a constant that is no string.
                if fold_const(self.cx, other).is_ok_and(|v| !matches!(v, Value::Str(_))) {
                    return Err(compile_error("Illegal class name", self.cur_line));
                }
                Ok(ClassRef::Dynamic(self.compile_expr(other)?))
            }
        }
    }

    /// `X::class` (zend_compile_class_name).
    fn compile_class_name(&mut self, class: &Expression) -> Result<Reg, Thrown> {
        let mut e = class;
        while let Expression::Parenthesized(p) = e {
            e = p.expression;
        }
        let class = match e {
            Expression::Identifier(id) => {
                let k = self.intern_const(Value::str(self.cx.resolve_class_name(id)));
                let dst = self.alloc_temp();
                self.emit(Instr::LoadConst { dst, k });
                return Ok(dst);
            }
            Expression::Self_(_) | Expression::Static(_) | Expression::Parent(_) => {
                self.class_ref(e)?
            }
            // zend_try_compile_const_expr_resolve_class_name: a string names
            // the class as written (one leading `\` dropped), or self/parent/
            // static.
            Expression::Literal(crate::parse::cst::Literal::String(s)) => {
                let name = s.value.unwrap_or(s.raw);
                let class = if name.eq_ignore_ascii_case(b"self") {
                    ClassRef::SelfClass
                } else if name.eq_ignore_ascii_case(b"parent") {
                    ClassRef::ParentClass
                } else if name.eq_ignore_ascii_case(b"static") {
                    ClassRef::StaticClass
                } else {
                    let name = name.strip_prefix(b"\\").unwrap_or(name);
                    let k = self.intern_const(Value::str(name.to_vec()));
                    let dst = self.alloc_temp();
                    self.emit(Instr::LoadConst { dst, k });
                    return Ok(dst);
                };
                let kind = std::str::from_utf8(name).unwrap_or("").to_ascii_lowercase();
                self.ensure_valid_class_fetch(&kind)?;
                class
            }
            Expression::Literal(_) => {
                return Err(compile_error("Illegal class name", self.cur_line))
            }
            other => {
                if let Ok(v) = fold_const(self.cx, other) {
                    return Err(compile_error(
                        format!("Cannot use \"::class\" on {}", value_name(&v)),
                        self.cur_line,
                    ));
                }
                ClassRef::Dynamic(self.compile_expr(other)?)
            }
        };
        let dst = self.alloc_temp();
        let const_expr = self.in_const_expr();
        self.emit(Instr::FetchClassName {
            dst,
            class,
            const_expr,
        });
        Ok(dst)
    }

    /// zend_is_scope_known: whether `self`/`parent` are known at compile time.
    pub(crate) fn scope_known(&self) -> bool {
        if self.scope_kind == ScopeKind::Closure {
            return false;
        }
        match self.cx.class_scope.borrow().as_ref() {
            Some(cs) => !cs.is_trait,
            None => matches!(self.scope_kind, ScopeKind::Function | ScopeKind::Method),
        }
    }

    /// zend_ensure_valid_class_fetch_type.
    pub(crate) fn ensure_valid_class_fetch(&self, kind: &str) -> Result<(), Thrown> {
        // Constant expressions check `X::class` only (constexpr.rs).
        if !self.scope_known() || self.in_const_expr() {
            return Ok(());
        }
        let msg = match self.cx.class_scope.borrow().as_ref() {
            None => format!("Cannot use \"{kind}\" when no class scope is active"),
            Some(cs) if kind == "parent" && cs.parent.is_none() => {
                "Cannot use \"parent\" when current class scope has no parent".to_string()
            }
            Some(_) => return Ok(()),
        };
        Err(compile_error(msg, self.cur_line))
    }

    /// A member selector as either a static interned name or a runtime name
    /// register (`$o->$x` / `$o->{expr}`).
    pub(crate) fn compile_selector(
        &mut self,
        sel: &crate::parse::cst::ClassLikeMemberSelector,
    ) -> Result<Sel, Thrown> {
        match sel {
            ClassLikeMemberSelector::Identifier(li) => Ok(Sel::Static(self.intern_name(li.value))),
            ClassLikeMemberSelector::Variable(Variable::Direct(d)) => {
                Ok(Sel::Dyn(match strip_dollar(d.name) {
                    b"GLOBALS" => {
                        let dst = self.alloc_temp();
                        self.emit(Instr::FetchGlobals { dst });
                        dst
                    }
                    b"this" => self.this_reg(),
                    n => self.cv(n),
                }))
            }
            ClassLikeMemberSelector::Variable(v) => Ok(Sel::Dyn(self.compile_variable(v)?)),
            ClassLikeMemberSelector::Expression(e) => {
                Ok(Sel::Dyn(self.compile_expr(e.expression)?))
            }
            _ => Err(Thrown::unsupported(
                "dynamic member name",
                "variable-variable member names deferred",
            )),
        }
    }

    /// The name of `C::$name` (static) or `C::${expr}` / `C::$$x` (runtime).
    pub(crate) fn static_prop_sel(&mut self, v: &Variable) -> Result<Sel, Thrown> {
        match v {
            Variable::Direct(d) => Ok(Sel::Static(self.intern_name(strip_dollar(d.name)))),
            other => Ok(Sel::Dyn(self.var_name_reg(other)?)),
        }
    }

    /// The register holding a variable-variable's *name*: `${expr}` is the
    /// expression's value, `$$x` the value of `$x` (recursively). For a direct
    /// `$x` (only reached as the root of `C::$x` lowering) the name itself.
    pub(crate) fn var_name_reg(&mut self, v: &Variable) -> Result<Reg, Thrown> {
        match v {
            Variable::Indirect(iv) => self.compile_expr(iv.expression),
            Variable::Nested(nv) => self.compile_variable(nv.variable),
            Variable::Direct(d) => Ok(self.load_string_const(strip_dollar(d.name))),
        }
    }

    /// A variable in value context: a CV, or a variable-variable read.
    pub(crate) fn compile_variable(&mut self, v: &Variable) -> Result<Reg, Thrown> {
        match v {
            Variable::Direct(d) => Ok(self.cv(strip_dollar(d.name))),
            other => {
                let name = self.var_name_reg(other)?;
                let dst = self.alloc_temp();
                self.emit(Instr::FetchVarVar { dst, name });
                Ok(dst)
            }
        }
    }

    /// `new class(args) extends P implements I { … }`: declare the class under
    /// its generated name and instantiate it.
    pub(crate) fn compile_new_anon(
        &mut self,
        ac: &crate::parse::cst::AnonymousClass,
    ) -> Result<Reg, Thrown> {
        use crate::parse::cst::PartialArgument;
        let proto = compile_anon_class(self.cx, ac)?;
        let class = ClassRef::Name(self.intern_name(proto.name.as_bytes()));
        self.cx.anon_classes.borrow_mut().push(proto);
        // With arguments, the object exists before they run (ZEND_NEW).
        let early = ac.argument_list.as_ref().is_some_and(|l| {
            !l.arguments.is_empty()
                && l.arguments
                    .iter()
                    .all(|a| matches!(a, PartialArgument::Positional(p) if p.ellipsis.is_none()))
        });
        let obj = early.then(|| {
            let dst = self.alloc_temp();
            let start = self.code.len() as rhp_bytecode::Pc;
            self.emit(Instr::NewObj { dst, class });
            (dst, start)
        });
        let mut regs = Vec::new();
        if let Some(list) = &ac.argument_list {
            for arg in list.arguments.iter() {
                match arg {
                    PartialArgument::Positional(p) if p.ellipsis.is_none() => {
                        regs.push(self.compile_expr(p.value)?)
                    }
                    PartialArgument::VariadicPlaceholder(_) => {
                        return Err(compile_error(
                            "Cannot create Closure for new expression",
                            self.cur_line,
                        ))
                    }
                    _ => {
                        return Err(Thrown::unsupported(
                            "anonymous class argument",
                            "spread / named / placeholder arguments deferred",
                        ))
                    }
                }
            }
        }
        let argc = regs.len() as u16;
        let base = self.alloc_temps(argc);
        for (i, r) in regs.into_iter().enumerate() {
            self.emit(Instr::Mov {
                dst: base + i as Reg,
                src: r,
            });
        }
        if let Some((dst, start)) = obj {
            self.new_ranges
                .push((start, self.code.len() as rhp_bytecode::Pc, dst));
            self.emit(Instr::NewCall { dst, base, argc });
            return Ok(dst);
        }
        let dst = self.alloc_temp();
        self.emit(Instr::New {
            dst,
            class,
            base,
            argc,
        });
        Ok(dst)
    }

    pub(crate) fn compile_new(
        &mut self,
        inst: &crate::parse::cst::Instantiation,
    ) -> Result<Reg, Thrown> {
        if let Expression::AnonymousClass(ac) = inst.class {
            return self.compile_new_anon(ac);
        }
        let class = self.class_ref(inst.class)?;
        let args = inst
            .argument_list
            .as_ref()
            .filter(|l| !l.arguments.is_empty());
        let Some(list) = args else {
            let dst = self.alloc_temp();
            self.emit(Instr::New {
                dst,
                class,
                base: 0,
                argc: 0,
            });
            return Ok(dst);
        };
        // ZEND_NEW creates the object before the arguments run (the ids
        // of nested `new`s follow from that).
        let dst = self.alloc_temp();
        let start = self.code.len() as rhp_bytecode::Pc;
        self.emit(Instr::NewObj { dst, class });
        if has_spread(list) || has_named(list) {
            let args = self.build_args_array(Some(list))?;
            self.new_ranges
                .push((start, self.code.len() as rhp_bytecode::Pc, dst));
            self.emit(Instr::NewCallV { dst, args });
            return Ok(dst);
        }
        let mask = self.method_ref_mask(b"__construct");
        let ctor = self.intern_name(b"__construct");
        let (base, argc) = self.compile_arg_window_t(
            Some(list),
            &mask,
            true,
            Some(ArgRefTarget::Static(class, ctor)),
        )?;
        self.new_ranges
            .push((start, self.code.len() as rhp_bytecode::Pc, dst));
        self.emit(Instr::NewCall { dst, base, argc });
        Ok(dst)
    }

    /// Compile `e` in php's BP_VAR_IS mode (an isset / empty / `??` operand):
    /// the property, static-property and dim fetches along its container chain
    /// are quiet (an inaccessible or missing property is null, no access
    /// error, `__isset` consulted before `__get`). Keys, dynamic member names
    /// and anything that is not a fetch compile normally.
    pub(crate) fn compile_quiet(&mut self, e: &Expression) -> Result<Reg, Thrown> {
        match e {
            Expression::Parenthesized(p) => self.compile_quiet(p.expression),
            Expression::ArrayAccess(aa) => {
                let arr = self.compile_quiet(aa.array)?;
                let key = self.compile_expr(aa.index)?;
                let dst = self.alloc_temp();
                self.emit(Instr::DimFetchIs { dst, arr, key });
                Ok(dst)
            }
            Expression::Access(Access::Property(pa)) => {
                let obj = self.compile_quiet(pa.object)?;
                let dst = self.alloc_temp();
                match self.compile_selector(&pa.property)? {
                    Sel::Static(name) => self.emit(Instr::FetchPropIs { dst, obj, name }),
                    Sel::Dyn(name) => self.emit(Instr::FetchPropDynIs { dst, obj, name }),
                }
                Ok(dst)
            }
            Expression::Access(Access::NullSafeProperty(npa)) => {
                let obj = self.compile_quiet(npa.object)?;
                let dst = self.alloc_temp();
                self.emit(Instr::MovIs { dst, src: obj });
                let jn = self.code.len();
                self.emit(Instr::Nop); // JmpNull dst -> end
                let t = self.alloc_temp();
                match self.compile_selector(&npa.property)? {
                    Sel::Static(name) => self.emit(Instr::FetchPropIs {
                        dst: t,
                        obj: dst,
                        name,
                    }),
                    Sel::Dyn(name) => self.emit(Instr::FetchPropDynIs {
                        dst: t,
                        obj: dst,
                        name,
                    }),
                }
                self.emit(Instr::Mov { dst, src: t });
                let end = self.here();
                self.code[jn] = Instr::JmpNull {
                    src: dst,
                    target: end,
                };
                Ok(dst)
            }
            Expression::Access(Access::StaticProperty(spa)) => {
                let class = self.class_ref(spa.class)?;
                let dst = self.alloc_temp();
                match self.static_prop_sel(&spa.property)? {
                    Sel::Static(name) => self.emit(Instr::FetchStaticPropIs { dst, class, name }),
                    Sel::Dyn(name) => self.emit(Instr::FetchStaticPropDynIs { dst, class, name }),
                }
                Ok(dst)
            }
            Expression::Variable(v @ (Variable::Indirect(_) | Variable::Nested(_))) => {
                let name = self.var_name_reg(v)?;
                let dst = self.alloc_temp();
                self.emit(Instr::FetchVarVarIs { dst, name });
                Ok(dst)
            }
            _ => self.compile_expr(e),
        }
    }

    /// The object of a property write: a property access or ArrayAccess
    /// element there is php's BP_VAR_W fetch (`$o->a->b = v` fetches
    /// `$o->a` for writing).
    pub(crate) fn compile_write_object(&mut self, e: &Expression) -> Result<Reg, Thrown> {
        // `$a[]->p = v`: the appended element is null, so the property write
        // fails ("Attempt to assign property on null"). The append itself is
        // not made.
        if let Expression::ArrayAppend(ap) = e {
            if !is_globals_var(ap.array) {
                self.compile_write_object(ap.array)?;
                let dst = self.alloc_temp();
                self.emit(Instr::LoadNull { dst });
                return Ok(dst);
            }
        }
        if let Expression::ArrayAccess(aa) = e {
            if !is_globals_var(aa.array) {
                let arr = self.compile_write_object(aa.array)?;
                let key = self.compile_expr(aa.index)?;
                let dst = self.alloc_temp();
                self.emit(Instr::DimFetchW { dst, arr, key });
                return Ok(dst);
            }
        }
        let Expression::Access(Access::Property(pa)) = e else {
            return self.compile_expr(e);
        };
        let obj = self.compile_write_object(pa.object)?;
        let dst = self.alloc_temp();
        match self.compile_selector(&pa.property)? {
            Sel::Static(name) => self.emit(Instr::FetchPropW { dst, obj, name }),
            Sel::Dyn(name) => self.emit(Instr::FetchPropDynW { dst, obj, name }),
        }
        Ok(dst)
    }

    /// The object of `unset($o->p)`: php fetches the properties along it
    /// with BP_VAR_UNSET, which `get_property_ptr_ptr` serves like a write
    /// (SimpleXML creates a missing element); elements are read.
    pub(crate) fn compile_unset_object(&mut self, e: &Expression) -> Result<Reg, Thrown> {
        match e {
            Expression::ArrayAccess(aa) if !is_globals_var(aa.array) => {
                let arr = self.compile_unset_object(aa.array)?;
                let key = self.compile_expr(aa.index)?;
                let dst = self.alloc_temp();
                self.emit(Instr::DimFetchUnset { dst, arr, key });
                Ok(dst)
            }
            Expression::Access(Access::Property(pa)) => {
                let obj = self.compile_unset_object(pa.object)?;
                let dst = self.alloc_temp();
                match self.compile_selector(&pa.property)? {
                    Sel::Static(name) => self.emit(Instr::FetchPropW { dst, obj, name }),
                    Sel::Dyn(name) => self.emit(Instr::FetchPropDynW { dst, obj, name }),
                }
                Ok(dst)
            }
            _ => self.compile_expr(e),
        }
    }

    fn fetch_prop(&mut self, obj: Reg, sel: &ClassLikeMemberSelector) -> Result<Reg, Thrown> {
        let sel = self.compile_selector(sel)?;
        if self.in_const_expr() {
            let name = match sel {
                Sel::Dyn(r) => r,
                Sel::Static(_) => Reg::MAX,
            };
            self.emit(Instr::ConstExprProp { obj, name });
        }
        let dst = self.alloc_temp();
        match sel {
            Sel::Static(name) => self.emit(Instr::FetchProp { dst, obj, name }),
            Sel::Dyn(name) => self.emit(Instr::FetchPropDyn { dst, obj, name }),
        }
        Ok(dst)
    }

    /// Compiling a constant expression (an initializer thunk, a parameter
    /// default or a `const` value).
    pub(crate) fn in_const_expr(&self) -> bool {
        self.scope_kind == ScopeKind::Init || self.in_const_decl
    }

    pub(crate) fn compile_access(&mut self, acc: &Access) -> Result<Reg, Thrown> {
        match acc {
            Access::Property(pa) => {
                let obj = self.compile_expr(pa.object)?;
                self.fetch_prop(obj, &pa.property)
            }
            Access::NullSafeProperty(npa) => {
                // `$o?->p`: null short-circuits the whole access to null.
                // The chain's outermost link sets the result to null.
                let obj = self.compile_expr(npa.object)?;
                self.nullsafe_guard(obj);
                self.fetch_prop(obj, &npa.property)
            }
            Access::StaticProperty(spa) => {
                let class = self.class_ref(spa.class)?;
                let dst = self.alloc_temp();
                match self.static_prop_sel(&spa.property)? {
                    Sel::Static(name) => {
                        self.emit(Instr::FetchStaticProp { dst, class, name });
                    }
                    Sel::Dyn(name) => self.emit(Instr::FetchStaticPropDyn { dst, class, name }),
                }
                Ok(dst)
            }
            Access::ClassConstant(cca) => {
                // `Name::class` is a COMPILE-TIME fully-qualified-name string — the
                // class need not exist (php uses `Foo::class` as a bareword FQN,
                // e.g. `function_exists(u::class)`). `self/static/parent::class`
                // and `$obj::class` stay runtime.
                if let crate::parse::cst::ClassLikeConstantSelector::Identifier(li) = &cca.constant
                {
                    if li.value.eq_ignore_ascii_case(b"class") {
                        return self.compile_class_name(cca.class);
                    }
                }
                let class = self.class_ref(cca.class)?;
                let name = match &cca.constant {
                    crate::parse::cst::ClassLikeConstantSelector::Identifier(li) => {
                        self.intern_name(li.value)
                    }
                    crate::parse::cst::ClassLikeConstantSelector::Expression(e) => {
                        let name = self.compile_expr(e.expression)?;
                        let dst = self.alloc_temp();
                        self.emit(Instr::FetchClassConstDyn { dst, class, name });
                        return Ok(dst);
                    }
                    _ => return Err(Thrown::unsupported("dynamic class constant", "deferred")),
                };
                let dst = self.alloc_temp();
                self.emit(Instr::FetchClassConst { dst, class, name });
                Ok(dst)
            }
        }
    }
}
