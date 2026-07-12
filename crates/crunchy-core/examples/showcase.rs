//! A guided tour of crunchy's capabilities.
//!
//! Run with:
//!     cargo run -p crunchy-core --example showcase
//!
//! It parses a small self-contained MCNP model, reads the typed data, proves the
//! parse is byte-for-byte lossless, and renumbers the whole geometry.

use crunchy_core::Model;

/// A small but representative model: comments, an inline `$` comment, a `&`
/// continuation, unions (`:`), a cell complement (`#4`), a region complement,
/// a `LIKE n BUT` cell, an `RPP` macrobody, and a transformed surface.
const MODEL: &str = "\
Crunchy demo: a small pin-cell-ish model
c --- cells ---
1 1 -10.5 -1 imp:n=1                  $ fuel pin
2 2 -1.0  1 -2 imp:n=1                $ water gap
3 0 (2 -3) #4 imp:n=1                 $ moderator minus the insert
4 3 -2.7 -40 imp:n=1 imp:p=1 &
     vol=8.0                          $ aluminium insert (macrobody)
5 0 3 : 20 imp:n=0                    $ graveyard
10 like 1 but mat=4 rho=-11.3         $ second pin, different fuel

c --- surfaces ---
1 CZ 0.5
2 CZ 0.6
3 CZ 5.0
20 SO 100
30 1 PZ 50                            $ surface positioned by transform 1
40 RPP -1 1 -1 1 -1 2

c --- data ---
m1 92235.80c 0.04 92238.80c 0.96      $ enriched uranium
m2 1001.31c 2 8016.31c 1              $ light water
mt2 lwtr.10t
m3 13027.80c 1                        $ aluminium
m4 92235.80c 0.20 92238.80c 0.80
tr1 0 0 5
mode n
sdef pos=0 0 0 erg=2.0
f4:n 1 2 4
";

fn rule(title: &str) {
    println!("\n=== {title} ===");
}

/// Reconstruct a cell's on-disk text from the CST (to show edits visually).
fn cell_source(model: &Model, id: i64) -> Option<String> {
    let idx = model.index();
    let &ci = idx.cells.get(&id)?;
    let tree = model.tree();
    let card = tree.cards()[ci];
    let text: String = (card.first_tok..card.tok_end)
        .map(|i| tree.token_text(i))
        .collect();
    Some(text.trim_end().to_string())
}

fn main() {
    let mut model = Model::parse(MODEL);

    rule("parse");
    println!("diagnostics: {}", model.diagnostics().len());
    let idx = model.index();
    println!(
        "cells={}  surfaces={}  materials={}  transforms={}",
        idx.cells.len(),
        idx.surfaces.len(),
        idx.materials.len(),
        idx.transforms.len(),
    );

    rule("surfaces");
    for s in model.surfaces() {
        let tr = s.transform.map(|t| format!(" (TR{t})")).unwrap_or_default();
        println!(
            "  {:>3}  {:<4} {:?}{}",
            s.id,
            s.kind.mnemonic(),
            s.coeffs,
            tr
        );
    }

    rule("cells");
    for c in model.cells() {
        if let Some(base) = c.like {
            println!("  {:>3}  LIKE {} BUT ...", c.id, base.id);
            continue;
        }
        let kind = if c.material == Some(0) {
            "void".to_string()
        } else {
            format!("mat {} @ {}", c.material.unwrap(), c.density.unwrap())
        };
        let refs: Vec<i64> = c
            .surface_refs()
            .iter()
            .map(|r| if r.negative { -r.id } else { r.id })
            .collect();
        let complements: Vec<i64> = c.cell_refs().iter().map(|r| r.id).collect();
        print!("  {:>3}  {:<14} surfaces {:?}", c.id, kind, refs);
        if !complements.is_empty() {
            print!("  complements #{complements:?}");
        }
        println!();
    }

    rule("materials");
    for m in model.materials() {
        let comp: Vec<String> = m
            .entries
            .iter()
            .map(|e| format!("{}={}", e.zaid, e.fraction))
            .collect();
        println!("  m{:<2} {}", m.id, comp.join("  "));
    }

    rule("transforms");
    for t in model.transforms() {
        println!("  tr{}  displacement {:?}", t.id, t.displacement);
    }

    rule("lossless round-trip");
    let roundtrips = model.to_source() == MODEL;
    println!("parse -> to_source() reproduces the input byte-for-byte: {roundtrips}");
    assert!(roundtrips);

    rule("whole-geometry renumbering");
    println!("cell 3 before:  {}", cell_source(&model, 3).unwrap());
    // Offset every surface by +1000 (definitions AND references), and shift
    // cells into the 900-series via a dict for the ones we care about.
    model.renumber_surfaces(|id| id + 1000);
    model.renumber_cells(|id| id + 900);
    println!("cell 3 after:   {}", cell_source(&model, 903).unwrap());
    println!("  (surfaces 2,3 -> 1002,1003; complement #4 -> #904; cell id 3 -> 903)");

    rule("edit is lossless everywhere else");
    let edited = model.to_source();
    println!(
        "comments preserved:      {}",
        edited.contains("$ moderator minus the insert")
    );
    println!("continuation preserved:  {}", edited.contains("imp:p=1 &"));
    println!(
        "surface def renumbered:  {}",
        edited.contains("1040 RPP -1 1 -1 1 -1 2")
    );
    println!(
        "LIKE base renumbered:    {}",
        edited.contains("910 like 901 but")
    );

    println!("\nDone. Full edited model:\n");
    println!("{edited}");
}
