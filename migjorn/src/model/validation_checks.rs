use std::collections::HashSet;
use std::fmt::Display;

use crate::{
    FillData,
    cell_card::{GeoElementSpanned, ParamType},
};

use super::Model;

/// Report produced by [`Model::validation_checks`]. Contains the set of IDs
/// that are either referenced but not defined, or defined more than once.
/// All sets are empty when the model is valid.
#[derive(Debug)]
pub struct ValidationReport {
    pub missing_cells: HashSet<u32>,
    pub missing_surfaces: HashSet<u32>,
    pub missing_materials: HashSet<u32>,
    pub missing_transforms: HashSet<u32>,
    pub duplicate_cells: HashSet<u32>,
    pub duplicate_surfaces: HashSet<u32>,
    pub duplicate_materials: HashSet<u32>,
    pub duplicate_transforms: HashSet<u32>,
}

impl ValidationReport {
    pub fn is_valid(&self) -> bool {
        self.missing_cells.is_empty()
            && self.missing_surfaces.is_empty()
            && self.missing_materials.is_empty()
            && self.missing_transforms.is_empty()
            && self.duplicate_cells.is_empty()
            && self.duplicate_surfaces.is_empty()
            && self.duplicate_materials.is_empty()
            && self.duplicate_transforms.is_empty()
    }
}

impl Display for ValidationReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_valid() {
            return write!(f, "The model passed the validation checks!");
        }
        if !self.duplicate_cells.is_empty() {
            writeln!(
                f,
                "Error: Duplicate cell IDs defined more than once: {:?}",
                self.duplicate_cells
            )?;
        }
        if !self.duplicate_surfaces.is_empty() {
            writeln!(
                f,
                "Error: Duplicate surface IDs defined more than once: {:?}",
                self.duplicate_surfaces
            )?;
        }
        if !self.duplicate_materials.is_empty() {
            writeln!(
                f,
                "Error: Duplicate material IDs defined more than once: {:?}",
                self.duplicate_materials
            )?;
        }
        if !self.duplicate_transforms.is_empty() {
            writeln!(
                f,
                "Error: Duplicate transform IDs defined more than once: {:?}",
                self.duplicate_transforms
            )?;
        }
        if !self.missing_cells.is_empty() {
            writeln!(
                f,
                "Error: Missing cell IDs referenced in geometry: {:?}",
                self.missing_cells
            )?;
        }
        if !self.missing_surfaces.is_empty() {
            writeln!(
                f,
                "Error: Missing surface IDs referenced in geometry: {:?}",
                self.missing_surfaces
            )?;
        }
        if !self.missing_materials.is_empty() {
            writeln!(
                f,
                "Error: Missing material IDs referenced in cells: {:?}",
                self.missing_materials
            )?;
        }
        if !self.missing_transforms.is_empty() {
            writeln!(
                f,
                "Error: Missing transform IDs referenced in cells or surfaces: {:?}",
                self.missing_transforms
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationReport {}

impl Model {
    /// Perform checks on the model.
    ///
    /// Returns `Ok(())` if the model is consistent, or `Err(Box<ValidationReport>)`
    /// containing the sets of missing or duplicate IDs for each entity type.
    pub fn validation_checks(&self) -> Result<(), Box<ValidationReport>> {
        // Detect duplicate cell IDs
        let (cell_ids, duplicate_cells) =
            collect_ids_and_duplicates(self.cells.iter().map(|c| c.cell_id.value));

        // Detect duplicate surface IDs
        let (surface_ids, duplicate_surfaces) =
            collect_ids_and_duplicates(self.surfaces.iter().map(|s| s.surface_id.value));

        // Detect duplicate material IDs
        let (material_ids, duplicate_materials) = collect_ids_and_duplicates(
            self.data_cards
                .iter()
                .filter_map(|d| d.as_material().map(|m| m.material_id.value)),
        );

        // Detect duplicate transform IDs
        let (transform_ids, duplicate_transforms) = collect_ids_and_duplicates(
            self.data_cards
                .iter()
                .filter_map(|d| d.as_transform().map(|t| t.transform_id.value)),
        );

        // Check for missing cell IDs referenced in geometry
        let missing_cells: HashSet<u32> = self
            .cells
            .iter()
            .flat_map(|cell| {
                cell.geometry.iter().filter_map(|geo_element| {
                    if let GeoElementSpanned::Cell(c) = geo_element {
                        Some(c.value)
                    } else {
                        None
                    }
                })
            })
            .filter(|c| !cell_ids.contains(c))
            .collect();

        // Check for missing surface IDs referenced in geometry
        let missing_surfaces: HashSet<u32> = self
            .cells
            .iter()
            .flat_map(|cell| {
                cell.geometry.iter().filter_map(|geo_element| {
                    if let GeoElementSpanned::Surface(s) = geo_element {
                        Some(s.value.unsigned_abs())
                    } else {
                        None
                    }
                })
            })
            .filter(|s| !surface_ids.contains(s))
            .collect();

        // Check for missing material IDs referenced in cells
        let missing_materials: HashSet<u32> = self
            .cells
            .iter()
            .map(|c| c.material_id.value)
            .filter(|m| *m != 0 && !material_ids.contains(m))
            .collect();

        // Check for missing transform IDs referenced in cells or surfaces
        let mut missing_transforms: HashSet<u32> = self
            .cells
            .iter()
            .flat_map(|c| {
                c.params.iter().filter_map(|p| {
                    if let ParamType::Fill(FillData {
                        transform: Some(t), ..
                    }) = &p.param_type
                    {
                        Some(*t)
                    } else {
                        None
                    }
                })
            })
            .filter(|t| !transform_ids.contains(t))
            .collect();
        self.surfaces
            .iter()
            .filter_map(|s| s.transform_id.as_ref())
            .filter(|t| !transform_ids.contains(&t.value))
            .for_each(|t| {
                missing_transforms.insert(t.value);
            });

        let report = ValidationReport {
            missing_cells,
            missing_surfaces,
            missing_materials,
            missing_transforms,
            duplicate_cells,
            duplicate_surfaces,
            duplicate_materials,
            duplicate_transforms,
        };

        if report.is_valid() {
            Ok(())
        } else {
            Err(Box::new(report))
        }
    }
}

/// Collect unique IDs into a `HashSet` and simultaneously gather any IDs that
/// appear more than once into a separate `HashSet`.
fn collect_ids_and_duplicates(iter: impl Iterator<Item = u32>) -> (HashSet<u32>, HashSet<u32>) {
    let mut seen = HashSet::new();
    let mut duplicates = HashSet::new();
    for id in iter {
        if !seen.insert(id) {
            duplicates.insert(id);
        }
    }
    (seen, duplicates)
}
