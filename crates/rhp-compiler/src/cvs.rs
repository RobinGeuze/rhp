//! The CV pre-pass: one register per named variable, created in the order php creates them.

use crate::parse::cst::{
    Access, Argument, ArrayElement, AssignmentOperator, ClassLikeConstantSelector,
    ClassLikeMemberSelector, CompositeString, Construct, Expression, ForeachBody, ForeachTarget,
    IndirectVariable, Literal, Node, PartialArgument, Statement, StringPart, Variable,
};

use crate::builder::ProtoBuilder;
use crate::util::{is_superglobal, strip_dollar};

impl ProtoBuilder<'_> {
    /// Give each variable of `stmt` its CV, in the order php's compiler
    /// first reaches it (which is the order of `$GLOBALS`, and of the
    /// destructor calls at shutdown, for the script's own variables).
    pub(crate) fn scan_stmt_vars(&mut self, stmt: &Statement) {
        use crate::parse::cst::{ForBody, IfBody, SwitchBody, SwitchCase, WhileBody};
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
            // php compiles a loop's body before its condition, which sits at
            // the bottom.
            Statement::While(w) => {
                match &w.body {
                    WhileBody::Statement(s) => self.scan_stmt_vars(s),
                    WhileBody::ColonDelimited(b) => {
                        b.statements.iter().for_each(|s| self.scan_stmt_vars(s))
                    }
                }
                self.scan_expr_vars(w.condition);
            }
            Statement::DoWhile(d) => {
                self.scan_stmt_vars(d.statement);
                self.scan_expr_vars(d.condition);
            }
            Statement::For(f) => {
                f.initializations
                    .iter()
                    .for_each(|e| self.scan_expr_vars(e));
                match &f.body {
                    ForBody::Statement(s) => self.scan_stmt_vars(s),
                    ForBody::ColonDelimited(b) => {
                        b.statements.iter().for_each(|s| self.scan_stmt_vars(s))
                    }
                }
                f.increments.iter().for_each(|e| self.scan_expr_vars(e));
                f.conditions.iter().for_each(|e| self.scan_expr_vars(e));
            }
            Statement::Switch(sw) => {
                self.scan_expr_vars(sw.expression);
                let cases = match &sw.body {
                    SwitchBody::BraceDelimited(b) => &b.cases,
                    SwitchBody::ColonDelimited(b) => &b.cases,
                };
                // Every case condition, then the case bodies.
                for case in cases.iter() {
                    if let SwitchCase::Expression(c) = case {
                        self.scan_expr_vars(c.expression);
                    }
                }
                for case in cases.iter() {
                    let stmts = match case {
                        SwitchCase::Expression(c) => &c.statements,
                        SwitchCase::Default(c) => &c.statements,
                    };
                    stmts.iter().for_each(|s| self.scan_stmt_vars(s));
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
                    // php compiles the value variable before the key's.
                    ForeachTarget::KeyValue(t) => {
                        self.scan_expr_vars(t.value);
                        self.scan_expr_vars(t.key);
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
            Statement::Global(g) => g.variables.iter().for_each(|v| self.scan_variable(v)),
            Statement::Static(s) => {
                for item in s.items.iter() {
                    self.cv(strip_dollar(item.variable().name));
                    if let crate::parse::cst::StaticItem::Concrete(c) = item {
                        self.scan_expr_vars(c.value);
                    }
                }
            }
            Statement::Try(t) => {
                t.block
                    .statements
                    .iter()
                    .for_each(|s| self.scan_stmt_vars(s));
                for clause in t.catch_clauses.iter() {
                    if let Some(v) = &clause.variable {
                        self.cv(strip_dollar(v.name));
                    }
                    clause
                        .block
                        .statements
                        .iter()
                        .for_each(|s| self.scan_stmt_vars(s));
                }
                if let Some(fin) = &t.finally_clause {
                    fin.block
                        .statements
                        .iter()
                        .for_each(|s| self.scan_stmt_vars(s));
                }
            }
            Statement::Declare(d) => {
                use crate::parse::cst::DeclareBody;
                match &d.body {
                    DeclareBody::Statement(s) => self.scan_stmt_vars(s),
                    DeclareBody::ColonDelimited(c) => {
                        c.statements.iter().for_each(|s| self.scan_stmt_vars(s))
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn scan_expr_vars(&mut self, expr: &Expression) {
        match expr {
            Expression::Variable(v) => self.scan_variable(v),
            Expression::Parenthesized(p) => self.scan_expr_vars(p.expression),
            Expression::Binary(b) => {
                self.scan_expr_vars(b.lhs);
                self.scan_expr_vars(b.rhs);
            }
            Expression::UnaryPrefix(u) => self.scan_expr_vars(u.operand),
            Expression::UnaryPostfix(u) => self.scan_expr_vars(u.operand),
            // php compiles a list() assignment's right-hand side first.
            Expression::Assignment(a)
                if matches!(a.operator, AssignmentOperator::Assign(_))
                    && matches!(a.lhs, Expression::Array(_) | Expression::List(_)) =>
            {
                self.scan_expr_vars(a.rhs);
                self.scan_expr_vars(a.lhs);
            }
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
            Expression::List(lst) => self.scan_array_vars(lst.elements.iter()),
            Expression::ArrayAccess(aa) => {
                self.scan_expr_vars(aa.array);
                self.scan_expr_vars(aa.index);
            }
            Expression::ArrayAppend(ap) => self.scan_expr_vars(ap.array),
            Expression::CompositeString(cs) => {
                let parts = match cs {
                    CompositeString::Interpolated(i) => Some(&i.parts),
                    CompositeString::Document(d) => Some(&d.parts),
                    CompositeString::ShellExecute(s) => Some(&s.parts),
                };
                if let Some(parts) = parts {
                    for part in parts.iter() {
                        match part {
                            StringPart::Expression(Expression::Variable(Variable::Indirect(
                                iv,
                            ))) => {
                                let src = self.cx.src;
                                match crate::strings::dollar_brace_varname(
                                    src,
                                    iv.dollar_left_brace.end.offset as usize,
                                ) {
                                    Some(name) => {
                                        if name != b"this" && name != b"GLOBALS" {
                                            self.cv(name);
                                        }
                                        if let Expression::ArrayAccess(aa) = iv.expression {
                                            self.scan_expr_vars(aa.index);
                                        }
                                    }
                                    None => self.scan_expr_vars(&Expression::Variable(
                                        Variable::Indirect(iv.clone()),
                                    )),
                                }
                            }
                            StringPart::Expression(e) => self.scan_expr_vars(e),
                            StringPart::BracedExpression(b) => self.scan_expr_vars(b.expression),
                            StringPart::Literal(_) => {}
                        }
                    }
                }
            }
            // Every arm's conditions, then the arm bodies.
            Expression::Match(m) => {
                use crate::parse::cst::MatchArm;
                self.scan_expr_vars(m.expression);
                for arm in m.arms.iter() {
                    if let MatchArm::Expression(a) = arm {
                        a.conditions.iter().for_each(|c| self.scan_expr_vars(c));
                    }
                }
                for arm in m.arms.iter() {
                    match arm {
                        MatchArm::Expression(a) => self.scan_expr_vars(a.expression),
                        MatchArm::Default(a) => self.scan_expr_vars(a.expression),
                    }
                }
            }
            Expression::Instantiation(inst) => {
                self.scan_expr_vars(inst.class);
                if let Some(list) = &inst.argument_list {
                    self.scan_args(list);
                }
            }
            Expression::AnonymousClass(ac) => {
                if let Some(list) = &ac.argument_list {
                    self.scan_partial_args(list);
                }
            }
            // A member name after its object or class.
            Expression::Access(acc) => match acc {
                Access::Property(pa) => {
                    self.scan_expr_vars(pa.object);
                    self.scan_selector(&pa.property);
                }
                Access::NullSafeProperty(pa) => {
                    self.scan_expr_vars(pa.object);
                    self.scan_selector(&pa.property);
                }
                // In `C::$x` the `$x` is the property's name, not a variable;
                // `C::$$x` reads `$x`.
                Access::StaticProperty(spa) => {
                    self.scan_expr_vars(spa.class);
                    if !matches!(spa.property, Variable::Direct(_)) {
                        self.scan_variable(&spa.property);
                    }
                }
                Access::ClassConstant(cca) => {
                    self.scan_expr_vars(cca.class);
                    if let ClassLikeConstantSelector::Expression(e) = &cca.constant {
                        self.scan_expr_vars(e.expression);
                    }
                }
            },
            Expression::Clone(cl) => self.scan_expr_vars(cl.object),
            Expression::Throw(t) => self.scan_expr_vars(t.exception),
            Expression::Construct(c) => self.scan_construct_vars(c),
            // The callee (or object and method name) before the arguments.
            Expression::Call(call) => {
                use crate::parse::cst::Call;
                match call {
                    Call::Function(fc) => {
                        self.scan_expr_vars(fc.function);
                        self.scan_args(&fc.argument_list);
                    }
                    Call::Method(mc) => {
                        self.scan_expr_vars(mc.object);
                        self.scan_selector(&mc.method);
                        self.scan_args(&mc.argument_list);
                    }
                    Call::NullSafeMethod(mc) => {
                        self.scan_expr_vars(mc.object);
                        self.scan_selector(&mc.method);
                        self.scan_args(&mc.argument_list);
                    }
                    Call::StaticMethod(smc) => {
                        self.scan_expr_vars(smc.class);
                        self.scan_selector(&smc.method);
                        self.scan_args(&smc.argument_list);
                    }
                }
            }
            Expression::PartialApplication(pa) => {
                use crate::parse::cst::PartialApplication as Pa;
                match pa {
                    Pa::Function(f) => {
                        self.scan_expr_vars(f.function);
                        self.scan_partial_args(&f.argument_list);
                    }
                    Pa::Method(m) => {
                        self.scan_expr_vars(m.object);
                        self.scan_selector(&m.method);
                        self.scan_partial_args(&m.argument_list);
                    }
                    Pa::StaticMethod(s) => {
                        self.scan_expr_vars(s.class);
                        self.scan_selector(&s.method);
                        self.scan_partial_args(&s.argument_list);
                    }
                }
            }
            // A closure binds its use() variables where it is created.
            Expression::Closure(cl) => {
                if let Some(uses) = &cl.use_clause {
                    for u in uses.variables.iter() {
                        self.cv(strip_dollar(u.variable.name));
                    }
                }
            }
            // An arrow function binds every variable its body uses (php's
            // find_implicit_binds) but its parameters, `$this` and the
            // superglobals.
            Expression::ArrowFunction(af) => {
                let mut used = Vec::new();
                collect_vars(af.expression, self.cx.src, &mut used);
                for name in used {
                    let param = af
                        .parameter_list
                        .parameters
                        .iter()
                        .any(|p| strip_dollar(p.variable.name) == name.as_slice());
                    if !param && name != b"this" && !is_superglobal(&name) {
                        self.cv(&name);
                    }
                }
            }
            Expression::Yield(y) => {
                use crate::parse::cst::Yield;
                match y {
                    Yield::Value(v) => {
                        if let Some(e) = v.value {
                            self.scan_expr_vars(e);
                        }
                    }
                    Yield::Pair(p) => {
                        self.scan_expr_vars(p.key);
                        self.scan_expr_vars(p.value);
                    }
                    Yield::From(f) => self.scan_expr_vars(f.iterator),
                }
            }
            _ => {}
        }
    }

    /// `$x` and `${'x'}` are CVs; `${expr}` and `$$x` read the variables of
    /// their name.
    fn scan_variable(&mut self, v: &Variable) {
        match v {
            Variable::Direct(d) => {
                self.cv(strip_dollar(d.name));
            }
            Variable::Indirect(iv) => match literal_var_name(iv) {
                Some(name) => {
                    self.cv(name);
                }
                None => self.scan_expr_vars(iv.expression),
            },
            Variable::Nested(nv) => self.scan_variable(nv.variable),
        }
    }

    /// A dynamic member name: `->$x` / `->{expr}`.
    fn scan_selector(&mut self, sel: &ClassLikeMemberSelector) {
        match sel {
            ClassLikeMemberSelector::Variable(v) => self.scan_variable(v),
            ClassLikeMemberSelector::Expression(e) => self.scan_expr_vars(e.expression),
            ClassLikeMemberSelector::Identifier(_) | ClassLikeMemberSelector::Missing(_) => {}
        }
    }

    fn scan_partial_args(&mut self, list: &crate::parse::cst::PartialArgumentList) {
        for arg in list.arguments.iter() {
            match arg {
                PartialArgument::Positional(p) => self.scan_expr_vars(p.value),
                PartialArgument::Named(n) => self.scan_expr_vars(n.value),
                _ => {}
            }
        }
    }

    fn scan_args(&mut self, list: &crate::parse::cst::ArgumentList) {
        for arg in list.arguments.iter() {
            match arg {
                Argument::Positional(p) => self.scan_expr_vars(p.value),
                Argument::Named(n) => self.scan_expr_vars(n.value),
            }
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
            Construct::Include(i) => self.scan_expr_vars(i.value),
            Construct::IncludeOnce(i) => self.scan_expr_vars(i.value),
            Construct::Require(i) => self.scan_expr_vars(i.value),
            Construct::RequireOnce(i) => self.scan_expr_vars(i.value),
            Construct::Eval(e) => self.scan_expr_vars(e.value),
            Construct::Exit(e) => {
                if let Some(list) = &e.arguments {
                    self.scan_args(list);
                }
            }
            Construct::Die(d) => {
                if let Some(list) = &d.arguments {
                    self.scan_args(list);
                }
            }
        }
    }

    /// Completeness backstop for the CV pre-pass: give each `$name` of the
    /// body still missing a slot one (see [`visit_vars`]).
    pub(crate) fn scan_node_vars(&mut self, node: Node) {
        let src = self.cx.src;
        visit_vars(node, false, src, &mut |name| {
            self.cv(name);
        });
    }

    /// End the CV pre-pass: CVs occupy `[0, num_cvs)`, temporaries follow.
    pub(crate) fn freeze_cvs(&mut self) {
        self.num_cvs = self.cv_map.len() as u16;
        self.next_temp = self.num_cvs;
        self.high_water = self.num_cvs;
        self.temp_floor = self.num_cvs;
        self.cvs_frozen = true;
    }
}

/// The name of `${'name'}`: php's compiler makes a constant string name the
/// CV itself (zend_try_compile_cv), except `$this` and the superglobals.
fn literal_var_name<'a>(iv: &IndirectVariable<'a>) -> Option<&'a [u8]> {
    let mut e = iv.expression;
    while let Expression::Parenthesized(p) = e {
        e = p.expression;
    }
    match e {
        Expression::Literal(Literal::String(s)) => {
            s.value.filter(|n| *n != b"this" && !is_superglobal(n))
        }
        _ => None,
    }
}

/// Every `$var` name an arrow function's body uses (deduplicated, in source
/// order): nested arrow functions' bodies included, so their free variables
/// propagate upward.
pub(crate) fn collect_vars(expr: &Expression, src: &[u8], out: &mut Vec<Vec<u8>>) {
    visit_vars(Node::Expression(expr), true, src, &mut |name| {
        if !out.iter().any(|n| n == name) {
            out.push(name.to_vec());
        }
    });
}

/// Call `f` with every `$name` below `node`, in source order, without
/// entering the scopes that own their variables (functions, classes,
/// attributes, arrow functions): a closure contributes its `use` variables,
/// an anonymous class its constructor arguments, and with `arrows` a nested
/// arrow function its body. `src` spots `"${name}"` (the scanner's
/// T_STRING_VARNAME, which the tree keeps as `${name}`).
fn visit_vars(node: Node, arrows: bool, src: &[u8], f: &mut impl FnMut(&[u8])) {
    match node {
        Node::StringPart(StringPart::Expression(Expression::Variable(Variable::Indirect(iv)))) => {
            if let Some(name) =
                crate::strings::dollar_brace_varname(src, iv.dollar_left_brace.end.offset as usize)
            {
                if name != b"this" && name != b"GLOBALS" {
                    f(name);
                }
                if let Expression::ArrayAccess(aa) = iv.expression {
                    visit_vars(Node::Expression(aa.index), arrows, src, f);
                }
                return;
            }
        }
        Node::DirectVariable(d) => {
            f(strip_dollar(d.name));
            return;
        }
        Node::Closure(cl) => {
            if let Some(uses) = &cl.use_clause {
                for u in uses.variables.iter() {
                    f(strip_dollar(u.variable.name));
                }
            }
            return;
        }
        Node::AnonymousClass(ac) => {
            if let Some(args) = &ac.argument_list {
                visit_vars(Node::PartialArgumentList(args), arrows, src, f);
            }
            return;
        }
        Node::IndirectVariable(iv) => {
            if let Some(name) = literal_var_name(iv) {
                f(name);
                return;
            }
        }
        // `C::$p`: the `$p` is the property name, not a variable; `C::$$p`
        // reads `$p`.
        Node::StaticPropertyAccess(spa) => {
            visit_vars(Node::Expression(spa.class), arrows, src, f);
            if !matches!(spa.property, Variable::Direct(_)) {
                visit_vars(Node::Variable(&spa.property), arrows, src, f);
            }
            return;
        }
        // php's find_implicit_binds: a nested arrow function's body.
        Node::ArrowFunction(af) => {
            if arrows {
                visit_vars(Node::Expression(af.expression), arrows, src, f);
            }
            return;
        }
        Node::Function(_)
        | Node::Class(_)
        | Node::Interface(_)
        | Node::Trait(_)
        | Node::Enum(_)
        | Node::AttributeList(_) => return,
        _ => {}
    }
    node.visit_children(|ch| visit_vars(ch, arrows, src, f));
}
