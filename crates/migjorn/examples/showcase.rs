//! A guided tour of migjorn's public API.
//!
//! Run with:
//!     cargo run -p migjorn --example showcase
//!
//! Every section below exercises a real part of the public surface, so this file
//! doubles as executable documentation: parse → read → edit → validate → emit,
//! losslessly throughout. The assertions are part of the point — they state what
//! the library guarantees, and the example fails loudly if one ever stops
//! holding.
//!
//! The API splits in two:
//!
//! * [`Model`] owns *editing* (renumbering, add/remove, in-place value edits)
//!   and re-emission.
//! * [`ModelView`], from [`Model::view`], owns *reading*. It materialises any
//!   pending edits once, so its readers are all `&self`: they compose, nest, and
//!   can never disagree with what the model emits.

use migjorn::{ConflictKind, EditError, Model, ModelView, Severity, SurfaceKind};

/// A small but representative model: line comments, an inline `$` comment, a `&`
/// continuation, a union (`:`), a cell complement (`#11`), a `LIKE n BUT` cell,
/// an `RPP` macrobody, a transform-positioned surface, a reflecting (`*`)
/// surface, a filled universe, and a tally.
const MODEL: &str = "\
Migjorn showcase: a pin cell inside a filled universe
c --- cells ---
1 1 -10.5 -1 imp:n=1 u=10             $ fuel pin (universe 10)
2 2 -1.0 1 -2 imp:n=1 u=10            $ water gap (universe 10)
3 0 2 imp:n=1 u=10                    $ rest of universe 10
10 0 -3 fill=10 imp:n=1               $ shell, filled by universe 10
11 3 -2.7 -40 imp:n=1 imp:p=1 &
     vol=8.0                          $ aluminium insert (macrobody)
12 0 (3 -20) #11 imp:n=1              $ moderator minus the insert
13 0 20 : 50 imp:n=0                  $ graveyard
14 like 11 but mat=4 rho=-11.3        $ second insert, different material

c --- surfaces ---
1 CZ 0.5
2 CZ 0.6
3 CZ 5.0
20 SO 100
30 1 PZ 50                            $ positioned by transform 1
40 RPP -1 1 -1 1 -1 2
*50 PX 60                             $ reflecting boundary

c --- data ---
m1 92235.80c 0.04 92238.80c 0.96      $ enriched uranium
m2 1001.31c 2 8016.31c 1              $ light water
mt2 lwtr.10t
m3 13027.80c 1                        $ aluminium
m4 92235.80c 0.20 92238.80c 0.80
tr1 0 0 5
mode n
sdef pos=0 0 0 erg=2.0
f4:n 1 2 11
";

fn rule(title: &str) {
    println!("\n=== {title} ===");
}

/// The on-disk text of the cell numbered `id` (inline `$` comment included).
fn cell_text(view: &ModelView, id: i64) -> String {
    let ci = view.index().cell(id).expect("cell exists");
    view.card_source(ci).trim_end().to_string()
}

fn main() {
    parsing();
    reading();
    geometry_expressions();
    stable_handles();
    value_editing();
    cell_parameters();
    geometry_editing();
    adding_and_removing_cards();
    renumbering();
    validation();
    universes_and_composition();
    error_handling();

    println!("\nAll sections passed.");
}

// ---------------------------------------------------------------------------

