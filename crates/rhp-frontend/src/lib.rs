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
//! This is sound *and* sufficient because both execution tiers consume the AST
//! within a single scope: the bytecode compiler turns it into **owned**
//! `rhp_bytecode` (no `'arena` borrow escapes), and the tree-walking oracle
//! finishes its run before the arena drops. Only owned `Value`s / output bytes
//! ever leave the scope.

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
