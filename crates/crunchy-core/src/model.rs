//! The high-level `Model` facade.
//!
//! `Model` wraps a parsed tree plus diagnostics and exposes typed iterators, an
//! id-lookup index, editing (renumbering), and lossless re-emission. It is the
//! surface downstream crates and the Python bindings consume — owned types, no
//! leaked lifetimes.

use crunchy_syntax::{Diagnostic, GreenTree, Parsed, SyntaxKind};
use rustc_hash::FxHashMap;

use crate::cell::{cell_id, cells, parse_cell, promote_cell, Cell, GeomExpr, OwnedCell};
use crate::datacard::{data_cards, DataCard};
use crate::emit::emit_cell;
use crate::material::{materials, parse_material, Material};
use crate::renumber::{
    renumber_cells, renumber_materials, renumber_surfaces, renumber_transforms, renumber_universes,
};
use crate::surface::{parse_surface, surface_id, surfaces, Surface};
use crate::transform::{transforms, Transform};

/// A parsed MCNP model: the lossless tree, diagnostics, and typed access.
pub struct Model {
    tree: GreenTree,
    diagnostics: Vec<Diagnostic>,
    /// Cards that have been structurally edited, keyed by stable slot. Once a
    /// cell is here it is the source of truth for reads (`read_cell`) and is
    /// re-emitted from this typed node; the CST holds the emitted text.
    owned_cells: FxHashMap<u32, OwnedCell>,
    /// Monotonic slot counter that survives structural reparses, so a slot is
    /// never reused and handles stay stable across add/remove.
    next_slot: u32,
}

impl Model {
    /// Parse MCNP source into a `Model`. Never panics; malformed input yields
    /// diagnostics and a best-effort model.
    pub fn parse(src: impl Into<String>) -> Model {
        let Parsed { tree, diagnostics } = crunchy_syntax::parse(src);
        let next_slot = tree.next_slot();
        Model {
            tree,
            diagnostics,
            owned_cells: FxHashMap::default(),
            next_slot,
        }
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

    /// Renumber every material (`Mn` defs, cell material fields, `MT`/`MX`) via
    /// `map`. Void cells are left unchanged.
    pub fn renumber_materials(&mut self, map: impl FnMut(i64) -> i64) {
        renumber_materials(&mut self.tree, map);
    }

    /// Renumber every transform (`TRn`/`*TRn` defs and surface transform fields,
    /// periodic sign preserved) via `map`.
    pub fn renumber_transforms(&mut self, map: impl FnMut(i64) -> i64) {
        renumber_transforms(&mut self.tree, map);
    }

    /// Renumber every universe (`u=` definitions and `fill=` references,
    /// including lattice fill arrays) via `map`. Universe 0 is left unchanged.
    pub fn renumber_universes(&mut self, map: impl FnMut(i64) -> i64) {
        renumber_universes(&mut self.tree, map);
    }

    /// Read the cell at `card_index`, preferring a structurally-edited (owned)
    /// view when one exists, else the freshly-parsed CST view. This is the
    /// single read path so that reads and re-emission agree after an edit.
    pub fn read_cell(&self, card_index: usize) -> Option<CellRead<'_>> {
        let slot = self.tree.card_slot(card_index);
        if let Some(oc) = self.owned_cells.get(&slot) {
            return Some(CellRead::Owned(oc));
        }
        Some(CellRead::Parsed(parse_cell(&self.tree, card_index)?))
    }

    /// Add a new cell from MCNP `text` (a cell card body, e.g.
    /// `"10 6 -7.85 -5 6 imp:n=1"`), appended to the end of the cell block.
    /// Returns the new card's stable slot. The text must parse as exactly one
    /// well-formed cell.
    pub fn add_cell(&mut self, text: &str) -> Result<u32, EditError> {
        self.add_card(text, SyntaxKind::CELL_CARD)
    }