/// `Model::parse`, `Model::diagnostics`, `Model::to_source`.
fn parsing() {
    rule("parse");
    let model = Model::parse(MODEL);
    println!("diagnostics: {}", model.diagnostics().len());
    assert!(model.diagnostics().is_empty());

    // The defining invariant: re-emission is byte-for-byte identical.
    assert_eq!(model.to_source(), MODEL);
    println!("parse -> to_source() reproduces the input byte-for-byte: true");

    // The parser is *recoverable*: it never panics on malformed input. Problems
    // surface in two distinct places.
    //
    // 1. `diagnostics()` reports whole-file *structure* — the blank lines that
    //    delimit the cell / surface / data blocks.
    let no_delim = Model::parse("t\n1 0 -1\n\n1 SO 5\nm1 1001 1\n");
    println!(
        "missing a block delimiter -> {} diagnostic(s):",
        no_delim.diagnostics().len()
    );
    for d in no_delim.diagnostics() {
        // `Severity` is `#[non_exhaustive]`, so downstream matches need a
        // wildcard arm. `Span` carries byte offsets into the source.
        let sev = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            _ => "other",
        };
        println!(
            "  [{sev}] {} (bytes {}..{})",
            d.message, d.span.start, d.span.end
        );
    }
    assert_eq!(no_delim.diagnostics().len(), 1);

    // 2. A *card* that does not parse cleanly is not a diagnostic: it comes back
    //    as a typed view with `well_formed == false`, so you can spot it while
    //    reading rather than having to cross-reference an error list.
    let mut bad_card = Model::parse("t\n1 0 (-1\n\n1 CZ 5\n\nm1 1001 1\n");
    assert!(bad_card.diagnostics().is_empty(), "structure is fine here");
    let cell = bad_card.view().cells().next().unwrap();
    println!(
        "unbalanced geometry -> cell {} well_formed={}",
        cell.id, cell.well_formed
    );
    assert!(!cell.well_formed);

    // Either way the text is preserved exactly: losslessness does not depend on
    // understanding the input.
    assert_eq!(no_delim.to_source(), "t\n1 0 -1\n\n1 SO 5\nm1 1001 1\n");
    assert_eq!(bad_card.to_source(), "t\n1 0 (-1\n\n1 CZ 5\n\nm1 1001 1\n");
    println!("both still round-trip byte-for-byte: true");
}

