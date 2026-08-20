//! The correct-parsing contract: assert every pinned fact in
//! `tests/expectations.toml` through the public view API.
//!
//! Where the round-trip suite proves we preserve the *bytes*, this proves we
//! understand the *structure*. It mirrors `tests/test_parsing.py` fact for fact,
//! so that suite is green by construction once the Python bindings exist.

use migjorn::{CellView, Model, SurfaceView};
use std::path::{Path, PathBuf};
use toml::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn expectations() -> toml::Table {
    let text = std::fs::read_to_string(root().join("tests/expectations.toml"))
        .expect("read expectations.toml");
    text.parse::<toml::Table>()
        .expect("parse expectations.toml")
}

fn model_for(name: &str) -> Model {
    let path = root().join("tests/corpus").join(format!("{name}.mcnp"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let model = Model::parse(&text);
    assert_eq!(model.to_source(), text, "{name}: round-trip");
    model
}

fn int(v: &Value) -> i64 {
    v.as_integer().expect("integer")
}

fn float(v: &Value) -> f64 {
    v.as_float()
        .or_else(|| v.as_integer().map(|i| i as f64))
        .expect("float")
}

fn floats(v: &Value) -> Vec<f64> {
    v.as_array().expect("array").iter().map(float).collect()
}

fn ints(v: &Value) -> Vec<i64> {
    v.as_array().expect("array").iter().map(int).collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

#[test]
fn every_pinned_fact_holds() {
    let expect = expectations();
    assert!(!expect.is_empty(), "expectations.toml is empty");

    for (name, section) in &expect {
        let section = section.as_table().expect("section table");
        let model = model_for(name);

        check_counts(name, &model, section);

        if let Some(cells) = section.get("cell").and_then(Value::as_table) {
            for (id, want) in cells {
                let id: i64 = id.parse().expect("cell id");
                let cell = model
                    .cell(id)
                    .unwrap_or_else(|| panic!("{name}: cell {id} not found"));
                check_cell(name, id, cell, want.as_table().expect("cell table"));
            }
        }

        if let Some(surfaces) = section.get("surface").and_then(Value::as_table) {
            for (id, want) in surfaces {
                let id: i64 = id.parse().expect("surface id");
                let surface = model
                    .surface(id)
                    .unwrap_or_else(|| panic!("{name}: surface {id} not found"));
                check_surface(name, id, surface, want.as_table().expect("surface table"));
            }
        }

        if let Some(materials) = section.get("material").and_then(Value::as_table) {
            for (id, want) in materials {
                let id: i64 = id.parse().expect("material id");
                let material = model
                    .material(id)
                    .unwrap_or_else(|| panic!("{name}: material {id} not found"));
                if let Some(entries) = want.get("entries") {
                    let got: Vec<(String, f64)> = material.entries();
                    let want: Vec<(String, f64)> = entries
                        .as_array()
                        .expect("entries array")
                        .iter()
                        .map(|e| {
                            let pair = e.as_array().expect("entry pair");
                            (pair[0].as_str().expect("zaid").to_owned(), float(&pair[1]))
                        })
                        .collect();
                    assert_eq!(got.len(), want.len(), "{name}: material {id} entry count");
                    for (g, w) in got.iter().zip(&want) {
                        assert_eq!(g.0, w.0, "{name}: material {id} zaid");
                        assert!(close(g.1, w.1), "{name}: material {id} fraction");
                    }
                }
            }
        }

        if let Some(transforms) = section.get("transform").and_then(Value::as_table) {
            for (id, want) in transforms {
                let id: i64 = id.parse().expect("transform id");
                let transform = model
                    .transform(id)
                    .unwrap_or_else(|| panic!("{name}: transform {id} not found"));
                if let Some(v) = want.get("degrees") {
                    assert_eq!(
                        transform.degrees(),
                        v.as_bool().expect("bool"),
                        "{name}: transform {id} degrees"
                    );
                }
                if let Some(v) = want.get("displacement") {
                    let got = transform.displacement();
                    let want = floats(v);
                    assert_eq!(got.len(), want.len(), "{name}: transform {id} displacement");
                    for (g, w) in got.iter().zip(&want) {
                        assert!(close(*g, *w), "{name}: transform {id} displacement");
                    }
                }
            }
        }
    }
}

fn check_counts(name: &str, model: &Model, section: &toml::Table) {
    for (key, got) in [
        ("num_cells", model.num_cells()),
        ("num_surfaces", model.num_surfaces()),
        ("num_materials", model.num_materials()),
        ("num_transforms", model.num_transforms()),
    ] {
        if let Some(want) = section.get(key) {
            assert_eq!(got as i64, int(want), "{name}: {key}");
        }
    }
}

fn check_cell(name: &str, id: i64, cell: CellView<'_>, want: &toml::Table) {
    let at = format!("{name}: cell {id}");
    if let Some(v) = want.get("material") {
        assert_eq!(cell.material(), Some(int(v)), "{at} material");
    }
    if let Some(v) = want.get("density") {
        let got = cell
            .density()
            .unwrap_or_else(|| panic!("{at} has no density"));
        assert!(close(got, float(v)), "{at} density: {got}");
    }
    if let Some(v) = want.get("void") {
        assert_eq!(cell.is_void(), v.as_bool().expect("bool"), "{at} void");
    }
    if let Some(v) = want.get("signed_surfaces") {
        assert_eq!(cell.signed_surfaces(), ints(v), "{at} signed_surfaces");
    }
    if let Some(v) = want.get("surface_ids") {
        let mut got = cell.surface_ids();
        got.sort_unstable();
        got.dedup();
        let mut w = ints(v);
        w.sort_unstable();
        w.dedup();
        assert_eq!(got, w, "{at} surface_ids");
    }
    if let Some(v) = want.get("cell_refs") {
        let mut got = cell.cell_refs();
        got.sort_unstable();
        got.dedup();
        let mut w = ints(v);
        w.sort_unstable();
        w.dedup();
        assert_eq!(got, w, "{at} cell_refs");
    }
    if let Some(v) = want.get("like") {
        assert_eq!(cell.like(), Some(int(v)), "{at} like");
    }
    if let Some(v) = want.get("universe") {
        assert_eq!(cell.universe(), Some(int(v)), "{at} universe");
    }
    if let Some(v) = want.get("lattice") {
        assert_eq!(
            cell.param("lat").is_some(),
            v.as_bool().expect("bool"),
            "{at} lattice"
        );
    }
    for (key, param) in [("imp_n", "imp:n"), ("imp_p", "imp:p"), ("vol", "vol")] {
        if let Some(v) = want.get(key) {
            let got = cell
                .param(param)
                .unwrap_or_else(|| panic!("{at} has no {param}"));
            assert_eq!(got.value, v.as_str().expect("string"), "{at} {param}");
        }
    }
    if let Some(v) = want.get("fill_universe") {
        let fill = cell.fill().unwrap_or_else(|| panic!("{at} has no fill"));
        assert_eq!(fill.universe, int(v), "{at} fill universe");
    }
    if let Some(v) = want.get("fill_starred") {
        let fill = cell.fill().unwrap_or_else(|| panic!("{at} has no fill"));
        assert_eq!(
            fill.starred,
            v.as_bool().expect("bool"),
            "{at} fill starred"
        );
    }
}

fn check_surface(name: &str, id: i64, surface: SurfaceView<'_>, want: &toml::Table) {
    let at = format!("{name}: surface {id}");
    if let Some(v) = want.get("kind") {
        assert_eq!(
            surface.kind(),
            Some(v.as_str().expect("string")),
            "{at} kind"
        );
    }
    if let Some(v) = want.get("coeffs") {
        let got = surface.coeffs();
        let w = floats(v);
        assert_eq!(got.len(), w.len(), "{at} coeff count: {got:?}");
        for (g, e) in got.iter().zip(&w) {
            assert!(close(*g, *e), "{at} coeffs: {got:?} != {w:?}");
        }
    }
    if let Some(v) = want.get("reflective") {
        assert_eq!(
            surface.reflective(),
            v.as_bool().expect("bool"),
            "{at} reflective"
        );
    }
    if let Some(v) = want.get("white") {
        assert_eq!(surface.white(), v.as_bool().expect("bool"), "{at} white");
    }
    if let Some(v) = want.get("transform") {
        assert_eq!(surface.transform(), Some(int(v)), "{at} transform");
    }
}
