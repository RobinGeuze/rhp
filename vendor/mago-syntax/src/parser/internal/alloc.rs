use mago_allocator::prelude::*;

use crate::cst::Sequence;
use crate::cst::TokenSeparatedSequence;
use crate::parser::Parser;
use crate::token::Token;

impl<'arena, A> Parser<'_, 'arena, A>
where
    A: Arena,
{
    /// Creates a new empty vector to collect a list's items.
    ///
    /// rhp: a heap vector, copied into the arena at its final length by
    /// [`Parser::sequence`] / [`Parser::token_separated`]. Growing a vector
    /// inside the bump arena left every outgrown buffer (and the spare
    /// capacity) behind.
    #[inline]
    #[must_use]
    pub fn new_vec<T>(&self) -> std::vec::Vec<T> {
        std::vec::Vec::new()
    }

    /// Creates a new vector with a single value.
    #[inline]
    #[must_use]
    pub fn new_vec_of<T>(&self, value: T) -> std::vec::Vec<T> {
        vec![value]
    }

    /// rhp: copies `nodes` into the arena at their exact length.
    #[inline]
    #[must_use]
    pub fn slice<T>(&self, nodes: std::vec::Vec<T>) -> &'arena [T] {
        if nodes.is_empty() { &[] } else { self.arena.alloc_slice_fill_iter(nodes) }
    }

    /// rhp: freezes collected nodes into an arena sequence.
    #[inline]
    #[must_use]
    pub fn sequence<T>(&self, nodes: std::vec::Vec<T>) -> Sequence<'arena, T> {
        Sequence::from_slice(self.slice(nodes))
    }

    /// rhp: freezes collected nodes and separators into an arena sequence.
    #[inline]
    #[must_use]
    pub fn token_separated<T>(
        &self,
        nodes: std::vec::Vec<T>,
        tokens: std::vec::Vec<Token<'arena>>,
    ) -> TokenSeparatedSequence<'arena, T> {
        TokenSeparatedSequence::from_slices(self.slice(nodes), self.slice(tokens))
    }

    /// Allocates a byte slice in the parser's arena.
    #[inline]
    #[must_use]
    pub fn bytes(&self, bytes: &[u8]) -> &'arena [u8] {
        self.arena.alloc_slice_copy(bytes)
    }
}
