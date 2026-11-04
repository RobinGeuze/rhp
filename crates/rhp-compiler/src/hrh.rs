//! php 8.5's compile-time deprecation of the `$http_response_header`
//! variable (zend_compile.c, `has_assigned_to_http_response_header`): a
//! read of it warns unless the same function assigned it (or has a
//! parameter of that name) earlier in source order. Variables php compiles
//! for writing (BP_VAR_W) set the flag; reads (BP_VAR_R) warn, `global` and
//! a plain variable passed as an argument included; isset/empty/`??`
//! (BP_VAR_IS), unset, compound assignments and elements or properties
//! passed to a function php does not know at compile time (SEND_FUNC_ARG)
//! do neither.
//!
//! php knows a function at compile time when the name is fully qualified
//! (or not in a namespace, or imported with `use function`) and it is a
//! builtin or a top-level function declared earlier in the same file. An
//! unqualified call inside a namespace is resolved at run time; for one of
//! php's frameless functions the arguments are compiled a second time, as
//! plain reads, for the frameless branch.

use std::collections::HashMap;
use std::rc::Rc;

use crate::parse::cst::{
    Access, Argument, ArgumentList, ArrayElement, AssignmentOperator, BinaryOperator, Expression,
    ForeachTarget, IndirectVariable, Literal, Node, Statement, UnaryPrefixOperator, Variable,
    Yield,
};
use mago_span::HasSpan;
use rhp_bytecode::signatures::Signature;

use crate::names::{NameKind, NsContext};

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

/// One op_array being compiled.
struct Scope {
    assigned: bool,
    /// ZEND_ACC_RETURN_REFERENCE: `return`/`yield` of a variable is a write.
    returns_ref: bool,
}

/// A user function declared earlier in the file: per parameter, its name,
/// whether it is by reference, and whether it is variadic.
type UserSig = Rc<Vec<(Vec<u8>, bool, bool)>>;

/// The callee of a call, as php's compiler sees it.
#[derive(Clone)]
enum Callee {
    Unknown,
    Builtin(&'static rhp_bytecode::signatures::Func),
    User(UserSig),
}

impl Callee {
    /// zend_get_arg_num: the position of named parameter `name`.
    fn arg_num(&self, name: &[u8]) -> Option<usize> {
        match self {
            Callee::Unknown => None,
            Callee::Builtin(f) => f.params.iter().position(|p| p.name.as_bytes() == name),
            Callee::User(u) => u.iter().position(|(n, _, _)| n.as_slice() == name),
        }
    }

    /// ARG_SHOULD_BE_SENT_BY_REF for argument `i` (0-based).
    fn by_ref(&self, i: usize) -> bool {
        match self {
            Callee::Unknown => false,
            Callee::Builtin(f) => f.by_ref(i),
            Callee::User(u) => match u.get(i) {
                Some((_, r, _)) => *r,
                None => u.last().is_some_and(|(_, r, v)| *r && *v),
            },
        }
    }
}

struct Scan<'a> {
    scopes: Vec<Scope>,
    /// The lines of the deprecations, in source order.
    lines: Vec<u32>,
    line_of: &'a dyn Fn(u32) -> u32,
    ns: Rc<NsContext>,
    /// Top-level functions compiled so far, by lowercase name.
    user_fns: HashMap<Vec<u8>, UserSig>,
    /// php's `in_jmp_frameless_branch`: no nested frameless branches.
    in_frameless: bool,
}

/// The lines at which php's compiler warns, for a file's statements.
pub(crate) fn scan(
    stmts: &[(Rc<NsContext>, &Statement)],
    line_of: &dyn Fn(u32) -> u32,
) -> Vec<u32> {
    let mut s = Scan {
        scopes: vec![Scope {
            assigned: false,
            returns_ref: false,
        }],
        lines: Vec::new(),
        line_of,
        ns: Rc::new(NsContext::default()),
        user_fns: HashMap::new(),
        in_frameless: false,
    };
    for (ctx, st) in stmts {
        s.ns = Rc::clone(ctx);
        s.node(Node::Statement(st));
        // a top-level function is in the function table once compiled
        if let Statement::Function(f) = st {
            let name = crate::names::join_ns(&ctx.ns, f.name.value).to_ascii_lowercase();
            let sig = f
                .parameter_list
                .parameters
                .iter()
                .map(|p| {
                    (
                        p.variable.name[1..].to_vec(),
                        p.ampersand.is_some(),
                        p.ellipsis.is_some(),
                    )
                })
                .collect();
            s.user_fns.insert(name, Rc::new(sig));
        }
    }
    s.lines
}

