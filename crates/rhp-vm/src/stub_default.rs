//! Evaluates a builtin parameter's default as the stubs print it (see
//! rhp_builtins::signatures): the constant expression php compiles in
//! zend_get_default_from_internal_arg_info for
//! ReflectionParameter::getDefaultValue(). The forms are gen_stub's:
//! literals, `[]` arrays, constants, class constants, unary and binary
//! operators, parentheses.

use rhp_semantics::ops::{self, BitOp};
use rhp_values::{ArrayKey, PhpArray, Thrown, Value};

/// A name the expression refers to.
pub(crate) enum Name<'a> {
    Const(&'a str),
    ClassConst(&'a str, &'a str),
}

/// Is the default a bare constant or class constant (what
/// isDefaultValueConstant() reports)? Returns its name.
pub(crate) fn constant_name(text: &str) -> Option<&str> {
    let ident = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'\\')
            && !s.as_bytes()[0].is_ascii_digit()
    };
    match text.split_once("::") {
        Some((c, n)) => (ident(c) && ident(n) && !n.eq_ignore_ascii_case("class")).then_some(text),
        None => (ident(text)
            && !["null", "true", "false"]
                .iter()
                .any(|k| k.eq_ignore_ascii_case(text)))
        .then_some(text),
    }
}

#[derive(Debug, PartialEq)]
enum Tok {
    Int(i64),
    Float(f64),
    Str(Vec<u8>),
    Ident(String),
    Op(&'static str),
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    const OPS: &[&str] = &[
        "::", "=>", "<<", ">>", "|", "&", "^", "+", "-", "*", "/", "%", ".", "~", "!", "(", ")",
        "[", "]", ",",
    ];
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.' || b[i] == b'_') {
                // A sign belongs to an exponent.
                if (b[i] == b'e' || b[i] == b'E')
                    && !src[start..].starts_with("0x")
                    && matches!(b.get(i + 1), Some(b'+' | b'-'))
                {
                    i += 1;
                }
                i += 1;
            }
            out.push(number(&src[start..i].replace('_', ""))?);
        } else if c == b'\'' || c == b'"' {
            let (s, n) = string(&b[i..])?;
            out.push(Tok::Str(s));
            i += n;
        } else if c.is_ascii_alphabetic() || c == b'_' || c == b'\\' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'\\') {
                i += 1;
            }
            out.push(Tok::Ident(src[start..i].to_string()));
        } else if let Some(op) = OPS.iter().find(|op| src[i..].starts_with(**op)) {
            out.push(Tok::Op(op));
            i += op.len();
        } else {
            return Err(format!("unexpected {:?}", c as char));
        }
    }
    Ok(out)
}

fn number(s: &str) -> Result<Tok, String> {
    let l = s.to_ascii_lowercase();
    let int = if let Some(h) = l.strip_prefix("0x") {
        i64::from_str_radix(h, 16).ok()
    } else if let Some(bn) = l.strip_prefix("0b") {
        i64::from_str_radix(bn, 2).ok()
    } else if let Some(o) = l.strip_prefix("0o") {
        i64::from_str_radix(o, 8).ok()
    } else if l.len() > 1 && l.starts_with('0') && l.bytes().all(|b| b.is_ascii_digit()) {
        i64::from_str_radix(&l[1..], 8).ok()
    } else if l.bytes().all(|b| b.is_ascii_digit()) {
        l.parse().ok()
    } else {
        None
    };
    match int {
        Some(n) => Ok(Tok::Int(n)),
        None => l
            .parse::<f64>()
            .map(Tok::Float)
            .map_err(|_| format!("bad number {s}")),
    }
}