/// `Model::view` and the whole `ModelView` reader surface, plus `ModelIndex`.
fn reading() {
    let mut model = Model::parse(MODEL);
    // One `&mut` at the boundary. Every read below borrows the view *shared*, so
    // they nest and compose freely.
    let view = model.view();

    rule("index & id lookups");
    let idx = view.index();
    println!(
        "cells={}  surfaces={}  materials={}  transforms={}",
        idx.cell_count(),
        idx.surface_count(),
        idx.material_count(),
        idx.transform_count(),
    );
    // `ModelIndex` maps an id to a card index; `*_ids` enumerate what is defined.
    assert!(idx.cell(10).is_some());
    assert!(idx.surface(40).is_some());
    assert!(idx.material(2).is_some());
    assert!(idx.transform(1).is_some());
    assert!(idx.cell(999).is_none());
    let mut sids: Vec<i64> = idx.surface_ids().collect();
    sids.sort_unstable();
    println!("surface ids: {sids:?}");
    let mut mids: Vec<i64> = idx.material_ids().collect();
    mids.sort_unstable();
    println!("material ids: {mids:?}");
    let mut tids: Vec<i64> = idx.transform_ids().collect();
    tids.sort_unstable();
    println!("transform ids: {tids:?}");
    let mut cids: Vec<i64> = idx.cell_ids().collect();
    cids.sort_unstable();
    println!("cell ids: {cids:?}");

    rule("surfaces");
    for s in view.surfaces() {
        let tr = s
            .transform
            .map(|t| format!("  (TR{t})"))
            .unwrap_or_default();
        let boundary = match (s.reflective, s.white) {
            (true, _) => "  [reflecting]",
            (_, true) => "  [white]",
            _ => "",
        };
        println!(
            "  {:>3}  {:<4} {:?}{}{}",
            s.id,
            s.kind.mnemonic(),
            s.coeffs,
            tr,
            boundary
        );
        assert!(s.well_formed);
    }
    // `SurfaceKind` round-trips through its mnemonic.
    assert_eq!(SurfaceKind::from_mnemonic("RPP").mnemonic(), "RPP");

    rule("cells");
    for c in view.cells() {
        // A `LIKE n BUT` cell has no geometry of its own.
        if let Some(base) = c.like_id() {
            println!("  {:>3}  LIKE {base} BUT ...", c.id);
            continue;
        }
        let kind = if c.is_void() {
            "void".to_string()
        } else {
            format!("mat {} @ {}", c.material.unwrap(), c.density.unwrap())
        };
        print!(
            "  {:>3}  {:<14} surfaces {:?}",
            c.id,
            kind,
            c.signed_surfaces()
        );
        // `#n` complements plus a `LIKE n` base.
        if !c.cell_ref_ids().is_empty() {
            print!("  complements #{:?}", c.cell_ref_ids());
        }
        // `surface_refs`/`cell_refs` give the same references with full detail.
        assert_eq!(
            c.surface_refs().len(),
            c.surface_ids().len(),
            "both spellings agree"
        );
        assert_eq!(c.cell_refs().len(), c.cell_ref_ids().len());
        println!();
        assert!(c.well_formed);
    }

    rule("cell parameters, universes & fills");
    for c in view.cells() {
        // Readers nest: iterate cells and read each one's params inside the loop.
        let params: Vec<String> = view
            .cell_params(c.card_index)
            .iter()
            .map(|p| {
                let star = if p.starred { "*" } else { "" };
                match &p.particle {
                    Some(part) => format!("{star}{}:{part}={}", p.key, p.value),
                    None => format!("{star}{}={}", p.key, p.value),
                }
            })
            .collect();
        let u = view
            .cell_universe(c.card_index)
            .map(|u| format!("  u={u}"))
            .unwrap_or_default();
        let fill = view
            .cell_fill(c.card_index)
            .map(|f| {
                let tr = f.transform.map(|t| format!(" ({t})")).unwrap_or_default();
                let star = if f.starred { "*" } else { "" };
                format!("  {star}fill={}{tr}", f.universe)
            })
            .unwrap_or_default();
        println!("  {:>3}  {}{u}{fill}", c.id, params.join(" "));
    }
    // A single parameter by key: bare, or particle-qualified.
    let c11 = view.index().cell(11).unwrap();
    assert_eq!(view.cell_param(c11, "vol").unwrap().value, "8.0");
    assert_eq!(view.cell_param(c11, "imp:n").unwrap().value, "1");
    assert_eq!(view.cell_param(c11, "imp:p").unwrap().value, "1");
    assert!(view.cell_param(c11, "tmp").is_none());
    println!("  cell 11: vol=8.0, imp:n=1, imp:p=1 (read by key)");
    // Every universe defined by a `u=`, sorted and deduplicated.
    assert_eq!(view.universe_ids(), vec![10]);
    println!("  universes in the model: {:?}", view.universe_ids());

    rule("materials & transforms");
    for m in view.materials() {
        let comp: Vec<String> = m
            .entries
            .iter()
            .map(|e| format!("{}={}", e.zaid, e.fraction))
            .collect();
        println!("  m{:<2} {}", m.id, comp.join("  "));
        // `MaterialEntry::za` strips the library suffix off a ZAID.
        assert!(m.entries[0].za().is_some());
        assert!(m.well_formed);
    }
    println!(
        "  m1's first ZA: {:?}",
        view.materials().next().unwrap().entries[0].za()
    );
    for t in view.transforms() {
        println!(
            "  tr{}  displacement {:?}  rotation {:?}  degrees={}",
            t.id, t.displacement, t.rotation, t.degrees
        );
        assert!(t.well_formed);
    }

    rule("data cards (generic)");
    for d in view.data_cards() {
        let star = if d.starred { "*" } else { "" };
        match &d.particle {
            Some(p) => println!("  {star}{}:{p}", d.name),
            None => println!("  {star}{}", d.name),
        }
    }

    rule("card text");
    // `card_source` is the card's exact text: inline comments and continuations
    // included. This is what makes text-based exploration work.
    let ci = view.index().cell(11).unwrap();
    println!("{}", view.card_source(ci).trim_end());
    assert!(view.card_source(ci).contains('&'), "continuation preserved");
    // `to_source` is available on the view too.
    assert_eq!(view.to_source(), MODEL);
}

