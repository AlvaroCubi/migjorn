//! Ad hoc verification for docs/05-parallelism-overhead.md.
//!
//! Simulates gitronics' shape: many filler-sized files parsed in parallel
//! (one rayon task per file, mirroring `load_fillers`), each immediately
//! calling `clear_data_cards()` (the 172x-sequential-reparse cost point 5
//! targets) and `merge()`'d together, then `validate()`d once.
//!
//! Not part of the crate's public surface or test suite — run directly:
//!   cargo run --release -p migjorn --example verify_multimodel -- <n_files> <cells_per_file>
//! and compare wall/sys time (via `/usr/bin/time -v`) across RAYON_NUM_THREADS.

use std::time::Instant;

use migjorn::Model;
use rayon::prelude::*;

fn gen_filler(n_cells: usize, offset: i64) -> String {
    let n_surfaces = (n_cells / 4).max(3);
    let mut out = String::new();
    out.push_str("Generated filler\n");
    for i in 1..=n_cells as i64 {
        let id = offset + i;
        let s = offset + (i % n_surfaces as i64) + 1;
        let mat = i % 5;
        if mat == 0 {
            out.push_str(&format!("{id} 0 -{s} imp:n=1 u={}\n", i % 100));
        } else {
            let mid = offset + mat; // matches the offset `m{mid}` cards below
            let rho = -1.0 - (i % 7) as f64 * 0.1;
            out.push_str(&format!(
                "{id} {mid} {rho:.3} -{s} imp:n=1 imp:p=0 vol={}.0\n",
                i % 1000
            ));
        }
    }
    out.push('\n');
    let kinds = ["SO", "PX", "PY", "PZ", "CX", "CY", "CZ"];
    for s in 1..=(n_surfaces as i64 + 1) {
        let sid = offset + s;
        let kind = kinds[(s as usize) % kinds.len()];
        out.push_str(&format!("{sid} {kind} {}.0\n", (s % 50) + 1));
    }
    out.push('\n');
    for m in 1..=4i64 {
        let mid = offset + m;
        out.push_str(&format!("m{mid} 1001.31c {mid} 8016.31c 1\n"));
    }
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let n_files: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(172);
    let cells_per_file: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(2000);

    println!(
        "n_files={n_files} cells_per_file={cells_per_file} threads={}",
        rayon::current_num_threads()
    );

    // Distinct id ranges per file so `merge` never sees a collision.
    let block = (cells_per_file as i64) * 4 + 100;
    let srcs: Vec<String> = (0..n_files)
        .map(|i| gen_filler(cells_per_file, i as i64 * block))
        .collect();

    let t0 = Instant::now();
    let cleared: Vec<Model> = srcs
        .par_iter()
        .map(|src| Model::parse(src).clear_data_cards())
        .collect();
    let t_parse = t0.elapsed();

    let t1 = Instant::now();
    let mut base = cleared[0].clone();
    let rest = cleared[1..].to_vec();
    base.merge(rest).expect("no id collisions by construction");
    let t_merge = t1.elapsed();

    let t2 = Instant::now();
    let problems = base.validate();
    let t_validate = t2.elapsed();

    // `problems` is expected to be nonzero: `clear_data_cards` drops each
    // filler's own materials by design (real gitronics folds materials back
    // in from elsewhere), and this script never does that, so most cells end
    // up pointing at a material nothing here defines. Only the timings below
    // matter for this script's purpose.
    println!(
        "parse+clear: {:?}  merge: {:?}  validate: {:?}  (cells={}, problems={})",
        t_parse,
        t_merge,
        t_validate,
        base.num_cells(),
        problems.len()
    );
}
