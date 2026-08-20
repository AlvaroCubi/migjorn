//! The card: the atomic unit of the whole design.

use crate::kind::{CardKind, SyntaxKind};
use crate::lex::lex;
use crate::token::Token;
use std::ops::Range;

/// Slot value for a card that has not been placed in a [`crate::Cst`] yet.
pub(crate) const UNPLACED: u32 = u32::MAX;

/// One card, owning its exact current bytes and the tokens indexing them.
///
/// Every card is this same shape whether it was just parsed, never touched, or
/// edited a hundred times. There is no borrowed/owned split and no edit overlay,
/// which is what makes emission a uniform per-card memcpy and every edit local.
#[derive(Debug, Clone)]
pub struct Card {
    pub(crate) slot: u32,
    kind: CardKind,
    /// Whether lexing produced any `Unknown` token.
    ///
    /// Recorded here, in padding the struct already had, so that a consumer
    /// looking for malformed cards does not have to walk every token of every
    /// card. On a million-card model that walk is tens of millions of scattered
    /// reads — it dominated index building before this flag existed.
    has_unknown: bool,
    /// This card's exact current bytes, terminator included. Emission copies
    /// this verbatim, so an unedited card is byte-for-byte its original source.
    text: String,
    /// Tokens indexing `text`. They do **not** tile it: whitespace has no token,
    /// and the gaps between spans are exactly the whitespace.
    tokens: Vec<Token>,
}

impl Card {
    pub(crate) fn new(kind: CardKind, text: String) -> Card {
        let tokens = lex(&text, kind);
        // The tokens were just written, so this pass is cache-hot and nearly
        // free; doing it later would not be.
        let has_unknown = tokens.iter().any(|t| t.kind == SyntaxKind::Unknown);
        Card {
            slot: UNPLACED,
            kind,
            has_unknown,
            text,
            tokens,
        }
    }

    /// Whether this card contains any byte run the lexer could not classify.
    #[inline]
    pub fn has_unknown(&self) -> bool {
        self.has_unknown
    }

    /// Stable id, unique for the model's lifetime and never reused. A live
    /// handle stores this, so an edit to a *different* card cannot invalidate it.
    #[inline]
    pub fn slot(&self) -> u32 {
        self.slot
    }

    #[inline]
    pub fn kind(&self) -> CardKind {
        self.kind
    }

    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[inline]
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    /// Source text of token `i`.
    #[inline]
    pub fn token_text(&self, i: usize) -> &str {
        &self.text[self.tokens[i].range()]
    }

    /// Index of the first non-trivia token at or after `i`.
    pub fn skip_trivia(&self, mut i: usize) -> usize {
        while i < self.tokens.len() && self.tokens[i].is_trivia() {
            i += 1;
        }
        i
    }

    /// Replace one token's text, keeping its kind and every other token's
    /// identity. The following tokens' spans shift by the length delta.
    ///
    /// This is the renumber / value-edit primitive: O(tokens in this card), and
    /// no other card is touched.
    pub fn set_token_text(&mut self, i: usize, new: &str) {
        let span = self.tokens[i].range();
        let old_len = span.len();
        self.text.replace_range(span.clone(), new);
        self.tokens[i].len = new.len() as u32;
        let delta = new.len() as i64 - old_len as i64;
        if delta != 0 {
            for t in &mut self.tokens[i + 1..] {
                t.start = (t.start as i64 + delta) as u32;
            }
        }
    }