    /// Add a new surface from MCNP `text` (e.g. `"5 SO 12.0"`), appended to the
    /// end of the surface block. Returns the new card's stable slot.
    pub fn add_surface(&mut self, text: &str) -> Result<u32, EditError> {
        self.add_card(text, SyntaxKind::SURFACE_CARD)
    }

    /// Add a new data card from MCNP `text` (e.g. `"m7 26000 -1.0"`), appended
    /// to the end of the data block. Returns the new card's stable slot.
    pub fn add_data_card(&mut self, text: &str) -> Result<u32, EditError> {
        self.add_card(text, SyntaxKind::DATA_CARD)
    }

    /// Remove the cell numbered `id`. Returns whether a cell was removed.
    pub fn remove_cell(&mut self, id: i64) -> Result<bool, EditError> {
        self.remove_card(id, SyntaxKind::CELL_CARD)
    }

    /// Remove the surface numbered `id`. Returns whether a surface was removed.
    pub fn remove_surface(&mut self, id: i64) -> Result<bool, EditError> {
        self.remove_card(id, SyntaxKind::SURFACE_CARD)
    }

    /// Check referential integrity of the model: every surface/cell/material a
    /// cell references must exist. Returns a list of human-readable problems
    /// (empty when the model is consistent).
    pub fn validate(&self) -> Vec<String> {
        let idx = self.index();
        let mut problems = Vec::new();
        for pos in 0..self.tree.cards().len() {
            let Some(cr) = self.read_cell(pos) else {
                continue;
            };
            let id = cr.id();
            for sid in cr.surface_ids() {
                if !idx.surfaces.contains_key(&sid) {
                    problems.push(format!("cell {id} references missing surface {sid}"));
                }
            }
            for rid in cr.cell_refs() {
                if !idx.cells.contains_key(&rid) {
                    problems.push(format!("cell {id} references missing cell {rid}"));
                }
            }
            if let Some(m) = cr.material() {
                if m != 0 && !idx.materials.contains_key(&m) {
                    problems.push(format!("cell {id} references missing material {m}"));
                }
            }
        }
        problems
    }

    /// Validate `text` as a single card of `kind`, splice it into the end of the
    /// matching block, reparse, and remap slots so existing handles survive.
    fn add_card(&mut self, text: &str, kind: SyntaxKind) -> Result<u32, EditError> {
        let text = text.trim();
        validate_snippet(text, kind)?;
        let cur = self.to_source();
        let tmp = crunchy_syntax::parse(&cur).tree;
        let last = last_pos_of_kind(&tmp, kind).ok_or(EditError::NoBlock)?;
        let (_, line_end) = card_line_span(&tmp, last);
        let new_source = format!("{}{}\n{}", &cur[..line_end], text, &cur[line_end..]);
        Ok(self.reparse_with_change(new_source, Change::Insert(last + 1)))
    }

    /// Locate the card numbered `id` of `kind`, remove its physical line(s)
    /// (preserving surrounding delimiters), reparse, and remap slots.
    fn remove_card(&mut self, id: i64, kind: SyntaxKind) -> Result<bool, EditError> {
        let Some(pos) = find_pos(&self.tree, id, kind) else {
            return Ok(false);
        };
        let cur = self.to_source();
        let tmp = crunchy_syntax::parse(&cur).tree;
        let (start, end) = card_line_span(&tmp, pos);
        let new_source = format!("{}{}", &cur[..start], &cur[end..]);
        self.reparse_with_change(new_source, Change::Delete(pos));
        Ok(true)
    }

