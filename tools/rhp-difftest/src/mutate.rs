//! First-order mutation of PHP source at the token level.
//!
//! Each mutant changes exactly one token — a literal swapped for a boundary
//! value, or an operator for a near neighbour — so a php/rhp divergence on a
//! mutant points at one precise construct. Tokens come from rhp's own lexer
//! ([`rhp_frontend::tokens`]), so comments, interpolated strings and inline HTML
//! are never corrupted by accident. Replacements that could re-lex differently
//! next to their neighbours (a negative literal after `-`) are parenthesised.
//!
//! Mutants are not guaranteed to be valid PHP (e.g. `|` in a union type becomes
//! `^`); the runner discards any mutant that php itself rejects with a parse
//! error, so only programs php accepts are compared.

use rhp_frontend::{tokens, Token, TokenKind};

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
mod tests {
    use super::*;

    fn all(src: &str) -> Vec<String> {
        let n = count(src.as_bytes());
        mutants(src.as_bytes(), &mut Rng::new(1), n)
            .into_iter()
            .map(|m| String::from_utf8(m.source).unwrap())
            .collect()
    }

    #[test]
    fn mutates_each_literal_and_operator_once_per_replacement() {
        let ms = all("<?php echo 1 + 2;");
        assert!(ms.contains(&"<?php echo 1 - 2;".to_string()));
        assert!(ms.contains(&"<?php echo PHP_INT_MAX + 2;".to_string()));
        assert!(ms.contains(&"<?php echo 1 + 3;".to_string())); // n + 1
        assert!(ms.contains(&"<?php echo (-1) + 2;".to_string())); // parenthesised
        assert!(ms.iter().all(|m| m != "<?php echo 1 + 2;")); // never the identity
    }

    #[test]
    fn leaves_comments_strings_with_interpolation_and_html_alone() {
        let src = "<html>1 + 2</html><?php // 1 + 2\n$x = \"a $y + 1\"; /* < */";
        for m in all(src) {
            assert!(m.starts_with("<html>1 + 2</html><?php // 1 + 2\n"), "{m}");
            assert!(m.contains("/* < */"), "{m}");
        }
    }

    #[test]
    fn skips_declare_and_jump_literals() {
        let ms = all("<?php declare(strict_types=1); while (1) { break 2; }");
        assert!(ms.iter().all(|m| m.contains("declare(strict_types=1)")));
        assert!(ms.iter().all(|m| m.contains("break 2;")));
        // ...but the `while (1)` literal is still mutated.
        assert!(ms.iter().any(|m| m.contains("while (0)")));
    }

    #[test]
    fn deterministic_and_bounded() {
        let src = b"<?php $a = 3 < 4 && 'x' == \"y\"; echo $a ? 1.5 : null;";
        let a: Vec<_> = mutants(src, &mut Rng::new(9), 5)
            .into_iter()
            .map(|m| m.description)
            .collect();
        let b: Vec<_> = mutants(src, &mut Rng::new(9), 5)
            .into_iter()
            .map(|m| m.description)
            .collect();
        assert_eq!(a.len(), 5);
        assert_eq!(a, b);
    }

    #[test]
    fn neighbours_only_for_decimal() {
        assert_eq!(int_neighbours(b"10"), vec!["11", "9"]);
        assert_eq!(int_neighbours(b"0"), vec!["1", "(-1)"]);
        assert_eq!(int_neighbours(b"1_000"), vec!["1001", "999"]);
        assert!(int_neighbours(b"0x1F").is_empty());
        assert!(int_neighbours(b"017").is_empty());
    }
}
