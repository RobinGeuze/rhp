//! XPath 1.0 (libxml2's `xmlXPathEval`) over the shared tree: the full
//! expression grammar, all thirteen axes but `namespace`, and the core
//! function library.

use std::collections::HashMap;

use super::tree::{Doc, Kind, NodeId};

#[derive(Clone, Debug)]
pub enum XVal {
    Nodes(Vec<NodeId>),
    Num(f64),
    Str(Vec<u8>),
    Bool(bool),
}

#[derive(Debug)]
pub enum XErr {
    /// Malformed expression.
    Syntax,
    /// A prefix with no registered namespace.
    UndefinedPrefix,
    /// Unknown function.
    UnknownFunction(Vec<u8>),
    /// Wrong argument count / type.
    Eval,
    UndefinedVariable,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(non_camel_case_types)]
enum Axis {
    Child,
    Descendant,
    DescendantOrSelf,
    Parent,
    Ancestor,
    AncestorOrSelf,
    FollowingSibling,
    PrecedingSibling,
    Following,
    Preceding,
    Attribute,
    Self_,
    Namespace,
}

#[derive(Clone, Debug)]
enum Test {
    /// (prefix, local) — local "*" is a wildcard.
    Name(Option<Vec<u8>>, Vec<u8>),
    Node,
    Text,
    Comment,
    Pi(Option<Vec<u8>>),
}

#[derive(Clone, Debug)]
struct Step {
    axis: Axis,
    test: Test,
    preds: Vec<Expr>,
}

#[derive(Clone, Debug)]
enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Cmp(&'static str, Box<Expr>, Box<Expr>),
    Arith(u8, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
    Union(Box<Expr>, Box<Expr>),
    /// Absolute or relative location path.
    Path(bool, Vec<Step>),
    /// filter-expr predicates, then location steps.
    Filter(Box<Expr>, Vec<Expr>, Vec<Step>),
    Lit(Vec<u8>),
    Num(f64),
    Var(Vec<u8>),
    Func(Vec<u8>, Vec<Expr>),
}

// ---------------------------------------------------------------- lexer

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Slash,
    DSlash,
    LParen,
    RParen,
    LBrack,
    RBrack,
    Dot,
    DDot,
    At,
    Comma,
    DColon,
    Pipe,
    Plus,
    Minus,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Star,
    Lit(Vec<u8>),
    Num(f64),
    Var(Vec<u8>),
    /// NCName or QName (prefix:local / prefix:*).
    Name(Vec<u8>),
    OpName(Vec<u8>),
}

fn is_ncstart(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}
fn is_ncchar(c: u8) -> bool {
    is_ncstart(c) || c.is_ascii_digit() || c == b'-' || c == b'.'
}

