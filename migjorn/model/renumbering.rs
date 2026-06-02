use std::ops::RangeInclusive;

use crate::cell_card::{GeoElementSpanned, ParamType};
use crate::{FillData, Model};

impl Model {
    /// Renumber cell ids for a specific subset of cell ids
    pub fn renumber_cells(&mut self, id_range_to_mod: RangeInclusive<u32>, offset: i32) {
        for cell in self.cells.iter_mut() {
            // Renumber the card ID
            if id_range_to_mod.contains(&cell.cell_id.value) {
                cell.cell_id.value = ((cell.cell_id.value as i32) + offset) as u32;
            }

            // Renumber any cell references in the geometry
            for geo_element in &mut cell.geometry {
                if let GeoElementSpanned::Cell(cell_ref) = geo_element
                    && id_range_to_mod.contains(&cell_ref.value)
                {
                    cell_ref.value = ((cell_ref.value as i32) + offset) as u32;
                }
            }
        }
    }

    /// Renumber surface ids for a specific subset of surface ids
    pub fn renumber_surfaces(&mut self, id_range_to_mod: RangeInclusive<u32>, offset: i32) {
        // Renumber surface IDs in cell definitions
        for cell in self.cells.iter_mut() {
            for geo_element in &mut cell.geometry {
                if let GeoElementSpanned::Surface(surface_ref) = geo_element
                    && id_range_to_mod.contains(&surface_ref.value.unsigned_abs())
                {
                    surface_ref.value += offset * surface_ref.value.signum();
                }
            }
        }

        // Renumber the card IDs
        for surface in self.surfaces.iter_mut() {
            if id_range_to_mod.contains(&surface.surface_id.value) {
                surface.surface_id.value = ((surface.surface_id.value as i32) + offset) as u32;
            }
        }
    }

    /// Renumber material IDs for a specific subset of materials
    pub fn renumber_materials(&mut self, id_range_to_mod: RangeInclusive<u32>, offset: i32) {
        // Renumber the material IDs in cell definitions
        for cell in self.cells.iter_mut() {
            if id_range_to_mod.contains(&cell.material_id.value) {
                cell.material_id.value = ((cell.material_id.value as i32) + offset) as u32;
            }
        }

        // Renumber the material IDs in data cards
        for data_card in self.data_cards.iter_mut() {
            if let Some(mat) = data_card.as_material_mut()
                && id_range_to_mod.contains(&mat.material_id.value)
            {
                mat.material_id.value = ((mat.material_id.value as i32) + offset) as u32;
            }
        }
    }

    // Renumber transformation cards
    pub fn renumber_transformations(&mut self, id_range_to_mod: RangeInclusive<u32>, offset: i32) {
        // Renumber the transformation IDs in the FILL parameters of cell definitions
        for cell in self.cells.iter_mut() {
            for param in cell.params.iter_mut() {
                if let ParamType::Fill(FillData { transform, .. }) = &mut param.param_type
                    && let Some(transform) = transform
                    && id_range_to_mod.contains(transform)
                {
                    *transform = ((*transform as i32) + offset) as u32;
                }
            }
        }

        // Renumber the transformation IDs in surface definitions
        for surface in self.surfaces.iter_mut() {
            if let Some(transform_id) = &mut surface.transform_id
                && id_range_to_mod.contains(&transform_id.value)
            {
                transform_id.value = ((transform_id.value as i32) + offset) as u32;
            }
        }

        // Renumber the transformation IDs in data cards
        for data_card in self.data_cards.iter_mut() {
            if let Some(trans) = data_card.as_transform_mut()
                && id_range_to_mod.contains(&trans.transform_id.value)
            {
                trans.transform_id.value = ((trans.transform_id.value as i32) + offset) as u32;
            }
        }
    }

