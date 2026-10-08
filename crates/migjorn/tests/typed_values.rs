//! Typed reads of the values llevant needs: fills, transforms, surface
//! references, scalar parameters, and the well-formed / diagnostic signal when
//! one of them cannot be read.

use migjorn::{FillEntry, FillSpec, Model, Severity, SurfaceRef, TransformSpec};

fn model(cells: &str, surfaces: &str, data: &str) -> Model {
    Model::parse(&format!("t\n{cells}\n\n{surfaces}\n\n{data}\n"))
}

fn cells(cells: &str) -> Model {
    model(cells, "1 SO 5\n2 RPP -1 1 -1 1 -1 1", "m1 1001 1")
}

#[test]
fn zoo_style_fill_array() {
    // The shape used by llevant's `universes.i`: transform groups inside the
    // array, both a TR number and an inline displacement.
    let m = cells("1 0 -1 lat=1 u=3 fill=-1:0 0:0 0:0 4 (4) 5 (0.5 0 0) imp:n=1");
    let c = m.cell(1).unwrap();
    assert!(c.well_formed(), "{:?}", c.diagnostics());
    let (spec, starred) = c.fill_spec().unwrap().unwrap();
    assert!(!starred);
    assert_eq!(
        spec,
        FillSpec::Array {
            ranges: [(-1, 0), (0, 0), (0, 0)],
            entries: vec![
                FillEntry {
                    universe: 4,
                    transform: Some(TransformSpec::Number(4)),
                },
                FillEntry {
                    universe: 5,
                    transform: Some(TransformSpec::Inline(vec![0.5, 0.0, 0.0])),
                },
            ],
        }
    );
    assert_eq!(c.lattice(), Some(1));
    assert_eq!(c.universe(), Some(3));
    // the editing form still declines the array
    assert!(c.fill().is_none());
}

#[test]
fn a_bad_fill_array_is_not_well_formed_and_says_why() {
    let m = cells("1 0 -1 lat=1 fill=0:1 0:1 0:0 1 2 3 imp:n=1");
    let c = m.cell(1).unwrap();
    assert_eq!(
        c.fill_spec().unwrap().unwrap_err(),
        "fill array has 3 entries for 4 elements"
    );
    assert!(!c.well_formed());
    let diags = c.diagnostics();
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(
        diags[0].message,
        "cell 1: fill array has 3 entries for 4 elements"
    );
    assert_eq!(diags[0].slot, Some(c.slot()));
    // the card-local span covers the array's tokens
    assert_eq!(&c.text()[diags[0].span.clone()], "0:1 0:1 0:0 1 2 3");
    // and the same problem is in the model-wide list, against the cell's slot
    assert!(m
        .diagnostics()
        .iter()
        .any(|d| d.slot == Some(c.slot()) && d.message.contains("3 entries")));
}

#[test]
fn star_fill_with_inline_transform() {
    let m = cells("1 0 -1 *fill=7 (1 2 3 30 60 90 120 30 90 90 90 0) imp:n=1");
    let (spec, starred) = m.cell(1).unwrap().fill_spec().unwrap().unwrap();
    assert!(starred);
    let FillSpec::Single {
        universe: 7,
        transform: Some(TransformSpec::Inline(v)),
    } = spec
    else {
        panic!("{spec:?}")
    };
    assert_eq!(v.len(), 12);
}

#[test]
fn trcl_forms_from_the_reference_model() {
    let m = cells(
        "1 0 -1 trcl=111 imp:n=1\n\
         2 0 -1 trcl=(284.0 1 0) imp:n=1\n\
         3 0 -1 *trcl=(314.0 0 0  30 60 90  120 30 90  90 90 0) imp:n=1\n\
         4 0 -1 trcl=(1 2) imp:n=1",
    );
    let trcl = |id| m.cell(id).unwrap().trcl().unwrap();
    assert_eq!(trcl(1).unwrap(), (TransformSpec::Number(111), false));
    assert_eq!(
        trcl(2).unwrap().0,
        TransformSpec::Inline(vec![284.0, 1.0, 0.0])
    );
    assert!(trcl(3).unwrap().1);
    assert!(trcl(4).is_err());
    assert!(!m.cell(4).unwrap().well_formed());
    assert!(m.cell(1).unwrap().well_formed());
    assert!(m.cell(2).unwrap().trcl().is_some());
    assert!(cells("1 0 -1 imp:n=1").cell(1).unwrap().trcl().is_none());
}

