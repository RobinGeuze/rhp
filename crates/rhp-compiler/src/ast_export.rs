//! zend_ast_export: php's AST pretty-printer, over the mago CST.
//!
//! php prints the parser's AST, not the source: parentheses come from
//! operator priorities, literals are printed as values, and the parser's
//! rewrites show (`isset($a, $b)` is two issets joined by `&&`, `exit` is a
//! call to `\exit`, `global $a, $b;` is two statements). Each function here
//! follows the zend_ast.c function or case of the same name.

use crate::parse::cst::*;
use mago_span::HasSpan;
use rhp_values::Value;

/// `zend_ast_export("assert(", arg, ")")`: the message compile_assert adds
/// to a one-argument `assert()`.
pub(crate) fn assert_message(src: &[u8], arg: &Argument) -> Vec<u8> {
    let mut x = Ex {
        out: b"assert(".to_vec(),
        src,
        ce: None,
    };
    x.arg(arg, 0, 0);
    x.out.push(b')');
    x.out
}

/// zend_ast_export of a compiled constant expression (an attribute
/// argument): names are printed as zend_compile_const_expr left them and
/// foldable parts as their value.
pub(crate) fn const_expr(src: &[u8], e: &Expression, ce: &dyn ConstExprNames) -> Vec<u8> {
    let mut x = Ex {
        out: Vec::new(),
        src,
        ce: Some(ce),
    };
    x.ex(e, 0, 0);
    x.out
}

/// A heredoc, nowdoc or double-quoted string without interpolation: its
/// value (the parser makes it a plain string zval).
pub(crate) fn plain_string(e: &Expression) -> Option<Vec<u8>> {
    match unparen(e) {
        Expression::CompositeString(cs) if !matches!(cs, CompositeString::ShellExecute(_)) => {
            let x = Ex {
                out: Vec::new(),
                src: b"",
                ce: None,
            };
            x.plain_composite(cs)
        }
        _ => None,
    }
}

/// What a compiled constant expression's export needs from the compiler.
pub(crate) trait ConstExprNames {
    /// The value zend_eval_const_expr folds `e` to, if any.
    fn fold(&self, e: &Expression) -> Option<Value>;
    /// A class name in a `new` or static call (zend_compile_const_expr_class_reference).
    fn class_ref(&self, id: &Identifier) -> Vec<u8>;
    /// The class of a class constant (zend_compile_const_expr_class_const).
    fn class_const_class(&self, id: &Identifier) -> Vec<u8>;
    /// A first-class callable's function name (zend_compile_const_expr_fcc).
    fn function(&self, id: &Identifier) -> Vec<u8>;
    /// A constant php could not substitute (zend_compile_const_expr_const).
    fn constant(&self, id: &Identifier) -> Vec<u8>;
    /// The generated name of a closure (ZEND_AST_OP_ARRAY).
    fn closure_name(&self, c: &Closure) -> Vec<u8>;
}

struct Ex<'s> {
    out: Vec<u8>,
    src: &'s [u8],
    ce: Option<&'s dyn ConstExprNames>,
}

/// The parts of a ZEND_AST_VAR: its child is a name, an expression
/// (`${expr}`) or another variable (`$$x`).
#[derive(Clone, Copy)]
enum Sv<'a> {
    Name(&'a [u8]),
    Expr(&'a Expression<'a>),
    Var(&'a Variable<'a>),
}

/// php's modifier flags (zend_compile.h).
const PUBLIC: u32 = 1 << 0;
const PROTECTED: u32 = 1 << 1;
const PRIVATE: u32 = 1 << 2;
const STATIC: u32 = 1 << 4;
const FINAL: u32 = 1 << 5;
const ABSTRACT: u32 = 1 << 6;
const READONLY: u32 = 1 << 7;
const PUBLIC_SET: u32 = 1 << 10;
const PROTECTED_SET: u32 = 1 << 11;
const PRIVATE_SET: u32 = 1 << 12;
const PPP: u32 = PUBLIC | PROTECTED | PRIVATE;

fn modifier_flags(mods: &Sequence<Modifier>) -> u32 {
    mods.iter().fold(0, |f, m| {
        f | match m {
            Modifier::Static(_) => STATIC,
            Modifier::Final(_) => FINAL,
            Modifier::Abstract(_) => ABSTRACT,
            Modifier::Readonly(_) => READONLY,
            Modifier::Public(_) => PUBLIC,
            Modifier::PublicSet(_) => PUBLIC_SET,
            Modifier::Protected(_) => PROTECTED,
            Modifier::ProtectedSet(_) => PROTECTED_SET,
            Modifier::Private(_) => PRIVATE,
            Modifier::PrivateSet(_) => PRIVATE_SET,
        }
    })
}

/// method_modifiers / class_const_modifiers: public unless a visibility
/// is given.
fn implicit_public(f: u32) -> u32 {
    if f & PPP == 0 {
        f | PUBLIC
    } else {
        f
    }
}

fn unparen<'a>(mut e: &'a Expression<'a>) -> &'a Expression<'a> {
    while let Expression::Parenthesized(p) = e {
        e = p.expression;
    }
    e
}

fn sv_of<'a>(v: &'a Variable<'a>) -> Sv<'a> {
    match v {
        Variable::Direct(d) => Sv::Name(strip_dollar(d.name)),
        Variable::Indirect(i) => Sv::Expr(i.expression),
        Variable::Nested(n) => Sv::Var(n.variable),
    }
}

fn strip_dollar(n: &[u8]) -> &[u8] {
    n.strip_prefix(b"$").unwrap_or(n)
}

/// zend_ast_valid_var_name.
fn valid_var_name(s: &[u8]) -> bool {
    let Some((&c, rest)) = s.split_first() else {
        return false;
    };
    let ok = |c: u8, digit: bool| {
        c == b'_' || c >= 127 || c.is_ascii_alphabetic() || (digit && c.is_ascii_digit())
    };
    ok(c, false) && rest.iter().all(|&c| ok(c, true))
}

/// zend_ast_var_needs_braces.
fn var_needs_braces(c: u8) -> bool {
    c == b'[' || c == b'_' || c >= 127 || c.is_ascii_alphanumeric()
}

impl<'s> Ex<'s> {
    fn s(&mut self, s: &str) {
        self.out.extend_from_slice(s.as_bytes());
    }

    fn b(&mut self, s: &[u8]) {
        self.out.extend_from_slice(s);
    }

    fn c(&mut self, c: u8) {
        self.out.push(c);
    }

    fn indent(&mut self, n: usize) {
        for _ in 0..n {
            self.s("    ");
        }
    }

    // -- strings and values ------------------------------------------------

    /// zend_ast_export_str: `'` and `\` escaped.
    fn str_body(&mut self, s: &[u8]) {
        for &c in s {
            if c == b'\'' || c == b'\\' {
                self.c(b'\\');
            }
            self.c(c);
        }
    }

    /// zend_ast_export_qstr.
    fn qstr(&mut self, quote: u8, s: &[u8]) {
        for &c in s {
            if c < b' ' {
                match c {
                    b'\n' => self.s("\\n"),
                    b'\r' => self.s("\\r"),
                    b'\t' => self.s("\\t"),
                    0x0c => self.s("\\f"),
                    0x0b => self.s("\\v"),
                    0x1b => self.s("\\e"),
                    _ => {
                        self.s("\\0");
                        self.c(b'0' + c / 8);
                        self.c(b'0' + c % 8);
                    }
                }
            } else {
                if c == quote || c == b'$' || c == b'\\' {
                    self.c(b'\\');
                }
                self.c(c);
            }
        }
    }

    /// zend_ast_export_quoted_str.
    fn quoted_str(&mut self, s: &[u8]) {
        if s.iter().any(|&c| c < b' ') {
            self.c(b'"');
            self.qstr(b'"', s);
            self.c(b'"');
        } else {
            self.c(b'\'');
            self.str_body(s);
            self.c(b'\'');
        }
    }