    /// Reparse `new_source` (which differs from the current model by exactly one
    /// inserted or deleted card at the given index) and reassign slots so
    /// carried-over cards keep their slot and a new card gets a fresh one.
    /// Flattens any pending value/geometry edits into the reparsed tokens.
    fn reparse_with_change(&mut self, new_source: String, change: Change) -> u32 {
        let old_slots: Vec<u32> = (0..self.tree.cards().len())
            .map(|p| self.tree.card_slot(p))
            .collect();
        let Parsed { tree, diagnostics } = crunchy_syntax::parse(new_source);
        let (new_slots, ret) = match change {
            Change::Insert(i) => {
                let fresh = self.next_slot;
                self.next_slot += 1;
                let mut v = Vec::with_capacity(old_slots.len() + 1);
                v.extend_from_slice(&old_slots[..i]);
                v.push(fresh);
                v.extend_from_slice(&old_slots[i..]);
                (v, fresh)
            }
            Change::Delete(i) => {
                let removed = old_slots[i];
                let mut v = old_slots;
                v.remove(i);
                (v, removed)
            }
        };
        let mut tree = tree;
        tree.set_card_slots(new_slots, self.next_slot);
        self.tree = tree;
        self.diagnostics = diagnostics;
        self.owned_cells.clear();
        ret
    }

    /// Set the material number of the cell at `card_index`, in place.
    ///
    /// This is a *value* edit: it replaces the existing material. It cannot
    /// change a cell between void (material 0) and a real material, because that
    /// adds or removes the density field — a structural edit — so such requests
    /// return [`EditError::VoidnessChange`].
    pub fn set_cell_material(&mut self, card_index: usize, material: i64) -> Result<(), EditError> {
        let slot = self.tree.card_slot(card_index);
        if self.owned_cells.contains_key(&slot) {
            let oc = self.owned_cells.get_mut(&slot).unwrap();
            if (oc.material == Some(0)) != (material == 0) {
                return Err(EditError::VoidnessChange);
            }
            oc.material = Some(material);
            let text = emit_cell(oc);
            self.tree.replace_card_content(card_index, text);
            return Ok(());
        }
        let cell = parse_cell(&self.tree, card_index).ok_or(EditError::NotACell)?;
        let tok = cell.material_token.ok_or(EditError::NoMaterialField)?;
        if (cell.material == Some(0)) != (material == 0) {
            return Err(EditError::VoidnessChange);
        }
        self.tree.set_token_int(tok, material);
        Ok(())
    }

    /// Set the density of the cell at `card_index`, in place (positive = atom
    /// density, negative = mass density). A void cell has no density field to
    /// set, which returns [`EditError::NoDensityField`] (adding one is a
    /// structural edit).
    pub fn set_cell_density(&mut self, card_index: usize, density: f64) -> Result<(), EditError> {
        let slot = self.tree.card_slot(card_index);
        if self.owned_cells.contains_key(&slot) {
            let oc = self.owned_cells.get_mut(&slot).unwrap();
            if oc.material == Some(0) || oc.density.is_none() {
                return Err(EditError::NoDensityField);
            }
            oc.density = Some(density);
            let text = emit_cell(oc);
            self.tree.replace_card_content(card_index, text);
            return Ok(());
        }
        let cell = parse_cell(&self.tree, card_index).ok_or(EditError::NotACell)?;
        let tok = cell.density_token.ok_or(EditError::NoDensityField)?;
        self.tree.set_token_text(tok, format!("{density}"));
        Ok(())
    }

    /// Intersect the cell's geometry with a signed surface (`id` magnitude,
    /// `negative` sense). Promotes the cell to an editable node on first use and
    /// re-emits it; every other card stays byte-for-byte.
    pub fn add_cell_surface(
        &mut self,
        card_index: usize,
        id: i64,
        negative: bool,
    ) -> Result<(), EditError> {
        self.edit_geometry(card_index, |g| g.intersect_surface(id, negative))
    }

    /// Intersect the cell's geometry with a `#n` cell complement.
    pub fn add_cell_complement(&mut self, card_index: usize, id: i64) -> Result<(), EditError> {
        self.edit_geometry(card_index, |g| g.intersect_cell_complement(id))
    }

