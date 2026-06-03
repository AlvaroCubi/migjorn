use migjorn::{Card, GeoElement, Model};
use std::path::Path;
use tempfile::NamedTempFile;

fn input_path() -> std::path::PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../resources/simple_model.mcnp"
    ))
    .to_path_buf()
}

#[test]
fn test_read_and_write() {
    let input_path = input_path();

    // Read the model
    let model = Model::from_file(input_path).unwrap();

    // Write to a temp file
    let temp_file = NamedTempFile::new().unwrap();
    let temp_path = temp_file.path();
    println!("Temp file: {}", temp_path.display());
    model.write_to_file(temp_path).unwrap();

    // Read it back
    let model_roundtrip = Model::from_file(temp_path).unwrap();

    // Verify same number of cards
    assert_eq!(model.cells.len(), model_roundtrip.cells.len());
    assert_eq!(model.surfaces.len(), model_roundtrip.surfaces.len());
    assert_eq!(model.data_cards.len(), model_roundtrip.data_cards.len());

    // Verify each card's bytes are the same (OriginalBytes::PartialEq compares content)
    assert_eq!(model.title, model_roundtrip.title);
    for (original_cell, roundtrip_cell) in model.cells.iter().zip(model_roundtrip.cells.iter()) {
        assert_eq!(original_cell.updated_text(), roundtrip_cell.updated_text());
    }
    for (original_surface, roundtrip_surface) in
        model.surfaces.iter().zip(model_roundtrip.surfaces.iter())
    {
        assert_eq!(
            original_surface.updated_text(),
            roundtrip_surface.updated_text()
        );
    }
    for (original_data_card, roundtrip_data_card) in model
        .data_cards
        .iter()
        .zip(model_roundtrip.data_cards.iter())
    {
        assert_eq!(
            original_data_card.updated_text(),
            roundtrip_data_card.updated_text()
        );
    }
}

#[test]
fn test_sequential_read() {
    let input_path = input_path();

    let model_seq = Model::from_file_sequential(&input_path).unwrap();
    let model_par = Model::from_file(&input_path).unwrap();

    // Semantic equality: same parsed content regardless of parallel vs sequential parsing
    assert!(model_seq == model_par);
}

#[test]
fn test_geometry_element_iteration() {
    let model = Model::from_file(&input_path()).unwrap();

    // Every cell has at least one geometry element
    for cell in &model.cells {
        assert!(
            cell.geometry().count() > 0,
            "cell {} has no geometry elements",
            cell.cell_id()
        );
    }

    let cell1_geo: Vec<GeoElement> = model.cells[0].geometry().collect();
    assert_eq!(cell1_geo[1], GeoElement::Surface(-10));
}
