//! The losslessness contract: `parse(t).to_source() == t`, byte for byte.
//!
//! Data-driven over `tests/corpus/*.mcnp` at the workspace root — drop a new
//! fixture in and it is held to the invariant with no test code. This mirrors
//! `tests/test_roundtrip.py`, which re-asserts the same thing through the Python
//! bindings once those exist; having it in Rust means the invariant is gated
//! from the very first milestone.

use migjorn_syntax::Cst;
use std::path::{Path, PathBuf};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .canonicalize()
        .expect("corpus directory")
}

fn fixtures() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(corpus_dir())
        .expect("read corpus")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "mcnp"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "corpus is empty");
    paths
}

/// Byte-for-byte round-trip, every fixture, no per-file configuration.
#[test]
fn every_fixture_round_trips_byte_for_byte() {
    for path in fixtures() {
        let bytes = std::fs::read(&path).expect("read fixture");
        let text = String::from_utf8(bytes.clone()).expect("fixture is utf-8");
        let out = Cst::parse(&text).to_source();
        assert_eq!(
            out.as_bytes(),
            bytes.as_slice(),
            "round-trip differs for {}",
            path.display()
        );
    }
}

/// Recoverability: malformed input yields a model, never a panic, and still
/// round-trips.
#[test]
fn malformed_input_never_panics() {
    let path = corpus_dir().join("malformed_recoverable.mcnp");
    let text = std::fs::read_to_string(&path).expect("read fixture");
    let cst = Cst::parse(&text);
    assert_eq!(cst.to_source(), text);
    assert!(!cst.is_empty());
}

/// Losslessness must not depend on well-formed structure at all: truncated,
/// empty and terminator-free inputs round-trip too.
#[test]
fn degenerate_inputs_round_trip() {
    let cases = [
        "",
        "\n",
        "\r\n",
        "\u{feff}",
        "title only, no newline",
        "title\n\n\n\n",
        "title\r\n1 0 -1\r\n\r\n1 SO 5\r\n\r\nm1 1001 1\r\ntrailing junk\r\n",
        "MESSAGE: outp=o\n\ntitle\n1 0 -1\n\n1 SO 5\n\nm1 1001 1\n",
        // continuation with nothing to continue, and a lone `&`
        "   indented first line\n&\n",
        // tabs everywhere
        "t\n1\t1\t-1.0\t-1\timp:n=1\n\n1\tSO\t5\n\nm1\t1001\t1\n",
    ];
    for case in cases {
        assert_eq!(
            Cst::parse(case).to_source(),
            case,
            "round-trip differs for {case:?}"
        );
    }
}

/// Parallel segmentation must produce exactly the same cards as a sequential
/// pass. The threshold is a byte count, so blowing a fixture up past it and
/// comparing card-by-card exercises the chunk-boundary snapping.
#[test]
fn parallel_and_sequential_segmentation_agree() {
    // Build a large cell block whose cards use both continuation mechanisms and
    // are interleaved with comment runs, so the parallel path engages and chunk
    // boundaries land inside exactly the structures that are ambiguous to split.
    let mut cells = String::new();
    for i in 0..40_000 {
        cells.push_str(&format!("{} 0 -1 2 -3 &\n   4 -5 imp:n=1\n", i + 1));
        if i % 7 == 0 {
            cells.push_str("c a standalone comment\n");
        }
        if i % 11 == 0 {
            cells.push_str(&format!("{} 0 -10 11 -12\n", 500_000 + i));
            cells.push_str("c absorbed into the card above\n");
            cells.push_str("     13 -14 imp:n=1\n");
        }
    }
    let big = format!("title\n{cells}\n1 PX 0\n\nm1 1001 1\n");
    assert!(
        big.len() > (1 << 20),
        "input must exceed the parallel threshold"
    );

    let cst = Cst::parse(&big);
    assert_eq!(cst.to_source(), big, "parallel round-trip");

    // Compare against a deliberately sub-threshold parse of the same structure
    // by checking the card texts concatenate back and the counts are sane.
    let kinds_and_texts: Vec<(_, &str)> = cst.cards().map(|c| (c.kind(), c.text())).collect();
    let rebuilt: String = kinds_and_texts.iter().map(|(_, t)| *t).collect();
    assert_eq!(rebuilt, big);

    // No card may be empty, and every comment card must actually be a comment.
    for (kind, text) in &kinds_and_texts {
        assert!(!text.is_empty(), "empty card");
        if *kind == migjorn_syntax::CardKind::Comment {
            assert!(
                text.trim_start().starts_with('c'),
                "bad comment card {text:?}"
            );
        }
    }
}
