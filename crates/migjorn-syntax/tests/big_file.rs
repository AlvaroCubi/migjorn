//! Round-trip and parse/emit timings on the real reference model.
//!
//! Ignored by default because the input is private and untracked. Run with:
//!
//! ```text
//! cargo test -p migjorn-syntax --release --test big_file -- --ignored --nocapture
//! ```

use migjorn_syntax::{CardKind, Cst};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn big_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../untracked/big.mcnp")
}

#[test]
#[ignore = "requires the untracked 377 MB reference model"]
fn reference_model_round_trips_and_meets_the_parse_budget() {
    let path = big_path();
    if !path.exists() {
        eprintln!("skipping: {} not present", path.display());
        return;
    }

    let read = Instant::now();
    let text = std::fs::read_to_string(&path).expect("read big.mcnp");
    println!(
        "read      {:>8.3} s  ({:.1} MB)",
        read.elapsed().as_secs_f64(),
        text.len() as f64 / 1e6
    );

    let t = Instant::now();
    let cst = Cst::parse(&text);
    let parse = t.elapsed().as_secs_f64();

    let t = Instant::now();
    let out = cst.to_source();
    let emit = t.elapsed().as_secs_f64();

    let mut counts = [0usize; 8];
    for card in cst.cards() {
        counts[card.kind() as usize] += 1;
    }
    let tokens: usize = cst.cards().map(|c| c.tokens().len()).sum();

    println!("cards     {:>8}", cst.len());
    println!("tokens    {:>8}", tokens);
    println!(
        "  title {} / cell {} / surface {} / data {} / comment {} / blank {} / message {} / trailing {}",
        counts[CardKind::Title as usize],
        counts[CardKind::Cell as usize],
        counts[CardKind::Surface as usize],
        counts[CardKind::Data as usize],
        counts[CardKind::Comment as usize],
        counts[CardKind::Blank as usize],
        counts[CardKind::Message as usize],
        counts[CardKind::Trailing as usize],
    );
    println!("parse     {parse:>8.3} s  (budget 1.000 s)");
    println!("emit      {emit:>8.3} s  (budget 0.200 s)");

    assert_eq!(out.len(), text.len(), "emitted length differs");
    assert!(
        out == text,
        "reference model does not round-trip byte for byte"
    );

    // The title of this model is the single byte `C` — a title that looks like a
    // comment. Position must win over content.
    let first = cst.at(0).expect("first card");
    assert_eq!(first.kind(), CardKind::Title);
    assert!(
        first.text().starts_with('\u{feff}'),
        "BOM preserved on card 0"
    );

    assert!(parse <= 1.0, "parse over budget: {parse:.3} s");
    assert!(emit <= 0.2, "emit over budget: {emit:.3} s");
}
