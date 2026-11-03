//! The parts of Bison's yacc.c skeleton (3.8.2) that decide whether a token
//! stream parses and how a syntax error is worded: the LR driver (the parser
//! runs the actions it needs through [`Parser::reduce`]),
//! `yypcontext_expected_tokens`, `yy_syntax_error_arguments`, `yysyntax_error`
//! and the default `yytnamerr`. The tables come from php's own Bison output
//! (`*_tables.rs`).
//!
//! The skeleton is GPL-3.0-or-later with the Bison exception, which lets a
//! parser built from it (and this port of the parts a parser uses) be
//! distributed under any terms; see THIRD-PARTY-NOTICES.

/// One Bison parser's tables (`yypact`, `yytable`, …) and constants.
pub struct Tables {
    pub final_state: i32,
    pub last: i32,
    pub ntokens: i32,
    pub maxutok: i32,
    pub pact_ninf: i32,
    pub table_ninf: i32,
    pub maxdepth: i32,
    pub translate: &'static [u8],
    pub pact: &'static [i16],
    pub defact: &'static [i16],
    pub pgoto: &'static [i16],
    pub defgoto: &'static [i16],
    pub table: &'static [i16],
    pub check: &'static [i16],
    pub r1: &'static [i16],
    pub r2: &'static [u8],
    pub tname: &'static [&'static str],
}

/// `YYSYMBOL_YYerror`.
const SYM_ERROR: i32 = 1;
/// `YYSYMBOL_YYUNDEF`.
const SYM_UNDEF: i32 = 2;
/// `YYARGS_MAX`: the unexpected token and at most four expected ones.
const ARGS_MAX: usize = 5;

/// The scanner and the grammar actions one parser runs over the automaton.
pub trait Parser {
    /// yylex: the next token (0 at the end, a character's code, or a token
    /// id) and its semantic value.
    fn lex(&mut self) -> (i32, u32);

    /// The action of `rule` over the values of its right-hand side: the
    /// rule's value, or `None` when the action raised an error (YYERROR or
    /// YYABORT; php's grammars have no `error` token, so both end the parse).
    /// Bison's default action is `$$ = $1`.
    fn reduce(&mut self, rule: usize, rhs: &[u32]) -> Option<u32> {
        let _ = rule;
        Some(rhs.first().copied().unwrap_or(0))
    }
}

/// A [`Parser`] that only scans.
struct Tokens<F>(F);

impl<F: FnMut() -> i32> Parser for Tokens<F> {
    fn lex(&mut self) -> (i32, u32) {
        ((self.0)(), 0)
    }
}

/// How a parse ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Accept,
    /// A grammar action raised an error.
    Aborted,
    /// A syntax error: `args` are the symbols `yysyntax_error` reports, the
    /// unexpected token first. Empty when there was no lookahead.
    SyntaxError {
        args: Vec<usize>,
    },
    /// The state stack reached `YYMAXDEPTH` ("memory exhausted").
    MemoryExhausted,
}

impl Tables {
    fn translate(&self, yychar: i32) -> i32 {
        if (0..=self.maxutok).contains(&yychar) {
            self.translate[yychar as usize] as i32
        } else {
            SYM_UNDEF
        }
    }

    fn table_at(&self, i: i32) -> Option<i32> {
        (0..=self.last)
            .contains(&i)
            .then(|| self.table[i as usize] as i32)
    }

    fn check_at(&self, i: i32) -> Option<i32> {
        (0..=self.last)
            .contains(&i)
            .then(|| self.check[i as usize] as i32)
    }

    /// Runs the automaton over the tokens `lex` returns (yylex's numbers:
    /// 0 at the end, a character's code, or a token id), with no actions,
    /// stopping at the first syntax error.
    pub fn parse(&self, lex: impl FnMut() -> i32) -> Outcome {
        self.run(&mut Tokens(lex))
    }

