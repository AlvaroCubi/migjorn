//! Corpus-driven regression tests.
//!
//! Every `*.mcnp` file under `tests/corpus/` is:
//!   1. asserted to round-trip **byte-for-byte** (the lossless invariant), and
//!   2. snapshotted as a compact card structure + diagnostics summary.
//!
//! ## Adding a bug repro
//!
//! Drop the offending deck (or a one-card snippet) into `tests/corpus/`
//! (feel free to use sub-folders like `cells/`, `surfaces/`, `continuations/`),
//! then run `cargo insta review` (or `cargo test`) to accept the new snapshot.
//! No test code required.

use crunchy_syntax::{parse, SyntaxKind};

/// A readable, stable summary of a parse: one line per card (kind + its
/// meaningful tokens) followed by the diagnostics.
fn summary(src: &str) -> String {
    let parsed = parse(src);
    let tree = &parsed.tree;

    let mut out = String::new();
    for card in tree.cards() {
        let kind = format!("{:?}", card.kind);
        let mut toks = String::new();
        for i in tree.card_content_tokens(card) {
            if !toks.is_empty() {
                toks.push(' ');
            }
            toks.push_str(&tree.token_text(i));
        }
        // Keep snapshots compact for pathological cards.
        if toks.len() > 200 {
            toks.truncate(197);
            toks.push_str("...");
        }
        out.push_str(&format!("{kind:<13}| {toks}\n"));
    }

    out.push_str("--\n");
    if parsed.diagnostics.is_empty() {
        out.push_str("diagnostics: none\n");
    } else {
        for d in &parsed.diagnostics {
            out.push_str(&format!(
                "{:?} @ {}..{}: {}\n",
                d.severity, d.span.start, d.span.end, d.message
            ));
        }
    }
    out
}

#[test]
fn corpus() {
    insta::glob!("corpus/**/*.mcnp", |path| {
        let src = std::fs::read_to_string(path).expect("read corpus file");

        // 1. Losslessness — the non-negotiable invariant.
        let parsed = parse(src.clone());
        assert_eq!(
            parsed.tree.to_source(),
            src,
            "round-trip mismatch for {}",
            path.display()
        );

        // 2. Never panics; token stream tiles the source.
        let mut expected = 0u32;
        for i in 0..parsed.tree.token_count() as u32 {
            assert_eq!(parsed.tree.token_span(i).start, expected);
            expected = parsed.tree.token_span(i).end;
        }
        assert_eq!(expected as usize, src.len());

        // Sanity: every card kind is a card kind.
        for c in parsed.tree.cards() {
            assert!(c.kind.is_card() || c.kind == SyntaxKind::TITLE_CARD);
        }

        // 3. Structural snapshot.
        insta::assert_snapshot!(summary(&src));
    });
}
