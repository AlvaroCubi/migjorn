use migjorn::cli::run;
use migjorn::{DataCard, Model};
use std::path::Path;
use tempfile::NamedTempFile;

fn input_path() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../resources/simple_model.mcnp"
    ))
}

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|&s| s.to_string()).collect()
}

#[test]
fn info_returns_0_for_valid_file() {
    let code = run(args(&["migjorn", "info", input_path().to_str().unwrap()]));
    assert_eq!(code, 0);
}

#[test]
fn check_returns_0_for_valid_file() {
    let code = run(args(&[
        "migjorn",
        "parsing-check",
        input_path().to_str().unwrap(),
    ]));
    assert_eq!(code, 0);
}

#[test]
fn check_returns_1_for_missing_file() {
    let code = run(args(&[
        "migjorn",
        "parsing-check",
        "/dev/null/does_not_exist.mcnp",
    ]));
    assert_eq!(code, 1);
}

#[test]
fn renumber_cells_shifts_all_ids_in_output() {
    let out = NamedTempFile::new().unwrap();
    let code = run(args(&[
        "migjorn",
        "renumber",
        input_path().to_str().unwrap(),
        out.path().to_str().unwrap(),
        "--cells",
        "1000",
    ]));
    assert_eq!(code, 0);

    let model = Model::from_file(out.path()).unwrap();
    let mut cell_ids: Vec<u32> = model.cells.iter().map(|c| c.cell_id()).collect();
    cell_ids.sort();
    assert_eq!(cell_ids, vec![1001, 1002, 1003, 1004, 1005]);
}

#[test]
fn renumber_with_cell_range_only_shifts_ids_in_range() {
    let out = NamedTempFile::new().unwrap();
    let code = run(args(&[
        "migjorn",
        "renumber",
        input_path().to_str().unwrap(),
        out.path().to_str().unwrap(),
        "--cells",
        "1000",
        "--cell-range",
        "3",
        "10",
    ]));
    assert_eq!(code, 0);

    let model = Model::from_file(out.path()).unwrap();
    let mut cell_ids: Vec<u32> = model.cells.iter().map(|c| c.cell_id()).collect();
    cell_ids.sort();
    assert_eq!(cell_ids, vec![1, 2, 1003, 1004, 1005]);
}

#[test]
fn renumber_surfaces() {
    let out = NamedTempFile::new().unwrap();
    let code = run(args(&[
        "migjorn",
        "renumber",
        input_path().to_str().unwrap(),
        out.path().to_str().unwrap(),
        "--surfaces",
        "-10",
        "--surface-range",
        "13",
        "15",
    ]));
    assert_eq!(code, 0);

    let model = Model::from_file(out.path()).unwrap();
    let surface_ids: Vec<u32> = model.surfaces.iter().map(|s| s.surface_id()).collect();
    assert_eq!(surface_ids, vec![10, 11, 12, 3, 4, 5]);
}

#[test]
fn renumber_materials() {
    let out = NamedTempFile::new().unwrap();
    let code = run(args(&[
        "migjorn",
        "renumber",
        input_path().to_str().unwrap(),
        out.path().to_str().unwrap(),
        "--materials",
        "1000",
        "--material-range",
        "300",
        "3000",
    ]));
    assert_eq!(code, 0);

    let model = Model::from_file(out.path()).unwrap();
    let mut material_ids: Vec<u32> = model
        .data_cards
        .iter()
        .filter_map(|card| match card {
            DataCard::Material(m) => Some(m.material_id()),
            _ => None,
        })
        .collect();
    material_ids.sort();
    assert_eq!(material_ids, vec![100, 1400]);
}
