//! The CV pre-pass: one register per named variable, created in the order php creates them.

use rhp_frontend::cst::{
    Access, Argument, ArrayElement, CompositeString, Construct, Expression, ForeachBody,
    ForeachTarget, Node, Statement, StringPart, Variable,
};

use crate::builder::ProtoBuilder;
use crate::util::strip_dollar;

impl ProtoBuilder<'_> {
    pub(crate) fn scan_stmt_vars(&mut self, stmt: &Statement) {
        use rhp_frontend::cst::{ForBody, IfBody, SwitchBody, SwitchCase, WhileBody};
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
                f.initializations
                    .iter()
                    .for_each(|e| self.scan_expr_vars(e));
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
            Statement::Global(g) => {
                for v in g.variables.iter() {
                    if let Variable::Direct(d) = v {
                        self.cv(strip_dollar(d.name));
                    }
                }
            }
            Statement::Static(s) => {
                for item in s.items.iter() {
                    self.cv(strip_dollar(item.variable().name));
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
            _ => {}
        }
    }

    pub(crate) fn scan_expr_vars(&mut self, expr: &Expression) {
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
                        rhp_frontend::cst::MatchArm::Expression(a) => {
                            a.conditions.iter().for_each(|c| self.scan_expr_vars(c));
                            self.scan_expr_vars(a.expression);
                        }
                        rhp_frontend::cst::MatchArm::Default(a) => {
                            self.scan_expr_vars(a.expression)
                        }
                    }
                }
            }
            Expression::Instantiation(inst) => {
                self.scan_expr_vars(inst.class);
                if let Some(list) = &inst.argument_list {
                    self.scan_args(list);
                }
            }
            Expression::Access(acc) => match acc {
                Access::Property(pa) => self.scan_expr_vars(pa.object),
                Access::NullSafeProperty(pa) => self.scan_expr_vars(pa.object),
                // Note: in `C::$x` the `$x` is the static-property NAME, not a local.
                Access::StaticProperty(spa) => self.scan_expr_vars(spa.class),
                Access::ClassConstant(cca) => self.scan_expr_vars(cca.class),
            },
            Expression::Clone(cl) => self.scan_expr_vars(cl.object),
            Expression::Throw(t) => self.scan_expr_vars(t.exception),
            Expression::Construct(c) => self.scan_construct_vars(c),
            Expression::Call(call) => {
                use rhp_frontend::cst::Call;
                match call {
                    Call::Function(fc) => self.scan_args(&fc.argument_list),
                    Call::Method(mc) => {
                        self.scan_expr_vars(mc.object);
                        self.scan_args(&mc.argument_list);
                    }
                    Call::NullSafeMethod(mc) => {
                        self.scan_expr_vars(mc.object);
                        self.scan_args(&mc.argument_list);
                    }
                    Call::StaticMethod(smc) => {
                        self.scan_expr_vars(smc.class);
                        self.scan_args(&smc.argument_list);
                    }
                }
            }
            _ => {}
        }
    }

    fn scan_args(&mut self, list: &rhp_frontend::cst::ArgumentList) {
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

    /// Completeness backstop for the CV pre-pass: walk every node of the body
    /// (not entering nested function/class scopes, whose variables are their
    /// own) and give each `$name` still missing a slot. A closure contributes
    /// its `use` variables; an arrow function nothing (it captures only
    /// variables the enclosing scope already has).
    pub(crate) fn scan_node_vars(&mut self, node: Node) {
        match node {
            Node::DirectVariable(d) => {
                self.cv(strip_dollar(d.name));
                return;
            }
            Node::Closure(cl) => {
                if let Some(u) = &cl.use_clause {
                    for v in u.variables.iter() {
                        self.cv(strip_dollar(v.variable.name));
                    }
                }
                return;
            }
            Node::AnonymousClass(ac) => {
                if let Some(args) = &ac.argument_list {
                    self.scan_node_vars(Node::PartialArgumentList(args));
                }
                return;
            }
            // `C::$p`: the `$p` is the property name, not a local.
            Node::StaticPropertyAccess(spa) => {
                self.scan_node_vars(Node::Expression(spa.class));
                return;
            }
            Node::ArrowFunction(_)
            | Node::Function(_)
            | Node::Class(_)
            | Node::Interface(_)
            | Node::Trait(_)
            | Node::Enum(_)
            | Node::AttributeList(_) => return,
            _ => {}
        }
        node.visit_children(|ch| self.scan_node_vars(ch));
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

/// Read-only collection of every `$var` name referenced in an expression, for
/// arrow-function auto-capture. Descends into nested closures/arrows so their
/// free variables propagate upward (over-collection is harmless: the caller
/// keeps only names that are CVs in the enclosing scope). Does not mutate the
/// builder, unlike [`ProtoBuilder::scan_expr_vars`].
pub(crate) fn collect_vars(expr: &Expression, out: &mut Vec<Vec<u8>>) {
    collect_node_vars(Node::Expression(expr), out);
}

/// [`collect_vars`] over any node: every `$name` below it (in source order,
/// deduplicated), entering nested arrow functions but not other function or
/// class scopes; a closure contributes its `use` variables.
fn collect_node_vars(node: Node, out: &mut Vec<Vec<u8>>) {
    let push = |name: &[u8], out: &mut Vec<Vec<u8>>| {
        if !out.iter().any(|n| n == name) {
            out.push(name.to_vec());
        }
    };
    match node {
        Node::DirectVariable(d) => {
            push(strip_dollar(d.name), out);
            return;
        }
        Node::Closure(cl) => {
            if let Some(uses) = &cl.use_clause {
                for u in uses.variables.iter() {
                    push(strip_dollar(u.variable.name), out);
                }
            }
            return;
        }
        Node::AnonymousClass(ac) => {
            if let Some(args) = &ac.argument_list {
                collect_node_vars(Node::PartialArgumentList(args), out);
            }
            return;
        }
        // `C::$p`: the `$p` is the property name, not a variable.
        Node::StaticPropertyAccess(spa) => {
            collect_node_vars(Node::Expression(spa.class), out);
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
    node.visit_children(|ch| collect_node_vars(ch, out));
}
