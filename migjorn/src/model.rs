use crate::parser_utils::OriginalBytes;
use crate::{CardError, CellCard, DataCard, SurfaceCard};

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

mod extract_universe;
mod reader;
mod renumbering;
mod validation_checks;
mod writer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSection {
    Cells,
    Surfaces,
    Data,
}

impl std::fmt::Display for ModelSection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelSection::Cells => write!(f, "Cells"),
            ModelSection::Surfaces => write!(f, "Surfaces"),
            ModelSection::Data => write!(f, "Data"),
        }
    }
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Parse error in {section} section: {error}")]
    Parse { section: ModelSection, error: CardError },
}

#[derive(Clone)]
pub struct Model {
    pub path: PathBuf,
    pub title: OriginalBytes,
    pub cells: Vec<CellCard>,
    pub surfaces: Vec<SurfaceCard>,
    pub data_cards: Vec<DataCard>,

    // Shared immutable source file bytes — cards reference this via OriginalBytes::Source.
    #[allow(dead_code)]
    source: Arc<[u8]>,
}

impl PartialEq for Model {
    /// Semantic model equality.
    ///
    /// Compares parsed card content and ignores file path, shared source bytes,
    /// spans, original formatting, and comments.
    fn eq(&self, other: &Self) -> bool {
        self.cells == other.cells
            && self.surfaces == other.surfaces
            && self.data_cards == other.data_cards
    }
}

impl Model {
    /// Get the IDs of the cells in the model
    pub fn cell_ids(&self) -> HashSet<u32> {
        self.cells.iter().map(|c| c.cell_id.value).collect()
    }

    /// Get the IDs of the surfaces in the model
    pub fn surface_ids(&self) -> HashSet<u32> {
        self.surfaces.iter().map(|s| s.surface_id.value).collect()
    }

    /// Get the IDs of the materials in the model
    pub fn material_ids(&self) -> HashSet<u32> {
        self.data_cards
            .iter()
            .filter_map(|d| d.as_material().map(|m| m.material_id.value))
            .collect()
    }

    /// Get the IDs of the transformation cards in the model
    pub fn transform_ids(&self) -> HashSet<u32> {
        self.data_cards
            .iter()
            .filter_map(|d| d.as_transform().map(|t| t.transform_id.value))
            .collect()
    }

    /// Get the IDs of all universes referenced in the model via U= cell parameters
    pub fn universe_ids(&self) -> HashSet<u32> {
        self.cells
            .iter()
            .filter_map(|cell| cell.get_universe())
            .collect()
    }
}