    /// Runs the automaton over `p`'s tokens and actions, stopping at the
    /// first syntax error or failed action.
    pub fn run(&self, p: &mut impl Parser) -> Outcome {
        let mut stack: Vec<i32> = vec![0];
        // the semantic values, one per stack entry
        let mut values: Vec<u32> = vec![0];
        let mut state = 0;
        // None: no lookahead (yychar == YYEMPTY)
        let mut lookahead: Option<(i32, u32)> = None;
        loop {
            if stack.len() as i32 >= self.maxdepth {
                return Outcome::MemoryExhausted;
            }
            if state == self.final_state {
                return Outcome::Accept;
            }
            let mut n = self.pact[state as usize] as i32;
            let mut rule = None;
            if n != self.pact_ninf {
                let (token, value) = *lookahead.get_or_insert_with(|| {
                    let (c, value) = p.lex();
                    (if c <= 0 { 0 } else { self.translate(c) }, value)
                });
                n += token;
                match (self.check_at(n), self.table_at(n)) {
                    (Some(c), Some(t)) if c == token => {
                        if t <= 0 {
                            if t == self.table_ninf {
                                return self.error(state, Some(token));
                            }
                            rule = Some(-t);
                        } else {
                            // shift
                            state = t;
                            stack.push(state);
                            values.push(value);
                            lookahead = None;
                            continue;
                        }
                    }
                    _ => {}
                }
            }
            let rule = match rule {
                Some(r) => r,
                None => {
                    let d = self.defact[state as usize] as i32;
                    if d == 0 {
                        return self.error(state, lookahead.map(|l| l.0));
                    }
                    d
                }
            };
            // reduce
            let len = self.r2[rule as usize] as usize;
            let base = stack.len() - len;
            let Some(value) = p.reduce(rule as usize, &values[base..]) else {
                return Outcome::Aborted;
            };
            stack.truncate(base);
            values.truncate(base);
            values.push(value);
            let top = *stack.last().unwrap();
            let lhs = self.r1[rule as usize] as i32 - self.ntokens;
            let i = self.pgoto[lhs as usize] as i32 + top;
            state = match (self.check_at(i), self.table_at(i)) {
                (Some(c), Some(t)) if c == top => t,
                _ => self.defgoto[lhs as usize] as i32,
            };
            stack.push(state);
        }
    }

    /// `yy_syntax_error_arguments` for the state where the error was found.
    fn error(&self, state: i32, lookahead: Option<i32>) -> Outcome {
        let mut args = Vec::new();
        if let Some(token) = lookahead {
            args.push(token as usize);
            args.extend(self.expected_tokens(state, ARGS_MAX - 1));
        }
        Outcome::SyntaxError { args }
    }

    /// `yypcontext_expected_tokens`: the expected tokens, or none when there
    /// are more than `max`.
    fn expected_tokens(&self, state: i32, max: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let n = self.pact[state as usize] as i32;
        if n == self.pact_ninf {
            return out;
        }
        let begin = if n < 0 { -n } else { 0 };
        let end = (self.last - n + 1).min(self.ntokens);
        for x in begin..end {
            let i = (x + n) as usize;
            if self.check[i] as i32 == x
                && x != SYM_ERROR
                && self.table[i] as i32 != self.table_ninf
            {
                if out.len() == max {
                    return Vec::new();
                }
                out.push(x as usize);
            }
        }
        out
    }
}

/// `yysyntax_error`'s message for `args` (unexpected first), naming each
/// symbol with `name(yytname entry, is_unexpected)`.
pub fn syntax_error_message(
    t: &Tables,
    args: &[usize],
    mut name: impl FnMut(&str, bool) -> Vec<u8>,
) -> Vec<u8> {
    let mut m = b"syntax error".to_vec();
    for (i, &a) in args.iter().enumerate() {
        m.extend_from_slice(match i {
            0 => b", unexpected ",
            1 => b", expecting ",
            _ => b" or ",
        });
        m.extend(name(t.tname[a], i == 0));
    }
    m
}

/// Bison's default `yytnamerr`: a `"…"` name loses its quotes unless it
/// holds `'`, `,` or a lone backslash; `\\` becomes `\`.
pub fn default_tnamerr(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    if b.first() == Some(&b'"') {
        let mut out = Vec::new();
        let mut i = 1;
        loop {
            match b.get(i) {
                Some(b'\'') | Some(b',') | None => break,
                Some(b'\\') => {
                    if b.get(i + 1) != Some(&b'\\') {
                        break;
                    }
                    out.push(b'\\');
                    i += 2;
                }
                Some(b'"') => return out,
                Some(&c) => {
                    out.push(c);
                    i += 1;
                }
            }
        }
    }
    b.to_vec()
}
