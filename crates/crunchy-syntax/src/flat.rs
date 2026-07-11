//! Prototype of a custom, lossless *flat arena* syntax tree.
//!
//! Rationale (M0 finding): rowan's `GreenNodeBuilder` interns every token
//! through a hashmap, which costs ~10x the lexer on large MCNP decks. Because
//! MCNP tokens tile the source exactly, we don't need to copy or intern token
//! text at all — we store each token as `(kind, len)` and recover its text as a
//! span into the original source. Node structure is a flat list of boundaries.
//!
//! This keeps allocation to a few contiguous `Vec`s (cache-friendly, no
//! per-node heap traffic) while remaining fully lossless.

use crate::{lex, SyntaxKind};

/// A lossless syntax tree stored as parallel arrays over the original source.
///
/// Token `i` covers `src[offset(i) .. offset(i) + lens[i]]`, where offsets are
/// the running prefix sum of `lens` (tokens tile the source with no gaps).
pub struct FlatTree {
    /// Token kinds, in source order.
    kinds: Vec<u16>,
    /// Token byte lengths, in source order (start offsets are implicit).
    lens: Vec<u32>,
    /// Token index at which each `LINE` node begins (node boundaries).
    line_starts: Vec<u32>,
}

impl FlatTree {
    /// Number of tokens.
    pub fn token_count(&self) -> usize {
        self.kinds.len()
    }

    /// Number of `LINE` nodes.
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Reconstruct the original source by walking the tree (not by returning a
    /// cached copy), proving navigation + losslessness.
    pub fn to_string(&self, src: &str) -> String {
        let mut out = String::with_capacity(src.len());
        let mut off = 0usize;
        for &len in &self.lens {
            let len = len as usize;
            out.push_str(&src[off..off + len]);
            off += len;
        }
        out
    }
}

/// Build a lossless flat tree from `src`, grouped into one `LINE` node per line.
pub fn build_flat_tree(src: &str) -> FlatTree {
    // Heuristic reservations to avoid reallocation churn on large inputs.
    let approx_tokens = src.len() / 4 + 16;
    let mut kinds = Vec::with_capacity(approx_tokens);
    let mut lens = Vec::with_capacity(approx_tokens);
    let mut line_starts = Vec::with_capacity(src.len() / 32 + 16);

    let mut at_line_start = true;
    lex(src.as_bytes(), |kind, _start, len| {
        if at_line_start {
            line_starts.push(kinds.len() as u32);
            at_line_start = false;
        }
        kinds.push(kind as u16);
        lens.push(len);
        if kind == SyntaxKind::NEWLINE {
            at_line_start = true;
        }
    });

    FlatTree {
        kinds,
        lens,
        line_starts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_roundtrips() {
        let src = "\u{feff}c title\n1 0 -1 imp:n=1 $ c\n\n2 PX 3.0\n";
        let tree = build_flat_tree(src);
        assert_eq!(tree.to_string(src), src);
    }

    #[test]
    fn counts_lines() {
        let src = "1 PX 1\n2 PY 2\n3 PZ 3\n";
        let tree = build_flat_tree(src);
        assert_eq!(tree.line_count(), 3);
    }
}