/// `GeomExpr` — the parsed geometry tree behind `Cell::geometry`.
fn geometry_expressions() {
    rule("geometry expressions");
    let mut model = Model::parse(MODEL);
    let view = model.view();

    // `cell.geometry` is the parsed region: a tree of intersections, unions,
    // signed surfaces, and `#n` complements. The `Cell` helpers used above
    // (`surface_ids`, `cell_ref_ids`, …) are conveniences over walking it.
    let c12 = view.cells().find(|c| c.id == 12).unwrap();
    let geom = c12.geometry.as_ref().unwrap();
    println!("cell 12 geometry: {geom:?}");

    // Walk it yourself with the visitors, which recurse into nested regions.
    let mut surfaces = Vec::new();
    geom.for_each_surface_ref(&mut |s| surfaces.push((s.id, s.negative)));
    let mut cells = Vec::new();
    geom.for_each_cell_ref(&mut |c| cells.push(c.id));
    println!("  surface refs (id, negative): {surfaces:?}");
    println!("  cell refs:                   {cells:?}");
    assert_eq!(
        c12.surface_ids(),
        surfaces.iter().map(|(id, _)| *id).collect::<Vec<_>>()
    );

    // A union parses as one, and shows how the tree nests.
    let c13 = view.cells().find(|c| c.id == 13).unwrap();
    println!("cell 13 (a union): {:?}", c13.geometry.as_ref().unwrap());

    // The tree is editable in its own right. `is_empty()` is the `Error`
    // placeholder a region collapses to when everything is removed — which is
    // what [`EditError::WouldEmptyGeometry`] reports.
    let mut g = geom.clone();
    g.intersect_surface(7, true); // AND in  -7
    g.intersect_cell_complement(9); // AND in  #9
    let mut after = Vec::new();
    g.for_each_surface_ref(&mut |s| after.push(if s.negative { -s.id } else { s.id }));
    println!("  after intersecting -7 and #9: surfaces {after:?}");
    assert!(after.contains(&-7));
    assert!(!g.is_empty());

    // Editing a detached `GeomExpr` does *not* touch the model — it is a value.
    // Go through [`Model::add_cell_surface`] for an edit that actually lands
    // (losslessly, as a token splice).
    assert_eq!(model.to_source(), MODEL, "the clone's edits went nowhere");
    println!("  ...the model itself is unchanged (use Model::add_cell_surface to edit)");
}

/// Stable slots: handles that survive structural edits.
fn stable_handles() {
    rule("stable handles (slots)");
    let mut model = Model::parse(MODEL);

    // Cards carry a stable *slot* that survives reparses, so a caller can hold a
    // durable handle and resolve it back to a card index on demand. Card
    // *indices* shift when cards are added or removed; slots do not.
    let slots = model.cell_slots();
    println!("cell slots:      {slots:?}");
    println!("surface slots:   {:?}", model.surface_slots());
    println!("material slots:  {:?}", model.material_slots());
    println!("transform slots: {:?}", model.transform_slots());

    let view = model.view();
    let ci = view.index().cell(11).unwrap();
    let slot = view.slot_at(ci);
    assert_eq!(view.cell_by_slot(slot).unwrap().id, 11);
    assert_eq!(view.card_index_of_slot(slot), Some(ci));
    // The by-slot readers exist for every card type.
    let s_slot = model.surface_slots()[0];
    let m_slot = model.material_slots()[0];
    let t_slot = model.transform_slots()[0];
    let view = model.view();
    println!(
        "surface at slot {s_slot}:   {}",
        view.surface_by_slot(s_slot).unwrap().id
    );
    println!(
        "material at slot {m_slot}:  {}",
        view.material_by_slot(m_slot).unwrap().id
    );
    println!(
        "transform at slot {t_slot}: {}",
        view.transform_by_slot(t_slot).unwrap().id
    );

    // Remove an earlier card: indices shift, the slot still resolves.
    model.remove_cell(1).unwrap();
    let view = model.view();
    let new_ci = view
        .card_index_of_slot(slot)
        .expect("slot survives removal");
    assert_eq!(view.cell_by_slot(slot).unwrap().id, 11);
    println!(
        "after removing cell 1, cell 11 moved card index {ci} -> {new_ci}, slot {slot} unchanged"
    );

    // `Model::slot_at` / `card_index_of_slot` / `card_source` also work without
    // a view (they are splice-safe).
    assert_eq!(model.slot_at(new_ci), slot);
    assert!(model.card_source(new_ci).contains("vol=8.0"));
}