    /// Remove every surface reference of magnitude `id` from the cell's
    /// geometry. Returns whether anything was removed. Refuses (with
    /// [`EditError::WouldEmptyGeometry`]) an edit that would leave the cell with
    /// no geometry.
    pub fn remove_cell_surface(&mut self, card_index: usize, id: i64) -> Result<bool, EditError> {
        self.remove_geometry(card_index, &move |g| g.remove_surface(id))
    }

    /// Remove every `#n` complement of cell `id` from the geometry.
    pub fn remove_cell_complement(
        &mut self,
        card_index: usize,
        id: i64,
    ) -> Result<bool, EditError> {
        self.remove_geometry(card_index, &move |g| g.remove_cell_complement(id))
    }

    /// Ensure the cell is promoted, apply `f` to its geometry, and re-emit.
    fn edit_geometry<R>(
        &mut self,
        card_index: usize,
        f: impl FnOnce(&mut GeomExpr) -> R,
    ) -> Result<R, EditError> {
        let slot = self.tree.card_slot(card_index);
        if !self.owned_cells.contains_key(&slot) {
            let oc = promote_cell(&self.tree, card_index).ok_or(EditError::NotEditableGeometry)?;
            self.owned_cells.insert(slot, oc);
        }
        let oc = self.owned_cells.get_mut(&slot).unwrap();
        let r = f(&mut oc.geometry);
        let text = emit_cell(oc);
        self.tree.replace_card_content(card_index, text);
        Ok(r)
    }

    /// Apply a removal `op` to the cell's geometry, guarding against emptying it
    /// and avoiding promotion when nothing is removed.
    fn remove_geometry(
        &mut self,
        card_index: usize,
        op: &dyn Fn(&mut GeomExpr) -> bool,
    ) -> Result<bool, EditError> {
        let slot = self.tree.card_slot(card_index);
        // Work on a copy of the current effective geometry.
        let mut trial = match self.owned_cells.get(&slot) {
            Some(oc) => oc.geometry.clone(),
            None => {
                let cell =
                    parse_cell(&self.tree, card_index).ok_or(EditError::NotEditableGeometry)?;
                if !cell.well_formed {
                    return Err(EditError::NotEditableGeometry);
                }
                cell.geometry
                    .clone()
                    .ok_or(EditError::NotEditableGeometry)?
            }
        };
        let removed = op(&mut trial);
        if !removed {
            return Ok(false);
        }
        if trial.is_empty() {
            return Err(EditError::WouldEmptyGeometry);
        }
        if !self.owned_cells.contains_key(&slot) {
            let oc = promote_cell(&self.tree, card_index).ok_or(EditError::NotEditableGeometry)?;
            self.owned_cells.insert(slot, oc);
        }
        let oc = self.owned_cells.get_mut(&slot).unwrap();
        oc.geometry = trial;
        let text = emit_cell(oc);
        self.tree.replace_card_content(card_index, text);
        Ok(true)
    }
}

/// A structural change applied by [`Model::reparse_with_change`].
enum Change {
    /// A card was inserted at this position in the reparsed tree.
    Insert(usize),
    /// The card at this position was removed.
    Delete(usize),
}

/// Byte span `[start, end)` of a card's own physical line(s): from the start of
/// its first line to just past the newline ending its last content line. Used
/// to remove a card cleanly while preserving surrounding blank delimiters and
/// comment lines.
fn card_line_span(tree: &GreenTree, pos: usize) -> (usize, usize) {
    let card = tree.cards()[pos];
    let start = tree.token_span(card.first_tok).start as usize;
    let mut content_last = card.first_tok;
    for i in tree.card_content_tokens(&card) {
        content_last = i;
    }
    let mut end = tree.token_span(content_last).end as usize;
    for i in content_last..card.tok_end {
        if tree.token_kind(i) == SyntaxKind::NEWLINE {
            end = tree.token_span(i).end as usize;
            break;
        }
    }
    (start, end)
}