    /// zend_ast_export_zval.
    fn zval(&mut self, v: &Value) {
        match v {
            Value::Null => self.s("null"),
            Value::Bool(false) => self.s("false"),
            Value::Bool(true) => self.s("true"),
            Value::Int(i) => self.s(&i.to_string()),
            Value::Float(f) => {
                // smart_str_append_double(precision, zero_fraction)
                let t = rhp_values::fmt::float_echo(*f);
                let dot = t.contains(&b'.');
                self.b(&t);
                if f.is_finite() && !dot {
                    self.s(".0");
                }
            }
            Value::Str(s) => self.quoted_str(s.as_bytes()),
            Value::Array(a) => {
                self.c(b'[');
                for (i, (k, v)) in a.iter().enumerate() {
                    if i > 0 {
                        self.s(", ");
                    }
                    match k {
                        rhp_values::ArrayKey::Int(n) => self.s(&n.to_string()),
                        rhp_values::ArrayKey::Str(k) => self.quoted_str(k.as_bytes()),
                    }
                    self.s(" => ");
                    self.zval(&rhp_values::ops::deref(v));
                }
                self.c(b']');
            }
            _ => {}
        }
    }

    /// A class name in a const expression: `self`/`parent`/`static` stay.
    fn ce_class(&mut self, e: &Expression, indent: usize, of_const: bool) -> bool {
        let Some(ce) = self.ce else {
            return false;
        };
        match unparen(e) {
            Expression::Identifier(id) => {
                let n = if of_const {
                    ce.class_const_class(id)
                } else {
                    ce.class_ref(id)
                };
                self.b(&n);
                true
            }
            Expression::Self_(_) | Expression::Static(_) | Expression::Parent(_) => {
                self.ns_name(e, indent);
                true
            }
            _ => false,
        }
    }

    // -- names and variables -----------------------------------------------

    /// zend_ast_export_ns_name for a class or function name position.
    fn ns_name(&mut self, e: &Expression, indent: usize) {
        match unparen(e) {
            Expression::Identifier(id) => self.b(id.value()),
            Expression::ConstantAccess(c) => self.b(c.name.value()),
            Expression::Self_(k) | Expression::Static(k) | Expression::Parent(k) => self.b(k.value),
            // A string literal's attr is ZEND_NAME_FQ (0): `'f'()` prints `\f()`.
            e => match self.zval_str(e) {
                Some(s) => {
                    self.b(b"\\");
                    self.b(&s);
                }
                None => self.ex(e, 0, indent),
            },
        }
    }

    /// zend_ast_export_name: a string literal prints raw.
    fn name_expr(&mut self, e: &Expression, indent: usize) {
        match self.zval_str(e) {
            Some(s) => self.b(&s),
            None => self.ex(e, 0, indent),
        }
    }

    /// A ZEND_AST_ZVAL string: a literal string, or a string without
    /// interpolation.
    fn zval_str(&self, e: &Expression) -> Option<Vec<u8>> {
        match self.zval_of(e)? {
            Value::Str(s) => Some(s.as_bytes().to_vec()),
            _ => None,
        }
    }

    /// The value of a ZEND_AST_ZVAL node: a literal number or string, a
    /// string without interpolation, or the parser's fold of `.` between
    /// two of them (zend_ast_create_concat_op).
    fn zval_of(&self, e: &Expression) -> Option<Value> {
        match unparen(e) {
            Expression::Literal(
                l @ (Literal::String(_) | Literal::Integer(_) | Literal::Float(_)),
            ) => Some(crate::fold::literal_value(l)),
            Expression::CompositeString(cs) if !matches!(cs, CompositeString::ShellExecute(_)) => {
                self.plain_composite(cs).map(Value::str)
            }
            Expression::Binary(Binary {
                lhs,
                operator: BinaryOperator::StringConcat(_),
                rhs,
            }) => {
                let mut l = rhp_values::conv::to_string_bytes(&self.zval_of(lhs)?);
                l.extend(rhp_values::conv::to_string_bytes(&self.zval_of(rhs)?));
                Some(Value::str(l))
            }
            _ => None,
        }
    }

    /// ZEND_AST_VAR.
    fn var(&mut self, v: &Variable, indent: usize) {
        self.c(b'$');
        self.export_var(sv_of(v), indent);
    }

    /// zend_ast_export_var.
    fn export_var(&mut self, sv: Sv, indent: usize) {
        match sv {
            Sv::Name(n) if valid_var_name(n) => self.b(n),
            Sv::Name(n) => {
                self.c(b'{');
                self.b(n);
                self.c(b'}');
            }
            Sv::Var(v) => self.var(v, indent),
            Sv::Expr(e) => {
                let e = unparen(e);
                if let Expression::Variable(v) = e {
                    return self.var(v, indent);
                }
                if let Some(s) = self.zval_str(e) {
                    if valid_var_name(&s) {
                        return self.b(&s);
                    }
                }
                self.c(b'{');
                self.name_expr(e, indent);
                self.c(b'}');
            }
        }
    }

    /// A property or method name (property_name / member_name).
    fn member(&mut self, m: &ClassLikeMemberSelector, indent: usize) {
        match m {
            ClassLikeMemberSelector::Identifier(id) => self.export_var(Sv::Name(id.value), indent),
            ClassLikeMemberSelector::Variable(v) => self.var(v, indent),
            ClassLikeMemberSelector::Expression(x) => {
                self.export_var(Sv::Expr(x.expression), indent)
            }
            ClassLikeMemberSelector::Missing(_) => {}
        }
    }

    // -- lists -----------------------------------------------------------

    fn args(&mut self, list: &ArgumentList, indent: usize) {
        for (i, a) in list.arguments.iter().enumerate() {
            if i > 0 {
                self.s(", ");
            }
            self.arg(a, 20, indent);
        }
    }

    fn arg(&mut self, a: &Argument, priority: u32, indent: usize) {
        match a {
            Argument::Positional(p) => {
                if p.ellipsis.is_some() {
                    self.s("...");
                }
                self.ex(p.value, priority, indent);
            }
            Argument::Named(n) => {
                self.b(n.name.value);
                self.s(": ");
                self.ex(n.value, priority, indent);
            }
        }
    }

    fn partial_args(&mut self, list: &PartialArgumentList, indent: usize) {
        for (i, a) in list.arguments.iter().enumerate() {
            if i > 0 {
                self.s(", ");
            }
            match a {
                PartialArgument::Positional(p) => {
                    if p.ellipsis.is_some() {
                        self.s("...");
                    }
                    self.ex(p.value, 20, indent);
                }
                PartialArgument::Named(n) => {
                    self.b(n.name.value);
                    self.s(": ");
                    self.ex(n.value, 20, indent);
                }
                PartialArgument::NamedPlaceholder(n) => {
                    self.b(n.name.value);
                    self.s(": ?");
                }
                PartialArgument::Placeholder(_) => self.c(b'?'),
                PartialArgument::VariadicPlaceholder(_) => self.s("..."),
            }
        }
    }