/// In-place *value* edits: numbers change, everything else stays byte-for-byte.
fn value_editing() {
    rule("value editing");
    let mut model = Model::parse(MODEL);
    let idx = model.view().index();
    let (c1, s3, m1, t1) = (
        idx.cell(1).unwrap(),
        idx.surface(3).unwrap(),
        idx.material(1).unwrap(),
        idx.transform(1).unwrap(),
    );

    println!("before: {}", cell_text(&model.view(), 1));
    model.set_cell_material(c1, 5).unwrap();
    model.set_cell_density(c1, -10.9).unwrap();
    println!("after:  {}", cell_text(&model.view(), 1));
    assert!(model.to_source().contains("1 5 -10.9 -1 imp:n=1 u=10"));

    // Assigning material 0 makes a cell void and drops its density; assigning a
    // real material back gives a placeholder density of 0 to fill in.
    model.set_cell_material(c1, 0).unwrap();
    assert!(model.view().cell_at(c1).unwrap().is_void());
    assert!(model.to_source().contains("1 0 -1 imp:n=1 u=10"));
    println!("void:   {}", cell_text(&model.view(), 1));
    model.set_cell_material(c1, 1).unwrap();
    model.set_cell_density(c1, -10.5).unwrap();

    // Surfaces: one coefficient, or the transform field.
    model.set_surface_coeff(s3, 0, 6.0).unwrap();
    assert_eq!(
        model.view().surfaces().find(|s| s.id == 3).unwrap().coeffs,
        [6.0]
    );
    model.set_surface_transform(s3, Some(1)).unwrap(); // attach TR1
    println!(
        "surface 3 with a transform:  {}",
        model.card_source(s3).trim_end()
    );
    model.set_surface_transform(s3, None).unwrap(); // and detach it again
    println!(
        "surface 3 without:           {}",
        model.card_source(s3).trim_end()
    );

    // Materials: a fraction or a whole ZAID, by entry index.
    model.set_material_fraction(m1, 0, 0.05).unwrap();
    model.set_material_zaid(m1, 1, "92238.70c").unwrap();
    println!("m1: {}", model.card_source(m1).trim_end());
    assert!(
        model.to_source().contains("$ enriched uranium"),
        "comment kept"
    );

    // Transforms: displacement, and a rotation matrix (spliced in when absent).
    model
        .set_transform_displacement(t1, [0.0, 0.0, 7.5])
        .unwrap();
    println!("tr1: {}", model.card_source(t1).trim_end());
    model
        .set_transform_rotation(t1, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])
        .unwrap();
    println!("tr1 with a rotation: {}", model.card_source(t1).trim_end());
    assert_eq!(model.view().transforms().next().unwrap().rotation.len(), 9);

    // Every other card is untouched.
    assert!(model.to_source().contains("$ moderator minus the insert"));
    assert!(Model::parse(model.to_source()).diagnostics().is_empty());
}

/// Reading and editing a cell's parameter tail (`imp:n`, `vol`, `u`, `fill`, …).
fn cell_parameters() {
    rule("cell parameter editing");
    let mut model = Model::parse(MODEL);
    let ci = model.view().index().cell(12).unwrap();
    println!("before:      {}", cell_text(&model.view(), 12));

    // Rewrite a value in place: the keyword and its position never move, so
    // unlike remove-then-add the card stays byte-for-byte apart from the value.
    assert!(model.set_cell_param(ci, "imp:n", "4").unwrap());
    // A key that is not present reports `false` rather than erroring.
    assert!(!model.set_cell_param(ci, "tmp", "1").unwrap());

    // Add a new parameter, spliced onto the end of the tail.
    model.add_cell_param(ci, "vol=12.5").unwrap();
    model.add_cell_param(ci, "u=20").unwrap();
    println!("after adds:  {}", cell_text(&model.view(), 12));

    let view = model.view();
    assert_eq!(view.cell_param(ci, "imp:n").unwrap().value, "4");
    assert_eq!(view.cell_param(ci, "vol").unwrap().value, "12.5");
    assert_eq!(view.cell_universe(ci), Some(20));
    println!(
        "params now:  {:?}",
        view.cell_params(ci)
            .iter()
            .map(|p| &p.key)
            .collect::<Vec<_>>()
    );

    // Remove parameters by key (bare or particle-qualified).
    assert!(model.remove_cell_param(ci, "vol").unwrap());
    assert!(model.remove_cell_param(ci, "u").unwrap());
    assert!(!model.remove_cell_param(ci, "vol").unwrap(), "already gone");
    println!("after removes: {}", cell_text(&model.view(), 12));
    assert!(model.view().cell_param(ci, "vol").is_none());
    // The inline comment rides through all of it.
    assert!(model.to_source().contains("$ moderator minus the insert"));
    assert!(Model::parse(model.to_source()).diagnostics().is_empty());
}

