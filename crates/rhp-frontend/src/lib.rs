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

    /// mago-syntax's first parse error, worded like php's (`syntax error,
    /// unexpected token ";"`) with its 1-based line. php's own verdict comes
    /// from `rhp_builtins::zend::syntax_error`; this one only matters for
    /// code php parses but mago does not.
    pub fn parse_error(&self) -> Option<(String, u32)> {
        use mago_syntax::error::{ParseError as E, SyntaxError as S};
        let src = self.program.source_text;
        let err = self.program.errors.first()?;
        let (offset, what) = match err {
            E::SyntaxError(S::UnexpectedToken(_, b, p) | S::UnrecognizedToken(_, b, p)) => {
                (p.offset, format!("character 0x{b:02X}"))
            }
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
        let end = (offset as usize).min(src.len());
        let line = src[..end].iter().filter(|&&c| c == b'\n').count() as u32 + 1;
        Some((format!("syntax error, unexpected {what}"), line))
    }
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
mod tests;
