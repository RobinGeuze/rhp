//! Zend/zend_ini_parser.y: the ini parser over [`super::ini_scanner`]'s
//! tokens, raising the scanner and parser callbacks' events, and the
//! wording of its syntax errors (from the Bison automaton in
//! [`super::ini_parser_tables`]).

use rhp_values::conv;

use crate::zend::ini_scanner::{lexer, trim_ws, Lexer, Tok, IV, MODE_TYPED};
use crate::Host;

// ---------------------------------------------------------------------------
// parser
// ---------------------------------------------------------------------------

pub(crate) struct Parser<'a, 'h> {
    pub(crate) lx: Lexer<'a>,
    pub(crate) tok: Tok,
    pub(crate) host: &'h dyn Host,
}

type PResult<T> = Result<T, ()>;

pub(crate) enum Event {
    Section(IV),
    Entry(IV, Option<IV>),
    PopEntry(IV, IV, IV),
}

/// `zend_ini_add_string`: concatenate, keeping op1's number flag.
fn add_string(a: IV, b: IV) -> IV {
    let flag = a.is_number();
    let mut s = a.into_bytes();
    s.extend_from_slice(&b.into_bytes());
    IV::Str(s, flag)
}

impl Parser<'_, '_> {
    fn bump(&mut self) -> Tok {
        let next = self.lx.next();
        std::mem::replace(&mut self.tok, next)
    }

    fn is_ch(&self, c: u8) -> bool {
        matches!(self.tok, Tok::Ch(x) if x == c)
    }

    fn expect_ch(&mut self, c: u8) -> PResult<()> {
        if self.is_ch(c) {
            self.bump();
            Ok(())
        } else {
            Err(())
        }
    }

    pub(crate) fn statement_list(&mut self, ev: &mut Vec<Event>) -> PResult<()> {
        loop {
            match self.tok.clone() {
                Tok::End => return Ok(()),
                Tok::Eol => {
                    self.bump();
                }
                Tok::Section => {
                    self.bump();
                    let v = if self.starts_section_item() {
                        self.var_string_list(true)?
                    } else {
                        IV::s(b"")
                    };
                    self.expect_ch(b']')?;
                    ev.push(Event::Section(v));
                }
                Tok::Label(name) => {
                    self.bump();
                    if self.is_ch(b'=') {
                        self.bump();
                        let v = self.string_or_value()?;
                        ev.push(Event::Entry(name, Some(v)));
                    } else {
                        ev.push(Event::Entry(name, None));
                    }
                }
                Tok::Offset(name) => {
                    self.bump();
                    let off = if self.starts_item() {
                        self.var_string_list(false)?
                    } else {
                        IV::s(b"")
                    };
                    self.expect_ch(b']')?;
                    self.expect_ch(b'=')?;
                    let v = self.string_or_value()?;
                    ev.push(Event::PopEntry(name, v, off));
                }
                _ => return Err(()),
            }
        }
    }

    fn string_or_value(&mut self) -> PResult<IV> {
        match self.tok.clone() {
            Tok::BoolTrue(v) | Tok::BoolFalse(v) | Tok::Null(v) => {
                self.bump();
                Ok(v)
            }
            Tok::Eol => {
                self.bump();
                Ok(IV::s(b""))
            }
            _ => {
                let v = self.expr()?;
                Ok(self.normalize(v))
            }
        }
    }

    /// `normalize_value`: TYPED mode turns number-flagged strings into
    /// int/float (an overflowing integer string stays a string).
    fn normalize(&self, v: IV) -> IV {
        if self.lx.mode != MODE_TYPED {
            return v;
        }
        if let IV::Str(s, true) = &v {
            match conv::numeric_string(s) {
                Some(conv::Num::Int(i)) => return IV::Long(i),
                Some(conv::Num::Float(f)) => {
                    let t = trim_ws(s);
                    let intlike = t
                        .iter()
                        .enumerate()
                        .all(|(k, c)| c.is_ascii_digit() || (k == 0 && (*c == b'-' || *c == b'+')));
                    if !intlike {
                        return IV::Double(f);
                    }
                }
                None => {}
            }
        }
        v
    }