/// Structural geometry edits: add/remove surfaces and `#n` complements.
fn geometry_editing() {
    rule("structural geometry editing");
    let mut model = Model::parse(MODEL);
    let ci = model.view().index().cell(2).unwrap();
    println!("before: {}", cell_text(&model.view(), 2));
    println!(
        "  surfaces {:?}",
        model.view().cell_at(ci).unwrap().signed_surfaces()
    );

    // Intersect the region with new terms.
    model.add_cell_surface(ci, 3, true).unwrap(); // AND in  -3
    model.add_cell_complement(ci, 11).unwrap(); // AND in  #11
                                                // ...and take terms back out (either sense).
    assert!(model.remove_cell_surface(ci, 1).unwrap());
    // Removing something that is not there reports `false`.
    assert!(!model.remove_cell_complement(ci, 99).unwrap());

    let view = model.view();
    println!("after:  {}", cell_text(&view, 2));
    println!(
        "  surfaces {:?}  complements {:?}",
        view.cell_at(ci).unwrap().signed_surfaces(),
        view.cell_at(ci).unwrap().cell_ref_ids()
    );
    assert_eq!(view.cell_at(ci).unwrap().signed_surfaces(), vec![-2, -3]);
    assert_eq!(view.cell_at(ci).unwrap().cell_ref_ids(), vec![11]);
    // The cell's parameters and inline comment survive a geometry rewrite.
    assert_eq!(view.cell_param(ci, "imp:n").unwrap().value, "1");
    assert_eq!(view.cell_universe(ci), Some(10));
    assert!(view.card_source(ci).contains("$ water gap"));

    assert!(model.remove_cell_complement(ci, 11).unwrap());
    assert!(Model::parse(model.to_source()).diagnostics().is_empty());
}

/// Whole-card construction and removal, plus `append_inline_comment`.
fn adding_and_removing_cards() {
    rule("adding & removing cards");
    let mut model = Model::parse(MODEL);

    // Each `add_*` validates the text as exactly one well-formed card of that
    // kind and returns the new card's stable slot.
    let s_slot = model.add_surface("60 SO 200").unwrap();
    let d_slot = model.add_data_card("m9 6000.80c 1").unwrap();
    let c_slot = model.add_cell("21 9 -2.0 -60 imp:n=1").unwrap();
    println!("added surface 60, material m9, cell 21");

    let view = model.view();
    let ci = view.card_index_of_slot(c_slot).unwrap();
    println!("  new cell:     {}", view.card_source(ci).trim_end());
    println!(
        "  new surface:  {}",
        view.card_source(view.card_index_of_slot(s_slot).unwrap())
            .trim_end()
    );
    println!(
        "  new material: {}",
        view.card_source(view.card_index_of_slot(d_slot).unwrap())
            .trim_end()
    );
    assert_eq!(view.index().cell_count(), 9);
    assert!(view.validate().is_empty(), "the new cell resolves cleanly");

    // A new card is editable like any other.
    model.add_cell_surface(ci, 20, true).unwrap();
    model.append_inline_comment(ci, "built from Rust").unwrap();
    println!("  edited:       {}", cell_text(&model.view(), 21));
    assert!(model.to_source().contains("$ built from Rust"));

    // Removal is by number and reports whether anything matched. It deletes the
    // card only — references are left alone, and `validate` reports the fallout.
    assert!(model.remove_surface(60).unwrap());
    assert!(!model.remove_surface(60).unwrap(), "already gone");
    println!("after removing surface 60: {:?}", model.view().validate());
    assert!(!model.view().validate().is_empty());

    // `remove_cell` / `remove_material` / `remove_transform` round it out.
    assert!(model.remove_cell(21).unwrap());
    assert!(model.remove_material(9).unwrap());
    assert!(model.remove_transform(1).unwrap());
    println!("removed cell 21, m9 and tr1");
    // Surface 30 used TR1, so that reference now dangles.
    println!("now: {:?}", model.view().validate());
}

