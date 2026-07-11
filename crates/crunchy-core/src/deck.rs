//! The high-level `Deck` facade.
//!
//! `Deck` wraps a parsed tree plus diagnostics and exposes typed iterators, an
//! id-lookup index, editing (renumbering), and lossless re-emission. It is the
//! surface downstream crates and the Python bindings consume — owned types, no
//! leaked lifetimes.

use crunchy_syntax::{Diagnostic, GreenTree, Parsed};
use rustc_hash::FxHashMap;

use crate::cell::{cell_id, cells, Cell};
use crate::datacard::{data_cards, DataCard};
use crate::material::{materials, Material};
use crate::renumber::{renumber_cells, renumber_surfaces};
use crate::surface::{surface_id, surfaces, Surface};
use crate::transform::{transforms, Transform};

/// A parsed MCNP deck: the lossless tree, diagnostics, and typed access.
pub struct Deck {
    tree: GreenTree,
    diagnostics: Vec<Diagnostic>,
}

impl Deck {
    /// Parse MCNP source into a `Deck`. Never panics; malformed input yields
    /// diagnostics and a best-effort model.
    pub fn parse(src: impl Into<String>) -> Deck {
        let Parsed { tree, diagnostics } = crunchy_syntax::parse(src);
        Deck { tree, diagnostics }
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

    /// Re-emit the deck, applying any edits. Byte-for-byte identical to the
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

    /// Build an id-lookup index over the deck.
    pub fn index(&self) -> DeckIndex {
        DeckIndex::build(&self.tree)
    }

    /// Renumber every surface (definitions + references) via `map`.
    pub fn renumber_surfaces(&mut self, map: impl FnMut(i64) -> i64) {
        renumber_surfaces(&mut self.tree, map);
    }

    /// Renumber every cell (definitions + references) via `map`.
    pub fn renumber_cells(&mut self, map: impl FnMut(i64) -> i64) {
        renumber_cells(&mut self.tree, map);
    }
}

/// Id → card-index lookup maps for a deck. Build once, query O(1).
#[derive(Debug, Default, Clone)]
pub struct DeckIndex {
    /// Cell number → `tree.cards()` index.
    pub cells: FxHashMap<i64, usize>,
    /// Surface number → `tree.cards()` index.
    pub surfaces: FxHashMap<i64, usize>,
    /// Material number → `tree.cards()` index.
    pub materials: FxHashMap<i64, usize>,
    /// Transform number → `tree.cards()` index.
    pub transforms: FxHashMap<i64, usize>,
}

impl DeckIndex {
    /// Build all id maps in a single pass over the cards, using the light
    /// header readers (no coefficient/geometry parsing).
    pub fn build(tree: &GreenTree) -> DeckIndex {
        let mut idx = DeckIndex::default();
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

    const DECK: &str = "\
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
        let d = Deck::parse(DECK);
        assert_eq!(d.cells().count(), 2);
        assert_eq!(d.surfaces().count(), 1);
        assert_eq!(d.materials().count(), 1);
        assert_eq!(d.transforms().count(), 1);
        // sdef + m1 + tr1 are all data cards.
        assert_eq!(d.data_cards().count(), 3);
        assert!(d.diagnostics().is_empty());
        assert_eq!(d.to_source(), DECK);
    }

    #[test]
    fn index_lookups() {
        let d = Deck::parse(DECK);
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
        let mut d = Deck::parse(DECK);
        d.renumber_surfaces(|id| id + 100);
        let out = d.to_source();
        assert!(out.contains("101 SO 5"));
        assert!(out.contains("-101 imp:n=1"));
    }
}
