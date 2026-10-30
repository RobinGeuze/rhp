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
use mago_syntax::parser::parse_file_content;

pub use mago_syntax::cst;

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
}
