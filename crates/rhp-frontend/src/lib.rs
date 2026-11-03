//! Thin, **safe** wrapper over `mago-syntax`: owns the parse arena and exposes
//! a scoped borrow of the AST so the rest of rhp can be written against a
//! stable surface (and the parser swapped later).
//!
//! mago-syntax is arena-allocated — a parsed `Program` borrows the arena. Rather
//! than store a self-referential `arena + &Program` (which needs `unsafe`), the
//! caller holds the [`Arena`] on the stack and [`Arena::parse`] hands back a
//! [`Parsed`] whose lifetime is tied to it. The borrow checker then guarantees
//! the AST cannot outlive the arena — no `unsafe` anywhere.
//!
//! This is sound *and* sufficient because the AST is consumed within a single
//! scope: the bytecode compiler turns it into **owned** `rhp_bytecode` (no
//! `'arena` borrow escapes) before the arena drops.

use mago_allocator::LocalArena;
use mago_database::file::FileId;
use mago_syntax::cst::Program;
use mago_syntax::lexer::Lexer;
use mago_syntax::parser::parse_file_content;
use mago_syntax::settings::LexerSettings;
use mago_syntax_core::input::Input;

pub use mago_syntax::cst;
pub use mago_syntax::token::TokenKind;

/// One lexical token of a source file: its kind and byte range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
}

/// Lex `source` into its full token stream (whitespace, comments and inline
/// HTML included), stopping at the first lexical error. Byte ranges index into
/// `source`, so callers can rewrite individual tokens in place.
pub fn tokens(source: &[u8]) -> Vec<Token> {
    let input = Input::new(FileId::new(b"tokens"), source);
    let mut lexer = Lexer::new(input, LexerSettings::default());
    let mut out = Vec::new();
    while let Some(Ok(tok)) = lexer.advance() {
        let start = tok.start.offset as usize;
        out.push(Token {
            kind: tok.kind,
            start,
            end: start + tok.value.len(),
        });
    }
    out
}

/// Owns a parse arena. Create one per source file, parse into it, use the AST,
/// then drop it. Cheap to create.
pub struct Arena {
    arena: LocalArena,
}

impl Default for Arena {
    fn default() -> Self {
        Arena::new()
    }
}

impl Arena {
    pub fn new() -> Self {
        Arena {
            arena: LocalArena::new(),
        }
    }

    /// Parse `source` under a logical file name. The returned [`Parsed`] borrows
    /// this arena; it (and the AST behind it) cannot outlive `self`.
    ///
    /// `source` is copied into the arena, so it need not outlive the call.
    pub fn parse<'a>(&'a self, logical_name: &[u8], source: &[u8]) -> Parsed<'a> {
        let file_id = FileId::new(logical_name);
        let program = parse_file_content(&self.arena, file_id, source);
        Parsed { program }
    }
}

/// A parsed program borrowing its [`Arena`].
pub struct Parsed<'a> {
    program: &'a Program<'a>,
}

impl<'a> Parsed<'a> {
    pub fn program(&self) -> &Program<'a> {
        self.program
    }

    pub fn has_parse_errors(&self) -> bool {
        self.program.has_errors()
    }

    /// The first parse error as php words it (`syntax error, unexpected
    /// token ";"`, zend_yytnamerr's token descriptions) and its 1-based line.
    /// php's `, expecting …` suffix and its special unclosed-bracket messages
    /// are not reproduced.
    pub fn parse_error(&self) -> Option<(String, u32)> {
        use mago_syntax::error::{ParseError as E, SyntaxError as S};
        let src = self.program.source_text;
        let err = self.program.errors.first()?;
        let (offset, what) = match err {
            E::SyntaxError(S::UnexpectedToken(_, b, p) | S::UnrecognizedToken(_, b, p)) => {
                (p.offset, format!("character 0x{b:02X}"))
            }
            // php's scanner is at the very end of the input then.
            E::SyntaxError(S::UnexpectedEndOfFile(..)) | E::UnexpectedEndOfFile(..) => {
                (src.len() as u32, "end of file".to_string())
            }
            E::SyntaxError(S::RecursionLimitExceeded(_, p)) => {
                (p.offset, "end of file".to_string())
            }
            E::UnexpectedToken(_, kind, span) => {
                let (a, b) = (span.start.offset as usize, span.end.offset as usize);
                let text = src.get(a..b).unwrap_or_default();
                (span.start.offset, describe_token(*kind, text))
            }
            E::UnclosedLiteralString(_, span) | E::RecursionLimitExceeded(span) => {
                (span.start.offset, "end of file".to_string())
            }
        };
        let line_at = |off: usize| {
            let end = off.min(src.len());
            src[..end].iter().filter(|&&c| c == b'\n').count() as u32 + 1
        };
        // The scanner's bracket-nesting errors fire as soon as it reaches the
        // offending token, before the parser sees it.
        if let Some((noff, msg)) = nesting_error(src, &line_at) {
            if noff <= offset as usize {
                return Some((msg, line_at(noff)));
            }
        }
        Some((
            format!("syntax error, unexpected {what}"),
            line_at(offset as usize),
        ))
    }
}

