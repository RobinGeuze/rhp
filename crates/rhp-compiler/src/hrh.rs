//! php 8.5's compile-time deprecation of the `$http_response_header`
//! variable (zend_compile.c, `has_assigned_to_http_response_header`): a
//! read of it warns unless the same function assigned it (or has a
//! parameter of that name) earlier in source order. Variables php compiles
//! for writing set the flag; reads (BP_VAR_R) warn; isset/unset, compound
//! assignments and arguments of functions php does not know at compile
//! time (SEND_FUNC_ARG) do neither.

use mago_span::HasSpan;
use rhp_frontend::cst::{
    Access, Argument, ArgumentList, ArrayElement, AssignmentOperator, Expression, ForeachTarget,
    Node, Statement, UnaryPrefixOperator, Variable,
};

const NAME: &[u8] = b"$http_response_header";
pub(crate) const MESSAGE: &str = "The predefined locally scoped $http_response_header variable is deprecated, call http_get_last_response_headers() instead";

/// How php compiles a variable at this position.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Read,
    Write,
    /// Neither warns nor sets the flag (RW, IS, UNSET, FUNC_ARG).
    Other,
}

struct Scan {
    /// One flag per op_array being compiled (innermost last).
    assigned: Vec<bool>,
    /// The lines of the deprecations, in source order.
    lines: Vec<u32>,
    line_of: fn(u32) -> u32,
}

/// The lines at which php's compiler warns, for a file's statements.
pub(crate) fn scan(stmts: &[&Statement], line_of: fn(u32) -> u32) -> Vec<u32> {
    let mut s = Scan {
        assigned: vec![false],
        lines: Vec::new(),
        line_of,
    };
    for st in stmts {
        s.node(Node::Statement(st));
    }
    s.lines
}

fn is_target(e: &Expression) -> bool {
    matches!(e, Expression::Variable(Variable::Direct(d)) if d.name == NAME)
}

impl Scan {
    fn var(&mut self, e: &Expression, mode: Mode) {
        let flag = self.assigned.last_mut().expect("scope");
        match mode {
            Mode::Write => *flag = true,
            Mode::Read if !*flag => {
                let line = (self.line_of)(e.span().start.offset);
                self.lines.push(line);
            }
            _ => {}
        }
    }

    /// A new op_array (function, method, closure, arrow function).
    fn function(
        &mut self,
        params: Option<&rhp_frontend::cst::FunctionLikeParameterList>,
        body: Node,
    ) {
        self.assigned.push(false);
        if let Some(p) = params {
            for param in p.parameters.iter() {
                if let Some(d) = &param.default_value {
                    self.expr(d.value, Mode::Read);
                }
                if param.variable.name == NAME {
                    *self.assigned.last_mut().expect("scope") = true;
                }
            }
        }
        body.visit_children(|ch| self.node(ch));
        self.assigned.pop();
    }

    /// An expression compiled with `mode` for its outermost variable.
    fn expr(&mut self, e: &Expression, mode: Mode) {
        if is_target(e) {
            self.var(e, mode);
            return;
        }
        match e {
            Expression::Parenthesized(p) => self.expr(p.expression, mode),
            // a dim or property fetch compiles its base with the same mode
            Expression::ArrayAccess(a) => {
                self.expr(a.array, mode);
                self.expr(a.index, Mode::Read);
            }
            Expression::ArrayAppend(a) => self.expr(a.array, mode),
            Expression::Access(Access::Property(p)) => {
                self.expr(p.object, mode);
                self.node(Node::ClassLikeMemberSelector(&p.property));
            }
            _ => self.node(Node::Expression(e)),
        }
    }

    /// A `list()` / `[...]` destructuring target: each element is written.
    fn destructure(&mut self, e: &Expression) {
        let elements: Vec<&ArrayElement> = match e {
            Expression::Array(a) => a.elements.iter().collect(),
            Expression::List(l) => l.elements.iter().collect(),
            Expression::LegacyArray(a) => a.elements.iter().collect(),
            _ => return self.expr(e, Mode::Write),
        };
        for el in elements {
            match el {
                ArrayElement::KeyValue(kv) => {
                    self.expr(kv.key, Mode::Read);
                    self.write_target(kv.value);
                }
                ArrayElement::Value(v) => self.write_target(v.value),
                _ => self.node(Node::ArrayElement(el)),
            }
        }
    }

    fn write_target(&mut self, e: &Expression) {
        match e {
            Expression::Array(_) | Expression::List(_) | Expression::LegacyArray(_) => {
                self.destructure(e)
            }
            Expression::UnaryPrefix(u)
                if matches!(u.operator, UnaryPrefixOperator::Reference(_)) =>
            {
                self.expr(u.operand, Mode::Write)
            }
            _ => self.expr(e, Mode::Write),
        }
    }

    /// zend_compile_foreach binds a plain value variable with
    /// `zend_try_compile_cv(BP_VAR_R)`; other targets are assigned.
    fn foreach_value(&mut self, e: &Expression) {
        if is_target(e) {
            self.var(e, Mode::Read);
        } else {
            self.write_target(e);
        }
    }