fn lex(s: &[u8]) -> Result<Vec<Tok>, XErr> {
    let mut t: Vec<Tok> = Vec::new();
    let mut i = 0;
    // whether the preceding token makes `*` / names operators
    let op_ctx = |t: &Vec<Tok>| -> bool {
        !matches!(
            t.last(),
            None | Some(
                Tok::At
                    | Tok::DColon
                    | Tok::LParen
                    | Tok::LBrack
                    | Tok::Comma
                    | Tok::Slash
                    | Tok::DSlash
                    | Tok::Pipe
                    | Tok::Plus
                    | Tok::Minus
                    | Tok::Eq
                    | Tok::Ne
                    | Tok::Lt
                    | Tok::Le
                    | Tok::Gt
                    | Tok::Ge
                    | Tok::OpName(_),
            )
        )
    };
    while i < s.len() {
        let c = s[i];
        match c {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'/' => {
                if s.get(i + 1) == Some(&b'/') {
                    t.push(Tok::DSlash);
                    i += 2;
                } else {
                    t.push(Tok::Slash);
                    i += 1;
                }
            }
            b'(' => {
                t.push(Tok::LParen);
                i += 1
            }
            b')' => {
                t.push(Tok::RParen);
                i += 1
            }
            b'[' => {
                t.push(Tok::LBrack);
                i += 1
            }
            b']' => {
                t.push(Tok::RBrack);
                i += 1
            }
            b'@' => {
                t.push(Tok::At);
                i += 1
            }
            b',' => {
                t.push(Tok::Comma);
                i += 1
            }
            b'|' => {
                t.push(Tok::Pipe);
                i += 1
            }
            b'+' => {
                t.push(Tok::Plus);
                i += 1
            }
            b'-' => {
                t.push(Tok::Minus);
                i += 1
            }
            b'=' => {
                t.push(Tok::Eq);
                i += 1
            }
            b'!' => {
                if s.get(i + 1) == Some(&b'=') {
                    t.push(Tok::Ne);
                    i += 2;
                } else {
                    return Err(XErr::Syntax);
                }
            }
            b'<' => {
                if s.get(i + 1) == Some(&b'=') {
                    t.push(Tok::Le);
                    i += 2;
                } else {
                    t.push(Tok::Lt);
                    i += 1;
                }
            }
            b'>' => {
                if s.get(i + 1) == Some(&b'=') {
                    t.push(Tok::Ge);
                    i += 2;
                } else {
                    t.push(Tok::Gt);
                    i += 1;
                }
            }
            b':' => {
                if s.get(i + 1) == Some(&b':') {
                    t.push(Tok::DColon);
                    i += 2;
                } else {
                    return Err(XErr::Syntax);
                }
            }
            b'*' => {
                if op_ctx(&t) {
                    t.push(Tok::OpName(b"*".to_vec()));
                } else {
                    t.push(Tok::Star);
                }
                i += 1;
            }
            b'"' | b'\'' => {
                let end = s[i + 1..]
                    .iter()
                    .position(|&x| x == c)
                    .ok_or(XErr::Syntax)?;
                t.push(Tok::Lit(s[i + 1..i + 1 + end].to_vec()));
                i += end + 2;
            }
            b'$' => {
                i += 1;
                let st = i;
                while i < s.len() && (is_ncchar(s[i]) || s[i] == b':') {
                    i += 1;
                }
                if st == i {
                    return Err(XErr::Syntax);
                }
                t.push(Tok::Var(s[st..i].to_vec()));
            }
            b'.' => {
                if s.get(i + 1) == Some(&b'.') {
                    t.push(Tok::DDot);
                    i += 2;
                } else if s.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
                    let st = i;
                    i += 1;
                    while i < s.len() && s[i].is_ascii_digit() {
                        i += 1;
                    }
                    let v: f64 = std::str::from_utf8(&s[st..i])
                        .unwrap()
                        .parse()
                        .unwrap_or(f64::NAN);
                    t.push(Tok::Num(v));
                } else {
                    t.push(Tok::Dot);
                    i += 1;
                }
            }
            b'0'..=b'9' => {
                let st = i;
                while i < s.len() && s[i].is_ascii_digit() {
                    i += 1;
                }
                if i < s.len() && s[i] == b'.' {
                    i += 1;
                    while i < s.len() && s[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                if i < s.len() && (s[i] == b'e' || s[i] == b'E') {
                    let mut j = i + 1;
                    if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
                        j += 1;
                    }
                    if j < s.len() && s[j].is_ascii_digit() {
                        while j < s.len() && s[j].is_ascii_digit() {
                            j += 1;
                        }
                        i = j;
                    }
                }
                let v: f64 = std::str::from_utf8(&s[st..i])
                    .unwrap()
                    .parse()
                    .unwrap_or(f64::NAN);
                t.push(Tok::Num(v));
            }
            _ if is_ncstart(c) => {
                let st = i;
                while i < s.len() && is_ncchar(s[i]) {
                    i += 1;
                }
                // QName / prefix:*
                if i + 1 < s.len() && s[i] == b':' && s[i + 1] != b':' {
                    if s[i + 1] == b'*' {
                        i += 2;
                    } else if is_ncstart(s[i + 1]) {
                        i += 1;
                        while i < s.len() && is_ncchar(s[i]) {
                            i += 1;
                        }
                    }
                }
                let name = s[st..i].to_vec();
                if op_ctx(&t) && matches!(&name[..], b"and" | b"or" | b"mod" | b"div") {
                    t.push(Tok::OpName(name));
                } else {
                    t.push(Tok::Name(name));
                }
            }
            _ => return Err(XErr::Syntax),
        }
    }
    Ok(t)
}

// ---------------------------------------------------------------- parser

