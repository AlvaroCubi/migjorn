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
        _ => {
            eprintln!("usage:");
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

fn lex_hist(path: &str) {
    let src = read(path);
    let mut counts = [0u64; 32];
    lex(src.as_bytes(), |k, _, _| counts[k as usize] += 1);
    for i in 0..counts.len() {
        if counts[i] == 0 {
            continue;
        }
        // SAFETY: only indices we actually emitted are non-zero, and those are
        // valid SyntaxKind discriminants.
        let kind: SyntaxKind = unsafe { std::mem::transmute::<u16, SyntaxKind>(i as u16) };
        println!("{kind:?}: {}", counts[i]);
    }
}
