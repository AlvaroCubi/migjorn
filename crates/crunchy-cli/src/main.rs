//! Development CLI for crunchy.
//!
//! `crunchy bench <file>` — measure read / lex / CST-build / round-trip timings.
//! `crunchy lex <file>`   — print a token-kind histogram.

use std::time::{Duration, Instant};

use crunchy_syntax::{build_flat_tree, build_line_tree, lex, SyntaxKind, SyntaxNode};

fn main() {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    let path = args.next();

    match (cmd.as_str(), path) {
        ("bench", Some(p)) => bench(&p),
        ("lex", Some(p)) => lex_hist(&p),
        ("parse", Some(p)) => parse_cmd(&p),
        ("surfaces", Some(p)) => surfaces_cmd(&p),
        _ => {
            eprintln!("usage:");
            eprintln!("  crunchy parse    <file.mcnp>  parse into cards, report structure + diagnostics");
            eprintln!("  crunchy surfaces <file.mcnp>  parse surfaces, report mnemonic histogram");
            eprintln!("  crunchy bench <file.mcnp>   measure lex + CST build timings");
            eprintln!("  crunchy lex   <file.mcnp>   print token-kind histogram");
            std::process::exit(2);
        }
    }
}

fn mbps(bytes: usize, dt: Duration) -> f64 {
    (bytes as f64 / (1024.0 * 1024.0)) / dt.as_secs_f64()
}

fn read(path: &str) -> String {
    let t = Instant::now();
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error: cannot read {path}: {e}");
        std::process::exit(1);
    });
    eprintln!(
        "read:      {:>10.3?}  {:>8.1} MB/s  ({} bytes)",
        t.elapsed(),
        mbps(src.len(), t.elapsed()),
        src.len()
    );
    src
}

fn bench(path: &str) {
    let src = read(path);

    // Pass 1: pure lex throughput (no allocation beyond counters).
    let t = Instant::now();
    let mut ntok = 0u64;
    lex(src.as_bytes(), |_, _, _| ntok += 1);
    let lex_dt = t.elapsed();
    eprintln!(
        "lex:       {:>10.3?}  {:>8.1} MB/s  ({ntok} tokens)",
        lex_dt,
        mbps(src.len(), lex_dt)
    );

    // Pass 2: build the lossless rowan CST.
    let t = Instant::now();
    let green = build_line_tree(&src);
    let build_dt = t.elapsed();
    eprintln!(
        "build CST: {:>10.3?}  {:>8.1} MB/s",
        build_dt,
        mbps(src.len(), build_dt)
    );

    // Round-trip: re-emit and verify byte-for-byte losslessness.
    let node = SyntaxNode::new_root(green);
    let t = Instant::now();
    let out = node.to_string();
    let rt_dt = t.elapsed();
    let ok = out == src;
    eprintln!(
        "roundtrip: {:>10.3?}  {:>8.1} MB/s  lossless={ok}",
        rt_dt,
        mbps(src.len(), rt_dt)
    );

    let total = lex_dt + build_dt;
    eprintln!("---");
    eprintln!(
        "lex+build: {:>10.3?}  {:>8.1} MB/s  (rowan)",
        total,
        mbps(src.len(), total)
    );
    if !ok {
        eprintln!("ERROR: round-trip mismatch — CST is not lossless");
        std::process::exit(1);
    }

    // Custom flat-arena tree (M0 fallback candidate).
    eprintln!("===");
    let t = Instant::now();
    let flat = build_flat_tree(&src);
    let flat_build = t.elapsed();
    eprintln!(
        "flat build:{:>10.3?}  {:>8.1} MB/s  ({} tokens, {} lines)",
        flat_build,
        mbps(src.len(), flat_build),
        flat.token_count(),
        flat.line_count()
    );
    let t = Instant::now();
    let flat_out = flat.to_string(&src);
    let flat_rt = t.elapsed();
    let flat_ok = flat_out == src;
    eprintln!(
        "flat rtrip:{:>10.3?}  {:>8.1} MB/s  lossless={flat_ok}",
        flat_rt,
        mbps(src.len(), flat_rt)
    );
    let flat_total = lex_dt + flat_build;
    eprintln!(
        "lex+flat:  {:>10.3?}  {:>8.1} MB/s  (custom)",
        flat_total,
        mbps(src.len(), flat_total)
    );
    if !flat_ok {
        eprintln!("ERROR: flat round-trip mismatch");
        std::process::exit(1);
    }
}

fn parse_cmd(path: &str) {
    let src = read(path);
    let t = Instant::now();
    let parsed = crunchy_syntax::parse(src.clone());
    let dt = t.elapsed();

    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for c in parsed.tree.cards() {
        *counts.entry(format!("{:?}", c.kind)).or_default() += 1;
    }

    eprintln!(
        "parse:     {:>10.3?}  {:>8.1} MB/s  ({} tokens, {} cards)",
        dt,
        mbps(src.len(), dt),
        parsed.tree.token_count(),
        parsed.tree.cards().len(),
    );
    for (kind, n) in &counts {
        eprintln!("  {kind:<14} {n}");
    }

    // Losslessness on real input.
    let ok = parsed.tree.to_source() == src;
    eprintln!("roundtrip: lossless={ok}");

    if parsed.diagnostics.is_empty() {
        eprintln!("diagnostics: none");
    } else {
        eprintln!("diagnostics: {}", parsed.diagnostics.len());
        for d in parsed.diagnostics.iter().take(20) {
            eprintln!("  {:?} @ {}..{}: {}", d.severity, d.span.start, d.span.end, d.message);
        }
    }
    if !ok {
        std::process::exit(1);
    }
}

fn surfaces_cmd(path: &str) {
    let src = read(path);
    let t = Instant::now();
    let parsed = crunchy_syntax::parse(src);
    let parse_dt = t.elapsed();

    let t = Instant::now();
    let mut hist: std::collections::BTreeMap<String, u64> = Default::default();
    let mut count = 0u64;
    let mut malformed = 0u64;
    let mut max_id = i64::MIN;
    let debug = std::env::var_os("CRUNCHY_DEBUG").is_some();
    for s in crunchy_core::surfaces(&parsed.tree) {
        count += 1;
        if !s.well_formed {
            malformed += 1;
            if debug && malformed <= 5 {
                let card = &parsed.tree.cards()[s.card_index];
                let text: String = (card.first_tok..card.tok_end)
                    .map(|i| parsed.tree.token_text(i))
                    .collect();
                eprintln!("[debug] malformed surface {}: {:?}", s.id, text.trim_end());
            }
        }
        max_id = max_id.max(s.id);
        *hist.entry(format!("{:?}", s.kind)).or_default() += 1;
    }
    let typed_dt = t.elapsed();

    eprintln!("parse:     {:>10.3?}", parse_dt);
    eprintln!("surfaces:  {:>10.3?}  ({count} surfaces, max id {max_id})", typed_dt);
    eprintln!("malformed (need shortcut expansion): {malformed}");
    eprintln!("mnemonic histogram:");
    let mut entries: Vec<_> = hist.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, n) in entries {
        eprintln!("  {k:<14} {n}");
    }
}

fn lex_hist(path: &str) {
    let src = read(path);
    let mut counts = [0u64; 32];
    lex(src.as_bytes(), |k, _, _| counts[k as usize] += 1);
    for (i, &n) in counts.iter().enumerate() {
        if n == 0 {
            continue;
        }
        println!("{:?}: {n}", SyntaxKind::from_u16(i as u16));
    }
}
