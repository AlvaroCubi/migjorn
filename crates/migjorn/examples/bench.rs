//! Performance-budget harness (definition-of-done item 4).
//!
//! Times every row of `docs/04-performance-budget.md` in isolation —
//! each whole-file measurement from a **fresh parse**, because a structural edit
//! mutates state and reusing an edited model skews the next timing. Prints a
//! `operation | target | measured | pass?` table and exits non-zero on any miss,
//! so CI breaks on a regression.
//!
//! Usage (release is mandatory — debug numbers are meaningless):
//!   cargo run --release -p migjorn --example bench -- path/to/big.mcnp
//!
//! Reproduce the ~1M-card input first:
//!   python benches/gen_input.py --cells 1000000 > big.mcnp
//!
//! Note the generated stand-in is ~59 MB (lighter cards than the real 380 MB
//! model), so it has card-count parity but not byte-size parity; parse/emit will
//! read faster than the byte-oriented targets. Report which input a run used —
//! the header line does.

use std::hint::black_box;
use std::time::{Duration, Instant};

use migjorn::Model;

struct Row {
    op: String,
    target: String,
    measured: String,
    pass: bool,
}

fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

/// A row measured and targeted in seconds.
fn row_s(op: &str, target_s: f64, measured_s: f64) -> Row {
    Row {
        op: op.to_owned(),
        target: format!("<= {target_s:.2} s"),
        measured: format!("{measured_s:.3} s"),
        pass: measured_s <= target_s,
    }
}

fn row_ms(op: &str, target_ms: f64, measured_ms: f64) -> Row {
    Row {
        op: op.to_owned(),
        target: format!("<= {target_ms:.0} ms"),
        measured: format!("{measured_ms:.3} ms"),
        pass: measured_ms <= target_ms,
    }
}

fn row_us(op: &str, target_us: f64, measured_us: f64) -> Row {
    Row {
        op: op.to_owned(),
        target: format!("<= {target_us:.1} us"),
        measured: format!("{measured_us:.3} us"),
        pass: measured_us <= target_us,
    }
}

fn median(mut xs: Vec<Duration>) -> Duration {
    xs.sort_unstable();
    xs[xs.len() / 2]
}