/// A variable php compiles as the CV `$http_response_header`: the plain
/// variable, or `${'http_response_header'}`.
fn is_target_var(v: &Variable) -> bool {
    match v {
        Variable::Direct(d) => d.name == NAME,
        Variable::Indirect(iv) => is_target_indirect(iv),
        Variable::Nested(_) => false,
    }
}

fn is_target_indirect(iv: &IndirectVariable) -> bool {
    matches!(
        iv.expression,
        Expression::Literal(Literal::String(s)) if s.value == Some(&NAME[1..])
    )
}

fn is_target(e: &Expression) -> bool {
    matches!(e, Expression::Variable(v) if is_target_var(v))
}

/// zend_is_variable.
fn is_variable(e: &Expression) -> bool {
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

/// zend_ast_is_short_circuited: a nullsafe access in the chain.
fn is_short_circuited(e: &Expression) -> bool {
    match e {
        Expression::ArrayAccess(a) => is_short_circuited(a.array),
        Expression::ArrayAppend(a) => is_short_circuited(a.array),
        Expression::Access(Access::Property(p)) => is_short_circuited(p.object),
        Expression::Access(Access::StaticProperty(p)) => is_short_circuited(p.class),
        Expression::Call(crate::parse::cst::Call::Method(m)) => is_short_circuited(m.object),
        Expression::Call(crate::parse::cst::Call::StaticMethod(m)) => is_short_circuited(m.class),
        Expression::Access(Access::NullSafeProperty(_))
        | Expression::Call(crate::parse::cst::Call::NullSafeMethod(_)) => true,
        Expression::Parenthesized(p) => is_short_circuited(p.expression),
        _ => false,
    }
}

fn is_reference<'a>(e: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    match e {
        Expression::UnaryPrefix(u) if matches!(u.operator, UnaryPrefixOperator::Reference(_)) => {
            Some(u.operand)
        }
        _ => None,
    }
}

/// zend_propagate_list_refs: a `&` anywhere in a destructuring pattern.
fn list_has_refs(e: &Expression) -> bool {
    let elements: Vec<&ArrayElement> = match e {
        Expression::Array(a) => a.elements.iter().collect(),
        Expression::List(l) => l.elements.iter().collect(),
        Expression::LegacyArray(a) => a.elements.iter().collect(),
        _ => return false,
    };
    elements.into_iter().any(|el| {
        let v = match el {
            ArrayElement::KeyValue(kv) => kv.value,
            ArrayElement::Value(v) => v.value,
            _ => return false,
        };
        is_reference(v).is_some() || list_has_refs(v)
    })
}