/// Position of the last card of `kind`, if any.
fn last_pos_of_kind(tree: &GreenTree, kind: SyntaxKind) -> Option<usize> {
    (0..tree.cards().len()).rfind(|&i| tree.cards()[i].kind == kind)
}

/// Position of the card numbered `id` of `kind` (cell or surface), if any.
fn find_pos(tree: &GreenTree, id: i64, kind: SyntaxKind) -> Option<usize> {
    (0..tree.cards().len()).find(|&i| {
        tree.cards()[i].kind == kind
            && match kind {
                SyntaxKind::CELL_CARD => cell_id(tree, i).is_some_and(|(_, cid)| cid == id),
                SyntaxKind::SURFACE_CARD => {
                    surface_id(tree, i).is_some_and(|(_, sid, _)| sid == id)
                }
                _ => false,
            }
    })
}

/// Validate that `text` is exactly one well-formed card of `kind`, by parsing it
/// alone in a minimal wrapper model.
fn validate_snippet(text: &str, kind: SyntaxKind) -> Result<(), EditError> {
    if text.is_empty() {
        return Err(EditError::InvalidCardText);
    }
    let wrapped = match kind {
        SyntaxKind::CELL_CARD => format!("t\n{text}\n\n1 PX 0\n\nm1 1001 1\n"),
        SyntaxKind::SURFACE_CARD => format!("t\n1 0 -1\n\n{text}\n\nm1 1001 1\n"),
        SyntaxKind::DATA_CARD => format!("t\n1 0 -1\n\n1 PX 0\n\n{text}\n"),
        _ => return Err(EditError::InvalidCardText),
    };
    let tree = crunchy_syntax::parse(wrapped).tree;
    let of_kind: Vec<usize> = (0..tree.cards().len())
        .filter(|&i| tree.cards()[i].kind == kind)
        .collect();
    if of_kind.len() != 1 {
        return Err(EditError::InvalidCardText);
    }
    let i = of_kind[0];
    let ok = match kind {
        SyntaxKind::CELL_CARD => parse_cell(&tree, i).is_some(),
        SyntaxKind::SURFACE_CARD => parse_surface(&tree, i).is_some(),
        // Materials must parse as such; other data cards are accepted as-is.
        SyntaxKind::DATA_CARD => {
            let looks_material = text.trim_start().to_ascii_uppercase().starts_with('M')
                && !text.trim_start().to_ascii_uppercase().starts_with("MODE");
            !looks_material || parse_material(&tree, i).is_some()
        }
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(EditError::InvalidCardText)
    }
}

/// A read view of a cell: either a structurally-edited owned node or a freshly
/// parsed CST view. Exposes the same fields either way so callers need not
/// branch. Returned by [`Model::read_cell`].
pub enum CellRead<'a> {
    /// A structurally-edited cell (source of truth is the owned node).
    Owned(&'a OwnedCell),
    /// An unedited cell, parsed from the CST.
    Parsed(Cell),
}

