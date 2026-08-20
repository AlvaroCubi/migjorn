//! Tokens: a kind plus a byte range **within the owning card's text**.

use crate::kind::SyntaxKind;
use std::ops::Range;

/// One token of a card.
///
/// Spans are relative to the card's own `text`, so a card is self-contained and
/// an edit to one card never perturbs another's tokens. Whitespace produces no
/// token, so consecutive tokens may have a gap between them — that gap *is* the
/// whitespace, and it is recovered from the card's text when needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: SyntaxKind,
    pub start: u32,
    pub len: u32,
}

impl Token {
    #[inline]
    pub fn new(kind: SyntaxKind, start: usize, len: usize) -> Token {
        Token {
            kind,
            start: start as u32,
            len: len as u32,
        }
    }

    #[inline]
    pub fn end(&self) -> u32 {
        self.start + self.len
    }

    #[inline]
    pub fn range(&self) -> Range<usize> {
        self.start as usize..self.end() as usize
    }

    #[inline]
    pub fn is_trivia(&self) -> bool {
        self.kind.is_trivia()
    }
}
