//! First-order mutation of PHP source at the token level.
//!
//! Each mutant changes exactly one token — a literal swapped for a boundary
//! value, or an operator for a near neighbour — so a php/rhp divergence on a
//! mutant points at one precise construct. Tokens come from rhp's own lexer
//! ([`rhp_compiler::parse::tokens`]), so comments, interpolated strings and inline HTML
//! are never corrupted by accident. Replacements that could re-lex differently
//! next to their neighbours (a negative literal after `-`) are parenthesised.
//!
//! Mutants are not guaranteed to be valid PHP (e.g. `|` in a union type becomes
//! `^`); the runner discards any mutant that php itself rejects with a parse
//! error, so only programs php accepts are compared.

use rhp_compiler::parse::{tokens, Token, TokenKind};

use crate::Rng;

/// One mutated program and a human-readable note of what changed.
pub struct Mutant {
    pub source: Vec<u8>,
    pub description: String,
}

/// A single-token rewrite of the original source.
struct Site {
    start: usize,
    end: usize,
    replacement: &'static str,
    computed: Option<String>,
}

impl Site {
    fn text(&self) -> &str {
        self.computed.as_deref().unwrap_or(self.replacement)
    }
}

const INT_REPLACEMENTS: &[&str] = &["0", "1", "(-1)", "2", "PHP_INT_MAX", "PHP_INT_MIN"];
const FLOAT_REPLACEMENTS: &[&str] = &[
    "0.0",
    "(-0.0)",
    "0.1",
    "1.5",
    "1e100",
    "NAN",
    "INF",
    "(-INF)",
    "PHP_FLOAT_EPSILON",
];
const STRING_REPLACEMENTS: &[&str] = &[
    "''", "'0'", "'1'", "' 1'", "'1e3'", "'abc'", "'0x1A'", "\"\\0\"", "'ä'",
];

/// Operator → near-neighbour replacements (each yields its own mutant).
fn operator_replacements(kind: TokenKind) -> &'static [&'static str] {
    use TokenKind::*;
    match kind {
        Plus => &["-", "*"],
        Minus => &["+"],
        Asterisk => &["/", "+"],
        Slash => &["*", "%"],
        Percent => &["/"],
        AsteriskAsterisk => &["*"],
        Dot => &["+"],
        LessThan => &["<=", ">"],
        LessThanEqual => &["<", ">="],
        GreaterThan => &[">=", "<"],
        GreaterThanEqual => &[">", "<="],
        EqualEqual => &["===", "!="],
        EqualEqualEqual => &["==", "!=="],
        BangEqual => &["!==", "=="],
        BangEqualEqual => &["!=", "==="],
        LessThanEqualGreaterThan => &["-"],
        AmpersandAmpersand => &["||"],
        PipePipe => &["&&"],
        Pipe => &["^"],
        Caret => &["|"],
        LeftShift => &[">>"],
        RightShift => &["<<"],
        PlusEqual => &["-="],
        MinusEqual => &["+="],
        DotEqual => &["+="],
        AsteriskEqual => &["/="],
        PlusPlus => &["--"],
        MinusMinus => &["++"],
        QuestionQuestion => &["?:"],
        True => &["false"],
        False => &["true"],
        Null => &["false"],
        _ => &[],
    }
}

fn is_trivia(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Whitespace
            | TokenKind::SingleLineComment
            | TokenKind::HashComment
            | TokenKind::MultiLineComment
            | TokenKind::DocBlockComment
    )
}

/// `n + 1` / `n - 1` for a plain decimal integer literal (underscores allowed).
fn int_neighbours(text: &[u8]) -> Vec<String> {
    let digits: String = text
        .iter()
        .filter(|&&b| b != b'_')
        .map(|&b| b as char)
        .collect();
    let decimal = digits.len() == 1 || !digits.starts_with('0');
    match digits.parse::<i64>() {
        Ok(n) if decimal => [n.checked_add(1), n.checked_sub(1)]
            .into_iter()
            .flatten()
            .map(|v| {
                if v < 0 {
                    format!("({v})")
                } else {
                    v.to_string()
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Every single-token mutation of `src`.
fn sites(src: &[u8]) -> Vec<Site> {
    let toks: Vec<Token> = tokens(src);
    let mut out = Vec::new();
    let mut declare_depth: Option<usize> = None; // paren depth inside `declare(...)`
    let mut prev: Option<TokenKind> = None; // previous non-trivia token
    for tok in &toks {
        let kind = tok.kind;
        if is_trivia(kind) {
            continue;
        }
        // `declare(strict_types=1)` and friends take literals only: leave them be.
        match (kind, declare_depth.as_mut()) {
            (TokenKind::LeftParenthesis, Some(d)) => *d += 1,
            (TokenKind::RightParenthesis, Some(d)) => {
                *d -= 1;
                if *d == 0 {
                    declare_depth = None;
                }
            }
            (TokenKind::LeftParenthesis, None) if prev == Some(TokenKind::Declare) => {
                declare_depth = Some(1)
            }
            _ => {}
        }
        let in_declare = declare_depth.is_some();
        let after_jump = matches!(prev, Some(TokenKind::Break | TokenKind::Continue));
        let text = &src[tok.start..tok.end];
        let fixed: &[&str] = match kind {
            _ if in_declare => &[],
            TokenKind::LiteralInteger if after_jump => &[], // `break 2;` needs a literal
            TokenKind::LiteralInteger => INT_REPLACEMENTS,
            TokenKind::LiteralFloat => FLOAT_REPLACEMENTS,
            TokenKind::LiteralString => STRING_REPLACEMENTS,
            other => operator_replacements(other),
        };
        for &r in fixed {
            if r.as_bytes() != text {
                out.push(Site {
                    start: tok.start,
                    end: tok.end,
                    replacement: r,
                    computed: None,
                });
            }
        }
        if kind == TokenKind::LiteralInteger && !in_declare && !after_jump {
            for n in int_neighbours(text) {
                if n.as_bytes() != text && !INT_REPLACEMENTS.contains(&n.as_str()) {
                    out.push(Site {
                        start: tok.start,
                        end: tok.end,
                        replacement: "",
                        computed: Some(n),
                    });
                }
            }
        }
        prev = Some(kind);
    }
    out
}

/// How many distinct single-token mutants `src` has.
pub fn count(src: &[u8]) -> usize {
    sites(src).len()
}

/// Up to `n` distinct first-order mutants of `src`, chosen deterministically
/// from `rng` (a seeded shuffle of every candidate site).
pub fn mutants(src: &[u8], rng: &mut Rng, n: usize) -> Vec<Mutant> {
    let mut all = sites(src);
    for i in (1..all.len()).rev() {
        all.swap(i, rng.below(i + 1));
    }
    all.truncate(n);
    all.into_iter()
        .map(|s| {
            let mut source = Vec::with_capacity(src.len() + 16);
            source.extend_from_slice(&src[..s.start]);
            source.extend_from_slice(s.text().as_bytes());
            source.extend_from_slice(&src[s.end..]);
            let line = src[..s.start].iter().filter(|&&b| b == b'\n').count() + 1;
            let original = String::from_utf8_lossy(&src[s.start..s.end]);
            Mutant {
                description: format!("line {line}: `{original}` -> `{}`", s.text()),
                source,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