    /// expr with '|' '&' '^' left-associative at one level; '~' '!' unary.
    fn expr(&mut self) -> PResult<IV> {
        let mut lhs = self.unary()?;
        while let Tok::Ch(c @ (b'|' | b'&' | b'^')) = self.tok {
            self.bump();
            let rhs = self.unary()?;
            lhs = self.do_op(c, lhs, Some(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> PResult<IV> {
        match self.tok {
            Tok::Ch(c @ (b'~' | b'!')) => {
                self.bump();
                let v = self.unary()?;
                Ok(self.do_op(c, v, None))
            }
            Tok::Ch(b'(') => {
                self.bump();
                let v = self.expr()?;
                self.expect_ch(b')')?;
                Ok(v)
            }
            _ => {
                if self.starts_item() {
                    self.var_string_list(false)
                } else {
                    Err(())
                }
            }
        }
    }

    fn int_val(v: &IV) -> i32 {
        match v {
            IV::Long(n) => *n as i32,
            IV::Str(s, _) => crate::c_atoi(s),
            IV::Bool(b) => *b as i32,
            IV::Null => 0,
            IV::Double(f) => *f as i32,
        }
    }

    fn do_op(&self, op: u8, a: IV, b: Option<IV>) -> IV {
        let x = Self::int_val(&a);
        let y = b.as_ref().map(Self::int_val).unwrap_or(0);
        let r = match op {
            b'|' => x | y,
            b'&' => x & y,
            b'^' => x ^ y,
            b'~' => !x,
            b'!' => (x == 0) as i32,
            _ => 0,
        };
        if self.lx.mode != MODE_TYPED {
            IV::s(r.to_string().as_bytes())
        } else {
            IV::Long(r as i64)
        }
    }

    fn starts_item(&self) -> bool {
        matches!(
            self.tok,
            Tok::DollarCurly
                | Tok::Constant(_)
                | Tok::Raw(_)
                | Tok::Number(_)
                | Tok::Str(_)
                | Tok::Ws(_)
                | Tok::Ch(b'"')
        )
    }

    fn starts_section_item(&self) -> bool {
        self.starts_item()
    }

    /// var_string_list / var_string_list_section (`section` = no constant
    /// substitution and no number flag).
    fn var_string_list(&mut self, section: bool) -> PResult<IV> {
        let mut acc: Option<IV> = None;
        while self.starts_item() {
            let item = match self.bump() {
                Tok::DollarCurly => self.cfg_var_ref()?,
                Tok::Constant(IV::Str(name, _)) => {
                    if section {
                        IV::Str(name, false)
                    } else {
                        self.get_constant(name)
                    }
                }
                Tok::Number(IV::Str(n, _)) => IV::Str(n, !section),
                Tok::Raw(v) | Tok::Str(v) | Tok::Ws(v) => v,
                Tok::Ch(b'"') => {
                    let v = self.encapsed_list()?;
                    self.expect_ch(b'"')?;
                    v
                }
                _ => return Err(()),
            };
            acc = Some(match acc {
                None => item,
                Some(a) => add_string(a, item),
            });
        }
        acc.ok_or(())
    }

    fn encapsed_list(&mut self) -> PResult<IV> {
        let mut acc = IV::s(b"");
        loop {
            match self.tok {
                Tok::DollarCurly => {
                    self.bump();
                    let v = self.cfg_var_ref()?;
                    acc = add_string(acc, v);
                }
                Tok::Quoted(_) => {
                    let Tok::Quoted(v) = self.bump() else {
                        unreachable!()
                    };
                    acc = add_string(acc, v);
                }
                _ => return Ok(acc),
            }
        }
    }

    /// After TC_DOLLAR_CURLY: TC_VARNAME '}' | TC_VARNAME TC_FALLBACK fallback '}'.
    fn cfg_var_ref(&mut self) -> PResult<IV> {
        let Tok::Varname(IV::Str(name, _)) = self.bump() else {
            return Err(());
        };
        let mut fallback = None;
        if matches!(self.tok, Tok::Fallback) {
            self.bump();
            fallback = Some(if self.starts_item() {
                self.var_string_list(false)?
            } else {
                IV::s(b"")
            });
        }
        self.expect_ch(b'}')?;
        // zend_ini_get_var: the configuration hash, then the environment.
        if let Some(rhp_values::Value::Str(v)) = crate::main::ini::cfg_get(&name) {
            return Ok(IV::s(v.as_bytes()));
        }
        if let Some(v) = crate::ext::standard::basic_functions::env_get(&name) {
            return Ok(IV::s(&v));
        }
        match fallback {
            Some(f) => {
                // zend_ini_get_var copies the fallback with strlen().
                let b = f.into_bytes();
                let b = match b.iter().position(|&c| c == 0) {
                    Some(p) => b[..p].to_vec(),
                    None => b,
                };
                Ok(IV::Str(b, false))
            }
            None => Ok(IV::s(b"")),
        }
    }

    fn get_constant(&self, name: Vec<u8>) -> IV {
        if !name.contains(&b':') {
            if let Some(v) = self.host.get_const(&name) {
                return IV::Str(conv::to_string_bytes(&v), false);
            }
        }
        IV::Str(name, false)
    }
}

/// zend_ini_parser.y's verdict on `src`: its syntax error message and line
/// (zend_ini_scanner_get_lineno) when the grammar rejects it.
pub(crate) fn syntax_error(src: &[u8], mode: i64) -> Option<(Vec<u8>, u32)> {
    use crate::zend::bison::{self, Outcome};
    use crate::zend::ini_parser_tables::{self as t, TABLES};
    let mut lx = lexer(src, mode);
    let outcome = TABLES.parse(|| match lx.next() {
        Tok::End => 0,
        Tok::Section => t::TC_SECTION,
        Tok::Raw(_) => t::TC_RAW,
        Tok::Constant(_) => t::TC_CONSTANT,
        Tok::Number(_) => t::TC_NUMBER,
        Tok::Str(_) => t::TC_STRING,
        Tok::Ws(_) => t::TC_WHITESPACE,
        Tok::Label(_) => t::TC_LABEL,
        Tok::Offset(_) => t::TC_OFFSET,
        Tok::DollarCurly => t::TC_DOLLAR_CURLY,
        Tok::Varname(_) => t::TC_VARNAME,
        Tok::Quoted(_) => t::TC_QUOTED_STRING,
        Tok::Fallback => t::TC_FALLBACK,
        Tok::BoolTrue(_) => t::BOOL_TRUE,
        Tok::BoolFalse(_) => t::BOOL_FALSE,
        Tok::Null(_) => t::NULL_NULL,
        Tok::Eol => t::END_OF_LINE,
        Tok::Ch(c) => c as i32,
    });
    match outcome {
        // no actions run, so none fails
        Outcome::Accept | Outcome::Aborted => None,
        Outcome::MemoryExhausted => Some((b"memory exhausted".to_vec(), lx.lineno)),
        Outcome::SyntaxError { args } => Some((
            bison::syntax_error_message(&TABLES, &args, |n, _| bison::default_tnamerr(n)),
            lx.lineno,
        )),
    }
}