/// The scanner's enter_nesting/exit_nesting/check_nesting_at_end: the offset
/// of the first bracket-nesting error and php's message for it.
fn nesting_error(src: &[u8], line_at: &dyn Fn(usize) -> u32) -> Option<(usize, String)> {
    let mut stack: Vec<(u8, u32)> = Vec::new();
    let bad = |opening: u8, open_line: u32, at_line: u32, closing: Option<u8>| {
        let mut m = format!("Unclosed '{}'", opening as char);
        if open_line != at_line {
            m.push_str(&format!(" on line {open_line}"));
        }
        if let Some(c) = closing {
            m.push_str(&format!(" does not match '{}'", c as char));
        }
        m
    };
    for t in tokens(src) {
        let open = match t.kind {
            TokenKind::LeftBrace | TokenKind::DollarLeftBrace => Some(b'{'),
            TokenKind::LeftBracket | TokenKind::HashLeftBracket => Some(b'['),
            TokenKind::LeftParenthesis => Some(b'('),
            _ => None,
        };
        if let Some(o) = open {
            stack.push((o, line_at(t.start)));
            continue;
        }
        let close = match t.kind {
            TokenKind::RightBrace => b'}',
            TokenKind::RightBracket => b']',
            TokenKind::RightParenthesis => b')',
            _ => continue,
        };
        let Some(&(o, ol)) = stack.last() else {
            return Some((t.start, format!("Unmatched '{}'", close as char)));
        };
        let matches = matches!((o, close), (b'{', b'}') | (b'[', b']') | (b'(', b')'));
        if !matches {
            return Some((t.start, bad(o, ol, line_at(t.start), Some(close))));
        }
        stack.pop();
    }
    let &(o, ol) = stack.last()?;
    Some((src.len(), bad(o, ol, line_at(src.len()), None)))
}

/// zend_yytnamerr for the unexpected token: `token "x"` for fixed-form
/// tokens, `<kind> "<content>"` for the variable ones.
fn describe_token(kind: TokenKind, text: &[u8]) -> String {
    let kind_name = match kind {
        TokenKind::Identifier => "identifier",
        TokenKind::FullyQualifiedIdentifier => "fully qualified name",
        TokenKind::QualifiedIdentifier => "namespaced name",
        TokenKind::Variable => "variable",
        TokenKind::LiteralInteger => "integer",
        TokenKind::LiteralFloat => "floating-point number",
        TokenKind::StringPart => "string content",
        TokenKind::InlineText => "T_INLINE_HTML",
        TokenKind::LiteralString | TokenKind::PartialLiteralString => match text.first() {
            Some(b'"') => "double-quoted string",
            Some(b'\'') => "single-quoted string",
            _ => "quoted string",
        },
        _ => {
            let t = String::from_utf8_lossy(text).to_ascii_lowercase();
            if t == "\"" {
                return "double-quote mark".into();
            }
            return format!("token \"{t}\"");
        }
    };
    let mut c = text;
    if let Some(i) = c.iter().position(|&b| b == b'\n') {
        c = &c[..i];
    }
    if matches!(c.first(), Some(b'\'' | b'"')) {
        c = &c[1..];
    }
    if matches!(c.last(), Some(b'\'' | b'"')) {
        c = &c[..c.len() - 1];
    }
    let c = String::from_utf8_lossy(c);
    if c.len() > 33 {
        let cut: String = c.chars().take(30).collect();
        format!("{kind_name} \"{cut}...\"")
    } else {
        format!("{kind_name} \"{c}\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_echo() {
        let arena = Arena::new();
        let parsed = arena.parse(b"t.php", br#"<?php echo "hi", 1+2;"#);
        assert!(!parsed.has_parse_errors());
        assert!(!parsed.program().statements.is_empty());
    }

    #[test]
    fn tokens_cover_source_with_exact_ranges() {
        let src = b"<?php $a = 10 <= 2.5; // c\necho 'x' . \"y\";";
        let toks = tokens(src);
        // Contiguous, gap-free cover of the whole source.
        assert_eq!(toks.first().unwrap().start, 0);
        assert_eq!(toks.last().unwrap().end, src.len());
        for w in toks.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
        let text = |k: TokenKind| {
            let t = toks.iter().find(|t| t.kind == k).unwrap();
            &src[t.start..t.end]
        };
        assert_eq!(text(TokenKind::LiteralInteger), b"10");
        assert_eq!(text(TokenKind::LiteralFloat), b"2.5");
        assert_eq!(text(TokenKind::LessThanEqual), b"<=");
        assert_eq!(text(TokenKind::LiteralString), b"'x'");
        assert_eq!(text(TokenKind::Dot), b".");
    }
}