/// The flagship: renumbering updates definitions *and* every reference.
fn renumbering() {
    rule("whole-geometry renumbering");
    let mut model = Model::parse(MODEL);
    println!("cell 12 before: {}", cell_text(&model.view(), 12));

    // A mapping is any `FnMut(i64) -> i64`, so a closure can offset everything...
    model.renumber_surfaces(|id| id + 1000);
    // ...or a lookup table can move just the ids you care about.
    model.renumber_cells(|id| if id == 12 { 912 } else { id + 900 });
    println!("cell 12 after:  {}", cell_text(&model.view(), 912));
    println!("  (surfaces 3,20 -> 1003,1020; complement #11 -> #911; id 12 -> 912)");

    // Every reference class moves with its definition.
    let out = model.to_source();
    assert!(
        out.contains("912 0 (1003 -1020) #911"),
        "geometry + complement"
    );
    assert!(out.contains("914 like 911 but"), "LIKE base");
    assert!(
        out.contains("1040 RPP -1 1 -1 1 -1 2"),
        "surface definition"
    );
    assert!(
        out.contains("f4:n 901 902 911"),
        "cell ids inside the tally bins"
    );
    assert!(
        out.contains("*1050 PX 60"),
        "the `*` reflecting prefix is kept"
    );

    // The same passes exist for materials, transforms, universes, and tallies.
    model.renumber_materials(|id| id + 100);
    model.renumber_transforms(|id| id + 5);
    model.renumber_universes(|id| id + 50);
    model.renumber_tallies(|id| id + 10);
    let out = model.to_source();
    assert!(out.contains("m101 92235.80c"), "material definition");
    assert!(out.contains("901 101 -10.5"), "cell material field");
    assert!(out.contains("mt102 lwtr.10t"), "MT companion card");
    assert!(out.contains("tr6 0 0 5"), "transform definition");
    assert!(out.contains("1030 6 PZ 50"), "surface transform field");
    assert!(out.contains("u=60"), "universe definitions");
    assert!(out.contains("fill=60"), "fill references");
    assert!(out.contains("f14:n"), "tally id");
    println!("materials, transforms, universes and tallies renumbered too");

    rule("edits are lossless everywhere else");
    println!(
        "line comment kept:      {}",
        out.contains("c --- surfaces ---")
    );
    println!(
        "inline comment kept:    {}",
        out.contains("$ moderator minus the insert")
    );
    println!("continuation kept:      {}", out.contains("imp:p=1 &"));
    println!("odd spacing kept:       {}", out.contains("     vol=8.0"));
    assert!(out.contains("     vol=8.0"));
    // The result still parses cleanly.
    assert!(Model::parse(&out).diagnostics().is_empty());
}

/// `ModelView::validate` — semantic checks, distinct from parse diagnostics.
fn validation() {
    rule("validation");
    let mut model = Model::parse(MODEL);
    assert!(model.view().validate().is_empty());
    println!("the pristine model is consistent: []");

    // Dangling references: a cell pointing at a surface/cell/material that is
    // gone, or a surface whose transform is undefined.
    let mut broken = Model::parse(MODEL);
    broken.remove_surface(40).unwrap();
    broken.remove_material(3).unwrap();
    broken.remove_transform(1).unwrap();
    for problem in broken.view().validate() {
        println!("  {problem}");
    }

    // Duplicate definitions are reported once per offending number, however
    // many times it is redefined.
    let mut dupes = Model::parse("t\n1 0 -5\n\n5 SO 1\n5 SO 2\n5 SO 3\n\nm1 1001 1\n");
    for problem in dupes.view().validate() {
        println!("  {problem}");
    }
    assert_eq!(
        dupes.view().validate().len(),
        1,
        "thrice-defined, reported once"
    );
}