struct Parser {
    t: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn peek2(&self) -> Option<&Tok> {
        self.t.get(self.i + 1)
    }
    fn eat(&mut self, k: &Tok) -> bool {
        if self.peek() == Some(k) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn eat_op(&mut self, name: &[u8]) -> bool {
        if let Some(Tok::OpName(n)) = self.peek() {
            if n == name {
                self.i += 1;
                return true;
            }
        }
        false
    }

    fn expr(&mut self) -> Result<Expr, XErr> {
        let mut l = self.and()?;
        while self.eat_op(b"or") {
            let r = self.and()?;
            l = Expr::Or(Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn and(&mut self) -> Result<Expr, XErr> {
        let mut l = self.eq()?;
        while self.eat_op(b"and") {
            let r = self.eq()?;
            l = Expr::And(Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn eq(&mut self) -> Result<Expr, XErr> {
        let mut l = self.rel()?;
        loop {
            let op = if self.eat(&Tok::Eq) {
                "="
            } else if self.eat(&Tok::Ne) {
                "!="
            } else {
                return Ok(l);
            };
            let r = self.rel()?;
            l = Expr::Cmp(op, Box::new(l), Box::new(r));
        }
    }
    fn rel(&mut self) -> Result<Expr, XErr> {
        let mut l = self.add()?;
        loop {
            let op = if self.eat(&Tok::Lt) {
                "<"
            } else if self.eat(&Tok::Le) {
                "<="
            } else if self.eat(&Tok::Gt) {
                ">"
            } else if self.eat(&Tok::Ge) {
                ">="
            } else {
                return Ok(l);
            };
            let r = self.add()?;
            l = Expr::Cmp(op, Box::new(l), Box::new(r));
        }
    }
    fn add(&mut self) -> Result<Expr, XErr> {
        let mut l = self.mul()?;
        loop {
            let op = if self.eat(&Tok::Plus) {
                b'+'
            } else if self.eat(&Tok::Minus) {
                b'-'
            } else {
                return Ok(l);
            };
            let r = self.mul()?;
            l = Expr::Arith(op, Box::new(l), Box::new(r));
        }
    }
    fn mul(&mut self) -> Result<Expr, XErr> {
        let mut l = self.unary()?;
        loop {
            let op = if self.eat_op(b"*") {
                b'*'
            } else if self.eat_op(b"div") {
                b'/'
            } else if self.eat_op(b"mod") {
                b'%'
            } else {
                return Ok(l);
            };
            let r = self.unary()?;
            l = Expr::Arith(op, Box::new(l), Box::new(r));
        }
    }
    fn unary(&mut self) -> Result<Expr, XErr> {
        if self.eat(&Tok::Minus) {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.union()
    }
    fn union(&mut self) -> Result<Expr, XErr> {
        let mut l = self.path()?;
        while self.eat(&Tok::Pipe) {
            let r = self.path()?;
            l = Expr::Union(Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn is_primary_start(&self) -> bool {
        match self.peek() {
            Some(Tok::Var(_) | Tok::LParen | Tok::Lit(_) | Tok::Num(_)) => true,
            Some(Tok::Name(n)) => {
                self.peek2() == Some(&Tok::LParen)
                    && !matches!(
                        &n[..],
                        b"comment" | b"text" | b"node" | b"processing-instruction"
                    )
            }
            _ => false,
        }
    }

    fn path(&mut self) -> Result<Expr, XErr> {
        if self.is_primary_start() {
            let prim = self.primary()?;
            let mut preds = Vec::new();
            while self.peek() == Some(&Tok::LBrack) {
                preds.push(self.predicate()?);
            }
            let mut steps = Vec::new();
            loop {
                if self.eat(&Tok::Slash) {
                    steps.push(self.step()?);
                } else if self.eat(&Tok::DSlash) {
                    steps.push(dos_step());
                    steps.push(self.step()?);
                } else {
                    break;
                }
            }
            if preds.is_empty() && steps.is_empty() {
                return Ok(prim);
            }
            return Ok(Expr::Filter(Box::new(prim), preds, steps));
        }
        // location path
        let mut steps = Vec::new();
        let absolute;
        if self.eat(&Tok::Slash) {
            absolute = true;
            if self.starts_step() {
                steps.push(self.step()?);
            } else {
                return Ok(Expr::Path(true, steps));
            }
        } else if self.eat(&Tok::DSlash) {
            absolute = true;
            steps.push(dos_step());
            steps.push(self.step()?);
        } else {
            absolute = false;
            steps.push(self.step()?);
        }
        loop {
            if self.eat(&Tok::Slash) {
                steps.push(self.step()?);
            } else if self.eat(&Tok::DSlash) {
                steps.push(dos_step());
                steps.push(self.step()?);
            } else {
                break;
            }
        }
        Ok(Expr::Path(absolute, steps))
    }

    fn starts_step(&self) -> bool {
        matches!(
            self.peek(),
            Some(Tok::Dot | Tok::DDot | Tok::At | Tok::Star | Tok::Name(_))
        )
    }

    fn predicate(&mut self) -> Result<Expr, XErr> {
        if !self.eat(&Tok::LBrack) {
            return Err(XErr::Syntax);
        }
        let e = self.expr()?;
        if !self.eat(&Tok::RBrack) {
            return Err(XErr::Syntax);
        }
        Ok(e)
    }

    fn step(&mut self) -> Result<Step, XErr> {
        if self.eat(&Tok::Dot) {
            return Ok(Step {
                axis: Axis::Self_,
                test: Test::Node,
                preds: vec![],
            });
        }
        if self.eat(&Tok::DDot) {
            return Ok(Step {
                axis: Axis::Parent,
                test: Test::Node,
                preds: vec![],
            });
        }
        let mut axis = Axis::Child;
        if self.eat(&Tok::At) {
            axis = Axis::Attribute;
        } else if let (Some(Tok::Name(n)), Some(Tok::DColon)) = (self.peek(), self.peek2()) {
            axis = match &n[..] {
                b"child" => Axis::Child,
                b"descendant" => Axis::Descendant,
                b"descendant-or-self" => Axis::DescendantOrSelf,
                b"parent" => Axis::Parent,
                b"ancestor" => Axis::Ancestor,
                b"ancestor-or-self" => Axis::AncestorOrSelf,
                b"following-sibling" => Axis::FollowingSibling,
                b"preceding-sibling" => Axis::PrecedingSibling,
                b"following" => Axis::Following,
                b"preceding" => Axis::Preceding,
                b"attribute" => Axis::Attribute,
                b"self" => Axis::Self_,
                b"namespace" => Axis::Namespace,
                _ => return Err(XErr::Syntax),
            };
            self.i += 2;
        }
        let test = match self.peek().cloned() {
            Some(Tok::Star) => {
                self.i += 1;
                Test::Name(None, b"*".to_vec())
            }
            Some(Tok::Name(n)) => {
                self.i += 1;
                if self.peek() == Some(&Tok::LParen)
                    && matches!(
                        &n[..],
                        b"comment" | b"text" | b"node" | b"processing-instruction"
                    )
                {
                    self.i += 1;
                    let t = match &n[..] {
                        b"comment" => Test::Comment,
                        b"text" => Test::Text,
                        b"node" => Test::Node,
                        _ => {
                            if let Some(Tok::Lit(l)) = self.peek().cloned() {
                                self.i += 1;
                                Test::Pi(Some(l))
                            } else {
                                Test::Pi(None)
                            }
                        }
                    };
                    if !self.eat(&Tok::RParen) {
                        return Err(XErr::Syntax);
                    }
                    t
                } else {
                    match n.iter().position(|&c| c == b':') {
                        Some(p) => Test::Name(Some(n[..p].to_vec()), n[p + 1..].to_vec()),
                        None => Test::Name(None, n),
                    }
                }
            }
            _ => return Err(XErr::Syntax),
        };
        let mut preds = Vec::new();
        while self.peek() == Some(&Tok::LBrack) {
            preds.push(self.predicate()?);
        }
        Ok(Step { axis, test, preds })
    }

    fn primary(&mut self) -> Result<Expr, XErr> {
        match self.peek().cloned() {
            Some(Tok::Var(v)) => {
                self.i += 1;
                Ok(Expr::Var(v))
            }
            Some(Tok::LParen) => {
                self.i += 1;
                let e = self.expr()?;
                if !self.eat(&Tok::RParen) {
                    return Err(XErr::Syntax);
                }
                Ok(e)
            }
            Some(Tok::Lit(l)) => {
                self.i += 1;
                Ok(Expr::Lit(l))
            }
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(Expr::Num(n))
            }
            Some(Tok::Name(n)) => {
                self.i += 2; // name (
                let mut args = Vec::new();
                if !self.eat(&Tok::RParen) {
                    loop {
                        args.push(self.expr()?);
                        if self.eat(&Tok::Comma) {
                            continue;
                        }
                        if self.eat(&Tok::RParen) {
                            break;
                        }
                        return Err(XErr::Syntax);
                    }
                }
                Ok(Expr::Func(n, args))
            }
            _ => Err(XErr::Syntax),
        }
    }
}

fn dos_step() -> Step {
    Step {
        axis: Axis::DescendantOrSelf,
        test: Test::Node,
        preds: vec![],
    }
}

fn compile(s: &[u8]) -> Result<Expr, XErr> {
    let toks = lex(s)?;
    if toks.is_empty() {
        return Err(XErr::Syntax);
    }
    let mut p = Parser { t: toks, i: 0 };
    let e = p.expr()?;
    if p.i != p.t.len() {
        return Err(XErr::Syntax);
    }
    Ok(e)
}

// ---------------------------------------------------------------- evaluator

pub struct Ctx<'a> {
    pub doc: &'a Doc,
    pub ns: &'a HashMap<Vec<u8>, Vec<u8>>,
    pub vars: &'a HashMap<Vec<u8>, XVal>,
}

/// Document-order key of a node: path of sibling indices from the root
/// (attributes sort right after their element).
fn order_key(doc: &Doc, mut id: NodeId) -> Vec<(usize, usize)> {
    let mut key = Vec::new();
    loop {
        let n = &doc.nodes[id];
        match n.parent {
            None => {
                // detached roots order by id
                key.push((0, id));
                break;
            }
            Some(p) => {
                if n.kind == Kind::Attr {
                    let i = doc.nodes[p]
                        .attrs
                        .iter()
                        .position(|&a| a == id)
                        .unwrap_or(0);
                    key.push((0, i));
                } else {
                    let mut i = 0;
                    let mut c = doc.nodes[p].first;
                    while let Some(x) = c {
                        if x == id {
                            break;
                        }
                        i += 1;
                        c = doc.nodes[x].next;
                    }
                    key.push((1, i));
                }
                id = p;
            }
        }
    }
    key.reverse();
    key
}

pub fn sort_doc_order(doc: &Doc, v: &mut Vec<NodeId>) {
    v.sort_unstable();
    v.dedup();
    if v.len() > 1 {
        let mut keyed: Vec<(Vec<(usize, usize)>, NodeId)> =
            v.iter().map(|&n| (order_key(doc, n), n)).collect();
        keyed.sort();
        *v = keyed.into_iter().map(|(_, n)| n).collect();
    }
}

pub fn string_value(doc: &Doc, id: NodeId) -> Vec<u8> {
    doc.text_content(id)
}

fn num_to_string(n: f64) -> Vec<u8> {
    // libxml2's xmlXPathFormatNumber
    if n.is_nan() {
        return b"NaN".to_vec();
    }
    if n.is_infinite() {
        return if n > 0.0 {
            b"Infinity".to_vec()
        } else {
            b"-Infinity".to_vec()
        };
    }
    if n == 0.0 {
        return b"0".to_vec();
    }
    if n > i32::MIN as f64 && n < i32::MAX as f64 && n == (n as i32) as f64 {
        return format!("{}", n as i32).into_bytes();
    }
    let a = n.abs();
    let trim = |s: String| -> String {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    };
    if !(1e-5..=1e9).contains(&a) {
        let s = format!("{:.14e}", n);
        let (mant, exp) = s.split_once('e').unwrap();
        let mant = trim(mant.to_string());
        let e: i32 = exp.parse().unwrap_or(0);
        return format!("{mant}e{}{:02}", if e < 0 { "-" } else { "+" }, e.abs()).into_bytes();
    }
    let integer_place = a.log10() as i32;
    let fraction_place = if integer_place > 0 {
        15 - integer_place - 1
    } else {
        15 - integer_place
    };
    let prec = if fraction_place < 0 {
        6
    } else {
        fraction_place as usize
    };
    trim(format!("{:.*}", prec, n)).into_bytes()
}

pub fn str_to_num(s: &[u8]) -> f64 {
    let t = String::from_utf8_lossy(s);
    let t = t.trim_matches(|c| c == ' ' || c == '\t' || c == '\n' || c == '\r');
    if t.is_empty() {
        return f64::NAN;
    }
    let body = t.strip_prefix('-').unwrap_or(t);
    let ok = !body.is_empty()
        && body.chars().all(|c| c.is_ascii_digit() || c == '.')
        && body.chars().filter(|&c| c == '.').count() <= 1
        && body != ".";
    if !ok {
        return f64::NAN;
    }
    t.parse().unwrap_or(f64::NAN)
}

impl Ctx<'_> {
    fn to_str(&self, v: &XVal) -> Vec<u8> {
        match v {
            XVal::Str(s) => s.clone(),
            XVal::Num(n) => num_to_string(*n),
            XVal::Bool(b) => {
                if *b {
                    b"true".to_vec()
                } else {
                    b"false".to_vec()
                }
            }
            XVal::Nodes(ns) => ns
                .first()
                .map(|&n| string_value(self.doc, n))
                .unwrap_or_default(),
        }
    }
    fn to_num(&self, v: &XVal) -> f64 {
        match v {
            XVal::Num(n) => *n,
            XVal::Bool(b) => *b as i32 as f64,
            _ => str_to_num(&self.to_str(v)),
        }
    }
    fn to_bool(&self, v: &XVal) -> bool {
        match v {
            XVal::Bool(b) => *b,
            XVal::Num(n) => *n != 0.0 && !n.is_nan(),
            XVal::Str(s) => !s.is_empty(),
            XVal::Nodes(n) => !n.is_empty(),
        }
    }

    fn matches(&self, id: NodeId, axis: Axis, test: &Test) -> Result<bool, XErr> {
        let n = &self.doc.nodes[id];
        Ok(match test {
            Test::Node => true,
            Test::Text => matches!(n.kind, Kind::Text | Kind::CData),
            Test::Comment => n.kind == Kind::Comment,
            Test::Pi(t) => n.kind == Kind::Pi && t.as_ref().is_none_or(|t| *t == n.name),
            Test::Name(prefix, local) => {
                let principal = if axis == Axis::Attribute {
                    Kind::Attr
                } else {
                    Kind::Element
                };
                if n.kind != principal {
                    return Ok(false);
                }
                let want_ns = match prefix {
                    Some(p) => Some(self.ns.get(p).cloned().ok_or(XErr::UndefinedPrefix)?),
                    None => None,
                };
                if local == b"*" {
                    match &want_ns {
                        None => true,
                        Some(u) => n.ns.as_ref() == Some(u),
                    }
                } else {
                    n.name == *local && n.ns == want_ns
                }
            }
        })
    }

    fn axis_nodes(&self, id: NodeId, axis: Axis) -> Vec<NodeId> {
        let d = self.doc;
        let mut out = Vec::new();
        match axis {
            Axis::Child => {
                if d.nodes[id].kind != Kind::Attr {
                    out = d.children(id);
                }
            }
            Axis::Descendant => {
                if d.nodes[id].kind != Kind::Attr {
                    d.descendants(id, &mut out);
                }
            }
            Axis::DescendantOrSelf => {
                out.push(id);
                if d.nodes[id].kind != Kind::Attr {
                    d.descendants(id, &mut out);
                }
            }
            Axis::Parent => {
                if let Some(p) = d.nodes[id].parent {
                    out.push(p);
                }
            }
            Axis::Ancestor | Axis::AncestorOrSelf => {
                if axis == Axis::AncestorOrSelf {
                    out.push(id);
                }
                let mut c = d.nodes[id].parent;
                while let Some(p) = c {
                    out.push(p);
                    c = d.nodes[p].parent;
                }
            }
            Axis::FollowingSibling => {
                if d.nodes[id].kind != Kind::Attr {
                    let mut c = d.nodes[id].next;
                    while let Some(x) = c {
                        out.push(x);
                        c = d.nodes[x].next;
                    }
                }
            }
            Axis::PrecedingSibling => {
                if d.nodes[id].kind != Kind::Attr {
                    let mut c = d.nodes[id].prev;
                    while let Some(x) = c {
                        out.push(x);
                        c = d.nodes[x].prev;
                    }
                }
            }
            Axis::Following => {
                let mut cur = id;
                if d.nodes[id].kind == Kind::Attr {
                    if let Some(p) = d.nodes[id].parent {
                        cur = p;
                        d.descendants(p, &mut out);
                    }
                }
                loop {
                    let mut s = d.nodes[cur].next;
                    while let Some(x) = s {
                        out.push(x);
                        d.descendants(x, &mut out);
                        s = d.nodes[x].next;
                    }
                    match d.nodes[cur].parent {
                        Some(p) => cur = p,
                        None => break,
                    }
                }
            }
            Axis::Preceding => {
                let mut cur = if d.nodes[id].kind == Kind::Attr {
                    d.nodes[id].parent.unwrap_or(id)
                } else {
                    id
                };
                loop {
                    let mut s = d.nodes[cur].prev;
                    while let Some(x) = s {
                        let mut sub = vec![x];
                        d.descendants(x, &mut sub);
                        sub.reverse();
                        out.extend(sub);
                        s = d.nodes[x].prev;
                    }
                    match d.nodes[cur].parent {
                        Some(p) => cur = p,
                        None => break,
                    }
                }
            }
            Axis::Attribute => {
                if d.nodes[id].kind == Kind::Element {
                    out = d.nodes[id].attrs.clone();
                }
            }
            Axis::Self_ => out.push(id),
            Axis::Namespace => {}
        }
        out
    }

    fn reverse_axis(axis: Axis) -> bool {
        matches!(
            axis,
            Axis::Ancestor | Axis::AncestorOrSelf | Axis::PrecedingSibling | Axis::Preceding
        )
    }

    fn apply_steps(&self, mut set: Vec<NodeId>, steps: &[Step]) -> Result<Vec<NodeId>, XErr> {
        for st in steps {
            let mut next = Vec::new();
            for &n in &set {
                let mut cands = Vec::new();
                for c in self.axis_nodes(n, st.axis) {
                    if self.matches(c, st.axis, &st.test)? {
                        cands.push(c);
                    }
                }
                // predicates in axis order (proximity positions)
                for p in &st.preds {
                    cands = self.filter(cands, p)?;
                }
                next.extend(cands);
            }
            sort_doc_order(self.doc, &mut next);
            set = next;
            let _ = Self::reverse_axis(st.axis);
        }
        Ok(set)
    }

    fn filter(&self, cands: Vec<NodeId>, pred: &Expr) -> Result<Vec<NodeId>, XErr> {
        let size = cands.len();
        let mut out = Vec::new();
        for (i, &c) in cands.iter().enumerate() {
            let v = self.eval(pred, c, i + 1, size)?;
            let keep = match v {
                XVal::Num(n) => n == (i + 1) as f64,
                other => self.to_bool(&other),
            };
            if keep {
                out.push(c);
            }
        }
        Ok(out)
    }

    fn eval(&self, e: &Expr, node: NodeId, pos: usize, size: usize) -> Result<XVal, XErr> {
        Ok(match e {
            Expr::Or(a, b) => XVal::Bool(
                self.to_bool(&self.eval(a, node, pos, size)?)
                    || self.to_bool(&self.eval(b, node, pos, size)?),
            ),
            Expr::And(a, b) => XVal::Bool(
                self.to_bool(&self.eval(a, node, pos, size)?)
                    && self.to_bool(&self.eval(b, node, pos, size)?),
            ),
            Expr::Cmp(op, a, b) => {
                let x = self.eval(a, node, pos, size)?;
                let y = self.eval(b, node, pos, size)?;
                XVal::Bool(self.compare(op, &x, &y))
            }
            Expr::Arith(op, a, b) => {
                let x = self.to_num(&self.eval(a, node, pos, size)?);
                let y = self.to_num(&self.eval(b, node, pos, size)?);
                XVal::Num(match op {
                    b'+' => x + y,
                    b'-' => x - y,
                    b'*' => x * y,
                    b'/' => x / y,
                    _ => x % y,
                })
            }
            Expr::Neg(a) => XVal::Num(-self.to_num(&self.eval(a, node, pos, size)?)),
            Expr::Union(a, b) => {
                let (XVal::Nodes(mut x), XVal::Nodes(y)) = (
                    self.eval(a, node, pos, size)?,
                    self.eval(b, node, pos, size)?,
                ) else {
                    return Err(XErr::Eval);
                };
                x.extend(y);
                sort_doc_order(self.doc, &mut x);
                XVal::Nodes(x)
            }
            Expr::Path(abs, steps) => {
                let start = if *abs { 0 } else { node };
                let start = if *abs {
                    // the root of the tree containing the context node
                    let mut r = node;
                    while let Some(p) = self.doc.nodes[r].parent {
                        r = p;
                    }
                    if self.doc.connected(node) {
                        0
                    } else {
                        r
                    }
                } else {
                    start
                };
                XVal::Nodes(self.apply_steps(vec![start], steps)?)
            }
            Expr::Filter(prim, preds, steps) => {
                let v = self.eval(prim, node, pos, size)?;
                let XVal::Nodes(mut set) = v else {
                    if preds.is_empty() && steps.is_empty() {
                        return Ok(v);
                    }
                    return Err(XErr::Eval);
                };
                for p in preds {
                    set = self.filter(set, p)?;
                }
                XVal::Nodes(self.apply_steps(set, steps)?)
            }
            Expr::Lit(s) => XVal::Str(s.clone()),
            Expr::Num(n) => XVal::Num(*n),
            Expr::Var(v) => self.vars.get(v).cloned().ok_or(XErr::UndefinedVariable)?,
            Expr::Func(name, args) => self.call(name, args, node, pos, size)?,
        })
    }

    fn compare(&self, op: &str, x: &XVal, y: &XVal) -> bool {
        match (x, y) {
            (XVal::Nodes(a), XVal::Nodes(b)) => a.iter().any(|&p| {
                let sp = string_value(self.doc, p);
                b.iter().any(|&q| {
                    let sq = string_value(self.doc, q);
                    self.cmp_atoms(op, &XVal::Str(sp.clone()), &XVal::Str(sq))
                })
            }),
            (XVal::Nodes(a), other) => a.iter().any(|&p| {
                let sp = string_value(self.doc, p);
                let left = match other {
                    XVal::Num(_) => XVal::Num(str_to_num(&sp)),
                    XVal::Bool(_) => XVal::Bool(true),
                    _ => XVal::Str(sp),
                };
                if let XVal::Bool(b) = other {
                    return self.cmp_atoms(op, &XVal::Bool(!a.is_empty()), &XVal::Bool(*b));
                }
                self.cmp_atoms(op, &left, other)
            }),
            (other, XVal::Nodes(b)) => {
                let flipped = match op {
                    "<" => ">",
                    ">" => "<",
                    "<=" => ">=",
                    ">=" => "<=",
                    o => o,
                };
                self.compare(flipped, &XVal::Nodes(b.clone()), other)
            }
            _ => self.cmp_atoms(op, x, y),
        }
    }

    fn cmp_atoms(&self, op: &str, x: &XVal, y: &XVal) -> bool {
        match op {
            "=" | "!=" => {
                let eq = if matches!(x, XVal::Bool(_)) || matches!(y, XVal::Bool(_)) {
                    self.to_bool(x) == self.to_bool(y)
                } else if matches!(x, XVal::Num(_)) || matches!(y, XVal::Num(_)) {
                    self.to_num(x) == self.to_num(y)
                } else {
                    self.to_str(x) == self.to_str(y)
                };
                if op == "=" {
                    eq
                } else {
                    !eq
                }
            }
            _ => {
                let (a, b) = (self.to_num(x), self.to_num(y));
                match op {
                    "<" => a < b,
                    "<=" => a <= b,
                    ">" => a > b,
                    _ => a >= b,
                }
            }
        }
    }

    fn call(
        &self,
        name: &[u8],
        args: &[Expr],
        node: NodeId,
        pos: usize,
        size: usize,
    ) -> Result<XVal, XErr> {
        let ev = |i: usize| -> Result<XVal, XErr> { self.eval(&args[i], node, pos, size) };
        let argn = |lo: usize, hi: usize| -> Result<(), XErr> {
            if args.len() < lo || args.len() > hi {
                Err(XErr::Eval)
            } else {
                Ok(())
            }
        };
        let str_arg = |i: usize| -> Result<Vec<u8>, XErr> {
            if args.len() > i {
                Ok(self.to_str(&ev(i)?))
            } else {
                Ok(string_value(self.doc, node))
            }
        };
        let node_arg = |i: usize| -> Result<Option<NodeId>, XErr> {
            if args.len() > i {
                match ev(i)? {
                    XVal::Nodes(n) => Ok(n.first().copied()),
                    _ => Err(XErr::Eval),
                }
            } else {
                Ok(Some(node))
            }
        };
        Ok(match name {
            b"last" => {
                argn(0, 0)?;
                XVal::Num(size as f64)
            }
            b"position" => {
                argn(0, 0)?;
                XVal::Num(pos as f64)
            }
            b"count" => {
                argn(1, 1)?;
                match ev(0)? {
                    XVal::Nodes(n) => XVal::Num(n.len() as f64),
                    _ => return Err(XErr::Eval),
                }
            }
            b"id" => XVal::Nodes(Vec::new()),
            b"local-name" => {
                argn(0, 1)?;
                XVal::Str(match node_arg(0)? {
                    Some(n) => {
                        let nd = &self.doc.nodes[n];
                        match nd.kind {
                            Kind::Element | Kind::Attr | Kind::Pi => nd.name.clone(),
                            _ => Vec::new(),
                        }
                    }
                    None => Vec::new(),
                })
            }
            b"namespace-uri" => {
                argn(0, 1)?;
                XVal::Str(match node_arg(0)? {
                    Some(n) => self.doc.nodes[n].ns.clone().unwrap_or_default(),
                    None => Vec::new(),
                })
            }
            b"name" => {
                argn(0, 1)?;
                XVal::Str(match node_arg(0)? {
                    Some(n) => {
                        let nd = &self.doc.nodes[n];
                        match nd.kind {
                            Kind::Element | Kind::Attr => nd.qname(),
                            Kind::Pi => nd.name.clone(),
                            _ => Vec::new(),
                        }
                    }
                    None => Vec::new(),
                })
            }
            b"string" => {
                argn(0, 1)?;
                XVal::Str(str_arg(0)?)
            }
            b"concat" => {
                if args.len() < 2 {
                    return Err(XErr::Eval);
                }
                let mut out = Vec::new();
                for i in 0..args.len() {
                    out.extend(self.to_str(&ev(i)?));
                }
                XVal::Str(out)
            }
            b"starts-with" => {
                argn(2, 2)?;
                let (a, b) = (str_arg(0)?, str_arg(1)?);
                XVal::Bool(a.starts_with(&b))
            }
            b"ends-with" => {
                argn(2, 2)?;
                let (a, b) = (str_arg(0)?, str_arg(1)?);
                XVal::Bool(a.ends_with(&b))
            }
            b"contains" => {
                argn(2, 2)?;
                let (a, b) = (str_arg(0)?, str_arg(1)?);
                XVal::Bool(b.is_empty() || a.windows(b.len()).any(|w| w == &b[..]))
            }
            b"substring-before" | b"substring-after" => {
                argn(2, 2)?;
                let (a, b) = (str_arg(0)?, str_arg(1)?);
                let at = if b.is_empty() {
                    Some(0)
                } else {
                    a.windows(b.len()).position(|w| w == &b[..])
                };
                XVal::Str(match at {
                    None => Vec::new(),
                    Some(i) => {
                        if name == b"substring-before" {
                            a[..i].to_vec()
                        } else {
                            a[i + b.len()..].to_vec()
                        }
                    }
                })
            }
            b"substring" => {
                argn(2, 3)?;
                let s: Vec<char> = String::from_utf8_lossy(&str_arg(0)?).chars().collect();
                let start = self.to_num(&ev(1)?);
                let start_r = xround(start);
                let end_r = if args.len() > 2 {
                    start_r + xround(self.to_num(&ev(2)?))
                } else {
                    f64::INFINITY
                };
                let mut out = String::new();
                for (i, ch) in s.iter().enumerate() {
                    let p = (i + 1) as f64;
                    if p >= start_r && p < end_r {
                        out.push(*ch);
                    }
                }
                XVal::Str(out.into_bytes())
            }
            b"string-length" => {
                argn(0, 1)?;
                XVal::Num(String::from_utf8_lossy(&str_arg(0)?).chars().count() as f64)
            }
            b"normalize-space" => {
                argn(0, 1)?;
                let s = str_arg(0)?;
                let parts: Vec<&[u8]> = s
                    .split(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r'))
                    .filter(|p| !p.is_empty())
                    .collect();
                XVal::Str(parts.join(&b' '))
            }
            b"translate" => {
                argn(3, 3)?;
                let s: Vec<char> = String::from_utf8_lossy(&str_arg(0)?).chars().collect();
                let from: Vec<char> = String::from_utf8_lossy(&str_arg(1)?).chars().collect();
                let to: Vec<char> = String::from_utf8_lossy(&str_arg(2)?).chars().collect();
                let mut out = String::new();
                for ch in s {
                    match from.iter().position(|&f| f == ch) {
                        Some(i) => {
                            if let Some(&t) = to.get(i) {
                                out.push(t);
                            }
                        }
                        None => out.push(ch),
                    }
                }
                XVal::Str(out.into_bytes())
            }
            b"boolean" => {
                argn(1, 1)?;
                XVal::Bool(self.to_bool(&ev(0)?))
            }
            b"not" => {
                argn(1, 1)?;
                XVal::Bool(!self.to_bool(&ev(0)?))
            }
            b"true" => XVal::Bool(true),
            b"false" => XVal::Bool(false),
            b"lang" => XVal::Bool(false),
            b"number" => {
                argn(0, 1)?;
                if args.is_empty() {
                    XVal::Num(str_to_num(&string_value(self.doc, node)))
                } else {
                    XVal::Num(self.to_num(&ev(0)?))
                }
            }
            b"sum" => {
                argn(1, 1)?;
                match ev(0)? {
                    XVal::Nodes(n) => XVal::Num(
                        n.iter()
                            .map(|&x| str_to_num(&string_value(self.doc, x)))
                            .sum(),
                    ),
                    _ => return Err(XErr::Eval),
                }
            }
            b"floor" => XVal::Num(self.to_num(&ev(0)?).floor()),
            b"ceiling" => XVal::Num(self.to_num(&ev(0)?).ceil()),
            b"round" => XVal::Num(xround(self.to_num(&ev(0)?))),
            _ => return Err(XErr::UnknownFunction(name.to_vec())),
        })
    }
}

fn xround(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() {
        return n;
    }
    (n + 0.5).floor()
}

/// Compile and evaluate `expr` with context node `node`.
pub fn evaluate(
    doc: &Doc,
    expr: &[u8],
    node: NodeId,
    ns: &HashMap<Vec<u8>, Vec<u8>>,
    vars: &HashMap<Vec<u8>, XVal>,
) -> Result<XVal, XErr> {
    let e = compile(expr)?;
    let cx = Ctx { doc, ns, vars };
    cx.eval(&e, node, 1, 1)
}
