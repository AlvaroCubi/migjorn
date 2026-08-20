//! Typed-layer timings and sanity on the real reference model.
//!
//! ```text
//! cargo test -p migjorn --release --test big_file -- --ignored --nocapture
//! ```

use migjorn::Model;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn big_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../untracked/big.mcnp")
}

#[test]
#[ignore = "requires the untracked 377 MB reference model"]
fn typed_layer_on_the_reference_model() {
    let path = big_path();
    if !path.exists() {
        eprintln!("skipping: {} not present", path.display());
        return;
    }
    let text = std::fs::read_to_string(&path).expect("read big.mcnp");

    let t = Instant::now();
    let model = Model::parse(&text);
    let parse = t.elapsed().as_secs_f64();

    let t = Instant::now();
    let out = model.to_source();
    let emit = t.elapsed().as_secs_f64();
    assert!(out == text, "reference model does not round-trip");

    println!("parse+index {parse:>8.3} s  (budget 1.000 s)");
    println!("emit        {emit:>8.3} s  (budget 0.200 s)");
    println!(
        "cells {} / surfaces {} / materials {} / transforms {}",
        model.num_cells(),
        model.num_surfaces(),
        model.num_materials(),
        model.num_transforms()
    );
    println!("diagnostics {}", model.diagnostics().len());
    for d in model.diagnostics().iter().take(5) {
        println!("  {:?} {} @ {:?}", d.severity, d.message, d.span);
    }

    // Collect ids up front so the lookup loop measures lookup, not iteration.
    let ids: Vec<i64> = model.cells().filter_map(|c| c.id()).take(100_000).collect();
    assert!(!ids.is_empty());

    let t = Instant::now();
    let mut found = 0usize;
    for &id in &ids {
        if model.cell(id).is_some() {
            found += 1;
        }
    }
    let per_lookup = t.elapsed().as_secs_f64() / ids.len() as f64;
    assert_eq!(found, ids.len(), "every collected id must resolve");
    println!("id lookup   {:>8.1} ns", per_lookup * 1e9);

    // A projected read of a single cell must be immediate — no whole-file work
    // can be hiding behind it.
    let probe = ids[ids.len() / 2];
    let t = Instant::now();
    for _ in 0..1000 {
        let cell = model.cell(probe).expect("probe cell");
        std::hint::black_box((cell.material(), cell.signed_surfaces().len()));
    }
    let per_read = t.elapsed().as_secs_f64() / 1000.0;
    println!("typed read  {:>8.1} us  (budget 1000 us)", per_read * 1e6);

    assert!(parse <= 1.0, "parse+index over budget: {parse:.3} s");
    assert!(emit <= 0.2, "emit over budget: {emit:.3} s");
    assert!(per_read <= 1e-3, "read over budget: {per_read} s");
}

#[test]
#[ignore = "requires the untracked 377 MB reference model"]
fn every_cell_and_surface_projects_without_panicking() {
    let path = big_path();
    if !path.exists() {
        return;
    }
    let text = std::fs::read_to_string(&path).expect("read big.mcnp");
    let model = Model::parse(&text);

    let mut malformed_cells = 0usize;
    let mut with_material = 0usize;
    for cell in model.cells() {
        if !cell.well_formed() {
            malformed_cells += 1;
        }
        if cell.material().is_some_and(|m| m != 0) {
            with_material += 1;
            assert!(
                cell.density().is_some() || !cell.well_formed(),
                "cell {:?} has a material but no density",
                cell.id()
            );
        }
        std::hint::black_box(cell.signed_surfaces());
    }

    let mut malformed_surfaces = 0usize;
    for surface in model.surfaces() {
        if !surface.well_formed() {
            malformed_surfaces += 1;
        }
        std::hint::black_box(surface.coeffs());
    }

    println!("cells with a material: {with_material}");
    println!("not well formed: {malformed_cells} cells / {malformed_surfaces} surfaces");

    // These counts used to be printed but never checked, so a parser
    // regression that quietly turned real fields into `None` (e.g. a numeric
    // format the field scanner doesn't understand) would pass this test
    // silently. The reference model is well-formed cover to cover; any
    // increase here is a real regression, not expected drift.
    assert_eq!(
        malformed_cells, 0,
        "reference model: {malformed_cells} cell(s) failed to parse a field"
    );
    assert_eq!(
        malformed_surfaces, 0,
        "reference model: {malformed_surfaces} surface(s) failed to parse a field"
    );
}