impl Scan<'_> {
    fn scope(&mut self) -> &mut Scope {
        self.scopes.last_mut().expect("scope")
    }

    /// The target variable at byte `offset`, compiled with `mode`.
    fn var_at(&mut self, offset: u32, mode: Mode) {
        match mode {
            Mode::Write => self.scope().assigned = true,
            Mode::Read if !self.scope().assigned => {
                let line = (self.line_of)(offset);
                self.lines.push(line);
            }
            _ => {}
        }
    }

    fn var(&mut self, e: &Expression, mode: Mode) {
        self.var_at(e.span().start.offset, mode);
    }

    /// A new op_array (function, method, closure, arrow function, hook).
    fn function(
        &mut self,
        params: Option<&crate::parse::cst::FunctionLikeParameterList>,
        returns_ref: bool,
        body: Node,
    ) {
        self.scopes.push(Scope {
            assigned: false,
            returns_ref,
        });
        if let Some(p) = params {
            for param in p.parameters.iter() {
                if let Some(d) = &param.default_value {
                    self.expr(d.value, Mode::Read);
                }
                if param.variable.name == NAME {
                    self.scope().assigned = true;
                }
            }
        }
        let saved = std::mem::replace(&mut self.in_frameless, false);
        body.visit_children(|ch| self.node(ch));
        self.in_frameless = saved;
        self.scopes.pop();
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
            Expression::Access(Access::NullSafeProperty(p)) => {
                self.expr(p.object, mode);
                self.node(Node::ClassLikeMemberSelector(&p.property));
            }
            _ => self.node(Node::Expression(e)),
        }
    }

    /// The second, BP_VAR_W compile of a `??=` target: its base variable
    /// (the offsets and names were memoized by the first).
    fn write_base(&mut self, e: &Expression) {
        match e {
            _ if is_target(e) => self.var(e, Mode::Write),
            Expression::Parenthesized(p) => self.write_base(p.expression),
            Expression::ArrayAccess(a) => self.write_base(a.array),
            Expression::ArrayAppend(a) => self.write_base(a.array),
            Expression::Access(Access::Property(p)) => self.write_base(p.object),
            _ => {}
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
            _ => match is_reference(e) {
                Some(inner) => self.expr(inner, Mode::Write),
                None => self.expr(e, Mode::Write),
            },
        }
    }

    /// zend_compile_foreach binds a plain value variable (by value or by
    /// reference) with `zend_try_compile_cv(BP_VAR_R)`; other targets are
    /// assigned.
    fn foreach_value(&mut self, e: &Expression) {
        let inner = is_reference(e).unwrap_or(e);
        if is_target(inner) {
            self.var(inner, Mode::Read);
        } else {
            self.write_target(e);
        }
    }

    /// zend_compile_args for `callee`.
    fn args(&mut self, list: &ArgumentList, callee: Callee) {
        let mut callee = callee;
        for (i, a) in list.arguments.iter().enumerate() {
            let (value, arg_num) = match a {
                Argument::Positional(p) if p.ellipsis.is_some() => {
                    // unpacking: an expression; later arguments are unknown
                    self.expr(p.value, Mode::Read);
                    callee = Callee::Unknown;
                    continue;
                }
                Argument::Positional(p) => (p.value, Some(i)),
                Argument::Named(n) => (n.value, callee.arg_num(n.name.value)),
            };
            let known = !matches!(callee, Callee::Unknown) && arg_num.is_some();
            if !is_variable(value) || is_short_circuited(value) {
                self.expr(value, Mode::Read);
            } else if known {
                let by_ref = arg_num.is_some_and(|n| callee.by_ref(n));
                self.expr(value, if by_ref { Mode::Write } else { Mode::Read });
            } else if is_target(value) {
                // SEND_VAR_EX: the CV is compiled as a read, at the line of
                // the argument list (its first argument's)
                let at = list.arguments.first().map_or(value.span(), |a| a.span());
                self.var_at(at.start.offset, Mode::Read);
            } else if matches!(value, Expression::Variable(_)) {
                self.expr(value, Mode::Read);
            } else {
                // SEND_FUNC_ARG: compiled with BP_VAR_FUNC_ARG
                self.expr(value, Mode::Other);
            }
        }
    }

    fn call(&mut self, fc: &crate::parse::cst::FunctionCall) {
        let Expression::Identifier(id) = fc.function else {
            self.expr(fc.function, Mode::Read);
            return self.args(&fc.argument_list, Callee::Unknown);
        };
        let (name, fallback) = crate::names::resolve_in(&self.ns, id, NameKind::Func);
        let Some(global) = fallback else {
            let lc = name.to_ascii_lowercase();
            let callee = match self.user_fns.get(&lc) {
                Some(u) => Callee::User(Rc::clone(u)),
                None => rhp_bytecode::signatures::function(&lc)
                    .filter(|_| !lc.contains(&b'\\'))
                    .map_or(Callee::Unknown, Callee::Builtin),
            };
            return self.args(&fc.argument_list, callee);
        };
        // ZEND_INIT_NS_FCALL_BY_NAME, then the frameless branch
        let list = &fc.argument_list;
        let plain = list
            .arguments
            .iter()
            .all(|a| matches!(a, Argument::Positional(p) if p.ellipsis.is_none()));
        let frameless = !self.in_frameless
            && plain
            && rhp_bytecode::signatures::function(&global.to_ascii_lowercase())
                .is_some_and(|f| f.frameless(list.arguments.len()));
        if !frameless {
            return self.args(list, Callee::Unknown);
        }
        self.in_frameless = true;
        self.args(list, Callee::Unknown);
        for a in list.arguments.iter() {
            if let Argument::Positional(p) = a {
                self.expr(p.value, Mode::Read);
            }
        }
        self.in_frameless = false;
    }

    fn node(&mut self, node: Node) {
        match node {
            Node::Function(f) => self.function(
                Some(&f.parameter_list),
                f.ampersand.is_some(),
                Node::Block(&f.body),
            ),
            Node::Method(m) => self.function(
                Some(&m.parameter_list),
                m.ampersand.is_some(),
                Node::MethodBody(&m.body),
            ),
            Node::Closure(c) => self.function(
                Some(&c.parameter_list),
                c.ampersand.is_some(),
                Node::Block(&c.body),
            ),
            Node::ArrowFunction(a) => {
                // the body is a `return`
                self.scopes.push(Scope {
                    assigned: false,
                    returns_ref: a.ampersand.is_some(),
                });
                for param in a.parameter_list.parameters.iter() {
                    if let Some(d) = &param.default_value {
                        self.expr(d.value, Mode::Read);
                    }
                    if param.variable.name == NAME {
                        self.scope().assigned = true;
                    }
                }
                let saved = std::mem::replace(&mut self.in_frameless, false);
                self.returned(a.expression);
                self.in_frameless = saved;
                self.scopes.pop();
            }
            Node::PropertyHook(h) => self.function(
                h.parameter_list.as_ref(),
                h.ampersand.is_some(),
                Node::PropertyHookBody(&h.body),
            ),
            Node::Expression(e) if is_target(e) => self.var(e, Mode::Read),
            Node::DirectVariable(d) => {
                if d.name == NAME {
                    self.var_at(d.span.start.offset, Mode::Read);
                }
            }
            Node::IndirectVariable(iv) => {
                if is_target_indirect(iv) {
                    self.var_at(iv.dollar_left_brace.start.offset, Mode::Read);
                } else {
                    node.visit_children(|ch| self.node(ch));
                }
            }
            Node::Return(r) => {
                if let Some(v) = r.value {
                    self.returned(v);
                }
            }
            Node::Yield(y) => match y {
                Yield::Value(v) => {
                    if let Some(e) = v.value {
                        self.returned(e);
                    }
                }
                Yield::Pair(p) => {
                    self.expr(p.key, Mode::Read);
                    self.returned(p.value);
                }
                Yield::From(_) => node.visit_children(|ch| self.node(ch)),
            },
            Node::Unset(u) => {
                for v in u.values.iter() {
                    self.expr(v, Mode::Other);
                }
            }
            Node::Foreach(fe) => {
                let by_ref = match &fe.target {
                    ForeachTarget::Value(t) => is_reference(t.value).is_some(),
                    ForeachTarget::KeyValue(t) => is_reference(t.value).is_some(),
                };
                let w = by_ref && is_variable(fe.expression);
                self.expr(fe.expression, if w { Mode::Write } else { Mode::Read });
                match &fe.target {
                    ForeachTarget::Value(t) => self.foreach_value(t.value),
                    ForeachTarget::KeyValue(t) => {
                        self.foreach_value(t.value);
                        self.write_target(t.key);
                    }
                }
                self.node(Node::ForeachBody(&fe.body));
            }
            // zend_compile_global_var: zend_try_compile_cv(BP_VAR_R)
            Node::Global(g) => {
                for v in g.variables.iter() {
                    if is_target_var(v) {
                        self.var_at(v.span().start.offset, Mode::Read);
                    }
                }
            }
            // `static` binds without compiling the variable
            Node::StaticAbstractItem(_) => {}
            Node::StaticConcreteItem(item) => self.expr(item.value, Mode::Read),
            // a catch variable is bound without compiling it
            Node::TryCatchClause(c) => self.node(Node::Block(&c.block)),
            Node::Assignment(a) => match a.operator {
                AssignmentOperator::Assign(_) => {
                    if let Some(src) = is_reference(a.rhs) {
                        // zend_compile_assign_ref: target, then source (W)
                        self.write_target(a.lhs);
                        self.expr(src, Mode::Write);
                    } else if matches!(
                        a.lhs,
                        Expression::Array(_) | Expression::List(_) | Expression::LegacyArray(_)
                    ) {
                        // list(): the value first (W when the pattern binds
                        // references), then each target
                        let w = list_has_refs(a.lhs) && is_variable(a.rhs);
                        self.expr(a.rhs, if w { Mode::Write } else { Mode::Read });
                        self.destructure(a.lhs);
                    } else {
                        // the target is compiled (W) before the value
                        self.write_target(a.lhs);
                        self.expr(a.rhs, Mode::Read);
                    }
                }
                AssignmentOperator::Coalesce(_) => {
                    // zend_compile_assign_coalesce: IS, the value, then W
                    self.expr(a.lhs, Mode::Other);
                    self.expr(a.rhs, Mode::Read);
                    self.write_base(a.lhs);
                }
                _ => {
                    // compound assignment: RW
                    self.expr(a.lhs, Mode::Other);
                    self.expr(a.rhs, Mode::Read);
                }
            },
            // zend_compile_coalesce: the left side with BP_VAR_IS
            Node::Binary(b) if matches!(b.operator, BinaryOperator::NullCoalesce(_)) => {
                self.expr(b.lhs, Mode::Other);
                self.expr(b.rhs, Mode::Read);
            }
            Node::UnaryPrefix(u)
                if matches!(
                    u.operator,
                    UnaryPrefixOperator::PreIncrement(_) | UnaryPrefixOperator::PreDecrement(_)
                ) =>
            {
                self.expr(u.operand, Mode::Other)
            }
            // `[&$x]`: a by-reference array element is compiled for writing
            Node::UnaryPrefix(u) if matches!(u.operator, UnaryPrefixOperator::Reference(_)) => {
                self.expr(u.operand, Mode::Write)
            }
            Node::UnaryPostfix(u) => self.expr(u.operand, Mode::Other),
            Node::IssetConstruct(i) => {
                for v in i.values.iter() {
                    self.expr(v, Mode::Other);
                }
            }
            // empty(expr) of a non-variable is `!expr`
            Node::EmptyConstruct(em) => self.expr(
                em.value,
                if is_variable(em.value) {
                    Mode::Other
                } else {
                    Mode::Read
                },
            ),
            Node::FunctionCall(fc) => self.call(fc),
            Node::MethodCall(mc) => {
                self.expr(mc.object, Mode::Read);
                self.node(Node::ClassLikeMemberSelector(&mc.method));
                self.args(&mc.argument_list, Callee::Unknown);
            }
            Node::NullSafeMethodCall(mc) => {
                self.expr(mc.object, Mode::Read);
                self.node(Node::ClassLikeMemberSelector(&mc.method));
                self.args(&mc.argument_list, Callee::Unknown);
            }
            Node::StaticMethodCall(sc) => {
                self.expr(sc.class, Mode::Read);
                self.node(Node::ClassLikeMemberSelector(&sc.method));
                self.args(&sc.argument_list, Callee::Unknown);
            }
            // zend_compile_new: the constructor is not known
            Node::Instantiation(n) => {
                self.expr(n.class, Mode::Read);
                if let Some(list) = &n.argument_list {
                    self.args(list, Callee::Unknown);
                }
            }
            _ => node.visit_children(|ch| self.node(ch)),
        }
    }

    /// The value of a `return` / `yield`: a variable is compiled for
    /// writing in a function that returns by reference.
    fn returned(&mut self, e: &Expression) {
        let w = self.scope().returns_ref && is_variable(e);
        self.expr(e, if w { Mode::Write } else { Mode::Read });
    }
}
