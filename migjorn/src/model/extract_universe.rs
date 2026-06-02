use std::{collections::HashSet, sync::Arc};

use crate::{CellCard, DataCard, GeoElement, SurfaceCard, parser_utils::OriginalBytes};

use super::Model;

#[derive(Default, Debug)]
struct IdsToExtract {
    universe_ids: HashSet<u32>, // To avoid repeating the same universe
    cell_ids: HashSet<u32>,
    surface_ids: HashSet<u32>,
    material_ids: HashSet<u32>,
    transform_ids: HashSet<u32>,
}

impl Model {
    /// Extract a new model containing only the cells, surfaces, and data cards
    /// that are reachable from the specified universe. This is useful for
    /// extracting a smaller sub-model.
    pub fn extract_universe(&self, universe_id: u32) -> Self {
        let mut ids_to_extract = IdsToExtract::default();
        self.get_ids_of_universe_by_cells(universe_id, &mut ids_to_extract);

        for surface in &self.surfaces {
            if ids_to_extract
                .surface_ids
                .contains(&surface.surface_id.value)
                && let Some(transform_id) = surface.transform_id()
            {
                ids_to_extract.transform_ids.insert(transform_id);
            }
        }

        let title = OriginalBytes::from(format!("Universe {}\n", universe_id).as_str());
        let cells: Vec<CellCard> = self
            .cells
            .iter()
            .filter(|c| ids_to_extract.cell_ids.contains(&c.cell_id()))
            .cloned()
            .collect();
        let surfaces: Vec<SurfaceCard> = self
            .surfaces
            .iter()
            .filter(|s| ids_to_extract.surface_ids.contains(&s.surface_id.value))
            .cloned()
            .collect();
        let data_cards: Vec<DataCard> = self
            .data_cards
            .iter()
            .filter(|d| {
                d.as_material()
                    .is_some_and(|m| ids_to_extract.material_ids.contains(&m.material_id.value))
                    || d.as_transform().is_some_and(|t| {
                        ids_to_extract.transform_ids.contains(&t.transform_id.value)
                    })
            })
            .cloned()
            .collect();
        Self {
            path: self.path.clone(),
            source: Arc::from(Vec::new()), // No source for the extracted model
            title,
            cells,
            surfaces,
            data_cards,
        }
    }

    /// Recursively collect IDs of cells, surfaces, materials and transforms that appear in
    /// the cell definitions of the cells in the specified universe. The transforms that
    /// are applied in the surfaces section are not considered here.
    fn get_ids_of_universe_by_cells(&self, universe_id: u32, ids_to_extract: &mut IdsToExtract) {
        ids_to_extract.universe_ids.insert(universe_id);

        let matching_cells = self
            .cells
            .iter()
            .filter(|c| c.get_universe() == Some(universe_id));

        for cell in matching_cells {
            ids_to_extract.cell_ids.insert(cell.cell_id());
            let mat_id = cell.material_id();
            if mat_id != 0 {
                ids_to_extract.material_ids.insert(mat_id);
            }
            ids_to_extract
                .surface_ids
                .extend(cell.geometry().filter_map(|geo| match geo {
                    GeoElement::Surface(surface_id) => Some(surface_id.unsigned_abs()),
                    _ => None,
                }));

            if let Some(fill) = cell.get_fill() {
                if let Some(transform_id) = fill.transform {
                    ids_to_extract.transform_ids.insert(transform_id);
                }

                if ids_to_extract.universe_ids.insert(fill.universe) {
                    self.get_ids_of_universe_by_cells(fill.universe, ids_to_extract);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    const EXAMPLE: &str = "\
Title of the file
1 0 -1 FILL=5
100 2 -2.5   $ an inline comment
        (12 : -55) imp:N = 1 U=5
999 0 99 U=5 FILL=98
5000 0 -50001 U=98

1 SO 100
12 PX 2
55 11 SO 120
99 SO 300
50001 SO 500

M2 26054.80c 5.85
TR11 10 10 10
SDEF pos 0 0 0
";

    const EXAMPLE_UNIVERSE_5: &str = "\
Universe 5
100 2 -2.5   $ an inline comment
        (12 : -55) IMP:N=1 U=5
999 0 99 U=5 FILL=98
5000 0 -50001 U=98

12 PX 2
55 11 SO 120
99 SO 300
50001 SO 500

M2 26054.80c 5.85
TR11 10 10 10
";

    #[test]
    fn test_extract_universe() {
        let model = Model::from_text(".", EXAMPLE).unwrap();
        let extracted_model = model.extract_universe(5);
        assert_eq!(
            extracted_model.cell_ids(),
            [100, 999, 5000].iter().cloned().collect()
        );
        assert_eq!(
            extracted_model.surface_ids(),
            [12, 55, 99, 50001].iter().cloned().collect()
        );
        assert_eq!(
            extracted_model.material_ids(),
            [2].iter().cloned().collect()
        );
        assert_eq!(
            extracted_model.transform_ids(),
            [11].iter().cloned().collect()
        );
        let temp_file = NamedTempFile::new().unwrap();
        let temp_path = temp_file.path();
        extracted_model.write_to_file(temp_path).unwrap();
        let extracted_text = std::fs::read_to_string(temp_path).unwrap();
        assert_eq!(extracted_text, EXAMPLE_UNIVERSE_5);
    }
}
