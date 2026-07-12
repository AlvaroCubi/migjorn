//! The high-level `Model` facade.
//!
//! `Model` wraps a parsed tree plus diagnostics and exposes typed iterators, an
//! id-lookup index, editing (renumbering), and lossless re-emission. It is the
//! surface downstream crates and the Python bindings consume — owned types, no
//! leaked lifetimes.

use crunchy_syntax::{Diagnostic, GreenTree, Parsed};
use rustc_hash::FxHashMap;

use crate::cell::{cell_id, cells, parse_cell, Cell};
use crate::datacard::{data_cards, DataCard};
use crate::material::{materials, Material};
use crate::renumber::{renumber_cells, renumber_surfaces};
use crate::surface::{surface_id, surfaces, Surface};
use crate::transform::{transforms, Transform};

/// A parsed MCNP model: the lossless tree, diagnostics, and typed access.
pub struct Model {
    tree: GreenTree,
    diagnostics: Vec<Diagnostic>,
}

impl Model {
    /// Parse MCNP source into a `Model`. Never panics; malformed input yields
    /// diagnostics and a best-effort model.
    pub fn parse(src: impl Into<String>) -> Model {
        let Parsed { tree, diagnostics } = crunchy_syntax::parse(src);
        Model { tree, diagnostics }
    }

    /// The underlying lossless tree.
    pub fn tree(&self) -> &GreenTree {
        &self.tree
    }

    /// Mutable access to the underlying tree (for low-level edits).
    pub fn tree_mut(&mut self) -> &mut GreenTree {
        &mut self.tree
    }

    /// Diagnostics produced during parsing.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Re-emit the model, applying any edits. Byte-for-byte identical to the
    /// input when unedited.
    pub fn to_source(&self) -> String {
        self.tree.to_source()
    }

    /// Iterate cells in source order.
    pub fn cells(&self) -> impl Iterator<Item = Cell> + '_ {
        cells(&self.tree)
    }

    /// Iterate surfaces in source order.
    pub fn surfaces(&self) -> impl Iterator<Item = Surface> + '_ {
        surfaces(&self.tree)
    }

    /// Iterate `TRn` transforms in source order.
    pub fn transforms(&self) -> impl Iterator<Item = Transform> + '_ {
        transforms(&self.tree)
    }

    /// Iterate `Mn` materials in source order.
    pub fn materials(&self) -> impl Iterator<Item = Material> + '_ {
        materials(&self.tree)
    }

    /// Iterate all data cards generically, in source order.
    pub fn data_cards(&self) -> impl Iterator<Item = DataCard> + '_ {
        data_cards(&self.tree)
    }

    /// Build an id-lookup index over the model.
    pub fn index(&self) -> ModelIndex {
        ModelIndex::build(&self.tree)
    }

    /// Renumber every surface (definitions + references) via `map`.
    pub fn renumber_surfaces(&mut self, map: impl FnMut(i64) -> i64) {
        renumber_surfaces(&mut self.tree, map);
    }

    /// Renumber every cell (definitions + references) via `map`.
    pub fn renumber_cells(&mut self, map: impl FnMut(i64) -> i64) {
        renumber_cells(&mut self.tree, map);
    }

    /// Set the material number of the cell at `card_index`, in place.
    ///
    /// This is a *value* edit: it replaces the existing material token. It
    /// cannot change a cell between void (material 0) and a real material,
    /// because that adds or removes the density field — a structural edit that
    /// the token-override overlay cannot express. Such requests return
    /// [`EditError::VoidnessChange`].
    pub fn set_cell_material(&mut self, card_index: usize, material: i64) -> Result<(), EditError> {
        let cell = parse_cell(&self.tree, card_index).ok_or(EditError::NotACell)?;
        let tok = cell.material_token.ok_or(EditError::NoMaterialField)?;
        if (cell.material == Some(0)) != (material == 0) {
            return Err(EditError::VoidnessChange);
        }
        self.tree.set_token_int(tok, material);
        Ok(())
    }

    /// Set the density of the cell at `card_index`, in place (positive = atom
    /// density, negative = mass density). Replaces the existing density token;
    /// a void cell has no density field to set, which returns
    /// [`EditError::NoDensityField`] (adding one is a structural edit).
    pub fn set_cell_density(&mut self, card_index: usize, density: f64) -> Result<(), EditError> {
        let cell = parse_cell(&self.tree, card_index).ok_or(EditError::NotACell)?;
        let tok = cell.density_token.ok_or(EditError::NoDensityField)?;
        self.tree.set_token_text(tok, format!("{density}"));
        Ok(())
    }
}

/// Why a value edit could not be applied. These all describe edits that would
/// require inserting or removing tokens (a structural change), which the
/// current overlay-based editing cannot express; they are not data errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    /// The target card is not a (well-formed) cell.
    NotACell,
    /// The cell has no material field to set (e.g. a `LIKE n BUT` cell).
    NoMaterialField,
    /// The cell has no density field to set (a void cell); adding one would be
    /// a structural edit.
    NoDensityField,
    /// The edit would change the cell between void and non-void, which adds or
    /// removes the density field — a structural edit.
    VoidnessChange,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            EditError::NotACell => "card is not a cell",
            EditError::NoMaterialField => "cell has no material field to set (a LIKE n BUT cell?)",
            EditError::NoDensityField => {
                "cell has no density field to set; adding density to a void cell \
                 is a structural edit (not yet supported)"
            }
            EditError::VoidnessChange => {
                "changing a cell between void (material 0) and a real material adds \
                 or removes the density field; this is a structural edit (not yet supported)"
            }
        };
        f.write_str(msg)
    }
}