    fn expr_list<'a>(&mut self, items: impl Iterator<Item = &'a Expression<'a>>, indent: usize) {
        for (i, e) in items.enumerate() {
            if i > 0 {
                self.s(", ");
            }
            self.ex(e, 20, indent);
        }
    }

    /// ZEND_AST_ARRAY: `[...]`, missing elements (list()) empty, the
    /// trailing ones dropped (zend_ast_list_rtrim).
    fn array(&mut self, els: &TokenSeparatedSequence<ArrayElement>, indent: usize) {
        let els: Vec<&ArrayElement> = els.iter().collect();
        let n = els.len()
            - els
                .iter()
                .rev()
                .take_while(|e| matches!(e, ArrayElement::Missing(_)))
                .count();
        self.c(b'[');
        for (i, el) in els[..n].iter().enumerate() {
            if i > 0 {
                self.s(", ");
            }
            match el {
                ArrayElement::KeyValue(kv) => {
                    self.ex(kv.key, 80, indent);
                    self.s(" => ");
                    self.ex(kv.value, 80, indent);
                }
                ArrayElement::Value(v) => self.ex(v.value, 80, indent),
                ArrayElement::Variadic(v) => {
                    self.s("...");
                    self.ex(v.value, 80, indent);
                }
                ArrayElement::Missing(_) => {}
            }
        }
        self.c(b']');
    }

    // -- composite strings -----------------------------------------------

    fn parts<'a>(cs: &'a CompositeString<'a>) -> &'a Sequence<'a, StringPart<'a>> {
        match cs {
            CompositeString::Interpolated(i) => &i.parts,
            CompositeString::Document(d) => &d.parts,
            CompositeString::ShellExecute(s) => &s.parts,
        }
    }

    /// A literal part's value, as the compiler reads it.
    fn literal_part(cs: &CompositeString, l: &LiteralStringPart) -> Vec<u8> {
        let mut v = l.value.unwrap_or(l.raw).to_vec();
        if let CompositeString::ShellExecute(_) = cs {
            v = crate::strings::shell_exec_value(&v);
        }
        if let CompositeString::Document(d) = cs {
            if d.kind == DocumentKind::Heredoc && matches!(d.indentation, DocumentIndentation::None)
            {
                if let Some(fixed) = crate::strings::heredoc_value(l.raw, &v) {
                    v = fixed;
                }
            }
        }
        v
    }

    /// A composite string without interpolation: its value.
    fn plain_composite(&self, cs: &CompositeString) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        for p in Self::parts(cs).iter() {
            match p {
                StringPart::Literal(l) => out.extend(Self::literal_part(cs, l)),
                _ => return None,
            }
        }
        Some(out)
    }

    fn composite(&mut self, cs: &CompositeString, indent: usize) {
        let quote = if matches!(cs, CompositeString::ShellExecute(_)) {
            b'`'
        } else {
            b'"'
        };
        match self.plain_composite(cs) {
            Some(s) if quote == b'`' => {
                self.c(b'`');
                self.qstr(b'`', &s);
                self.c(b'`');
            }
            Some(s) => self.quoted_str(&s),
            None => {
                self.c(quote);
                self.encaps(cs, quote, indent);
                self.c(quote);
            }
        }
    }

    /// zend_ast_export_encaps_list.
    fn encaps(&mut self, cs: &CompositeString, quote: u8, indent: usize) {
        let parts: Vec<&StringPart> = Self::parts(cs)
            .iter()
            .filter(
                |p| !matches!(p, StringPart::Literal(l) if Self::literal_part(cs, l).is_empty()),
            )
            .collect();
        for (i, p) in parts.iter().enumerate() {
            match p {
                StringPart::Literal(l) => {
                    let v = Self::literal_part(cs, l);
                    self.qstr(quote, &v);
                }
                StringPart::Expression(e) => {
                    self.encaps_expr(e, parts.get(i + 1).copied(), cs, indent)
                }
                StringPart::BracedExpression(b) => {
                    self.encaps_expr(b.expression, parts.get(i + 1).copied(), cs, indent)
                }
            }
        }
    }

    fn encaps_expr(
        &mut self,
        e: &Expression,
        next: Option<&StringPart>,
        cs: &CompositeString,
        indent: usize,
    ) {
        let e = unparen(e);
        // A variable whose child is a name (`$a`, `${a}`, `${'x'}`) may
        // print without braces.
        let simple = match e {
            Expression::Variable(Variable::Direct(_)) => true,
            Expression::Variable(Variable::Indirect(iv)) => {
                self.dollar_brace(iv).is_some_and(|(_, idx)| idx.is_none())
                    || self.zval_str(iv.expression).is_some()
            }
            _ => false,
        };
        let next_needs = match next {
            Some(StringPart::Literal(l)) => Self::literal_part(cs, l)
                .first()
                .is_some_and(|&c| var_needs_braces(c)),
            _ => false,
        };
        if simple && !next_needs {
            self.encaps_node(e, indent);
        } else {
            self.c(b'{');
            self.encaps_node(e, indent);
            self.c(b'}');
        }
    }

    /// `"${name}"` / `"${name[expr]}"`: the name, and the index.
    fn dollar_brace<'a>(
        &self,
        iv: &'a IndirectVariable<'a>,
    ) -> Option<(&'s [u8], Option<&'a Expression<'a>>)> {
        let name = crate::strings::dollar_brace_varname(
            self.src,
            iv.dollar_left_brace.end.offset as usize,
        )?;
        match iv.expression {
            Expression::ArrayAccess(aa) => Some((name, Some(aa.index))),
            _ => Some((name, None)),
        }
    }

    /// An interpolated expression: `$a[key]` has a string key, `${name}`
    /// is a variable of that name.
    fn encaps_node(&mut self, e: &Expression, indent: usize) {
        match e {
            Expression::Variable(Variable::Indirect(iv)) => match self.dollar_brace(iv) {
                Some((name, idx)) => {
                    self.c(b'$');
                    self.export_var(Sv::Name(name), indent);
                    if let Some(idx) = idx {
                        self.c(b'[');
                        self.ex(idx, 0, indent);
                        self.c(b']');
                    }
                }
                None => self.ex(e, 0, indent),
            },
            Expression::ArrayAccess(aa) => {
                self.ex(aa.array, 260, indent);
                self.c(b'[');
                match aa.index {
                    Expression::Identifier(id) => self.quoted_str(id.value()),
                    i => self.ex(i, 0, indent),
                }
                self.c(b']');
            }
            e => self.ex(e, 0, indent),
        }
    }

    // -- operators ---------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn binary(
        &mut self,
        op: &str,
        p: u32,
        pl: u32,
        pr: u32,
        l: &Expression,
        r: &Expression,
        priority: u32,
        indent: usize,
    ) {
        if priority > p {
            self.c(b'(');
        }
        self.ex(l, pl, indent);
        self.s(op);
        self.ex(r, pr, indent);
        if priority > p {
            self.c(b')');
        }
    }

    fn prefix(&mut self, op: &str, p: u32, pl: u32, e: &Expression, priority: u32, indent: usize) {
        if priority > p {
            self.c(b'(');
        }
        self.s(op);
        self.ex(e, pl, indent);
        if priority > p {
            self.c(b')');
        }
    }

    fn func_op(&mut self, op: &str, e: &Expression, indent: usize) {
        self.s(op);
        self.c(b'(');
        self.ex(e, 0, indent);
        self.c(b')');
    }

    /// `isset($a, $b, $c)`: the parser's `isset($a) && isset($b) && ...`.
    fn isset_chain(&mut self, vals: &[&Expression], priority: u32, indent: usize) {
        match vals {
            [] => {}
            [v] => self.func_op("isset", v, indent),
            [rest @ .., last] => {
                if priority > 130 {
                    self.c(b'(');
                }
                self.isset_chain(rest, 130, indent);
                self.s(" && ");
                self.func_op("isset", last, indent);
                if priority > 130 {
                    self.c(b')');
                }
            }
        }
    }

    // -- expressions -------------------------------------------------------

    /// zend_ast_export_ex for an expression.
    fn ex(&mut self, e: &Expression, priority: u32, indent: usize) {
        if let Some(ce) = self.ce {
            if !matches!(unparen(e), Expression::Literal(_)) {
                if let Some(v) = ce.fold(e) {
                    if matches!(
                        v,
                        Value::Null
                            | Value::Bool(_)
                            | Value::Int(_)
                            | Value::Float(_)
                            | Value::Str(_)
                            | Value::Array(_)
                    ) {
                        return self.zval(&v);
                    }
                }
            }
            match unparen(e) {
                Expression::ConstantAccess(c) => return self.b(&ce.constant(&c.name)),
                Expression::Closure(c) => {
                    self.s("Closure(");
                    self.b(&ce.closure_name(c));
                    return self.c(b')');
                }
                _ => {}
            }
        }
        match e {
            Expression::Parenthesized(p) => match unparen(p.expression) {
                // ZEND_PARENTHESIZED_ARROW_FUNC
                Expression::ArrowFunction(f) => self.arrow_fn(f, true, indent),
                inner => self.ex(inner, priority, indent),
            },
            Expression::Literal(lit) => match lit {
                Literal::True(k) | Literal::False(k) | Literal::Null(k) => self.b(k.value),
                lit => self.zval(&crate::fold::literal_value(lit)),
            },
            Expression::CompositeString(cs) => self.composite(cs, indent),
            Expression::Binary(b) => match self.zval_of(e) {
                Some(v) => self.zval(&v),
                None => self.binop(b, priority, indent),
            },
            Expression::Pipe(p) => {
                self.binary(" |> ", 183, 183, 184, p.input, p.callable, priority, indent)
            }
            Expression::UnaryPrefix(u) => self.unary(u, priority, indent),
            Expression::UnaryPostfix(u) => {
                let op = match u.operator {
                    UnaryPostfixOperator::PostIncrement(_) => "++",
                    UnaryPostfixOperator::PostDecrement(_) => "--",
                };
                if priority > 240 {
                    self.c(b'(');
                }
                self.ex(u.operand, 241, indent);
                self.s(op);
                if priority > 240 {
                    self.c(b')');
                }
            }
            Expression::Assignment(a) => self.assignment(a, priority, indent),
            Expression::Conditional(c) => {
                if priority > 100 {
                    self.c(b'(');
                }
                self.ex(c.condition, 100, indent);
                match c.then {
                    Some(t) => {
                        self.s(" ? ");
                        self.ex(t, 101, indent);
                        self.s(" : ");
                    }
                    None => self.s(" ?: "),
                }
                self.ex(c.r#else, 101, indent);
                if priority > 100 {
                    self.c(b')');
                }
            }
            Expression::Array(a) => self.array(&a.elements, indent),
            Expression::LegacyArray(a) => self.array(&a.elements, indent),
            Expression::List(l) => self.array(&l.elements, indent),
            Expression::ArrayAccess(aa) => {
                self.ex(aa.array, 260, indent);
                self.c(b'[');
                self.ex(aa.index, 0, indent);
                self.c(b']');
            }
            Expression::ArrayAppend(aa) => {
                self.ex(aa.array, 260, indent);
                self.s("[]");
            }
            Expression::AnonymousClass(c) => {
                self.s("new ");
                self.attributes(&c.attribute_lists, indent, false);
                self.s("class");
                if let Some(args) = &c.argument_list {
                    if !args.arguments.is_empty() {
                        self.c(b'(');
                        self.partial_args(args, indent);
                        self.c(b')');
                    }
                }
                self.class_no_header(
                    c.extends.as_ref(),
                    c.implements.as_ref().map(|i| &i.types),
                    &c.members,
                    indent,
                );
            }
            Expression::Closure(c) => self.closure(c, indent),
            Expression::ArrowFunction(f) => self.arrow_fn(f, false, indent),
            Expression::Variable(v) => self.var(v, indent),
            Expression::ConstantAccess(c) => self.b(c.name.value()),
            Expression::Identifier(id) => self.b(id.value()),
            Expression::Self_(k) | Expression::Static(k) | Expression::Parent(k) => self.b(k.value),
            Expression::Match(m) => {
                self.s("match (");
                self.ex(m.expression, 0, indent);
                self.s(") {\n");
                for arm in m.arms.iter() {
                    self.indent(indent + 1);
                    let body = match arm {
                        MatchArm::Expression(a) => {
                            self.expr_list_p(a.conditions.iter().copied(), 0, indent + 1);
                            self.s(" => ");
                            a.expression
                        }
                        MatchArm::Default(d) => {
                            self.s("default => ");
                            d.expression
                        }
                    };
                    self.ex(body, 0, 0);
                    self.s(",\n");
                }
                self.indent(indent);
                self.c(b'}');
            }
            Expression::Yield(y) => match y {
                Yield::Value(v) => {
                    if priority > 70 {
                        self.c(b'(');
                    }
                    self.s("yield ");
                    if let Some(v) = v.value {
                        self.ex(v, 70, indent);
                    }
                    if priority > 70 {
                        self.c(b')');
                    }
                }
                Yield::Pair(p) => {
                    if priority > 70 {
                        self.c(b'(');
                    }
                    self.s("yield ");
                    self.ex(p.key, 70, indent);
                    self.s(" => ");
                    self.ex(p.value, 70, indent);
                    if priority > 70 {
                        self.c(b')');
                    }
                }
                Yield::From(f) => self.prefix("yield from ", 85, 86, f.iterator, priority, indent),
            },
            Expression::Construct(c) => self.construct(c, priority, indent),
            Expression::Throw(t) => {
                self.s("throw ");
                self.ex(t.exception, priority, indent);
            }
            Expression::Clone(c) => {
                self.s("\\clone(");
                match self.clone_arg_name(c.object) {
                    Some((name, e)) => {
                        self.out.extend_from_slice(name);
                        self.s(": ");
                        self.ex(e, 0, indent);
                    }
                    None => self.ex(c.object, 20, indent),
                }
                self.c(b')');
            }
            Expression::Call(call) => self.call(call, indent),
            Expression::PartialApplication(pa) => match pa {
                PartialApplication::Function(f) => {
                    match (self.ce, unparen(f.function)) {
                        (Some(ce), Expression::Identifier(id)) => self.b(&ce.function(id)),
                        _ => self.callee(f.function, indent),
                    }
                    self.c(b'(');
                    self.partial_args(&f.argument_list, indent);
                    self.c(b')');
                }
                PartialApplication::Method(m) => {
                    self.ex(m.object, 0, indent);
                    self.s("->");
                    self.member(&m.method, indent);
                    self.c(b'(');
                    self.partial_args(&m.argument_list, indent);
                    self.c(b')');
                }
                PartialApplication::StaticMethod(s) => {
                    if !self.ce_class(s.class, indent, false) {
                        self.ns_name(s.class, indent);
                    }
                    self.s("::");
                    self.member(&s.method, indent);
                    self.c(b'(');
                    self.partial_args(&s.argument_list, indent);
                    self.c(b')');
                }
            },
            Expression::Access(a) => match a {
                Access::Property(p) => {
                    self.ex(p.object, 0, indent);
                    self.s("->");
                    self.member(&p.property, indent);
                }
                Access::NullSafeProperty(p) => {
                    self.ex(p.object, 0, indent);
                    self.s("?->");
                    self.member(&p.property, indent);
                }
                Access::StaticProperty(p) => {
                    self.ns_name(p.class, indent);
                    self.s("::$");
                    self.export_var(sv_of(&p.property), indent);
                }
                Access::ClassConstant(c) => {
                    if !self.ce_class(c.class, indent, true) {
                        self.ns_name(c.class, indent);
                    }
                    self.s("::");
                    match &c.constant {
                        ClassLikeConstantSelector::Identifier(id)
                            if id.value.eq_ignore_ascii_case(b"class") =>
                        {
                            self.s("class")
                        }
                        ClassLikeConstantSelector::Identifier(id) => self.b(id.value),
                        ClassLikeConstantSelector::Expression(x) => {
                            self.name_expr(x.expression, indent)
                        }
                        ClassLikeConstantSelector::Missing(_) => {}
                    }
                }
            },
            Expression::Instantiation(n) => {
                self.s("new ");
                if !self.ce_class(n.class, indent, false) {
                    self.ns_name(n.class, indent);
                }
                self.c(b'(');
                if let Some(args) = &n.argument_list {
                    self.args(args, indent);
                }
                self.c(b')');
            }
            Expression::MagicConstant(m) => self.s(match m {
                MagicConstant::Line(_) => "__LINE__",
                MagicConstant::File(_) => "__FILE__",
                MagicConstant::Directory(_) => "__DIR__",
                MagicConstant::Trait(_) => "__TRAIT__",
                MagicConstant::Method(_) => "__METHOD__",
                MagicConstant::Function(_) => "__FUNCTION__",
                MagicConstant::Property(_) => "__PROPERTY__",
                MagicConstant::Namespace(_) => "__NAMESPACE__",
                MagicConstant::Class(_) => "__CLASS__",
            }),
            _ => {}
        }
    }

    fn expr_list_p<'a>(
        &mut self,
        items: impl Iterator<Item = &'a Expression<'a>>,
        p: u32,
        indent: usize,
    ) {
        for (i, e) in items.enumerate() {
            if i > 0 {
                self.s(", ");
            }
            self.ex(e, p, indent);
        }
    }

    fn binop(&mut self, b: &Binary, priority: u32, indent: usize) {
        use BinaryOperator as B;
        let (op, p, pl, pr) = match &b.operator {
            B::Addition(_) => (" + ", 200, 200, 201),
            B::Subtraction(_) => (" - ", 200, 200, 201),
            B::Multiplication(_) => (" * ", 210, 210, 211),
            B::Division(_) => (" / ", 210, 210, 211),
            B::Modulo(_) => (" % ", 210, 210, 211),
            B::LeftShift(_) => (" << ", 190, 190, 191),
            B::RightShift(_) => (" >> ", 190, 190, 191),
            B::StringConcat(_) => (" . ", 185, 185, 186),
            B::BitwiseOr(_) => (" | ", 140, 140, 141),
            B::BitwiseAnd(_) => (" & ", 160, 160, 161),
            B::BitwiseXor(_) => (" ^ ", 150, 150, 151),
            B::Identical(_) => (" === ", 170, 171, 171),
            B::NotIdentical(_) => (" !== ", 170, 171, 171),
            B::Equal(_) => (" == ", 170, 171, 171),
            B::NotEqual(_) | B::AngledNotEqual(_) => (" != ", 170, 171, 171),
            B::LessThan(_) => (" < ", 180, 181, 181),
            B::LessThanOrEqual(_) => (" <= ", 180, 181, 181),
            B::GreaterThan(_) => (" > ", 180, 181, 181),
            B::GreaterThanOrEqual(_) => (" >= ", 180, 181, 181),
            B::Spaceship(_) => (" <=> ", 180, 181, 181),
            B::Exponentiation(_) => (" ** ", 250, 251, 250),
            B::LowXor(_) => (" xor ", 40, 40, 41),
            B::And(_) | B::LowAnd(_) => (" && ", 130, 130, 131),
            B::Or(_) | B::LowOr(_) => (" || ", 120, 120, 121),
            B::NullCoalesce(_) => (" ?? ", 110, 111, 110),
            B::Instanceof(_) => {
                self.ex(b.lhs, 0, indent);
                self.s(" instanceof ");
                self.ns_name(b.rhs, indent);
                return;
            }
        };
        self.binary(op, p, pl, pr, b.lhs, b.rhs, priority, indent);
    }

    fn unary(&mut self, u: &UnaryPrefix, priority: u32, indent: usize) {
        use UnaryPrefixOperator as U;
        let op = match &u.operator {
            U::Reference(_) => {
                self.c(b'&');
                return self.ex(u.operand, priority, indent);
            }
            U::ErrorControl(_) => "@",
            U::ArrayCast(..) => "(array)",
            U::BoolCast(..) | U::BooleanCast(..) => "(bool)",
            U::DoubleCast(..) | U::RealCast(..) | U::FloatCast(..) => "(float)",
            U::IntCast(..) | U::IntegerCast(..) => "(int)",
            U::ObjectCast(..) => "(object)",
            U::UnsetCast(..) => "(unset)",
            U::StringCast(..) | U::BinaryCast(..) => "(string)",
            U::VoidCast(..) => "(void)",
            U::BitwiseNot(_) => "~",
            U::Not(_) => "!",
            U::PreIncrement(_) => "++",
            U::PreDecrement(_) => "--",
            U::Plus(_) => "+",
            U::Negation(_) => "-",
        };
        self.prefix(op, 240, 241, u.operand, priority, indent);
    }

    fn assignment(&mut self, a: &Assignment, priority: u32, indent: usize) {
        use AssignmentOperator as A;
        let op = match a.operator {
            A::Assign(_) => {
                if let Expression::UnaryPrefix(UnaryPrefix {
                    operator: UnaryPrefixOperator::Reference(_),
                    operand,
                }) = a.rhs
                {
                    return self.binary(" =& ", 90, 91, 90, a.lhs, operand, priority, indent);
                }
                " = "
            }
            A::Addition(_) => " += ",
            A::Subtraction(_) => " -= ",
            A::Multiplication(_) => " *= ",
            A::Division(_) => " /= ",
            A::Modulo(_) => " %= ",
            A::Exponentiation(_) => " **= ",
            A::Concat(_) => " .= ",
            A::BitwiseAnd(_) => " &= ",
            A::BitwiseOr(_) => " |= ",
            A::BitwiseXor(_) => " ^= ",
            A::LeftShift(_) => " <<= ",
            A::RightShift(_) => " >>= ",
            A::Coalesce(_) => " ??= ",
        };
        self.binary(op, 90, 91, 90, a.lhs, a.rhs, priority, indent);
    }

    fn construct(&mut self, c: &Construct, priority: u32, indent: usize) {
        match c {
            Construct::Isset(i) => {
                let vals: Vec<&Expression> = i.values.iter().copied().collect();
                self.isset_chain(&vals, priority, indent);
            }
            Construct::Empty(e) => self.func_op("empty", e.value, indent),
            Construct::Eval(e) => self.func_op("eval", e.value, indent),
            Construct::Include(e) => self.func_op("include", e.value, indent),
            Construct::IncludeOnce(e) => self.func_op("include_once", e.value, indent),
            Construct::Require(e) => self.func_op("require", e.value, indent),
            Construct::RequireOnce(e) => self.func_op("require_once", e.value, indent),
            Construct::Print(p) => self.prefix("print ", 60, 61, p.value, priority, indent),
            Construct::Exit(e) => self.exit(e.arguments.as_ref(), indent),
            Construct::Die(e) => self.exit(e.arguments.as_ref(), indent),
        }
    }

    /// `clone(name: $x)`: mago folds a single named argument into a
    /// parenthesized clone operand; php keeps the call with its name.
    fn clone_arg_name<'a>(&self, e: &'a Expression<'a>) -> Option<(&'s [u8], &'a Expression<'a>)> {
        let Expression::Parenthesized(p) = e else {
            return None;
        };
        let from = p.left_parenthesis.end.offset as usize;
        let to = p.expression.span().start.offset as usize;
        let head = self.src.get(from..to)?;
        let colon = head.iter().position(|&b| b == b':')?;
        Some((head[..colon].trim_ascii(), p.expression))
    }

    /// `exit` / `die`: a call to `\exit`.
    fn exit(&mut self, args: Option<&ArgumentList>, indent: usize) {
        self.s("\\exit(");
        if let Some(a) = args {
            self.args(a, indent);
        }
        self.c(b')');
    }

    /// The callee of ZEND_AST_CALL: a closure or arrow function is
    /// parenthesized (callable_expr's parentheses set no flag).
    fn callee(&mut self, f: &Expression, indent: usize) {
        let f = match f {
            Expression::Parenthesized(p) => p.expression,
            f => f,
        };
        // `clone(...)` is the keyword: the parser names the global
        // clone() (ZEND_NAME_FQ).
        let id = match f {
            Expression::Identifier(id) => Some(id),
            Expression::ConstantAccess(ca) => Some(&ca.name),
            _ => None,
        };
        if let Some(Identifier::Local(l)) = id {
            if l.value.eq_ignore_ascii_case(b"clone") {
                self.s("\\clone");
                return;
            }
        }
        if matches!(f, Expression::Closure(_) | Expression::ArrowFunction(_)) {
            self.c(b'(');
            self.ex(f, 0, indent);
            self.c(b')');
        } else {
            self.ns_name(f, indent);
        }
    }

    fn call(&mut self, call: &Call, indent: usize) {
        match call {
            Call::Function(f) => {
                self.callee(f.function, indent);
                self.c(b'(');
                self.args(&f.argument_list, indent);
                self.c(b')');
            }
            Call::Method(m) => {
                self.ex(m.object, 0, indent);
                self.s("->");
                self.member(&m.method, indent);
                self.c(b'(');
                self.args(&m.argument_list, indent);
                self.c(b')');
            }
            Call::NullSafeMethod(m) => {
                self.ex(m.object, 0, indent);
                self.s("?->");
                self.member(&m.method, indent);
                self.c(b'(');
                self.args(&m.argument_list, indent);
                self.c(b')');
            }
            Call::StaticMethod(s) => {
                self.ns_name(s.class, indent);
                self.s("::");
                self.member(&s.method, indent);
                self.c(b'(');
                self.args(&s.argument_list, indent);
                self.c(b')');
            }
        }
    }

    // -- declarations ------------------------------------------------------

    /// zend_ast_export_attributes.
    fn attributes(&mut self, lists: &Sequence<AttributeList>, indent: usize, newlines: bool) {
        for list in lists.iter() {
            self.s("#[");
            for (i, a) in list.attributes.iter().enumerate() {
                if i > 0 {
                    self.s(", ");
                }
                self.b(a.name.value());
                if let Some(args) = &a.argument_list {
                    self.c(b'(');
                    self.partial_args(args, indent);
                    self.c(b')');
                }
            }
            self.c(b']');
            if newlines {
                self.c(b'\n');
                self.indent(indent);
            } else {
                self.c(b' ');
            }
        }
    }

    /// zend_ast_export_visibility.
    fn visibility(&mut self, f: u32, set: bool) {
        if f & PUBLIC != 0 {
            self.s("public ");
        } else if f & PROTECTED != 0 {
            self.s("protected ");
        } else if f & PRIVATE != 0 {
            self.s("private ");
        }
        if set {
            if f & PRIVATE_SET != 0 {
                self.s("private(set) ");
            } else if f & PROTECTED_SET != 0 {
                self.s("protected(set) ");
            } else if f & PUBLIC_SET != 0 {
                self.s("public(set) ");
            }
        }
    }

    /// zend_ast_export_type.
    fn hint(&mut self, h: &Hint) {
        match h {
            Hint::Union(u) => {
                self.hint(u.left);
                self.c(b'|');
                self.hint(u.right);
            }
            Hint::Intersection(i) => {
                self.hint(i.left);
                self.c(b'&');
                self.hint(i.right);
            }
            Hint::Parenthesized(p) => self.hint(p.hint),
            Hint::Nullable(n) => {
                self.c(b'?');
                self.hint(n.hint);
            }
            Hint::Identifier(id) => self.b(id.value()),
            Hint::Array(_) => self.s("array"),
            Hint::Callable(_) => self.s("callable"),
            Hint::Static(_) => self.s("static"),
            Hint::Null(k) | Hint::True(k) | Hint::False(k) | Hint::Self_(k) | Hint::Parent(k) => {
                self.b(k.value)
            }
            Hint::Void(i)
            | Hint::Never(i)
            | Hint::Float(i)
            | Hint::Bool(i)
            | Hint::Integer(i)
            | Hint::String(i)
            | Hint::Object(i)
            | Hint::Mixed(i)
            | Hint::Iterable(i) => self.b(i.value),
        }
    }

    fn params(&mut self, list: &FunctionLikeParameterList, indent: usize) {
        self.c(b'(');
        for (i, p) in list.parameters.iter().enumerate() {
            if i > 0 {
                self.s(", ");
            }
            self.attributes(&p.attribute_lists, indent, false);
            let f = modifier_flags(&p.modifiers);
            self.visibility(f, true);
            if f & FINAL != 0 {
                self.s("final ");
            }
            if let Some(h) = &p.hint {
                self.hint(h);
                self.c(b' ');
            }
            if p.ampersand.is_some() {
                self.c(b'&');
            }
            if p.ellipsis.is_some() {
                self.s("...");
            }
            self.c(b'$');
            self.b(strip_dollar(p.variable.name));
            if let Some(d) = &p.default_value {
                self.s(" = ");
                self.ex(d.value, 0, indent);
            }
            if let Some(hooks) = &p.hooks {
                self.hook_list(hooks, indent);
            }
        }
        self.c(b')');
    }

    fn return_type(&mut self, r: Option<&FunctionLikeReturnTypeHint>) {
        if let Some(r) = r {
            self.s(": ");
            self.hint(&r.hint);
        }
    }

    fn body(&mut self, b: &Block, indent: usize) {
        self.s(" {\n");
        self.stmts(&b.statements, indent + 1);
        self.indent(indent);
        self.c(b'}');
    }

    fn closure(&mut self, c: &Closure, indent: usize) {
        self.attributes(&c.attribute_lists, indent, false);
        if c.r#static.is_some() {
            self.s("static ");
        }
        self.s("function ");
        if c.ampersand.is_some() {
            self.c(b'&');
        }
        self.params(&c.parameter_list, indent);
        if let Some(u) = &c.use_clause {
            self.s(" use(");
            for (i, v) in u.variables.iter().enumerate() {
                if i > 0 {
                    self.s(", ");
                }
                if v.ampersand.is_some() {
                    self.c(b'&');
                }
                self.c(b'$');
                self.b(strip_dollar(v.variable.name));
            }
            self.c(b')');
        }
        self.return_type(c.return_type_hint.as_ref());
        self.body(&c.body, indent);
    }

    fn arrow_fn(&mut self, f: &ArrowFunction, parens: bool, indent: usize) {
        if parens {
            self.c(b'(');
        }
        self.attributes(&f.attribute_lists, indent, false);
        if f.r#static.is_some() {
            self.s("static ");
        }
        self.s("fn");
        if f.ampersand.is_some() {
            self.c(b'&');
        }
        self.params(&f.parameter_list, indent);
        self.return_type(f.return_type_hint.as_ref());
        self.s(" => ");
        self.ex(f.expression, 0, indent);
        if parens {
            self.c(b')');
        }
    }

    /// ZEND_AST_FUNC_DECL / ZEND_AST_METHOD: the body ends with a newline.
    #[allow(clippy::too_many_arguments)]
    fn function_decl(
        &mut self,
        attrs: &Sequence<AttributeList>,
        flags: u32,
        by_ref: bool,
        name: &[u8],
        params: &FunctionLikeParameterList,
        ret: Option<&FunctionLikeReturnTypeHint>,
        body: Option<&Block>,
        indent: usize,
    ) {
        self.attributes(attrs, indent, true);
        self.visibility(flags, false);
        if flags & STATIC != 0 {
            self.s("static ");
        }
        if flags & ABSTRACT != 0 {
            self.s("abstract ");
        }
        if flags & FINAL != 0 {
            self.s("final ");
        }
        self.s("function ");
        if by_ref {
            self.c(b'&');
        }
        self.b(name);
        self.params(params, indent);
        self.return_type(ret);
        match body {
            Some(b) => {
                self.body(b, indent);
                self.c(b'\n');
            }
            None => self.s(";\n"),
        }
    }

    /// zend_ast_export_class_no_header.
    fn class_no_header(
        &mut self,
        extends: Option<&Extends>,
        implements: Option<&TokenSeparatedSequence<Identifier>>,
        members: &Sequence<ClassLikeMember>,
        indent: usize,
    ) {
        if let Some(e) = extends {
            self.s(" extends ");
            self.names(&e.types);
        }
        if let Some(i) = implements {
            self.s(" implements ");
            self.names(i);
        }
        self.s(" {\n");
        for m in members.iter() {
            self.member_stmt(m, indent + 1);
        }
        self.indent(indent);
        self.c(b'}');
    }

    fn names(&mut self, ids: &TokenSeparatedSequence<Identifier>) {
        for (i, id) in ids.iter().enumerate() {
            if i > 0 {
                self.s(", ");
            }
            self.b(id.value());
        }
    }

    /// ZEND_AST_CLASS as a statement.
    fn class_like(&mut self, s: &Statement, indent: usize) {
        match s {
            Statement::Class(c) => {
                self.attributes(&c.attribute_lists, indent, true);
                let f = modifier_flags(&c.modifiers);
                if f & ABSTRACT != 0 {
                    self.s("abstract ");
                }
                if f & FINAL != 0 {
                    self.s("final ");
                }
                if f & READONLY != 0 {
                    self.s("readonly ");
                }
                self.s("class ");
                self.b(c.name.value);
                self.class_no_header(
                    c.extends.as_ref(),
                    c.implements.as_ref().map(|i| &i.types),
                    &c.members,
                    indent,
                );
            }
            Statement::Interface(i) => {
                self.attributes(&i.attribute_lists, indent, true);
                self.s("interface ");
                self.b(i.name.value);
                // An interface's parents are its child[1], printed as
                // "implements".
                self.class_no_header(
                    None,
                    i.extends.as_ref().map(|e| &e.types),
                    &i.members,
                    indent,
                );
            }
            Statement::Trait(t) => {
                self.attributes(&t.attribute_lists, indent, true);
                self.s("trait ");
                self.b(t.name.value);
                self.class_no_header(None, None, &t.members, indent);
            }
            Statement::Enum(e) => {
                self.attributes(&e.attribute_lists, indent, true);
                self.s("enum ");
                self.b(e.name.value);
                if let Some(b) = &e.backing_type_hint {
                    self.s(": ");
                    self.hint(&b.hint);
                }
                self.class_no_header(
                    None,
                    e.implements.as_ref().map(|i| &i.types),
                    &e.members,
                    indent,
                );
            }
            _ => {}
        }
        self.c(b'\n');
    }

    /// zend_ast_export_hook_list.
    fn hook_list(&mut self, list: &PropertyHookList, indent: usize) {
        self.s(" {\n");
        let indent = indent + 1;
        self.indent(indent);
        let n = list.hooks.len();
        for (i, h) in list.hooks.iter().enumerate() {
            let f = modifier_flags(&h.modifiers);
            self.visibility(f, true);
            if f & FINAL != 0 {
                self.s("final ");
            }
            self.b(h.name.value);
            match &h.body {
                PropertyHookBody::Abstract(_) => self.c(b';'),
                PropertyHookBody::Concrete(PropertyHookConcreteBody::Expression(e)) => {
                    self.s(" => ");
                    self.ex(e.expression, 0, indent);
                    self.c(b';');
                }
                PropertyHookBody::Concrete(PropertyHookConcreteBody::Block(b)) => {
                    self.body(b, indent)
                }
            }
            if i + 1 < n {
                self.c(b'\n');
                self.indent(indent);
            }
        }
        self.c(b'\n');
        self.indent(indent - 1);
        self.c(b'}');
    }

    /// A class member, as zend_ast_export_stmt prints it.
    fn member_stmt(&mut self, m: &ClassLikeMember, indent: usize) {
        self.indent(indent);
        let semi = match m {
            ClassLikeMember::TraitUse(t) => {
                self.s("use ");
                self.names(&t.trait_names);
                match &t.specification {
                    TraitUseSpecification::Concrete(c) if !c.adaptations.is_empty() => {
                        self.s(" {\n");
                        for a in c.adaptations.iter() {
                            self.indent(indent + 1);
                            self.adaptation(a);
                            self.s(";\n");
                        }
                        self.indent(indent);
                        self.c(b'}');
                    }
                    _ => self.c(b';'),
                }
                false
            }
            ClassLikeMember::Constant(c) => {
                self.attributes(&c.attribute_lists, indent, true);
                self.visibility(implicit_public(modifier_flags(&c.modifiers)), false);
                self.s("const ");
                if let Some(h) = &c.hint {
                    self.hint(h);
                    self.c(b' ');
                }
                for (i, item) in c.items.iter().enumerate() {
                    if i > 0 {
                        self.s(", ");
                    }
                    self.b(item.name.value);
                    self.s(" = ");
                    self.ex(item.value, 0, indent);
                }
                true
            }
            ClassLikeMember::Property(p) => {
                let (attrs, mods, var, hint) = match p {
                    Property::Plain(p) => (
                        &p.attribute_lists,
                        &p.modifiers,
                        p.var.is_some(),
                        p.hint.as_ref(),
                    ),
                    Property::Hooked(p) => (
                        &p.attribute_lists,
                        &p.modifiers,
                        p.var.is_some(),
                        p.hint.as_ref(),
                    ),
                };
                self.attributes(attrs, indent, true);
                let mut f = modifier_flags(mods);
                if var {
                    f |= PUBLIC;
                }
                self.visibility(f, true);
                if f & STATIC != 0 {
                    self.s("static ");
                }
                if f & READONLY != 0 {
                    self.s("readonly ");
                }
                if let Some(h) = hint {
                    self.hint(h);
                    self.c(b' ');
                }
                match p {
                    Property::Plain(p) => {
                        for (i, item) in p.items.iter().enumerate() {
                            if i > 0 {
                                self.s(", ");
                            }
                            self.prop_item(item, indent);
                        }
                        true
                    }
                    Property::Hooked(p) => {
                        self.prop_item(&p.item, indent);
                        self.hook_list(&p.hook_list, indent);
                        false
                    }
                }
            }
            ClassLikeMember::EnumCase(c) => {
                self.attributes(&c.attribute_lists, indent, true);
                self.s("case ");
                match &c.item {
                    EnumCaseItem::Unit(u) => self.b(u.name.value),
                    EnumCaseItem::Backed(b) => {
                        self.b(b.name.value);
                        self.s(" = ");
                        self.ex(b.value, 0, indent);
                    }
                }
                true
            }
            ClassLikeMember::Method(m) => {
                let f = implicit_public(modifier_flags(&m.modifiers));
                let body = match &m.body {
                    MethodBody::Concrete(b) => Some(b),
                    MethodBody::Abstract(_) => None,
                };
                self.function_decl(
                    &m.attribute_lists,
                    f,
                    m.ampersand.is_some(),
                    m.name.value,
                    &m.parameter_list,
                    m.return_type_hint.as_ref(),
                    body,
                    indent,
                );
                false
            }
        };
        if semi {
            self.c(b';');
        }
        self.c(b'\n');
    }

    fn prop_item(&mut self, item: &PropertyItem, indent: usize) {
        match item {
            PropertyItem::Abstract(a) => {
                self.c(b'$');
                self.b(strip_dollar(a.variable.name));
            }
            PropertyItem::Concrete(c) => {
                self.c(b'$');
                self.b(strip_dollar(c.variable.name));
                self.s(" = ");
                self.ex(c.value, 0, indent + 1);
            }
        }
    }

    fn adaptation(&mut self, a: &TraitUseAdaptation) {
        match a {
            TraitUseAdaptation::Precedence(p) => {
                self.b(p.method_reference.trait_name.value());
                self.s("::");
                self.b(p.method_reference.method_name.value);
                self.s(" insteadof ");
                self.names(&p.trait_names);
            }
            TraitUseAdaptation::Alias(al) => {
                match &al.method_reference {
                    TraitUseMethodReference::Identifier(id) => self.b(id.value),
                    TraitUseMethodReference::Absolute(r) => {
                        self.b(r.trait_name.value());
                        self.s("::");
                        self.b(r.method_name.value);
                    }
                }
                match &al.modifier {
                    Some(Modifier::Public(_)) => self.s(" as public"),
                    Some(Modifier::Protected(_)) => self.s(" as protected"),
                    Some(Modifier::Private(_)) => self.s(" as private"),
                    _ if al.alias.is_some() => self.s(" as"),
                    _ => {}
                }
                if let Some(alias) = &al.alias {
                    self.c(b' ');
                    self.b(alias.value);
                }
            }
        }
    }

    // -- statements --------------------------------------------------------

    fn stmts(&mut self, list: &Sequence<Statement>, indent: usize) {
        for s in list.iter() {
            self.stmt(s, indent);
        }
    }

    /// One line of zend_ast_export_stmt: indent, the node, `;` unless it is
    /// a block-like statement, newline.
    fn line(&mut self, indent: usize, semi: bool, f: impl FnOnce(&mut Self)) {
        self.indent(indent);
        f(self);
        if semi {
            self.c(b';');
        }
        self.c(b'\n');
    }

    /// The statement a control structure's body holds: a block is a
    /// statement list, `;` is nothing.
    fn body_stmt(&mut self, s: &Statement, indent: usize) {
        self.stmt(s, indent);
    }

    /// zend_ast_export_stmt.
    fn stmt(&mut self, s: &Statement, indent: usize) {
        match s {
            Statement::OpeningTag(_) | Statement::ClosingTag(_) | Statement::Noop(_) => {}
            Statement::Block(b) => self.stmts(&b.statements, indent),
            Statement::Inline(i) => self.line(indent, true, |x| {
                x.s("echo ");
                x.quoted_str(i.value);
            }),
            Statement::Echo(e) => {
                for v in e.values.iter() {
                    self.line(indent, true, |x| {
                        x.s("echo ");
                        x.ex(v, 0, indent);
                    });
                }
            }
            Statement::EchoTag(e) => {
                for v in e.values.iter() {
                    self.line(indent, true, |x| {
                        x.s("echo ");
                        x.ex(v, 0, indent);
                    });
                }
            }
            Statement::Global(g) => {
                for v in g.variables.iter() {
                    self.line(indent, true, |x| {
                        x.s("global ");
                        x.var(v, indent);
                    });
                }
            }
            Statement::Static(st) => {
                for item in st.items.iter() {
                    self.line(indent, true, |x| {
                        x.s("static $");
                        match item {
                            StaticItem::Abstract(a) => x.b(strip_dollar(a.variable.name)),
                            StaticItem::Concrete(c) => {
                                x.b(strip_dollar(c.variable.name));
                                x.s(" = ");
                                x.ex(c.value, 0, indent);
                            }
                        }
                    });
                }
            }
            Statement::Unset(u) => {
                for v in u.values.iter() {
                    self.line(indent, true, |x| x.func_op("unset", v, indent));
                }
            }
            Statement::Expression(e) => self.line(indent, true, |x| x.ex(e.expression, 0, indent)),
            Statement::Return(r) => self.line(indent, true, |x| {
                x.s("return");
                if let Some(v) = r.value {
                    x.c(b' ');
                    x.ex(v, 0, indent);
                }
            }),
            Statement::Break(b) => self.line(indent, true, |x| {
                x.s("break");
                if let Some(v) = b.level {
                    x.c(b' ');
                    x.ex(v, 0, indent);
                }
            }),
            Statement::Continue(b) => self.line(indent, true, |x| {
                x.s("continue");
                if let Some(v) = b.level {
                    x.c(b' ');
                    x.ex(v, 0, indent);
                }
            }),
            Statement::Goto(g) => self.line(indent, true, |x| {
                x.s("goto ");
                x.b(g.label.value);
            }),
            Statement::Label(l) => self.line(indent, false, |x| {
                x.b(l.name.value);
                x.c(b':');
            }),
            Statement::HaltCompiler(_) => self.line(indent, true, |x| x.s("__HALT_COMPILER()")),
            Statement::If(i) => self.line(indent, false, |x| x.if_stmt(i, indent)),
            Statement::While(w) => self.line(indent, false, |x| {
                x.s("while (");
                x.ex(w.condition, 0, indent);
                x.s(") {\n");
                match &w.body {
                    WhileBody::Statement(s) => x.body_stmt(s, indent + 1),
                    WhileBody::ColonDelimited(b) => x.stmts(&b.statements, indent + 1),
                }
                x.indent(indent);
                x.c(b'}');
            }),
            Statement::DoWhile(d) => self.line(indent, true, |x| {
                x.s("do {\n");
                x.body_stmt(d.statement, indent + 1);
                x.indent(indent);
                x.s("} while (");
                x.ex(d.condition, 0, indent);
                x.c(b')');
            }),
            Statement::For(f) => self.line(indent, false, |x| {
                x.s("for (");
                x.expr_list(f.initializations.iter().copied(), indent);
                x.c(b';');
                if !f.conditions.is_empty() {
                    x.c(b' ');
                    x.expr_list(f.conditions.iter().copied(), indent);
                }
                x.c(b';');
                if !f.increments.is_empty() {
                    x.c(b' ');
                    x.expr_list(f.increments.iter().copied(), indent);
                }
                x.s(") {\n");
                match &f.body {
                    ForBody::Statement(s) => x.body_stmt(s, indent + 1),
                    ForBody::ColonDelimited(b) => x.stmts(&b.statements, indent + 1),
                }
                x.indent(indent);
                x.c(b'}');
            }),
            Statement::Foreach(f) => self.line(indent, false, |x| {
                x.s("foreach (");
                x.ex(f.expression, 0, indent);
                x.s(" as ");
                match &f.target {
                    ForeachTarget::Value(v) => x.ex(v.value, 0, indent),
                    ForeachTarget::KeyValue(kv) => {
                        x.ex(kv.key, 0, indent);
                        x.s(" => ");
                        x.ex(kv.value, 0, indent);
                    }
                }
                x.s(") {\n");
                match &f.body {
                    ForeachBody::Statement(s) => x.body_stmt(s, indent + 1),
                    ForeachBody::ColonDelimited(b) => x.stmts(&b.statements, indent + 1),
                }
                x.indent(indent);
                x.c(b'}');
            }),
            Statement::Switch(sw) => self.line(indent, false, |x| {
                x.s("switch (");
                x.ex(sw.expression, 0, indent);
                x.s(") {\n");
                let cases = match &sw.body {
                    SwitchBody::BraceDelimited(b) => &b.cases,
                    SwitchBody::ColonDelimited(b) => &b.cases,
                };
                for c in cases.iter() {
                    x.indent(indent + 1);
                    let stmts = match c {
                        SwitchCase::Expression(e) => {
                            x.s("case ");
                            x.ex(e.expression, 0, indent + 1);
                            x.s(":\n");
                            &e.statements
                        }
                        SwitchCase::Default(d) => {
                            x.s("default:\n");
                            &d.statements
                        }
                    };
                    x.stmts(stmts, indent + 2);
                }
                x.indent(indent);
                x.c(b'}');
            }),
            Statement::Try(t) => self.line(indent, false, |x| {
                x.s("try {\n");
                x.stmts(&t.block.statements, indent + 1);
                x.indent(indent);
                for c in t.catch_clauses.iter() {
                    x.s("} catch (");
                    x.hint(&c.hint);
                    if let Some(v) = &c.variable {
                        x.s(" $");
                        x.export_var(Sv::Name(strip_dollar(v.name)), indent);
                    }
                    x.s(") {\n");
                    x.stmts(&c.block.statements, indent + 1);
                    x.indent(indent);
                }
                if let Some(f) = &t.finally_clause {
                    x.s("} finally {\n");
                    x.stmts(&f.block.statements, indent + 1);
                    x.indent(indent);
                }
                x.c(b'}');
            }),
            Statement::Declare(d) => self.line(indent, false, |x| {
                x.s("declare(");
                for (i, item) in d.items.iter().enumerate() {
                    if i > 0 {
                        x.s(", ");
                    }
                    x.b(item.name.value);
                    x.s(" = ");
                    x.ex(item.value, 0, indent);
                }
                x.c(b')');
                match &d.body {
                    DeclareBody::Statement(Statement::Noop(_)) => x.c(b';'),
                    DeclareBody::Statement(s) => {
                        x.s(" {\n");
                        x.body_stmt(s, indent + 1);
                        x.indent(indent);
                        x.c(b'}');
                    }
                    DeclareBody::ColonDelimited(b) => {
                        x.s(" {\n");
                        x.stmts(&b.statements, indent + 1);
                        x.indent(indent);
                        x.c(b'}');
                    }
                }
            }),
            Statement::Function(f) => self.line(indent, false, |x| {
                x.function_decl(
                    &f.attribute_lists,
                    0,
                    f.ampersand.is_some(),
                    f.name.value,
                    &f.parameter_list,
                    f.return_type_hint.as_ref(),
                    Some(&f.body),
                    indent,
                )
            }),
            Statement::Class(_)
            | Statement::Interface(_)
            | Statement::Trait(_)
            | Statement::Enum(_) => self.line(indent, false, |x| x.class_like(s, indent)),
            Statement::Constant(c) => self.line(indent, true, |x| {
                x.attributes(&c.attribute_lists, indent, true);
                x.s("const ");
                for (i, item) in c.items.iter().enumerate() {
                    if i > 0 {
                        x.s(", ");
                    }
                    x.b(item.name.value);
                    x.s(" = ");
                    x.ex(item.value, 0, indent);
                }
            }),
            // Not allowed inside a function body.
            _ => {}
        }
    }

    /// zend_ast_export_if_stmt.
    fn if_stmt(&mut self, i: &If, indent: usize) {
        self.s("if (");
        self.ex(i.condition, 0, indent);
        self.s(") {\n");
        match &i.body {
            IfBody::Statement(b) => {
                self.body_stmt(b.statement, indent + 1);
                for c in b.else_if_clauses.iter() {
                    self.indent(indent);
                    self.s("} elseif (");
                    self.ex(c.condition, 0, indent);
                    self.s(") {\n");
                    self.body_stmt(c.statement, indent + 1);
                }
                if let Some(e) = &b.else_clause {
                    self.indent(indent);
                    self.s("} else ");
                    if let Statement::If(inner) = e.statement {
                        // `else if`: the inner if's list, printed in place.
                        return self.if_stmt(inner, indent);
                    }
                    self.s("{\n");
                    self.body_stmt(e.statement, indent + 1);
                }
            }
            IfBody::ColonDelimited(b) => {
                self.stmts(&b.statements, indent + 1);
                for c in b.else_if_clauses.iter() {
                    self.indent(indent);
                    self.s("} elseif (");
                    self.ex(c.condition, 0, indent);
                    self.s(") {\n");
                    self.stmts(&c.statements, indent + 1);
                }
                if let Some(e) = &b.else_clause {
                    self.indent(indent);
                    self.s("} else {\n");
                    self.stmts(&e.statements, indent + 1);
                }
            }
        }
        self.indent(indent);
        self.c(b'}');
    }
}