    // Renumber universes
    pub fn renumber_universes(&mut self, id_range_to_mod: RangeInclusive<u32>, offset: i32) {
        for cell in self.cells.iter_mut() {
            for parameter in cell.params.iter_mut() {
                if let ParamType::U(universe_id) = &mut parameter.param_type
                    && id_range_to_mod.contains(universe_id)
                {
                    *universe_id = ((*universe_id as i32) + offset) as u32;
                } else if let ParamType::Fill(FillData { universe, .. }) = &mut parameter.param_type
                    && id_range_to_mod.contains(universe)
                {
                    *universe = ((*universe as i32) + offset) as u32;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell_card::CellCard;
    use std::path::PathBuf;

    fn mock_model() -> Model {
        Model::from_text(
            &PathBuf::new(),
            "Mock model title
10 101 -1.23 -1 #100 IMP:N=1 FILL=1 (67)
20 101 -1.23  2 #200 IMP:N=1 FILL=1 (67)
30 102 -1.23 -3 #300 IMP:N=1 FILL=2 (3)

1 67 SO 10
2 SO 20
3 3 SO 30

M101 92235.70c 1.0
M102 92235.70c 1.0
*TR3 1 1 1 
TR67 2 2 2
",
        )
        .unwrap()
    }

    fn cell_geo_surface_refs(cell: &CellCard) -> Vec<i32> {
        cell.geometry
            .iter()
            .filter_map(|g| {
                if let GeoElementSpanned::Surface(r) = g {
                    Some(r.value)
                } else {
                    None
                }
            })
            .collect()
    }

    fn cell_geo_cell_refs(cell: &CellCard) -> Vec<u32> {
        cell.geometry
            .iter()
            .filter_map(|g| {
                if let GeoElementSpanned::Cell(r) = g {
                    Some(r.value)
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn cells_in_range_are_renumbered() {
        // Cells 10 and 20 are in range [10..=20]; cell 30 is outside.
        let mut model = mock_model();

        model.renumber_cells(10..=20, 5);

        let ids: Vec<u32> = model.cells.iter().map(|c| c.cell_id.value).collect();
        assert_eq!(ids, vec![15, 25, 30]);
    }

    #[test]
    fn geometry_cell_refs_in_range_are_renumbered() {
        // Cell 1 has geometry referencing cells 10 and 30 via the complement (#) operator.
        // Only the reference to cell 10 (inside range [10..=20]) should change.
        let mut model = mock_model();

        model.renumber_cells(100..=200, 5);

        let refs = cell_geo_cell_refs(&model.cells[0]);
        assert_eq!(refs, vec![105]);
        let refs = cell_geo_cell_refs(&model.cells[1]);
        assert_eq!(refs, vec![205]);
        let refs = cell_geo_cell_refs(&model.cells[2]);
        assert_eq!(refs, vec![300]);
    }

    #[test]
    fn negative_offset_renumbers_correctly() {
        let mut model = mock_model();

        model.renumber_cells(10..=20, -5);

        let ids: Vec<u32> = model.cells.iter().map(|c| c.cell_id.value).collect();
        // Both cell IDs should decrease by 5
        assert_eq!(ids, vec![5, 15, 30]);

        // The cell reference inside cell 20 should also decrease by 5
        model.renumber_cells(200..=200, -199);
        let refs = cell_geo_cell_refs(&model.cells[1]);
        assert_eq!(refs, vec![1]);
    }

    #[test]
    fn surface_ids_in_range_are_renumbered() {
        let mut model = mock_model();

        model.renumber_surfaces(1..=2, 10);

        let ids: Vec<u32> = model.surfaces.iter().map(|s| s.surface_id.value).collect();
        assert_eq!(ids, vec![11, 12, 3]);

        let refs = cell_geo_surface_refs(&model.cells[0]);
        assert_eq!(refs, vec![-11]);
        let refs = cell_geo_surface_refs(&model.cells[1]);
        assert_eq!(refs, vec![12]);
        let refs = cell_geo_surface_refs(&model.cells[2]);
        assert_eq!(refs, vec![-3]);
    }

    #[test]
    fn material_numbers_in_range_are_renumbered() {
        let mut model = mock_model();
        model.renumber_materials(101..=101, 10);

        // Check cells
        let material_numbers: Vec<u32> = model.cells.iter().map(|c| c.material_id.value).collect();
        assert_eq!(material_numbers, vec![111, 111, 102]);

        // Check data cards
        let material_numbers: Vec<u32> = model
            .data_cards
            .iter()
            .filter_map(|dc| {
                if let crate::DataCard::Material(mat_card) = dc {
                    Some(mat_card.material_id.value)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(material_numbers, vec![111, 102]);
    }

    #[test]
    fn transformation_numbers_in_range_are_renumbered() {
        let mut model = mock_model();
        model.renumber_transformations(50..=100, 10);

        // Check cell FILL parameters
        let fill_transforms: Vec<Option<u32>> = model
            .cells
            .iter()
            .map(|c| {
                c.params.iter().find_map(|p| {
                    if let ParamType::Fill(FillData { transform, .. }) = &p.param_type {
                        *transform
                    } else {
                        None
                    }
                })
            })
            .collect();
        assert_eq!(fill_transforms, vec![Some(77), Some(77), Some(3)]);

        // Check surface cards
        let transform_numbers: Vec<u32> = model
            .surfaces
            .iter()
            .filter_map(|s| s.transform_id.as_ref().map(|t| t.value))
            .collect();
        assert_eq!(transform_numbers, vec![77, 3]);

        // Check data cards
        let transformation_numbers: Vec<u32> = model
            .data_cards
            .iter()
            .filter_map(|dc| {
                if let crate::DataCard::Transform(trans_card) = dc {
                    Some(trans_card.transform_id.value)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(transformation_numbers, vec![3, 77]);
    }

    #[test]
    fn universe_numbers_in_range_are_renumbered() {
        let mut model = Model::from_text(
            &PathBuf::new(),
            "Mock model title
10 101 -1.23 -1 IMP:N=1 FILL=101 (67)
20 101 -1.23  2 IMP:N=1 U=101 (67)
30 102 -1.23 -3 IMP:N=1 U=105 (3)

1 67 SO 10
2 SO 20
3 3 SO 30

M101 92235.70c 1.0
",
        )
        .unwrap();
        model.renumber_universes(101..=101, 10);

        // Check cell U parameters
        let universe_params: Vec<Option<u32>> = model
            .cells
            .iter()
            .map(|c| {
                c.params.iter().find_map(|p| {
                    if let ParamType::U(universe_id) = &p.param_type {
                        Some(*universe_id)
                    } else {
                        None
                    }
                })
            })
            .collect();
        assert_eq!(universe_params, vec![None, Some(111), Some(105)]);

        // Check cell FILL parameters
        let fill_universes: Vec<Option<u32>> = model
            .cells
            .iter()
            .map(|c| {
                c.params.iter().find_map(|p| {
                    if let ParamType::Fill(FillData { universe, .. }) = &p.param_type {
                        Some(*universe)
                    } else {
                        None
                    }
                })
            })
            .collect();
        assert_eq!(fill_universes, vec![Some(111), None, None]);
    }
}