#[test]
fn transform_cards_with_shortcuts_are_not_well_formed() {
    let m = model(
        "1 0 -1 imp:n=1",
        "1 SO 5",
        "tr1 0 0 0 1 0 0 2J 0 0 1\ntr2 0 0 0\n*tr3 1 2 3 30 60 90 120 30 90 90 90 0",
    );
    let tr1 = m.transform(1).unwrap();
    assert!(!tr1.well_formed());
    // nothing after the shortcut is shifted into an earlier slot
    assert_eq!(tr1.coeffs(), vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    assert!(tr1.diagnostics()[0].message.contains("`2J`"));
    assert!(m.transform(2).unwrap().well_formed());
    assert!(m.transform(3).unwrap().well_formed());
    assert!(m
        .diagnostics()
        .iter()
        .any(|d| d.message.contains("transform 1")));
}

#[test]
fn surface_cards_with_shortcuts_are_not_well_formed() {
    let m = model("1 0 -1 imp:n=1", "1 RPP -1 1 2R\n2 PX 3", "");
    let s = m.surface(1).unwrap();
    assert!(!s.well_formed());
    assert_eq!(s.coeffs(), vec![-1.0, 1.0]);
    assert!(s.diagnostics()[0].message.contains("`2R`"));
    assert!(m.surface(2).unwrap().well_formed());
}

#[test]
fn geometry_terms_carry_parsed_references() {
    let m = cells("1 0 -2.1 +1 #3 #(-1 2.6) imp:n=1\n3 0 1 imp:n=0");
    let c = m.cell(1).unwrap();
    assert!(c.well_formed(), "{:?}", c.diagnostics());
    assert_eq!(
        c.surface_refs(),
        vec![
            SurfaceRef {
                id: 2,
                facet: Some(1),
                negative: true
            },
            SurfaceRef {
                id: 1,
                facet: None,
                negative: false
            },
            SurfaceRef {
                id: 1,
                facet: None,
                negative: true
            },
            SurfaceRef {
                id: 2,
                facet: Some(6),
                negative: false
            },
        ]
    );
    // facets count as their macrobody
    assert_eq!(c.signed_surfaces(), vec![-2, 1, -1, 2]);
    assert_eq!(c.surface_ids(), vec![2, 1, 1, 2]);
    assert_eq!(c.cell_refs(), vec![3]);
    let complement = c.geometry().into_iter().find(|t| t.cell.is_some()).unwrap();
    assert_eq!(complement.cell, Some(3));
    assert_eq!(complement.surface, None);
    assert_eq!(SurfaceRef::parse("-470.1").unwrap().to_string(), "-470.1");
}

#[test]
fn unreadable_geometry_numbers_are_not_well_formed() {
    for (cell, needle) in [
        ("1 0 -1.0 imp:n=1", "`-1.0`"),
        ("1 0 -1e3 imp:n=1", "`-1e3`"),
        ("1 0 -1 #1.5 imp:n=1", "`#1.5`"),
        ("1 0 (-1 imp:n=1", "unclosed"),
        ("1 0 -1) imp:n=1", "unmatched"),
    ] {
        let m = cells(cell);
        let c = m.cell(1).unwrap();
        assert!(!c.well_formed(), "{cell}");
        assert!(
            c.diagnostics().iter().any(|d| d.message.contains(needle)),
            "{cell}: {:?}",
            c.diagnostics()
        );
    }
}

#[test]
fn scalar_parameters() {
    let m = cells(
        "1 1 -1.0 -1 imp:n=1\n\
         2 like 1 but mat=0 rho=1.0-3 imp:p=0 imp:n=2\n\
         3 0 -1 imp:n,p=0.5 lat=2\n\
         4 0 -1 imp:n=1 lat=3 mat=-1 rho=x u=1.5",
    );
    let c2 = m.cell(2).unwrap();
    assert_eq!(c2.material_override(), Some(0));
    assert_eq!(c2.density_override(), Some(1.0e-3));
    // the right particle, not the first IMP
    assert_eq!(c2.importance("n"), Some(2.0));
    assert_eq!(c2.importance("P"), Some(0.0));
    assert_eq!(c2.importance("e"), None);
    assert!(c2.well_formed(), "{:?}", c2.diagnostics());

    let c3 = m.cell(3).unwrap();
    assert_eq!(c3.importance("n"), Some(0.5));
    assert_eq!(c3.importance("p"), Some(0.5));
    assert_eq!(c3.lattice(), Some(2));
    assert_eq!(m.cell(1).unwrap().material_override(), None);

    let c4 = m.cell(4).unwrap();
    assert!(!c4.well_formed());
    assert_eq!(c4.lattice(), None);
    assert_eq!(c4.material_override(), None);
    assert_eq!(c4.density_override(), None);
    assert_eq!(c4.universe(), None);
    let messages: Vec<String> = c4.diagnostics().into_iter().map(|d| d.message).collect();
    assert_eq!(messages.len(), 4, "{messages:?}");
    assert!(messages.iter().all(|m| m.starts_with("cell 4: ")));
}

#[test]
fn a_parameter_given_twice_is_reported() {
    let m = cells("1 0 -1 imp:n=1 imp:n,p=0\n2 0 -1 u=1 U=2 imp:n=1");
    let c1 = m.cell(1).unwrap();
    assert!(!c1.well_formed());
    assert!(c1.diagnostics()[0]
        .message
        .contains("IMP:N is given more than once"));
    assert!(!m.cell(2).unwrap().well_formed());
}

#[test]
fn per_card_diagnostics_follow_edits() {
    let mut m = cells("1 0 -1 lat=4 imp:n=1");
    let slot = m.cell(1).unwrap().slot();
    assert_eq!(m.cell(1).unwrap().diagnostics().len(), 1);
    m.set_cell_param(slot, "lat", "1").unwrap();
    let c = m.cell(1).unwrap();
    assert!(c.diagnostics().is_empty(), "{:?}", c.diagnostics());
    assert!(c.well_formed());
}

#[test]
fn duplicate_ids_name_the_card() {
    let m = cells("1 0 -1 imp:n=1\n1 0 1 imp:n=0");
    let d = m
        .diagnostics()
        .iter()
        .find(|d| d.message == "duplicate cell id 1")
        .unwrap();
    assert_eq!(d.severity, Severity::Error);
    assert!(d.slot.is_some());
}

#[test]
fn parse_float_reads_implicit_exponents() {
    assert_eq!(migjorn::parse_float("6.02+23"), Some(6.02e23));
    assert_eq!(migjorn::parse_float("1001.31c"), None);
}