    fn args(&mut self, list: &ArgumentList, known: Option<&[u8]>) {
        let by_ref = known.and_then(|n| crate::builtin_by_ref(n, list.arguments.len()));
        for (i, a) in list.arguments.iter().enumerate() {
            let value = match a {
                Argument::Positional(p) => p.value,
                Argument::Named(n) => n.value,
            };
            let is_var = matches!(
                value,
                Expression::Variable(_)
                    | Expression::ArrayAccess(_)
                    | Expression::Access(Access::Property(_))
                    | Expression::Access(Access::StaticProperty(_))
            );
            if !is_var {
                self.expr(value, Mode::Read);
            } else if by_ref
                .as_ref()
                .is_some_and(|r| r.get(i).copied().unwrap_or(false))
            {
                self.expr(value, Mode::Write);
            } else if known.is_none() && !is_target(value) {
                // SEND_FUNC_ARG: compiled with BP_VAR_FUNC_ARG (a plain
                // variable is SEND_VAR_EX, compiled as a read)
                self.expr(value, Mode::Other);
            } else {
                self.expr(value, Mode::Read);
            }
        }
    }

    fn node(&mut self, node: Node) {
        match node {
            Node::Function(f) => self.function(Some(&f.parameter_list), Node::Block(&f.body)),
            Node::Method(m) => self.function(Some(&m.parameter_list), Node::MethodBody(&m.body)),
            Node::Closure(c) => self.function(Some(&c.parameter_list), Node::Block(&c.body)),
            Node::ArrowFunction(a) => {
                self.function(Some(&a.parameter_list), Node::Expression(a.expression))
            }
            Node::Expression(e) if is_target(e) => self.var(e, Mode::Read),
            Node::DirectVariable(d) => {
                if d.name == NAME && !*self.assigned.last().expect("scope") {
                    let line = (self.line_of)(d.span.start.offset);
                    self.lines.push(line);
                }
            }
            Node::Unset(u) => {
                for v in u.values.iter() {
                    self.expr(v, Mode::Other);
                }
            }
            Node::Foreach(fe) => {
                self.expr(fe.expression, Mode::Read);
                match &fe.target {
                    ForeachTarget::Value(t) => self.foreach_value(t.value),
                    ForeachTarget::KeyValue(t) => {
                        self.foreach_value(t.value);
                        self.write_target(t.key);
                    }
                }
                self.node(Node::ForeachBody(&fe.body));
            }
            // `global` / `static` bind without compiling the variable
            Node::Global(_) | Node::StaticAbstractItem(_) => {}
            Node::StaticConcreteItem(item) => self.expr(item.value, Mode::Read),
            // a catch variable is bound without compiling it
            Node::TryCatchClause(c) => self.node(Node::Block(&c.block)),
            Node::Assignment(a) => {
                if matches!(a.operator, AssignmentOperator::Assign(_)) {
                    // the target is compiled (W) before the value
                    match a.rhs {
                        Expression::UnaryPrefix(u)
                            if matches!(u.operator, UnaryPrefixOperator::Reference(_)) =>
                        {
                            self.write_target(a.lhs);
                            self.expr(u.operand, Mode::Write);
                        }
                        _ => {
                            self.write_target(a.lhs);
                            self.expr(a.rhs, Mode::Read);
                        }
                    }
                } else {
                    // compound assignment: RW
                    self.expr(a.lhs, Mode::Other);
                    self.expr(a.rhs, Mode::Read);
                }
            }
            Node::UnaryPrefix(u)
                if matches!(
                    u.operator,
                    UnaryPrefixOperator::PreIncrement(_) | UnaryPrefixOperator::PreDecrement(_)
                ) =>
            {
                self.expr(u.operand, Mode::Other)
            }
            Node::UnaryPostfix(u) => self.expr(u.operand, Mode::Other),
            Node::IssetConstruct(i) => {
                for v in i.values.iter() {
                    self.expr(v, Mode::Other);
                }
            }
            Node::EmptyConstruct(em) => self.expr(em.value, Mode::Other),
            Node::FunctionCall(fc) => {
                let known = match fc.function {
                    Expression::Identifier(id) => {
                        let name = id.value().to_ascii_lowercase();
                        let base = name
                            .rsplit(|&c| c == b'\\')
                            .next()
                            .unwrap_or(&name)
                            .to_vec();
                        rhp_builtins::signatures::function(&base)
                            .is_some()
                            .then_some(base)
                    }
                    other => {
                        self.expr(other, Mode::Read);
                        None
                    }
                };
                self.args(&fc.argument_list, known.as_deref());
            }
            Node::MethodCall(mc) => {
                self.expr(mc.object, Mode::Read);
                self.node(Node::ClassLikeMemberSelector(&mc.method));
                self.args(&mc.argument_list, None);
            }
            Node::NullSafeMethodCall(mc) => {
                self.expr(mc.object, Mode::Read);
                self.node(Node::ClassLikeMemberSelector(&mc.method));
                self.args(&mc.argument_list, None);
            }
            Node::StaticMethodCall(sc) => {
                self.expr(sc.class, Mode::Read);
                self.node(Node::ClassLikeMemberSelector(&sc.method));
                self.args(&sc.argument_list, None);
            }
            _ => node.visit_children(|ch| self.node(ch)),
        }
    }
}