/// Apply `k` scattered in-card value edits across the cell block, addressing real
/// cell ids spread evenly through the model.
fn apply_scattered_edits(m: &mut Model, cell_ids: &[i64], k: usize) {
    let step = (cell_ids.len() / k).max(1);
    for (j, id) in cell_ids.iter().step_by(step).take(k).enumerate() {
        if let Some(c) = m.cell(*id) {
            let slot = c.slot();
            let _ = m.set_cell_material(slot, 2 + (j % 3) as i64);
        }
    }
}

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: bench <input.mcnp>  (build --release)");
            std::process::exit(2);
        }
    };
    let src = std::fs::read_to_string(&path).expect("read input file");
    let mb = src.len() as f64 / 1e6;

    let model = Model::parse(&src);
    let num_cells = model.num_cells();
    let num_surfaces = model.num_surfaces();
    println!(
        "input: {path}  ({mb:.1} MB, {num_cells} cells, {num_surfaces} surfaces, {} threads)",
        rayon::current_num_threads()
    );

    // Discover real ids so the bench works on any model, not just the generated
    // one (whose ids happen to be 1..N). The real `big.mcnp` has arbitrary ids.
    let cell_ids: Vec<i64> = model.cells().filter_map(|c| c.id()).collect();
    if cell_ids.is_empty() {
        eprintln!("input has no readable cell ids");
        std::process::exit(2);
    }
    let pivot_id = cell_ids[cell_ids.len() / 2]; // an id that exists, for remove/read
    let value_id = model
        .cells()
        .find(|c| c.material().is_some_and(|m| m != 0))
        .and_then(|c| c.id())
        .unwrap_or(pivot_id); // a real (non-void) cell, for the token-swap edit
    let add_base = cell_ids.iter().copied().max().unwrap() + 1; // unique new ids

    let mut rows = Vec::new();

    // 1. parse
    {
        let t = Instant::now();
        let m = Model::parse(&src);
        let d = t.elapsed();
        black_box(m.num_cells());
        rows.push(row_s("parse", 1.0, secs(d)));
    }

    // 2. emit, unedited
    {
        let t = Instant::now();
        let out = model.to_source();
        let d = t.elapsed();
        black_box(out.len());
        rows.push(row_s("emit, unedited", 0.20, secs(d)));
    }

    // 3. emit, after K scattered edits — must stay flat (edit-independent)
    for k in [1usize, 100, 10_000] {
        let mut m = Model::parse(&src);
        apply_scattered_edits(&mut m, &cell_ids, k);
        let t = Instant::now();
        let out = m.to_source();
        let d = t.elapsed();
        black_box(out.len());
        rows.push(row_s(&format!("emit, after {k} edits"), 0.20, secs(d)));
    }

    // 4. add_cell (single) — median of fresh-parse repeats
    {
        let text = format!("{add_base} 0 -1 imp:n=1");
        let d = median(
            (0..5)
                .map(|_| {
                    let mut m = Model::parse(&src);
                    let t = Instant::now();
                    m.add_cell(&text).unwrap();
                    let e = t.elapsed();
                    black_box(m.num_cells());
                    e
                })
                .collect(),
        );
        rows.push(row_ms("add_cell (single)", 10.0, secs(d) * 1e3));
    }

    // 5. remove_cell (single)
    {
        let d = median(
            (0..5)
                .map(|_| {
                    let mut m = Model::parse(&src);
                    let t = Instant::now();
                    black_box(m.remove_cell(pivot_id));
                    t.elapsed()
                })
                .collect(),
        );
        rows.push(row_ms("remove_cell (single)", 10.0, secs(d) * 1e3));
    }

    // 6. 1000-edit session — the headline workload
    {
        let mut m = Model::parse(&src);
        let t = Instant::now();
        for i in 0..1000i64 {
            let id = add_base + i;
            let _ = m.add_cell(&format!("{id} 0 -1 imp:n=1"));
            black_box(m.cell(id).map(|c| c.is_void()));
            if i % 2 == 0 {
                m.remove_cell(id);
            }
            if let Some(c) = m.cell(value_id) {
                let slot = c.slot();
                let _ = m.set_cell_material(slot, 2 + (i % 3));
            }
        }
        let d = t.elapsed();
        black_box(m.num_cells());
        rows.push(row_s("1000-edit session", 1.0, secs(d)));
    }

    // 7. read after edit
    {
        let mut m = Model::parse(&src);
        let slot = m.cell(value_id).unwrap().slot();
        m.set_cell_material(slot, 3).unwrap();
        let t = Instant::now();
        let v = m.cell(value_id).unwrap().material();
        let d = t.elapsed();
        black_box(v);
        rows.push(row_ms("read after edit", 1.0, secs(d) * 1e3));
    }

    // in-card value edit (per-op median over a tight loop)
    {
        let mut m = Model::parse(&src);
        let slot = m.cell(value_id).unwrap().slot();
        let iters = 100_000u32;
        let t = Instant::now();
        for i in 0..iters as i64 {
            let _ = m.set_cell_material(slot, 2 + (i % 3));
        }
        let per = t.elapsed() / iters;
        rows.push(row_us("in-card value edit", 5.0, secs(per) * 1e6));
    }

    // 8. renumber_cells + emit  (defs + #refs + LIKE, then the emit it feeds)
    {
        let mut m = Model::parse(&src);
        let t = Instant::now();
        m.renumber_cells(|i| i + 1);
        let out = m.to_source();
        let d = t.elapsed();
        black_box(out.len());
        rows.push(row_s("renumber_cells + emit", 0.5, secs(d)));
    }

    // renumber_surfaces + emit  (defs + every geometry sense)
    {
        let mut m = Model::parse(&src);
        let t = Instant::now();
        m.renumber_surfaces(|i| i + 1);
        let out = m.to_source();
        let d = t.elapsed();
        black_box(out.len());
        rows.push(row_s("renumber_surfaces + emit", 2.0, secs(d)));
    }

    // 9. id lookup — O(1); assert flat (per-lookup well under 1 us)
    {
        let sample = &cell_ids[..cell_ids.len().min(200_000)];
        let t = Instant::now();
        let mut acc = 0u64;
        for &id in sample {
            if let Some(c) = model.cell(id) {
                acc = acc.wrapping_add(c.slot() as u64);
            }
        }
        let per = t.elapsed() / sample.len() as u32;
        black_box(acc);
        rows.push(row_us("id lookup", 1.0, secs(per) * 1e6));
    }

    print_table(&rows);

    if rows.iter().any(|r| !r.pass) {
        eprintln!("\nFAIL: at least one budget row exceeded its target.");
        std::process::exit(1);
    }
    println!("\nOK: every budget row within target.");
}

fn print_table(rows: &[Row]) {
    let w_op = rows.iter().map(|r| r.op.len()).max().unwrap_or(0).max(9);
    let w_t = rows
        .iter()
        .map(|r| r.target.len())
        .max()
        .unwrap_or(0)
        .max(6);
    let w_m = rows
        .iter()
        .map(|r| r.measured.len())
        .max()
        .unwrap_or(0)
        .max(8);
    println!(
        "\n{:<w_op$}  {:>w_t$}  {:>w_m$}  pass?",
        "operation",
        "target",
        "measured",
        w_op = w_op,
        w_t = w_t,
        w_m = w_m
    );
    println!("{}", "-".repeat(w_op + w_t + w_m + 12));
    for r in rows {
        println!(
            "{:<w_op$}  {:>w_t$}  {:>w_m$}  {}",
            r.op,
            r.target,
            r.measured,
            if r.pass { "yes" } else { "NO" },
            w_op = w_op,
            w_t = w_t,
            w_m = w_m
        );
    }
}
