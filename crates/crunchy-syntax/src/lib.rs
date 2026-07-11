//! `crunchy-syntax` — the lossless syntax layer for the crunchy MCNP parser.
//!
//! This crate owns the lexer and the concrete syntax tree (CST). Higher layers
//! (`crunchy-core`) project a typed AST on top of it.

mod flat;
mod lexer;
mod syntax_kind;

pub use flat::{build_flat_tree, FlatTree};
pub use lexer::lex;
pub use syntax_kind::{McnpLang, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

use rowan::{GreenNode, GreenNodeBuilder};

/// Build a lossless CST from `src`, grouping tokens into one `LINE` node per
/// physical source line.
///
/// This is the M0 spike shape: it exercises the full lex + green-tree build
/// cost so we can measure it, and it round-trips byte-for-byte. Card-level
/// grouping (which produces *fewer* nodes) replaces the per-line grouping in
/// M1+, so this is a conservative upper bound on tree-construction cost.
pub fn build_line_tree(src: &str) -> GreenNode {
    let mut builder = GreenNodeBuilder::new();
    builder.start_node(SyntaxKind::ROOT.into());

    let mut line_open = false;
    lex(src.as_bytes(), |kind, start, len| {
        if !line_open {
            builder.start_node(SyntaxKind::LINE.into());
            line_open = true;
        }
        let text = &src[start as usize..(start + len) as usize];
        builder.token(kind.into(), text);
        if kind == SyntaxKind::NEWLINE {
            builder.finish_node();
            line_open = false;
        }
    });
    if line_open {
        builder.finish_node();
    }
    builder.finish_node();
    builder.finish()
}

/// Parse `src` into a lossless `SyntaxNode` (M0 line-grouped tree).
pub fn parse_lossless(src: &str) -> SyntaxNode {
    SyntaxNode::new_root(build_line_tree(src))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_byte_for_byte() {
        let src = "\u{feff}c title\n1 0 -1 imp:n=1 $ comment\n\n2 PX 3.0\n";
        assert_eq!(parse_lossless(src).to_string(), src);
    }

    #[test]
    fn roundtrips_without_trailing_newline() {
        let src = "1 SO 5";
        assert_eq!(parse_lossless(src).to_string(), src);
    }
}
