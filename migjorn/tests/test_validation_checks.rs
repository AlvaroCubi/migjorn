use migjorn::Model;

const VALID_MODEL: &str = "Title\n\
1 1 -1 -10 IMP:N=1\n\
2 0 10 IMP:N=0\n\
\n\
10 PX 0\n\
\n\
M1 1001.80c 1.0\n";

#[test]
fn valid_model_passes() {
    let model = Model::from_text("test.mcnp", VALID_MODEL).unwrap();
    assert!(model.validation_checks().is_ok());
}

#[test]
fn missing_surface_is_detected() {
    let text = "Title\n\
1 1 -1 -99 IMP:N=1\n\
2 0 99 IMP:N=0\n\
\n\
10 PX 0\n\
\n\
M1 1001.80c 1.0\n";
    let model = Model::from_text("test.mcnp", text).unwrap();
    let report = model.validation_checks().unwrap_err();
    assert!(report.missing_surfaces.contains(&99));
    assert!(report.duplicate_surfaces.is_empty());
}

#[test]
fn missing_material_is_detected() {
    let text = "Title\n\
1 99 -1 -10 IMP:N=1\n\
2 0 10 IMP:N=0\n\
\n\
10 PX 0\n\
\n\
M1 1001.80c 1.0\n";
    let model = Model::from_text("test.mcnp", text).unwrap();
    let report = model.validation_checks().unwrap_err();
    assert!(report.missing_materials.contains(&99));
    assert!(report.duplicate_materials.is_empty());
}

#[test]
fn duplicate_cell_ids_are_detected() {
    let text = "Title\n\
1 1 -1 -10 IMP:N=1\n\
1 0 10 IMP:N=0\n\
\n\
10 PX 0\n\
\n\
M1 1001.80c 1.0\n";
    let model = Model::from_text("test.mcnp", text).unwrap();
    let report = model.validation_checks().unwrap_err();
    assert!(report.duplicate_cells.contains(&1));
    assert!(report.missing_cells.is_empty());
}

#[test]
fn duplicate_surface_ids_are_detected() {
    let text = "Title\n\
1 1 -1 -10 IMP:N=1\n\
2 0 10 IMP:N=0\n\
\n\
10 PX 0\n\
10 PX 1\n\
\n\
M1 1001.80c 1.0\n";
    let model = Model::from_text("test.mcnp", text).unwrap();
    let report = model.validation_checks().unwrap_err();
    assert!(report.duplicate_surfaces.contains(&10));
    assert!(report.missing_surfaces.is_empty());
}

#[test]
fn duplicate_material_ids_are_detected() {
    let text = "Title\n\
1 1 -1 -10 IMP:N=1\n\
2 0 10 IMP:N=0\n\
\n\
10 PX 0\n\
\n\
M1 1001.80c 1.0\n\
M1 8016.80c 1.0\n";
    let model = Model::from_text("test.mcnp", text).unwrap();
    let report = model.validation_checks().unwrap_err();
    assert!(report.duplicate_materials.contains(&1));
    assert!(report.missing_materials.is_empty());
}

#[test]
fn duplicate_transform_ids_are_detected() {
    let text = "Title\n\
1 1 -1 -10 IMP:N=1\n\
2 0 10 IMP:N=0\n\
\n\
10 PX 0\n\
\n\
M1 1001.80c 1.0\n\
TR7 1 0 0\n\
TR7 0 1 0\n";
    let model = Model::from_text("test.mcnp", text).unwrap();
    let report = model.validation_checks().unwrap_err();
    assert!(report.duplicate_transforms.contains(&7));
    assert!(report.missing_transforms.is_empty());
}

#[test]
fn missing_transform_via_fill_is_detected() {
    let text = "Title\n\
1 1 -1 -10 FILL=5 (99) IMP:N=1\n\
2 0 10 IMP:N=0\n\
\n\
10 PX 0\n\
\n\
M1 1001.80c 1.0\n";
    let model = Model::from_text("test.mcnp", text).unwrap();
    let report = model.validation_checks().unwrap_err();
    assert!(report.missing_transforms.contains(&99));
    assert!(report.duplicate_transforms.is_empty());
}

#[test]
fn missing_transform_via_surface_is_detected() {
    // Surface 10 references transform 99, which is not defined
    let text = "Title\n\
1 1 -1 -10 IMP:N=1\n\
2 0 10 IMP:N=0\n\
\n\
10 99 PX 0\n\
\n\
M1 1001.80c 1.0\n";
    let model = Model::from_text("test.mcnp", text).unwrap();
    let report = model.validation_checks().unwrap_err();
    assert!(report.missing_transforms.contains(&99));
    assert!(report.duplicate_transforms.is_empty());
}

#[test]
fn missing_cell_in_complement_is_detected() {
    // Cell 2 uses #99 (complement of cell 99), but cell 99 is not defined
    let text = "Title\n\
1 1 -1 -10 IMP:N=1\n\
2 0 #99 IMP:N=0\n\
\n\
10 PX 0\n\
\n\
M1 1001.80c 1.0\n";
    let model = Model::from_text("test.mcnp", text).unwrap();
    let report = model.validation_checks().unwrap_err();
    assert!(report.missing_cells.contains(&99));
    assert!(report.duplicate_cells.is_empty());
}

#[test]
fn report_display_mentions_duplicate_and_missing() {
    let text = "Title\n\
1 1 -1 -99 IMP:N=1\n\
1 0 99 IMP:N=0\n\
\n\
10 PX 0\n\
\n\
M1 1001.80c 1.0\n";
    let model = Model::from_text("test.mcnp", text).unwrap();
    let report = model.validation_checks().unwrap_err();
    let msg = report.to_string();
    assert!(msg.contains("Duplicate cell"), "got: {msg}");
    assert!(msg.contains("Missing surface"), "got: {msg}");
}