/// Splitting a model into universe sub-models and stitching them back together.
fn universes_and_composition() {
    rule("universes: extract & compose");
    let mut model = Model::parse(MODEL);
    println!("universes: {:?}", model.view().universe_ids());

    // Carve universe 10 into a standalone model: its cells plus *everything*
    // they reference (surfaces, materials, transforms, cells reached through
    // `#n`/`LIKE`), so the extraction runs on its own.
    let mut filler = model.extract_universe(10);
    let v = filler.view();
    println!(
        "universe 10 -> cells {:?}  surfaces {:?}  materials {:?}",
        v.cells().map(|c| c.id).collect::<Vec<_>>(),
        v.surfaces().map(|s| s.id).collect::<Vec<_>>(),
        v.materials().map(|m| m.id).collect::<Vec<_>>(),
    );
    assert!(v.validate().is_empty(), "the extraction is self-contained");

    // The inverse selection: every cell with no `u=`.
    let mut shell = model.extract_level0();
    println!(
        "level 0    -> cells {:?}",
        shell.view().cells().map(|c| c.id).collect::<Vec<_>>()
    );

    // Drop the data block for a geometry-only sub-model.
    let mut geometry_only = model.extract_universe(10);
    geometry_only.clear_data_cards();
    assert_eq!(geometry_only.view().data_cards().count(), 0);
    assert_eq!(geometry_only.view().cells().count(), 3);
    println!(
        "geometry-only universe 10: {} cells, 0 data cards",
        geometry_only.view().cells().count()
    );

    // Merge disjoint models back together.
    let mut a = Model::parse("shell\n1 0 -1 fill=10 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
    let b = Model::parse("filler\n2 0 -2 u=10 imp:n=1\n\n2 SO 3\n");
    a.merge(&[&b]).unwrap();
    println!(
        "merged -> {} cells, {} surfaces",
        a.view().cells().count(),
        a.view().surfaces().count()
    );
    assert!(a.view().validate().is_empty());

    // Merging models that share an id is refused, and reports every collision.
    let mut x = Model::parse("a\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
    let y = Model::parse("b\n1 0 -1 imp:n=1\n\n1 SO 9\n\nm1 1001 1\n");
    match x.merge(&[&y]) {
        Ok(()) => unreachable!("the ids collide"),
        Err(conflicts) => {
            for c in &conflicts {
                let kind = match c.kind {
                    ConflictKind::Cell => "cell",
                    ConflictKind::Surface => "surface",
                    ConflictKind::Material => "material",
                    ConflictKind::Transform => "transform",
                };
                println!("  conflict: {kind} {} ({c})", c.id);
            }
            assert_eq!(conflicts.len(), 3); // cell 1, surface 1, material 1
        }
    }
    // A refused merge leaves the model untouched.
    assert!(x.to_source().contains("1 SO 5"));
}

/// `EditError` — an edit that cannot be expressed, reported rather than panicked.
fn error_handling() {
    rule("edit errors");
    let mut model = Model::parse(MODEL);
    let idx = model.view().index();
    let (c13, c14, s3) = (
        idx.cell(13).unwrap(),
        idx.cell(14).unwrap(),
        idx.surface(3).unwrap(),
    );

    // A void cell has no density field to write.
    assert_eq!(
        model.set_cell_density(c13, -1.0),
        Err(EditError::NoDensityField)
    );
    println!(
        "  set_cell_density on a void cell -> {}",
        EditError::NoDensityField
    );

    // A `LIKE n BUT` cell has no geometry to restructure.
    assert_eq!(
        model.add_cell_surface(c14, 1, false),
        Err(EditError::NotEditableGeometry)
    );
    println!(
        "  add_cell_surface on a LIKE cell -> {}",
        EditError::NotEditableGeometry
    );

    // Removing a cell's last term would leave it with no region.
    let mut tiny = Model::parse("t\n1 0 -1 imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
    let ci = tiny.view().index().cell(1).unwrap();
    assert_eq!(
        tiny.remove_cell_surface(ci, 1),
        Err(EditError::WouldEmptyGeometry)
    );
    println!(
        "  removing a cell's last surface -> {}",
        EditError::WouldEmptyGeometry
    );

    // Wrong card kind, out-of-range index, and malformed card text.
    assert_eq!(
        model.set_surface_coeff(c13, 0, 1.0),
        Err(EditError::NotASurface)
    );
    assert_eq!(
        model.set_surface_coeff(s3, 9, 1.0),
        Err(EditError::IndexOutOfRange)
    );
    assert_eq!(
        model.add_cell("this is not a cell"),
        Err(EditError::InvalidCardText)
    );
    println!(
        "  set_surface_coeff on a cell    -> {}",
        EditError::NotASurface
    );
    println!(
        "  coefficient index out of range -> {}",
        EditError::IndexOutOfRange
    );
    println!(
        "  add_cell with malformed text   -> {}",
        EditError::InvalidCardText
    );

    // None of it changed the model.
    assert_eq!(model.to_source(), MODEL);
    println!("  ...and the model is untouched: to_source() still matches the input");
}
