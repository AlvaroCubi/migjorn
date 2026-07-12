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
        ("cells", Some(p)) => cells_cmd(&p),
        ("renumber", Some(p)) => renumber_cmd(&p, args.next()),
        ("model", Some(p)) => model_cmd(&p),
        _ => {
            eprintln!("usage:");
            eprintln!(
                "  crunchy model    <file.mcnp>          parse via Model facade, print summary"
            );
            eprintln!("  crunchy parse    <file.mcnp>          parse into cards, report structure");
            eprintln!("  crunchy surfaces <file.mcnp>          parse surfaces, mnemonic histogram");
            eprintln!("  crunchy cells    <file.mcnp>          parse cells + geometry, ref counts");
            eprintln!(
                "  crunchy renumber <file.mcnp> [offset] offset all surfaces + cells, verify"
            );
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
            eprintln!(
                "  {:?} @ {}..{}: {}",
                d.severity, d.span.start, d.span.end, d.message
            );
        }
    }
    if !ok {
        std::process::exit(1);
    }
}

fn model_cmd(path: &str) {
    let src = read(path);
    let t = Instant::now();
    let model = crunchy_core::Model::parse(src.clone());
    let parse_dt = t.elapsed();

    let t = Instant::now();
    let idx = model.index();
    let index_dt = t.elapsed();

    let materials: Vec<_> = model.materials().collect();
    let total_entries: usize = materials.iter().map(|m| m.entries.len()).sum();

    eprintln!("parse:     {:>10.3?}", parse_dt);
    eprintln!("index:     {:>10.3?}", index_dt);
    eprintln!("cells:       {}", idx.cells.len());
    eprintln!("surfaces:    {}", idx.surfaces.len());
    eprintln!(
        "materials:   {}  ({total_entries} zaid/fraction entries)",
        idx.materials.len()
    );
    eprintln!("transforms:  {}", idx.transforms.len());
    eprintln!("data cards:  {}", model.data_cards().count());
    eprintln!("diagnostics: {}", model.diagnostics().len());
    eprintln!("roundtrip:   lossless={}", model.to_source() == src);
}

fn renumber_cmd(path: &str, offset_arg: Option<String>) {
    let offset: i64 = offset_arg.and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let src = read(path);

    let parsed = crunchy_syntax::parse(&src);
    let orig_tokens = parsed.tree.token_count();
    let orig_cards = parsed.tree.cards().len();
    let orig_surfaces = crunchy_core::surfaces(&parsed.tree).count();

    // Apply the whole-geometry renumber: every surface and cell shifted by
    // `offset` (definitions + all references).
    let mut tree = parsed.tree;
    let t = Instant::now();
    crunchy_core::renumber_surfaces(&mut tree, |id| id + offset);
    crunchy_core::renumber_cells(&mut tree, |id| id + offset);
    let renum_dt = t.elapsed();

    let t = Instant::now();
    let out = tree.to_source();
    let emit_dt = t.elapsed();

    eprintln!("offset:    +{offset}");
    eprintln!(
        "renumber:  {:>10.3?}  (surfaces + cells, defs + refs)",
        renum_dt
    );
    eprintln!("emit:      {:>10.3?}  ({} bytes)", emit_dt, out.len());

    // Validate: re-parse the output and check consistency.
    let re = crunchy_syntax::parse(&out);
    let re_tokens = re.tree.token_count();
    let re_cards = re.tree.cards().len();

    // Every surface reference in a cell must resolve to a surface definition.
    let surf_ids: std::collections::HashSet<i64> =
        crunchy_core::surfaces(&re.tree).map(|s| s.id).collect();
    let mut dangling = 0u64;
    let mut refs = 0u64;
    let mut min_new = i64::MAX;
    for c in crunchy_core::cells(&re.tree) {
        for r in c.surface_refs() {
            refs += 1;
            min_new = min_new.min(r.id);
            if !surf_ids.contains(&r.id) {
                dangling += 1;
            }
        }
    }

    eprintln!("--- validation (re-parsed output) ---");
    eprintln!(
        "tokens:    {orig_tokens} -> {re_tokens}  {}",
        ok(orig_tokens == re_tokens)
    );
    eprintln!(
        "cards:     {orig_cards} -> {re_cards}  {}",
        ok(orig_cards == re_cards)
    );
    eprintln!(
        "surfaces:  {orig_surfaces} -> {}  {}",
        surf_ids.len(),
        ok(orig_surfaces == surf_ids.len())
    );
    eprintln!(
        "min surface id after offset: {min_new}  (>= {offset}? {})",
        ok(min_new >= offset)
    );
    eprintln!("surface refs checked: {refs}");
    eprintln!("dangling refs: {dangling}  {}", ok(dangling == 0));

    let all_ok = orig_tokens == re_tokens
        && orig_cards == re_cards
        && orig_surfaces == surf_ids.len()
        && dangling == 0;
    eprintln!("RESULT: {}", if all_ok { "OK" } else { "FAILED" });
    if !all_ok {
        std::process::exit(1);
    }
}

fn ok(b: bool) -> &'static str {
    if b {
        "ok"
    } else {
        "MISMATCH"
    }
}

fn cells_cmd(path: &str) {
    let src = read(path);
    let t = Instant::now();
    let parsed = crunchy_syntax::parse(src);
    let parse_dt = t.elapsed();

    let t = Instant::now();
    let (mut count, mut void, mut like, mut malformed) = (0u64, 0u64, 0u64, 0u64);
    let (mut surf_refs, mut cell_refs) = (0u64, 0u64);
    let mut max_id = i64::MIN;
    let debug = std::env::var_os("CRUNCHY_DEBUG").is_some();
    for c in crunchy_core::cells(&parsed.tree) {
        count += 1;
        max_id = max_id.max(c.id);
        if c.material == Some(0) {
            void += 1;
        }
        if c.like.is_some() {
            like += 1;
        }
        if !c.well_formed {
            malformed += 1;
            if debug && malformed <= 5 {
                let card = &parsed.tree.cards()[c.card_index];
                let text: String = (card.first_tok..card.tok_end)
                    .map(|i| parsed.tree.token_text(i))
                    .collect();
                eprintln!("[debug] malformed cell {}: {:?}", c.id, text.trim_end());
            }
        }
        surf_refs += c.surface_refs().len() as u64;
        cell_refs += c.cell_refs().len() as u64;
    }
    let typed_dt = t.elapsed();

    eprintln!("parse:     {:>10.3?}", parse_dt);
    eprintln!(
        "cells:     {:>10.3?}  ({count} cells, max id {max_id})",
        typed_dt
    );
    eprintln!("  void:            {void}");
    eprintln!("  like-but:        {like}");
    eprintln!("  malformed:       {malformed}");
    eprintln!("  surface refs:    {surf_refs}");
    eprintln!("  cell refs (#n):  {cell_refs}");
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
    eprintln!(
        "surfaces:  {:>10.3?}  ({count} surfaces, max id {max_id})",
        typed_dt
    );
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