/// A quoted literal at the start of `b`: its value and length.
fn string(b: &[u8]) -> Result<(Vec<u8>, usize), String> {
    let q = b[0];
    let mut out = Vec::new();
    let mut i = 1;
    while i < b.len() {
        let c = b[i];
        if c == q {
            return Ok((out, i + 1));
        }
        if c != b'\\' || i + 1 >= b.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let n = b[i + 1];
        if q == b'\'' {
            if n == b'\'' || n == b'\\' {
                out.push(n);
            } else {
                out.extend_from_slice(&[c, n]);
            }
            i += 2;
            continue;
        }
        i += 2;
        match n {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'v' => out.push(0x0b),
            b'f' => out.push(0x0c),
            b'e' => out.push(0x1b),
            b'\\' | b'$' | b'"' => out.push(n),
            b'x' if b.get(i).is_some_and(u8::is_ascii_hexdigit) => {
                let mut v = 0u8;
                let mut k = 0;
                while k < 2 && b.get(i).is_some_and(u8::is_ascii_hexdigit) {
                    v = v * 16 + (b[i] as char).to_digit(16).unwrap_or(0) as u8;
                    i += 1;
                    k += 1;
                }
                out.push(v);
            }
            b'0'..=b'7' => {
                let mut v = (n - b'0') as u32;
                let mut k = 1;
                while k < 3 && matches!(b.get(i), Some(b'0'..=b'7')) {
                    v = v * 8 + (b[i] - b'0') as u32;
                    i += 1;
                    k += 1;
                }
                out.push(v as u8);
            }
            _ => out.extend_from_slice(&[b'\\', n]),
        }
    }
    Err("unterminated string".into())
}

struct Parser<'t, 'f> {
    toks: &'t [Tok],
    at: usize,
    lookup: &'f mut dyn FnMut(Name) -> Result<Value, Thrown>,
}

fn prec(op: &str) -> Option<u8> {
    Some(match op {
        "|" => 1,
        "^" => 2,
        "&" => 3,
        "." => 4,
        "<<" | ">>" => 5,
        "+" | "-" => 6,
        "*" | "/" | "%" => 7,
        _ => return None,
    })
}

fn bad(msg: impl Into<String>) -> Thrown {
    Thrown::error(
        "Error",
        format!("cannot evaluate default value: {}", msg.into()),
    )
}