impl std::error::Error for EditError {}

/// Id → card-index lookup maps for a model. Build once, query O(1).
#[derive(Debug, Default, Clone)]
pub struct ModelIndex {
    /// Cell number → `tree.cards()` index.
    pub cells: FxHashMap<i64, usize>,
    /// Surface number → `tree.cards()` index.
    pub surfaces: FxHashMap<i64, usize>,
    /// Material number → `tree.cards()` index.
    pub materials: FxHashMap<i64, usize>,
    /// Transform number → `tree.cards()` index.
    pub transforms: FxHashMap<i64, usize>,
}

impl ModelIndex {
    /// Build all id maps in a single pass over the cards, using the light
    /// header readers (no coefficient/geometry parsing).
    pub fn build(tree: &GreenTree) -> ModelIndex {
        let mut idx = ModelIndex::default();
        for i in 0..tree.cards().len() {
            if let Some((_, id)) = cell_id(tree, i) {
                idx.cells.insert(id, i);
            }
            if let Some((_, id, _)) = surface_id(tree, i) {
                idx.surfaces.insert(id, i);
            }
        }
        // Materials/transforms are few; parse them directly.
        for m in materials(tree) {
            idx.materials.insert(m.id, m.card_index);
        }
        for tr in transforms(tree) {
            idx.transforms.insert(tr.id, tr.card_index);
        }
        idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = "\
title
1 1 -1.0 -1 imp:n=1
2 0 1 imp:n=0

1 SO 5

m1 1001.31c 1
tr1 0 0 5
sdef pos=0 0 0
";

    #[test]
    fn facade_iterators_and_roundtrip() {
        let d = Model::parse(MODEL);
        assert_eq!(d.cells().count(), 2);
        assert_eq!(d.surfaces().count(), 1);
        assert_eq!(d.materials().count(), 1);
        assert_eq!(d.transforms().count(), 1);
        // sdef + m1 + tr1 are all data cards.
        assert_eq!(d.data_cards().count(), 3);
        assert!(d.diagnostics().is_empty());
        assert_eq!(d.to_source(), MODEL);
    }

    #[test]
    fn index_lookups() {
        let d = Model::parse(MODEL);
        let idx = d.index();
        assert_eq!(idx.cells.len(), 2);
        assert!(idx.cells.contains_key(&1));
        assert!(idx.cells.contains_key(&2));
        assert!(idx.surfaces.contains_key(&1));
        assert!(idx.materials.contains_key(&1));
        assert!(idx.transforms.contains_key(&1));
    }

    #[test]
    fn facade_renumber() {
        let mut d = Model::parse(MODEL);
        d.renumber_surfaces(|id| id + 100);
        let out = d.to_source();
        assert!(out.contains("101 SO 5"));
        assert!(out.contains("-101 imp:n=1"));
    }

    /// Position of the first cell card whose id equals `id`.
    fn cell_pos(m: &Model, id: i64) -> usize {
        *m.index().cells.get(&id).unwrap()
    }

    #[test]
    fn set_material_and_density_in_place() {
        let mut m = Model::parse(MODEL);
        let ci = cell_pos(&m, 1); // "1 1 -1.0 -1 imp:n=1"
        m.set_cell_material(ci, 124).unwrap();
        m.set_cell_density(ci, 7.93).unwrap();
        let out = m.to_source();
        assert!(out.contains("1 124 7.93 -1 imp:n=1"), "got: {out}");
        // Everything else is byte-identical.
        assert!(out.contains("2 0 1 imp:n=0"));
        assert!(out.contains("1 SO 5"));
    }

    #[test]
    fn density_on_void_cell_is_rejected() {
        let mut m = Model::parse(MODEL);
        let ci = cell_pos(&m, 2); // void cell "2 0 1"
        assert_eq!(m.set_cell_density(ci, 1.0), Err(EditError::NoDensityField));
    }

    #[test]
    fn voidness_change_is_rejected() {
        let mut m = Model::parse(MODEL);
        // void -> real material would need a density field inserted.
        let void_ci = cell_pos(&m, 2);
        assert_eq!(
            m.set_cell_material(void_ci, 5),
            Err(EditError::VoidnessChange)
        );
        // real material -> void would need the density field removed.
        let mat_ci = cell_pos(&m, 1);
        assert_eq!(
            m.set_cell_material(mat_ci, 0),
            Err(EditError::VoidnessChange)
        );
    }

    #[test]
    fn material_swap_between_real_materials_is_allowed() {
        let mut m = Model::parse(MODEL);
        let ci = cell_pos(&m, 1);
        m.set_cell_material(ci, 7).unwrap();
        assert!(m.to_source().contains("1 7 -1.0 -1 imp:n=1"));
    }
}