    /// Replace many tokens in one pass, rebuilding the card's text once.
    ///
    /// `edits` must be sorted by token index and free of duplicates. Renumbering
    /// a surface can touch thousands of tokens in a single geometry-heavy card;
    /// doing that as N independent [`Card::set_token_text`] calls would be
    /// quadratic in the card's length, so this exists.
    pub fn rewrite_tokens(&mut self, edits: &[(usize, &str)]) {
        if edits.is_empty() {
            return;
        }
        let extra: i64 = edits
            .iter()
            .map(|&(i, s)| s.len() as i64 - self.tokens[i].len as i64)
            .sum();
        let mut out = String::with_capacity((self.text.len() as i64 + extra).max(0) as usize);

        // Split the borrow so the text can be read while the tokens are updated.
        let Card { text, tokens, .. } = self;
        let mut cursor = 0usize;
        let mut delta = 0i64;
        let mut next = 0usize;
        for (i, token) in tokens.iter_mut().enumerate() {
            let start = token.start as usize;
            if next < edits.len() && edits[next].0 == i {
                let new = edits[next].1;
                out.push_str(&text[cursor..start]);
                out.push_str(new);
                cursor = start + token.len as usize;
                token.start = (start as i64 + delta) as u32;
                delta += new.len() as i64 - token.len as i64;
                token.len = new.len() as u32;
                next += 1;
            } else {
                token.start = (start as i64 + delta) as u32;
            }
        }
        out.push_str(&text[cursor..]);
        self.text = out;
    }

    /// Structural in-card edit: replace a byte range of the card's text and
    /// re-lex just this card. Used when the token structure itself changes
    /// (adding a parameter, inserting a density field). O(card length).
    pub fn splice(&mut self, range: Range<usize>, new: &str) {
        self.text.replace_range(range, new);
        self.tokens = lex(&self.text, self.kind);
        self.has_unknown = self.tokens.iter().any(|t| t.kind == SyntaxKind::Unknown);
    }

    #[inline]
    pub fn len_bytes(&self) -> usize {
        self.text.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::SyntaxKind;

    fn cell(text: &str) -> Card {
        Card::new(CardKind::Cell, text.to_owned())
    }

    #[test]
    fn set_token_text_shifts_following_spans() {
        let mut c = cell("1 1 -1.0 -1 imp:n=1 $ fuel\n");
        c.set_token_text(1, "22"); // material 1 -> 22
        assert_eq!(c.text(), "1 22 -1.0 -1 imp:n=1 $ fuel\n");
        // every other token still points at the right bytes
        assert_eq!(c.token_text(2), "-1.0");
        assert_eq!(c.token_text(3), "-1");
        assert_eq!(c.tokens().last().unwrap().kind, SyntaxKind::Comment);
        assert_eq!(c.token_text(c.tokens().len() - 1), "$ fuel");
    }

    #[test]
    fn rewrite_tokens_matches_repeated_single_edits() {
        let src = "2 0 (1 : (2 3)) -4 imp:n=1\n";
        let mut batch = cell(src);
        let mut one_by_one = cell(src);

        // tokens: 0:"2" 1:"0" 2:"(" 3:"1" 4:":" 5:"(" 6:"2" 7:"3" 8:")" 9:")" 10:"-4"
        // renumber surfaces 1,2,3,4 -> 101,102,103,-104
        let edits: Vec<(usize, &str)> = vec![(3, "101"), (6, "102"), (7, "103"), (10, "-104")];
        batch.rewrite_tokens(&edits);
        for &(i, s) in &edits {
            one_by_one.set_token_text(i, s);
        }

        assert_eq!(batch.text(), one_by_one.text());
        assert_eq!(batch.text(), "2 0 (101 : (102 103)) -104 imp:n=1\n");
        assert_eq!(batch.tokens(), one_by_one.tokens());
        assert_eq!(batch.token_text(10), "-104");
    }

    #[test]
    fn splice_relexes_only_this_card() {
        let mut c = cell("2 0 1 imp:n=0\n");
        // void -> material 5 with a placeholder density
        let at = c.tokens()[1].range();
        c.splice(at, "5 0");
        assert_eq!(c.text(), "2 5 0 1 imp:n=0\n");
        assert_eq!(c.token_text(1), "5");
        assert_eq!(c.token_text(2), "0");
        assert_eq!(c.token_text(3), "1");
    }
}