impl Parser<'_, '_> {
    fn peek_op(&self) -> Option<&'static str> {
        match self.toks.get(self.at) {
            Some(Tok::Op(o)) => Some(o),
            _ => None,
        }
    }

    fn eat(&mut self, op: &str) -> bool {
        if self.peek_op() == Some(op) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expr(&mut self, min: u8) -> Result<Value, Thrown> {
        let mut lhs = self.unary()?;
        while let Some(op) = self.peek_op() {
            let Some(p) = prec(op) else { break };
            if p < min {
                break;
            }
            self.at += 1;
            let rhs = self.expr(p + 1)?;
            lhs = match op {
                "|" => ops::bitwise(&lhs, &rhs, BitOp::Or),
                "^" => ops::bitwise(&lhs, &rhs, BitOp::Xor),
                "&" => ops::bitwise(&lhs, &rhs, BitOp::And),
                "<<" => ops::bitwise(&lhs, &rhs, BitOp::Shl),
                ">>" => ops::bitwise(&lhs, &rhs, BitOp::Shr),
                "." => ops::concat(&lhs, &rhs),
                "+" => ops::add(&lhs, &rhs)?,
                "-" => ops::sub(&lhs, &rhs)?,
                "*" => ops::mul(&lhs, &rhs)?,
                "/" => ops::div(&lhs, &rhs)?,
                _ => ops::modulo(&lhs, &rhs)?,
            };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Value, Thrown> {
        if self.eat("-") {
            return ops::negate(&self.unary()?);
        }
        if self.eat("+") {
            return ops::mul(&self.unary()?, &Value::Int(1));
        }
        if self.eat("~") {
            return Ok(ops::bitwise_not(&self.unary()?));
        }
        if self.eat("!") {
            return Ok(Value::Bool(!rhp_values::conv::to_bool(&self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Value, Thrown> {
        let Some(t) = self.toks.get(self.at) else {
            return Err(bad("unexpected end"));
        };
        self.at += 1;
        match t {
            Tok::Int(n) => Ok(Value::Int(*n)),
            Tok::Float(f) => Ok(Value::Float(*f)),
            Tok::Str(s) => Ok(Value::str(s.clone())),
            Tok::Op("(") => {
                let v = self.expr(0)?;
                if !self.eat(")") {
                    return Err(bad("missing )"));
                }
                Ok(v)
            }
            Tok::Op("[") => {
                let mut a = PhpArray::new();
                while !self.eat("]") {
                    let v = self.expr(0)?;
                    if self.eat("=>") {
                        let val = self.expr(0)?;
                        if let Some(k) = ArrayKey::from_value(&v) {
                            a.insert(k, val);
                        }
                    } else {
                        a.push(v);
                    }
                    if !self.eat(",") && self.peek_op() != Some("]") {
                        return Err(bad("bad array"));
                    }
                }
                Ok(Value::Array(a))
            }
            Tok::Ident(name) => {
                if self.eat("::") {
                    let Some(Tok::Ident(member)) = self.toks.get(self.at) else {
                        return Err(bad("bad class constant"));
                    };
                    self.at += 1;
                    if member.eq_ignore_ascii_case("class") {
                        return Ok(Value::str(
                            name.trim_start_matches('\\').as_bytes().to_vec(),
                        ));
                    }
                    return (self.lookup)(Name::ClassConst(name, member));
                }
                match name.to_ascii_lowercase().as_str() {
                    "null" => Ok(Value::Null),
                    "true" => Ok(Value::Bool(true)),
                    "false" => Ok(Value::Bool(false)),
                    _ => (self.lookup)(Name::Const(name)),
                }
            }
            Tok::Op(o) => Err(bad(format!("unexpected {o}"))),
        }
    }
}

/// Evaluates `text`, resolving names through `lookup`.
pub(crate) fn eval(
    text: &str,
    lookup: &mut dyn FnMut(Name) -> Result<Value, Thrown>,
) -> Result<Value, Thrown> {
    let toks = lex(text).map_err(bad)?;
    let mut p = Parser {
        toks: &toks,
        at: 0,
        lookup,
    };
    let v = p.expr(0)?;
    if p.at != toks.len() {
        return Err(bad("trailing input"));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The value, as `type:repr`.
    fn ev(text: &str) -> String {
        let v = eval(text, &mut |n| match n {
            Name::Const("A") => Ok(Value::Int(1)),
            Name::Const("B") => Ok(Value::Int(4)),
            _ => Err(bad("unknown")),
        });
        match v {
            Ok(Value::Int(n)) => format!("int:{n}"),
            Ok(Value::Float(f)) => format!("float:{f}"),
            Ok(Value::Str(s)) => format!("string:{:?}", s.as_bytes()),
            Ok(Value::Array(a)) => format!("array:{}", a.len()),
            Ok(_) => "other".into(),
            Err(_) => "error".into(),
        }
    }

    #[test]
    fn evaluates_stub_defaults() {
        assert_eq!(ev("A | B"), "int:5");
        assert_eq!(ev("2 * 1024 * 1024"), "int:2097152");
        assert_eq!(ev("-1"), "int:-1");
        assert_eq!(ev("0777"), "int:511");
        assert_eq!(ev("1.0"), "float:1");
        assert_eq!(ev("\" \\n\\x00\""), format!("string:{:?}", b" \n\0"));
        assert_eq!(ev("'a\\'b'"), format!("string:{:?}", b"a'b"));
        assert_eq!(ev("X::class"), format!("string:{:?}", b"X"));
        assert_eq!(ev("[]"), "array:0");
        assert_eq!(ev("C"), "error");
        assert_eq!(constant_name("PHP_INT_MAX"), Some("PHP_INT_MAX"));
        assert_eq!(constant_name("A::B"), Some("A::B"));
        assert_eq!(constant_name("A::class"), None);
        assert_eq!(constant_name("null"), None);
        assert_eq!(constant_name("A | B"), None);
    }
}