impl CellRead<'_> {
    /// Cell number.
    pub fn id(&self) -> i64 {
        match self {
            CellRead::Owned(o) => o.id,
            CellRead::Parsed(c) => c.id,
        }
    }
    /// Material number (0 = void; `None` for `LIKE n BUT`).
    pub fn material(&self) -> Option<i64> {
        match self {
            CellRead::Owned(o) => o.material,
            CellRead::Parsed(c) => c.material,
        }
    }
    /// Density (positive = atom, negative = mass; `None` for void).
    pub fn density(&self) -> Option<f64> {
        match self {
            CellRead::Owned(o) => o.density,
            CellRead::Parsed(c) => c.density,
        }
    }
    /// True when the material number is 0.
    pub fn is_void(&self) -> bool {
        self.material() == Some(0)
    }
    /// Base cell for a `LIKE n BUT` card (never for an owned cell).
    pub fn like(&self) -> Option<i64> {
        match self {
            CellRead::Owned(_) => None,
            CellRead::Parsed(c) => c.like.map(|r| r.id),
        }
    }
    /// Referenced surface numbers (magnitudes), in order.
    pub fn surface_ids(&self) -> Vec<i64> {
        match self {
            CellRead::Owned(o) => o.surface_ids(),
            CellRead::Parsed(c) => c.surface_refs().iter().map(|r| r.id).collect(),
        }
    }
    /// Referenced surfaces with sense, in order.
    pub fn signed_surfaces(&self) -> Vec<i64> {
        match self {
            CellRead::Owned(o) => o.signed_surfaces(),
            CellRead::Parsed(c) => c
                .surface_refs()
                .iter()
                .map(|r| if r.negative { -r.id } else { r.id })
                .collect(),
        }
    }
    /// Referenced cells (`#n` complements, `LIKE n` base), in order.
    pub fn cell_refs(&self) -> Vec<i64> {
        match self {
            CellRead::Owned(o) => o.cell_refs(),
            CellRead::Parsed(c) => c.cell_refs().iter().map(|r| r.id).collect(),
        }
    }
    /// Whether the geometry parsed cleanly (owned cells are always well-formed).
    pub fn well_formed(&self) -> bool {
        match self {
            CellRead::Owned(_) => true,
            CellRead::Parsed(c) => c.well_formed,
        }
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
    /// The cell has no editable geometry (a `LIKE n BUT` cell, or geometry that
    /// did not parse cleanly).
    NotEditableGeometry,
    /// The edit would leave the cell with no geometry at all.
    WouldEmptyGeometry,
    /// The text supplied for a new card did not parse as exactly one card of the
    /// expected kind.
    InvalidCardText,
    /// The target block (cell/surface/data) has no card to append after.
    NoBlock,
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
            EditError::NotEditableGeometry => {
                "cell has no editable geometry (a LIKE n BUT cell, or malformed geometry)"
            }
            EditError::WouldEmptyGeometry => "edit would leave the cell with no geometry",
            EditError::InvalidCardText => {
                "text did not parse as exactly one card of the expected kind"
            }
            EditError::NoBlock => "the target block has no card to append after",
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

    #[test]
    fn add_surface_to_cell_reemits_and_is_lossless_elsewhere() {
        let mut m = Model::parse(MODEL);
        let ci = cell_pos(&m, 1); // "1 1 -1.0 -1 imp:n=1"
        m.add_cell_surface(ci, 5, true).unwrap(); // add -5
                                                  // Read view reflects the edit immediately.
        assert_eq!(m.read_cell(ci).unwrap().signed_surfaces(), vec![-1, -5]);
        let out = m.to_source();
        assert!(out.contains("1 1 -1 -1 -5 imp:n=1"), "got: {out}");
        // Other cards untouched.
        assert!(out.contains("2 0 1 imp:n=0"));
        assert!(out.contains("1 SO 5"));
        // Re-parse is clean.
        assert!(Model::parse(&out).diagnostics().is_empty());
    }

    #[test]
    fn remove_surface_and_guard_against_emptying() {
        let mut m = Model::parse("t\n1 0 -1 2 imp:n=1\n\n1 SO 5\n2 PX 0\n\nm1 1001 1\n");
        let ci = *m.index().cells.get(&1).unwrap();
        assert!(m.remove_cell_surface(ci, 2).unwrap());
        assert!(m.to_source().contains("1 0 -1 imp:n=1"));
        // Removing the last surface is refused.
        assert_eq!(
            m.remove_cell_surface(ci, 1),
            Err(EditError::WouldEmptyGeometry)
        );
    }

    #[test]
    fn material_edit_after_geometry_edit_is_consistent() {
        let mut m = Model::parse(MODEL);
        let ci = cell_pos(&m, 1);
        m.add_cell_surface(ci, 9, false).unwrap(); // promotes the cell
        m.set_cell_material(ci, 3).unwrap(); // must route through the owned node
        let out = m.to_source();
        assert!(out.contains("1 3 -1 -1 9 imp:n=1"), "got: {out}");
        assert_eq!(m.read_cell(ci).unwrap().material(), Some(3));
    }

    #[test]
    fn geometry_edit_refused_on_like_cell() {
        let mut m = Model::parse("t\n1 0 -1\n9 LIKE 1 BUT imp:n=1\n\n1 SO 5\n\nm1 1001 1\n");
        let ci = *m.index().cells.get(&9).unwrap();
        assert_eq!(
            m.add_cell_surface(ci, 2, false),
            Err(EditError::NotEditableGeometry)
        );
    }

    #[test]
    fn add_cell_appends_and_is_addressable_and_editable() {
        let mut m = Model::parse(MODEL);
        let slot = m.add_cell("10 1 -2.0 -1 imp:n=1").unwrap();
        let out = m.to_source();
        // Appended into the cell block, before the surface block delimiter.
        assert!(
            out.contains("2 0 1 imp:n=0\n10 1 -2.0 -1 imp:n=1\n\n1 SO 5"),
            "got:\n{out}"
        );
        // Addressable by number and by the returned slot; still editable.
        let ci = *m.index().cells.get(&10).unwrap();
        assert_eq!(m.tree().card_slot(ci), slot);
        m.add_cell_surface(ci, 1, false).unwrap(); // surface 1 exists in MODEL
        assert_eq!(m.read_cell(ci).unwrap().signed_surfaces(), vec![-1, 1]);
        // (Emitting a restructured cell reformats the density -2.0 as -2.)
        assert!(
            m.to_source().contains("10 1 -2 -1 1 imp:n=1"),
            "got:\n{}",
            m.to_source()
        );
        assert!(m.validate().is_empty(), "{:?}", m.validate());
    }

    #[test]
    fn add_surface_appends_to_surface_block() {
        let mut m = Model::parse(MODEL);
        m.add_surface("9 SO 12.0").unwrap();
        let out = m.to_source();
        assert!(out.contains("1 SO 5\n9 SO 12.0\n\nm1"), "got:\n{out}");
        assert!(Model::parse(&out).diagnostics().is_empty());
    }

    #[test]
    fn add_rejects_bad_text() {
        let mut m = Model::parse(MODEL);
        assert_eq!(
            m.add_cell("not a cell !!!"),
            Err(EditError::InvalidCardText)
        );
        assert_eq!(
            m.add_surface("10 1 -2.0 -1"),
            Err(EditError::InvalidCardText)
        );
    }

    #[test]
    fn remove_cell_preserves_other_cards_and_slots() {
        let mut m = Model::parse(MODEL);
        // Hold a handle-slot for cell 1 before removing cell 2.
        let ci1 = *m.index().cells.get(&1).unwrap();
        let slot1 = m.tree().card_slot(ci1);
        assert!(m.remove_cell(2).unwrap());
        let out = m.to_source();
        assert!(!out.contains("\n2 0 1"), "cell 2 not removed:\n{out}");
        assert!(out.contains("1 SO 5"), "surfaces intact");
        // Cell 1's slot still resolves to cell 1 (handle survived).
        let pos = m.tree().card_by_slot(slot1).unwrap();
        assert_eq!(m.read_cell(pos).unwrap().id(), 1);
        // Removing a missing cell is a no-op false.
        assert!(!m.remove_cell(999).unwrap());
    }

    #[test]
    fn validate_flags_dangling_reference() {
        // Cell 1 complements cell 3; removing cell 3 leaves a dangling ref.
        let mut m =
            Model::parse("t\n1 0 -1 #3 imp:n=1\n3 0 -2 imp:n=1\n\n1 SO 5\n2 SO 9\n\nm1 1001 1\n");
        assert!(m.validate().is_empty());
        m.remove_cell(3).unwrap();
        let problems = m.validate();
        assert!(
            problems.iter().any(|p| p.contains("missing cell 3")),
            "got: {problems:?}"
        );
    }
}
